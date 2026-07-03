//! Scratch harness: boot a cart and log every GIME palette-register change
//! with the PC that made it, plus periodic palette snapshots.

use coco_core::cart::RomPak;
use coco_core::{Machine, MachineConfig};

fn main() {
    let cart_path = std::env::args().nth(1).expect("cart path");
    let rom = std::fs::read("roms/coco3.rom").unwrap().into_boxed_slice();
    let cart = std::fs::read(&cart_path).unwrap();
    let mut m = Machine::new(MachineConfig::default(), rom);
    m.insert_cartridge(Box::new(RomPak::from_bytes(&cart, true).unwrap()));
    m.reset();

    let stop_pulse_at: Option<u32> = std::env::args()
        .nth(2)
        .map(|s| s.parse().expect("field number"));

    let mut writes = 0u64;
    let mut sync_steps = 0u64;
    for field in 0..600u32 {
        if stop_pulse_at == Some(field) {
            // Swap in the same image with autostart=false: the CART line goes
            // quiet, ROM reads unchanged.
            let quiet = RomPak::from_bytes(&std::fs::read(&cart_path).unwrap(), false).unwrap();
            m.bus.cart = Box::new(quiet);
            println!("--- field {field}: CART line silenced");
        }
        for _line in 0..262 {
            let cycles_per_line = if m.bus.gime.cpu_fast { 114 } else { 57 };
            let mut spent = 0u32;
            while spent < cycles_per_line {
                if m.bus.firq_asserted() {
                    m.cpu.firq(&mut m.bus);
                }
                if m.bus.irq_asserted() {
                    m.cpu.irq(&mut m.bus);
                }
                let before = m.bus.gime.palette;
                if matches!(m.cpu.state, mc6809::State::Syncing) {
                    sync_steps += 1;
                }
                spent += m.step();
                if m.bus.gime.palette != before {
                    writes += 1;
                    if writes <= 40 || writes.is_multiple_of(500) {
                        println!(
                            "f{field} w{writes}: pc={:04X} palette={:02X?}",
                            m.cpu.pc, m.bus.gime.palette
                        );
                    }
                }
            }
            m.bus.hsync();
            let ticks = if m.bus.gime.timer_is_fast() { 228 } else { 1 };
            m.bus.gime.tick_timer(ticks);
        }
        m.bus.vsync();
        if field % 120 == 0 {
            println!(
                "--- field {field}: palette={:02X?} sync_steps={sync_steps}",
                m.bus.gime.palette
            );
        }
    }
    println!("total palette writes: {writes}, sync steps: {sync_steps}");
}
