//! Booting the NitrOS-9 Becker image to its shell and driving it from the
//! keyboard and the host side of the DriveWire channels.

use std::collections::HashMap;

use coco_core::cart::ROMPak;
use coco_core::drivewire::{CHANNEL_COUNT, ChannelHandle, DWImage, DWServer};
use coco_core::{Machine, MachineConfig};
use test_assets::{
    disk::NOS9_L2_COCO3_BECKER,
    rom::{COCO3, HDBDW3BC3},
};

/// Upper bound for each wait: the same budget as the NitrOS-9 DriveWire boot
/// test in `drivewire_boot.rs`.
const MAX_FIELDS: usize = 12_000;
/// Fields run between screen or channel checks.
const POLL_FIELDS: usize = 60;
/// Shell+'s prompt on this image (see `drivewire_boot.rs`).
pub const SHELL_PROMPT: &str = "{Term|02}/DD:";

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

pub fn type_line(m: &mut Machine, line: &str) {
    for c in line.chars().chain(['\r']) {
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
}

pub fn screen(m: &mut Machine) -> String {
    m.text_screen_lines().join("\n")
}

/// Runs fields until `done` holds, panicking with the screen after
/// [`MAX_FIELDS`].
pub fn run_until(m: &mut Machine, what: &str, mut done: impl FnMut(&mut Machine) -> bool) {
    let mut fields = 0;
    while !done(m) {
        assert!(
            fields < MAX_FIELDS,
            "timed out waiting for {what}; screen:\n{}",
            screen(m)
        );
        for _ in 0..POLL_FIELDS {
            m.run_field();
        }
        fields += POLL_FIELDS;
    }
}

/// Waits until `text` appears on screen with the shell prompt after it.
pub fn run_until_prompt_after(m: &mut Machine, text: &str) {
    run_until(m, &format!("the prompt after {text:?}"), |m| {
        let screen = screen(m);
        screen
            .rfind(text)
            .is_some_and(|at| screen[at..].contains(SHELL_PROMPT))
    });
}

pub fn dw(m: &mut Machine) -> &mut DWServer {
    m.bus.drivewire.as_mut().unwrap()
}

/// Collects guest output from every channel session into `inbox` and
/// returns the first session whose output satisfies `complete`.
pub fn collect(
    m: &mut Machine,
    inbox: &mut HashMap<ChannelHandle, Vec<u8>>,
    complete: impl Fn(&[u8]) -> bool,
) -> Option<ChannelHandle> {
    let server = dw(m);
    let handles: Vec<ChannelHandle> = (0..CHANNEL_COUNT as u8)
        .filter_map(|channel| server.channel_info(channel)?.handle)
        .collect();
    handles.into_iter().find(|&handle| {
        let received = server.channel_receive(handle, usize::MAX).unwrap();
        let buffered = inbox.entry(handle).or_default();
        buffered.extend(received);
        complete(buffered)
    })
}

/// Boots NitrOS-9 over DriveWire to the shell prompt, or `None` to skip.
pub fn boot_nitros9() -> Option<Machine> {
    let read = |path: std::path::PathBuf| match std::fs::read(&path) {
        Ok(bytes) => Some(bytes),
        Err(_) => {
            eprintln!(
                "skipping DriveWire channel test: {} not present",
                path.display()
            );
            None
        }
    };
    let coco = read(test_assets::rom(COCO3))?;
    let hdbdos = read(test_assets::rom(HDBDW3BC3))?;
    let disk = read(test_assets::disk(NOS9_L2_COCO3_BECKER))?;

    let mut m = Machine::new(MachineConfig::default(), coco.into_boxed_slice());
    m.insert_cartridge(ROMPak::from_bytes(&hdbdos, false).unwrap());
    m.bus.enable_drivewire();
    m.reset();
    run_until(&mut m, "the HDB-DOS OK prompt", |m| {
        screen(m).contains("OK")
    });
    // NitrOS-9's own rbdw sends per-drive LSNs, so HDB-DOS mode stays off.
    dw(&mut m).mount(0, DWImage::Memory(disk));
    type_line(&mut m, "DOS");
    run_until(&mut m, "the NitrOS-9 shell prompt", |m| {
        screen(m).contains(SHELL_PROMPT)
    });
    Some(m)
}
