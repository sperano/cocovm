use super::*;

#[test]
fn hardware_reset_state_reads_tdre_only() {
    let mut acia = ACIA6551::new();
    assert_eq!(acia.read(1), status::TDRE);
    assert!(!acia.irq_asserted());
}

#[test]
fn tdr_write_clears_tdre_then_consume_at_start_resets_it() {
    let mut acia = ACIA6551::new();
    acia.write(2, command::DTR); // DTR enabled, tx-IRQ off (RTS_OFF)
    acia.write(0, 0x55);
    // TDRE clears then immediately re-sets via write_tdr's consume-at-start check.
    assert_eq!(acia.read(1) & status::TDRE, status::TDRE);
}

#[test]
fn consume_at_start_fires_irq_only_when_tx_irq_enabled() {
    // tx-IRQ disabled (RTS_OFF): no IRQ on consume.
    let mut acia = ACIA6551::new();
    acia.write(2, command::DTR);
    acia.write(0, 0x11);
    assert!(!acia.irq_asserted());

    // tx-IRQ enabled (transmitter control = IRQ_ENABLED).
    let mut acia = ACIA6551::new();
    let cmd = command::DTR | (tx_control::IRQ_ENABLED << command::TX_CONTROL_SHIFT);
    acia.write(2, cmd);
    acia.write(0, 0x11);
    assert!(acia.irq_asserted());
    assert_eq!(acia.read(1) & status::IRQ, status::IRQ);
}

/// Baud index 15 = 19200 (divider 6); default control word length 8, no
/// parity, 1 stop bit -> frame_bits = 10.
#[test]
fn take_tx_byte_after_exact_frame_cycles_baud_19200() {
    let mut acia = ACIA6551::new();
    acia.write(2, command::DTR);
    acia.write(3, 15); // baud index 15
    acia.write(0, 0xA5);

    const EXPECTED_CYCLES: u32 = 466; // 10 * 6 * 16 * 894_886 / 1_843_200
    acia.tick(EXPECTED_CYCLES - 1);
    assert_eq!(acia.take_tx_byte(), None);
    acia.tick(1);
    assert_eq!(acia.take_tx_byte(), Some(0xA5));
}

/// Baud index 8 = 1200 (divider 96); frame_bits = 10.
#[test]
fn take_tx_byte_after_exact_frame_cycles_baud_1200() {
    let mut acia = ACIA6551::new();
    acia.write(2, command::DTR);
    acia.write(3, 8); // baud index 8
    acia.write(0, 0x7E);

    const EXPECTED_CYCLES: u32 = 7457; // 10 * 96 * 16 * 894_886 / 1_843_200
    acia.tick(EXPECTED_CYCLES - 1);
    assert_eq!(acia.take_tx_byte(), None);
    acia.tick(1);
    assert_eq!(acia.take_tx_byte(), Some(0x7E));
}

#[test]
fn rdrf_set_after_receive_byte_and_frame_time_rdr_read_clears_it() {
    let mut acia = ACIA6551::new();
    acia.write(2, command::DTR);
    assert!(acia.rx_ready());
    acia.receive_byte(0x42);
    assert!(!acia.rx_ready());

    let cycles = acia_test_cycles_per_frame(&acia);
    acia.tick(cycles - 1);
    assert_eq!(acia.read(1) & status::RDRF, 0);
    acia.tick(1);
    assert_eq!(acia.read(1) & status::RDRF, status::RDRF);
    assert!(acia.rx_ready());

    assert_eq!(acia.read(0), 0x42);
    assert_eq!(acia.read(1) & status::RDRF, 0);
}

#[test]
fn overrun_set_when_second_byte_completes_before_rdr_read_rdr_still_replaces() {
    let mut acia = ACIA6551::new();
    acia.write(2, command::DTR);
    let cycles = acia_test_cycles_per_frame(&acia);

    acia.receive_byte(0x01);
    acia.tick(cycles);
    assert!(acia.rx_ready());
    assert_eq!(acia.read(1) & status::RDRF, status::RDRF);

    // RDR not read yet: second byte completes and still replaces RDR, setting overrun.
    acia.receive_byte(0x02);
    acia.tick(cycles);

    let status_val = acia.read(1);
    assert_eq!(status_val & status::OVERRUN, status::OVERRUN);
    assert_eq!(status_val & status::RDRF, status::RDRF);
    assert_eq!(acia.read(0), 0x02);
}

#[test]
fn status_read_clears_irq_output_but_not_rdrf() {
    let mut acia = ACIA6551::new();
    acia.write(2, command::DTR); // rx-IRQ enabled by default (bit1 clear)
    let cycles = acia_test_cycles_per_frame(&acia);
    acia.receive_byte(0x9); // arbitrary
    acia.tick(cycles);

    assert!(acia.irq_asserted());
    let status_val = acia.read(1);
    assert_eq!(status_val & status::IRQ, status::IRQ);
    assert!(!acia.irq_asserted());
    // RDRF must survive the status read.
    assert_eq!(acia.read(1) & status::RDRF, status::RDRF);
}

