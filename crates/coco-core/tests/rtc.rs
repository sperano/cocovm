//! Disto MEB real-time clock ($FF50-$FF53): the 4-N-1's MSM6242 register
//! readout, settability and control bits; the 2-N-1's MSM5832 register map
//! as `clock2_disto2.asm` walks it; and routing through the cartridge port /
//! Multi-Pak.

use std::cell::Cell;
use std::rc::Rc;

use coco_core::Machine;
use coco_core::cart::{Cartridge, MultiPak};
use coco_core::config::MachineConfig;
use coco_core::rtc::{DistoRTC, DistoRTCModel, RTCTime};
use mc6809::Bus;
use test_assets::rom::COCO3;

const RTC_DATA: u16 = 0xFF50;
/// Register-select latch as `clock2_disto4.asm` (Disto 4-N-1) uses it.
const RTC_SELECT_DISTO4: u16 = 0xFF51;
/// Register-select latch as `clock2_disto2.asm` (Disto 2-N-1) uses it.
const RTC_SELECT_DISTO2: u16 = 0xFF52;

/// MSM6242 register numbers (MAME `msm6242.cpp` enum).
const REG_S1: u8 = 0;
const REG_H1: u8 = 4;
const REG_H10: u8 = 5;
const REG_Y10: u8 = 11;
const REG_CD: u8 = 13;
const REG_CF: u8 = 15;

/// MSM5832 register numbers (2-N-1): weekday at 6, date digits one up.
const MSM5832_REG_S10: u8 = 1;
const MSM5832_REG_MI10: u8 = 3;
const MSM5832_REG_H10: u8 = 5;
const MSM5832_REG_W: u8 = 6;
const MSM5832_REG_D10: u8 = 8;
const MSM5832_REG_MO10: u8 = 10;
const MSM5832_REG_Y10: u8 = 12;
/// First register number past the MSM5832's 13 (no control registers).
const MSM5832_REG_UNUSED: u8 = 13;
const MSM5832_UNUSED_READ: u8 = 0x0F;
/// MSM5832 hour-tens bit 3 (24-hour mode) and day-tens bit 2 (leap year).
const MSM5832_H10_24H: u8 = 0x08;
const MSM5832_D10_LEAP: u8 = 0x04;
/// `clock2_disto2.asm`'s hour-tens read mask (`anda #3`).
const MSM5832_H10_TENS_MASK: u8 = 0x03;

/// CD HOLD bit; CF RESET/STOP/24-12 bits.
const CD_HOLD: u8 = 0x01;
const CF_RESET: u8 = 0x01;
const CF_STOP: u8 = 0x02;
const CF_24H: u8 = 0x04;

const FIXED_TIME: RTCTime = RTCTime {
    year: 2026,
    month: 7,
    day: 8,
    hour: 21,
    minute: 34,
    second: 56,
};

fn fixed_rtc() -> DistoRTC {
    DistoRTC::new(Box::new(|| FIXED_TIME))
}

fn fixed_rtc_2n1() -> DistoRTC {
    DistoRTC::with_model(DistoRTCModel::TwoInOne, Box::new(|| FIXED_TIME))
}

/// An RTC whose time source's seconds-within-minute field the test can
/// advance by hand.
fn ticking_rtc() -> (DistoRTC, Rc<Cell<u8>>) {
    let seconds = Rc::new(Cell::new(0u8));
    let source = Rc::clone(&seconds);
    let rtc = DistoRTC::new(Box::new(move || RTCTime {
        second: source.get(),
        ..FIXED_TIME
    }));
    (rtc, seconds)
}

/// Read register `reg` the way a NitrOS-9 Disto driver does: write the
/// register number to the select latch, read the data port.
fn read_reg(cart: &mut dyn Cartridge, select: u16, reg: u8) -> u8 {
    cart.write(select, reg);
    cart.read(RTC_DATA)
}

fn write_reg(cart: &mut dyn Cartridge, select: u16, reg: u8, val: u8) {
    cart.write(select, reg);
    cart.write(RTC_DATA, val);
}

/// The NitrOS-9 `GetTime` walk: for each field year -> month -> day -> hour
/// -> minute -> second, select the tens register, read, select the ones
/// register, read (registers 11 down to 0, exactly the drivers' `decb`
/// pattern). The hour-tens read masks off the 12-hour-mode PM bit like the
/// drivers do.
fn nitros9_gettime(cart: &mut dyn Cartridge, select: u16) -> [u8; 6] {
    let mut fields = [0u8; 6];
    let mut reg = REG_Y10;
    for field in &mut fields {
        let tens = read_reg(cart, select, reg) & if reg == REG_H10 { 0x3 } else { 0xF };
        let ones = read_reg(cart, select, reg - 1);
        *field = tens * 10 + ones;
        reg = reg.wrapping_sub(2);
    }
    fields
}

