//! The `dw` command service answering the guest's own `dw` utility through
//! a host share: browse a folder, retrieve files, and insert, read, and
//! eject a disk image.

use std::fs;
use std::path::{Path, PathBuf};

use coco_core::Machine;
use coco_core::drivewire::share::{LeaseOwner, ShareAccess, ShareSpec, ShareTable};
use coco_core::drivewire::{GuestMediaChange, MediaOrigin};
use test_assets::disk::NOS9_L2_COCO3_40_TRACK;

use crate::common::{boot_nitros9, dw, run_until, run_until_prompt_after, screen, type_line};

/// The share name the guest types.
const SHARE: &str = "share";
/// A text file with CR line ends, as OS-9 text files have.
const HELLO_LINES: [&str; 2] = ["HELLO FROM THE HOST SHARE", "SECOND LINE OF THE FILE"];
/// `/N6`: the shell redirects `dw`'s output there.
const OUTPUT_CHANNEL: u8 = 6;
/// Two passes over every byte value: longer than one 255-byte block.
const PROBE_PASSES: usize = 2;
/// The guest's DriveWire drive 1 (`/X1`).
const DRIVE: usize = 1;

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("cocovm-dw-cmd-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Boots NitrOS-9 sharing `root`, or `None` to skip.
fn boot_sharing(root: &Path, access: ShareAccess) -> Option<Machine> {
    let mut m = boot_nitros9()?;
    let table = ShareTable::new(vec![ShareSpec {
        name: SHARE.to_string(),
        root: root.to_path_buf(),
        access,
    }])
    .unwrap();
    dw(&mut m).set_shares(table, LeaseOwner::new());
    Some(m)
}

#[test]
fn nitros9_dw_browses_a_share_and_prints_a_file() {
    let scratch = Scratch::new("browse");
    // `dw` copies the payload with I$Write: no line conversion, so a host
    // text file shows line by line only with CR LF line ends.
    let hello: String = HELLO_LINES
        .iter()
        .map(|line| format!("{line}\r\n"))
        .collect();
    fs::write(scratch.path().join("hello.txt"), hello).unwrap();
    fs::create_dir(scratch.path().join("games")).unwrap();
    let Some(mut m) = boot_sharing(scratch.path(), ShareAccess::ReadOnly) else {
        return;
    };

    type_line(&mut m, "dw server dir share");
    run_until_prompt_after(&mut m, "hello.txt");
    let text = screen(&mut m);
    assert!(text.contains("Directory of /share"), "screen:\n{text}");
    assert!(text.contains("games/"), "screen:\n{text}");
    assert!(
        !text.contains("OK command successful"),
        "dw strips the status line; screen:\n{text}"
    );

    type_line(&mut m, "dw server list share/hello.txt");
    run_until_prompt_after(&mut m, HELLO_LINES[1]);
    let text = screen(&mut m);
    let listed = &text[text.rfind("dw server list").unwrap()..];
    assert!(listed.contains(HELLO_LINES[0]), "screen:\n{text}");

    type_line(&mut m, "dw server list share/missing.txt");
    run_until_prompt_after(&mut m, "share/missing.txt: not found");
    let diagnostics = dw(&mut m).channel_diagnostics();
    assert_eq!(diagnostics.to_guest, 0, "every reply byte was read");
    assert_eq!(diagnostics.short_reads, 0);
    assert_eq!(diagnostics.unknown_channel_ops, 0);
    assert_eq!(dw(&mut m).unknown_opcodes(), 0);
}

#[test]
fn nitros9_dw_retrieves_every_byte_value() {
    let scratch = Scratch::new("bytes");
    let payload: Vec<u8> = (0..=u8::MAX).cycle().take(PROBE_PASSES * 256).collect();
    fs::write(scratch.path().join("probe.bin"), &payload).unwrap();
    let Some(mut m) = boot_sharing(scratch.path(), ShareAccess::ReadOnly) else {
        return;
    };

    type_line(&mut m, "dw server list share/probe.bin >/n6");
    let mut relayed = Vec::new();
    let mut output = None;
    run_until(&mut m, "the guest to close /N6", |m| {
        let server = dw(m);
        let info = server.channel_info(OUTPUT_CHANNEL).unwrap();
        output = output.or(info.handle);
        if let Some(handle) = output {
            relayed.extend(server.channel_receive(handle, usize::MAX).unwrap());
        }
        output.is_some() && !info.open
    });
    let first_difference = relayed.iter().zip(&payload).position(|(a, b)| a != b);
    assert_eq!(
        (relayed.len(), first_difference),
        (payload.len(), None),
        "relayed bytes must equal the file"
    );
}

#[test]
fn nitros9_dw_inserts_reads_and_ejects_an_image() {
    let scratch = Scratch::new("disk");
    let image = test_assets::disk(NOS9_L2_COCO3_40_TRACK);
    let Ok(bytes) = fs::read(&image) else {
        eprintln!("skipping: {} not present", image.display());
        return;
    };
    let host_path = scratch.path().join("nitros9 disk.dsk");
    fs::write(&host_path, &bytes).unwrap();
    // An OS-9 text file: CR line ends.
    let hello: String = HELLO_LINES.iter().map(|line| format!("{line}\r")).collect();
    fs::write(scratch.path().join("hello.txt"), &hello).unwrap();
    let Some(mut m) = boot_sharing(scratch.path(), ShareAccess::ReadWrite) else {
        return;
    };

    type_line(&mut m, "dw disk insert 1 share/nitros9 disk.dsk");
    run_until_prompt_after(&mut m, "Disk inserted in drive 1.");
    let media = dw(&mut m).drive_media(DRIVE).cloned().unwrap();
    assert_eq!(media.origin, MediaOrigin::Guest);
    assert!(!media.write_protected, "the share is read/write");
    assert_eq!(
        dw(&mut m).take_guest_media_changes(),
        vec![GuestMediaChange {
            drive: DRIVE,
            host_path: Some(host_path.clone()),
        }]
    );
    type_line(&mut m, "dw disk show");
    run_until_prompt_after(&mut m, "X1   /share/nitros9 disk.dsk");

    // Retrieve a file onto the inserted disk, then read it back through
    // the disk driver.
    let before = dw(&mut m).drive_ops(DRIVE);
    type_line(&mut m, "dw server list share/hello.txt >/x1/hello.txt");
    type_line(&mut m, "list /x1/hello.txt");
    run_until_prompt_after(&mut m, HELLO_LINES[1]);
    let text = screen(&mut m);
    let listed = &text[text.rfind("list /x1/hello.txt").unwrap()..];
    assert!(listed.contains(HELLO_LINES[0]), "screen:\n{text}");
    type_line(&mut m, "dir /x1");
    run_until_prompt_after(&mut m, "CMDS");
    assert!(dw(&mut m).drive_ops(DRIVE) > before);
    assert!(dw(&mut m).dirty(DRIVE), "the guest wrote to the image");

    type_line(&mut m, "dw disk eject 1");
    run_until_prompt_after(&mut m, "Disk ejected from drive 1.");
    assert!(!dw(&mut m).is_mounted(DRIVE));
    assert_ne!(
        fs::read(&host_path).unwrap(),
        bytes,
        "the host image changed"
    );
    type_line(&mut m, "dw disk eject 1");
    run_until_prompt_after(&mut m, "There is no disk in drive 1");
}
