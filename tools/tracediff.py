#!/usr/bin/env python3
"""Trace-diff cocovm against MAME (or two cocovm traces) instruction-by-instruction.

Both sides log CPU pre-instruction state (PC + A B X Y U S DP CC). This tool
auto-aligns the two streams (MAME's trace usually omits the reset entry that
cocovm logs, so there is a small leading offset), finds the first full-state
divergence, and enumerates every distinct divergence "site" grouped by the PC
of the producing instruction so interrupt noise and cascades collapse into a
few classes.

Producing the two traces
------------------------
cocovm (interrupts OFF, deterministic cold start):
    cargo run -q -p coco-core --example trace -- <N> > coco.trace

MAME (register-augmented tracelog, stops at first IRQ vector so the window is
the deterministic pre-interrupt path):
    # debugscript trace.dbg:
    trace mame.trace,maincpu,noloop,{tracelog "  A=%02X B=%02X X=%04X Y=%04X U=%04X S=%04X DP=%02X CC=%02X",a,b,x,y,u,s,dp,cc}
    bpset 0xFEF7          # <- first ROM IRQ vector; adjust per ROM
    go
    trace off
    quit

    mame coco3 -ext "" -rompath <ROMS> -video none -sound none -nothrottle \
        -debug -debugscript trace.dbg -window

Then:
    tools/tracediff.py coco.trace mame.trace
"""
import argparse
import re
import sys
from collections import OrderedDict

FIELDS = ["PC", "A", "B", "X", "Y", "U", "S", "DP", "CC"]

# cocovm:  "8C1D:  A=00 B=00 X=0000 Y=0000 U=0000 S=0000 DP=00 CC=50"
COCO_RE = re.compile(
    r"^([0-9A-Fa-f]{2,4}):\s+A=(..) B=(..) X=(....) Y=(....) "
    r"U=(....) S=(....) DP=(..) CC=(..)"
)
# MAME tracelog: "  A=00 B=00 ... CC=508C1D: LDA    #$0A"  (regs prefixed, PC glued to CC)
MAME_RE = re.compile(
    r"^\s*A=(..) B=(..) X=(....) Y=(....) U=(....) S=(....) DP=(..) "
    r"CC=(..)([0-9A-Fa-f]{2,4}):\s*(.*)$"
)


class Row:
    __slots__ = ("regs", "text")

    def __init__(self, regs, text):
        self.regs = regs  # dict field -> uppercase hex string
        self.text = text  # raw-ish display line

    def __getitem__(self, f):
        return self.regs[f]


def load(path):
    rows = []
    for ln in open(path):
        ln = ln.rstrip("\n")
        m = COCO_RE.match(ln)
        if m:
            pc, a, b, x, y, u, s, dp, cc = m.groups()
            regs = dict(zip(FIELDS, (pc, a, b, x, y, u, s, dp, cc)))
            rows.append(Row({k: v.upper() for k, v in regs.items()}, ln))
            continue
        m = MAME_RE.match(ln)
        if m:
            a, b, x, y, u, s, dp, cc, pc, ins = m.groups()
            regs = dict(zip(FIELDS, (pc, a, b, x, y, u, s, dp, cc)))
            rows.append(Row({k: v.upper() for k, v in regs.items()},
                            f"{pc}: {ins}"))
    return rows


def norm_pc(rows):
    for r in rows:
        r.regs["PC"] = r.regs["PC"].zfill(4)


def find_offset(a, b, run=200, window=4):
    """Return (ai, bi) start indices giving a stable >=run PC match, or None."""
    for ai in range(window):
        for bi in range(window):
            ok = True
            for k in range(run):
                if ai + k >= len(a) or bi + k >= len(b):
                    ok = False
                    break
                if a[ai + k]["PC"] != b[bi + k]["PC"]:
                    ok = False
                    break
            if ok:
                return ai, bi
    return None


