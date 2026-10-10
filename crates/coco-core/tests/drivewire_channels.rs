//! Real-guest acceptance for DriveWire virtual serial channels: the stock
//! NitrOS-9 Level 2 3.3.0 Becker image (`tests/nos96809l2v030300coco3_becker.dsk`)
//! boots with `dwio`, `scdwv`, `/N`, `/N1`–`/N13` in its bootfile and `dw` in
//! `CMDS`. The test plays the host side of a channel the way the DriveWire 4
//! Java server's command thread does (`DWUtilDWThread`): it reads the `dw`
//! request, replies `OK command successful` (LF, CR) followed by the payload,
//! and hangs up once the payload is queued. The `dw` client prints the payload
//! and exits only after its poller delivers the close as `S$HUP`, so the shell
//! prompt returning after the last payload line is the guest observing EOF.
//! Skips when an asset is absent.

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
const SHELL_PROMPT: &str = "{Term|02}/DD:";
/// The Java server's success envelope: `OK command successful`, LF, CR.
const OK_ENVELOPE: &[u8] = b"OK command successful\n\r";
/// Payload lines, each 25 bytes with CR LF, so the reply spans several
/// 255-byte poll advertisements and the guest's receive buffer.
const PAYLOAD_LINES: usize = 20;

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

fn type_line(m: &mut Machine, line: &str) {
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

fn screen(m: &mut Machine) -> String {
    m.text_screen_lines().join("\n")
}

/// Runs fields until `done` holds, panicking with the screen after
/// [`MAX_FIELDS`].
fn run_until(m: &mut Machine, what: &str, mut done: impl FnMut(&mut Machine) -> bool) {
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

fn dw(m: &mut Machine) -> &mut DWServer {
    m.bus.drivewire.as_mut().unwrap()
}

/// Collects guest output from every channel session into `inbox` and
/// returns the first session whose output satisfies `complete`.
fn collect(
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
fn boot_nitros9() -> Option<Machine> {
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

fn payload() -> Vec<u8> {
    (1..=PAYLOAD_LINES)
        .flat_map(|line| format!("CHANNEL LINE {line:02} OF {PAYLOAD_LINES}\r\n").into_bytes())
        .collect()
}

/// Waits for a `dw` request, answers it like the Java command thread, and
/// returns the request bytes.
fn serve_dw_request(m: &mut Machine, reply: &[u8]) -> Vec<u8> {
    let mut inbox = HashMap::new();
    let mut session = None;
    run_until(m, "a dw request on a channel", |m| {
        session = collect(m, &mut inbox, |bytes| {
            bytes.starts_with(b"dw ") && bytes.contains(&b'\r')
        });
        session.is_some()
    });
    let handle = session.unwrap();

    let mut sent = 0;
    run_until(m, "the guest to accept the whole reply", |m| {
        sent += dw(m).channel_send(handle, &reply[sent..]).unwrap();
        sent == reply.len()
    });
    dw(m).channel_hangup(handle).unwrap();
    inbox.remove(&handle).unwrap()
}

#[test]
fn nitros9_dw_client_exchanges_data_and_sees_close() {
    let Some(mut m) = boot_nitros9() else {
        return;
    };
    let payload = payload();
    let mut reply = OK_ENVELOPE.to_vec();
    reply.extend(&payload);

    type_line(&mut m, "dw server dir share");
    let request = serve_dw_request(&mut m, &reply);
    assert_eq!(request, b"dw server dir share\r");

    let last_line = format!("CHANNEL LINE {PAYLOAD_LINES:02} OF {PAYLOAD_LINES}");
    run_until(&mut m, "the prompt after the payload", |m| {
        let text = screen(m);
        let last = text.rfind(&last_line);
        last.is_some_and(|at| text[at..].contains(SHELL_PROMPT))
    });
    let text = screen(&mut m);
    assert!(
        !text.contains("OK command successful"),
        "dw strips the status line; screen:\n{text}"
    );
    let diagnostics = dw(&mut m).channel_diagnostics();
    assert_eq!(diagnostics.to_guest, 0, "every reply byte was read");
    assert_eq!(diagnostics.short_reads, 0);
    assert_eq!(diagnostics.unknown_channel_ops, 0);
    assert_eq!(dw(&mut m).unknown_opcodes(), 0);
}

#[test]
fn nitros9_shell_redirection_writes_to_a_numbered_channel() {
    /// `/N5`: a numbered descriptor that the boot leaves unused.
    const CHANNEL: u8 = 5;
    let Some(mut m) = boot_nitros9() else {
        return;
    };
    assert!(!dw(&mut m).channel_info(CHANNEL).unwrap().open);

    type_line(&mut m, "echo HELLO FROM NITROS9 >/n5");
    let mut inbox = HashMap::new();
    let mut session = None;
    run_until(&mut m, "echo output on /N5", |m| {
        session = collect(m, &mut inbox, |bytes| bytes.contains(&b'\r'));
        session.is_some()
    });
    let handle = session.unwrap();
    assert_eq!(handle.channel(), CHANNEL);

    run_until(&mut m, "the guest to close /N5", |m| {
        !dw(m).channel_info(CHANNEL).unwrap().open
    });
    let mut received = inbox.remove(&handle).unwrap();
    received.extend(dw(&mut m).channel_receive(handle, usize::MAX).unwrap());
    // Shell+ passes the space before the redirection as part of the argument.
    assert_eq!(
        String::from_utf8_lossy(&received),
        "HELLO FROM NITROS9 \r",
        "output written before the close stays readable"
    );
    assert_eq!(dw(&mut m).channel_diagnostics().dropped_bytes, 0);
}

#[test]
fn nitros9_dw_client_relays_every_byte_value_to_another_channel() {
    /// `/N6`: the shell redirects `dw`'s output there.
    const OUTPUT_CHANNEL: u8 = 6;
    /// Two passes over every byte value: longer than one 255-byte block.
    const PASSES: usize = 2;
    let Some(mut m) = boot_nitros9() else {
        return;
    };
    let payload: Vec<u8> = (0..=u8::MAX).cycle().take(PASSES * 256).collect();
    let mut reply = OK_ENVELOPE.to_vec();
    reply.extend(&payload);

    type_line(&mut m, "dw server list share/probe.bin >/n6");
    let request = serve_dw_request(&mut m, &reply);
    assert_eq!(request, b"dw server list share/probe.bin \r");

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
        "relayed bytes must equal the payload"
    );
}
