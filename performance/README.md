# Measure CocoVM performance

Use these baselines before changing the renderer, audio pipeline, repaint scheduling,
resource management, or synchronous host operations. The harness separates native
app measurements from unpaced headless core throughput. Performance comparisons
have no absolute CI timing assertions.

The [September 8, 2026 baseline](results/2026-09-08/RESULTS.md) publishes 92 runs,
profiler summaries, measurement limits, and targets for subsequent optimizations.
The [audio buffer comparison](results/2026-09-08-audio/RESULTS.md) measures the
first audio allocation optimization against that baseline.
The [display comparison](results/2026-09-08-display/RESULTS.md) measures
presentation caching and reusable TV-processing buffers.
The [scheduling comparison](results/2026-09-09-scheduling/RESULTS.md) measures
deadline scheduling, per-window focus policies, and audio continuity. See
[Schedule frontend work](scheduling.md) for the timing and queue budgets.

## Build and run

Install the application assets first. The core example resolves `coco3.rom` through
`cocovm-test-assets`. Synthetic DAC, graphics, and Orchestra-90 programs are included
in source. No copyrighted ROMs, manuals, or media belong in a results directory.

```sh
cargo build --release -p coco-core --example perf_baseline
cargo build --release -p coco-egui --features perf --bin cocovm
python3 scripts/perf/baseline.py core --output /tmp/cocovm-core-baseline
python3 scripts/perf/baseline.py native --output /tmp/cocovm-native-baseline
python3 scripts/perf/report.py /tmp/cocovm-core-baseline
python3 scripts/perf/report.py /tmp/cocovm-native-baseline
```

The default is three fresh-process runs per scenario, with 3 s warmup and a 10 s
measurement window. `--warmup`, `--duration`, and `--repeats` override these values.
Warmup and measurement windows must each be finite, positive, and at most 3600 s.
The runner validates these limits before creating its output directory, bounding
the resource observations retained for each run.
The core loop completes its last field before stopping. Native windows complete
an update before stopping. The reports record actual elapsed time.

The native harness opens windows and uses the default audio device. Leave the
benchmark windows unobstructed and avoid other host workloads during measurements.
The background scenario activates Finder on macOS. Recorded focus counters describe
the whole application. `viewport_states_at_finish` records each viewport's focus
and minimized state when the report is written. Neither establishes physical
occlusion. Inspect the recorded states before comparing runs.

For an application-focused macOS run, add `--keep-foreground`. Once half the warmup time has
elapsed, the runner checks the owned benchmark process by PID during each resource
sampling cycle and brings it forward only when needed. The background scenario
skips this action. `foreground.json` records check and focus-change counts. Automation
failures fail the run. These checks and any resulting focus/input events add overhead;
use the same setting in comparisons and continue checking recorded viewport focus.

The fixture initially focuses the manager. To measure a focused VM, also pass
`--focus-vm`. The runner targets the owned process's **Performance 0** window and
verifies its main-window and focused-window attributes. It raises the window only
when needed. `foreground.json` records successful verifications and missing-window
checks separately; a run without a successful verification fails. Keep
manager-focused and VM-focused results separate when comparing presentation rates.

The runner creates temporary XDG configuration and data directories, links the
installed ROM directory, and generates machine definitions there. The manager image
directory contains one generated 1280 × 960 RGB gradient PNG, making its otherwise
random image selection reproducible. `inputs.json` records the generator version,
image and pixel SHA-256 hashes, and installed ROM filenames and SHA-256 hashes.
Core runs also identify the CoCo 3 BASIC ROM hash. It rejects an
existing output directory. The native driver rejects a nonempty machine library.
Snapshots, previews, and printer fixtures remain in the temporary directory and
are removed when the process exits. Run from the repository root. Python uses only
its standard library. Native focus automation and process sampling target macOS;
other hosts require equivalent focus control and profiler tools.

For development-build measurements, build without `--release` and pass
`--profile dev`. Keep those results separate: workspace configuration optimizes
core dependencies in development, while the frontend remains unoptimized.

## Workload matrix

All default native VMs use CoCo 3, 512 KiB RAM, NTSC, and RGB unless the scenario
selects TV. BASIC starts from a real ROM and receives 120 boot fields before warmup.
The default manager window is 1080 × 720 logical points. Native backend, display,
and audio information are recorded with the run.

| Scenario | Reproducible input |
|---|---|
| `manager-idle` | Empty manager library with the normal manager UI. |
| `basic-idle` | One VM at the BASIC prompt. |
| `graphics` | Guest machine code writes changing pixels into native 320 × 192, 16-color graphics. |
| `paused` | BASIC VM with emulation paused and its window open. |
| `suspended` | BASIC VM saved through the manager's suspend path, with its window open. |
| `background` | Running BASIC VM with Finder activated during warmup. |
| `multi-vm` | Four running BASIC VMs with separate native windows and audio streams. |
| `tv` | BASIC with default color-TV scanline, noise, and overscan effects. |
| `dac` | Guest `STA / ADDA #4 / BRA` loop with PIA DAC routing enabled. |
| `cartridge` | Guest stereo writes to an Orchestra-90 with a synthetic cartridge ROM. |
| `saved-previews` | 500 saved machine definitions and valid suspended states with previews. |
| `printer` | 2000 pages of generated sparse printer output. |
| `snapshot` | Save and restore an actual VM snapshot once per second. |
| `lifecycle` | Repeated launch, suspend, resume, stop, and window-close transitions. |
| `control-load` | Four concurrent local MCP clients repeatedly request `list_vms`. |

Run combinations explicitly to compare effects and VM counts:

