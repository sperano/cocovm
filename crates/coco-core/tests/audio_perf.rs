//! Coarse perf guard for the audio pipeline (`docs/plan-audio-pipeline.md`
//! risk list): a busy field loop must stay far faster than real time even
//! in debug builds — event recording on the io_write path and the 4-slot
//! grid flush are supposed to be branch-cheap.

use std::time::Instant;

use coco_core::{Machine, MachineConfig};
use mc6809::Bus;

#[test]
fn field_loop_stays_well_faster_than_real_time() {
    let mut m = Machine::new(
        MachineConfig::default(),
        vec![0u8; 32 * 1024].into_boxed_slice(),
    );
    // A tight DAC-hammering loop — the audio-event worst case: every third
    // instruction is an audio write. STA $FF20 / INCA / BRA loop.
    m.bus.write(0x0000, 0xB7); // STA $FF20
    m.bus.write(0x0001, 0xFF);
    m.bus.write(0x0002, 0x20);
    m.bus.write(0x0003, 0x4C); // INCA
    m.bus.write(0x0004, 0x20); // BRA -6
    m.bus.write(0x0005, 0xF9);

    const FIELDS: u32 = 120; // 2 seconds of emulated time
    let start = Instant::now();
    for _ in 0..FIELDS {
        m.run_field();
        m.take_audio().count();
    }
    let elapsed = start.elapsed().as_secs_f64();
    let emulated = f64::from(FIELDS) / m.config.video.field_rate_hz();
    assert!(
        elapsed < emulated,
        "audio-hammering field loop slower than real time even for a debug \
         build: {elapsed:.2}s wall for {emulated:.2}s emulated"
    );
    println!("{FIELDS} fields: {elapsed:.3}s wall vs {emulated:.3}s emulated");
}
