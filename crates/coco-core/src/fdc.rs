//! FD-502 floppy disk controller cartridge: [`JVCDisk`] (JVC/.dsk image parsing)
//! and [`DiskCart`], the [`Cartridge`](crate::cart::Cartridge) that wires a
//! [`crate::wd1773::WD1773`] and four drive slots to the CoCo's DSKREG latch and
//! the SCS/CTS windows.
//!
//! Hardware facts (DSKREG bit layout, drive/side resolution, the HALT*/NMI
//! control-line recomputation, JVC geometry) are cited from MAME source
//! (`coco_fdc.cpp`, `wd_fdc.cpp`/`.h`, `jvc_dsk.cpp`) in the following doc comments,
//! per the verified spec this module was built from.

mod disk_cart;
mod jvc;

pub use disk_cart::{DiskCart, dskreg};
pub use jvc::{
    DEFAULT_FIRST_SECTOR_ID, DEFAULT_SECTOR_SIZE_CODE, DEFAULT_SECTORS_PER_TRACK, DEFAULT_SIDES,
    JVCDisk, JVCError, MAX_FORMAT_TRACKS,
};

/// Number of physical drive slots the FD-502 exposes (DSKREG selects among
/// these four).
pub const DRIVE_COUNT: usize = 4;
