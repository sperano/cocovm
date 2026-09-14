# UI stalls and remaining-core results

Printer export no longer rasterizes and encodes on the UI thread. An export
captures a bounded immutable paper snapshot, then streams pages through a bounded
worker. The selected bus fast path replaces division-based RAM wrapping with an
equivalent mask and improves the BASIC core workload by 4.4% in the primary
comparison and 7.3% in a matched recheck.

The public measurements are the JSON files in this directory. See
[the methods](METHODS.md) for commands, exclusions, and unsupported scenarios,
and [the profile summary](PROFILES.md) for the ranked core costs.

## Keep printer export off the UI thread

The release probe uses a deterministic 32-page sparse roll at 144 dpi. Values are
medians of three runs except peak RSS, which is one macOS observation per mode.

| Export | Caller-thread path | Streamed end to end | Background submission | Peak RSS, buffered → streamed |
|---|---:|---:|---:|---:|
| PDF | 317.543 ms | 301.943 ms | 0.011 ms | 277.4 → 11.3 MiB |
| Whole-roll PNG | 192.550 ms | 447.073 ms | same background path | 536.1 → 11.0 MiB |

The PDF worker is slightly faster end to end. Page-at-a-time roll PNG is 2.3
times slower, but it removes a UI stall and a roll-sized raster allocation. This
is a latency and memory tradeoff, not a PNG throughput improvement. A 2,000-page
sparse paper snapshot takes 0.746 ms (0.740–0.748 ms). A deliberately adverse
15.1 MiB fixture with 110,000 distinct B-tree rows clones in 2.230 ms
(2.012–2.264 ms), below the 16.7 ms foreground-frame budget.

Every output uses a temporary sibling file or directory and commits by rename.
Cancellation and errors remove the temporary output and preserve an existing
destination. The UI reports page progress, success, cancellation, and actionable
errors. Tear-off, printer detach, and later printing cannot change an in-flight
immutable snapshot.

The budgets are explicit:

- At most two printer exports run in the process, with no pending queue.
- Each paper window owns at most one job. Duplicate and third-process requests
  fail immediately with a retry instruction.
- Each immutable paper snapshot is limited to an estimated 16 MiB before clone.
- Each worker has a 32 MiB raster, encoder, and PDF-object-table budget.
- Rendering visits the immutable snapshot in place; it does not construct an
  unbudgeted page-local dot-coordinate copy.
- The process-wide incremental ceiling is 96 MiB for two worst-case jobs,
  excluding the live paper already owned by each VM.
- Cancellation is checked between pages, during dense-dot rasterization, and
  during PDF row encoding. Closing a VM or shutting down cancels and joins its
  worker. A currently blocking
  filesystem call cannot be preempted, but it can only affect temporary output.

## Attribute other UI operations

The harness now records each periodic operation's caller-thread duration and
success or error. It also writes the partial report before a failed operation
terminates the scenario.

RAM-only snapshot round trips take 1.749 ms median across 30 attempts, with a
1.673–2.306 ms range. Save p99 is 1.376–1.638 ms across runs, and restore p99 is
0.492–0.721 ms. This does not justify copying a full machine to a background
worker. The fixture has 512 KiB RAM and no mounted media, so it does not establish
large-media or slow-storage behavior.

The 60-second lifecycle probe identifies remaining synchronous ownership stalls:

| Lifecycle action | Attempts | Median | Range |
|---|---:|---:|---:|
| Suspend for warm resume | 21 | 1.640 ms | 1.562–1.935 ms |
| Resume live VM | 21 | 0.116 ms | 0.098–0.155 ms |
| Close suspended window | 21 | 42.847 ms | 37.857–51.640 ms |
| Resume cold VM | 21 | 29.852 ms | 27.567–30.850 ms |
| Stop VM | 21 | 43.527 ms | 38.592–51.561 ms |
| Start VM | 18 | 29.238 ms | 27.440–30.588 ms |
| Close running window | 18 | 45.112 ms | 39.053–51.553 ms |
| Restart after close | 18 | 29.538 ms | 27.878–31.017 ms |

Launch and close costs include synchronous host audio-stream construction and
destruction. The probe does not isolate individual CPAL calls. Moving machine
ownership to a worker is unsafe because the machine contains main-thread-only
state; a shared audio-host redesign needs its own ownership and device-failure
work rather than a speculative thread handoff.

## Optimize the measured bus path

