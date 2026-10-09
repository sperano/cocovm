CoCoVM: a Tandy Color Computer (CoCo 1/2/3) emulator in Rust + egui, aiming
for Virtual ][-level polish. Workspace: `crates/mc6809` (CPU),
`crates/coco-core` (headless machine), `crates/coco-egui` (frontend + VM
manager).

## Local resources (copyrighted, present only on this machine)

- `./docs/` — authoritative reference PDFs (6809/6309 instruction sets, MC6809
  programming manual, CoCo 3 Service Manual, Super Extended BASIC Unravelled II,
  memory maps). Verify hardware claims against these with
  `pdftotext -layout <pdf>` instead of guessing or web search. The whole
  directory is git-ignored; don't commit anything under it. Pre-extracted
  text lives in `docs/txt/`; regenerate with `scripts/extract-docs.sh`.
  Verification findings and other research notes derived from this material
  (bit-banger, cartridge, DMP printer, and SSC specs) live on the wiki under
  `cocovm/`, not as files here.
- `~/.local/share/cocovm/assets/roms/` — real ROM images: `coco3.rom` (32K Super
  Extended Color BASIC, maps to `$8000–$FFFF`), `disk11.rom` (8K Disk BASIC),
  `hdbdw3bc3.rom` (HDB-DOS 1.4 Becker), the CoCo 1/2 BASIC sets, `sp0256-al2.rom` (the Sound/Speech Cartridge's
  2K allophone ROM) and `ssc-tms7040.rom` (its 4K TMS7040 firmware); the
  SSC tests skip without them. Used to boot real code and trace-diff against
  XRoar/MAME. Installed by the app's first-run asset download
  (`manager/assets.rs` dialog over `startup::missing_assets`); tests resolve them
  via `crates/test-assets`. There is no repo-root `roms/` directory — don't
  create one.
- `~/.local/share/cocovm/assets/tests/` — the disk/VHD images the integration
  tests boot (EOU 1.0.1 `68EMU.dsk`/`68SDC.VHD`, NitrOS-9, DriveWire `.dsk`s).
  Not part of the app's bundle: `crates/test-assets` downloads the separate
  test bundle on first use (`COCOVM_TEST_ASSETS_URL` overrides the URL; empty
  disables the fetch and the tests skip).
