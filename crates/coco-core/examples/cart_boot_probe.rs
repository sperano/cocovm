//! Scratch harness: boot an arbitrary cartridge image and dump periodic
//! framebuffer snapshots + CPU state so a human can see whether it comes up.
//!
//!     cargo run -p coco-core --example cart_boot_probe <cart.ccc> <outdir>

use coco_core::cart::ROMPak;
use coco_core::{Machine, MachineConfig};
use test_assets::rom;

fn write_ppm(m: &Machine, path: &str) {
    let (w, h) = (m.fb_width as usize, m.fb_height as usize);
    let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
    for px in m.framebuffer.as_chunks::<4>().0 {
        out.extend_from_slice(&px[..3]);
    }
    std::fs::write(path, out).unwrap();
    println!("wrote {path} ({w}x{h})");
}

fn main() {
    let cart_path = std::env::args().nth(1).expect("cart path");
    let dir = std::env::args().nth(2).unwrap_or_else(|| ".".into());
    let rom = std::fs::read(test_assets::rom(rom::COCO3))
        .unwrap()
        .into_boxed_slice();
    let cart = std::fs::read(&cart_path).unwrap();
    let mut m = Machine::new(MachineConfig::default(), rom);
    m.insert_cartridge(ROMPak::from_bytes(&cart, true).unwrap());
    m.reset();

    let stem = std::path::Path::new(&cart_path)
        .file_stem()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    for chunk in 0..6 {
        for _ in 0..120 {
            m.run_field();
        }
        println!(
            "+{}s pc={:04X} cc={:02X} init0={:02X} ff22={:02X} fb={}x{}",
            (chunk + 1) * 2,
            m.cpu.pc,
            m.cpu.cc,
            m.bus.gime.init0,
            m.bus.pia1.b.output,
            m.fb_width,
            m.fb_height,
        );
        write_ppm(&m, &format!("{dir}/{stem}-{chunk}.ppm"));
    }
}
