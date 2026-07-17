//! Visual probe for the three text character generators: boots a CoCo 1
//! (plain MC6847), a CoCo 2 with the MC6847T1, and a CoCo 3 (GIME
//! `lowres_font`) to the BASIC prompt and writes each framebuffer as a PPM
//! for eyeballing (square vs rounded 'O', GIME glyphs one pixel left).
//!
//!     cargo run -p coco-core --example vdg_font_probe <outdir>

use std::path::PathBuf;

use coco_core::{
    Machine, MachineConfig, MachineVariant, MemorySize, MonitorType, VdgVariant, VideoStandard,
};

const BOOT_FIELDS: usize = 120;

fn try_load(name: &str) -> Option<Vec<u8>> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../roms")
        .join(name);
    std::fs::read(&path).ok()
}

fn write_ppm(path: &str, fb: &[u8], w: usize, h: usize) {
    let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
    for px in fb.chunks_exact(4) {
        out.extend_from_slice(&px[..3]);
    }
    std::fs::write(path, out).unwrap();
    println!("wrote {path} ({w}x{h})");
}

fn boot_and_dump(config: MachineConfig, rom: Box<[u8]>, path: &str) {
    let mut m = Machine::new(config, rom);
    for _ in 0..BOOT_FIELDS {
        m.run_field();
    }
    println!("--- {path}\n{}", m.text_screen_lines().join("\n"));
    write_ppm(
        path,
        &m.framebuffer,
        m.fb_width as usize,
        m.fb_height as usize,
    );
}

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| ".".into());

    let coco12_rom = || -> Box<[u8]> {
        let mut image = try_load("extbas11.rom").expect("extbas11.rom");
        image.extend_from_slice(&try_load("bas12.rom").expect("bas12.rom"));
        image.into_boxed_slice()
    };
    let base12 = MachineConfig {
        variant: MachineVariant::Coco1,
        video: VideoStandard::Ntsc,
        memory: MemorySize::K64,
        monitor: MonitorType::Rgb,
        vdg: VdgVariant::Mc6847,
    };

    boot_and_dump(base12, coco12_rom(), &format!("{dir}/coco1_mc6847.ppm"));
    boot_and_dump(
        MachineConfig {
            variant: MachineVariant::Coco2,
            vdg: VdgVariant::Mc6847T1,
            ..base12
        },
        coco12_rom(),
        &format!("{dir}/coco2_t1.ppm"),
    );
    boot_and_dump(
        MachineConfig {
            variant: MachineVariant::Coco3,
            memory: MemorySize::K512,
            vdg: VdgVariant::Mc6847,
            ..base12
        },
        try_load("coco3.rom").expect("coco3.rom").into_boxed_slice(),
        &format!("{dir}/coco3_gime.ppm"),
    );
}
