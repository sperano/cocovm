# Audio comparison measurements

Values are medians with minimum–maximum ranges across retained runs.
CPU uses one core = 100%. All throughput captures use release builds.

## Headless core

| Workload | Revision | Runs | Fields/s | Allocations/field | Requested bytes/field |
|---|---|---:|---:|---:|---:|
| dac | before | 3 | 4817.0 (4769.1–4840.0) | 524 (524–524) | 75456 (75456–75456) |
| dac | after | 3 | 5263.2 (5234.5–5268.1) | 0 (0–0) | 0 (0–0) |
| cartridge | before | 3 | 4660.0 (4617.3–4666.8) | 524 (524–524) | 75456 (75456–75456) |
| cartridge | after | 3 | 5038.8 (5016.9–5043.5) | 0 (0–0) | 0 (0–0) |

## Native workload matrix

Whole-process allocations include display and UI work. Foreground DAC has three
retained before runs and four after runs. Cartridge has four per revision. Other
scenarios have three per revision. The first before DAC capture remains in the
raw data but is excluded because compiler overlap during startup is uncertain.

| Scenario/revision | Runs | CPU % | RSS MiB | Fields/s | Allocations/s | Allocated MiB/s | Enqueued MiB/s | VM UI p99 ms | Missing audio frames |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| dac / before | 3 | 21.77 (20.48–21.84) | 175.25 (162.83–175.31) | 60.0 (59.9–60.0) | 97442 (89649–97952) | 80.71 (71.77–81.27) | 55.94 (49.31–56.35) | 0.426 (0.426–0.623) | 101 (0–253) |
| dac / after | 4 | 22.28 (20.87–22.52) | 175.23 (160.30–175.77) | 59.9 (59.9–60.0) | 67699 (61140–68336) | 77.66 (70.14–78.39) | 57.48 (51.91–58.02) | 0.410 (0.393–0.475) | 50 (0–221) |
| cartridge / before | 4 | 21.88 (21.27–22.08) | 175.20 (159.97–175.33) | 60.0 (59.9–60.0) | 97742 (93323–98046) | 81.04 (75.97–81.39) | 56.18 (52.42–56.44) | 0.442 (0.442–0.475) | 85 (0–471) |
| cartridge / after | 4 | 22.44 (20.99–22.61) | 175.09 (160.38–175.91) | 60.0 (59.9–60.0) | 68101 (61817–68387) | 78.12 (70.92–78.45) | 57.82 (52.48–58.06) | 0.426 (0.426–0.459) | 96 (0–343) |
| background / before | 3 | 4.04 (3.93–4.05) | 175.42 (174.48–175.67) | 59.9 (59.9–60.0) | 8320 (8318–8941) | 10.32 (10.32–11.03) | 7.03 (7.03–7.56) | 1.245 (1.245–1.245) | 0 (0–0) |
| background / after | 3 | 4.17 (4.05–4.29) | 170.83 (169.53–175.48) | 59.9 (59.9–60.0) | 8359 (8359–8363) | 9.57 (9.57–9.58) | 7.08 (7.08–7.09) | 1.245 (1.245–1.376) | 0 (0–0) |
| multi-vm / before | 3 | 43.78 (42.97–44.03) | 182.67 (182.08–183.30) | 239.8 (239.8–240.1) | 127683 (126113–129159) | 245.46 (242.48–248.26) | 195.53 (193.10–197.79) | 0.360 (0.360–0.688) | 872 (573–1302) |
| multi-vm / after | 3 | 45.84 (45.74–50.15) | 183.31 (182.86–183.55) | 239.9 (239.8–240.0) | 134575 (134125–140295) | 256.41 (255.56–267.32) | 207.01 (206.32–215.82) | 0.377 (0.377–0.377) | 244 (201–1019) |
| snapshot / before | 3 | 21.93 (21.86–21.94) | 180.95 (180.38–181.58) | 58.6 (58.5–58.7) | 67471 (67024–69028) | 82.53 (82.03–84.30) | 56.02 (55.65–57.32) | 0.377 (0.377–0.377) | 15448 (15426–15611) |
| snapshot / after | 3 | 23.02 (22.64–23.52) | 178.50 (178.45–178.97) | 58.6 (58.6–58.7) | 68040 (67857–68372) | 82.55 (82.35–82.92) | 56.63 (56.47–56.90) | 0.377 (0.377–0.377) | 15065 (14802–15792) |
| lifecycle / before | 3 | 10.82 (10.66–11.87) | 181.17 (180.34–181.34) | 23.1 (22.7–23.2) | 27656 (26685–27809) | 33.58 (32.14–34.38) | 21.71 (20.56–22.43) | 0.393 (0.393–1.049) | 90380 (87784–93512) |
| lifecycle / after | 3 | 10.38 (9.66–10.39) | 181.02 (180.81–182.25) | 22.6 (22.6–22.7) | 26620 (22320–26709) | 33.07 (28.18–33.16) | 21.69 (18.06–21.74) | 0.393 (0.393–1.049) | 93684 (88340–93851) |

