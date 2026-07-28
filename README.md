# CocoVM

A Tandy Color Computer (CoCo 1/2/3) emulator in Rust + egui, aiming for
Virtual ][-level polish.

- `crates/mc6809` — MC6809 CPU core
- `crates/coco-core` — the headless machine (GIME, SAM, PIAs, disk, tape, sound…)
- `crates/coco-egui` — the frontend: direct-boot emulator window and a
  VirtualBox-style VM manager with per-machine suspend/resume
- `book/` — a 16-chapter course that builds the emulator from scratch

Run `cargo run` for the VM manager, or pass CLI flags to boot a machine
directly. ROM images are not included; place `coco3.rom` in `./roms/`.
