# Reproduce the audio buffer comparison

Use the [baseline harness](../../README.md) with installed application assets.
The workloads generate DAC and Orchestra-90 input from checked-in fixtures.
No ROM or private media is included in these results.

## Build and capture

Build each source revision in its own checkout and retain both binaries:

```sh
cargo build --release -p coco-egui --features perf --bin cocovm
cargo build --release -p coco-core --example perf_baseline
```

Set `CARGO_TARGET_DIR` to a directory containing that revision's
`release/cocovm` and `release/examples/perf_baseline` to select retained binaries.
Use a distinct, absent output directory for each capture:

```sh
python3 scripts/perf/baseline.py native --scenario dac --scenario cartridge --scenario background --scenario multi-vm --scenario snapshot --scenario lifecycle --keep-foreground --output /tmp/audio-native
python3 scripts/perf/baseline.py core --scenario dac --scenario cartridge --output /tmp/audio-core
python3 scripts/perf/report.py /tmp/audio-native
python3 scripts/perf/report.py /tmp/audio-core
```

Defaults are 3 s warmup, 10 s measurement, and three fresh-process repeats.
For alternating revision order, use `--repeats 1` and alternate the retained
binary directory between captures. Keep compiler, test, and profiler processes
stopped during measurement. The native harness opens isolated windows, uses the
default audio device, and activates Finder for the background case.

For the counter-disabled control, repeat both core scenarios on each revision:

```sh
python3 scripts/perf/baseline.py core --scenario dac --scenario cartridge --repeats 1 --no-allocations --output /tmp/audio-core-no-counts
```

The main native matrix uses three repeats. A later foreground recheck uses the
same native command with only `--scenario dac --scenario cartridge --repeats 1`
on each revision. The core comparison alternates before/after, after/before,
and before/after, with both workloads in each one-repeat capture.

## Isolate producer and callback allocation traffic

The ignored measurement compares the previous allocating producer with the
reused-buffer producer in the same executable. It checks identical output
checksums after warming both paths. It also exercises the production callback
with full and empty queues, including enabled timing telemetry:

```sh
cargo test -p coco-egui --features perf,debug-ui audio_steady_state_allocation_measurement -- --ignored --test-threads=1 --nocapture
```

Run this test alone. Allocation counters cover the process. Each case runs
2000 iterations after warmup. Producer batches contain 1048 stereo frames at
62,866 Hz and target 48,000 Hz. Callback batches contain 256 stereo frames.
The development profile is sufficient for allocation counts and sample checks;
its elapsed times do not establish release performance.

## Interpret the measurements

Native allocation totals include the renderer, UI, and every VM. Compare both
whole-process totals and the isolated audio measurement. Headless runs drain
core output without a device and measure unpaced field throughput.

Callback lock-wait and lock-hold percentiles are fixed-histogram upper bounds.
The added hold span measures callback filling and mutex release; it adds timing
work only in a `perf` build with recording enabled. Existing wait, underrun,
overflow, and queue-depth telemetry remains in place. Queue depth divided by
the device rate estimates queued playback duration, excluding OS/device buffering.
It does not measure acoustic or input-to-output latency.

Snapshot and lifecycle workloads deliberately reset or stop playback. Their
missing-frame counts include intentional silence and discontinuities. Assess
steady running audio separately. Focus observations establish backend focus,
not physical occlusion. Short captures characterize the sampled host workload;
they cannot guarantee glitch-free playback on every device or under every host stall.

## Storage and behavior

Core event storage retains its allocation after each scanline. Replay order,
slot assignment, and final-slot tail state use the existing algorithm. Producer
scratch starts with 16,384 source frames and 250 ms of device frames plus one
resampler frame. Larger observed batches can grow scratch capacity, which later
batches reuse. Existing core output and field catch-up limits remain in place.
At 44,100 Hz, initial scratch requests 219,280 bytes per enabled output, and the
queue requests 88,200 bytes. Disabled outputs allocate no producer scratch.

The queue retains at most 250 ms of device frames. It discards the oldest queued
frames and, for an oversized batch, the oldest incoming frames before appending.
This preserves the previous append-then-truncate result without temporary queue
growth. Reset clears samples and filter state while retaining storage and the
volume/mute controls. The mutex remains subject to measurement rather than being
replaced speculatively.
