//! The IR edge replay and rollback engine, shared by every networked
//! transport (direct TCP, relay, and the browser client).
//!
//! Incoming edges are timestamped in the sender's emulated nanoseconds.
//! Network jitter would corrupt a frame if edges were replayed as they
//! arrived: within a frame the firmware free-runs on its 8192Hz tick and
//! tolerates only a fraction of a bit time (~1ms) of skew. Edges are
//! therefore replayed per burst: the first edge after a gap anchors the
//! burst `REPLAY_LATENCY_NS` into the local future, and the rest of the
//! burst keeps its exact sender-relative spacing. Only the idle gaps
//! between frames stretch, which the protocol does not care about.
//!
//! The engine also decides when the platform must roll the machine back
//! (see `crate::ir::IrRollbackControl` for the coordination protocol):
//! a frame that arrives while the receiver is powered off, or a receiver
//! powering off with a frame still replaying, is held and replayed early
//! in the restored listen window instead. Outgoing edges always stream;
//! a rollback only grows `wire_offset_ns` so that outgoing timestamps
//! stay monotonic, and optionally closes a dangling carrier-on.
//!
//! A rollback may only rewind what is still recallable. Any *completed*
//! burst since the snapshot — outgoing (the peer will consume that
//! frame) or fully delivered incoming (the local firmware consumed it)
//! — vetoes the rollback, because rewinding would fork history the
//! other side already acted on. Partial bursts are fine: a mid-frame
//! outgoing burst is truncated by the closing edge and rejected by the
//! peer's checksum, and a partially delivered incoming burst is
//! retained and replayed whole.
//!
//! # Atomic burst resend
//!
//! The live stream sends each edge as its own tiny wire message, and
//! real internet paths clump such thin streams: measured against a
//! deployed relay, mid-burst stalls of 50–120ms (one round trip of
//! head-of-line blocking) blow the `REPLAY_LATENCY_NS` budget on every
//! frame, while single-message bursts arrive unscathed. So after a
//! burst completes, the sender re-sends it whole as one message; a
//! burst in one message replays perfectly no matter how the network
//! stalled it, because all of it is present when the anchor is placed.
//!
//! The receiver tells the copy apart from live traffic without any
//! framing: outgoing wire time is strictly monotonic, so a timestamp
//! regression can only start a copy. If the live replay of that burst
//! was clean the copy is dropped; if a mid-burst edge missed its slot
//! (past the firmware's ~1ms skew tolerance, so the frame was
//! certainly rejected — a missed slot on the *final* edge is
//! inconclusive and does not condemn the frame), the corrupt replay is
//! withdrawn and the copy replays in its place, exactly like a fresh
//! burst — including the hold-and-rollback path when the listen window
//! has meanwhile closed. A replacement copy is reassembled before it
//! replays, so even a copy the network split still replays whole.

use crate::ir::RollbackDirective;
use std::collections::VecDeque;

/// A gap this long in the sender's timeline separates bursts (frames).
/// The longest in-frame carrier-off run is one bit time, ~1ms.
pub const BURST_GAP_NS: u64 = 3_000_000;

/// How far into the local future the start of a burst is scheduled. This
/// is the jitter budget: edges arriving up to this much later than the
/// burst's first edge still replay with exact relative timing.
///
/// It also delays every frame. Live delivery works while two of these
/// plus the network round trip fit into the firmware's ~98ms listen
/// window; beyond that, rollback takes over.
pub const REPLAY_LATENCY_NS: u64 = 10_000_000;

/// A snapshot older than this cannot be rolled back to: past a couple of
/// seconds the peer's own retransmissions are the better recovery.
pub const SNAPSHOT_MAX_AGE_NS: u64 = 2_000_000_000;

/// An incoming edge scheduled this far before its arrival marks the
/// burst's live replay as corrupt (and its atomic copy as the recovery).
/// Set above the firmware's ~1ms modem skew tolerance so a flagged burst
/// was certainly rejected — replaying the copy after a frame the
/// firmware might have consumed would deliver that frame twice.
pub const LATE_EDGE_THRESHOLD_NS: u64 = 2_000_000;

/// Bursts with more edges than this are not resent whole: they would
/// not fit one relay message, and no real frame comes close (this many
/// edges is several seconds of carrier activity).
const MAX_RESEND_EDGES: usize = 7_000;

/// Completed bursts queued for resend beyond this are dropped; the
/// transports pump the queue every pacing quantum, so reaching it means
/// the platform stopped polling.
const MAX_QUEUED_RESENDS: usize = 16;

/// Placeholder until `set_clock_rate` is called.
const DEFAULT_CLOCK_RATE: u64 = 16_000_000;

/// Backstop against unbounded growth if the emulator stops polling.
const MAX_TRACKED_EDGES: usize = 100_000;

pub struct ReplayEngine {
    clock_rate: u64,

    /// The receiver's current demodulated level.
    level: bool,
    /// Undelivered edges: (local delivery cycle, sender ns, level).
    play_queue: VecDeque<(u64, u64, bool)>,
    last_sender_ns: Option<u64>,
    /// (sender ns, local cycle) correspondence for the current burst.
    anchor: Option<(u64, u64)>,
    last_scheduled_cycle: u64,

    /// Already-delivered edges of the burst currently replaying, in
    /// sender time, retained so an interrupted burst can be replayed
    /// whole after a rollback.
    delivered_burst: Vec<(u64, bool)>,

    /// Receiver power as reported by the board (PB6 on the Miuchiz).
    rx_powered: bool,
    /// Edges (sender ns, level) waiting to be delivered into a restored
    /// or freshly opened listen window instead of live.
    held: Vec<(u64, bool)>,
    /// While set, incoming edges accumulate into `held`.
    holding: bool,

    want_snapshot: bool,
    armed_snapshot_cycle: Option<u64>,
    rollback_requested: bool,

    /// Cycle at which the most recent incoming edge was delivered.
    last_delivery_cycle: Option<u64>,
    /// Delivery cycle of the final edge of the most recent incoming
    /// burst known to have been delivered in full: a frame the firmware
    /// consumed, which a rollback must not rewind past.
    last_consumed_cycle: Option<u64>,

