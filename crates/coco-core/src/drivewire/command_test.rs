use std::collections::HashMap;
use std::path::Path;

use super::line::MAX_LINE_BYTES;
use crate::drivewire::host::{HostExecutor, ManualHost};
use crate::drivewire::share::tests::{ScratchDir, table};
use crate::drivewire::share::{LeaseOwner, ShareAccess};
use crate::drivewire::{CHANNEL_BUFFER_BYTES, DWServer, SS_CLOSE, SS_OPEN, opcode};

/// `/N1`: the first numbered channel.
const CH: u8 = 1;
const OTHER_CH: u8 = 2;
const QUEUE_CAPACITY: usize = 16;
/// Largest `OP_SERWRITEM` block.
const MAX_WRITE_BLOCK: usize = 255;
/// Poll reply byte 1 for a status reply; byte 2 holds the channel.
const POLL_STATUS: u8 = 0x10;
const POLL_CHANNEL_MASK: u8 = 0x0F;
const OK: &[u8] = b"OK command successful\n\r";

/// A guest driving the server over the wire, with host jobs run on demand.
pub(in crate::drivewire) struct Guest {
    pub server: DWServer,
    host: ManualHost,
    cycle: u64,
    /// Bytes read so far per channel.
    received: HashMap<u8, Vec<u8>>,
}

impl Guest {
    pub fn new() -> Self {
        let (executor, host) = HostExecutor::manual(QUEUE_CAPACITY);
        Self {
            server: DWServer::with_host_executor(executor),
            host,
            cycle: 0,
            received: HashMap::new(),
        }
    }

