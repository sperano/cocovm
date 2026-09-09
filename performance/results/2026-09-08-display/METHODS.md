# Reproduce the display comparison

Build each revision in its own checkout with installed application assets:

```sh
cargo build --release -p coco-egui --features perf --bin cocovm
```

Use the [performance harness](../../README.md) from that checkout. It records the
checkout's revision, so a retained binary from another revision needs separate
provenance. Each output directory must be absent before capture:

```sh
python3 scripts/perf/baseline.py native --scenario basic-idle --scenario graphics --scenario paused --scenario suspended --scenario multi-vm --scenario tv --scenario snapshot --scenario lifecycle --keep-foreground --repeats 3 --output /tmp/display-native
python3 scripts/perf/baseline.py native --scenario paused --scenario suspended --scenario graphics --scenario multi-vm --display tv --keep-foreground --repeats 3 --output /tmp/display-tv
python3 scripts/perf/report.py /tmp/display-native
python3 scripts/perf/collect.py /tmp/display-native native.json
```

The final recheck uses `--scenario basic-idle --scenario tv --repeats 1` on the
before revision and `--scenario tv --repeats 1` on the after revision, both with
`--keep-foreground` and separate output directories. The before BASIC result
replaces the initial, potentially contaminated capture in the comparison tables.
TV recheck measurements remain separate from the three-repeat matrix.

Run `python3 performance/results/2026-09-08-display/summarize.py` to regenerate
the detailed comparison tables from the collected JSON files.

Defaults are 3 s warmup and 10 s measurement in each fresh process. Stop compilers,
tests, and profilers during captures. The harness isolates configuration, generates
fixtures, and uses the default audio device. No ROMs or personal media are published.

## Isolate TV processing

The ignored release measurement compares the previous allocating pass composition
with the reusable processor in one executable. Each case warms the lookup tables
and buffers, then processes 120 frames at a fixed seed. Run it alone because
allocation counters cover the process. Repeat the command three times to
record run-to-run timing variation:

```sh
cargo test --release -p coco-egui --features perf tv_steady_state_allocation_measurement -- --ignored --test-threads=1 --nocapture
```

The test checks identical pixel output and zero steady-state allocations in the
reusable processor. Color and black-and-white TV cases cover scanlines enabled
and disabled. Deterministic correctness tests additionally cover noise strengths,
seeds, representative widths, and changes in size and settings.

## Presentation behavior and retained storage

Each VM retains the last presented source pixels and compares them with the core
framebuffer before conversion. The cache key includes source width, display type,
effective scanline and noise settings, and animation tick. Pixel comparison also
covers source height and partial scanline changes without hashing collisions or
changes to the core's public framebuffer interface.

Memory and register changes become visible when the core renders them. Debugger
steps, reset, power-cycle, and snapshot restoration follow that same boundary.
A write that has not changed framebuffer pixels does not require an upload.
Window resizing, aspect correction, and overscan affect drawing geometry or UV
coordinates and reuse the texture. Switching displays updates texture filtering.

TV noise advances on a 60 Hz wall-clock grid, including paused and suspended
windows. Missed ticks are skipped. Unrelated repaints within a tick use the same
seed. When every viewport reports unfocused, animation requests preserve the
existing 100 ms background interval. The broader emulation/repaint scheduler
retains its existing behavior.

Static paused or suspended displays request no animation deadline. TVs with
noise enabled request deadlines and therefore do more work than the former
paused TV, whose noise advanced only when some other event repainted it. Compare
these cases separately. Native TV fixtures use default noise; noise-disabled
static TV invalidation is covered by deterministic frontend tests.

The source cache needs one RGBA framebuffer. TV processing retains one signal
buffer and one output buffer. The output buffer holds black-and-white luma input
and then the expanded scanlines. Buffers retain their largest observed capacity
until the VM is dropped. At 640 × 240, the source cache retains 600 KiB.
TV signal and doubled output need another 1800 KiB at that geometry. Allocation
capacity can exceed logical buffer length after a size change.
At fixed geometry, the TV processor does not allocate
after warmup. A changed texture still needs an owned `ColorImage` for egui.

## Measurement boundaries

Display conversion spans cover TV processing and `ColorImage` construction on
cache misses. Cache comparison runs outside that span and remains included in
VM UI time. Texture counts and bytes measure CPU enqueue requests, not actual
GPU transfers or GPU execution time. The backend can consolidate requests.

Whole-process allocations include UI, audio, rendering, and control work.
Retained RSS includes framework allocations and intentionally retained buffers.
Compare field progression and audio measurements alongside presentation counts.
Snapshot and lifecycle workloads include intentional playback resets and silence.

Focus observations describe backend focus, not occlusion or viewport-to-panel
mapping. Short captures cannot establish input-to-photon latency or reliability
under every host load. GPU timing and unavailable display information must remain
explicitly unavailable.
