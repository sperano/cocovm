# Resource bounding results

The changes remove machine-library and printer work that scaled with total
collection size, stop lifecycle thread accumulation, and bound every control
server resource. The collected measurements are in the six JSON files in this
directory. See [the methods](METHODS.md) for commands and limitations.

## Compare the short workloads

Values are medians across three runs. Ranges are in the raw measurements.

| Workload | Metric | Base | Branch | Change |
|---|---|---:|---:|---:|
| 500 previews | Peak sampled RSS | 782.97 MiB | 172.59 MiB | -77.9% |
| 500 previews | Cold manager-update maximum | 180.66 ms | 2.71 ms | -98.5% |
| 500 previews | Warmup allocations | 106,442 | 9,127 | -91.4% |
| 2,000 printer pages | CPU, one core = 100% | 24.36% | 5.23% | -78.5% |
| 2,000 printer pages | Allocations per second | 65,890 | 10,442 | -84.2% |
| 2,000 printer pages | VM UI p99 | 11.53 ms | 9.96 ms | -13.6% |
| Control load | Successful calls per run | 3,160–3,188 | 3,143–3,176 | comparable |
| Control load | Client p95 bucket | 15 ms | 16 ms | +1 ms |
| Control load | Client p99 bucket | 16–18 ms | 17–20 ms | +0–2 ms |

The preview cache stays within its 32 MiB RGBA8 estimate, excluding shared live
VM framebuffer textures. Each manager update attempts at most four decodes, and
the loader rejects images larger than 640 × 480 or an 8 MiB decode allocation.
Missing and invalid files remain negatively cached. Evicted valid previews can
load again when they return near the viewport.

The printer workload's peak RSS increased from a 201.34–202.94 MiB range to
226.84–227.47 MiB. The new cache eagerly retains the visible range plus its two
page margin, which costs about 33.1 MiB at the default window size. Retention now
depends on viewport height rather than the 2,000-page roll. Original dot data is
unchanged and remains until the user tears off the paper.

Normal four-client control traffic remains usable after adding bounds. Peak
thread counts changed from 22–25 to 20–22. Peak file descriptors increased from
17 to 25 because each active connection retains shutdown and disconnect-probe
handles; the connection cap bounds them. Control tests verify HTTP 503 recovery,
bounded queue errors, disconnect reclamation, and joined shutdown; the HTTP 503,
disconnect, and shutdown cases use real sockets.

## Compare beginning, middle, and end scrolling

Every requested target rendered successfully across 30 saved-preview phases
and 30 printer phases per checkout. Values below combine three fresh-process
runs; maxima are the worst individual phase.

| Workload | Metric | Base | Branch |
|---|---|---:|---:|
| 500 previews | Resident textures | 500 | 15–49 |
| 500 previews | Maximum resident texture estimate | 292.97 MiB | 28.71 MiB |
| 500 previews | Median scroll-area draw | 1.21 ms | 0.07 ms |
| 500 previews | Maximum request-to-render | 1.42 ms | 2.00 ms |
| 500 previews | Median peak sampled RSS | 780.39 MiB | 197.16 MiB |
| 2,000 printer pages | Resident page textures | 3–4 | 3–4 |
| 2,000 printer pages | Maximum resident texture estimate | 33.06 MiB | 33.06 MiB |
| 2,000 printer pages | Median scroll-area draw | 9.94 ms | 9.84 ms |
| 2,000 printer pages | Maximum request-to-render | 11.29 ms | 12.89 ms |

The branch pays bounded decode work when a large preview jump reaches uncached
rows, which explains its 2.00 ms worst request despite the much lower median.
The 32 MiB policy remained satisfied after every phase. For printer jumps,
rasterizing three or four newly exposed pages dominates both builds; the worst
branch sample was 1.60 ms slower. The production improvement appears between
jumps: the steady-state matrix above shows 78.5% lower CPU and 84.2% fewer
allocations because the branch no longer constructs 2,000 page widgets on every
frame.

## Compare lifecycle resources

The base 60-second runs accumulated threads throughout each process:

| Resource | Base, three runs | Branch, three runs |
|---|---:|---:|
| Thread envelope | 14–75 | 11–18 |
| File-descriptor envelope | 8 | 8 |
| Threads at equivalent completed cycles | increasing by six per old cycle | 15–17, stable after warmup |
| RSS change, penultimate to final completed cycle | unavailable | 16–144 KiB |

The shared gamepad host removes the per-VM `gilrs` backend, disables unused
force feedback, and drains host events from the manager every 100 ms even when
no VM is live. File descriptors remain fixed at eight. Equivalent recovery
points hold at 15 and 16 threads in two runs. The third warms from 15 to 17
threads between its first two points, then remains at 17 through the sixth; no
run shows per-cycle accumulation. RSS grows by 1.00–2.41 MiB from the first
through sixth recovery point, but the final-cycle increment falls to 16–144
KiB. This supports a warmed resource plateau without claiming that RSS is
perfectly flat.

Lifecycle median CPU falls from 11.53% to 9.16%, and median allocation traffic
falls from 15.71 to 12.05 MiB/s. The revised nine-step workload performs fewer
cold VM constructions per minute than the old six-step workload, so these two
rates aren't an isolated gamepad comparison. The stable equivalent-state thread
and handle counts are the applicable ownership result.

## State remaining limits

The scroll driver uses deterministic one-second jumps rather than synthesized
wheel or trackpad gestures. Its draw duration is scoped CPU time, not
input-to-photon latency. Native GPU allocation and GPU execution time remain
unavailable, so resident texture bytes are egui's RGBA estimates. The control
stress checks assert hard bounds and recovery in process, but only the normal
four-client workload has host-level CPU, memory, thread, handle, and latency
samples.
