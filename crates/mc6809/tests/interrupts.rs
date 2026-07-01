//! Test-driven coverage for the misc inherent ops (ORCC/ANDCC/SEX/ABX/MUL/DAA)
//! and the interrupt / halt subsystem (SWI/SWI2/SWI3, RTI, CWAI, SYNC, and the
//! external NMI/IRQ/FIRQ delivery API).
//!
//! Frame conventions verified against the reference: full frame pushes
//! PC,U,Y,X,DP,B,A,CC with E=1; FIRQ pushes only CC,PC with E=0. IRQ sets I;
//! FIRQ and NMI set I+F; SWI sets I+F; SWI2/SWI3 leave the masks alone.

mod common;

use common::Sys;
use mc6809::{cc, State};

// ======================================================================
// Misc inherent
// ======================================================================

#[test]
fn orcc_and_andcc() {
    let mut s = Sys::code(0x0000, &[0x1A, 0x50]); // ORCC #$50
    s.cpu.cc = 0x01;
    assert_eq!(s.step(), 3);
    assert_eq!(s.cpu.cc, 0x51);

    let mut s = Sys::code(0x0000, &[0x1C, 0x0F]); // ANDCC #$0F
    s.cpu.cc = 0xFF;
    assert_eq!(s.step(), 3);
    assert_eq!(s.cpu.cc, 0x0F);
}

#[test]
fn sex_extends_negative_and_positive() {
    let mut s = Sys::code(0x0000, &[0x1D]); // SEX
    s.cpu.b = 0x80;
    assert_eq!(s.step(), 2);
    assert_eq!(s.cpu.a, 0xFF);
    assert_eq!(s.cpu.d(), 0xFF80);
    assert_ne!(s.cpu.cc & cc::NEGATIVE, 0);

    let mut s = Sys::code(0x0000, &[0x1D]);
    s.cpu.b = 0x7F;
    s.step();
    assert_eq!(s.cpu.a, 0x00);
    assert_eq!(s.cpu.cc & cc::NEGATIVE, 0);

    let mut s = Sys::code(0x0000, &[0x1D]);
    s.cpu.b = 0x00;
    s.step();
    assert_ne!(s.cpu.cc & cc::ZERO, 0);
}

#[test]
fn abx_adds_b_unsigned() {
    let mut s = Sys::code(0x0000, &[0x3A]); // ABX
    s.cpu.x = 0x1000;
    s.cpu.b = 0xFF;
    let cycles = s.step();
    assert_eq!(s.cpu.x, 0x10FF); // B is unsigned, not sign-extended
    assert_eq!(cycles, 3);
}

#[test]
fn mul_sets_z_and_carry() {
    let mut s = Sys::code(0x0000, &[0x3D]); // MUL
    s.cpu.a = 0x0C;
    s.cpu.b = 0x0F;
    let cycles = s.step();
    assert_eq!(s.cpu.d(), 0x00B4); // 12 * 15 = 180
    assert_eq!(cycles, 11);
    assert_eq!(s.cpu.cc & cc::ZERO, 0);
    assert_ne!(s.cpu.cc & cc::CARRY, 0); // bit 7 of result set

    let mut s = Sys::code(0x0000, &[0x3D]);
    s.cpu.a = 0x00;
    s.cpu.b = 0x05;
    s.step();
    assert_eq!(s.cpu.d(), 0);
    assert_ne!(s.cpu.cc & cc::ZERO, 0);
    assert_eq!(s.cpu.cc & cc::CARRY, 0);
}

#[test]
fn daa_adjusts_bcd_sum() {
    // $64 + $27 = binary $8B; DAA -> BCD $91.
    let mut s = Sys::code(0x0000, &[0x19]);
    s.cpu.a = 0x8B;
    let cycles = s.step();
    assert_eq!(s.cpu.a, 0x91);
    assert_eq!(cycles, 2);
    assert_eq!(s.cpu.cc & cc::CARRY, 0);
}

#[test]
fn daa_produces_carry() {
    let mut s = Sys::code(0x0000, &[0x19]);
    s.cpu.a = 0x9A;
    s.step();
    assert_eq!(s.cpu.a, 0x00); // 0x9A + 0x66 = 0x100
    assert_ne!(s.cpu.cc & cc::CARRY, 0);
    assert_ne!(s.cpu.cc & cc::ZERO, 0);
}

