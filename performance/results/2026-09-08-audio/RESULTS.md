# Audio buffer reuse: September 8, 2026

Audio event and producer storage now retain their allocations after warmup. Core
sample regressions and frontend comparisons preserve event order, sample values,
resampler phase, mute/reset behavior, channel mapping, and underrun fade. The queue
keeps its 250 ms bound and oldest-frame discard policy without growing temporarily
when an oversized batch arrives.

The measurements establish an allocation reduction and higher headless throughput.
They do not establish lower whole-app CPU usage or glitch-free audio.

## Allocation and throughput results

| Measurement | Before | After |
|---|---:|---:|
| DAC/cartridge core allocations per field | 524 | 0 |
| DAC/cartridge core requested bytes per field | 75,456 | 0 |
| Producer allocations across 2000 warmed batches | 4000 | 0 |
| Producer requested bytes across those batches | 29,600,000 | 0 |
| Full/empty callback allocations across 2000 calls each | Not isolated | 0 |

The isolated producer paths produce the same bitwise checksum,
`7275021362362243596`. Callback measurements include the production callback and
active timing telemetry. The isolated measurement uses the development profile;
its elapsed times are not release speed comparisons.

Three alternating release captures per revision give median DAC throughput of
4817.0 → 5263.2 fields/s, a 9.3% increase. Cartridge throughput is
4660.0 → 5038.8 fields/s, an 8.1% increase. These runs count allocations.
[Detailed measurements](MEASUREMENTS.md) retain the run-to-run ranges.
A separate counter-disabled control gives DAC throughput of 4805.3 → 5244.8
fields/s, a 9.1% increase, and cartridge throughput of 4678.3 → 5056.7 fields/s,
an 8.1% increase. Each control has one capture per revision and workload, so it
checks the direction of the result rather than establishing a repeatability range.

## Native behavior and the queue decision

Foreground DAC and cartridge workloads reduce whole-process allocation counts by
about 30%, despite executing more UI updates. Requested bytes fall about 3.8% and
3.6%, respectively. Display allocations still dominate byte traffic. Native CPU
medians rise about 2–3%, while UI update rates rise about 3%. Four-VM CPU and
allocation totals also rise alongside more updates. These captures do not establish
a whole-app CPU improvement. Repaint scheduling and display processing remain
separate work.

Running scenarios retain approximately 60 fields/s per VM. All captures report
zero overflow. Background runs have zero missing frames in both revisions. DAC
missing-frame medians fall from 101 to 50.5 per capture, and four-VM medians fall
from 872 to 244. Cartridge medians are 85 and 96.5, with overlapping ranges. Missing
frames remain in foreground playback; the short samples do not establish a
systematic reliability improvement or regression.

After optimization, callback wait p99 is at most 2.047 µs, and maximum observed
wait is 7.416 µs. Hold p99 is at most 6.655 µs, and maximum hold is 11.333 µs.
These costs are small relative to the approximately 11.6 ms device callback period.
This evidence supports retaining the mutex. The callback allocates no memory in
the isolated full/empty workloads. Timing telemetry uses bounded histograms and
compiles out without the `perf` feature.

Queue-depth samples remain below the 250 ms logical limit. The tables express
queue maxima as playback duration, excluding OS/device buffering. Snapshot and
lifecycle captures include deliberate resets and silent intervals; their missing
frames are not steady-playback failures. Those scenarios complete ten operations
per capture, preserving restore and suspend/resume behavior.

## Reproduce and validate

[Methods and commands](METHODS.md) describe the fixtures, retained storage,
measurement boundaries, and counter-disabled control. Source revisions are
`88b62d56a159bb10020b65e8cd48b5acbae286f9` before and
`aafb8e69ca6da5bdebb308e5cedab60cf2ee1864` after. The after worktree's dirty flag
reflects documentation and result files only. The measured code matches that commit.

Host: Apple M3 Max, 64 GB RAM, macOS 26.6.2, Rust 1.97.0. Native audio uses
MacBook Pro Speakers at 44,100 Hz, stereo. VMs use CoCo 3, NTSC, 512 KiB RAM, and
RGB display. The initial native matrix used the internal 120 Hz display. The
follow-up captures also had two 60 Hz external panels attached. Aggregate native
ranges span both topologies, further limiting CPU comparisons. Viewport-to-panel
mapping was not captured. Default captures use 3 s warmup and 10 s measurement.
Foreground focus was requested. Before captures still recorded 29 unfocused
multi-VM updates, three unfocused snapshot updates, and ten unfocused follow-up
DAC updates. Background scenarios deliberately release focus; lifecycle transitions
can also leave all windows unfocused.

Core order is before/after, after/before, before/after across the three repeats.
Native runs capture the full matrix before and then after, followed by a DAC and
cartridge recheck of both revisions. The first before DAC capture is excluded
because compilation may have overlapped startup. Its raw data remains published.
The recheck replaces that capture, producing three retained before DAC runs and
four after runs. Cartridge has four per revision; other scenarios have three.
No compiler or tests ran during subsequent performance captures.

Validation passed: 1392 workspace tests, 476 frontend tests with `perf,debug-ui`,
workspace checks, default/feature clippy with warnings denied, and formatting.
The ignored isolated allocation measurement also passed. The other ignored test
is the existing golden-fixture generator. Rust-analyzer reported no errors and
three feature-dependent unused-variable warnings in unchanged manager code.
Socket tests required an unrestricted rerun after the sandbox denied local binds.

The JSON files retain measurements, host observations, input hashes, and warmup
reports. Serial identifiers and non-default personal input-device names are omitted
or redacted from published metadata. No copyrighted ROMs or private media are included.
