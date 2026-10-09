//! Boot NitrOS-9 EOU from the VHD, start gshell, and dump the GIME video
//! state (palette registers, $FF98/$FF99) to diagnose color rendering —
//! specifically whether EOU writes composite-monitor palette encodings
//! (MONTYPE=0) that our RGB-only `GIME::rgb_color` misdecodes.
//!
//! Usage: cargo run -p coco-core --example eou_gshell_probe --release

use coco_core::fdc::{DiskCart, JVCDisk};
use coco_core::vhd::VHDImage;
use coco_core::{Machine, MachineConfig, MonitorType};
use test_assets::{disk, rom};

fn tap(m: &mut Machine, pos: (u8, u8)) {
    for _ in 0..3 {
        m.bus.keyboard.set(pos, true);
        m.run_field();
    }
    m.bus.keyboard.set(pos, false);
    for _ in 0..3 {
        m.run_field();
    }
}

fn tap_char(m: &mut Machine, c: char) {
    let (pos, shift) =
        coco_core::keyboard::char_key(c).unwrap_or_else(|| panic!("no key for {c:?}"));
    if shift {
        m.bus.keyboard.set(coco_core::keyboard::SHIFT, true);
    }
    tap(m, pos);
    if shift {
        m.bus.keyboard.set(coco_core::keyboard::SHIFT, false);
    }
}

fn type_str(m: &mut Machine, s: &str) {
    for c in s.chars() {
        tap_char(m, c);
    }
}

fn wait_for(m: &mut Machine, marker: &str, max_fields: usize) -> String {
    const POLL: usize = 300;
    let mut fields = 0;
    loop {
        for _ in 0..POLL {
            m.run_field();
        }
        fields += POLL;
        let screen = m.text_screen_lines().join("\n");
        if screen.contains(marker) || fields >= max_fields {
            return screen;
        }
    }
}

fn dump_video(m: &Machine, label: &str) {
    let g = &m.bus.gime;
    println!("== {label}");
    println!(
        "   vmode($FF98)={:#04x} vres($FF99)={:#04x} border($FF9A)={:#04x}",
        g.vmode, g.vres, g.border
    );
    print!("   palette:");
    for (i, p) in g.palette.iter().enumerate() {
        print!(" [{i}]={p:#04x}");
    }
    println!();
}

fn main() {
    let coco = std::fs::read(test_assets::rom(rom::COCO3)).expect("coco3.rom");
    let disk_rom = std::fs::read(test_assets::rom(rom::DISK11)).expect("disk11.rom");
    let dsk = std::fs::read(test_assets::disk(disk::EOU_BOOT)).expect("68EMU.dsk");
    let vhd_src = test_assets::disk(disk::EOU_SYSTEM_VHD);
    let vhd_copy = std::env::temp_dir().join("cocovm-eou-probe.VHD");
    std::fs::copy(&vhd_src, &vhd_copy).expect("copy VHD");
    let vhd_file = std::fs::File::options()
        .read(true)
        .write(true)
        .open(&vhd_copy)
        .expect("open VHD");

    let mut m = Machine::new(MachineConfig::default(), coco.into_boxed_slice());
    let mut cart = DiskCart::new(disk_rom.into_boxed_slice());
    cart.insert_disk(0, JVCDisk::from_bytes(dsk).unwrap());
    m.insert_cartridge(cart);
    m.bus.vhd.insert(0, VHDImage::File(vhd_file));
    m.reset();

    for _ in 0..300 {
        m.run_field();
    }
    type_str(&mut m, "DOS");
    tap_char(&mut m, '\r');
    wait_for(&mut m, "Time ?", 12_000);
    tap_char(&mut m, '\r');
    let screen = wait_for(&mut m, "}/DD:", 12_000);
    println!("-- at shell --\n{screen}\n");
    dump_video(&m, "text shell");

    type_str(&mut m, "gshell");
    tap_char(&mut m, '\r');
    for _ in 0..3_000 {
        m.run_field();
    }
    dump_video(&m, "after gshell (3000 fields)");
    println!(
        "-- text_screen_lines now --\n{}",
        m.text_screen_lines().join("\n")
    );

    // Screenshot the same gshell frame through both monitor types.
    if let Some(dir) = std::env::args().nth(1) {
        for (monitor, name) in [
            (MonitorType::RGB, "gshell-rgb.ppm"),
            (MonitorType::Composite, "gshell-cmp.ppm"),
        ] {
            m.bus.gime.monitor = monitor;
            m.run_field();
            write_ppm(
                &format!("{dir}/{name}"),
                &m.framebuffer,
                m.fb_width as usize,
                m.fb_height as usize,
            );
        }
        println!("wrote {dir}/gshell-rgb.ppm and {dir}/gshell-cmp.ppm");
    }

    let _ = std::fs::remove_file(&vhd_copy);
}

fn write_ppm(path: &str, fb: &[u8], w: usize, h: usize) {
    let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
    for px in fb.as_chunks::<4>().0 {
        out.extend_from_slice(&px[..3]);
    }
    std::fs::write(path, out).unwrap();
}
