CocoVM: a Tandy Color Computer (CoCo 1/2/3) emulator in Rust + egui, aiming
for Virtual ][-level polish. Workspace: `crates/mc6809` (CPU),
`crates/coco-core` (headless machine), `crates/coco-egui` (frontend + VM
manager). `book/` is a 16-chapter course built from this codebase.

## Local resources (git-ignored — copyrighted, present only on this machine)

- `./docs/` — authoritative reference PDFs (6809/6309 instruction sets, MC6809
  programming manual, CoCo 3 Service Manual, Super Extended BASIC Unravelled II,
  memory maps). Verify hardware claims against these with
  `pdftotext -layout <pdf>` instead of guessing or web search.
- `./roms/` — real ROM images: `coco3.rom` (32K Super Extended Color BASIC,
  maps to `$8000–$FFFF`) and `disk11.rom` (8K Disk BASIC). Used to boot real
  code and trace-diff against XRoar/MAME. `crates/coco-core`'s boot tests read
  `roms/coco3.rom`.

Both directories are in `.gitignore`; don't commit their contents.
