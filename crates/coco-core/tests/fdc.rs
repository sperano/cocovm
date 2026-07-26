//! FD-502 disk controller coverage: JVC image geometry, DSKREG decode and the
//! HALT*/NMI control-line recomputation, the WD1773 command state machine, and
//! an end-to-end boot-and-`DIR` integration test against the real
//! `roms/disk11.rom`.

#[path = "fdc/common.rs"]
mod common;

#[path = "fdc/boot.rs"]
mod boot;
#[path = "fdc/dskreg.rs"]
mod dskreg;
#[path = "fdc/image_geometry.rs"]
mod image_geometry;
#[path = "fdc/wd1773.rs"]
mod wd1773;
#[path = "fdc/write_track.rs"]
mod write_track;