// ======================================================================
// SWI / RTI round trip
// ======================================================================

#[test]
fn swi_stacks_full_frame_and_vectors() {
    let mut s = Sys::code(0x1000, &[0x3F]); // SWI
    s.bus.load(0xFFFA, &[0x90, 0x00]); // SWI vector -> $9000
    s.cpu.s = 0x2000;
    s.cpu.cc = 0x00;
    let cycles = s.step();
    assert_eq!(s.cpu.pc, 0x9000);
    assert_eq!(s.cpu.s, 0x2000 - 12); // full 12-byte frame
    assert_eq!(cycles, 19);
    assert_ne!(s.cpu.cc & cc::IRQ_MASK, 0); // SWI sets I
    assert_ne!(s.cpu.cc & cc::FIRQ_MASK, 0); // and F
    assert_ne!(s.cpu.cc & cc::ENTIRE, 0); // E set
    assert_eq!(s.mem(0x1FF4), 0x80); // stacked CC = original | E (masks set after push)
}

#[test]
fn swi_then_rti_restores_all_registers() {
    let mut s = Sys::code(0x1000, &[0x3F]); // SWI at $1000
    s.bus.load(0xFFFA, &[0x90, 0x00]);
    s.bus.load(0x9000, &[0x3B]); // RTI handler
    s.cpu.s = 0x2000;
    s.cpu.a = 0x11;
    s.cpu.b = 0x22;
    s.cpu.x = 0x3333;
    s.cpu.y = 0x4444;
    s.cpu.u = 0x5555;
    s.cpu.dp = 0x66;
    s.cpu.cc = 0x00;

    s.step(); // SWI -> handler
    assert_eq!(s.cpu.pc, 0x9000);

    // Clobber everything the handler might use.
    s.cpu.a = 0;
    s.cpu.b = 0;
    s.cpu.x = 0;
    s.cpu.y = 0;
    s.cpu.u = 0;
    s.cpu.dp = 0;

    let rti_cycles = s.step(); // RTI
    assert_eq!(s.cpu.a, 0x11);
    assert_eq!(s.cpu.b, 0x22);
    assert_eq!(s.cpu.x, 0x3333);
    assert_eq!(s.cpu.y, 0x4444);
    assert_eq!(s.cpu.u, 0x5555);
    assert_eq!(s.cpu.dp, 0x66);
    assert_eq!(s.cpu.pc, 0x1001); // return address (after the SWI opcode)
    assert_eq!(s.cpu.s, 0x2000); // stack unwound
    assert_eq!(rti_cycles, 15); // full-frame RTI
    assert_eq!(s.cpu.cc & cc::ENTIRE, cc::ENTIRE); // restored CC had E set
}

#[test]
fn swi2_and_swi3_do_not_touch_masks() {
    let mut s = Sys::code(0x1000, &[0x10, 0x3F]); // SWI2
    s.bus.load(0xFFF4, &[0xA0, 0x00]);
    s.cpu.s = 0x2000;
    s.cpu.cc = 0x00;
    let cycles = s.step();
    assert_eq!(s.cpu.pc, 0xA000);
    assert_eq!(cycles, 20);
    assert_eq!(s.cpu.cc & cc::IRQ_MASK, 0); // masks unchanged
    assert_eq!(s.cpu.cc & cc::FIRQ_MASK, 0);
    assert_ne!(s.cpu.cc & cc::ENTIRE, 0); // but E still set (full frame)

    let mut s = Sys::code(0x1000, &[0x11, 0x3F]); // SWI3
    s.bus.load(0xFFF2, &[0xB0, 0x00]);
    s.cpu.s = 0x2000;
    s.cpu.cc = 0x00;
    assert_eq!(s.step(), 20);
    assert_eq!(s.cpu.pc, 0xB000);
    assert_eq!(s.cpu.cc & cc::IRQ_MASK, 0);
}

// ======================================================================
// External interrupt delivery
// ======================================================================

#[test]
fn irq_ignored_when_masked() {
    let mut s = Sys::new();
    s.cpu.pc = 0x1234;
    s.cpu.s = 0x2000;
    s.cpu.cc = cc::IRQ_MASK; // masked
    let serviced = s.cpu.irq(&mut s.bus);
    assert!(!serviced);
    assert_eq!(s.cpu.pc, 0x1234); // unchanged
    assert_eq!(s.cpu.s, 0x2000);
}

