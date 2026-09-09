# Display reuse comparison: September 8, 2026

Presentation caching removes conversion and texture uploads for unchanged static
frames. TV processing reuses buffers and produces byte-identical pixels at fixed
settings and noise seeds. The first texture upload no longer clones and enqueues
the image twice.

The captures show lower allocation traffic and fewer texture enqueue requests.
Audio underruns increased in some TV workloads and remain an unresolved limitation.
Clock-driven noise also increases work in paused and suspended TVs compared with
their former, irregular animation.

## Presentation and allocation results

These are medians across three fresh-process captures per revision and workload.
Allocation traffic includes the whole process. Texture counts measure CPU requests,
not actual GPU transfers. [Detailed tables](MEASUREMENTS.md) include ranges, CPU,
resident memory, timing percentiles, field progression, and audio counts.

| Workload | Uploads/s before → after | Allocated MiB/s before → after |
|---|---:|---:|
| BASIC, RGB | 89.46 → 8.80 | 70.83 → 23.53 |
| Active graphics, RGB | 89.58 → 59.37 | 70.92 → 53.22 |
| Four BASIC VMs, RGB | 283.27 → 34.98 | 205.59 → 60.17 |
| BASIC, default TV | 89.39 → 61.59 | 280.27 → 90.42 |
| Active graphics, default TV | 89.67 → 73.27 | 281.16 → 104.27 |
| Four BASIC VMs, default TV | 278.04 → 230.70 | 853.47 → 307.15 |

All three paused-monitor and suspended-monitor captures have zero uploads and
zero display conversions after warmup. Their unrelated repaints previously caused
three or four uploads per capture. Deterministic frontend tests also cover static
color and black-and-white TVs with scanlines enabled and noise disabled.

Continuously running fixtures retain approximately 60 fields/s per VM. CPU medians
fall from 20.77% to 18.63% of one core for BASIC RGB, 38.47% to 33.95% for four RGB
VMs, and 30.67% to 25.86% for default TV. These short captures establish observations
on this host, not universal CPU or latency guarantees. Snapshot and lifecycle runs
complete ten operations each. Their pauses and synchronous work remain part of
those workloads.

## Isolated TV processing

Three release measurements compare the old and reused pipelines in the same test
executable. Each warms the buffers and processes 120 fixed-seed frames. The reused
processor requests zero allocations and bytes in every case. Timing values below
are medians; [raw measurements](isolated.json) retain each repeat.

| TV processing at 640 × 240 | Allocations/frame before → after | Requested bytes/frame before → after | µs/frame before → after |
|---|---:|---:|---:|
| Color, no scanlines | 1 → 0 | 614,400 → 0 | 727.9 → 699.7 |
| Color, default scanlines | 2 → 0 | 1,843,200 → 0 | 879.1 → 837.4 |
| Black and white, no scanlines | 2 → 0 | 1,228,800 → 0 | 1251.4 → 1205.7 |
| Black and white, default scanlines | 3 → 0 | 2,457,600 → 0 | 1408.2 → 1359.3 |

The isolated timing improvement is approximately 3.5–4.7%. Changed presentations
still allocate an owned `ColorImage` for egui. A source cache and TV scratch buffers
remain allocated between frames; [methods](METHODS.md) describe their memory cost.

## Animation and audio limitations

Noise advances on a 60 Hz clock grid. A paused or suspended TV with noise enabled
therefore requests presentations without waiting for incidental UI activity.
These captures produce approximately 60 uploads/s and 89 UI updates/s, using about
24.5% of one core, compared with approximately 0.4 uploads/s and 0.2–0.3% CPU before.
Static displays remain event-driven. Framebuffer changes between noise ticks can
cause additional uploads in running TV graphics workloads.

Default-TV missing audio frames are 50, 0, and 571 before versus 363, 491, and 523
after in the main matrix. A later consecutive before/after recheck records 0 versus
316 missing frames. Four-TV medians also rise from 555 to 1627. The captures report
zero audio overflow, and deterministic audio tests pass, but unchanged audio code
does not rule out a timing regression. Audio reliability preservation is unresolved;
these results must not be presented as a reliability improvement. Snapshot and
lifecycle missing-frame counts include deliberate resets and silent intervals.

## Reproduce and validate

[Methods and commands](METHODS.md) describe capture and table regeneration. The
75 native captures use 3 s warmup and 10 s measurement. The order is the full before
matrix, before TV combinations, after matrix, after TV combinations, then the
BASIC replacement and TV recheck. The initial before BASIC capture is excluded
from comparison because a test build might have overlapped startup; its raw data
remains published. The later BASIC capture replaces it, leaving three per revision.
No compiler or test process ran during the subsequent captures.

Before source: `2adcf2a`, after source:
`036b734e9c8bead013154311f94c05265e4b0ac4`. The after checkout's dirty flag reflects
result files and documentation only. Host: Apple M3 Max, 64 GB RAM, macOS 26.6.2,
Rust 1.97.0. Audio uses MacBook Pro Speakers at 44,100 Hz, stereo. The host has a
120 Hz internal panel and two 60 Hz external panels. Viewport-to-panel mapping was
not captured; running UI rates above 60 updates/s exercise redundant presentations.
Focus observations and transient unfocused updates are included in the raw data.
GPU execution time and input-to-photon latency are unavailable.

Validation passed: 1,416 workspace tests; 500 frontend tests with `perf,debug-ui`;
workspace checks; default and feature Clippy with warnings denied; and formatting.
The isolated initial-upload counter test and three TV allocation measurements also
pass. Regression coverage includes debugger instruction and scanline steps,
rendered memory changes, pause/resume, suspend/resume, reset, power-cycle, snapshot
restore, dimensions, texture filtering, overscan, and window resizing. Socket tests
required an unrestricted rerun after the sandbox denied local binds. Rust-analyzer
reports no errors and three feature-dependent unused-variable warnings in unchanged
manager code. Code review found no concrete implementation defect; the audio
measurement limitation remains explicit.
