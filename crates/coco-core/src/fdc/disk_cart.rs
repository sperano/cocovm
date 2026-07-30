//! DSKREG (`$FF40`) and [`DiskCart`], the [`Cartridge`] that wires a
//! [`WD1773`] and four drive slots to the CoCo's DSKREG latch and the
//! SCS/CTS windows.

use serde::{Deserialize, Serialize};

use crate::cart::{Cartridge, ROMPak, RomPakError, IO_OPEN_BUS};
use crate::wd1773::WD1773;

use super::jvc::JVCDisk;
use super::DRIVE_COUNT;

/// DSKREG latch bit assignments (`$FF40`; SCS writes at `$FF40-$FF47` all hit
/// it — MAME `coco_fdc.cpp` `dskreg_w`).
pub mod dskreg {
    /// Halt-enable: while set, the HALT* control line asserts whenever DRQ is
    /// low (see `DiskCart`'s `Cartridge::halt_asserted` implementation below).
    pub const HALT_ENABLE: u8 = 0x80;
    /// Drive-select 3 when no lower drive-select bit is set, else the side
    /// (head) select for drives 0-2.
    pub const DRIVE3_OR_SIDE: u8 = 0x40;
    /// Density select (1 = double). MAME wires this same bit to gate NMI on
    /// INTRQ; the `dden` pin the WD1773 actually sees is the inverse of this
    /// bit, which doesn't matter here since FM/MFM density isn't modelled —
    /// only the NMI-enable use of this bit is implemented.
    pub const DENSITY_AND_NMI_ENABLE: u8 = 0x20;
    /// Write precompensation select — stored, not acted on (spec).
    pub const WRITE_PRECOMP: u8 = 0x10;
    /// Motor on, all drives.
    pub const MOTOR_ON: u8 = 0x08;
    pub const DRIVE2: u8 = 0x04;
    pub const DRIVE1: u8 = 0x02;
    pub const DRIVE0: u8 = 0x01;
}

/// Resolve DSKREG's drive-select bits to a drive index (MAME `dskreg_w`):
/// bit2 wins if set, else bit1, else bit0, else bit6 (drive 3); `None` if none
/// of those four bits are set (no drive selected).
fn selected_drive(reg: u8) -> Option<usize> {
    if reg & dskreg::DRIVE2 != 0 {
        Some(2)
    } else if reg & dskreg::DRIVE1 != 0 {
        Some(1)
    } else if reg & dskreg::DRIVE0 != 0 {
        Some(0)
    } else if reg & dskreg::DRIVE3_OR_SIDE != 0 {
        Some(3)
    } else {
        None
    }
}

/// Resolve DSKREG's side (head) select: bit6 selects side 1, but only for
/// drives 0-2 — for drive 3, bit6 is the drive-select bit itself, not a side
/// select (spec).
fn selected_side(reg: u8, drive: Option<usize>) -> u8 {
    if reg & dskreg::DRIVE3_OR_SIDE != 0 && drive != Some(3) {
        1
    } else {
        0
    }
}

/// Free function (not a `DiskCart` method) so callers can borrow `drives`
/// mutably alongside a disjoint mutable borrow of `DiskCart::fdc` — going
/// through a `&mut self` method here would make the borrow checker see the
/// whole `DiskCart` as borrowed instead of just this one field.
fn selected_disk(
    drives: &mut [Option<JVCDisk>; DRIVE_COUNT],
    drive: Option<usize>,
) -> Option<&mut JVCDisk> {
    drive.and_then(|i| drives[i].as_mut())
}

// ---- WD1773 register offsets within the SCS window -------------------------

const STATUS_COMMAND_REG: u16 = 0xFF48;
const TRACK_REG: u16 = 0xFF49;
const SECTOR_REG: u16 = 0xFF4A;
const DATA_REG: u16 = 0xFF4B;
/// DSKREG mirrors across all of `$FF40-$FF47` (spec).
const DSKREG_BASE: u16 = 0xFF40;
const DSKREG_LAST: u16 = 0xFF47;