    /// Sender timestamp of the current incoming burst's first edge;
    /// an atomic copy of the burst starts with the same timestamp.
    burst_first_ns: Option<u64>,
    /// Whether a mid-burst edge of the current incoming burst missed
    /// its replay slot by more than `LATE_EDGE_THRESHOLD_NS`: the
    /// replay is corrupt, and only the burst's atomic copy can still
    /// deliver it.
    burst_late: bool,
    /// Whether the previous edge of the current burst missed its slot;
    /// that only condemns the burst once another edge follows it (a
    /// missed slot on the final edge is inconclusive).
    prev_edge_missed_slot: bool,
    /// While set, incoming edges up to this sender timestamp belong to
    /// the atomic copy of a burst that already replayed cleanly, and
    /// are dropped.
    dedup_until_ns: Option<u64>,
    /// While set, incoming edges within a burst gap of this timestamp
    /// belong to a burst (or its copy) that arrived while an earlier
    /// frame was held, and are dropped as redundant retransmission.
    discard_tail_ns: Option<u64>,
    /// A burst copy being reassembled, and the sender timestamp of its
    /// final record; it replays only once complete, so a copy split by
    /// the network still replays whole.
    copy_buffer: Vec<(u64, bool)>,
    copy_end_ns: Option<u64>,

    /// Cycle of the most recent outgoing edge.
    last_outgoing_cycle: Option<u64>,
    /// Cycle of the final edge of the most recent completed outgoing
    /// burst: a frame the peer will consume, which a rollback must not
    /// rewind past.
    outgoing_completed_cycle: Option<u64>,

    /// Added to outgoing timestamps; grows by the rewound span on each
    /// rollback so wire time never runs backwards.
    wire_offset_ns: u64,
    last_sent_level: bool,
    /// The last timestamp put on the wire. Outgoing wire time is kept
    /// *strictly* increasing past this, so the receiving engine can
    /// recognize a burst copy by regression alone.
    last_wire_ns: u64,

    /// Records of the outgoing burst currently in progress, retained so
    /// the completed burst can be resent whole.
    outgoing_burst: Vec<(u64, bool)>,
    /// Completed bursts awaiting pickup by the transport (via
    /// [`Self::take_burst_resend`]), oldest first.
    resend_queue: VecDeque<Vec<(u64, bool)>>,
}

/// Whether a requested rollback may proceed. Denial is final for the
/// armed snapshot; deferral means the answer hinges on whether the
/// outgoing burst in flight turns out to be mid-frame or complete.
enum Admissibility {
    Grant,
    Deny,
    Defer,
}

impl Default for ReplayEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl ReplayEngine {
    pub fn new() -> Self {
        Self {
            clock_rate: DEFAULT_CLOCK_RATE,
            level: false,
            play_queue: VecDeque::new(),
            last_sender_ns: None,
            anchor: None,
            last_scheduled_cycle: 0,
            delivered_burst: Vec::new(),
            // PB6 idles high until the firmware drives it.
            rx_powered: true,
            held: Vec::new(),
            holding: false,
            want_snapshot: false,
            armed_snapshot_cycle: None,
            rollback_requested: false,
            last_delivery_cycle: None,
            last_consumed_cycle: None,
            burst_first_ns: None,
            burst_late: false,
            prev_edge_missed_slot: false,
            dedup_until_ns: None,
            discard_tail_ns: None,
            copy_buffer: Vec::new(),
            copy_end_ns: None,
            last_outgoing_cycle: None,
            outgoing_completed_cycle: None,
            wire_offset_ns: 0,
            last_sent_level: false,
            last_wire_ns: 0,
            outgoing_burst: Vec::new(),
            resend_queue: VecDeque::new(),
        }
    }

    pub fn set_clock_rate(&mut self, clock_rate: u64) {
        if clock_rate > 0 {
            self.clock_rate = clock_rate;
        }
    }

    /// Forgets all link state (edges, holds, snapshot arming). Called on
    /// reconnect or re-pairing; the wire offset (and the strictness
    /// watermark) survive so outgoing timestamps stay monotonic
    /// regardless.
    pub fn reset(&mut self) {
        self.play_queue.clear();
        self.level = false;
        self.last_sender_ns = None;
        self.anchor = None;
        self.delivered_burst.clear();
        self.held.clear();
        self.holding = false;
        self.rollback_requested = false;
        self.armed_snapshot_cycle = None;
        self.want_snapshot = false;
        self.last_delivery_cycle = None;
        self.last_consumed_cycle = None;
        self.burst_first_ns = None;
        self.burst_late = false;
        self.prev_edge_missed_slot = false;
        self.dedup_until_ns = None;
        self.discard_tail_ns = None;
        self.copy_buffer.clear();
        self.copy_end_ns = None;
        self.last_outgoing_cycle = None;
        self.outgoing_completed_cycle = None;
        self.last_sent_level = false;
        self.outgoing_burst.clear();
        self.resend_queue.clear();
    }

    fn cycles_to_ns(&self, cycles: u64) -> u64 {
        (cycles as u128 * 1_000_000_000 / self.clock_rate as u128) as u64
    }

    fn ns_to_cycles(&self, ns: u64) -> u64 {
        (ns as u128 * self.clock_rate as u128 / 1_000_000_000) as u64
    }

    fn snapshot_is_fresh(&self, now_cycle: u64) -> bool {
        match self.armed_snapshot_cycle {
            Some(at) => now_cycle.saturating_sub(at) < self.ns_to_cycles(SNAPSHOT_MAX_AGE_NS),
            None => false,
        }
    }

    /// Accepts one received edge, scheduling it for replay (or holding
    /// it, when it can only be heard through a rollback).
    pub fn push_incoming(&mut self, cycle: u64, sender_ns: u64, level: bool) {
        // Tail (or atomic copy) of a burst being discarded because it
        // arrived while its predecessor was held.
        if let Some(tail) = self.discard_tail_ns {
            if sender_ns.saturating_sub(tail) <= BURST_GAP_NS {
                self.discard_tail_ns = Some(tail.max(sender_ns));
                return;
            }
            self.discard_tail_ns = None;
        }

        // A burst copy being reassembled: buffered until its final
        // record so it replays whole even if the network split it.
        if let Some(end) = self.copy_end_ns {
            if sender_ns <= end {
                if self.copy_buffer.len() < MAX_RESEND_EDGES {
                    self.copy_buffer.push((sender_ns, level));
                }
                if sender_ns == end {
                    self.replay_copy(cycle);
                }
                return;
            }
            // The stream moved past the copy without completing it
            // (records were dropped): a partial copy is unusable.
            self.copy_end_ns = None;
            self.copy_buffer.clear();
        }

        if let Some(boundary) = self.dedup_until_ns {
            if sender_ns <= boundary {
                return; // rest of a copy whose live replay was clean
            }
            self.dedup_until_ns = None;
        }

        if let Some(previous) = self.last_sender_ns {
            if sender_ns <= previous {
                // Live wire time is strictly increasing, so a regression
                // can only start the atomic copy of a completed burst.
                // It replaces the current burst only if the live replay
                // was certainly rejected by the firmware; otherwise it
                // has nothing to add.
                let replaces_current =
                    self.burst_late && !self.holding && self.burst_first_ns == Some(sender_ns);
                if !replaces_current {
                    self.dedup_until_ns = Some(previous);
                    return;
                }
                // The copy ends where the live burst ended (identical
                // records); collect it before replaying.
                self.copy_end_ns = Some(previous);
                self.copy_buffer.clear();
                self.copy_buffer.push((sender_ns, level));
                return;
            }
        }

        self.ingest(cycle, sender_ns, level);
    }

