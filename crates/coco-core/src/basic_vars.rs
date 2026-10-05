//! RAM variables Color BASIC and Super Extended Color BASIC keep the text
//! cursor in. These are ROM state, not hardware: they mean something only
//! while the BASIC ROM owns the screen.
//!
//! Every address here was read off the ROM code that uses it (disassembly of
//! `bas13.rom` mapped at `$A000` and `coco3.rom` mapped at `$8000`); the
//! citations name the routine's address in that map. Color BASIC runs with
//! DP = `$00` (`bas13.rom` `$A05E`, `coco3.rom` `$9C3E`), so its direct-page
//! variables sit at the logical addresses below.

/// CURPOS: big-endian absolute address of the 32-column VDG cursor cell.
/// The character-output routine (`bas13.rom` `$A30A`) loads it, writes the
/// character, and stores it back; CLS homes it to [`VDG_SCREEN_BASE`]
/// (`bas13.rom` `$A92A`). The CoCo 3's `WIDTH 32` reuses the same routine
/// (`coco3.rom` `$F652`).
pub const CURPOS: u16 = 0x0088;

/// Start of BASIC's 32×16 VDG text screen, an immediate in every routine
/// that touches it (`LDX #$0400` at `bas13.rom` `$A92A`).
pub const VDG_SCREEN_BASE: u16 = 0x0400;

/// Last cell of BASIC's VDG text screen (`CMPX #$05FF`, `bas13.rom` `$A92A`).
pub const VDG_SCREEN_LAST: u16 = 0x05FF;

/// HRWIDTH (CoCo 3 only): the current text screen, set by the `WIDTH`
/// statement (`coco3.rom` `$F636`). See [`hrwidth`] for the values.
pub const HRWIDTH: u16 = 0x00E7;

/// [`HRWIDTH`] values (`coco3.rom` `$F652`/`$F65C`/`$F679`).
pub mod hrwidth {
    /// `WIDTH 32`: the VDG-compatible screen at [`super::VDG_SCREEN_BASE`].
    pub const VDG_32: u8 = 0;
    /// `WIDTH 40`: the GIME hi-res text screen, 40 columns.
    pub const HIRES_40: u8 = 1;
    /// `WIDTH 80`: the GIME hi-res text screen, 80 columns.
    pub const HIRES_80: u8 = 2;
}

/// H.CURSX (CoCo 3 only): 0-based cursor column on the `WIDTH 40`/`80`
/// screen. The cursor-advance routine (`coco3.rom` `$F807`) bumps it and
/// wraps it at [`H_COLUMN`]; home clears it (`coco3.rom` `$F68C`). It lives
/// in the `$FE00` constant-RAM page, so read it at its logical address.
pub const H_CURSX: u16 = 0xFE02;

/// H.CURSY (CoCo 3 only): 0-based cursor row on the `WIDTH 40`/`80`
/// screen, advanced alongside [`H_CURSX`] (`coco3.rom` `$F807`).
pub const H_CURSY: u16 = 0xFE03;

/// H.COLUMN (CoCo 3 only): columns on the hi-res text screen, 40 or 80
/// (`STD $FE04` at `coco3.rom` `$F663`/`$F680`).
pub const H_COLUMN: u16 = 0xFE04;

/// H.ROW (CoCo 3 only): rows on the hi-res text screen, stored with
/// [`H_COLUMN`] by the same `STD`.
pub const H_ROW: u16 = 0xFE05;
