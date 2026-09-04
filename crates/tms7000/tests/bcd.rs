//! DAC/DSB: decimal add/subtract with carry, checked against plain decimal
//! arithmetic over every valid BCD pair.

mod common;

use common::Sys;

fn bcd(n: u8) -> u8 {
    (n / 10) << 4 | (n % 10)
}

#[test]
fn dac_adds_decimally_over_all_bcd_pairs() {
    for a in 0..100u8 {
        for b in 0..100u8 {
            for carry_in in [false, true] {
                let mut s = Sys::code(&[0x2E, bcd(b)]); // DAC %b,A
                s.set_a(bcd(a));
                s.set_c(carry_in);
                assert_eq!(s.insn(), 7 + 2);
                let sum = u16::from(a) + u16::from(b) + u16::from(carry_in);
                assert_eq!(s.a(), bcd((sum % 100) as u8), "{a} + {b} + {carry_in}");
                assert_eq!(s.flags().0, sum >= 100, "carry for {a} + {b} + {carry_in}");
            }
        }
    }
}

#[test]
fn dsb_subtracts_decimally_over_all_bcd_pairs() {
    for a in 0..100u8 {
        for b in 0..100u8 {
            for borrow_in in [false, true] {
                let mut s = Sys::code(&[0x2F, bcd(b)]); // DSB %b,A
                s.set_a(bcd(a));
                s.set_c(!borrow_in);
                assert_eq!(s.insn(), 7 + 2);
                let diff = i16::from(a) - i16::from(b) - i16::from(borrow_in);
                assert_eq!(
                    s.a(),
                    bcd(diff.rem_euclid(100) as u8),
                    "{a} - {b} - {borrow_in}"
                );
                assert_eq!(s.flags().0, diff >= 0, "no-borrow carry for {a} - {b}");
            }
        }
    }
}
