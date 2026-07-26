use coco_core::ssc::{reg as ssc_reg, Ssc};
use coco_core::{MachineVariant, MemorySize, SystemBus};
use mc6809::Bus;

/// Generator step for `sound_probe` (the AY drain is call-count based, so
/// this only feeds the (absent) crystal generators).
pub const PROBE_DT: f64 = 1.0 / 62_866.0;

pub const FF7D: u16 = ssc_reg::RESET;
pub const FF7E: u16 = ssc_reg::DATA;

/// Synthetic hold time for `busy` after a `$FF7E` write (see
/// `crates/coco-core/src/ssc.rs`'s `BUSY_HOLD_CYCLES` doc comment) — not
/// exported, so tests need their own large-enough tick count to clear busy
/// between protocol bytes. Comfortably larger than the real constant (100).
pub const CLEAR_BUSY: u32 = 1_000;

pub fn bus_with_ssc(variant: MachineVariant, memory: MemorySize) -> SystemBus {
    let mut b = SystemBus::new(variant, memory, vec![0u8; 32 * 1024].into_boxed_slice());
    b.cart = Ssc::new().into();
    b
}

pub fn coco3_bus_with_ssc() -> SystemBus {
    bus_with_ssc(MachineVariant::Coco3, MemorySize::K512)
}

const PIA0_CRA: u16 = 0xFF01;
const PIA0_CRB: u16 = 0xFF03;
const PIA1_CRB: u16 = 0xFF23;
/// Control value: data register selected, C2 set/reset output low/high.
const CR_C2_LOW: u8 = 0x34;
const CR_C2_HIGH: u8 = 0x3C;
/// Control value selecting the DDR (bit 2 clear) -- unused here since this
/// suite only drives Cx2 (SNDEN/SEL1/SEL2), never the DAC's data pins.
const CR_DDR: u8 = 0x30;

/// A bus with an `Ssc` inserted and the sound mux routed to the cartridge
/// input (SEL2:SEL1 = 10, SNDEN high) — same PIA-poking pattern as
/// `tests/sound.rs`'s `bus()` helper.
pub fn bus_with_ssc_selected() -> SystemBus {
    let mut b = coco3_bus_with_ssc();
    b.write(PIA1_CRB, CR_DDR);
    b.write(PIA1_CRB, CR_C2_HIGH); // SNDEN high
    b.write(PIA0_CRA, CR_C2_LOW); // SEL1 = 0
    b.write(PIA0_CRB, CR_C2_HIGH); // SEL2 = 1 -> mux 10: cartridge
    b
}

/// Advance the cart's clock and pump `count` `sound_probe` calls, returning
/// the last sample.
pub fn pump(b: &mut SystemBus, count: u32) -> f32 {
    let mut last = 0.0;
    for _ in 0..count {
        b.cart.tick(100);
        last = b.sound_probe(PROBE_DT)[0];
    }
    last
}