    /// Replays a fully reassembled burst copy in place of its corrupt
    /// live replay. The live version never reached the firmware as a
    /// valid frame (a mid-burst edge missed its slot past the modem's
    /// skew tolerance), so re-delivering the frame cannot duplicate it.
    fn replay_copy(&mut self, cycle: u64) {
        self.copy_end_ns = None;
        self.play_queue.clear();
        self.delivered_burst.clear();
        self.level = false;
        self.last_sender_ns = None;
        // The withdrawn schedule must not displace the copy's.
        self.last_scheduled_cycle = cycle;
        self.prev_edge_missed_slot = false;
        let buffer = std::mem::take(&mut self.copy_buffer);
        for (sender_ns, level) in buffer {
            self.ingest(cycle, sender_ns, level);
        }
    }

    /// Burst bookkeeping and scheduling for one live (or copy-replayed)
    /// edge, past all the copy/dedup handling.
    fn ingest(&mut self, cycle: u64, sender_ns: u64, level: bool) {
        let new_burst = match self.last_sender_ns {
            None => true,
            Some(previous) => sender_ns.saturating_sub(previous) > BURST_GAP_NS,
        };

        if self.holding && new_burst {
            // The held frame is complete (its tail preceded this burst
            // in the stream), so this newer burst is a retransmission
            // of it: redundant once the held frame replays into the
            // restored window. Discard it, and its copy with it.
            self.discard_tail_ns = Some(sender_ns);
            return;
        }
        self.last_sender_ns = Some(sender_ns);

        if new_burst {
            // The previous burst is over; if every edge of it was
            // delivered on time, the firmware consumed a frame, and no
            // rollback may rewind past that delivery. (A late-corrupted
            // burst was rejected, not consumed, so it vetoes nothing.)
            if self.play_queue.is_empty() && !self.delivered_burst.is_empty() && !self.burst_late {
                self.last_consumed_cycle = self.last_delivery_cycle;
            }
            self.delivered_burst.clear();
            self.burst_first_ns = Some(sender_ns);
            self.burst_late = false;
            self.prev_edge_missed_slot = false;
            if !self.rx_powered && self.snapshot_is_fresh(cycle) {
                // The receiver cannot hear this frame; hold it and ask
                // for a rollback into the last listen window.
                self.holding = true;
                self.held.clear();
                self.rollback_requested = true;
            } else {
                self.anchor = Some((sender_ns, cycle + self.ns_to_cycles(REPLAY_LATENCY_NS)));
            }
        }

        if self.holding {
            if self.held.len() < MAX_TRACKED_EDGES {
                self.held.push((sender_ns, level));
            }
            return;
        }

        let (anchor_ns, anchor_cycle) = self.anchor.expect("anchor set for scheduled edges");
        let scheduled = anchor_cycle + self.ns_to_cycles(sender_ns.saturating_sub(anchor_ns));
        let mut at = scheduled;
        if at <= self.last_scheduled_cycle {
            at = self.last_scheduled_cycle + 1;
        }
        self.last_scheduled_cycle = at;
        // An edge that misses its slot (arrived too late, or displaced
        // past it by a still-replaying earlier burst) distorts the
        // waveform — but only provably garbles the frame if more of the
        // frame follows it. A missed slot on the burst's final edge
        // merely stretches the trailing carrier run, which the firmware
        // may well have decoded past already; condemning the burst then
        // would re-deliver a frame it may have consumed.
        let tolerance = self.ns_to_cycles(LATE_EDGE_THRESHOLD_NS);
        if self.prev_edge_missed_slot {
            self.burst_late = true;
        }
        self.prev_edge_missed_slot = scheduled + tolerance < cycle || at > scheduled + tolerance;
        if self.play_queue.len() < MAX_TRACKED_EDGES {
            self.play_queue.push_back((at, sender_ns, level));
        }
    }

    /// Delivers due edges and returns the receiver's current level.
    pub fn current_level(&mut self, cycle: u64) -> bool {
        while let Some(&(at, ns, level)) = self.play_queue.front() {
            if at > cycle {
                break;
            }
            self.level = level;
            self.last_delivery_cycle = Some(at);
            if self.delivered_burst.len() < MAX_TRACKED_EDGES {
                self.delivered_burst.push((ns, level));
            }
            self.play_queue.pop_front();
        }
        self.level
    }

    /// The board's receiver power changed. `can_snapshot` gates snapshot
    /// requests (a transport passes its connectedness: snapshots are
    /// pointless without a peer).
    pub fn receiver_power_changed(&mut self, cycle: u64, powered: bool, can_snapshot: bool) {
        if powered == self.rx_powered {
            return;
        }
        self.rx_powered = powered;

        if powered {
            // A listen window opened: this is the snapshot point. Any
            // held frame no longer needs a rollback — it can be heard
            // live in this window.
            if can_snapshot {
                self.want_snapshot = true;
            }
            if self.holding {
                self.deliver_held(cycle);
            }
        } else if !self.play_queue.is_empty() && self.snapshot_is_fresh(cycle) {
            // The window closed with a frame still replaying: the
            // firmware heard at most a truncated prefix. Retain the
            // whole burst and ask to replay it into the reopened window.
            self.holding = true;
            self.held.clear();
            self.held.append(&mut self.delivered_burst);
            while let Some((_, ns, level)) = self.play_queue.pop_front() {
                if self.held.len() < MAX_TRACKED_EDGES {
                    self.held.push((ns, level));
                }
            }
            self.level = false;
            self.rollback_requested = true;
        }
    }

