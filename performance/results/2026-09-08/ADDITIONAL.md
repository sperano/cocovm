# Additional combinations and lifecycle resources

Additional native measurements cover 21 runs from source `0500187d6aca583a37378822cbb08f7af3df045c`: controlled foreground manager/BASIC, four RGB VMs in the background, paused/suspended TVs, four foreground TVs, and three 60 s lifecycle runs. Each configuration has three repetitions. No run had unknown focus. Foreground cases remained focused throughout measurement; the four-VM background case remained unfocused. These controlled BASIC and manager runs supersede the mixed-focus versions in the main matrix.

| Configuration | CPU % of one core | Aggregate fields/s | Aggregate VM updates/s | Peak RSS MiB | Allocated MiB/VM update | Enqueued MiB/s | VM p99 ms |
|---|---:|---:|---:|---:|---:|---:|---:|
| focused manager | 0.00 | 0.00 | 0.00 | 149.81–150.81 | — | 0.00 | — |
| focused BASIC | 20.87–21.21 | 59.85–59.98 | 90.96–91.62 | 153.34–159.14 | 0.801 | 53.30–53.69 | 0.377–0.393 |
| four RGB background | 19.41–20.37 | 239.40–239.94 | 112.00–121.92 | 167.95–168.73 | 0.753–0.755 | 65.62–71.44 | 1.114 |
| paused TV | 0.10–0.21 | 0.00 | 0.20–0.30 | 164.27–164.91 | 3.135 | 0.23–0.35 | 1.180–1.311 |
| suspended TV | 0.21 | 0.00 | 0.20–0.30 | 167.16–168.11 | 3.154–3.160 | 0.23–0.35 | 1.180–1.311 |
| four TVs foreground | 67.44–67.56 | 239.84–239.98 | 260.81–263.76 | 173.59–175.06 | 3.082 | 305.64–309.09 | 1.573 |
| 60 s lifecycle | 12.02–12.03 | 27.76–27.80 | 43.38–43.54 | 165.81–167.17 | 0.882–0.883 | 25.32–25.41 | 0.459 |



The controlled empty manager performed four measured UI updates per run. CPU deltas rounded to 0.00% at the sampler’s precision; this is not proof of zero work. It allocated 163 times and 0.0427 MiB per manager update, with manager p99 0.043–0.066 ms and seven sampled interrupt wakeups per run. Sampled file descriptors remained eight; thread ranges were 4–7.

Controlled BASIC ran 59.85–59.98 fields/s but 90.96–91.62 UI updates/s, or 1.517–1.531 updates per field. Allocation traffic was about 692 allocations and 0.8006–0.8007 MiB per UI update. Manager p95/p99 was 13.107/13.631 ms; VM p99 was 0.377–0.393 ms. Missing audio counts were 24, 0, and 75 of 441,344 requested frames per run. Queue extrema across repeats were 0–2,079 frames; callback lock-wait p99 was 1.087–1.215 µs. Thus the stable foreground baseline still contains small playback gaps.

Four background RGB VMs maintained 239.40–239.94 aggregate fields/s, with 112–122 aggregate VM updates/s, corresponding to 28.0–30.5 manager updates/s. The single-background baseline had 12.08 manager updates/s and 4.18–4.30% CPU; four background VMs measured 19.41–20.37% CPU. Background cadence therefore does not remain at the single-VM repaint rate. This observation does not identify which native viewport scheduling path causes the additional updates. Per-VM-update allocation traffic was 0.753–0.755 MiB. Audio missing counts were 228, 855, and 0 of roughly 1.765 million requested frames; maximum queue depth was 6,962 frames and lock-wait p99 stayed below 1 µs. File descriptors remained eight; threads ranged 25–35.

Four foreground TVs sustained 239.84–239.98 fields/s at 67.44–67.56% CPU, versus 39.56–39.91% for four foreground RGB VMs in the main matrix. Their aggregate UI rate was 260.81–263.76 updates/s, lower than RGB’s 296.17–297.56, so normalize conversion/allocation comparisons by updates. TV allocation traffic was 3.082 MiB per aggregate VM update versus RGB’s 0.737 MiB. TV texture submissions totaled 305.64–309.09 MiB/s. Manager p99 was 25.166–28.312 ms, while individual VM p99 was 1.573 ms and display conversion p99 was 1.180–1.245 ms. Missing audio counts were 0, 143, and 0 of roughly 1.766 million requested frames. Queues ranged 0–2,610 frames; lock-wait p99 was 0.799–0.927 µs. Threads ranged 26–36, with eight file descriptors.

Paused and suspended TVs rendered only two or three measured updates in 10 s. They still allocated about 3.14–3.16 MiB and submitted 1.172 MiB for each update. CPU was 0.10–0.21%; the large per-update TV cost did not produce a large idle CPU rate because repaint count was small. Their p99 values are based on only two or three observations. Every requested audio frame was absent because these scenarios intentionally stop production; those counts are not steady-playback underruns.