#[test]
fn rx_irq_disable_suppresses_rdrf_irq() {
    let mut acia = ACIA6551::new();
    acia.write(2, command::DTR | command::RX_IRQ_DISABLE);
    let cycles = acia_test_cycles_per_frame(&acia);
    acia.receive_byte(0x55);
    acia.tick(cycles);
    assert_eq!(acia.read(1) & status::RDRF, status::RDRF);
    assert!(!acia.irq_asserted());
}

#[test]
fn dtr_disabled_blocks_transmit_and_rx_irq() {
    let mut acia = ACIA6551::new();
    // DTR left disabled (command defaults to 0).
    acia.write(0, 0x99);
    assert_eq!(acia.read(1) & status::TDRE, 0); // write clears TDRE...
    acia.tick(10_000); // ...but nothing ever transmits without DTR
    assert_eq!(acia.take_tx_byte(), None);
    assert_eq!(acia.read(1) & status::TDRE, 0);

    acia.receive_byte(0x11);
    let cycles = acia_test_cycles_per_frame(&acia);
    acia.tick(cycles);
    // RDRF still sets (frame still "arrives"), but rx-IRQ never arms with DTR disabled.
    assert_eq!(acia.read(1) & status::RDRF, status::RDRF);
    assert!(!acia.irq_asserted());
}

#[test]
fn programmed_reset_clears_overrun_and_command_bits_0_4_preserves_parity_and_control() {
    let mut acia = ACIA6551::new();
    acia.write(3, 7); // control: baud index 7, non-default
    let parity_odd = 1u8 << command::PARITY_SHIFT;
    acia.write(
        2,
        command::DTR
            | command::ECHO
            | parity_odd
            | (tx_control::RTS_ON << command::TX_CONTROL_SHIFT),
    );
    let cycles = acia_test_cycles_per_frame(&acia);
    acia.receive_byte(0x01);
    acia.tick(cycles);
    acia.receive_byte(0x02);
    acia.tick(cycles);
    assert_eq!(acia.read(1) & status::OVERRUN, status::OVERRUN);

    acia.write(1, 0); // programmed reset, value ignored
    assert_eq!(acia.read(1) & status::OVERRUN, 0);
    // Bits 0-4 cleared: DTR off, rx-IRQ enabled, tx control RTS_OFF, echo off.
    assert_eq!(acia.command & 0x1F, 0);
    // Parity bits (7:5) survive.
    assert_eq!(acia.command & command::PARITY_MASK, parity_odd);
    // Control register untouched.
    assert_eq!(acia.control, 7);
}

#[test]
fn programmed_reset_preserves_rdrf_and_tdre_irq_sources() {
    let mut acia = ACIA6551::new();
    let cmd = command::DTR | (tx_control::IRQ_ENABLED << command::TX_CONTROL_SHIFT);
    acia.write(2, cmd);
    acia.write(0, 0xAA); // consume-at-start arms the TDRE IRQ source
    assert!(acia.irq_asserted());

    acia.write(1, 0); // programmed reset
    // TDRE's IRQ survives programmed reset even though DTR is now off.
    assert!(acia.irq_asserted());
}

#[test]
fn dcd_change_sets_status_bit_and_raises_irq_only_while_dtr_enabled() {
    let mut acia = ACIA6551::new();
    // DTR disabled: level tracks, but no IRQ.
    acia.set_dcd(true);
    acia.tick(0);
    assert_eq!(acia.read(1) & status::DCD, status::DCD);
    assert!(!acia.irq_asserted());

    acia.set_dcd(false);
    acia.tick(0);
    acia.write(2, command::DTR);
    acia.set_dcd(true);
    acia.tick(0);
    assert!(acia.irq_asserted());
    assert_eq!(acia.read(1) & status::DCD, status::DCD);
}

#[test]
fn echo_mode_retransmits_received_bytes() {
    let mut acia = ACIA6551::new();
    acia.write(2, command::DTR | command::ECHO);
    let cycles = acia_test_cycles_per_frame(&acia);
    acia.receive_byte(0x5A);
    acia.tick(cycles);
    assert_eq!(acia.take_tx_byte(), Some(0x5A));
}

/// Test-only helper mirroring `cycles_per_frame` for the ACIA's current
/// config, so RX tests don't hardcode a value that only holds by default.
fn acia_test_cycles_per_frame(acia: &ACIA6551) -> u32 {
    acia.cycles_per_frame()
}
