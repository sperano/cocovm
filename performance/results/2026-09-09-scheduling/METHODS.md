# Reproduce the repaint scheduling comparison

Build each revision in its own checkout with the unchanged installed application assets:

```sh
cargo build --release -p coco-egui --features perf --bin cocovm
```

Use the [performance harness](../../README.md) from the same checkout. Each output
directory must be absent before capture. The commands below use three fresh
processes per scenario, a 3 s warmup, and a 10 s measurement window:

```sh
python3 scripts/perf/baseline.py native \
  --scenario manager-idle --scenario basic-idle \
  --scenario paused --scenario suspended --scenario background \
  --scenario multi-vm --scenario control-load \
  --keep-foreground --repeats 3 --output /tmp/scheduling-rgb

python3 scripts/perf/baseline.py native \
  --scenario basic-idle --scenario paused --scenario suspended \
  --scenario graphics --scenario multi-vm --display tv \
  --keep-foreground --repeats 3 --output /tmp/scheduling-tv

python3 scripts/perf/baseline.py native \
  --scenario graphics --scenario dac --scenario cartridge \
  --keep-foreground --repeats 3 --output /tmp/scheduling-active

python3 scripts/perf/baseline.py native \
  --scenario saved-previews --scenario printer \
  --scenario snapshot --scenario lifecycle \
  --keep-foreground --repeats 3 --output /tmp/scheduling-misc
```

The background scenario ignores `--keep-foreground` and activates Finder halfway
through warmup. Other scenarios use the harness's once-per-sample focus check.
Inspect the scenario focus counts and `foreground.json` before interpreting a run.

Add `--focus-vm` with `--keep-foreground` to raise the first VM window and verify
that macOS reports it as both focused and main. The capture fails unless at least
one check verifies the named window. `--vm-position X Y` also verifies the requested
window position. `--focus-after-warmup` delays the focus change until measurement
starts and therefore creates a transition workload. To measure a binary from another
checkout with this runner, pass that checkout to `--checkout`; revision metadata
and the default binary path then come from that checkout.

Use these commands for the focus-transition and steady physical-display probes:

```sh
python3 scripts/perf/baseline.py native --scenario basic-idle \
  --keep-foreground --focus-vm --focus-after-warmup \
  --output /tmp/focus-transition
python3 scripts/perf/baseline.py native --scenario basic-idle --repeats 1 \
  --keep-foreground --focus-vm --vm-position -1450 80 --output /tmp/panel-120
python3 scripts/perf/baseline.py native --scenario basic-idle --repeats 1 \
  --keep-foreground --focus-vm --vm-position 100 100 --output /tmp/panel-60
```

Run the commands on the unchanged revision first. Rebuild the candidate revision,
and then repeat the commands without changing the host display or audio setup.
Before the full candidate matrix, run one TV repeat and confirm approximately
60 fields/s, continuing UI callbacks, and functioning audio callbacks.

Collect public measurement data without temporary fixtures or device identifiers:

```sh
python3 scripts/perf/collect.py /tmp/scheduling-rgb before-rgb.json
python3 scripts/perf/collect.py /tmp/scheduling-tv before-tv.json
python3 scripts/perf/collect.py /tmp/scheduling-active before-active.json
python3 scripts/perf/collect.py /tmp/scheduling-misc before-misc.json
```

The clean before RGB capture replaces an earlier capture that overlapped repository
checks. The final candidate captures replace measurements from the candidate before
equal-cadence VM deadlines shared an epoch. The retained one-repeat four-VM background
probes document that intermediate regression and the correction. Default comparison
tables exclude both superseded matrices.

Regenerate the detailed tables and validate deferred control waits from the
repository root:

```sh
python3 performance/results/2026-09-09-scheduling/summarize.py
python3 performance/results/2026-09-09-scheduling/validate_controls.py \
  target/release/cocovm
```

## Measurement boundaries

Process CPU and wakeup deltas cover only resource samples wholly inside the
measurement markers. Package idle and interrupt wakeups come from macOS
`proc_pid_rusage`; they don't count every scheduler wakeup. Divide each delta by
`sample_span_seconds` to compare rates.

`manager_update` and `vm_ui_update` counts measure application UI callbacks. They
are useful scheduling and presentation-work proxies, but don't count native
presentation, GPU submissions, or display refreshes. Field counts measure guest
emulation throughput. Audio counters describe the default output stream and retain
the paused and suspended intentional-silence behavior described by the harness.

The control-load clients repeatedly call `list_vms`. Their latency histograms cover
local control request completion, but don't measure keyboard-to-photon input latency
or deferred control `wait` behavior. Validate deferred waits separately with protocol
requests that cover pause timeout, pause and resume, and VM stop. GPU timing and
input-to-photon latency are unavailable.

Core Graphics reports a 120 Hz internal panel at bounds `-1512,0,1512,982` and two
60 Hz external panels. The steady probes position the VM at `-1450,80` on the internal
panel and `100,100` on the main external panel. Nine checks in each run verify the
requested position, `AXMain`, and `AXFocused`; the final egui snapshot also reports
`Performance 0` focused and unminimized. These runs use the host's nominal active
refresh modes. CocoVM doesn't select a display refresh rate, and no global display
setting changes during capture. Controlled `predicted_dt` tests cover adapter behavior
that these two native modes don't exercise.

The unchanged binary didn't produce a verified VM-focused comparison. System Events
timed out while accessing its VM window, and a bounded direct `AXUIElement` attempt
returned `kAXErrorCannotComplete` (`-25204`). The candidate's earlier three-repeat
focus datasets therefore remain transition observations, not matched steady-state
before-and-after comparisons.

Three bounded minimize attempts didn't produce a measurement. Setting
`AXMinimized` didn't persist, the VM window exposed no `AXMinimize` action
(`-1728`), and manager-window focus verification failed. The failure-only attempt
directories are excluded. The control validator also couldn't find an accessible
**Suspend** button for a native mixed suspended-state check. Lifecycle captures
cover suspend and restore operations, and the validator covers mixed paused and
running VMs.

The final comparisons use `before-rgb-clean.json`, `after-rgb-final.json`,
`before-tv.json`, `after-tv-final.json`, `before-active.json`,
`after-active-final.json`, `before-misc.json`, and `after-misc-final.json`.