#[test]
fn gettime_via_ff51_matches_injected_time_disto4_dialect() {
    let mut cart = fixed_rtc();
    let [y, mo, d, h, mi, s] = nitros9_gettime(&mut cart, RTC_SELECT_DISTO4);
    assert_eq!(
        (y, mo, d, h, mi, s),
        (26, 7, 8, 21, 34, 56),
        "expected the injected 2026-07-08 21:34:56"
    );
}

#[test]
fn four_in_one_also_latches_through_ff52() {
    // MAME's superset decode: every latch address reaches the same chip.
    let mut cart = fixed_rtc();
    let [y, mo, d, h, mi, s] = nitros9_gettime(&mut cart, RTC_SELECT_DISTO2);
    assert_eq!((y, mo, d, h, mi, s), (26, 7, 8, 21, 34, 56));
}

/// `clock2_disto2.asm`'s `GetTime` walk over the MSM5832: registers 12 down
/// to 7 for year/month/day, then 5 down to 0 for hour/minute/second, hour
/// tens masked with `anda #3`.
fn clock2_disto2_gettime(cart: &mut dyn Cartridge) -> [u8; 6] {
    let mut fields = [0u8; 6];
    let mut reg = MSM5832_REG_Y10;
    for (i, field) in fields.iter_mut().enumerate() {
        if i == 3 {
            reg = MSM5832_REG_H10;
        }
        let mask = if reg == MSM5832_REG_H10 {
            MSM5832_H10_TENS_MASK
        } else {
            0x0F
        };
        let tens = read_reg(cart, RTC_SELECT_DISTO2, reg) & mask;
        let ones = read_reg(cart, RTC_SELECT_DISTO2, reg - 1);
        *field = tens * 10 + ones;
        reg = reg.wrapping_sub(2);
    }
    fields
}

#[test]
fn two_in_one_gettime_matches_injected_time_disto2_dialect() {
    let mut cart = fixed_rtc_2n1();
    assert_eq!(clock2_disto2_gettime(&mut cart), [26, 7, 8, 21, 34, 56]);
}

#[test]
fn two_in_one_weekday_sits_at_register_6_with_leap_and_24h_flags() {
    // 2026-07-08 is a Wednesday (Sunday = 0); 2026 is not a leap year.
    let mut cart = fixed_rtc_2n1();
    assert_eq!(read_reg(&mut cart, RTC_SELECT_DISTO2, MSM5832_REG_W), 3);
    assert_eq!(
        read_reg(&mut cart, RTC_SELECT_DISTO2, MSM5832_REG_D10) & MSM5832_D10_LEAP,
        0
    );
    assert_eq!(
        read_reg(&mut cart, RTC_SELECT_DISTO2, MSM5832_REG_H10),
        MSM5832_H10_24H | 2,
        "power-on 24-hour mode shows in hour-tens bit 3"
    );
    let mut leap = DistoRTC::with_model(
        DistoRTCModel::TwoInOne,
        Box::new(|| RTCTime {
            year: 2024,
            ..FIXED_TIME
        }),
    );
    assert_eq!(
        read_reg(&mut leap, RTC_SELECT_DISTO2, MSM5832_REG_D10),
        MSM5832_D10_LEAP,
        "leap year sets day-tens bit 2 (day 08 -> tens 0)"
    );
}

#[test]
fn two_in_one_settime_writes_stick_in_disto2_register_order() {
    let mut cart = fixed_rtc_2n1();
    // `clock2_disto2.asm` SetTime: year (12/11) down to day (8/7), hour with
    // `$08` OR-ed into the tens (5/4), minute, second — tens then ones.
    let fields: [(u8, u8); 6] = [
        (MSM5832_REG_Y10, 99),
        (MSM5832_REG_MO10, 12),
        (MSM5832_REG_D10, 31),
        (MSM5832_REG_H10, 23),
        (MSM5832_REG_MI10, 59),
        (MSM5832_REG_S10, 10),
    ];
    for (tens_reg, value) in fields {
        let mut tens = value / 10;
        if tens_reg == MSM5832_REG_H10 {
            tens |= MSM5832_H10_24H;
        }
        write_reg(&mut cart, RTC_SELECT_DISTO2, tens_reg, tens);
        write_reg(&mut cart, RTC_SELECT_DISTO2, tens_reg - 1, value % 10);
    }
    assert_eq!(clock2_disto2_gettime(&mut cart), [99, 12, 31, 23, 59, 10]);
}

