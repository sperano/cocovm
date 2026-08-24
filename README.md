# CocoVM

A Tandy Color Computer (CoCo 1/2/3) emulator in Rust + egui, aiming for
Virtual ][-level polish.

- `crates/mc6809` — MC6809 CPU core
- `crates/coco-core` — the headless machine (GIME, SAM, PIAs, disk, tape, sound…)
- `crates/coco-egui` — the frontend: a VirtualBox-style VM manager with
  per-machine suspend/resume
- `book/` — a 16-chapter course that builds the emulator from scratch

Run `cargo run` to open the VM manager. ROM images are not included in the
repository; the manager downloads them on first launch into its data
directory (`~/.local/share/cocovm/roms/` on Linux/macOS).