    /// Decides whether a requested rollback may rewind to the armed
    /// snapshot. Only recallable spans may be rewound: a completed burst
    /// since the snapshot, in either direction, was already consumed (by
    /// the peer, or by the local firmware) and rewinding past it would
    /// fork history. A still-open outgoing burst is recallable — the
    /// closing edge truncates it mid-frame and the peer's checksum
    /// rejects it — but with the carrier momentarily off, "between bits"
    /// and "frame just ended" look the same, so the decision is deferred
    /// until another edge or the burst gap settles it.
    fn rollback_admissibility(&mut self, now_cycle: u64) -> Admissibility {
        let Some(snapshot_cycle) = self.armed_snapshot_cycle else {
            return Admissibility::Deny;
        };
        if !self.snapshot_is_fresh(now_cycle) {
            return Admissibility::Deny;
        }
        if matches!(self.last_consumed_cycle, Some(at) if at > snapshot_cycle) {
            return Admissibility::Deny;
        }
        if matches!(self.outgoing_completed_cycle, Some(at) if at > snapshot_cycle) {
            return Admissibility::Deny;
        }
        match self.last_outgoing_cycle {
            Some(at) if at > snapshot_cycle => {
                if self.last_sent_level {
                    // Mid-frame: truncation invalidates it for the peer.
                    Admissibility::Grant
                } else if now_cycle.saturating_sub(at) > self.ns_to_cycles(BURST_GAP_NS) {
                    // Nothing followed the carrier-off: that frame was
                    // complete, and the peer keeps it.
                    self.outgoing_completed_cycle = Some(at);
                    Admissibility::Deny
                } else {
                    Admissibility::Defer
                }
            }
            _ => Admissibility::Grant,
        }
    }

    /// The run loop's regular poll; see `crate::ir::IrRollbackControl`.
    pub fn poll(&mut self, now_cycle: u64) -> RollbackDirective {
        if self.rollback_requested {
            match self.rollback_admissibility(now_cycle) {
                Admissibility::Grant => {
                    // One rollback per snapshot: re-arming requires a new
                    // receiver-on snapshot. `holding` stays set so edges
                    // of the held burst that are still arriving keep
                    // accumulating until `rolled_back` schedules them.
                    self.armed_snapshot_cycle = None;
                    self.rollback_requested = false;
                    return RollbackDirective::RollBack;
                }
                Admissibility::Deny => {
                    // Too stale to rewind, or the span holds a consumed
                    // frame; let the held frame play (unheard) and rely
                    // on the peer's retransmissions.
                    self.armed_snapshot_cycle = None;
                    self.deliver_held(now_cycle);
                }
                Admissibility::Defer => {}
            }
        }

        if self.want_snapshot {
            self.want_snapshot = false;
            return RollbackDirective::TakeSnapshot;
        }

        RollbackDirective::Continue
    }

    pub fn snapshot_taken(&mut self, cycle: u64) {
        self.armed_snapshot_cycle = Some(cycle);
    }

    /// The machine was restored to `restored_cycle`. Replays the held
    /// frame into the reopened window. Returns the wire timestamp of a
    /// carrier-off record the transport must send if the abandoned
    /// timeline left the carrier dangling on (the peer sees a truncated
    /// burst and discards it on checksum).
    pub fn rolled_back(&mut self, restored_cycle: u64, abandoned_cycle: u64) -> Option<u64> {
        // Wire time must not run backwards: absorb the rewound span into
        // the outgoing offset.
        let rewound = abandoned_cycle.saturating_sub(restored_cycle);
        let mut close_ns = self.cycles_to_ns(abandoned_cycle) + self.wire_offset_ns;
        if close_ns <= self.last_wire_ns {
            close_ns = self.last_wire_ns + 1;
        }
        self.wire_offset_ns += self.cycles_to_ns(rewound);

        let close = if self.last_sent_level {
            self.last_sent_level = false;
            self.last_wire_ns = close_ns;
            // The truncated burst is garbage to the peer by design; its
            // atomic copy would only resend that garbage.
            self.outgoing_burst.clear();
            Some(close_ns)
        } else {
            // Any burst still tracked here completed before the snapshot
            // (a completed burst since it would have vetoed the
            // rollback), so its copy is still owed to the peer.
            self.finish_outgoing_burst();
            None
        };

        // Snapshots are only taken while the receiver is on.
        self.rx_powered = true;
        // The abandoned timeline's traffic bookkeeping is meaningless on
        // the restored one (its cycles postdate everything to come).
        self.last_delivery_cycle = None;
        self.last_consumed_cycle = None;
        self.last_outgoing_cycle = None;
        self.outgoing_completed_cycle = None;
        self.deliver_held(restored_cycle);
        close
    }

    /// Timestamps an outgoing edge, applying the rollback offset. The
    /// edge is also retained so the burst can be resent whole once it
    /// completes (see the module docs on atomic resend).
    pub fn outgoing_wire_ns(&mut self, cycle: u64, level: bool) -> u64 {
        // A burst-gap of carrier silence between edges means the earlier
        // burst completed: the peer consumed that frame.
        if let Some(previous) = self.last_outgoing_cycle {
            if cycle.saturating_sub(previous) > self.ns_to_cycles(BURST_GAP_NS) {
                self.outgoing_completed_cycle = Some(previous);
                self.finish_outgoing_burst();
            }
        }
        self.last_outgoing_cycle = Some(cycle);
        self.last_sent_level = level;
        let mut ns = self.cycles_to_ns(cycle) + self.wire_offset_ns;
        // Strictly increasing, so only a burst copy ever regresses.
        if ns <= self.last_wire_ns {
            ns = self.last_wire_ns + 1;
        }
        self.last_wire_ns = ns;
        if self.outgoing_burst.len() < MAX_RESEND_EDGES {
            self.outgoing_burst.push((ns, level));
        }
        ns
    }

    /// Moves the finished outgoing burst into the resend queue. A burst
    /// that outgrew one message's worth of records is dropped instead
    /// (its live stream was still sent, matching the old behavior).
    fn finish_outgoing_burst(&mut self) {
        if self.outgoing_burst.is_empty() || self.outgoing_burst.len() >= MAX_RESEND_EDGES {
            self.outgoing_burst.clear();
            return;
        }
        if self.resend_queue.len() < MAX_QUEUED_RESENDS {
            self.resend_queue
                .push_back(std::mem::take(&mut self.outgoing_burst));
        } else {
            self.outgoing_burst.clear();
        }
    }