#[test]
fn two_in_one_twelve_hour_mode_is_hour_tens_bit_3() {
    let mut cart = fixed_rtc_2n1();
    // Writing the tens digit with bit 3 clear drops to 12-hour mode; the
    // digit written is the 12-hour one: tens 0 + PM (bit 2) keeps 21:xx as 9 PM.
    const PM: u8 = 0x04;
    write_reg(&mut cart, RTC_SELECT_DISTO2, MSM5832_REG_H10, PM);
    assert_eq!(read_reg(&mut cart, RTC_SELECT_DISTO2, MSM5832_REG_H10), 0x4);
    assert_eq!(
        read_reg(&mut cart, RTC_SELECT_DISTO2, MSM5832_REG_H10 - 1),
        9
    );
    assert_eq!(
        read_reg(&mut cart, RTC_SELECT_DISTO2, MSM5832_REG_UNUSED),
        MSM5832_UNUSED_READ,
        "no control registers: 13-15 read $0F like MAME's msm5832"
    );
}

#[test]
fn weekday_register_derives_from_date() {
    // 2026-07-08 is a Wednesday; W is 0-6 with Sunday = 0.
    let mut cart = fixed_rtc();
    const REG_W: u8 = 12;
    assert_eq!(read_reg(&mut cart, RTC_SELECT_DISTO4, REG_W), 3);
}

#[test]
fn defaults_to_24_hour_mode_for_the_init_less_disto2_driver() {
    // `clock2_disto2.asm` has no Init: it relies on the chip's power-on CF
    // (MAME `device_start` default) already being in 24-hour mode.
    let mut cart = fixed_rtc();
    assert_eq!(
        read_reg(&mut cart, RTC_SELECT_DISTO4, REG_CF) & CF_24H,
        CF_24H
    );
    assert_eq!(read_reg(&mut cart, RTC_SELECT_DISTO4, REG_H10), 2);
    assert_eq!(read_reg(&mut cart, RTC_SELECT_DISTO4, REG_H1), 1);
}

#[test]
fn twelve_hour_mode_latches_on_reset_release_and_sets_pm() {
    let mut cart = fixed_rtc();
    // The 24/12 bit only changes on a RESET 1 -> 0 transition.
    write_reg(&mut cart, RTC_SELECT_DISTO4, REG_CF, CF_RESET);
    write_reg(&mut cart, RTC_SELECT_DISTO4, REG_CF, 0x00); // RESET 1->0, H24 = 0
    assert_eq!(read_reg(&mut cart, RTC_SELECT_DISTO4, REG_CF) & CF_24H, 0);
    // 21:00 -> 9 PM: H10 = PM bit (bit 2) | 0 tens, H1 = 9.
    assert_eq!(read_reg(&mut cart, RTC_SELECT_DISTO4, REG_H10), 0x4);
    assert_eq!(read_reg(&mut cart, RTC_SELECT_DISTO4, REG_H1), 9);
}

#[test]
fn twelve_hour_mode_survives_later_cf_writes_and_reset_reads_back_clear() {
    // Regression: MAME's CF write keeps the old RESET bit on the release
    // branch, so RESET reads back stuck at 1 and any later CF write
    // re-latches 24/12. We store what was written instead — after entering
    // 12-hour mode, RESET must read 0 and a non-transition CF write (H24 set
    // but no RESET release) must NOT flip the mode back.
    let mut cart = fixed_rtc();
    write_reg(&mut cart, RTC_SELECT_DISTO4, REG_CF, CF_RESET);
    write_reg(&mut cart, RTC_SELECT_DISTO4, REG_CF, 0x00);
    assert_eq!(
        read_reg(&mut cart, RTC_SELECT_DISTO4, REG_CF),
        0x00,
        "RESET must not stick"
    );
    write_reg(&mut cart, RTC_SELECT_DISTO4, REG_CF, CF_24H);
    assert_eq!(
        read_reg(&mut cart, RTC_SELECT_DISTO4, REG_CF) & CF_24H,
        0,
        "24/12 must only latch on a RESET 1->0 transition"
    );
}

#[test]
fn twelve_hour_mode_shows_midnight_as_12_am() {
    let mut cart = DistoRTC::new(Box::new(|| RTCTime {
        hour: 0,
        ..FIXED_TIME
    }));
    write_reg(&mut cart, RTC_SELECT_DISTO4, REG_CF, CF_RESET);
    write_reg(&mut cart, RTC_SELECT_DISTO4, REG_CF, 0x00);
    // 00:34 -> 12 AM: H10 = 1 (no PM bit), H1 = 2.
    assert_eq!(read_reg(&mut cart, RTC_SELECT_DISTO4, REG_H10), 1);
    assert_eq!(read_reg(&mut cart, RTC_SELECT_DISTO4, REG_H1), 2);
}

