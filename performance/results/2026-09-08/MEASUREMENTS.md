# Detailed baseline measurements

Each cell is the median (minimum–maximum) of three fresh processes, except the two single profiler captures. CPU uses one core = 100%. RSS is the peak of external samples per run. Allocation bytes describe traffic, not retained memory. See [results and limitations](RESULTS.md) before comparing values.

## native

[Raw measurements](native.json).

| Scenario | Runs | CPU % | RSS MiB | Fields/s | Allocations/s | Allocated MiB/s | Enqueued MiB/s | VM UI p99 ms | Missing audio frames |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| background | 3 | 4.29 (4.18–4.30) | 159.98 (158.77–160.09) | 60.0 (59.9–60.0) | 8380 (8380–8380) | 10.39 (10.39–10.39) | 7.08 (7.08–7.08) | 1.311 (1.245–1.311) | 0 (0–0) |
| basic-idle | 3 | 18.73 (11.61–22.04) | 160.33 (159.62–161.36) | 59.3 (59.3–59.9) | 50763 (28365–57978) | 58.84 (33.26–66.78) | 42.94 (24.01–48.74) | 1.114 (1.049–1.442) | 0 (0–377) |
| cartridge | 3 | 21.11 (21.03–21.24) | 159.20 (158.86–159.31) | 59.9 (59.9–60.0) | 93934 (93763–94069) | 76.69 (76.46–76.85) | 52.96 (52.79–53.08) | 0.475 (0.442–0.508) | 40 (11–55) |
| control-load | 3 | 25.42 (25.41–25.43) | 159.17 (159.08–159.25) | 60.0 (59.9–60.0) | 90586 (90500–90635) | 79.74 (79.65–79.75) | 52.42 (52.37–52.43) | 0.459 (0.459–0.475) | 68 (0–430) |
| dac | 3 | 21.09 (21.02–21.20) | 158.98 (158.94–159.81) | 60.0 (59.9–60.0) | 93760 (93723–93904) | 76.46 (76.45–76.64) | 52.79 (52.79–52.93) | 0.492 (0.475–0.492) | 334 (0–376) |
| graphics | 3 | 21.14 (20.96–23.35) | 159.28 (158.77–159.55) | 59.9 (59.9–60.0) | 62326 (62324–62823) | 72.14 (72.14–72.30) | 52.79 (52.79–52.84) | 0.475 (0.475–0.492) | 0 (0–0) |
| lifecycle | 3 | 10.38 (10.22–10.39) | 164.17 (163.50–166.08) | 22.5 (22.5–22.6) | 25851 (25563–25911) | 32.36 (32.05–32.46) | 21.02 (20.79–21.10) | 0.426 (0.377–0.492) | 94011 (93483–94123) |
| manager-idle | 3 | 1.25 (0.00–2.28) | 152.84 (151.16–154.00) | 0.0 (0.0–0.0) | 2022 (342–3656) | 0.53 (0.09–0.96) | 0.00 (0.00–0.00) | unavailable | unavailable |
| multi-vm | 3 | 39.81 (39.56–39.91) | 167.86 (167.58–168.50) | 239.8 (239.8–239.9) | 113520 (113349–113884) | 218.54 (218.22–219.23) | 173.79 (173.54–174.35) | 0.475 (0.475–0.492) | 327 (83–361) |
| paused | 3 | 0.21 (0.10–0.21) | 158.03 (157.42–158.28) | 0.0 (0.0–0.0) | 276 (207–276) | 0.32 (0.24–0.32) | 0.23 (0.18–0.23) | 0.172 (0.147–0.205) | 441344 (441344–441856) |
| printer | 3 | 25.70 (25.64–25.97) | 200.61 (200.36–200.78) | 60.0 (59.9–60.0) | 67194 (66932–67603) | 68.91 (68.65–69.33) | 47.27 (47.09–47.56) | 12.583 (12.583–13.107) | 153 (67–186) |
| saved-previews | 3 | 0.00 (0.00–0.12) | 782.80 (777.86–783.20) | 0.0 (0.0–0.0) | 4506 (4505–6757) | 0.49 (0.49–0.74) | 0.00 (0.00–0.00) | unavailable | unavailable |
| snapshot | 3 | 20.88 (20.85–20.90) | 164.73 (163.69–164.78) | 58.6 (58.5–58.7) | 63136 (63069–63271) | 77.64 (77.56–77.79) | 52.42 (52.36–52.53) | 0.410 (0.377–0.442) | 15927 (15837–16116) |
| suspended | 3 | 0.21 (0.10–0.21) | 159.73 (159.14–159.97) | 0.0 (0.0–0.0) | 225 (225–374) | 0.24 (0.24–0.40) | 0.18 (0.18–0.29) | 0.180 (0.180–0.197) | 441344 (440832–441344) |
| tv | 3 | 31.13 (31.06–31.58) | 163.16 (163.14–163.17) | 60.0 (59.9–60.0) | 62706 (62695–63255) | 284.20 (284.15–286.69) | 105.92 (105.90–106.84) | 1.442 (1.442–1.573) | 94 (0–202) |

