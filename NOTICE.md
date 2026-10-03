# Third-party / licensing notices

cocovm — a Tandy Color Computer 3 emulator.
Copyright (C) 2026 Éric Spérano

Licensing is per crate:

- **`crates/mc6809`** (reusable MC6809 CPU core) — dual-licensed
  **MIT OR Apache-2.0** at your option (`crates/mc6809/LICENSE-MIT`,
  `crates/mc6809/LICENSE-APACHE`), so other projects can adopt it without
  copyleft obligations.
- **`crates/tms7000`** (reusable TMS7040 microcontroller core) — the
  crate's own code is dual-licensed **MIT OR Apache-2.0**
  (`crates/tms7000/LICENSE-MIT`, `crates/tms7000/LICENSE-APACHE`), but it
  ports MAME's BSD-3-Clause `tms7000` CPU core and disassembler, so the
  crate as a whole is **(MIT OR Apache-2.0) AND BSD-3-Clause**
  (`crates/tms7000/NOTICE`); see the attribution below.
- **`crates/test-assets`** (development-only test asset helper) —
  dual-licensed **MIT OR Apache-2.0** under the same terms in
  `crates/mc6809/LICENSE-MIT` and `crates/mc6809/LICENSE-APACHE`.
- **`crates/coco-core`, `crates/coco-egui`** (the emulator itself) —
  **GPL-3.0-or-later** (see `LICENSE`): you can redistribute and/or modify
  them under the GNU GPL as published by the Free Software Foundation,
  version 3 or (at your option) any later version.

This program is distributed WITHOUT ANY WARRANTY; see the licenses for
details. Note the GPL crates depend on the permissive `mc6809` crate (fine:
permissive code may be combined into a GPL work), never the reverse — keep
`mc6809` and `tms7000` free of GPL-licensed code.

## Bundled third-party material

Several tables, decoders, and CPU cores were copied or ported from **MAME**.
The source files are per-file licensed **BSD-3-Clause**, with the copyright
holder noted per entry below (verified against the file headers 2026-07-01 —
an earlier version of this notice recorded the font/palette entries as
GPL-2.0-or-later, which was wrong, and later misattributed the TMS7000
entries to Nathan Woods). BSD-3-Clause is GPL-compatible; the attribution
below satisfies its notice requirement.

- **MC6847 fonts (`crates/coco-core/src/font6847.rs`)** — `vdg_fontdata8x12`
  and `vdg_t1_fontdata8x12` from `src/devices/video/mc6847.cpp`, copyright
  **Nathan Woods** (MAME project).
- **MC6847 NTSC RG6 artifact decoder
  (`crates/coco-core/src/video/artifact.rs`)** — a port of
  `mc6847_base_device::artifacter` from `src/devices/video/mc6847.cpp` and
  `mc6847.h` (artifact color factors, correction lookup, six-pixel
  neighborhood, phase selection, and scaling), which are BSD-3-Clause,
  copyright **Nathan Woods** (MAME project).
- **GIME fonts (`crates/coco-core/src/font_gime.rs`)** —
  `gime_device::hires_font` and `gime_device::lowres_font` from
  `src/mame/trs/gime.cpp`, copyright **Nathan Woods** (MAME project).
- **Composite-monitor palette tables (`crates/coco-core/src/gime/palette.rs`,
  `COMPOSITE_PALETTE` / `COMPOSITE_PALETTE_180`)** —
  `gime_device::get_composite_color` from `src/mame/trs/gime.cpp`, copyright
  **Nathan Woods** (MAME project).
- **VDG fixed palette (`crates/coco-core/src/video.rs`,
  `VDG_FIXED_PALETTE`)** — `mc6847_base_device::s_palette` from
  `src/devices/video/mc6847.cpp`, copyright **Nathan Woods** (MAME project).
- **TMS7000/TMS7040 CPU core (`crates/tms7000/src/exec.rs`,
  `exec/modes.rs`, `exec/extended.rs`, `exec/ops.rs`, `decode.rs`,
  `peripheral.rs`, `timer.rs`, `memory.rs`, `lib.rs`)** — a port of
  `src/devices/cpu/tms7000/tms7000.cpp` and `tms7000op.cpp` (opcode map,
  cycle costs, BCD correction constants, peripheral-file and interrupt
  semantics), which are BSD-3-Clause, copyright **hap** and
  **Tim Lindner** (MAME project).
- **TMS7000 disassembler (`crates/tms7000/src/disasm.rs`,
  `disasm/tables.rs`)** — a port of
  `src/devices/cpu/tms7000/7000dasm.cpp` (disassembler spellings), which
  is BSD-3-Clause, copyright **Tim Lindner** (MAME project).

The full BSD-3-Clause text and copyright lines for the TMS7000 entries are
also reproduced in `crates/tms7000/NOTICE`, which ships with the crate.

> Copyright (c) Nathan Woods.
> Copyright (c) hap, Tim Lindner.
>
> Redistribution and use in source and binary forms, with or without
> modification, are permitted provided that the following conditions are met:
>
> 1. Redistributions of source code must retain the above copyright notice,
>    this list of conditions and the following disclaimer.
> 2. Redistributions in binary form must reproduce the above copyright notice,
>    this list of conditions and the following disclaimer in the documentation
>    and/or other materials provided with the distribution.
> 3. Neither the name of the copyright holder nor the names of its contributors
>    may be used to endorse or promote products derived from this software
>    without specific prior written permission.
>
> THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
> AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
> IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE
> ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE
> LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR
> CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF
> SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS
> INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN
> CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE)
> ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE
> POSSIBILITY OF SUCH DAMAGE.

## 3D model attribution

- "TRS-80 Color Computer 2" (https://skfb.ly/6V6IL) by **ericomont**,
  licensed under Creative Commons Attribution 4.0
  (http://creativecommons.org/licenses/by/4.0/). Not yet committed to the
  repo or the assets tarball; whenever it ships or is rendered in-app,
  this credit must also be shown to the user (e.g. the About window).

## Local assets (not distributed with the repository)

- `~/.local/share/cocovm/assets/roms/` — copyrighted Tandy/Microsoft ROM images
  (`coco3.rom`, `disk11.rom`, …), installed outside the repo by the app's
  first-run asset download.
- `docs/*.pdf` — copyrighted reference PDFs, excluded via `.gitignore`
  (as is any repo-root `roms/`, as a safety net).
