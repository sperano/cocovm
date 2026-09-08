CocoVM: a Tandy Color Computer (CoCo 1/2/3) emulator in Rust + egui, aiming
for Virtual ][-level polish. Workspace: `crates/mc6809` (CPU),
`crates/coco-core` (headless machine), `crates/coco-egui` (frontend + VM
manager). `book/` is a 16-chapter course built from this codebase.

## Vikunja task workflow

- Add the `in progress` label when starting work on a task.
- Never mention Vikunja or link to its tasks in GitHub PR titles, descriptions,
  or comments.
- Always create a dedicated git worktree and branch before editing for a
  Vikunja task. Never implement a Vikunja task directly in the primary
  worktree or on `main`.
- Finish every Vikunja task by creating a pull request.
- Temporarily skip the book PR hooks when creating these pull requests.

## Local resources (copyrighted, present only on this machine)

- `./docs/` — authoritative reference PDFs (6809/6309 instruction sets, MC6809
  programming manual, CoCo 3 Service Manual, Super Extended BASIC Unravelled II,
  memory maps). Verify hardware claims against these with
  `pdftotext -layout <pdf>` instead of guessing or web search. The PDFs are
  git-ignored; don't commit them.
- `~/.local/share/cocovm/assets/roms/` — real ROM images: `coco3.rom` (32K Super
  Extended Color BASIC, maps to `$8000–$FFFF`), `disk11.rom` (8K Disk BASIC),
  and the CoCo 1/2 BASIC sets. Used to boot real code and trace-diff against
  XRoar/MAME. Installed by the app's first-run asset download
  (`manager/assets.rs` dialog over `startup::missing_assets`); tests resolve them
  via `crates/test-assets`. There is no repo-root `roms/` directory — don't
  create one.
- `~/.local/share/cocovm/assets/tests/` — the disk/VHD images the integration
  tests boot (EOU 1.0.1 `68EMU.dsk`/`68SDC.VHD`, NitrOS-9, DriveWire `.dsk`s).
  Not part of the app's bundle: `crates/test-assets` downloads the separate
  test bundle on first use (`COCOVM_TEST_ASSETS_URL` overrides the URL; empty
  disables the fetch and the tests skip).
- `~/code/mame/` — local MAME source checkout. Use it to verify device and
  machine behavior instead of fetching MAME source from the web.
