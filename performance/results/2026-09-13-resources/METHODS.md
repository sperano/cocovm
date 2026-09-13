# Resource bounding methods

These measurements compare base commit `06d7f38a2c9eb4065bdba96f1906e4f3b5342705`
with the feature-branch working tree based on that commit. The working-tree
capture is marked dirty because the measurements ran before the final commit.

The host was a Mac15,8 with an Apple M3 Max, 64 GB of memory, and Rust 1.97.0.
The manager stayed focused on a 2560 × 1440 display at 60 Hz. All runs used the
release profile, a three-second warmup, isolated configuration and data
directories, and the synthetic fixtures from the performance harness.

## Record the steady-state matrix

The original short captures ran each checkout three times for 10 seconds:

```sh
python3 scripts/perf/baseline.py native \
  --scenario saved-previews --scenario printer --scenario control-load \
  --repeats 3 --keep-foreground --output /tmp/task228-fast
```

The saved-preview fixture contains 500 suspended machines that share the same
640 × 240 PNG and checkpoint through hard links. The printer fixture contains
2,000 sparse pages. The control fixture runs four clients that repeatedly call
`list_vms`. These captures predate the deterministic scroll driver described
below, so they isolate initial loading and steady-state rendering.

## Run the scroll matrix

The current harness moves through the beginning, middle, and end once per
second. Run the saved-preview and printer scenarios three times for 10 seconds:

```sh
python3 scripts/perf/baseline.py native \
  --scenario saved-previews --scenario printer \
  --repeats 3 --duration 10 --keep-foreground \
  --output /tmp/cocovm-resource-scroll
```

The manager targets rows 0, 250, and 499. The paper window targets printed
pages 0, 1,000, and 1,999. Each request records the requested position; the
rendered acknowledgment records the actual visible range, request-to-render
time, scroll-area CPU time, and resident texture count and byte estimate. A run
fails if the requested row or page is not visible in the acknowledging frame.

The detached base checkout carried the same feature-gated driver and metrics,
without the production virtualization and resource-bound changes. This keeps
the input sequence and measurement boundaries identical across the two builds.

## Run the lifecycle matrix

Run each checkout three times for 60 seconds:

```sh
python3 scripts/perf/baseline.py native \
  --scenario lifecycle --duration 60 --repeats 3 \
  --keep-foreground --output /tmp/task228-lifecycle
```

The branch uses a named nine-step cycle. It covers warm resume, suspended-window
close, cold resume, stop and start, running-window close, and restart. The app
records each completed transition. The runner samples resources every 0.2
seconds and correlates equivalent `restart-after-window-close` states in
`lifecycle.json`.

## Validate overload and cleanup

The real-socket control tests cover queue saturation, 32 concurrent connections
plus an overload attempt, client disconnect, reply timeout, session capacity and
expiry, pending capacity before VM mutation, and server shutdown:

```sh
cargo test -p coco-egui control::server::tests -- --test-threads=1
cargo test -p coco-egui control::http::tests
cargo test -p coco-egui manager::control
```

The server limits are 32 active connections, 16 queued calls, 64 sessions with a
five-minute idle lifetime, 16 deferred calls, and eight UI-thread dispatches per
manager update. Socket I/O has a 30-second timeout, and accepted calls have a
90-second reply limit.

## Interpret the data

Resident set size (RSS), physical footprint, thread count, and file-descriptor
count come from periodic process sampling. Allocation counters cover the whole
process. UI timings measure scoped CPU time, not input-to-photon latency.

GPU allocation is unavailable on this host. Saved-preview estimates use RGBA8
texture size: the base workload can retain about 585.9 MiB for 500 maximum-size
previews, while the branch enforces a 32 MiB saved-preview budget. The measured
fixture uses smaller processed frames, so its 500 resident base textures total
about 293.0 MiB. A 144 dpi printer page is about 8.27 MiB as RGBA8; the cache
retains only visible pages plus one page on each side. These estimates aren't
observed driver allocations.

Lifecycle correlation is observational. Sampling can miss short-lived handles,
threads, and memory peaks. The after runs use a denser sampling interval than the
base runs, so compare equivalent-state plateaus and envelope bounds instead of
individual transition spikes.