/// The FD-502: a WD1773 plus DSKREG plus four drive slots, serving the Disk
/// Extended Color BASIC ROM through the cartridge's CTS window.
#[derive(Serialize, Deserialize)]
pub struct DiskCart {
    rom: ROMPak,
    fdc: WD1773,
    dskreg: u8,
    drives: [Option<JVCDisk>; DRIVE_COUNT],
    /// Previous state of the NMI line (`intrq && DENSITY_AND_NMI_ENABLE`), so
    /// [`DiskCart::update_lines`] can detect its rising edge.
    nmi_line: bool,
    /// A rising edge of the NMI line since the last [`Cartridge::take_nmi`].
    nmi_pending: bool,
}

impl std::fmt::Debug for DiskCart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiskCart")
            .field("dskreg", &self.dskreg)
            .field("fdc", &self.fdc)
            .field("mounted", &self.drives.each_ref().map(Option::is_some))
            .finish()
    }
}

impl DiskCart {
    /// Build a disk controller cartridge serving `rom` (Disk Extended Color
    /// BASIC, `disk11.rom`) through the CTS window, mirror-filled the same way
    /// a plain [`ROMPak`] is (reused directly: same MAME `coco_pak_device`
    /// mirror-fill, same CTS-first/half-swap indexing). Never ties CART* to Q —
    /// like other Disk BASIC paks, BASIC finds it via the cold-start `DK` probe,
    /// not autostart.
    ///
    /// Reset state: DSKREG = 0, INTRQ clear, DRQ set (spec — a default-clear DRQ
    /// would spuriously assert HALT* the moment boot code sets DSKREG's
    /// halt-enable bit, before any command has run).
    pub fn new(rom: Box<[u8]>) -> Self {
        const AUTOSTART: bool = false;
        let rom = ROMPak::from_bytes(&rom, AUTOSTART)
            .unwrap_or_else(|e: RomPakError| panic!("invalid disk controller ROM image: {e}"));
        Self {
            rom,
            fdc: WD1773::new(),
            dskreg: 0,
            drives: [None, None, None, None],
            nmi_line: false,
            nmi_pending: false,
        }
    }

    /// Restore-path-only: re-inject the Disk Extended Color BASIC ROM image
    /// after a snapshot restore — delegates to the inner
    /// [`ROMPak::reattach_image`] (`docs/plan-save-states.md`). Unlike
    /// [`DiskCart::new`], returns a `Result` instead of panicking: a restore
    /// path must not crash the process on a bad ROM.
    pub fn reattach_rom(&mut self, rom: &[u8]) -> Result<(), RomPakError> {
        self.rom.reattach_image(rom)
    }

    pub fn insert_disk(&mut self, drive: usize, disk: JVCDisk) {
        self.drives[drive] = Some(disk);
    }

    pub fn eject_disk(&mut self, drive: usize) {
        self.drives[drive] = None;
    }

    pub fn is_mounted(&self, drive: usize) -> bool {
        self.drives[drive].is_some()
    }

    /// The floppy in `drive`, if any (status display, write-back on eject).
    pub fn disk(&self, drive: usize) -> Option<&JVCDisk> {
        self.drives[drive].as_ref()
    }

    /// Mutable twin of [`DiskCart::disk`]: the snapshot restore flow uses
    /// this to reach [`JVCDisk::reattach_data`] for whichever drives came
    /// back from a snapshot with a disk mounted (`docs/plan-save-states.md`).
    pub fn disk_mut(&mut self, drive: usize) -> Option<&mut JVCDisk> {
        self.drives[drive].as_mut()
    }

    /// Restore-only: after every mounted drive's data has been reattached
    /// (`crate::snapshot::restore_disks`), bound-check an in-flight Read/
    /// Write Sector transfer against the drive it currently targets (per
    /// `dskreg`'s drive-select bits) — see
    /// [`WD1773::validate_transfer_bounds`]. Must run AFTER reattachment:
    /// `JvcDisk::data` is `#[serde(skip)]`, empty until then, so any earlier
    /// check would reject every in-flight transfer, not just corrupted ones.
    pub(crate) fn validate_restored_transfer(&self) -> Result<(), String> {
        let disk = self.drive_index().and_then(|i| self.drives[i].as_ref());
        self.fdc.validate_transfer_bounds(disk)
    }

    /// Whether `drive` is selected with its motor on — what a real drive's
    /// front-panel light shows. Drives the status bar's activity LED.
    pub fn drive_active(&self, drive: usize) -> bool {
        self.motor_on() && self.drive_index() == Some(drive)
    }

