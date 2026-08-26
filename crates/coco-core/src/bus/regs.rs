//! Address-decode constants for [`super::SystemBus`]: the I/O page device
//! ranges (`DESIGN.md` §3), the ROM/RAM window boundaries, and the plain-SAM
//! path's fixed ROM offsets.

// I/O page device ranges (`DESIGN.md` §3). PIA0/PIA1 mirror every 4 bytes.
pub(super) const IO_BASE: u16 = 0xFF00;
pub(super) const PIA0_LAST: u16 = 0xFF1F;
pub(super) const PIA1_BASE: u16 = 0xFF20;
pub(super) const PIA1_LAST: u16 = 0xFF3F;
/// Register-select mask within each 4-byte mirrored PIA block
/// (`MC6821::write`/`read`'s `reg & 0x03` decode in `pia.rs`).
pub(super) const PIA1_REG_MASK: u16 = 0x03;
/// Offset of PIA1 Port A's data/DDR register within each 4-byte mirrored
/// PIA block (0 = Port A, 1 = CRA, 2 = Port B, 3 = CRB — see
/// [`PIA1_REG_MASK`]). Cassette record-out only samples the DAC
/// on writes here, not on CRA ($FF21) writes — MAME's `update_cassout()` is
/// called only from `pia1_pa_changed()`, never from `pia1_ca2_w()` (the
/// motor-relay callback); see `SystemBus::write_pia1` in `io.rs`.
pub(super) const PIA1_PORT_A_OFFSET: u16 = 0x00;
/// Standard SCS* window: gated as one unit by INIT0 MC2 on the GIME path
/// (`GIME::scs_enabled`, `SystemBus::io_read`/`io_write`) — MAME
/// `coco3_m.cpp` `ff40_read`/`ff40_write`. Not gated on the plain-SAM path
/// (`sam_path.rs`): a real CoCo 1/2 has no GIME to hold MC2.
pub(super) const SCS_BASE: u16 = 0xFF40;
pub(super) const SCS_LAST: u16 = 0xFF5F;
/// $FF60-$FF7E is unmapped on the motherboard, so some carts (the RS-232
/// Pak, Orchestra-90, the Sound/Speech Cartridge) decode registers of their
/// own there too — the full address bus reaches the expansion connector
/// regardless (`docs/cartridges.md` "Carts can decode addresses outside
/// SCS"). Outside the SCS* decode, so INIT0 MC2 never gates it. Routes to
/// `cart.read`/`write` same as [`SCS_BASE`]..=[`SCS_LAST`]; carts that don't
/// claim an address here fall through to their own open-bus default.
pub(super) const CART_EXT_BASE: u16 = 0xFF60;
pub(super) const CART_EXT_LAST: u16 = 0xFF7E;
/// What the GIME's SCS* logic drives when INIT0 MC2 gates the window
/// closed: a hard 0, not open bus (MAME `coco3_m.cpp` `ff40_read`).
pub(super) const SCS_GATE_CLOSED: u8 = 0x00;
/// Becker-port status register: read-only, `DwServer::status_read()`.
/// Writes are swallowed while the Becker port is enabled. Intercepts ahead
/// of cartridge dispatch, behind INIT0 MC2's SCS gate on the GIME path —
/// see `SystemBus::becker_read`/`becker_write`.
pub(super) const BECKER_STATUS: u16 = 0xFF41;
/// Becker-port data register: read pops a `DwServer` reply byte, write
/// feeds a client byte into the DriveWire protocol state machine.
pub(super) const BECKER_DATA: u16 = 0xFF42;
/// Multi-Pak Interface select register: decoded by the MPI itself (when one
/// is inserted), never by the plugged-in cartridges' own `read`/`write` — see
/// [`Cartridge::control_read`].
///
/// [`Cartridge::control_read`]: crate::cart::Cartridge::control_read
pub(super) const MPI_CONTROL_REG: u16 = 0xFF7F;
// VHD (virtual hard disk, NitrOS-9 `emudsk`) register window — see `vhd.rs`.
// $FF87-$FF8F stays open-bus/unmapped.
pub(super) const VHD_LRN_HI: u16 = 0xFF80;
pub(super) const VHD_LRN_MID: u16 = 0xFF81;
pub(super) const VHD_LRN_LO: u16 = 0xFF82;
pub(super) const VHD_COMMAND_STATUS: u16 = 0xFF83;
pub(super) const VHD_BUFFER_HI: u16 = 0xFF84;
pub(super) const VHD_BUFFER_LO: u16 = 0xFF85;
pub(super) const VHD_SELECT: u16 = 0xFF86;
pub(super) const INIT0_REG: u16 = 0xFF90;
pub(super) const INIT1_REG: u16 = 0xFF91;
/// IRQ enable/status register (write = enables, read = latched status).
pub(super) const IRQENR_REG: u16 = 0xFF92;
/// FIRQ enable/status register (write = enables, read = latched status).
pub(super) const FIRQENR_REG: u16 = 0xFF93;
pub(super) const TIMER_MSB_REG: u16 = 0xFF94;
pub(super) const TIMER_LSB_REG: u16 = 0xFF95;
/// $FF96/$FF97 are reserved on the GIME.
pub(super) const GIME_RESERVED_BASE: u16 = 0xFF96;
pub(super) const GIME_RESERVED_LAST: u16 = 0xFF97;
pub(super) const VMODE_REG: u16 = 0xFF98;
pub(super) const VRES_REG: u16 = 0xFF99;
pub(super) const BORDER_REG: u16 = 0xFF9A;
pub(super) const VBANK_REG: u16 = 0xFF9B;
pub(super) const VSCROLL_REG: u16 = 0xFF9C;
pub(super) const VOFFSET1_REG: u16 = 0xFF9D;
pub(super) const VOFFSET0_REG: u16 = 0xFF9E;
pub(super) const HOFFSET_REG: u16 = 0xFF9F;
pub(super) const GIME_LAST: u16 = 0xFF9F;
pub(super) const MMU_BASE: u16 = 0xFFA0;
pub(super) const MMU_LAST: u16 = 0xFFAF;
pub(super) const PALETTE_BASE: u16 = 0xFFB0;
pub(super) const PALETTE_LAST: u16 = 0xFFBF;

