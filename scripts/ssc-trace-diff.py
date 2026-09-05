#!/usr/bin/env python3
"""Diff a cocovm TMS7040 trace against MAME's, instruction by instruction.

Both sides print one line per executed instruction:

    A=00 B=4A ST=00 SP=4C F0AE: MOVP %>3C,P0

cocovm:  cargo run -q -p tms7000 --example firmware_trace -- --max N [--at CYCLE:BYTE] > coco.log
MAME:    debugscript containing
             trace mame.log,:ext:ssc:pic7040,,{tracelog "A=%02X B=%02X ST=%02X SP=%02X ",a,b,st,sp}
             go
         mame coco3 -ext ssc -debug -debugscript that.dbg -video soft -sound none \
             -nothrottle -window -seconds_to_run 6 -autoboot_delay 1 \
             -autoboot_command 'poke 65406,175\n'

Modes:
  boot  compare from the first line until the first divergence (the reset to
        idle-loop path is deterministic on both sides);
  int3  align each file on its first INT3 handler entry (PC F012) and compare
        the following N lines (default 2000).

Register fields and PC are compared; the disassembly text is compared too
unless --loose is given.

Expected noise: MAME's TMS7040 timeslices are chopped at fractional-cycle
boundaries by other devices' timers (the SP0256's 10 kHz stream timer), so
its timer-1 phase wanders by up to a cycle against the instruction grid and
an INT2 entry can land one instruction earlier or later than cocovm's
cycle-exact model. Such shifts are resynchronized and counted. With the
stub board in `firmware_trace`, the INT3 path diverges once the firmware
reads port D (no RAM / data latch behind it); the full-cartridge trace in
coco-core is the tool for that half.

Reference run (2026-09-04, MAME 0.289): boot mode identical for 32,308
instructions with 2 resyncs, up to cocovm's injected host byte.
"""
import argparse
import re
import sys

# cocovm appends " ;cycles=N" (with --cycles); MAME's tracelog can add " C=N"
# (totalcycles) before the PC. Both are optional.
LINE = re.compile(
    r"^A=([0-9A-F]{2}) B=([0-9A-F]{2}) ST=([0-9A-F]{2}) SP=([0-9A-F]{2})"
    r"(?: C=(\d+))? ([0-9A-F]{4}): (.*?)(?: ;cycles=(\d+))?$"
)
INT3_VECTOR = "F012"


def parse(path):
    """Entries: (A, B, ST, SP, PC, disasm, cycles-or-None)."""
    out = []
    with open(path, errors="replace") as f:
        for raw in f:
            m = LINE.match(raw.rstrip("\n"))
            if m:
                a, b, st, sp, mame_cycles, pc, text, coco_cycles = m.groups()
                cycles = mame_cycles or coco_cycles
                out.append((a, b, st, sp, pc, text, int(cycles) if cycles else None))
    return out


def first_index(entries, pc):
    for i, e in enumerate(entries):
        if e[4] == pc:
            return i
    return None


RESYNC_WINDOW = 80
RESYNC_RUN = 8


def find_resync(a, b, i, j):
    """Offsets `(da, db)` within RESYNC_WINDOW at which RESYNC_RUN consecutive
    entries agree again, smallest total offset first."""
    best = None
    for da in range(RESYNC_WINDOW):
        for db in range(RESYNC_WINDOW):
            if best is not None and da + db >= best[0] + best[1]:
                continue
            if i + da + RESYNC_RUN > len(a) or j + db + RESYNC_RUN > len(b):
                continue
            if all(a[i + da + k][:6] == b[j + db + k][:6] for k in range(RESYNC_RUN)):
                best = (da, db)
    return best


def compare(a, b, count, loose, a_off=0, b_off=0):
    """Walk both streams together. A divergence where the two sides took an
    interrupt one instruction apart (MAME's timeslicing wanders by up to a
    cycle) is resynchronized and counted; any other divergence stops."""
    i, j = a_off, b_off
    end_i, end_j = min(len(a), a_off + count), len(b)
    compared = 0
    resyncs = []
    while i < end_i and j < end_j:
        x, y = a[i], b[j]
        same = x[:5] == y[:5] if loose else x[:6] == y[:6]
        if same:
            i += 1
            j += 1
            compared += 1
            continue
        found = find_resync(a, b, i, j)
        if found is None:
            print(f"diverge after {compared} matching instructions (cocovm #{i}, mame #{j})")
            for k in range(3, -1, -1):
                if i - k >= a_off and j - k >= b_off:
                    print(f"  cocovm: {fmt(a[i - k])}")
                    print(f"  mame:   {fmt(b[j - k])}")
            return False
        da, db = found
        resyncs.append((i, j, da, db))
        if len(resyncs) <= 5:
            print(f"resync at cocovm #{i} / mame #{j}: skipped {da} / {db} lines "
                  f"(cocovm at {x[4]}: {x[5]}; mame at {y[4]}: {y[5]})")
        i += da
        j += db
    print(f"{compared} instructions identical, {len(resyncs)} resyncs")
    return True


def fmt(e):
    return f"A={e[0]} B={e[1]} ST={e[2]} SP={e[3]} {e[4]}: {e[5]}"


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("cocovm")
    ap.add_argument("mame")
    ap.add_argument("--mode", choices=["boot", "int3"], default="boot")
    ap.add_argument("--count", type=int, default=2000)
    ap.add_argument("--loose", action="store_true", help="ignore disassembly text")
    args = ap.parse_args()
    a, b = parse(args.cocovm), parse(args.mame)
    print(f"cocovm: {len(a)} lines, mame: {len(b)} lines")
    if args.mode == "boot":
        # MAME's trace begins after its breakpoint stop, i.e. an instruction or
        # two into the boot: align on the first PC MAME logged.
        start = next((i for i, e in enumerate(a[:8]) if e[4] == b[0][4]), 0)
        if start:
            print(f"aligned: cocovm line {start} is MAME line 0")
        ok = compare(a, b, args.count, args.loose, start, 0)
    else:
        ia, ib = first_index(a, INT3_VECTOR), first_index(b, INT3_VECTOR)
        if ia is None or ib is None:
            print(f"no INT3 entry: cocovm #{ia}, mame #{ib}")
            sys.exit(2)
        print(f"INT3 entry: cocovm #{ia}, mame #{ib}")
        ok = compare(a, b, args.count, args.loose, ia, ib)
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