## core

[Raw measurements](core.json).

| Scenario | Runs | CPU % | RSS MiB | Fields/s | Allocations/s | Allocated MiB/s | Enqueued MiB/s | VM UI p99 ms | Missing audio frames |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| basic-idle | 3 | 99.95 (99.91–99.97) | 2.59 (2.59–2.59) | 6112.7 (6086.1–6136.6) | 0 (0–0) | 0.00 (0.00–0.00) | unavailable | unavailable | unavailable |
| cartridge | 3 | 99.78 (99.72–99.93) | 2.66 (2.64–2.66) | 4648.8 (4636.0–4667.7) | 2435996 (2429283–2445870) | 334.53 (333.61–335.89) | unavailable | unavailable | unavailable |
| dac | 3 | 99.89 (99.78–99.90) | 2.62 (2.61–2.64) | 4769.4 (4768.5–4783.3) | 2499142 (2498698–2506463) | 343.20 (343.14–344.21) | unavailable | unavailable | unavailable |
| graphics | 3 | 99.79 (99.76–99.96) | 2.55 (2.55–2.55) | 4138.1 (4136.7–4139.2) | 0 (0–0) | 0.00 (0.00–0.00) | unavailable | unavailable | unavailable |

## focused

[Raw measurements](focused.json).

| Scenario | Runs | CPU % | RSS MiB | Fields/s | Allocations/s | Allocated MiB/s | Enqueued MiB/s | VM UI p99 ms | Missing audio frames |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| basic-idle | 3 | 20.98 (20.87–21.21) | 153.97 (153.34–159.14) | 60.0 (59.9–60.0) | 63263 (62927–63384) | 73.22 (72.83–73.36) | 53.58 (53.30–53.69) | 0.377 (0.377–0.393) | 24 (0–75) |
| manager-idle | 3 | 0.00 (0.00–0.00) | 150.17 (149.81–150.81) | 0.0 (0.0–0.0) | 65 (65–65) | 0.02 (0.02–0.02) | 0.00 (0.00–0.00) | unavailable | unavailable |

## core-no-counts

[Raw measurements](core-no-counts.json).

| Scenario | Runs | CPU % | RSS MiB | Fields/s | Allocations/s | Allocated MiB/s | Enqueued MiB/s | VM UI p99 ms | Missing audio frames |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| dac | 3 | 99.92 (99.92–99.95) | 2.61 (2.61–2.61) | 4823.6 (4820.0–4829.7) | unavailable | unavailable | unavailable | unavailable | unavailable |

## native-no-counts

[Raw measurements](native-no-counts.json).

| Scenario | Runs | CPU % | RSS MiB | Fields/s | Allocations/s | Allocated MiB/s | Enqueued MiB/s | VM UI p99 ms | Missing audio frames |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| basic-idle | 3 | 20.93 (20.85–20.95) | 159.14 (159.08–159.88) | 59.9 (59.9–60.0) | unavailable | unavailable | unavailable | unavailable | unavailable |

## multi-tv

[Raw measurements](multi-tv.json).

| Scenario | Runs | CPU % | RSS MiB | Fields/s | Allocations/s | Allocated MiB/s | Enqueued MiB/s | VM UI p99 ms | Missing audio frames |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| multi-vm | 3 | 67.55 (67.44–67.56) | 174.47 (173.59–175.06) | 240.0 (239.8–240.0) | 101387 (100368–101504) | 812.00 (803.85–812.89) | 308.75 (305.64–309.09) | 1.573 (1.573–1.573) | 0 (0–143) |

## multi-background

[Raw measurements](multi-background.json).

| Scenario | Runs | CPU % | RSS MiB | Fields/s | Allocations/s | Allocated MiB/s | Enqueued MiB/s | VM UI p99 ms | Missing audio frames |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| background | 3 | 19.57 (19.41–20.37) | 168.70 (167.95–168.73) | 239.8 (239.4–239.9) | 45602 (42872–46669) | 89.73 (84.56–91.76) | 69.80 (65.62–71.44) | 1.114 (1.114–1.114) | 228 (0–855) |

## static-tv

[Raw measurements](static-tv.json).

| Scenario | Runs | CPU % | RSS MiB | Fields/s | Allocations/s | Allocated MiB/s | Enqueued MiB/s | VM UI p99 ms | Missing audio frames |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| paused | 3 | 0.10 (0.10–0.21) | 164.52 (164.27–164.91) | 0.0 (0.0–0.0) | 208 (138–208) | 0.94 (0.63–0.94) | 0.35 (0.23–0.35) | 1.311 (1.180–1.311) | 441344 (440832–441344) |
| suspended | 3 | 0.21 (0.21–0.21) | 167.25 (167.16–168.11) | 0.0 (0.0–0.0) | 226 (151–226) | 0.95 (0.63–0.95) | 0.35 (0.23–0.35) | 1.311 (1.180–1.311) | 441344 (440832–441344) |

