# Third-party / licensing notices

coco-rs — a Tandy Color Computer 3 emulator.
Copyright (C) 2026 Éric Spérano

This program is free software: you can redistribute it and/or modify it under
the terms of the GNU General Public License as published by the Free Software
Foundation, either version 3 of the License, or (at your option) any later
version. See `LICENSE` for the full text. This program is distributed WITHOUT
ANY WARRANTY; see the license for details.

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

## Local, git-ignored assets (not distributed)

- `roms/` — copyrighted Tandy/Microsoft ROM images (`coco3.rom`, `disk11.rom`).
- `docs/*.pdf` — copyrighted reference PDFs.

Both are excluded via `.gitignore`.
