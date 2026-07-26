# Third-party / licensing notices

coco-rs — a Tandy Color Computer 3 emulator.
Copyright (C) 2026 Éric Spérano

Licensing is per crate:

- **`crates/mc6809`** (reusable MC6809 CPU core) — dual-licensed
  **MIT OR Apache-2.0** at your option (`crates/mc6809/LICENSE-MIT`,
  `crates/mc6809/LICENSE-APACHE`), so other projects can adopt it without
  copyleft obligations.
- **`crates/coco-core`, `crates/coco-egui`** (the emulator itself) —
  **GPL-3.0-or-later** (see `LICENSE`): you can redistribute and/or modify
  them under the GNU GPL as published by the Free Software Foundation,
  version 3 or (at your option) any later version.

This program is distributed WITHOUT ANY WARRANTY; see the licenses for
details. Note the GPL crates depend on the permissive `mc6809` crate (fine:
permissive code may be combined into a GPL work), never the reverse — keep
`mc6809` free of GPL-licensed code.

## Bundled third-party material

Both character-generator bitmap tables were copied from **MAME**. The source
files are per-file licensed **BSD-3-Clause**, copyright **Nathan Woods**
(verified against the file headers 2026-07-01 — an earlier version of this
notice recorded them as GPL-2.0-or-later, which was wrong). BSD-3-Clause is
GPL-compatible; the attribution below satisfies its notice requirement.

- **MC6847 font (`crates/coco-core/src/font6847.rs`)** — `vdg_t1_fontdata8x12`
  from `src/devices/video/mc6847.cpp`.
- **GIME hi-res font (`crates/coco-core/src/font_gime.rs`)** —
  `gime_device::hires_font` from `src/mame/trs/gime.cpp`.
- **Composite-monitor palette tables (`crates/coco-core/src/gime.rs`,
  `COMPOSITE_PALETTE` / `COMPOSITE_PALETTE_180`)** —
  `gime_device::get_composite_color` from `src/mame/trs/gime.cpp`.

> Copyright (c) Nathan Woods (MAME project).
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

## Local, git-ignored assets (not distributed)

- `roms/` — copyrighted Tandy/Microsoft ROM images (`coco3.rom`, `disk11.rom`).
- `docs/*.pdf` — copyrighted reference PDFs.

Both are excluded via `.gitignore`.
