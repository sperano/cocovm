CocoVM: a Tandy Color Computer (CoCo 1/2/3) emulator in Rust + egui, aiming
for Virtual ][-level polish. Workspace: `crates/mc6809` (CPU),
`crates/coco-core` (headless machine), `crates/coco-egui` (frontend + VM
manager). `book/` is a 16-chapter course built from this codebase.

## Local resources (copyrighted, present only on this machine)

- `./docs/` — authoritative reference PDFs (6809/6309 instruction sets, MC6809
  programming manual, CoCo 3 Service Manual, Super Extended BASIC Unravelled II,
  memory maps). Verify hardware claims against these with
  `pdftotext -layout <pdf>` instead of guessing or web search. The PDFs are
  git-ignored; don't commit them. Pre-extracted text lives in `docs/txt/`
  (also git-ignored); regenerate with `scripts/extract-docs.sh`.
- `~/.local/share/cocovm/roms/` — real ROM images: `coco3.rom` (32K Super
  Extended Color BASIC, maps to `$8000–$FFFF`), `disk11.rom` (8K Disk BASIC),
  the CoCo 1/2 BASIC sets, `sp0256-al2.rom` (the Sound/Speech Cartridge's
  2K allophone ROM) and `ssc-tms7040.rom` (its 4K TMS7040 firmware); the
  SSC tests skip without them. Used to boot real code and trace-diff against
  XRoar/MAME. Installed by the app's first-run asset download
  (`ensure_assets`, `crates/coco-egui/src/startup.rs`); tests resolve them
  via `crates/test-assets`. There is no repo-root `roms/` directory — don't
  create one.
