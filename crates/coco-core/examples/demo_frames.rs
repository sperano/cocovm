//! Run a DECB `LOADM` binary (a demo like SockMaster's Boink) headless and
//! dump canonical-raster frames as PPM — the harness for eyeballing
//! per-scanline effects against MAME screenshots.
//!
//! Usage: `cargo run -p coco-core --example demo_frames -- <file.bin> <outdir> [128|512|2048]`
//!
//! The binary is injected exactly as `LOADM` would (segments poked through
//! the MMU-mapped bus at the idle BASIC prompt) and the CPU jumped to its
//! exec address. Many 1990s demos (Boink included) assume a 128K machine's
//! physical-address aliasing — pass `128` for those.

use std::path::PathBuf;

use coco_core::{Machine, MachineConfig, MemorySize};
use mc6809::Bus;
use test_assets::rom;

/// Fields to run before injecting — enough to reach the idle BASIC prompt.
const BOOT_FIELDS: usize = 150;
/// Fields to run after exec, dumping one frame every `DUMP_EVERY`.
const RUN_FIELDS: usize = 360;
const DUMP_EVERY: usize = 60;

fn main() {
    let mut args = std::env::args().skip(1);
    let bin_path = PathBuf::from(
        args.next()
            .expect("usage: demo_frames <file.bin> <outdir> [ram-kb]"),
    );
    let out = PathBuf::from(
        args.next()
            .expect("usage: demo_frames <file.bin> <outdir> [ram-kb]"),
    );
    let memory = match args.next().as_deref() {
        Some("128") => MemorySize::K128,
        Some("2048") => MemorySize::K2048,
        _ => MemorySize::K512,
    };

    let rom = std::fs::read(test_assets::rom(rom::COCO3))
        .expect("coco3.rom in the cocovm XDG data directory")
        .into_boxed_slice();
    let bin = std::fs::read(&bin_path).unwrap_or_else(|e| panic!("{}: {e}", bin_path.display()));

    let config = MachineConfig {
        memory,
        ..MachineConfig::default()
    };
    let mut m = Machine::new(config, rom);
    for _ in 0..BOOT_FIELDS {
        m.run_field();
    }

    // DECB LOADM segment format: $00,len16,addr16,data... repeated; $FF,0,exec16.
    let mut i = 0;
    let exec_addr = loop {
        match bin[i] {
            0x00 => {
                let len = usize::from(bin[i + 1]) << 8 | usize::from(bin[i + 2]);
                let addr = u16::from(bin[i + 3]) << 8 | u16::from(bin[i + 4]);
                for (k, &b) in bin[i + 5..i + 5 + len].iter().enumerate() {
                    m.bus.write(addr.wrapping_add(k as u16), b);
                }
                i += 5 + len;
            }
            0xFF => break u16::from(bin[i + 3]) << 8 | u16::from(bin[i + 4]),
            tag => panic!("unexpected LOADM tag {tag:02X} at offset {i}"),
        }
    };
    m.cpu.pc = exec_addr;

    std::fs::create_dir_all(&out).unwrap();
    for field in 1..=RUN_FIELDS {
        m.run_field();
        if field % DUMP_EVERY == 0 {
            let (w, h) = (m.fb_width as usize, m.fb_height as usize);
            let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
            for px in m.framebuffer.chunks_exact(4) {
                ppm.extend_from_slice(&px[..3]);
            }
            let path = out.join(format!("frame_{field:03}.ppm"));
            std::fs::write(&path, ppm).unwrap();
            println!("{} ({w}x{h}, pc={:04X})", path.display(), m.cpu.pc);
        }
    }
}
