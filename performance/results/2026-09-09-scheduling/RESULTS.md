# Deadline scheduling comparison: September 9, 2026

Absolute emulation and presentation deadlines reduce redundant UI callbacks for
VMs outside the focused viewport while preserving guest field progression. Per-VM
focus and minimized state select foreground or background scheduling. Deferred
controls use deadline wakeups instead of continuous polling.

The matched native captures show the effect when the harness keeps the application
foreground and focuses the manager window. A separate probe focuses the VM viewport
and verifies the foreground path. Application focus counters alone don't identify
which viewport has focus, so the two capture types answer different questions.

## Scheduling results

The primary values are medians across three fresh-process captures per revision
and workload. Each capture uses a 3 s warmup and a 10 s measurement window.
[Detailed measurements](MEASUREMENTS.md) include ranges and the remaining
resource, audio, and control data.

| Workload | CPU before → after | Manager UI callbacks/s before → after | Fields/s before → after |
|---|---:|---:|---:|
| BASIC, RGB | 19.71% → 4.21% | 89.69 → 13.18 | 59.97 → 59.98 |
| Four BASIC VMs, RGB | 34.08% → 11.19% | 71.69 → 12.28 | 239.76 → 239.56 |
| BASIC, default TV | 27.02% → 5.51% | 89.66 → 12.19 | 59.98 → 59.89 |
| Four BASIC VMs, default TV | 65.53% → 16.24% | 70.91 → 12.29 | 239.87 → 239.77 |

The RGB BASIC CPU median falls by 78.6%, and the four-RGB median falls by 67.2%.
Both workloads retain approximately 60 fields/s per VM. Missing audio changes
from a median of 277 to 11 frames for one RGB VM and from 853 to 554 frames for
four RGB VMs. The default-TV medians fall by 79.6% for one VM and 75.2% for four
VMs. TV missing audio changes from 403 to zero frames for one VM and from 153 to
85 frames for four VMs. All runs report zero overflow frames. Missing-frame counts
vary between repeats, so these captures don't establish an audio-reliability
guarantee.

Default-TV texture uploads fall from 61.50 to 11.99/s for one VM and from
243.00 to 47.96/s for four VMs.

The baseline target called for at least a 20% CPU reduction in the matched
focused-BASIC fixture. The 78.6% reduction exceeds that target for the fixture's
actual manager-focused state.

The active graphics, DAC, and cartridge workloads retain 59.94–59.98 fields/s
after the change. Their CPU medians fall from 19.91–21.25% to 4.22–4.79%, and
their manager callback medians fall from 89.59–89.87/s to 12.29–12.79/s. All
nine final runs report zero missing and overflow audio frames. Final median queue
maxima rise to 4,530–4,779 frames because background scheduling maintains a larger
audio cushion.

The control-load p99 median remains 18 ms, with a range of 18–21 ms before and
18 ms in all final repeats. Its p95 median changes from 16 to 17 ms. Median
successes change from 3,164 to 3,181 requests. Both revisions have a median of
28 observed failures, and the captures don't classify their cause. This
fixed-duration `list_vms` load doesn't validate deferred field-wait behavior.

Printer cadence changes from 59.98 to 59.90 fields/s. Lifecycle and snapshot
fixtures complete their operations without an aggregate cadence regression, and
saved previews remain effectively idle. Lifecycle and snapshot audio counts
include their deliberate resets and silent intervals.

## Focus and deadline behavior

Single steady-focus probes place the VM on the 120 Hz internal panel and the
60 Hz external panel. All nine accessibility checks per probe report the VM as
both main and focused. The probes record 60.28 and 60.19 UI callbacks/s,
respectively, while retaining 59.98 and 60.00 fields/s. Both record 8.80 texture
uploads/s and zero overflow frames. Missing audio is zero on the 120 Hz probe and
197 frames on the 60 Hz probe. Application callbacks don't measure native
physical presents, so these results establish guest cadence on the two panels,
not presentation cadence.

Earlier VM-focus datasets apply focus at the measurement boundary and therefore
include a focus-state transition. They are published as transition probes and
aren't pooled with the steady-focus results. A matched unchanged-revision
steady-focus result is unavailable because direct accessibility operations failed
with `kAXErrorCannotComplete` (`-25204`).

A native minimized-state transition is also unavailable. Setting `AXMinimized`
didn't persist, the accessibility minimize action returned `-1728`, and manager
focus verification failed. The failed automation attempts are excluded from the
published captures. Deterministic frontend tests cover minimized-state scheduling.

Native control validation observes a paused 60-field wait timing out in 11.128 s
with zero fields, display conversions, texture uploads, and audio pushes. Its
15 s instrumented run records 75 manager callbacks, including native, control,
and harness events; this doesn't establish a deadline-only callback count. In a
mixed run, the paused target times out in 11.150 s while `list_vms` completes in
13.84 ms and a running peer completes a 60-field wait in 1.012 s. The two VMs
advance 2,284 and 2,997 fields, respectively, with zero audio overflow.

A paused 600-field wait resumes after 2 s and completes in 11.943 s. Closing a
lifecycle target fails its pending wait in 6.130 s. The native mixed-suspended
probe is unavailable because automation reports `accessible Suspend button not
found`; lifecycle coverage includes suspend and close, but it doesn't establish
mixed suspended and running behavior.

A separate one-repeat probe checks four background RGB VMs. The unchanged
revision uses 17.39% CPU with 27.46 manager callbacks/s and 109.83 aggregate VM
callbacks/s. The final coalesced scheduler uses 10.82% CPU with 12.20 manager
callbacks/s and 48.79 aggregate VM callbacks/s. It retains 239.87 fields/s and
reports zero missing and overflow audio frames. Interrupt wakeups fall from
101.28/s to 88.53/s. These results are consistent with coalescing aligned VM
deadlines into shared host wakeups. One repeat doesn't establish a distribution.

## Measurement boundaries

UI callback and upload rates are application scheduling and presentation-work
proxies. They don't measure native window presents, GPU submissions, or physical
refreshes. The harness doesn't establish native presentation cadence from
callbacks.

Interrupt wakeup counts come from process resource samples and don't count every
scheduler wakeup. Package-idle wakeup deltas of zero don't mean that the process
caused no scheduler activity. Control request latency measures local list-request
completion, not input-to-photon latency. GPU timing and input-to-photon latency
are unavailable.

## Reproduce and validate

[Methods and commands](METHODS.md) describe the builds, capture order, focus
checks, and table regeneration. The clean before-TV captures are isolated from
compiler and language-server work. The original 21 before-RGB runs are excluded
from the published comparison because early runs might overlap that work. A clean
replacement set supplies the final before-RGB values.

Before source: `da8b553`. After source: `f76f2b9`. The result files and
documentation account for the after checkout's dirty state.

Validation passes 1,468 workspace tests with one ignored and 519 frontend tests
with `perf,debug-ui` with three ignored. All 24 Python runner tests pass. Workspace all-target checks, `perf,debug-ui` Clippy with
warnings denied, formatting, and rust-analyzer diagnostics also pass. Review found
no concrete implementation defect.
