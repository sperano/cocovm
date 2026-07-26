//! Phase 6 (`docs/coco12-plan.md`): a CoCo 2 running real Extended Color
//! BASIC 1.1 + Color BASIC 1.2 ROMs, covering the sign-on boot, PMODE/speed/
//! all-RAM pokes, and a cassette CSAVE/CLOAD round trip.
//!
//! Skipped (not failed) if the ROMs aren't present locally, matching
//! `tests/boot.rs`/`tests/alive.rs`.

#[path = "coco2_boot/common.rs"]
mod common;

#[path = "coco2_boot/boot.rs"]
mod boot;
#[path = "coco2_boot/cassette.rs"]
mod cassette;
#[path = "coco2_boot/pokes.rs"]
mod pokes;