/// Base of the ROM window. `$8000–$FFFF` reads return ROM when it is mapped, with
/// the fixed I/O page overlaid on top of `$FF00–$FFEF` (`DESIGN.md` §3).
pub(super) const ROM_WINDOW_BASE: u16 = 0x8000;
/// `$FE00–$FEFF` — the interrupt-vector page. INIT0 MC3 selects its mapping
/// (SEB Unravelled II; MAME `gime.cpp update_memory` bank 8): MC3=1 pins it to
/// constant RAM at physical `$7FE00` regardless of the MMU or ROM mode (BASIC
/// boots with MC3 set and writes its JMP trampolines here); MC3=0 makes it
/// follow the normal map like the rest of the `$8000+` window — MMU RAM in
/// all-RAM mode, ROM in ROM mode (internal or cartridge per MC1:MC0, as the
/// tail of the `$E000` bank). Sokoban relies on the MC3=0 ROM path: it is the
/// only way to address the last `$200` bytes of a pak image (CTS stops at
/// `$FDFF`), where it keeps its palette tables.
pub(super) const CONSTANT_RAM_BASE: u16 = 0xFE00;
pub(super) const CONSTANT_RAM_LAST: u16 = 0xFEFF;
/// Physical base of the constant `$FE00` page when INIT0 MC3 is set.
pub(super) const CONSTANT_RAM_PHYS: usize = 0x7_FE00;
/// `$FFE0–$FFFF` — the top 32 bytes of the `$8000–$FFFF` window, including the
/// 6809 hardware vectors — is hardwired to internal ROM on every read,
/// regardless of INIT0 MC1:MC0, the SAM TY map-type bit (`$FFDE`/`$FFDF`,
/// all-RAM mode), MMU state, or any inserted cartridge. MAME `coco3.cpp:53-58`
/// documents this as verified by William Astle's real-hardware test, which
/// refutes SEB Unravelled II p.28's claim that this range aliases `$BFFx`.
/// Writes here are dropped (`SystemBus::write`) — it isn't backed by RAM.
pub(super) const HARDWIRED_ROM_BASE: u16 = 0xFFE0;

pub(super) const OPEN_BUS: u8 = 0xFF;

/// Plain-SAM path only: flat 32K ROM image offset where Color BASIC starts —
/// extbas at 0, bas at $2000.
pub(super) const SAM_BAS_ROM_OFFSET: usize = 0x2000;
/// Plain-SAM path only: base CPU address of the cartridge CTS* ROM window,
/// added back to a `SAMTarget::Cart` offset before calling
/// [`Cartridge::rom_read`].
///
/// [`Cartridge::rom_read`]: crate::cart::Cartridge::rom_read
pub(super) const SAM_CART_ROM_BASE: u16 = 0xC000;
