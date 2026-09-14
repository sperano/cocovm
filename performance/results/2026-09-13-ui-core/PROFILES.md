# Remaining core profile

Fresh release profiles at base commit `4d80867` rank scanline rendering first,
bus reads second, and machine/peripheral dispatch third. Each profile is one
approximately 1.68-second macOS `sample` capture. Use the shares to rank work,
not as timing improvements.

| Workload | Rendering | Bus read | Machine step | CPU step | Bit-banger | Cartridge plumbing |
|---|---:|---:|---:|---:|---:|---:|
| BASIC | 40.9% | 24.4% | 7.4% | 4.8% | 3.9% | 8.7% |
| Graphics | 62.1% | 11.7% | 5.8% | 4.4% | 2.4% | 6.4% |
| DAC | 40.3% | 18.6% | 9.1% | 8.4% | 4.2% | 9.0% |
| Cartridge | 37.9% | 16.4% | 8.8% | 5.2% | 3.1% | 7.6% |

Cartridge audio-write handling is an additional 8.1% in its workload. It is
1.5% in DAC. The implementation already rejects unchanged input and preserves
cycle-stamped changes, so the sample does not justify more coalescing.

The selected core optimization replaces RAM-size remainder operations inside
`SystemBus::phys` with an equivalent power-of-two mask. All supported memory
sizes are powers of two, and snapshot restore rejects a RAM length that differs
from the configured size. This change does not alter address decode ordering or
hardware behavior, so it does not need a new hardware-semantics claim.

The profiles do not justify these candidates:

- A scanline cache could address the largest cost, but its key must preserve
  live and latched GIME state, MMU-visible reads, scanline splits, palette and
  border changes, and snapshot invalidation. That requires hardware validation
  and narrower renderer attribution before implementation.
- An isolated GIME RAM-mask experiment improved the graphics median by 0.53%,
  within overlapping run ranges. It also would have narrowed the public
  renderer's support for arbitrary RAM slices. The experiment was discarded.
- CPU opcode, interrupt, HALT, and machine-step changes lack opcode-family or
  hardware-counter attribution. Their semantics and timing are too sensitive
  for a speculative rewrite.
- The core harness bypasses the frontend debugger. Debugger-open and debugger-
  closed native profiles are still needed before adding a fast path.

The raw `sample` files remain local because they contain machine paths. The
checked summary above is derived from captures in
`/private/tmp/task229-before-core-profiles`; the public core JSON retains the
revision, host data, run inputs, and clean throughput results.