    /// A guest whose VM shares `shares`.
    pub fn sharing(shares: &[(&str, &Path, ShareAccess)]) -> Self {
        let mut guest = Self::new();
        guest.server.set_shares(table(shares), LeaseOwner::new());
        guest
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Vec<u8> {
        for &byte in bytes {
            self.server.data_write(byte, self.cycle);
            self.cycle += 1;
        }
        let mut reply = Vec::new();
        while self.server.status_read() != 0 {
            reply.push(self.server.data_read());
        }
        reply
    }

    /// Runs queued host jobs until the server submits no more.
    pub fn pump(&mut self) {
        self.server.poll_host();
        while let Some(request) = self.host.take_request() {
            self.host
                .complete(request.run())
                .expect("completion queue has room");
            self.server.poll_host();
        }
    }

    pub fn open(&mut self, channel: u8) {
        assert!(
            self.feed(&[opcode::SERSETSTAT, channel, SS_OPEN])
                .is_empty()
        );
        self.received.remove(&channel);
    }

    pub fn close(&mut self, channel: u8) {
        assert!(
            self.feed(&[opcode::SERSETSTAT, channel, SS_CLOSE])
                .is_empty()
        );
    }

    pub fn write(&mut self, channel: u8, bytes: &[u8]) {
        for block in bytes.chunks(MAX_WRITE_BLOCK) {
            let mut request = vec![opcode::SERWRITEM, channel, block.len() as u8];
            request.extend_from_slice(block);
            assert!(self.feed(&request).is_empty());
        }
    }

    /// Polls once, reading any block; returns the channel reported closed.
    pub fn poll_once(&mut self) -> Option<Option<u8>> {
        let reply = self.feed(&[opcode::SERREAD]);
        match reply[..] {
            [0, 0] => None,
            [POLL_STATUS, status] => Some(Some(status & POLL_CHANNEL_MASK)),
            [flag, count] => {
                let channel = (flag & POLL_CHANNEL_MASK) - 1;
                let data = self.feed(&[opcode::SERREADM, channel, count]);
                self.received.entry(channel).or_default().extend(data);
                Some(None)
            }
            _ => panic!("bad poll reply {reply:?}"),
        }
    }

    /// Reads until `channel` hangs up, returning its bytes, or `None` if
    /// the server stops making progress first.
    pub fn read_until_hangup(&mut self, channel: u8) -> Option<Vec<u8>> {
        loop {
            self.pump();
            match self.poll_once() {
                None => return None,
                Some(Some(closed)) if closed == channel => {
                    return Some(self.received.remove(&channel).unwrap_or_default());
                }
                Some(_) => {}
            }
        }
    }

    /// Runs `line` the way NitrOS-9's `dw` does and returns the reply.
    pub fn run(&mut self, line: &str) -> Vec<u8> {
        self.open(CH);
        self.write(CH, format!("{line}\r").as_bytes());
        let reply = self
            .read_until_hangup(CH)
            .unwrap_or_else(|| panic!("{line}: no hangup"));
        self.close(CH);
        reply
    }

    pub fn run_text(&mut self, line: &str) -> String {
        String::from_utf8(self.run(line)).expect("ASCII reply")
    }
}

fn ok_with(payload: &[u8]) -> Vec<u8> {
    [OK, payload].concat()
}

fn games(name: &str) -> (ScratchDir, Guest) {
    let dir = ScratchDir::new(name);
    let guest = Guest::sharing(&[("games", dir.path(), ShareAccess::ReadOnly)]);
    (dir, guest)
}

#[test]
fn dir_lists_entries_with_a_heading() {
    let (dir, mut guest) = games("dir");
    dir.file("b.bin", b"x");
    dir.file("My File.txt", b"x");
    dir.dir("sub");
    assert_eq!(
        guest.run_text("dw server dir games"),
        "OK command successful\n\rDirectory of /games\r\n\nMy File.txt\r\nb.bin\r\nsub/\r\n"
    );
}

#[test]
fn dir_of_the_top_level_lists_the_shares() {
    let (_dir, mut guest) = games("dir-top");
    assert_eq!(
        guest.run_text("dw server dir /"),
        "OK command successful\n\rDirectory of /\r\n\ngames/\r\n"
    );
}

#[test]
fn an_empty_directory_lists_only_the_heading() {
    let (dir, mut guest) = games("dir-empty");
    dir.dir("empty");
    assert_eq!(
        guest.run(" dw  server dir games/empty  "),
        ok_with(b"Directory of /games/empty\r\n\n")
    );
}

#[test]
fn a_large_directory_streams_every_entry() {
    /// Enough long names to span several host pages and channel buffers.
    const FILES: usize = 600;
    let (dir, mut guest) = games("dir-large");
    let names: Vec<String> = (0..FILES)
        .map(|index| format!("entry {index:04} with a deliberately long name.txt"))
        .collect();
    for name in &names {
        dir.file(name, b"");
    }
    let listing: String = names.iter().map(|name| format!("{name}\r\n")).collect();
    assert!(listing.len() > 2 * CHANNEL_BUFFER_BYTES);
    let reply = guest.run("dw server dir games");
    assert_eq!(
        reply,
        ok_with(format!("Directory of /games\r\n\n{listing}").as_bytes())
    );
}

#[test]
fn list_returns_file_bytes_unchanged() {
    /// More than two host chunks, and every byte value, CR and NUL included.
    const LEN: usize = 10_000;
    let (dir, mut guest) = games("list");
    let bytes: Vec<u8> = (0..=u8::MAX).cycle().take(LEN).collect();
    dir.file("data/probe file.bin", &bytes);
    assert_eq!(
        guest.run("dw server list games/data/probe file.bin"),
        ok_with(&bytes)
    );
    assert_eq!(
        guest.run("dw server list games/data/probe file.bin "),
        ok_with(&bytes)
    );
}

#[test]
fn an_empty_file_replies_ok_and_hangs_up() {
    let (dir, mut guest) = games("list-empty");
    dir.file("empty.txt", b"");
    assert_eq!(guest.run("dw server list games/empty.txt"), OK);
}

#[test]
fn missing_and_wrong_type_paths_fail_with_201() {
    let (dir, mut guest) = games("list-missing");
    dir.dir("sub");
    for (line, text) in [
        ("dw server list games/nope.bin", "games/nope.bin: not found"),
        ("dw server list games/sub", "games/sub: is a directory"),
        ("dw server dir games/nope", "games/nope: not found"),
        ("dw server list other/a.bin", "other/a.bin: no such share"),
    ] {
        assert_eq!(
            guest.run_text(line),
            format!("FAIL 201 {text}\n\r"),
            "{line}"
        );
    }
}

#[test]
fn paths_cannot_leave_the_shares() {
    let (dir, mut guest) = games("list-escape");
    let outside = ScratchDir::new("list-escape-outside");
    let secret = outside.file("secret.txt", b"secret");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&secret, dir.path().join("link.txt")).unwrap();
    assert_eq!(
        guest.run_text("dw server list games/../../etc/passwd"),
        "FAIL 201 games/../../etc/passwd: no such share\n\r"
    );
    // A host absolute path names a share at the guest's top level.
    let absolute = guest.run_text(&format!("dw server list {}", secret.display()));
    assert!(absolute.starts_with("FAIL 201 "), "{absolute}");
    assert!(absolute.ends_with(": no such share\n\r"), "{absolute}");
    #[cfg(unix)]
    assert_eq!(
        guest.run_text("dw server list games/link.txt"),
        "FAIL 201 games/link.txt: path leaves the share\n\r"
    );
}