Fresh profiles put `SystemBus::read` at 11.7% to 24.4% of samples. Every supported
RAM size is a power of two, and restore validates RAM length against the machine
configuration. `SystemBus::phys` can therefore replace `% ram.len()` with
`& (ram.len() - 1)` without changing MMU translation or constant-page aliases.

| Core workload | Before fields/s | After fields/s | Median change |
|---|---:|---:|---:|
| BASIC | 6279.5 (6270.6–6284.8) | 6554.6 (6547.6–6554.6) | +4.4% |
| Graphics | 4172.5 (4149.2–4198.2) | 4213.9 (4213.7–4215.9) | +1.0% |
| DAC | 5340.6 (5259.8–5343.7) | 5485.3 (5474.3–5501.8) | +2.7% |
| Cartridge | 5085.1 (5082.7–5103.0) | 5106.7 (5053.4–5127.8) | +0.4% |

All runs retain zero steady-state allocations. The graphics and cartridge ranges
overlap, so they do not establish improvements. A separate after/base/after/base
BASIC sequence gives 6246.4 fields/s before and 6702.1 after across six runs per
revision, a 7.3% increase. One later BASIC batch that fell to 4687–5847 fields/s
despite approximately 100% CPU is excluded as an environmental outlier; its
companion workloads remained stable, and the repeated BASIC sequence supplies
the confirmation measurement.

Renderer masking was tested and discarded: its 0.53% graphics median change was
inside overlapping ranges and would have narrowed a public renderer API's prior
support for arbitrary RAM slices. No CPU, interrupt, HALT, or scanline semantics
changed.

## Compare all six performance tasks

Percentages below belong to each task's matched comparison. Do not add them: the
revisions, focus states, display topology, and some workload details differ.

| Task | Attributable result | Remaining qualification |
|---|---|---|
| 1. Baseline | Established reproducible core/native fixtures and latency, allocation, resource, and audio targets. | GPU time, input-to-photon latency, and several host-I/O cases are unavailable. |
| 2. Display | BASIC uploads 89.46 → 8.80/s; default-TV allocation traffic 280.27 → 90.42 MiB/s; isolated TV buffers allocate zero after warmup. | Clocked TV noise intentionally keeps animated paused TVs active. |
| 3. Audio | DAC/cartridge allocations 524 → 0 per field; throughput +9.3% and +8.1%. | Native CPU did not show a matched improvement; short runs do not guarantee glitch-free audio. |
| 4. Scheduling | BASIC CPU 19.71% → 4.21%; four RGB 34.08% → 11.19%; TV 27.02% → 5.51%, with field rates preserved. | Input-to-photon latency remains unavailable. |
| 5. Resources | 500-preview RSS 782.97 → 172.59 MiB; 2,000-page printer CPU 24.36% → 5.23%; lifecycle threads 14–75 → 11–18. | GPU memory is estimated; scroll input is deterministic rather than a real gesture. |
| 6. UI/core | Export submission 0.011 ms with roll-sized allocations removed; BASIC core throughput +4.4%, confirmed by a +7.3% recheck. | Lifecycle audio ownership, large mounted-media I/O, debugger costs, and renderer work remain. |

From the original baseline to the final task's primary core measurements, BASIC
changes from 6112.7 to 6554.6 fields/s, graphics from 4138.1 to 4213.9, DAC from
4769.4 to 5485.3, and cartridge from 4648.8 to 5106.7. These start-to-finish values
include earlier audio and other merged changes; they are not attribution to this
task alone.

## Remaining limits

Rendering remains 38% to 62% of sampled core work. A safe scanline cache requires
more renderer-local attribution and hardware validation of live versus latched
GIME state. Debugger-open costs have no matching scenario. Media flush and hash
costs for large disk, VHD, DriveWire, cassette, and photo inputs, injected slow or
failing storage, and persistent configuration-save failures are also unsupported.
No optimization is claimed for those cases.

## Validation

- `cargo fmt --all -- --check`, `cargo check --workspace`, and workspace clippy
  with all targets and warnings denied pass.
- The workspace test suite passes 1,597 tests with one ignored; the
  `perf,debug-ui` suite passes 626 tests with three ignored.
- Feature-specific check and clippy, the three Python report tests, and JSON
  parsing of all eight result files pass.
- The standalone rust-analyzer diagnostic scan reports E0282 at the unchanged
  `manager/settings.rs` log-reload call. Both compiler checks and both clippy
  configurations resolve that call without an error or warning; no edited file
  has a compiler diagnostic.
