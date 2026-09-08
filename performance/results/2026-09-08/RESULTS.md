# Performance baseline: September 8, 2026

The largest measured opportunities are repeated display work, audio-event allocation,
preview loading, and resources retained across VM lifecycle transitions. This change
establishes a baseline; it does not optimize those paths or establish a valid speedup
against the old audio guard. That guard did not exercise its intended workload.

## Reproduce and inspect

Use the [harness instructions](../../README.md) and the commands in [this run manifest](RUNS.md).
[Detailed tables](MEASUREMENTS.md) include medians and run-to-run ranges. Adjacent
JSON files retain every run's metrics, host samples, build metadata, fixture hashes,
focus observations, and warmup counters. The two sampling profiles are summarized
in [PROFILES.md](PROFILES.md); their timings are excluded from the primary results.

All captures used source commit `0500187d6aca583a37378822cbb08f7af3df045c`, with a
clean worktree. Later changes cap telemetry duration and publish these results;
they do not change the valid-window scenario execution measured here.
Release builds use the workspace's normal release profile. Development builds are
reported separately because dependencies, including the core, use optimization
level 2, while the frontend is unoptimized.

Host: Apple M3 Max, 16 CPU cores (12 performance, 4 efficiency), 40 GPU cores,
64 GB RAM; macOS 26.6.2; Rust 1.97.0. Native rendering used OpenGL 4.1 Metal 90.5.
The attached displays reported two Dell panels at 60 Hz and the built-in panel at
120 Hz. Per-viewport monitor association was not captured. Audio used MacBook Pro
Speakers at 44,100 Hz, stereo. Default VMs were CoCo 3, 512 KiB RAM, NTSC, RGB;
TV and VM-count overrides appear in each run. No other benchmark or build ran
concurrently. Ordinary OS activity and the external observer remained present.

Each ordinary dataset has three fresh-process runs, 3 s warmup, and 10 s measured
execution. Lifecycle stress uses 60 s; each profiler capture has one run. Core
throughput is unpaced. Native field execution remains paced near 60 fields/s/VM.
Native foreground runs use `--keep-foreground`, including the recording-overhead
comparison. Main-matrix manager and BASIC runs had mixed focus observations;
retain them as evidence but use `focused.json` for those two scenarios. All other
main-matrix scenarios reported focused updates, except intentional background runs.
The observer cannot establish physical occlusion.

## Core and frontend costs

Headless BASIC sustains 6112.7 fields/s (6086.1–6136.6), and graphics sustains
4138.1 (4136.7–4139.2). Both allocate zero Rust heap bytes during the measured
steady state. Corrected DAC and Orchestra-90 workloads sustain 4769.4
(4768.5–4783.3) and 4648.8 (4636.0–4667.7) fields/s, respectively. Each audio
workload allocates exactly **524 times and requests 75,456 bytes per field**.
This is 343.20 MiB/s of allocation traffic for the unpaced DAC workload. These
results separate audio-event storage from the much larger native UI/display costs.

Focused BASIC uses 20.98% of one CPU core (20.87–21.21), with 73.22 MiB/s of
allocation traffic and 53.58 MiB/s of CPU-side texture submissions. Active RGB
graphics uses 21.14% (20.96–23.35). It executes 60 fields/s but processes about
90 VM updates/s. A single background VM retains 60 fields/s at 4.29% CPU
(4.18–4.30), about 12 updates/s, and 7.08 MiB/s of texture submissions. The
existing background cadence therefore provides a useful reference for reducing
presentation work without slowing emulation.

One TV VM uses 31.13% CPU (31.06–31.58), 284.20 MiB/s of allocation traffic,
and 105.92 MiB/s of texture submissions. Allocation traffic per VM update is
3.144 MiB, versus about 0.802 MiB for active RGB graphics. Display conversion
p99 is 1.180–1.245 ms for TV, versus about 0.102 ms for RGB graphics. TV output
has twice the height: 1.172 MiB per submitted image versus 0.586 MiB for RGB.
Four RGB VMs use 39.81% CPU and 218.54 MiB/s allocation traffic; four TV VMs
use 67.55% CPU and about 812 MiB/s. Both retain about 240 aggregate fields/s.
Normalize comparisons by VM update counts because presentation cadence differs.

