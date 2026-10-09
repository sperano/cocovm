//! Visual harness for the GIME-native renderers: writes a synthetic 80-column
//! attribute text screen and an HSCREEN-2 colour-bar frame as PPM files for
//! eyeballing (pass an output directory, default `.`).
//!
//!     cargo run -p coco-core --example gime_demo /tmp

use coco_core::gime::GIME;
use coco_core::gime_video;

fn write_ppm(path: &str, fb: &[u8], w: usize, h: usize) {
    let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
    for px in fb.as_chunks::<4>().0 {
        out.extend_from_slice(&px[..3]);
    }
    std::fs::write(path, out).unwrap();
    println!("wrote {path} ({w}x{h})");
}

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    let mut ram = vec![0u8; 0x20000];
    let base = 0x8000usize;

    // --- 80-column text with attributes ---
    let mut g = GIME::new();
    g.vmode = 0x03; // BP=0, LPR=8
    g.vres = 0x15; // 80 cols, attributes
    g.vertical_offset = (base >> 3) as u16;
    g.border = 0x12;
    // A CoCo-ish palette: mimic the ROM's defaults loosely.
    g.palette = [0, 9, 18, 27, 36, 45, 54, 63, 0, 63, 46, 26, 12, 5, 38, 56];

    let msg = b"cocovm GIME 80-column text  ABCDEFGHIJKLMNOPQRSTUVWXYZ abcdefghijklmnopqrstuvwxyz 0123456789";
    for row in 0..24 {
        for col in 0..80 {
            let i = base + (row * 80 + col) * 2;
            ram[i] = msg[(col + row) % msg.len()];
            let fg = (row % 8) as u8;
            let bg = if row >= 16 { (row % 8) as u8 } else { 0 };
            let mut attr = (fg << 3) | bg;
            if row == 4 {
                attr |= 0x40; // underline
            }
            if row == 5 {
                attr |= 0x80; // blink
            }
            ram[i + 1] = attr;
        }
    }
    let mut fb = Vec::new();
    let (w, h) = gime_video::render_field(&g, &ram, false, &mut fb);
    write_ppm(&format!("{dir}/text80.ppm"), &fb, w, h);

    // --- HSCREEN 2: 320x192x16 colour bars ---
    g.vmode = 0x80;
    g.vres = 0x1E;
    for y in 0..192 {
        for bx in 0..160 {
            let c = ((bx * 16 / 160) as u8) & 0x0F;
            ram[base + y * 160 + bx] = c << 4 | ((c + y as u8 / 12) & 0x0F);
        }
    }
    let (w, h) = gime_video::render_field(&g, &ram, false, &mut fb);
    write_ppm(&format!("{dir}/hscreen2.ppm"), &fb, w, h);
}
