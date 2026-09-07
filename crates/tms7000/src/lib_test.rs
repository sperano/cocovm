use super::*;

fn zero_rom() -> Vec<u8> {
    vec![0; ROM_SIZE]
}

#[test]
fn new_rejects_a_wrong_sized_rom() {
    assert_eq!(
        TMS7040::new(&[0; ROM_SIZE + 1]).err(),
        Some(ROMSizeError {
            actual: ROM_SIZE + 1
        })
    );
    assert!(TMS7040::new(&zero_rom()).is_ok());
}

#[test]
fn default_is_a_powered_off_chip_with_a_reset_pending() {
    let cpu = TMS7040::default();
    assert!(cpu.pending_reset());
    assert_eq!(cpu.port_ddr(Port::B), 0xFF, "port B is output-only");
    assert_eq!(cpu.port_ddr(Port::C), 0);
    assert_eq!(cpu.peek(ROM_BASE), 0, "no ROM reads as 0");
}

#[test]
fn first_step_runs_the_reset_sequence() {
    let mut rom = zero_rom();
    rom[ROM_SIZE - 2] = 0xF1;
    rom[ROM_SIZE - 1] = 0x23;
    let mut cpu = TMS7040::new(&rom).unwrap();
    let mut board = FlatBoard::new();
    let step = cpu.step(&mut board);
    assert_eq!(step.kind, StepKind::Reset);
    assert_eq!(step.cycles, 17);
    assert_eq!(cpu.pc, 0xF123);
    assert!(!cpu.pending_reset());
    assert_eq!(cpu.cycles, 17);
}

#[test]
fn validate_accepts_the_default_chip() {
    assert!(TMS7040::default().validate().is_ok());
}

/// Port A has no DDR register (writes to it are ignored) and is hardwired
/// all-input; a deserialized chip with any other value is unreachable.
#[test]
fn validate_rejects_port_a_ddr_off_its_hardwired_value() {
    let mut cpu = TMS7040::default();
    cpu.port_ddr[Port::A.index()] = 1;
    assert!(cpu.validate().is_err());
}

/// Port B's DDR is hardwired all-output (MAME `device_start`); there is no
/// BDDR register to write it to any other value.
#[test]
fn validate_rejects_port_b_ddr_off_its_hardwired_value() {
    let mut cpu = TMS7040::default();
    cpu.port_ddr[Port::B.index()] = 0;
    assert!(cpu.validate().is_err());
}

#[test]
fn peek_covers_register_file_rom_and_peripheral_latches() {
    let mut rom = zero_rom();
    rom[0x10] = 0xAB;
    let mut cpu = TMS7040::new(&rom).unwrap();
    cpu.set_rf(5, 0x42);
    assert_eq!(cpu.peek(0x0005), 0x42);
    assert_eq!(cpu.peek(0x0080), 0, "unmapped register file");
    assert_eq!(cpu.peek(ROM_BASE + 0x10), 0xAB);
    assert_eq!(cpu.peek(0x0109), 0, "DDR C");
    assert_eq!(cpu.peek(0x2000), 0, "external space is not peeked");
}