#[test]
fn irq_serviced_when_unmasked() {
    let mut s = Sys::new();
    s.bus.load(0xFFF8, &[0x80, 0x00]); // IRQ vector -> $8000
    s.cpu.pc = 0x1234;
    s.cpu.s = 0x2000;
    s.cpu.cc = 0x00;
    let serviced = s.cpu.irq(&mut s.bus);
    assert!(serviced);
    assert_eq!(s.cpu.pc, 0x8000);
    assert_eq!(s.cpu.s, 0x2000 - 12); // full frame
    assert_ne!(s.cpu.cc & cc::IRQ_MASK, 0); // I set
    assert_eq!(s.cpu.cc & cc::FIRQ_MASK, 0); // F left alone by IRQ
    assert_ne!(s.cpu.cc & cc::ENTIRE, 0);
}

#[test]
fn firq_uses_partial_frame() {
    let mut s = Sys::new();
    s.bus.load(0xFFF6, &[0x70, 0x00]); // FIRQ vector -> $7000
    s.cpu.pc = 0x1234;
    s.cpu.s = 0x2000;
    s.cpu.cc = 0x00;
    let serviced = s.cpu.firq(&mut s.bus);
    assert!(serviced);
    assert_eq!(s.cpu.pc, 0x7000);
    assert_eq!(s.cpu.s, 0x2000 - 3); // CC + PC only
    assert_ne!(s.cpu.cc & cc::IRQ_MASK, 0); // FIRQ sets both masks
    assert_ne!(s.cpu.cc & cc::FIRQ_MASK, 0);
    assert_eq!(s.mem(0x1FFD), 0x00); // stacked CC has E clear (partial frame)
}

#[test]
fn nmi_is_non_maskable() {
    let mut s = Sys::new();
    s.bus.load(0xFFFC, &[0x60, 0x00]); // NMI vector -> $6000
    s.cpu.pc = 0x1234;
    s.cpu.s = 0x2000;
    s.cpu.cc = cc::IRQ_MASK | cc::FIRQ_MASK; // fully masked
    s.cpu.nmi(&mut s.bus);
    assert_eq!(s.cpu.pc, 0x6000); // serviced anyway
    assert_eq!(s.cpu.s, 0x2000 - 12);
}

// ======================================================================
// SYNC / CWAI halt states
// ======================================================================

#[test]
fn sync_halts_and_idles_until_interrupt() {
    let mut s = Sys::code(0x1000, &[0x13, 0x12]); // SYNC ; NOP
    s.step(); // SYNC
    assert_eq!(s.cpu.state, State::Syncing);
    let pc_after_sync = s.cpu.pc;

    // While syncing, step() just idles.
    let idle = s.step();
    assert_eq!(idle, 1);
    assert_eq!(s.cpu.pc, pc_after_sync); // no fetch

    // A masked IRQ still wakes SYNC (without servicing).
    s.cpu.cc = cc::IRQ_MASK;
    let serviced = s.cpu.irq(&mut s.bus);
    assert!(!serviced);
    assert_eq!(s.cpu.state, State::Running);
    // Next step runs the instruction after SYNC.
    s.step();
    assert_eq!(s.cpu.pc, pc_after_sync + 1);
}

#[test]
fn cwai_stacks_frame_then_interrupt_skips_restacking() {
    let mut s = Sys::code(0x1000, &[0x3C, 0xEF]); // CWAI #$EF (clears I mask)
    s.bus.load(0xFFF8, &[0x80, 0x00]); // IRQ vector
    s.cpu.s = 0x2000;
    s.cpu.cc = 0xFF; // everything set
    let cycles = s.step();
    assert_eq!(cycles, 22);
    assert_eq!(s.cpu.state, State::Waiting);
    assert_eq!(s.cpu.cc & cc::IRQ_MASK, 0); // ANDed with $EF cleared I
    let s_after_cwai = s.cpu.s;
    assert_eq!(s_after_cwai, 0x2000 - 12); // full frame stacked once

    // IRQ now allowed (I clear). Because CWAI already stacked, the interrupt
    // must NOT push a second frame.
    let serviced = s.cpu.irq(&mut s.bus);
    assert!(serviced);
    assert_eq!(s.cpu.s, s_after_cwai); // stack pointer unchanged — no re-stack
    assert_eq!(s.cpu.pc, 0x8000);
    assert_eq!(s.cpu.state, State::Running);
}
