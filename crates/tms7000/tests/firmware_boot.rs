//! The Sound/Speech Cartridge firmware booting on the core: reaches its idle
//! loop without an illegal opcode, sets up its ports and interrupts.
//! Skips when `~/.local/share/cocovm/roms/ssc-tms7040.rom` is not installed.

use tms7000::{FlatBoard, Port, ROM_SIZE, StepKind, TMS7040};

/// INT3 enable in IOCNT0.
const INT3_ENABLE: u8 = 0x10;
/// Port C bit 7 is BUSY* to the host; the firmware raises it when ready.
const BUSY_HIGH: u8 = 0x80;
/// Cycles to give the firmware to finish its power-on initialisation.
const BOOT_CYCLES: u64 = 200_000;

fn load_firmware() -> Option<Vec<u8>> {
    let rom = std::fs::read(test_assets::rom(test_assets::rom::SSC_TMS7040)).ok()?;
    assert_eq!(rom.len(), ROM_SIZE, "installed firmware image size");
    Some(rom)
}

#[test]
fn firmware_boots_to_its_idle_loop() {
    let Some(rom) = load_firmware() else {
        eprintln!("skipping firmware_boots_to_its_idle_loop: ssc-tms7040.rom not present");
        return;
    };
    let mut cpu = TMS7040::new(&rom).unwrap();
    let mut board = FlatBoard::new();
    // The SP0256's load request idles high into INT1.
    cpu.set_int1(true);
    let mut pcs_seen = std::collections::HashSet::new();
    while cpu.cycles < BOOT_CYCLES {
        let step = cpu.step(&mut board);
        if step.kind == StepKind::Instruction {
            pcs_seen.insert(cpu.pc);
        }
    }
    assert_eq!(cpu.illegal_count, 0, "firmware hit an illegal opcode");
    assert_eq!(cpu.port_ddr(Port::C), 0xFF, "port C drives the board");
    assert_eq!(cpu.port_ddr(Port::D), 0xFF, "port D drives the data bus");
    assert!(
        board
            .writes
            .iter()
            .any(|&(port, v)| port == Port::C && v & BUSY_HIGH != 0),
        "BUSY* released"
    );
    assert_eq!(
        cpu.io_control() & INT3_ENABLE,
        INT3_ENABLE,
        "waiting for a host byte: IOCNT0 {:#04x}",
        cpu.io_control()
    );
    assert!(
        pcs_seen.len() > 20,
        "ran real code, not a tight trap: {} distinct PCs",
        pcs_seen.len()
    );
}
