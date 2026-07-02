# Third-party / licensing notices

## Outstanding licensing debt

- **MC6847 font (`crates/coco-core/src/font6847.rs`)** — the character-generator
  bitmap table (`MC6847_FONT`) is copied from **MAME**'s `src/devices/video/mc6847.cpp`
  (`vdg_t1_fontdata8x12`), which is licensed **GPL-2.0-or-later**. It was pulled in
  to get an authentic CoCo text glyph set for the first "it's alive" milestone.

- **GIME hi-res font (`crates/coco-core/src/font_gime.rs`)** — the GIME internal
  character-generator table (`GIME_FONT`) is copied from **MAME**'s
  `src/mame/trs/gime.cpp` (`gime_device::hires_font`), also **GPL-2.0-or-later**.
  Same situation as the MC6847 font: it renders the native 32/40/64/80-column
  text modes (WIDTH 40/80).

  **This must be resolved before any distribution.** Options:
  1. Replace it with an original, clean-room font (no licensing constraint).
  2. Adopt a GPL-compatible license for the project and attribute MAME properly.
  3. Source the glyphs from a permissively-licensed or public-domain font ROM.

## Local, git-ignored assets (not distributed)

- `roms/` — copyrighted Tandy/Microsoft ROM images (`coco3.rom`, `disk11.rom`).
- `docs/` — copyrighted reference PDFs.

Both are excluded via `.gitignore`.
