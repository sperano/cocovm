use coco_core::ssc::{SoundSpeechCartridge, reg as ssc_reg};
use coco_core::{MachineVariant, MemorySize, SystemBus};
use mc6809::Bus;
use test_assets::rom::{SP0256_AL2, SSC_TMS7040};

/// Generator step for `sound_probe` (the AY drain is call-count based, so
/// this only feeds the (absent) crystal generators).
pub const PROBE_DT: f64 = 1.0 / 62_866.0;

pub const FF7D: u16 = ssc_reg::RESET;
pub const FF7E: u16 = ssc_reg::DATA;

/// `$FF7E` bit 7: set while the firmware is ready for a byte.
pub const NOT_BUSY: u8 = 0x80;
/// `$FF7E` bit 6: SP0256 SBY, set while idle.
pub const SPEECH_READY: u8 = 0x40;
/// `$FF7E` bit 5: Sound Activity Circuit, set while the PSG is quiet.
pub const QUIET: u8 = 0x20;

/// E-cycles per tick while polling the status byte.
pub const POLL_STEP: u32 = 8;
/// Cycles the firmware's power-on initialisation is given before a test
/// starts talking to it.
pub const BOOT_CYCLES: u32 = 100_000;
/// Bound on how long the firmware may hold BUSY* for one byte; its longest
/// command paths finish well inside this.
pub const BUSY_TIMEOUT_CYCLES: u32 = 400_000;
/// The firmware releases BUSY* before it acts on a command (the manual:
/// "the speech and sound status bits are not valid immediately following
/// a speech or sound execution command"), queueing bytes it hasn't
/// processed; a burst of ten takes it ~25,000 cycles to drain. This is how
/// long tests give it before looking at the chips.
pub const SETTLE_CYCLES: u32 = 60_000;
/// Cycles the firmware spends per queued byte, for bursts longer than the
/// ten [`SETTLE_CYCLES`] budgets for.
pub const CYCLES_PER_QUEUED_BYTE: u32 = 3_000;

/// Zero-filled images: the firmware executes NOPs forever and the speech
/// chip halts on any load, so the bus, handshake, and PSG plumbing can be
/// exercised without the real ROMs.
pub const BLANK_FIRMWARE: [u8; tms7000::ROM_SIZE] = [0; tms7000::ROM_SIZE];
pub const BLANK_SPEECH_ROM: [u8; coco_core::sp0256::ROM_SIZE] = [0; coco_core::sp0256::ROM_SIZE];

pub fn blank_ssc() -> SoundSpeechCartridge {
    SoundSpeechCartridge::new(&BLANK_FIRMWARE, &BLANK_SPEECH_ROM).expect("blank ROMs fit")
}

/// The real cartridge, or `None` when either ROM isn't installed.
pub fn try_real_ssc() -> Option<SoundSpeechCartridge> {
    let firmware = std::fs::read(test_assets::rom(SSC_TMS7040)).ok()?;
    let speech = std::fs::read(test_assets::rom(SP0256_AL2)).ok()?;
    Some(SoundSpeechCartridge::new(&firmware, &speech).expect("installed ROMs are the right size"))
}

pub fn bus_with(
    cart: SoundSpeechCartridge,
    variant: MachineVariant,
    memory: MemorySize,
) -> SystemBus {
    let mut b = SystemBus::new(variant, memory, vec![0u8; 32 * 1024].into_boxed_slice());
    b.cart = cart.into();
    b
}

pub fn bus_with_ssc(variant: MachineVariant, memory: MemorySize) -> SystemBus {
    bus_with(blank_ssc(), variant, memory)
}

/// A CoCo 3 with a blank-ROM cartridge.
pub fn coco3_bus_with_ssc() -> SystemBus {
    bus_with_ssc(MachineVariant::Coco3, MemorySize::K512)
}

/// A CoCo 3 with the real cartridge, booted; `None` without the ROMs.
pub fn try_coco3_bus_with_ssc() -> Option<SystemBus> {
    let mut b = bus_with(try_real_ssc()?, MachineVariant::Coco3, MemorySize::K512);
    boot(&mut b);
    Some(b)
}

const PIA0_CRA: u16 = 0xFF01;
const PIA0_CRB: u16 = 0xFF03;
const PIA1_CRB: u16 = 0xFF23;
/// Control value: data register selected, C2 set/reset output low/high.
const CR_C2_LOW: u8 = 0x34;
const CR_C2_HIGH: u8 = 0x3C;
/// Control value selecting the DDR (bit 2 clear) — unused here because this
/// suite only drives Cx2 (SNDEN/SEL1/SEL2), never the DAC's data pins.
const CR_DDR: u8 = 0x30;

/// Route the sound mux to the cartridge input (SEL2:SEL1 = 10, SNDEN high)
/// — same PIA-poking pattern as `tests/sound.rs`'s `bus()` helper.
pub fn select_cartridge_mux(b: &mut SystemBus) {
    b.write(PIA1_CRB, CR_DDR);
    b.write(PIA1_CRB, CR_C2_HIGH); // SNDEN high
    b.write(PIA0_CRA, CR_C2_LOW); // SEL1 = 0
    b.write(PIA0_CRB, CR_C2_HIGH); // SEL2 = 1 -> mux 10: cartridge
}

/// A blank-ROM cartridge with the mux listening to it.
pub fn bus_with_ssc_selected() -> SystemBus {
    let mut b = coco3_bus_with_ssc();
    select_cartridge_mux(&mut b);
    b
}

/// The real cartridge, booted, with the mux listening to it; `None`
/// without the ROMs.
pub fn try_bus_with_ssc_selected() -> Option<SystemBus> {
    let mut b = try_coco3_bus_with_ssc()?;
    select_cartridge_mux(&mut b);
    Some(b)
}

/// Let the firmware finish its power-on initialisation.
pub fn boot(b: &mut SystemBus) {
    b.cart.tick(BOOT_CYCLES);
}

/// Tick until `$FF7E` bit 7 sets; returns the cycles that took.
pub fn wait_not_busy(b: &mut SystemBus) -> u32 {
    let mut waited = 0;
    while b.read(FF7E) & NOT_BUSY == 0 {
        b.cart.tick(POLL_STEP);
        waited += POLL_STEP;
        assert!(waited < BUSY_TIMEOUT_CYCLES, "firmware never cleared BUSY*");
    }
    waited
}

/// Hand the firmware one byte and wait for it to accept it.
pub fn send(b: &mut SystemBus, byte: u8) {
    b.write(FF7E, byte);
    wait_not_busy(b);
}

/// Give the firmware time to act on what it accepted.
pub fn settle(b: &mut SystemBus) {
    b.cart.tick(SETTLE_CYCLES);
}

/// [`settle`] scaled for a burst of `bytes` queued bytes.
pub fn settle_bytes(b: &mut SystemBus, bytes: u32) {
    b.cart.tick(SETTLE_CYCLES + bytes * CYCLES_PER_QUEUED_BYTE);
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
