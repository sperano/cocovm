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
"""
import argparse
import re
import sys

LINE = re.compile(
    r"^A=([0-9A-F]{2}) B=([0-9A-F]{2}) ST=([0-9A-F]{2}) SP=([0-9A-F]{2}) ([0-9A-F]{4}): (.*)$"
)
INT3_VECTOR = "F012"


def parse(path):
    out = []
    with open(path, errors="replace") as f:
        for raw in f:
            m = LINE.match(raw.rstrip("\n"))
            if m:
                out.append(m.groups())
    return out


def first_index(entries, pc):
    for i, e in enumerate(entries):
        if e[4] == pc:
            return i
    return None


def compare(a, b, count, loose, a_off=0, b_off=0):
    n = min(count, len(a) - a_off, len(b) - b_off)
    for i in range(n):
        x, y = a[a_off + i], b[b_off + i]
        fields = x[:5] if loose else x
        fields_y = y[:5] if loose else y
        if fields != fields_y:
            print(f"diverge at line {i} (cocovm #{a_off + i}, mame #{b_off + i})")
            for j in range(max(0, i - 3), i + 1):
                print(f"  cocovm: {fmt(a[a_off + j])}")
                print(f"  mame:   {fmt(b[b_off + j])}")
            return False
    print(f"{n} instructions identical")
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
        ok = compare(a, b, args.count, args.loose)
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