#[test]
fn setime_style_register_writes_stick_and_clock_keeps_running() {
    let (mut cart, seconds) = ticking_rtc();
    // NitrOS-9 SetTime: freeze with HOLD, write the digits, release.
    write_reg(&mut cart, RTC_SELECT_DISTO4, REG_CD, CD_HOLD);
    let digits: [(u8, u8); 12] = [
        (11, 9), // Y10 -> 99
        (10, 9), // Y1
        (9, 1),  // MO10 -> 12
        (8, 2),  // MO1
        (7, 3),  // D10 -> 31
        (6, 1),  // D1
        (5, 2),  // H10 -> 23
        (4, 3),  // H1
        (3, 5),  // MI10 -> 59
        (2, 9),  // MI1
        (1, 1),  // S10 -> 10
        (0, 0),  // S1
    ];
    for (reg, val) in digits {
        write_reg(&mut cart, RTC_SELECT_DISTO4, reg, val);
    }
    write_reg(&mut cart, RTC_SELECT_DISTO4, REG_CD, 0x00);
    let [y, mo, d, h, mi, s] = nitros9_gettime(&mut cart, RTC_SELECT_DISTO4);
    assert_eq!((y, mo, d, h, mi, s), (99, 12, 31, 23, 59, 10));

    // The set clock runs: +5 host seconds -> +5 emulated seconds.
    seconds.set(5);
    assert_eq!(read_reg(&mut cart, RTC_SELECT_DISTO4, REG_S1), 5);
}

#[test]
fn hold_freezes_readout_without_losing_time() {
    let (mut cart, seconds) = ticking_rtc();
    write_reg(&mut cart, RTC_SELECT_DISTO4, REG_CD, CD_HOLD);
    seconds.set(7);
    assert_eq!(
        read_reg(&mut cart, RTC_SELECT_DISTO4, REG_S1),
        0,
        "held readout must freeze"
    );
    write_reg(&mut cart, RTC_SELECT_DISTO4, REG_CD, 0x00);
    assert_eq!(
        read_reg(&mut cart, RTC_SELECT_DISTO4, REG_S1),
        7,
        "no time lost across HOLD"
    );
}

#[test]
fn stop_loses_the_time_spent_stopped() {
    let (mut cart, seconds) = ticking_rtc();
    write_reg(&mut cart, RTC_SELECT_DISTO4, REG_CF, CF_24H | CF_STOP);
    seconds.set(9);
    assert_eq!(
        read_reg(&mut cart, RTC_SELECT_DISTO4, REG_S1),
        0,
        "stopped clock must not advance"
    );
    write_reg(&mut cart, RTC_SELECT_DISTO4, REG_CF, CF_24H);
    // Resumes from 0 seconds while the host source says 9: 9 seconds lost.
    assert_eq!(read_reg(&mut cart, RTC_SELECT_DISTO4, REG_S1), 0);
    seconds.set(11);
    assert_eq!(read_reg(&mut cart, RTC_SELECT_DISTO4, REG_S1), 2);
}

#[test]
fn reads_route_through_the_machine_bus_scs_window() {
    let mut m = Machine::new(MachineConfig::default(), load_rom(COCO3));
    m.insert_cartridge(fixed_rtc());
    m.power_cycle();
    // power_cycle() resets GIME state but never runs any ROM code, so INIT0
    // MC2 stays at its power-on 0: state the SCS-window precondition
    // explicitly, same as a real boot's cold-start probe would.
    m.bus.gime.write_init0(coco_core::gime::init0::MC2);
    m.bus.write(RTC_SELECT_DISTO4, REG_S1);
    assert_eq!(m.bus.read(RTC_DATA), 6, "S1 of :56 through the live bus");
}

#[test]
fn rtc_in_a_multipak_slot_answers_when_scs_selected() {
    const RTC_SLOT: usize = 2; // slot 3, the classic RTC-next-to-FD-502 spot
    let mut mpi = MultiPak::new(3);
    mpi.insert(RTC_SLOT, fixed_rtc());

    // Not SCS-selected (switch on slot 4): the empty slot answers instead.
    assert_eq!(
        read_reg(&mut mpi, RTC_SELECT_DISTO4, REG_S1),
        coco_core::cart::IO_OPEN_BUS
    );

    // The driver's $FF7F write selects the RTC's slot, like on real hardware.
    let select_rtc_slot = coco_core::cart::mpi::SWITCH_VALUES[RTC_SLOT];
    mpi.control_write(select_rtc_slot);
    assert_eq!(read_reg(&mut mpi, RTC_SELECT_DISTO4, REG_S1), 6);

    // And the frontend can still find the clock behind the MPI.
    assert!(mpi.find_disto_rtc().is_some());
}

/// Same ROM-loading convention as `tests/cart.rs`.
fn load_rom(name: &str) -> Box<[u8]> {
    let path = test_assets::rom(name);
    std::fs::read(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
        .into_boxed_slice()
}
