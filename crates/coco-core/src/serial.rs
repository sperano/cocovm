//! Host-side serial wire backends for the Deluxe RS-232 Program Pak
//! "Host serial backend": the
//! [`SerialEndpoint`] seam the 6551 ACIA transmits into and receives from,
//! with TCP / Unix PTY / loopback implementations. Kept in `coco-core`
//! (std-only, no host-audio-style deps); the egui frontend chooses which
//! endpoint to plug in.
//!
//! Same shape as [`crate::bitbanger::PrinterSink`]: a minimal trait the CPU
//! side drives every poll, with the host-facing plumbing (sockets, ptys)
//! kept out of the ACIA core (`acia6551.rs`) entirely. All three
//! implementations are non-blocking end to end — the ACIA is polled from the
//! CPU loop, so a backend that could block would stall emulation.
//!
//! [`Loopback`] is the test/CI endpoint and the acceptance-test seam
//! ("Testing / acceptance": "byte written to
//! `$FF68` reappears at `$FF68`"). [`TCPEndpoint`] and [`PTYEndpoint`] are
//! the real host-facing backends the egui frontend offers.

use std::collections::VecDeque;
use std::io::{self, ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};

/// The wire a 6551 ACIA's TX/RX shift registers talk to, from the host side.
/// Deliberately minimal — matches [`crate::bitbanger::PrinterSink`]'s
/// "seam, not a protocol" shape (`bitbanger.rs`'s module doc comment).
pub trait SerialEndpoint {
    /// Next byte from the host side, if one is available. Non-blocking:
    /// `None` means "nothing waiting right now", never an error.
    fn poll_rx(&mut self) -> Option<u8>;
    /// Transmit one byte to the host side. Non-blocking / best-effort — a
    /// serial line with nothing attached just eats the byte, like real hardware with an unplugged cable.
    fn tx(&mut self, b: u8);
    /// Data Carrier Detect: is something connected on the host side?
    /// Feeds the ACIA's status register DCD bit.
    fn dcd(&self) -> bool;
}

/// Loopback endpoint: every byte handed to [`Self::tx`] comes straight back
/// out of [`Self::poll_rx`], in order. `dcd()` is always true (nothing to
/// disconnect). This is the test/CI endpoint — no host I/O at all — and the
/// acceptance-test seam for "byte written to `$FF68` reappears at `$FF68`"
/// ("Testing / acceptance").
#[derive(Debug, Default)]
pub struct Loopback {
    queue: VecDeque<u8>,
}

impl Loopback {
    pub fn new() -> Self {
        Self::default()
    }

    /// Inject a byte as if it arrived from the host side, without going
    /// through `tx` first — lets a test drive the RX path directly.
    pub fn inject(&mut self, b: u8) {
        self.queue.push_back(b);
    }

    /// Number of bytes currently queued and not yet drained by `poll_rx`.
    pub fn pending(&self) -> usize {
        self.queue.len()
    }
}

impl SerialEndpoint for Loopback {
    fn poll_rx(&mut self) -> Option<u8> {
        self.queue.pop_front()
    }

    fn tx(&mut self, b: u8) {
        self.queue.push_back(b);
    }

    fn dcd(&self) -> bool {
        true
    }
}

/// TCP endpoint: binds a listener on a caller-supplied address and serves
/// one client at a time. Every [`Self::poll_rx`]/[`Self::tx`] call first
/// tries to accept a pending connection if none is attached yet, so a host
/// terminal (`telnet`/`nc`/a real terminal emulator) can connect at any
/// point without a separate "wait for client" step. All socket I/O is
/// non-blocking (`set_nonblocking(true)` on both the listener and every
/// accepted stream); `WouldBlock` means "nothing to do right now", never
/// surfaced as an error.
pub struct TCPEndpoint {
    listener: TcpListener,
    client: Option<TcpStream>,
}

impl TCPEndpoint {
    /// Bind and start listening at `addr` (e.g. `"127.0.0.1:6551"`, or
    /// `"127.0.0.1:0"` for an OS-assigned port — see [`Self::local_addr`]).
    pub fn bind(addr: impl ToSocketAddrs) -> io::Result<Self> {
        let listener = TcpListener::bind(addr)?;
        listener.set_nonblocking(true)?;
        Ok(Self {
            listener,
            client: None,
        })
    }

    /// The address actually bound — needed to discover an OS-assigned port
    /// when binding to port 0.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// Accept a waiting connection if no client is currently attached.
    /// One client at a time: a pending connection stays in the listener's backlog until the current one goes away.
    fn try_accept(&mut self) {
        if self.client.is_some() {
            return;
        }
        match self.listener.accept() {
            Ok((stream, _addr)) => {
                // A stream must be set non-blocking explicitly; drop it if that fails.
                if stream.set_nonblocking(true).is_ok() {
                    self.client = Some(stream);
                }
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => {}
            Err(_) => {}
        }
    }
}

impl SerialEndpoint for TCPEndpoint {
    fn poll_rx(&mut self) -> Option<u8> {
        self.try_accept();
        let stream = self.client.as_mut()?;
        let mut byte = [0u8; 1];
        match stream.read(&mut byte) {
            Ok(0) => {
                // Peer closed cleanly: drop back to listening.
                self.client = None;
                None
            }
            Ok(_) => Some(byte[0]),
            Err(e) if e.kind() == ErrorKind::WouldBlock => None,
            Err(_) => {
                // Any other error is treated as a clean disconnect: drop the client and go back to listening.
                self.client = None;
                None
            }
        }
    }