def diff_fields(ra, rb):
    return [f for f in FIELDS if ra[f] != rb[f]]


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("left", help="cocovm (or any) trace")
    ap.add_argument("right", help="MAME (or any) trace")
    ap.add_argument("--context", type=int, default=10,
                    help="instructions of agreeing context before first divergence")
    ap.add_argument("--pc-only", action="store_true",
                    help="compare PC only (ignore register divergences)")
    ap.add_argument("--max-classes", type=int, default=20,
                    help="max distinct divergence sites to list")
    args = ap.parse_args()

    a = load(args.left)
    b = load(args.right)
    norm_pc(a)
    norm_pc(b)
    print(f"left  {args.left}: {len(a)} rows, first PC {a[0]['PC'] if a else '-'}")
    print(f"right {args.right}: {len(b)} rows, first PC {b[0]['PC'] if b else '-'}")
    if not a or not b:
        sys.exit("empty trace")

    off = find_offset(a, b)
    if off is None:
        sys.exit("no stable PC alignment in first 4x4 window")
    ai0, bi0 = off
    print(f"aligned at left[{ai0}] / right[{bi0}] (leading offset "
          f"{ai0}-{bi0})\n")

    cmp_fields = ["PC"] if args.pc_only else FIELDS

    # walk while PC agrees (PC divergence = hard desync, e.g. interrupt); collect
    # register-level divergence sites along the way.
    sites = OrderedDict()  # producing-PC -> (first_idx, set(fields), example)
    first_full = None
    i = 0
    pc_break = None
    while ai0 + i < len(a) and bi0 + i < len(b):
        ra, rb = a[ai0 + i], b[bi0 + i]
        if ra["PC"] != rb["PC"]:
            pc_break = i
            break
        d = [f for f in cmp_fields if f != "PC" and ra[f] != rb[f]]
        if d:
            if first_full is None:
                first_full = i
            # producing instruction is the previous row's PC (this row's state
            # is the result of executing prev instruction)
            prod = a[ai0 + i - 1]["PC"] if i > 0 else ra["PC"]
            key = prod
            if key not in sites:
                sites[key] = [i, set(d), (ra, rb, a[ai0 + i - 1] if i > 0 else None)]
            else:
                sites[key][1].update(d)
        i += 1

    n = i
    print(f"compared {n} aligned instructions before "
          f"{'PC desync' if pc_break is not None else 'end of stream'}\n")

    if first_full is None and pc_break is None:
        print("*** FULL CPU STATE IDENTICAL across the whole aligned window ***")
        return
    if first_full is None:
        print("No register divergence before PC desync.")
    else:
        idx = first_full
        ra, rb = a[ai0 + idx], b[bi0 + idx]
        print("=== FIRST FULL-STATE DIVERGENCE ===")
        print(f"aligned idx {idx}  (left line ~{ai0 + idx + 1}, right line ~{bi0 + idx + 1})")
        print(f"differing fields: {diff_fields(ra, rb)}")
        prod = a[ai0 + idx - 1] if idx > 0 else None
        if prod:
            print(f"produced by: {prod.text}")
        print(f"--- {args.context} preceding (agree) ---")
        for k in range(max(0, idx - args.context), idx):
            print(f"  L {a[ai0 + k].text}")
            print(f"  R {b[bi0 + k].text}")
        print("--- divergence point ---")
        print(f"  L {ra.text}   regs {ra.regs}")
        print(f"  R {rb.text}   regs {rb.regs}")

    if pc_break is not None:
        ra, rb = a[ai0 + pc_break], b[bi0 + pc_break]
        print(f"\n=== PC DESYNC at aligned idx {pc_break} ===")
        prod = a[ai0 + pc_break - 1] if pc_break > 0 else None
        if prod:
            print(f"after: {prod.text}")
        print(f"  L -> {ra.text}")
        print(f"  R -> {rb.text}")
        print("  (typically = interrupt delivery: one side vectored, other did not)")

    print(f"\n=== REGISTER DIVERGENCE SITES (grouped by producing PC), "
          f"up to {args.max_classes} ===")
    for k, (idx, fset, ex) in list(sites.items())[:args.max_classes]:
        ra, rb, prod = ex
        pt = prod.text if prod else "?"
        print(f"  produce@{k} first idx {idx} fields {sorted(fset)}")
        print(f"      L {ra.text}")
        print(f"      R {rb.text}")


if __name__ == "__main__":
    main()
