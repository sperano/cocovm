# Capture manifest

Run from the repository root after installing the application ROM assets.
These commands reproduce the published datasets. Use a new output directory for
each run. The original captures used source `0500187d6aca583a37378822cbb08f7af3df045c`.
Avoid simultaneous benchmarks, builds, and other sustained host work.

```sh
cargo build --release -p coco-core --example perf_baseline
cargo build --release -p coco-egui --features perf --bin cocovm
python3 scripts/perf/baseline.py native --keep-foreground --output /tmp/cocovm-final-native
python3 scripts/perf/baseline.py core --output /tmp/cocovm-final-core
python3 scripts/perf/baseline.py core --scenario dac --no-allocations --output /tmp/cocovm-final-core-no-counts
python3 scripts/perf/baseline.py native --scenario manager-idle --scenario basic-idle --keep-foreground --output /tmp/cocovm-final-focused
python3 scripts/perf/baseline.py native --scenario basic-idle --keep-foreground --no-telemetry --output /tmp/cocovm-final-native-no-counts
python3 scripts/perf/baseline.py native --scenario multi-vm --display tv --keep-foreground --output /tmp/cocovm-final-multi-tv
python3 scripts/perf/baseline.py native --scenario background --vm-count 4 --output /tmp/cocovm-final-multi-background
python3 scripts/perf/baseline.py native --scenario paused --scenario suspended --display tv --keep-foreground --output /tmp/cocovm-final-static-tv
python3 scripts/perf/baseline.py native --scenario lifecycle --duration 60 --keep-foreground --output /tmp/cocovm-final-lifecycle-long
python3 scripts/perf/baseline.py native --scenario tv --keep-foreground --sample-profile --repeats 1 --output /tmp/cocovm-final-profile-tv
python3 scripts/perf/baseline.py core --scenario dac --sample-profile --repeats 1 --output /tmp/cocovm-final-profile-core
cargo build -p coco-egui --features perf --bin cocovm
cargo build -p coco-core --example perf_baseline
python3 scripts/perf/baseline.py core --scenario dac --profile dev --output /tmp/cocovm-final-dev-core
python3 scripts/perf/baseline.py native --scenario basic-idle --profile dev --keep-foreground --output /tmp/cocovm-final-dev-native
```

The default warmup, duration, and repetitions are 3 seconds, 10 seconds, and 3.
The manifest names map directly to adjacent JSON files: `cocovm-final-core` maps
to `core.json`, for example. Collect a directory without ROMs or generated fixtures:

```sh
python3 scripts/perf/collect.py /tmp/cocovm-final-core /tmp/core.json
python3 scripts/perf/report.py /tmp/cocovm-final-core
```

`inputs.json` in each original run, copied into each collected JSON run, identifies
the generated image, ROM inventory, and actual core ROM hash. Native fixture
parameters appear in `metrics.scenario`. The generated image and fixture
instructions are in the [harness documentation](../../README.md).

The original main native capture had mixed focus for manager and BASIC. The
focused reruns retain the same inputs and sampling configuration; use those for
comparison. All original runs remain published instead of discarding inconvenient
results. Inspect observed focus again on your own host.