```sh
python3 scripts/perf/baseline.py native --scenario paused --scenario suspended --display tv --output /tmp/cocovm-static-tv
python3 scripts/perf/baseline.py native --scenario graphics --scenario dac --display tv --vm-count 4 --output /tmp/cocovm-four-tv
python3 scripts/perf/baseline.py native --scenario background --vm-count 4 --output /tmp/cocovm-four-background
python3 scripts/perf/baseline.py native --scenario lifecycle --duration 60 --output /tmp/cocovm-lifecycle
```

Fixture source is in `crates/coco-core/examples/perf/workloads.rs` and
`crates/coco-egui/src/manager/perf_scenarios/fixtures.rs`. A configuration rejected
by a fixture is an unsupported combination, not a successful measurement.

## Metrics and their boundaries

`metrics.json` contains fixed-size timing histograms, allocation traffic, field
counts, CPU-side texture enqueue counts and bytes, and audio queue statistics.
Percentiles are histogram upper bounds, with at most 6.25% bucket width. Counts
and maxima are exact for recorded events. VM and manager update timings are CPU
scope durations, not input-to-photon latency. Manager updates include immediate
child viewport work and backend waits. Do not add nested timing scopes together.

Texture bytes count `load_texture` and `TextureHandle::set` calls. They describe
requested CPU-to-renderer work. The original baseline includes a first-frame
double enqueue that the presentation cache removes.
They do not establish actual GPU transfer volume or GPU execution time. The
renderer can consolidate requests. GPU timing must come from a native GPU profiler.

Allocation traffic counts successful allocation, zeroed allocation, and reallocation
requests through Rust's global allocator. Reallocation records the requested new
size. These counters do not measure retained heap, native framework allocations,
or GPU memory. Samples aggregate across all VM and host threads. Audio callback
instrumentation records once per callback and allocates no memory.

Audio reports count missing frames, affected callbacks, discarded overflow frames,
and sampled queue extrema. Intentional silence while paused or suspended is
reported separately from running audio reliability. Device configuration describes
the last successfully opened stream. Multiple VMs use the same default device.
Startup and reset can overlap a callback; the reset boundary is not an atomic
transaction across threads.

`warmup_including_cold_first_update` preserves initial UI and preview-loading costs
before the steady-state counter reset. Fixture generation precedes both intervals.

`resources.json` and `samples.json` record externally sampled process CPU, resident
memory, threads, and file descriptors. On macOS, `proc_pid_rusage` also reports
package idle wakeups, interrupt wakeups, and physical footprint. These wakeup
counters do not count every scheduler wakeup. Only samples wholly inside the
measurement markers contribute to summaries. CPU and wakeup deltas cover that
sampled subinterval. Sampling has finite resolution and can miss
brief peaks. Unavailable counters remain null. Host metadata includes CPU, GPU,
OS, Rust compiler, commit, profile, display information, and audio devices. If a
refresh rate is absent from host metadata, record it manually with the result;
frame cadence is not a substitute for panel refresh rate.

## Profiling and measurement overhead

Use a separate capture run so sampling-profiler overhead does not contaminate the
main comparison:

```sh
python3 scripts/perf/baseline.py native --scenario tv --sample-profile --repeats 1 --output /tmp/cocovm-tv-profile
python3 scripts/perf/baseline.py core --scenario dac --no-allocations --output /tmp/cocovm-dac-no-allocation-counts
python3 scripts/perf/baseline.py native --scenario basic-idle --no-telemetry --output /tmp/cocovm-native-no-counters
```

The first command captures a two-second macOS `sample` report. For GPU timing,
input-to-photon latency, and hardware display behavior, use an appropriate native
capture and attach its settings and limitations. Missing measurements must remain
explicitly unavailable.

The native `--no-telemetry` comparison uses the same feature-enabled binary with
recording disabled. Allocator and span enable checks, callback-local accounting,
the scenario driver, and external sampling remain active. This comparison measures
recording overhead, not the entire cost of compiling telemetry into the application.

The `perf` feature is disabled in normal builds. Without it, measurement hooks
compile to empty functions. With it, counters remain disabled until a scenario
starts or `COCOVM_PERF_OUTPUT` is set. A standalone instrumented app can emit
cumulative JSONL once per second by setting that variable without a scenario.
Histograms and counters have fixed memory use. The standalone reporter does not
retain prior snapshots and appends at most 3600 reports per process, one per second.
It disables recording after reaching that limit or encountering an output error.
Existing output files can contain reports from earlier processes. Timed scenario
runs use their configured measurement windows and write one final report instead.

For an optimization comparison, build the same fixtures and instrumentation on
both revisions, alternate run order, and retain all repetitions. Compare medians
and ranges, audio continuity, and resource growth. A corrected audio workload is
not directly comparable to the old silent, escaped loop.

## Verify the audio fixture

The old guard wrote `$FF20` while the PIA selected the data-direction register,
left sound disabled, and encoded a branch to `$FFFF`. Its `INCA` would also change
only discarded DAC bits for three out of four writes.

The corrected fixture sets PA2–PA7 as outputs, selects data access, selects the
DAC mux input, enables sound, and increments by four. The local CoCo 3 Service
Manual, pages 9–10 and 43, establishes the PIA setup. The MC6809 Programming
Manual, section 2.2.6, establishes PC-relative branch displacement. Local MAME
`src/mame/trs/coco3.cpp` and `src/mame/trs/coco.cpp` corroborate the routing.
Orchestra-90 addresses follow the constants already exercised by `tests/orch90.rs`.

```sh
cargo test -p coco-core --test audio_perf --test audio_grid --test sound --test orch90
cargo test -p coco-egui --features perf audio
cargo test -p coco-egui --features perf perf
```

Correctness checks establish loop progression, continued changing output, finite
samples, mono DAC routing, expected sample count, and changing graphics frames.
No test compares elapsed wall time against an absolute host-speed threshold.