    fn tx(&mut self, b: u8) {
        self.try_accept();
        let Some(stream) = self.client.as_mut() else {
            // Nothing attached: silently drop what's sent, like an unplugged cable.
            return;
        };
        match stream.write_all(&[b]) {
            Ok(()) => {}
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                // Send buffer momentarily full: drop this byte rather than block the CPU loop.
            }
            Err(_) => {
                self.client = None;
            }
        }
    }

    fn dcd(&self) -> bool {
        self.client.is_some()
    }
}

/// Unix pseudo-terminal endpoint: allocates a PTY pair via `libc`, exposes
/// the slave device path so the frontend can tell the user "connect your
/// terminal to `/dev/ttysNNN`", and reads/writes the master side
/// non-blocking. Only the master side is touched here — the slave's line
/// discipline (echo, canonical mode, etc.) is the connecting host program's
/// business, not the emulator's; bytes cross the master fd raw and
/// unmodified in both directions.
#[cfg(unix)]
pub struct PTYEndpoint {
    master_fd: libc::c_int,
    slave_path: String,
}

#[cfg(unix)]
impl PTYEndpoint {
    /// Allocate a new PTY pair via `posix_openpt`/`grantpt`/`unlockpt`, then
    /// resolve the slave's device path. The master fd is set non-blocking before returning.
    pub fn new() -> io::Result<Self> {
        // SAFETY: each libc call's return is checked before the next; fd is closed on every error path.
        unsafe {
            let master_fd = libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY);
            if master_fd < 0 {
                return Err(io::Error::last_os_error());
            }
            if libc::grantpt(master_fd) != 0 {
                let err = io::Error::last_os_error();
                libc::close(master_fd);
                return Err(err);
            }
            if libc::unlockpt(master_fd) != 0 {
                let err = io::Error::last_os_error();
                libc::close(master_fd);
                return Err(err);
            }
            let slave_path = match Self::slave_name(master_fd) {
                Ok(path) => path,
                Err(err) => {
                    libc::close(master_fd);
                    return Err(err);
                }
            };
            if let Err(err) = Self::set_nonblocking(master_fd) {
                libc::close(master_fd);
                return Err(err);
            }
            Ok(Self {
                master_fd,
                slave_path,
            })
        }
    }

    /// Resolve the slave device path via `ptsname_r` (thread-safe, Linux/glibc).
    /// Not available on macOS — see the `not(target_os = "linux")` variant below.
    #[cfg(target_os = "linux")]
    unsafe fn slave_name(master_fd: libc::c_int) -> io::Result<String> {
        // SAFETY: `master_fd` is a valid, just-opened PTY master fd; `buf` outlives the call.
        unsafe {
            // `c_char` signedness is ABI-specific, so the buffer must use the alias.
            let mut buf = [0 as libc::c_char; 128];
            if libc::ptsname_r(master_fd, buf.as_mut_ptr(), buf.len()) != 0 {
                return Err(io::Error::last_os_error());
            }
            let cstr = std::ffi::CStr::from_ptr(buf.as_ptr());
            Ok(cstr.to_string_lossy().into_owned())
        }
    }

    #[cfg(not(target_os = "linux"))]
    unsafe fn slave_name(master_fd: libc::c_int) -> io::Result<String> {
        // SAFETY: `master_fd` is a valid, just-opened PTY master fd.
        unsafe {
            let ptr = libc::ptsname(master_fd);
            if ptr.is_null() {
                return Err(io::Error::last_os_error());
            }
            let cstr = std::ffi::CStr::from_ptr(ptr);
            Ok(cstr.to_string_lossy().into_owned())
        }
    }

    /// Set `O_NONBLOCK` on `fd` via `fcntl`, preserving any other flags
    /// already set.
    unsafe fn set_nonblocking(fd: libc::c_int) -> io::Result<()> {
        // SAFETY: caller guarantees `fd` is a valid, open fd.
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFL, 0);
            if flags < 0 {
                return Err(io::Error::last_os_error());
            }
            if libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }
    }

    /// The slave device path (e.g. `/dev/ttys003`) the frontend should tell
    /// the user to connect a terminal emulator to.
    pub fn path(&self) -> &str {
        &self.slave_path
    }
}

#[cfg(unix)]
impl SerialEndpoint for PTYEndpoint {
    fn poll_rx(&mut self) -> Option<u8> {
        let mut byte = [0u8; 1];
        // SAFETY: `byte` outlives the call; `master_fd` is owned by `self` and open until `Drop`.
        let n = unsafe {
            libc::read(
                self.master_fd,
                byte.as_mut_ptr().cast::<libc::c_void>(),
                byte.len(),
            )
        };
        // n <= 0 covers both EAGAIN and any other read error — both mean "no byte right now".
        if n == 1 { Some(byte[0]) } else { None }
    }

    fn tx(&mut self, b: u8) {
        let byte = [b];
        // SAFETY: `byte` outlives the call; a full pty buffer or no reader just drops the byte.
        unsafe {
            libc::write(self.master_fd, byte.as_ptr().cast::<libc::c_void>(), 1);
        }
    }

    fn dcd(&self) -> bool {
        // No distinct "someone connected" event for a pty short of watching the slave's open count.
        true
    }
}

#[cfg(unix)]
impl Drop for PTYEndpoint {
    fn drop(&mut self) {
        // SAFETY: `master_fd` is owned exclusively by this struct and only closed here.
        unsafe {
            libc::close(self.master_fd);
        }
    }
}

#[cfg(test)]
#[path = "serial_test.rs"]
mod tests;
