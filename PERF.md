# Performance Log

Speed-up work targeting desktop (x86-64) and eventually embedded (ESP32) use.

## Benchmark setup

- Command: `cargo run -r -- firmware/OTP.dat "firmware/Yasmin 1.09.03.dat" --bench 30`
- Headless: null screen/GPIO/audio backends (`src/bench.rs`); audio stub still
  consumes PSG samples at 44.1 kHz cadence so that work is not skipped.
- Emulated CPU clock: 8 MHz (16 MHz oscillator / 2). "Speed" = emulated time /
  host time; 1.0x = real device speed.
- The benchmark is deterministic: `--verify` hash-chains every executed
  instruction's registers + start cycle. Implementations that report the same
  verify_hash executed identical instruction streams with identical timing.
- Host: AMD Ryzen 9 5900X, rustc 1.96.0, `--release`.

## Results

Primary benchmark: `Yasmin 1.09.03.dat`, 30 emulated seconds (boots to menu,
then mostly idles in WAI).

| Phase | Speed (30 emu-sec) | verify_hash (30s) | ram_hash |
|---|---|---|---|
| 0: Baseline | 5.4x realtime | `68c4294775bbe628` | `c1282cee7f110734` |
| 1: Event-driven peripherals | 87.5x realtime | `68c4294775bbe628` | `c1282cee7f110734` |
| 2: Static machine dispatch | 96x realtime | `68c4294775bbe628` | `c1282cee7f110734` |
| 3: Decode cache + fetch TLB | 111x realtime | `68c4294775bbe628` | `c1282cee7f110734` |
| 4: Fused dispatch + fetch slots | 168x realtime | `68c4294775bbe628` | `c1282cee7f110734` |
| 5: Instruction chaining | 180x realtime | `68c4294775bbe628` | `c1282cee7f110734` |

Cross-check on a busy in-game workload (`save.dat`, 20 emu-sec, ~2.1M
executed instructions per emulated second — worst-case-like load):

| Build | Speed | verify_hash (20s) |
|---|---|---|
| Baseline (HEAD + bench harness) | 7.4x realtime | `38f721b9d8ff7c01` |
| Phases 0-3 | 26x realtime | `38f721b9d8ff7c01` |
| Phase 4 | 43x realtime (~89M instr/sec) | `38f721b9d8ff7c01` |
| Phase 5 | 48.5x realtime (~101M instr/sec) | `38f721b9d8ff7c01` |

`Dash 1.09.03.dat` verify_hash also matches baseline (`e482bdcced75cebc`).
Every phase is cycle-exact: identical instruction streams at identical
cycles, verified per instruction.

### Phase 0: Baseline (benchmark harness only)

- 3 runs: 5.52x / 5.41x / 5.39x realtime (host 5.43-5.56 s for 30 emu-sec).
- 200,894,122 steps for 240,000,000 cycles → 1.19 cycles/step: the firmware
  idles in WAI most of the time, and each WAI step advances only 1 cycle while
  still paying the full per-step peripheral-update cost.
- Extrapolation: ~5.4x on a ~4.8 GHz desktop core means an ESP32 at 240 MHz
  would run well below realtime with this architecture.

### Phase 1: Event-driven peripheral scheduling — 16x faster than baseline

- 3 runs: 87.8x / 88.4x / 86.5x realtime. verify_hash identical to baseline,
  i.e. the same instructions executed at the same cycles.
- What changed:
  - Timers T0-T3 are closed-form (counter = base + elapsed/divisor) instead of
    ticking in a per-cycle loop; counter reads compute lazily; overflow cycles
    are precomputed.
  - Base timer, RTC, audio sampling, and timers publish a "next event cycle";
    the per-instruction hot path is one `cycles >= next_event` compare.
    Peripheral register writes that can move an event set a dirty flag.
  - WAI fast-forwards straight to the next event instead of stepping 1 cycle
    at a time (the firmware idles in WAI most of the time; this is why baseline
    steps/sec barely moved while wall speed rose 16x).
  - GPIO inputs are polled every 8000 cycles (~1 ms emulated) instead of every
    instruction. Only deliberate timing divergence: button transitions are
    detected with up to ~1 ms emulated latency (was: next instruction), below
    human perception; with constant inputs behavior is bit-identical.
  - Interrupt dispatch is gated on a cheap `shadow_ireq != 0` check.

### Phase 2: Static machine-space dispatch — +10%

- 3 runs: 94.9x / 95.9x / 97.4x realtime. Hashes identical.
- `St2205uAddressSpace` (and `Mcu`) are now generic over the machine address
  space instead of holding `Box<dyn AddressSpace>`, so every banked memory
  access inlines the whole chain (bank math → region select → flash/OTP array
  index) with no virtual call.