Empty-manager CPU is below the external sampler's resolution. Paused and suspended
RGB windows use about 0.1–0.21% CPU and redraw only 3–5 times per 10 s, although
those incidental redraws still submit unchanged images. These measurements support
removing redundant work; they do not support claiming a large idle CPU problem.
Animated TV effects need an explicit animation cadence when changing invalidation.

## Audio and responsiveness

Running foreground scenarios occasionally report missing audio frames, with no
measured overflow. Four RGB VMs report 83–361 missing frames out of 1,765,376
requested per run. Their callback lock-wait p99 is 0.863–0.927 microseconds, which
does not establish lock contention as the main audio bottleneck. Intentional
paused/suspended silence and lifecycle resets must be separated from running
playback reliability. Queue extrema and callback counts are available in raw JSON;
queue sampling and counter reset can straddle callback boundaries.

The 500-entry saved library reaches 777.86–783.20 MiB RSS and approximately
995–1002 MiB physical footprint. These are different memory measures; do not add
them. Its first manager update takes 174.993–176.208 ms. Steady redraws still
allocate about 11,266 times and request 1.233 MiB each. Previews are valid but
hardlinked copies of one fixture, so this measures cold UI decoding with a warm
filesystem, not cold disk access or diverse image content. Framebuffer submission
counters exclude library thumbnail uploads.

A 2000-page sparse printer roll uses 25.64–25.97% CPU, with VM UI p99
12.58–13.11 ms. That scope includes native printer viewport work and backend
waits; it cannot attribute the full duration to page layout.

Snapshot runs complete ten save/restore pairs each. Save p99 is 1.442–1.573 ms,
restore p99 is 0.508–0.524 ms, and combined operation p99 is 1.901–2.097 ms.
They report 3.59–3.65% missing audio frames across intentional restore
continuity breaks. This small RAM-only fixture does not establish the cost of
large media hashing, flushes, or slow storage. Lifecycle operation p99 reaches
50.33–54.53 ms in short runs; its aggregate scope does not identify which
transition caused the longest stall.

Four control clients complete 3152–3160 successful requests per run, with p95 in
[16,17) ms and p99 in [17,19) ms across runs; exact maximum is 22.1–26.5 ms.
The integer-millisecond client histogram reports lower bounds, unlike the Rust
upper-bound histograms. Each run also records 32 failures. Their timing and error
categories were not captured, so these cannot be classified as steady-state
server failures or assumed to be shutdown-only failures. Client startup and
shutdown follow external observation of markers, not the exact measured interval.
Successful counts therefore are not an exact ten-second request rate. Native CPU
is 25.41–25.43%, with 15–22 sampled threads and 9–17 file descriptors.

## Lifecycle and additional combinations

All three 60-second lifecycle runs complete 60 operations, or ten six-step cycles.
Later cycle envelopes grow by exactly six threads per cycle, reaching 67–74
threads in the final cycle. File descriptors remain at eight. RSS growth slows,
but late cycle peaks still rise by 0.65–0.72 MiB over 24 seconds; a strict memory
plateau is not demonstrated. Nominal cycle matching uses elapsed time because
per-operation state/timestamp markers were not recorded. No post-workload recovery
interval was captured.

Code inspection identifies joystick initialization as a concrete ownership
candidate: each new VM constructs `Gilrs`, whose resolved macOS dependencies
start a HID run-loop worker and a force-feedback worker with no observed shutdown
path. Three VM constructions per cycle predict six retained workers. This matches
the counts but requires runtime thread identities/stacks to confirm attribution.
The [additional analysis](ADDITIONAL.md) gives cycle envelopes, dependency versions,
and code locations, plus four-VM background and static-TV results.

Four background RGB VMs keep about 240 fields/s but require 28.0–30.5 manager
updates/s, versus 12.08 for one background VM. Their CPU is 19.41–20.37%, and
missing audio ranges from 0 to 855 frames per run. Background scheduling therefore
needs multi-VM validation; the single-VM cadence does not scale unchanged.
Paused/suspended TVs perform only two or three measured updates, each still
allocating about 3.14–3.16 MiB and submitting 1.172 MiB.

