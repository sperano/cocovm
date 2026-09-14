# UI and remaining-core performance methods

The core comparison uses base commit `4d80867b361d5aef184cc8c050f7948c8670339d`
and the feature working tree based on that commit. Measurements ran before the
final commit, so branch metadata is marked dirty. The host was a Mac15,8 with an
Apple M3 Max, 64 GB of memory, macOS 26.6.2, and Rust 1.97.0.

## Profile and measure the core

Build the release core harness, then capture clean throughput separately from
sampling overhead:

```sh
cargo build --release -p coco-core --example perf_baseline
python3 scripts/perf/baseline.py core \
  --scenario basic-idle --scenario graphics \
  --scenario dac --scenario cartridge \
  --repeats 3 --output /tmp/task229-core
python3 scripts/perf/baseline.py core \
  --scenario basic-idle --scenario graphics \
  --scenario dac --scenario cartridge \
  --sample-profile --repeats 1 \
  --output /tmp/task229-core-profiles
```

Each run uses the default three-second warmup and ten-second measurement. The
published comparison reports medians and full ranges. One initial after-change
DAC set ran at only 65% to 85% CPU and is excluded; a clean three-run replacement
at approximately 100% CPU is retained. No compiler or test ran during retained
captures.

The address-mask regression gates cover every configured RAM size, the bus map,
GIME rendering, scanline splits, fixed-field machine behavior, snapshot lockstep,
audio grids, cartridge audio, and CPU timing through the focused and workspace
test suites.

## Attribute UI operations

The native harness now records the caller-thread duration and success or error
for every periodic operation. Reproduce the lifecycle and RAM-only snapshot
measurements with:

```sh
cargo build --release -p coco-egui --features perf
python3 scripts/perf/baseline.py native \
  --scenario lifecycle --duration 60 --repeats 3 \
  --keep-foreground --output /tmp/task229-lifecycle
python3 scripts/perf/baseline.py native \
  --scenario snapshot --duration 10 --repeats 3 \
  --keep-foreground --output /tmp/task229-snapshot
python3 scripts/perf/report.py /tmp/task229-lifecycle
python3 scripts/perf/report.py /tmp/task229-snapshot
```

The lifecycle actions include synchronous audio-device construction and stream
destruction. The durations identify ownership costs but do not isolate individual
CPAL calls. Snapshot fixtures use maximum standard CoCo 3 RAM but no mounted disk,
VHD, DriveWire image, or slow filesystem.

## Measure printer export

The export comparison uses a deterministic 32-page sparse roll at 144 dpi. Each
page contains one short ink line followed by 65 blank line feeds. The base probe
executes the removed caller-thread flow: rasterize the entire roll or all PDF
pages, then encode. The branch probe submits the same immutable paper snapshot to
the production background exporter and waits outside the timed submission path.

Run all modes with:

```sh
cargo build --release -p coco-egui --example paper_export_perf
for mode in legacy-pdf streamed-pdf legacy-roll-png streamed-roll-png async-pdf \
  snapshot dense-row-snapshot dense-dot-snapshot; do
  target/release/examples/paper_export_perf "$mode"
done
```

Three retained legacy runs block for 315.2 to 317.7 ms for PDF and 192.4 to
193.6 ms for whole-roll PNG. A page raster is 1,368 by 1,584 RGBA8 pixels, or
8.27 MiB. The base 32-page raster set therefore retains at least 264.52 MiB;
whole-roll PNG clones that buffer. Single-run process RSS peaks were 277.4 MiB
for legacy PDF and 536.1 MiB for legacy PNG. The same implementation would
require 16.14 GiB for a 2,000-page roll before the PNG clone.

The final branch command and submission/end-to-end results are recorded in
[the results](RESULTS.md). These timings are CPU duration on one host. They do
not represent slow storage, and filesystem caches were not flushed between runs.

The snapshot limit is checked before cloning on the caller thread. The dense-row
fixture puts one dot in each of 110,000 B-tree rows and estimates 15,840,552 bytes
of owned storage, close to the 16 MiB production limit and slower to clone than a
dense single-row vector. The dense-dot fixture puts 2,000,000 dots in one row and
estimates 8,389,288 bytes. Both setup phases are outside the measured interval.
The renderer consumes dots through a visitor, so neither fixture produces a
second page-local coordinate vector during export.

## Measurement boundaries

The available harness directly covers RAM-only snapshot round trips, saved-
preview decoding, printer scrolling and export, lifecycle transitions, and
configuration writes that occur within lifecycle operations. It does not yet
create large disk, VHD, DriveWire, cassette, or photo inputs; inject storage
latency; force configuration-save failures; record debugger-open costs; observe
GPU execution; or measure input-to-photon latency. These are unsupported
scenarios, not assumed successes.

RAM-only snapshot save and restore stay below the 16.7 ms foreground-frame
budget, so this work does not add a full-machine background copy. Machine state
uses main-thread-only ownership, and mounted media would require an explicit
freeze or immutable prepared boundary. Moving it without those semantics would
risk an inconsistent snapshot. Configuration retry scheduling likewise remains
unchanged because no measured configuration stall or retry storm is available.
