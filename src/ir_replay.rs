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

    /// Added to outgoing timestamps; grows by the rewound span on each
    /// rollback so wire time never runs backwards.
    wire_offset_ns: u64,
    last_sent_level: bool,
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
            wire_offset_ns: 0,
            last_sent_level: false,
        }
    }

    pub fn set_clock_rate(&mut self, clock_rate: u64) {
        if clock_rate > 0 {
            self.clock_rate = clock_rate;
        }
    }

    /// Forgets all link state (edges, holds, snapshot arming). Called on
    /// reconnect or re-pairing; the wire offset survives so outgoing
    /// timestamps stay monotonic regardless.
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
        self.last_sent_level = false;
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
        let new_burst = match self.last_sender_ns {
            None => true,
            Some(previous) => sender_ns.saturating_sub(previous) > BURST_GAP_NS,
        };
        self.last_sender_ns = Some(sender_ns);

        if new_burst && !self.holding {
            self.delivered_burst.clear();
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
        let mut at = anchor_cycle + self.ns_to_cycles(sender_ns.saturating_sub(anchor_ns));
        if at <= self.last_scheduled_cycle {
            at = self.last_scheduled_cycle + 1;
        }
        self.last_scheduled_cycle = at;
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

    /// The run loop's regular poll; see `crate::ir::IrRollbackControl`.
    pub fn poll(&mut self, now_cycle: u64) -> RollbackDirective {
        if self.rollback_requested {
            if self.snapshot_is_fresh(now_cycle) {
                // One rollback per snapshot: re-arming requires a new
                // receiver-on snapshot. `holding` stays set so edges of
                // the held burst that are still arriving keep
                // accumulating until `rolled_back` schedules them.
                self.armed_snapshot_cycle = None;
                self.rollback_requested = false;
                return RollbackDirective::RollBack;
            }
            // Too stale to rewind; let the frame play (unheard) and rely
            // on the peer's retransmissions.
            self.deliver_held(now_cycle);
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
        let close_ns = self.cycles_to_ns(abandoned_cycle) + self.wire_offset_ns;
        self.wire_offset_ns += self.cycles_to_ns(rewound);

        let close = if self.last_sent_level {
            self.last_sent_level = false;
            Some(close_ns)
        } else {
            None
        };

        // Snapshots are only taken while the receiver is on.
        self.rx_powered = true;
        self.deliver_held(restored_cycle);
        close
    }

    /// Timestamps an outgoing edge, applying the rollback offset.
    pub fn outgoing_wire_ns(&mut self, cycle: u64, level: bool) -> u64 {
        self.last_sent_level = level;
        self.cycles_to_ns(cycle) + self.wire_offset_ns
    }

    /// Schedules the held edges to replay starting `REPLAY_LATENCY_NS`
    /// after `start_cycle`, dropping whatever else was scheduled (the
    /// sender's retransmissions cover anything lost).
    fn deliver_held(&mut self, start_cycle: u64) {
        self.play_queue.clear();
        self.level = false;
        self.delivered_burst.clear();
        self.last_scheduled_cycle = start_cycle;

        let held = std::mem::take(&mut self.held);
        if let Some(&(first_ns, _)) = held.first() {
            let anchor_cycle = start_cycle + self.ns_to_cycles(REPLAY_LATENCY_NS);
            self.anchor = Some((first_ns, anchor_cycle));
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
    fn wire_time_is_monotonic_across_rollbacks() {
        let mut engine = engine();
        let before = engine.outgoing_wire_ns(1_600_000, true); // 100ms

        // Roll back 90ms with the carrier dangling on: the engine asks
        // for a closing edge at the abandonment point.
        let close = engine.rolled_back(160_000, 1_600_000);
        assert_eq!(close, Some(before));

        // Later outgoing edges never predate what was already sent.
        let after = engine.outgoing_wire_ns(320_000, true); // 20ms + offset
        assert!(after >= before);
    }
}