## Development builds

Development DAC throughput is 3931.7 fields/s (3921.5–3950.1), about 17.6% below
the release median despite core optimization level 2. Development native BASIC
uses 24.67% CPU (24.49–25.49), versus 20.98% in release, and VM UI p99 is
0.623–0.688 ms. It retains 59.8–60.0 fields/s and reports 127–496 missing audio
frames per run. These are separate build-profile observations, not optimization
regressions or a guarantee for other development workloads.

## Instrumentation overhead and limits

Disabling allocation recording raises headless DAC throughput from 4769.4 to
4823.6 fields/s (4820.0–4829.7): about 1.1% observed recording cost. Focused
native BASIC uses 20.98% CPU with recording and 20.93% without it
(20.85–20.95); the ranges overlap, so this experiment does not resolve a native
CPU recording cost. Both comparisons retain enable checks and the same harness.
They do not measure the whole feature's compilation overhead. Sequential run
order is a limitation; alternate order for optimization comparisons.

Rust allocation counters describe traffic through the global allocator, not
retained heap, native allocations, or GPU memory. CPU texture submissions are
requests to the renderer; actual GPU transfer volume and execution time are
unavailable. Input-to-photon latency, physical occlusion, total scheduler wakeups,
and exact per-transition resource ownership are also unavailable. Manager and VM
scope durations are nested, include backend waits, and are not interchangeable
with end-to-end input latency. Fixed histogram p95/p99 values are upper bounds
with at most 6.25% bucket width; maxima are exact recorded values.

External samples can miss brief peaks. CPU and wakeup deltas cover only the
sampled subinterval within measurement markers. Package-idle wakeups were zero;
that does not imply zero scheduler activity. Interrupt wakeups, thread counts,
file descriptors, and physical footprint remain in the raw data. Development and
profile captures must not be pooled with release primary runs.

## Targets for subsequent changes

These are provisional goals derived from this host and these workloads, not
absolute CI timing assertions. Re-run matched scenarios, preserve display and
audio behavior, and evaluate medians and ranges before claiming improvement.

| Workstream | Measured starting point | Target and validation |
|---|---|---|
| Framebuffer processing | About 90 updates for 60 fields/s; repeated unchanged submissions | Submit only on image or display invalidation; at most once per new field absent independently due animation; remove duplicate first upload. |
| TV buffers | 3.144 MiB allocated/update; conversion p99 1.180–1.245 ms | Reduce traffic/update by at least 25% to at most 2.358 MiB; investigate p99 below 0.9 ms with equivalent output. |
| Audio buffers | 524 allocations/field for changing DAC and cartridge audio | Zero repeated event/producer scratch allocations after capacity warmup; preserve sample values, phase, queue limits, and timing. |
| Repaint scheduling | Focused BASIC 20.98% CPU; background 4.29%, both 60 fields/s | Reduce matched focused CPU by at least 20%; tie presentation to new output or animation deadlines and retain background field/audio continuity. |
| Preview resources | Roughly 625 MiB incremental RSS for 500 entries; 11,266 allocations/redraw | Bound preview residency to a documented viewport cache, provisionally below 64 MiB incremental RSS; reduce redraw allocations by at least 90%. |
| Printer layout | 2000 pages; VM UI p99 12.58–13.11 ms includes waits | Make layout scale with visible pages; separately time layout before setting a CPU-duration target. |
| Lifecycle resources | Thread counts grow through ten six-step cycles | Reach a stable same-state resource plateau after cache warmup; identify and release retained ownership. |
| Control resources | Four clients, 15–22 threads, 9–17 descriptors | Bound concurrency and retained requests/sessions; add timestamped failure categories before deriving an overload/error-rate target. |
| UI stalls | First preview update about 175 ms; lifecycle operations up to about 55 ms | Keep synchronous preview work below one 60 Hz field (16.7 ms) using bounded loading; profile lifecycle transitions individually. |
| Remaining core work | Zero steady allocations in BASIC/graphics; high unpaced throughput | Reprofile after display/audio changes and optimize only remaining measured hotspots; small snapshot results do not justify a storage rewrite by themselves. |
