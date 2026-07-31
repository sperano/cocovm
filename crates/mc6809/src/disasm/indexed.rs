//! Indexed-addressing postbyte rendering — mirrors [`crate::MC6809::ea_indexed`]
//! byte-for-byte (see that function's doc comment for the postbyte layout).

use super::Reader;
use crate::postbyte;

/// Decode one indexed-addressing postbyte into its operand string, mirroring
/// `MC6809::ea_indexed` byte-for-byte (see that function's doc comment for
/// the postbyte layout: `1 rr i mmmm` when bit 7 is set, `0 rr nnnnn` — a
/// non-indirectable 5-bit constant offset — when clear).
///
/// Numeric constant offsets (5-bit, 8-bit, 16-bit, and both PCR forms) are
/// rendered in signed decimal (e.g. `5,Y`, `-1,X`, `300,PCR`) rather than
/// hex — see the crate-level task report's "indexed offset radix" judgment
/// call, which follows the task spec's own `LDX 5,Y` example literally.
/// Absolute addresses (extended indirect `[$XXXX]`) stay hex.
pub(super) fn decode_indexed<F: FnMut(u16) -> u8>(r: &mut Reader<F>) -> String {
    let pb = r.u8();

    // 5-bit signed constant offset — not indirectable (bit 7 clear).
    if pb & 0x80 == 0 {
        return decode_indexed_offset5(pb);
    }

    let sel = pb >> postbyte::REG_SHIFT;
    let indirect = pb & postbyte::INDIRECT != 0;
    let body = decode_indexed_body(r, sel, pb & postbyte::MODE_MASK);

    if indirect { format!("[{body}]") } else { body }
}

/// The `0 rr nnnnn` postbyte form: a 5-bit signed constant offset, never
/// indirectable.
fn decode_indexed_offset5(pb: u8) -> String {
    let reg = index_reg_name(pb >> postbyte::REG_SHIFT);
    let n = pb & postbyte::OFFSET5_MASK;
    let offset: i16 = if n & postbyte::OFFSET5_SIGN != 0 {
        n as i16 - (postbyte::OFFSET5_SIGN as i16 * 2)
    } else {
        n as i16
    };
    format!("{offset},{reg}")
}

/// The sub-mode (`mmmm`) field of a `1 rr i mmmm` postbyte, rendered without
/// the indirect `[...]` wrap (applied by the caller).
fn decode_indexed_body<F: FnMut(u16) -> u8>(r: &mut Reader<F>, sel: u8, mode: u8) -> String {
    let reg = index_reg_name(sel);
    match mode {
        0b0000 => format!(",{reg}+"),
        0b0001 => format!(",{reg}++"),
        0b0010 => format!(",-{reg}"),
        0b0011 => format!(",--{reg}"),
        0b0100 => format!(",{reg}"),
        0b0101 => format!("B,{reg}"),
        0b0110 => format!("A,{reg}"),
        0b1000 => {
            let offset = r.u8() as i8;
            format!("{offset},{reg}")
        }
        0b1001 => {
            let offset = r.u16() as i16;
            format!("{offset},{reg}")
        }
        0b1011 => format!("D,{reg}"),
        0b1100 => {
            let offset = r.u8() as i8;
            format!("{offset},PCR")
        }
        0b1101 => {
            let offset = r.u16() as i16;
            format!("{offset},PCR")
        }
        0b1111 => {
            // Extended indirect: register field ignored. The `[...]` wrap is
            // applied uniformly below via the postbyte's indirect bit, same
            // as every other sub-mode (the core does not special-case this
            // one — see `ea_indexed`'s doc comment).
            let addr = r.u16();
            format!("${addr:04X}")
        }
        // Reserved/illegal postbytes (0b0111, 0b1010, 0b1110): the core
        // falls back to a plain register read with 0 extra bytes. Marked
        // with a `???` suffix so this doesn't read as a valid addressing
        // form.
        _ => format!(",{reg}???"),
    }
}

/// Index register name for the indexed-postbyte `rr` field (raw, unmasked —
/// callers pass `pb >> REG_SHIFT` directly, matching `MC6809::index_reg`'s
/// own calling convention, which masks internally).
fn index_reg_name(sel: u8) -> &'static str {
    match sel & 0b11 {
        0b00 => "X",
        0b01 => "Y",
        0b10 => "U",
        _ => "S",
    }
}