#[cfg(unix)]
#[test]
fn a_file_the_host_denies_fails_with_201() {
    use std::os::unix::fs::PermissionsExt;
    let (dir, mut guest) = games("list-denied");
    let path = dir.file("locked.bin", b"x");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::File::open(&path).is_ok() {
        eprintln!("skipping: the host user can read mode 000 files");
        return;
    }
    assert_eq!(
        guest.run_text("dw server list games/locked.bin"),
        "FAIL 201 games/locked.bin: permission denied\n\r"
    );
}

#[test]
fn a_command_split_across_writes_runs_once_complete() {
    let (dir, mut guest) = games("partial");
    dir.file("a.txt", b"hello");
    guest.open(CH);
    guest.write(CH, b"dw server li");
    guest.pump();
    assert_eq!(guest.poll_once(), None, "nothing before the CR");
    guest.write(CH, b"st games/a.txt\r");
    assert_eq!(guest.read_until_hangup(CH), Some(ok_with(b"hello")));
}

#[test]
fn a_session_closed_before_its_cr_is_forgotten() {
    let (dir, mut guest) = games("partial-close");
    dir.file("a.txt", b"hello");
    guest.open(CH);
    guest.write(CH, b"dw server list games/a.t");
    guest.pump();
    guest.close(CH);
    guest.pump();
    assert_eq!(guest.run("dw server list games/a.txt"), ok_with(b"hello"));
}

#[test]
fn the_hangup_follows_the_last_byte() {
    let (dir, mut guest) = games("eof");
    let bytes = vec![b'z'; CHANNEL_BUFFER_BYTES + 1];
    dir.file("big.bin", &bytes);
    guest.open(CH);
    guest.write(CH, b"dw server list games/big.bin\r");
    guest.pump();
    // The channel is full; the close may not be reported until it drains.
    let info = guest.server.channel_info(CH).unwrap();
    assert_eq!(info.to_guest, CHANNEL_BUFFER_BYTES);
    assert!(!info.closing);
    let reply = guest.read_until_hangup(CH).unwrap();
    assert_eq!(reply, ok_with(&bytes));
    assert!(!guest.server.channel_info(CH).unwrap().open);
}

#[test]
fn closing_mid_stream_releases_the_file() {
    let (dir, mut guest) = games("abandon");
    let path = dir.file("big.bin", &vec![b'q'; 3 * CHANNEL_BUFFER_BYTES]);
    guest.open(CH);
    guest.write(CH, b"dw server list games/big.bin\r");
    guest.pump();
    assert_eq!(guest.poll_once(), Some(None), "first block read");
    guest.close(CH);
    guest.pump();
    // A writer elsewhere would conflict with a reader that was still open.
    let mut writer = Guest::sharing(&[("rw", dir.path(), ShareAccess::ReadWrite)]);
    assert_eq!(
        writer.run_text("dw disk insert 0 rw/big.bin"),
        "OK command successful\n\rDisk inserted in drive 0.\r\n"
    );
    assert!(path.exists());
}