## Queue and callback measurements

Queued duration excludes device and operating-system buffers. Callback percentiles
are histogram upper bounds. Hold telemetry is added by the after revision.

| Scenario/revision | VM updates/s | Missing frames % | Maximum queued ms | Wait p99 µs | Hold p99 µs |
|---|---:|---:|---:|---:|---:|
| dac / before | 95.5 (84.2–96.2) | 0.0229 (0.0000–0.0573) | 48.34 (44.22–152.11) | 0.927 (0.799–1.215) | unavailable |
| dac / after | 98.1 (88.6–99.0) | 0.0114 (0.0000–0.0501) | 44.05 (42.88–50.02) | 0.959 (0.863–2.047) | 5.759 (5.631–6.399) |
| cartridge / before | 95.9 (89.5–96.3) | 0.0193 (0.0000–0.1067) | 42.20 (41.41–47.44) | 0.863 (0.767–1.343) | unavailable |
| cartridge / after | 98.7 (89.6–99.1) | 0.0219 (0.0000–0.0778) | 45.77 (42.00–47.94) | 0.975 (0.863–1.087) | 5.759 (5.631–6.655) |
| background / before | 12.0 (12.0–12.9) | 0.0000 (0.0000–0.0000) | 147.37 (141.93–147.85) | 0.799 (0.799–0.863) | unavailable |
| background / after | 12.1 (12.1–12.1) | 0.0000 (0.0000–0.0000) | 141.61 (138.71–142.36) | 0.927 (0.863–1.087) | 5.887 (5.119–5.887) |
| multi-vm / before | 333.7 (329.6–337.6) | 0.0495 (0.0325–0.0738) | 54.65 (49.98–150.41) | 0.671 (0.639–0.735) | unavailable |
| multi-vm / after | 353.3 (352.1–368.3) | 0.0138 (0.0114–0.0578) | 51.61 (48.32–54.69) | 0.639 (0.639–0.671) | 5.631 (5.375–5.887) |
| snapshot / before | 95.6 (95.0–97.8) | 3.5002 (3.4952–3.5413) | 42.04 (41.86–100.11) | 0.799 (0.799–0.799) | unavailable |
| snapshot / after | 96.6 (96.4–97.1) | 3.4174 (3.3538–3.5782) | 42.06 (40.48–43.51) | 0.863 (0.799–0.863) | 5.631 (5.375–5.887) |
| lifecycle / before | 37.1 (35.1–38.4) | 34.7487 (33.9511–36.2382) | 128.32 (37.57–133.17) | 0.863 (0.799–2.303) | unavailable |
| lifecycle / after | 37.1 (30.9–37.2) | 36.1614 (34.1662–36.2258) | 40.50 (37.66–127.44) | 0.895 (0.799–0.927) | 5.887 (5.887–5.887) |

## Allocation-counter control

One release capture per revision/workload with `--no-allocations`, 3 s warmup,
and 10 s measurement. Counts are disabled, so zero counter values in these raw
files are not allocation evidence.

| Workload | Before fields/s | After fields/s | Change |
|---|---:|---:|---:|
| DAC | 4805.3 | 5244.8 | +9.1% |
| Cartridge | 4678.3 | 5056.7 | +8.1% |