    /// A completed outgoing burst to resend whole, if one is ready.
    /// Transports call this every pacing quantum (and after sending an
    /// edge, so a copy precedes the next burst's live stream) and put
    /// all returned records on the wire as ONE message.
    pub fn take_burst_resend(&mut self, now_cycle: u64) -> Option<Vec<(u64, bool)>> {
        if self.resend_queue.is_empty() && !self.outgoing_burst.is_empty() && !self.last_sent_level
        {
            if let Some(at) = self.last_outgoing_cycle {
                if now_cycle.saturating_sub(at) > self.ns_to_cycles(BURST_GAP_NS) {
                    self.finish_outgoing_burst();
                }
            }
        }
        self.resend_queue.pop_front()
    }

    /// Schedules the held edges to replay starting `REPLAY_LATENCY_NS`
    /// after `start_cycle`, dropping whatever else was scheduled (the
    /// sender's retransmissions cover anything lost).
    fn deliver_held(&mut self, start_cycle: u64) {
        self.play_queue.clear();
        self.level = false;
        self.delivered_burst.clear();
        self.last_scheduled_cycle = start_cycle;
        self.burst_late = false;
        self.prev_edge_missed_slot = false;

        let held = std::mem::take(&mut self.held);
        if let Some(&(first_ns, _)) = held.first() {
            let anchor_cycle = start_cycle + self.ns_to_cycles(REPLAY_LATENCY_NS);
            self.anchor = Some((first_ns, anchor_cycle));
            self.burst_first_ns = Some(first_ns);
            for (ns, level) in held {
                let mut at = anchor_cycle + self.ns_to_cycles(ns.saturating_sub(first_ns));
                if at <= self.last_scheduled_cycle {
                    at = self.last_scheduled_cycle + 1;
                }
                self.last_scheduled_cycle = at;
                self.play_queue.push_back((at, ns, level));
            }
        }

        self.holding = false;
        self.rollback_requested = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 16MHz: 16_000 cycles per millisecond.
    const MS: u64 = 16_000;
    const LATENCY: u64 = (REPLAY_LATENCY_NS / 1_000_000) * MS;
    const GAP: u64 = (BURST_GAP_NS / 1_000_000) * MS;

    fn engine() -> ReplayEngine {
        let mut engine = ReplayEngine::new();
        engine.set_clock_rate(16_000_000);
        engine
    }

    /// Arms a snapshot at `cycle` the way the real flow does: power
    /// cycles the receiver, polls the snapshot request, confirms.
    fn arm_snapshot(engine: &mut ReplayEngine, cycle: u64) {
        engine.receiver_power_changed(cycle.saturating_sub(1), false, true);
        engine.receiver_power_changed(cycle, true, true);
        assert!(matches!(
            engine.poll(cycle),
            RollbackDirective::TakeSnapshot
        ));
        engine.snapshot_taken(cycle);
    }

    #[test]
    fn bursts_replay_with_sender_relative_spacing() {
        let mut engine = engine();
        engine.push_incoming(1_000, 0, true);
        engine.push_incoming(1_000, 1_000_000, false); // 1ms later

        assert!(!engine.current_level(1_000 + LATENCY - 1));
        assert!(engine.current_level(1_000 + LATENCY));
        assert!(engine.current_level(1_000 + LATENCY + MS - 1));
        assert!(!engine.current_level(1_000 + LATENCY + MS));
    }

    #[test]
    fn a_gap_reanchors_the_next_burst_at_arrival() {
        let mut engine = engine();
        engine.push_incoming(0, 0, true);
        engine.push_incoming(0, 1_000_000, false);
        engine.current_level(LATENCY + 2 * MS);

        // The next frame is 100ms later in sender time but arrives at
        // local cycle 10M: it replays relative to its own arrival.
        engine.push_incoming(10_000_000, 100_000_000, true);
        assert!(!engine.current_level(10_000_000 + LATENCY - 1));
        assert!(engine.current_level(10_000_000 + LATENCY));
    }

    #[test]
    fn frame_arriving_while_receiver_is_off_rolls_back() {
        let mut engine = engine();
        arm_snapshot(&mut engine, 2_000);
        engine.receiver_power_changed(3_000, false, true);

        // A frame arrives mid retransmission, 100ms later.
        engine.push_incoming(1_600_000, 0, true);
        engine.push_incoming(1_600_000, 1_000_000, false);

        // It must not play live...
        assert!(!engine.current_level(1_600_000 + LATENCY + 1));
        // ...but request a rollback instead.
        assert!(matches!(
            engine.poll(1_600_100),
            RollbackDirective::RollBack
        ));
        assert!(engine.rolled_back(2_000, 1_600_100).is_none());

        // The frame replays early in the restored window.
        assert!(engine.current_level(2_000 + LATENCY));
        assert!(!engine.current_level(2_000 + LATENCY + MS));
    }

    #[test]
    fn receiver_off_mid_replay_retains_the_whole_frame() {
        let mut engine = engine();
        arm_snapshot(&mut engine, 2_000);

        engine.push_incoming(10_000, 0, true);
        engine.push_incoming(10_000, 1_000_000, false);
        engine.push_incoming(10_000, 2_000_000, true);
        engine.push_incoming(10_000, 4_000_000, false);

        // Half the frame has been heard when the window closes.
        assert!(engine.current_level(10_000 + LATENCY));
        engine.receiver_power_changed(10_000 + LATENCY + 3 * MS / 2, false, true);

        assert!(matches!(
            engine.poll(10_000 + LATENCY + 3 * MS / 2),
            RollbackDirective::RollBack
        ));
        engine.rolled_back(2_000, 10_000 + LATENCY + 3 * MS / 2);

        // The whole frame replays from its first edge.
        assert!(engine.current_level(2_000 + LATENCY));
        assert!(!engine.current_level(2_000 + LATENCY + MS));
        assert!(engine.current_level(2_000 + LATENCY + 2 * MS));
        assert!(!engine.current_level(2_000 + LATENCY + 4 * MS));
    }

    #[test]
    fn one_rollback_per_snapshot() {
        let mut engine = engine();
        arm_snapshot(&mut engine, 2_000);
        engine.receiver_power_changed(3_000, false, true);

        engine.push_incoming(1_600_000, 0, true);
        assert!(matches!(
            engine.poll(1_600_100),
            RollbackDirective::RollBack
        ));
        engine.rolled_back(2_000, 1_600_100);

        // Receiver goes off again without a new snapshot: a second late
        // frame plays live (unheard) instead of rolling back again.
        engine.receiver_power_changed(2_000 + LATENCY + 2 * MS, false, false);
        engine.push_incoming(3_200_000, 100_000_000, true);
        assert!(matches!(
            engine.poll(3_200_100),
            RollbackDirective::Continue
        ));
    }

    #[test]
    fn a_stale_snapshot_is_not_rolled_back_to() {
        let mut engine = engine();
        arm_snapshot(&mut engine, 0);
        engine.receiver_power_changed(1_000, false, true);

        // Over 2s later (the snapshot age cap) a frame arrives.
        let late = 40_000 * MS;
        engine.push_incoming(late, 0, true);
        assert!(matches!(engine.poll(late), RollbackDirective::Continue));
        // It plays live on its normal schedule.
        assert!(engine.current_level(late + LATENCY));
    }

    #[test]
    fn receiver_reopening_cancels_the_rollback() {
        let mut engine = engine();
        arm_snapshot(&mut engine, 2_000);
        engine.receiver_power_changed(3_000, false, true);

        engine.push_incoming(1_600_000, 0, true);
        engine.push_incoming(1_600_000, 1_000_000, false);

        // The firmware opens a new listen window before the run loop
        // polls: the held frame is delivered live into it instead.
        engine.receiver_power_changed(1_610_000, true, true);
        assert!(matches!(
            engine.poll(1_610_100),
            RollbackDirective::TakeSnapshot
        ));
        assert!(engine.current_level(1_610_000 + LATENCY));
    }

    #[test]
    fn completed_transmission_since_snapshot_blocks_rollback() {
        let mut engine = engine();
        arm_snapshot(&mut engine, 2_000);
        engine.receiver_power_changed(3_000, false, true);

        // A whole frame went out after the snapshot: the peer keeps it,
        // so its transmission must not be rewound.
        engine.outgoing_wire_ns(4_000, true);
        engine.outgoing_wire_ns(4_000 + MS, false);

        // The peer's frame arrives 100ms later, well past the burst gap.
        engine.push_incoming(1_600_000, 0, true);
        engine.push_incoming(1_600_000, 1_000_000, false);
        assert!(matches!(
            engine.poll(1_600_100),
            RollbackDirective::Continue
        ));

        // The held frame plays live (unheard) instead of rolling back.
        assert!(engine.current_level(1_600_100 + LATENCY));
        assert!(!engine.current_level(1_600_100 + LATENCY + MS));
    }

    #[test]
    fn transmission_in_progress_still_rolls_back() {
        let mut engine = engine();
        arm_snapshot(&mut engine, 2_000);
        engine.receiver_power_changed(3_000, false, true);

        // A frame is mid-transmission (carrier currently on) when the
        // peer's frame arrives: truncation recalls it.
        engine.outgoing_wire_ns(4_000, true);
        engine.outgoing_wire_ns(4_000 + MS / 2, false);
        engine.outgoing_wire_ns(4_000 + MS, true);

        engine.push_incoming(20_000, 0, true);
        engine.push_incoming(20_000, 1_000_000, false);
        assert!(matches!(engine.poll(20_100), RollbackDirective::RollBack));

        // The dangling carrier is closed for the peer, and the held
        // frame replays in the restored window.
        assert!(engine.rolled_back(2_000, 20_100).is_some());
        assert!(engine.current_level(2_000 + LATENCY));
    }

    #[test]
    fn carrier_off_defers_until_the_burst_gap_denies() {
        let mut engine = engine();
        arm_snapshot(&mut engine, 2_000);
        engine.receiver_power_changed(3_000, false, true);

        engine.outgoing_wire_ns(4_000, true);
        engine.outgoing_wire_ns(4_000 + MS, false);

        // The peer's frame arrives 1ms after our last edge: it is not
        // yet knowable whether our frame ended or is between bits.
        engine.push_incoming(4_000 + 2 * MS, 0, true);
        assert!(matches!(
            engine.poll(4_000 + 2 * MS),
            RollbackDirective::Continue
        ));
        // Deferred: the held frame is not scheduled yet.
        assert!(!engine.current_level(4_000 + 3 * MS));

        // The burst gap elapses with no further edges: the frame was
        // complete, so the rollback is denied and the frame plays live.
        let later = 4_000 + MS + GAP + 1;
        assert!(matches!(engine.poll(later), RollbackDirective::Continue));
        assert!(engine.current_level(later + LATENCY));
    }

    #[test]
    fn carrier_off_defers_until_the_frame_continues_and_grants() {
        let mut engine = engine();
        arm_snapshot(&mut engine, 2_000);
        engine.receiver_power_changed(3_000, false, true);

        engine.outgoing_wire_ns(4_000, true);
        engine.outgoing_wire_ns(4_000 + MS, false);

        engine.push_incoming(4_000 + 2 * MS, 0, true);
        assert!(matches!(
            engine.poll(4_000 + 2 * MS),
            RollbackDirective::Continue
        ));

        // The transmission resumes within the gap: it was mid-frame all
        // along, so the deferred rollback is granted.
        engine.outgoing_wire_ns(4_000 + 3 * MS, true);
        assert!(matches!(
            engine.poll(4_000 + 3 * MS + 100),
            RollbackDirective::RollBack
        ));
    }

    #[test]
    fn heard_frame_since_snapshot_blocks_rollback() {
        let mut engine = engine();
        arm_snapshot(&mut engine, 2_000);

        // A whole frame is heard live in the window: the firmware
        // consumed it, so it must not be un-heard.
        engine.push_incoming(10_000, 0, true);
        engine.push_incoming(10_000, 1_000_000, false);
        assert!(engine.current_level(10_000 + LATENCY));
        assert!(!engine.current_level(10_000 + LATENCY + MS));

        // The window closes, and a second frame arrives too late.
        engine.receiver_power_changed(10_000 + LATENCY + 2 * MS, false, true);
        engine.push_incoming(1_600_000, 100_000_000, true);
        assert!(matches!(
            engine.poll(1_600_100),
            RollbackDirective::Continue
        ));

        // It plays live instead of rewinding past the heard frame.
        assert!(engine.current_level(1_600_100 + LATENCY));
    }

    #[test]
    fn wire_time_is_monotonic_across_rollbacks() {
        let mut engine = engine();
        let before = engine.outgoing_wire_ns(1_600_000, true); // 100ms

        // Roll back 90ms with the carrier dangling on: the engine asks
        // for a closing edge at the abandonment point (nudged forward,
        // since wire time is strictly increasing).
        let close = engine.rolled_back(160_000, 1_600_000);
        assert_eq!(close, Some(before + 1));

        // Later outgoing edges never predate what was already sent.
        let after = engine.outgoing_wire_ns(320_000, true); // 20ms + offset
        assert!(after > before);
    }

    #[test]
    fn completed_burst_is_offered_for_atomic_resend() {
        let mut engine = engine();
        let a = engine.outgoing_wire_ns(4_000, true);
        let b = engine.outgoing_wire_ns(4_000 + MS, false);

        // Not before the burst gap has settled it...
        assert_eq!(engine.take_burst_resend(4_000 + MS + GAP), None);
        // ...but exactly once after.
        let copy = engine
            .take_burst_resend(4_000 + MS + GAP + 1)
            .expect("completed burst should be resent");
        assert_eq!(copy, vec![(a, true), (b, false)]);
        assert_eq!(engine.take_burst_resend(4_000 + MS + GAP + 2), None);

        // The next burst's copy contains only its own records.
        let c = engine.outgoing_wire_ns(4_000 + MS + 2 * GAP, true);
        let d = engine.outgoing_wire_ns(4_000 + 2 * MS + 2 * GAP, false);
        let later = 4_000 + 2 * MS + 3 * GAP + 1;
        assert_eq!(
            engine.take_burst_resend(later),
            Some(vec![(c, true), (d, false)])
        );
    }

    #[test]
    fn dangling_burst_is_not_resendable_while_open() {
        let mut engine = engine();
        engine.outgoing_wire_ns(4_000, true);
        // Carrier still on: no matter how long, the burst is not over.
        assert_eq!(engine.take_burst_resend(4_000 + 10 * GAP), None);
    }

    #[test]
    fn truncated_burst_is_not_resent_after_rollback() {
        let mut engine = engine();
        engine.outgoing_wire_ns(20_000, true);
        engine.outgoing_wire_ns(20_000 + MS, false);
        engine.outgoing_wire_ns(20_000 + 2 * MS, true);

        // Mid-frame rollback: the peer rejects the truncated frame, so
        // no copy of it should follow.
        let close = engine.rolled_back(2_000, 20_000 + 2 * MS + 100);
        assert!(close.is_some());
        assert_eq!(engine.take_burst_resend(2_000 + 10 * GAP), None);
    }

    #[test]
    fn burst_completed_before_snapshot_is_resent_after_rollback() {
        let mut engine = engine();
        let a = engine.outgoing_wire_ns(4_000, true);
        let b = engine.outgoing_wire_ns(4_000 + MS, false);

        // The rollback happens before any poll noticed the burst gap;
        // the burst predates the snapshot, so the peer still gets it.
        assert!(engine.rolled_back(2_000, 4_000 + 2 * MS).is_none());
        assert_eq!(
            engine.take_burst_resend(2_000),
            Some(vec![(a, true), (b, false)])
        );
    }

    #[test]
    fn copy_of_cleanly_replayed_burst_is_dropped() {
        let mut engine = engine();
        engine.push_incoming(1_000, 1_000, true);
        engine.push_incoming(1_000, 1_000 + 1_000_000, false);

        // The burst replays cleanly.
        assert!(engine.current_level(1_000 + LATENCY));
        assert!(!engine.current_level(1_000 + LATENCY + MS));

        // Its atomic copy arrives: a wire-time regression. Nothing may
        // replay again.
        let now = 1_000 + LATENCY + 2 * MS;
        engine.push_incoming(now, 1_000, true);
        engine.push_incoming(now, 1_000 + 1_000_000, false);
        for probe in 0..40 {
            assert!(
                !engine.current_level(now + probe * MS / 2),
                "duplicate burst replayed at probe {probe}"
            );
        }

        // The next real burst still replays normally.
        let next = now + 20 * MS;
        engine.push_incoming(next, 200_000_000, true);
        assert!(engine.current_level(next + LATENCY));
    }

    #[test]
    fn late_corrupted_burst_is_replaced_by_its_copy() {
        let mut engine = engine();
        // First edge on time; the rest of the burst arrives one network
        // stall (~50ms) late, far past the replay budget.
        engine.push_incoming(1_000, 1_000, true);
        assert!(engine.current_level(1_000 + LATENCY));
        let stalled = 1_000 + LATENCY + 50 * MS;
        engine.push_incoming(stalled, 1_000 + 1_000_000, false);
        engine.push_incoming(stalled, 1_000 + 2_000_000, true);
        engine.push_incoming(stalled, 1_000 + 3_000_000, false);
        engine.current_level(stalled + 1); // squashed, corrupt delivery

        // The atomic copy arrives (regression to the burst's first
        // timestamp) and replays whole, on a fresh anchor.
        let copy_at = stalled + 2 * MS;
        engine.push_incoming(copy_at, 1_000, true);
        engine.push_incoming(copy_at, 1_000 + 1_000_000, false);
        engine.push_incoming(copy_at, 1_000 + 2_000_000, true);
        engine.push_incoming(copy_at, 1_000 + 3_000_000, false);

        assert!(engine.current_level(copy_at + LATENCY));
        assert!(!engine.current_level(copy_at + LATENCY + MS));
        assert!(engine.current_level(copy_at + LATENCY + 2 * MS));
        assert!(!engine.current_level(copy_at + LATENCY + 3 * MS));
    }

    #[test]
    fn corrupted_delivery_does_not_veto_rollback() {
        let mut engine = engine();
        arm_snapshot(&mut engine, 2_000);

        // A burst whose middle missed its slot: the firmware saw
        // garbage, not a frame, so it must not count as consumed.
        engine.push_incoming(10_000, 0, true);
        assert!(engine.current_level(10_000 + LATENCY));
        let stalled = 10_000 + LATENCY + 50 * MS;
        engine.push_incoming(stalled, 1_000_000, false);
        engine.push_incoming(stalled, 2_000_000, true);
        engine.push_incoming(stalled, 3_000_000, false);
        engine.current_level(stalled + 1);

        // The window closes; the next frame arrives too late and must
        // still be able to roll back past the garbage.
        engine.receiver_power_changed(stalled + 2 * MS, false, true);
        engine.push_incoming(stalled + 20 * MS, 100_000_000, true);
        assert!(matches!(
            engine.poll(stalled + 20 * MS + 100),
            RollbackDirective::RollBack
        ));
    }

    #[test]
    fn late_final_edge_alone_is_not_corruption() {
        let mut engine = engine();
        arm_snapshot(&mut engine, 2_000);

        // The data edge replays on time; only the closing carrier-off
        // misses its slot. The firmware may have decoded past the
        // trailing carrier already, so the frame counts as consumed.
        engine.push_incoming(10_000, 0, true);
        assert!(engine.current_level(10_000 + LATENCY));
        let stalled = 10_000 + LATENCY + 50 * MS;
        engine.push_incoming(stalled, 1_000_000, false);
        assert!(!engine.current_level(stalled + 1));

        // The atomic copy is dropped, not replayed as a duplicate...
        engine.push_incoming(stalled + MS, 0, true);
        engine.push_incoming(stalled + MS, 1_000_000, false);
        assert!(!engine.current_level(stalled + MS + LATENCY + 1));

        // ...and a later rollback may not rewind past the frame.
        engine.receiver_power_changed(stalled + 2 * MS, false, true);
        engine.push_incoming(stalled + 20 * MS, 100_000_000, true);
        assert!(matches!(
            engine.poll(stalled + 20 * MS + 100),
            RollbackDirective::Continue
        ));
    }

    #[test]
    fn split_copy_replays_whole_once_complete() {
        let mut engine = engine();
        engine.push_incoming(1_000, 1_000, true);
        assert!(engine.current_level(1_000 + LATENCY));
        let stalled = 1_000 + LATENCY + 50 * MS;
        engine.push_incoming(stalled, 1_000 + 1_000_000, false);
        engine.push_incoming(stalled, 1_000 + 2_000_000, true);
        engine.push_incoming(stalled, 1_000 + 3_000_000, false);
        engine.current_level(stalled + 1); // squashed, corrupt delivery

        // The copy itself arrives split by another stall: its first
        // half must not replay by itself.
        let first_half = stalled + 2 * MS;
        engine.push_incoming(first_half, 1_000, true);
        engine.push_incoming(first_half, 1_000 + 1_000_000, false);
        assert!(!engine.current_level(first_half + LATENCY + 1));

        // The rest lands 50ms later; the whole frame replays from
        // there, intact.
        let second_half = first_half + 50 * MS;
        engine.push_incoming(second_half, 1_000 + 2_000_000, true);
        engine.push_incoming(second_half, 1_000 + 3_000_000, false);
        assert!(engine.current_level(second_half + LATENCY));
        assert!(!engine.current_level(second_half + LATENCY + MS));
        assert!(engine.current_level(second_half + LATENCY + 2 * MS));
        assert!(!engine.current_level(second_half + LATENCY + 3 * MS));
    }

    #[test]
    fn incomplete_copy_is_abandoned() {
        let mut engine = engine();
        engine.push_incoming(1_000, 1_000, true);
        assert!(engine.current_level(1_000 + LATENCY));
        let stalled = 1_000 + LATENCY + 50 * MS;
        engine.push_incoming(stalled, 1_000 + 1_000_000, false);
        engine.push_incoming(stalled, 1_000 + 2_000_000, true);
        engine.push_incoming(stalled, 1_000 + 3_000_000, false);
        engine.current_level(stalled + 1);

        // The copy starts but its tail was dropped in transit; the
        // stream moving past it must discard it, not wedge on it.
        let copy_at = stalled + 2 * MS;
        engine.push_incoming(copy_at, 1_000, true);
        engine.push_incoming(copy_at, 1_000 + 1_000_000, false);

        let next = copy_at + 20 * MS;
        engine.push_incoming(next, 200_000_000, true);
        engine.push_incoming(next, 201_000_000, false);
        assert!(engine.current_level(next + LATENCY));
        assert!(!engine.current_level(next + LATENCY + MS));
    }

    #[test]
    fn held_frame_survives_newer_arrivals() {
        let mut engine = engine();
        arm_snapshot(&mut engine, 2_000);
        engine.receiver_power_changed(3_000, false, true);

        // Frame A arrives unheard and is held complete...
        engine.push_incoming(1_600_000, 0, true);
        engine.push_incoming(1_600_000, 1_000_000, false);
        // ...and its retransmission B (100ms later in sender time, 2ms
        // long) arrives while the rollback is still pending. B is
        // redundant — A will replay whole — and is discarded; holding
        // both would replay A, then 100ms of silence, then B, far past
        // any listen window.
        engine.push_incoming(1_602_000, 100_000_000, true);
        engine.push_incoming(1_602_000, 102_000_000, false);

        assert!(matches!(
            engine.poll(1_602_100),
            RollbackDirective::RollBack
        ));
        engine.rolled_back(2_000, 1_602_100);

        // The complete held frame A replays into the restored window:
        // 1ms of carrier, not B's 2ms.
        assert!(engine.current_level(2_000 + LATENCY));
        assert!(!engine.current_level(2_000 + LATENCY + MS));

        // B's atomic copy was discarded along with B.
        engine.push_incoming(2_000 + LATENCY + 2 * MS, 100_000_000, true);
        engine.push_incoming(2_000 + LATENCY + 2 * MS, 102_000_000, false);
        for probe in 0..40 {
            assert!(
                !engine.current_level(2_000 + LATENCY + 2 * MS + probe * MS / 2),
                "discarded burst replayed at probe {probe}"
            );
        }

        // The peer's next frame, past the burst gap, plays normally.
        let next = 2_000 + LATENCY + 40 * MS;
        engine.push_incoming(next, 200_000_000, true);
        assert!(engine.current_level(next + LATENCY));
    }

    #[test]
    fn copy_arriving_while_holding_is_dropped() {
        let mut engine = engine();
        arm_snapshot(&mut engine, 2_000);
        engine.receiver_power_changed(3_000, false, true);

        // The whole burst arrives with the receiver off and is held.
        engine.push_incoming(1_600_000, 1_000, true);
        engine.push_incoming(1_600_000, 1_000 + 1_000_000, false);
        // Its atomic copy arrives before the rollback executes: the
        // held burst is already complete, so the copy adds nothing.
        engine.push_incoming(1_600_200, 1_000, true);
        engine.push_incoming(1_600_200, 1_000 + 1_000_000, false);

        assert!(matches!(
            engine.poll(1_600_300),
            RollbackDirective::RollBack
        ));
        engine.rolled_back(2_000, 1_600_300);

        // Exactly one replay of the frame, not two back to back.
        assert!(engine.current_level(2_000 + LATENCY));
        assert!(!engine.current_level(2_000 + LATENCY + MS));
        for probe in 1..40 {
            assert!(
                !engine.current_level(2_000 + LATENCY + MS + probe * MS / 2),
                "held burst replayed twice at probe {probe}"
            );
        }
    }
}