#[test]
fn lines_for_other_services_stay_on_the_channel() {
    let (_dir, mut guest) = games("other");
    guest.open(CH);
    guest.write(CH, b"HELLO FROM NITROS9 \r");
    guest.pump();
    assert_eq!(guest.poll_once(), None);
    let handle = guest.server.channel_info(CH).unwrap().handle.unwrap();
    assert_eq!(
        guest.server.channel_receive(handle, usize::MAX).unwrap(),
        b"HELLO FROM NITROS9 \r"
    );
}

#[test]
fn a_command_line_without_cr_fails_at_the_limit() {
    let (_dir, mut guest) = games("too-long");
    guest.open(CH);
    let mut line = b"dw server list ".to_vec();
    line.resize(MAX_LINE_BYTES, b'a');
    guest.write(CH, &line);
    assert_eq!(
        guest.read_until_hangup(CH).unwrap(),
        b"FAIL 010 Command line too long\n\r"
    );
}

#[test]
fn bare_dw_lists_the_commands() {
    let mut guest = Guest::new();
    assert_eq!(
        guest.run_text("dw"),
        // `colLayout` pads every name, the last one included.
        "OK command successful\n\rPossible commands:\r\n\r\ndisk    server  \r\n"
    );
}

#[test]
fn a_protocol_reset_ends_a_running_command() {
    let (dir, mut guest) = games("reset");
    dir.file("a.txt", b"hello");
    guest.open(CH);
    guest.write(CH, b"dw server list games/a.txt\r");
    assert!(guest.feed(&[opcode::RESET1]).is_empty());
    guest.pump();
    assert_eq!(guest.poll_once(), None, "no reply after the reset");
    assert_eq!(guest.run("dw server list games/a.txt"), ok_with(b"hello"));
}

#[test]
fn suspend_hangs_up_a_running_command() {
    let (dir, mut guest) = games("suspend");
    dir.file("a.txt", b"hello");
    guest.open(CH);
    guest.write(CH, b"dw server list games/a.txt\r");
    // Start the command; its host job is queued but has not run.
    guest.server.poll_host();
    guest.server.suspend_host();
    guest.server.resume_host();
    assert_eq!(guest.read_until_hangup(CH), Some(Vec::new()));
}

#[test]
fn commands_on_two_channels_run_side_by_side() {
    let (dir, mut guest) = games("two-channels");
    dir.file("one.txt", &[b'1'; CHANNEL_BUFFER_BYTES + 10]);
    dir.file("two.txt", b"two");
    guest.open(CH);
    guest.open(OTHER_CH);
    guest.write(CH, b"dw server list games/one.txt\r");
    guest.write(OTHER_CH, b"dw server list games/two.txt\r");
    assert_eq!(guest.read_until_hangup(OTHER_CH), Some(ok_with(b"two")));
    assert_eq!(
        guest.read_until_hangup(CH),
        Some(ok_with(&[b'1'; CHANNEL_BUFFER_BYTES + 10]))
    );
}

#[test]
fn two_vms_keep_their_own_shares_and_directories() {
    let left = ScratchDir::new("vm-left");
    let right = ScratchDir::new("vm-right");
    left.file("only-left.txt", b"L");
    right.file("only-right.txt", b"R");
    let mut vm_a = Guest::sharing(&[("games", left.path(), ShareAccess::ReadOnly)]);
    let mut vm_b = Guest::sharing(&[("games", right.path(), ShareAccess::ReadOnly)]);
    let mut vm_c = Guest::new();
    // Interleave the two VMs' sessions on the same channel number.
    vm_a.open(CH);
    vm_b.open(CH);
    vm_a.write(CH, b"dw server dir games\r");
    vm_b.write(CH, b"dw server dir games\r");
    let b = vm_b.read_until_hangup(CH).unwrap();
    let a = vm_a.read_until_hangup(CH).unwrap();
    assert_eq!(a, ok_with(b"Directory of /games\r\n\nonly-left.txt\r\n"));
    assert_eq!(b, ok_with(b"Directory of /games\r\n\nonly-right.txt\r\n"));
    assert_eq!(
        vm_c.run_text("dw server list games/only-left.txt"),
        "FAIL 201 games/only-left.txt: no such share\n\r"
    );
}