- The modest gain confirms the remaining bottleneck is re-decoding every
  instruction from scratch, not memory dispatch. A separate data-read page
  table was considered and skipped: the decode cache (Phase 3) needs the same
  virtual→physical resolution, and data reads mostly hit RAM/zero-page which
  is already a direct match arm.

### Phase 3: Decode cache + fetch TLB — +15% idle, 2x+ busy workloads

Instructions are decoded once and cached; the hot fetch path is a window
check, a page-crossing check, and an array load.

- Cache keyed by *device-local* address (flash offset / OTP offset via
  `AddressSpace::code_cache_key_range`), so bank switches invalidate nothing
  and aliased mappings share entries.
- Flash rewrites: the flash reports exactly which range a program/erase
  changed (`take_content_change`); only overlapping 4K cache pages drop.
  A global-generation scheme was tried first and benchmarked ~0: this
  firmware programs flash ~4000 bytes/sec during saves, invalidating
  everything continuously.
- Self-modifying RAM code (the firmware runs its flash-programming loop from
  RAM and patches its operands in place, ~116K fetches/emu-sec): RAM entries
  store the raw bytes they decoded from and re-verify against RAM on each
  hit (1-3 byte compare). Only the actually-patched instruction re-decodes;
  RAM writes need no cache bookkeeping.
- Uncacheable by construction (always decoded fresh): fetches from hardware
  registers, the flash status address while status-polling mode is active,
  and instructions spanning a bank-window or cache-page boundary.
- One-entry fetch "TLB" caches the resolved bank window (start/end/kind);
  invalidated on bank register writes, interrupt bank switches (PRR↔IRR),
  and flash content changes. Fetch hit path: 2 compares + add + cache load.
- `FetchedInstruction` (12 B, length precomputed) replaces the 24 B
  `DecodedInstruction` on the hot path.
- Fetch stats on the busy save.dat run: 40.6M cached / 0.77M decode+insert /
  0.30M uncacheable.

### Phase 4: Fused dispatch + per-window fetch slots — busy workload 1.6x

Three changes, applied in sequence on the `perf-experiments` branch:

- Fetch hot path fully inlined into `Core::step`; miss/refill/uncached
  paths outlined as `#[cold]` functions; cache pages became fixed-size
  arrays behind thin pointers (26x -> 29.4x on save.dat).
- Fused dispatch: a 256-entry handler table indexed by the raw opcode
  byte, one monomorphic function per (operation, addressing mode) pair.
  Instruction implementations are unchanged: handlers rebuild the
  `AddressingMode` from a raw u16 payload, and inlining folds all the
  addressing-mode matches away. The table is built at `Core` construction
  by decoding every opcode byte, so the decoder stays the single source
  of truth. Cache entries shrank from 12 to 8 bytes (29.4x -> 41.5x).
- Fetch TLB grew from one entry to one slot per 8K window, with separate
  halves for normal/interrupt mode (PRR vs IRR mapping coexist), so
  interrupt entry/exit and window-to-window jumps invalidate nothing;
  bank register writes invalidate only their own window's slots. Data
  reads/writes inline their hot register/RAM arms and outline the banked
  path (41.5x -> 43x).

### Phase 5: Instruction chaining between events — busy +13%

Interrupts can only become pending inside `process_events`, so between
two event boundaries `Mcu::run` executes instructions back-to-back with
no per-instruction WAI or interrupt-dispatch checks. Straight-line code
also fetches sequentially by decode-cache key (`key + length`), skipping
window resolution entirely; a streak breaks on any branch, 8K virtual
boundary, or fetch-slot invalidation (generation counter). The run loop
takes a per-instruction observer closure, so `--verify` hashes the exact
chained execution path (and confirmed identical hashes *and* instruction
counts); the GUI/web loops pass a no-op that compiles away.

## Where this leaves the ESP32 goal

Busy-workload cost is ~47 host cycles per emulated instruction on a Zen 3
core at ~4.8 GHz (~48.5x realtime, ~101M instr/sec). Realtime needs ~2.1M
instr/sec, so a 240 MHz ESP32 has a ~115-cycle budget — the desktop cycle
count is now well inside it, making realtime on ESP32-class hardware
plausible-to-likely even accounting for Xtensa's lower IPC; the idle path
has ~4x further headroom. Remaining desktop cost is diffuse (handler
bodies, cache loads, the run loop itself); the next wins are likely
ESP32-specific rather than algorithmic: cap decode-cache pages with LRU
eviction (entries are 8 B/byte, ~32 KB per hot 4K code page), pin hot
state in internal SRAM, and map the flash image XIP with a RAM overlay
for rewritten sectors.

## Reproducing

```
cargo build -r
./target/release/emiu2 firmware/OTP.dat "firmware/Yasmin 1.09.03.dat" --bench 30 --verify
./target/release/emiu2 firmware/OTP.dat firmware/save.dat --bench 20 --verify
```