All additional runs reported zero overflow frames. Package-idle wakeup deltas were zero; this is not a count of all scheduling wakeups. Sampled interrupt wakeups were 753–806 for four background RGB VMs, 1,020–1,027 for four foreground TVs, and about 185–186 for paused/suspended TVs over each sampled subinterval. Physical-footprint peaks were 538–571 MiB for four background RGB VMs and 519–521 MiB for four TVs; footprint and RSS are separate measurements and must not be summed.

The 60 s lifecycle test completed 60 operations—ten repetitions of the six-step suspend, close suspended window, cold resume, stop, start, and close-and-restart sequence. It remained focused and ended with one live VM. Host-operation p95 was 52.429–56.623 ms and p99 was 54.526–58.720 ms. It generated roughly 488,000 missing frames while deliberately pausing, powering off, and resetting audio; these counts do not measure uninterrupted playback reliability.

Each lifecycle run has 50 complete resource observations. Grouping elapsed time into nominal six-second cycle windows gives the following envelopes. Ranges in a cell are across the three repetitions. Actual operation timestamps and lifecycle state were not recorded with resource samples, so these windows are a phase-aligned approximation, not confirmed state labels.

| Nominal cycle | Elapsed window s | Minimum threads | Maximum threads | Minimum RSS MiB | Maximum RSS MiB |
|---|---:|---:|---:|---:|---:|
| 1 | 0–6 | 14 | 20 | 158.80–159.45 | 162.12–162.95 |
| 2 | 6–12 | 19 | 26 | 162.06–163.20 | 163.28–164.53 |
| 3 | 12–18 | 25 | 31–32 | 162.52–163.59 | 163.89–164.89 |
| 4 | 18–24 | 30–31 | 37–38 | 163.28–164.41 | 164.67–166.02 |
| 5 | 24–30 | 36–37 | 44 | 163.55–164.81 | 164.88–166.14 |
| 6 | 30–36 | 43 | 50 | 163.77–165.20 | 165.09–166.52 |
| 7 | 36–42 | 49 | 56 | 163.98–165.38 | 165.31–166.70 |
| 8 | 42–48 | 55 | 62 | 164.12–165.53 | 165.45–166.86 |
| 9 | 48–54 | 61 | 68 | 164.31–165.70 | 165.62–167.03 |
| 10 | 54–60 | 67 | 74 | 164.48–165.84 | 165.81–167.17 |



The thread count does not plateau: the later cycle minimum and maximum both rise by six threads per cycle, reaching 67–74 in the final cycle. Nominal closed-window-phase samples around 1.3–1.5 s into successive cycles also rise from 14 to 67 threads; the source phase is inferred because sample records lack state. File descriptors remain eight in every complete lifecycle observation.

RSS growth slows after the initial cycles but does not demonstrate a strict plateau. From the 30–36 s window to the 54–60 s window, the per-run RSS maximum still increases by 0.65–0.72 MiB, with corresponding increases in the cycle minimum. Peak RSS remains much narrower, 165.81–167.17 MiB, than the thread-count change might suggest. This observation does not attribute RSS growth to any particular allocation or prove an unbounded heap leak. Aggregate physical-footprint peaks were 445.50–447.14 MiB; they vary with lifecycle phase. A recovery interval after stopping the workload was not recorded.

Code inspection identifies a specific thread-ownership candidate. `CocoApp::new` constructs `JoystickInputs` (`crates/coco-egui/src/app.rs:309`), which creates a `gilrs::Gilrs` even when both input sources are `None` (`src/joy.rs:121`). The resolved macOS dependency `gilrs-core` 0.6.8 spawns a detached HID thread that enters `CFRunLoop::run()` without a receiver-drop stop path (`src/platform/macos/gamepad.rs:38–99`). `gilrs` 0.11.2 enables force feedback by default (`src/gamepad.rs:644`) and spawns another detached thread (`src/ff/server.rs:279–293`); its outer loop continues when `try_recv()` returns disconnected (`src/ff/server.rs:146–276`). Each lifecycle cycle creates three VM instances, so two persistent workers per construction predicts the observed six-thread increase. This mechanism follows the inspected code and matches the counts, but runtime thread identities or stacks were not captured here; distinguish it from proven attribution of each sampled thread.

The resource follow-up needs a concrete acceptance check: thread counts must return to a bounded range at equivalent completed lifecycle states after repeated cycles and a recovery interval. Add timestamped lifecycle-state markers and capture worker identities before deciding the ownership change. The display/scheduling follow-ups can use the controlled BASIC cadence and four-VM background behavior directly. Allocation traffic is not retained memory, texture submission bytes are not measured GPU transfers, and native UI scope durations are not input-to-photon latency.