## lifecycle-long

[Raw measurements](lifecycle-long.json).

| Scenario | Runs | CPU % | RSS MiB | Fields/s | Allocations/s | Allocated MiB/s | Enqueued MiB/s | VM UI p99 ms | Missing audio frames |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| lifecycle | 3 | 12.02 (12.02–12.03) | 165.81 (165.81–167.17) | 27.8 (27.8–27.8) | 31050 (31024–31127) | 38.33 (38.30–38.41) | 25.35 (25.32–25.41) | 0.459 (0.459–0.459) | 488423 (487885–489832) |

## profile-tv

[Raw measurements](profile-tv.json).

| Scenario | Runs | CPU % | RSS MiB | Fields/s | Allocations/s | Allocated MiB/s | Enqueued MiB/s | VM UI p99 ms | Missing audio frames |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| tv | 1 | 30.55 (30.55–30.55) | 165.77 (165.77–165.77) | 59.7 (59.7–59.7) | 60769 (60769–60769) | 275.47 (275.47–275.47) | 102.65 (102.65–102.65) | 1.966 (1.966–1.966) | 51 (51–51) |

## profile-core

[Raw measurements](profile-core.json).

| Scenario | Runs | CPU % | RSS MiB | Fields/s | Allocations/s | Allocated MiB/s | Enqueued MiB/s | VM UI p99 ms | Missing audio frames |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| dac | 1 | 98.63 (98.63–98.63) | 2.61 (2.61–2.61) | 4710.3 (4710.3–4710.3) | 2468209 (2468209–2468209) | 338.96 (338.96–338.96) | unavailable | unavailable | unavailable |

## dev-core

[Raw measurements](dev-core.json).

| Scenario | Runs | CPU % | RSS MiB | Fields/s | Allocations/s | Allocated MiB/s | Enqueued MiB/s | VM UI p99 ms | Missing audio frames |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| dac | 3 | 100.00 (99.99–100.11) | 2.59 (2.59–2.59) | 3931.7 (3921.5–3950.1) | 2060216 (2054841–2069873) | 282.93 (282.19–284.25) | unavailable | unavailable | unavailable |

## dev-native

[Raw measurements](dev-native.json).

| Scenario | Runs | CPU % | RSS MiB | Fields/s | Allocations/s | Allocated MiB/s | Enqueued MiB/s | VM UI p99 ms | Missing audio frames |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| basic-idle | 3 | 24.67 (24.49–25.49) | 162.03 (161.34–162.06) | 59.9 (59.8–60.0) | 85657 (85188–86109) | 74.99 (74.59–75.39) | 53.07 (52.78–53.35) | 0.623 (0.623–0.688) | 403 (127–496) |

## Native scope latency

Release primary matrix, replacing manager and BASIC with focused reruns. Percentiles are the median of each run’s histogram upper bound in milliseconds. These scopes are nested and can include backend waits. They are not GPU or input-to-photon latency. All run ranges and maxima remain in the raw JSON.

| Scenario | VM p95 | VM p99 | Manager p95 | Manager p99 | Display p95 | Display p99 |
|---|---:|---:|---:|---:|---:|---:|
| background | 1.245 | 1.311 | 8.389 | 9.961 | 0.098 | 0.115 |
| basic-idle | 0.360 | 0.377 | 13.107 | 13.631 | 0.098 | 0.106 |
| cartridge | 0.426 | 0.475 | 13.107 | 13.631 | 0.098 | 0.111 |
| control-load | 0.377 | 0.459 | 14.156 | 14.680 | 0.102 | 0.111 |
| dac | 0.410 | 0.492 | 13.107 | 13.631 | 0.098 | 0.111 |
| graphics | 0.426 | 0.475 | 13.107 | 13.631 | 0.098 | 0.102 |
| lifecycle | 0.360 | 0.426 | 14.156 | 14.680 | 0.102 | 0.119 |
| manager-idle | unavailable | unavailable | 0.051 | 0.051 | unavailable | unavailable |
| multi-vm | 0.360 | 0.475 | 15.729 | 18.874 | 0.098 | 0.106 |
| paused | 0.172 | 0.172 | 12.059 | 12.059 | 0.123 | 0.123 |
| printer | 12.583 | 12.583 | 15.204 | 17.826 | 0.098 | 0.106 |
| saved-previews | unavailable | unavailable | 1.245 | 1.245 | unavailable | unavailable |
| snapshot | 0.360 | 0.410 | 13.631 | 14.156 | 0.098 | 0.106 |
| suspended | 0.180 | 0.180 | 10.486 | 10.486 | 0.127 | 0.127 |
| tv | 1.376 | 1.442 | 13.107 | 13.631 | 1.114 | 1.180 |