    fn drive_index(&self) -> Option<usize> {
        selected_drive(self.dskreg)
    }

    fn side(&self) -> u8 {
        selected_side(self.dskreg, self.drive_index())
    }

    fn motor_on(&self) -> bool {
        self.dskreg & dskreg::MOTOR_ON != 0
    }


    /// Recompute the control lines the DSKREG/WD1773 pair drives, per MAME
    /// `coco_fdc.cpp update_lines`: called after every event that could change
    /// INTRQ, DRQ, or DSKREG (register access or a `tick`).
    ///
    /// 1. A high INTRQ clears DSKREG's halt-enable bit (hardware does this).
    /// 2. The NMI line is `intrq && DENSITY_AND_NMI_ENABLE`; an edge on *that*
    ///    line (not on INTRQ itself) is what queues an NMI.
    /// 3. HALT* is `!drq && HALT_ENABLE` — read live by [`Cartridge::halt_asserted`],
    ///    not cached here.
    fn update_lines(&mut self) {
        if self.fdc.intrq {
            self.dskreg &= !dskreg::HALT_ENABLE;
        }
        let nmi_line = self.fdc.intrq && self.dskreg & dskreg::DENSITY_AND_NMI_ENABLE != 0;
        if nmi_line && !self.nmi_line {
            self.nmi_pending = true;
        }
        self.nmi_line = nmi_line;
    }
}

impl Cartridge for DiskCart {
    fn read(&mut self, addr: u16) -> u8 {
        let val = match addr {
            DSKREG_BASE..=DSKREG_LAST => IO_OPEN_BUS, // spec: reads here are open bus
            STATUS_COMMAND_REG => {
                let disk_present = self.drive_index().is_some_and(|i| self.drives[i].is_some());
                let motor_on = self.motor_on();
                self.fdc.read_status(disk_present, motor_on)
            }
            TRACK_REG => self.fdc.track,
            SECTOR_REG => self.fdc.sector,
            DATA_REG => self.fdc.read_data(),
            _ => IO_OPEN_BUS,
        };
        self.update_lines();
        val
    }

    fn write(&mut self, addr: u16, val: u8) {
        match addr {
            DSKREG_BASE..=DSKREG_LAST => self.dskreg = val,
            STATUS_COMMAND_REG => {
                self.fdc.set_double_density(self.dskreg & dskreg::DENSITY_AND_NMI_ENABLE != 0);
                let side = self.side();
                let idx = self.drive_index();
                let disk = selected_disk(&mut self.drives, idx);
                self.fdc.write_command(val, disk, side);
            }
            TRACK_REG => self.fdc.track = val,
            SECTOR_REG => self.fdc.sector = val,
            DATA_REG => {
                let side = self.side();
                let idx = self.drive_index();
                let disk = selected_disk(&mut self.drives, idx);
                self.fdc.write_data(val, disk, side);
            }
            _ => {}
        }
        self.update_lines();
    }

    fn rom_read(&mut self, addr: u16) -> u8 {
        self.rom.rom_read(addr)
    }

    fn rom_peek(&self, addr: u16) -> u8 {
        self.rom.rom_peek(addr)
    }

    fn cart_line_ties_q(&self) -> bool {
        false
    }

    fn tick(&mut self, cycles: u32) {
        let side = self.side();
        let idx = self.drive_index();
        let disk = selected_disk(&mut self.drives, idx);
        self.fdc.tick(cycles, disk, side);
        self.update_lines();
    }

    fn halt_asserted(&self) -> bool {
        !self.fdc.drq && self.dskreg & dskreg::HALT_ENABLE != 0
    }

    fn take_nmi(&mut self) -> bool {
        std::mem::replace(&mut self.nmi_pending, false)
    }

    fn nmi_pending(&self) -> bool {
        self.nmi_pending
    }

    /// Structural half of the WD1773 transfer check — see
    /// [`WD1773::validate_restored`]. The disk-bound half
    /// ([`DiskCart::validate_restored_transfer`]) needs floppy reattachment
    /// first, so it isn't reachable from this trait method (called before
    /// any media is resolved) and runs separately, later in the restore
    /// flow.
    fn validate_restored(&self) -> Result<(), String> {
        self.fdc.validate_restored().map_err(|e| format!("WD1773: {e}"))
    }
}
