use super::*;
use std::time::{Duration, Instant};

/// Bounded poll-with-retry: real socket/pty I/O crosses the kernel, so
/// delivery isn't instant even on loopback — polls `f` until `timeout`.
fn poll_until<T>(timeout: Duration, mut f: impl FnMut() -> Option<T>) -> Option<T> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(v) = f() {
            return Some(v);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

const TEST_TIMEOUT: Duration = Duration::from_secs(2);

#[test]
fn loopback_dcd_always_true() {
    let lb = Loopback::new();
    assert!(lb.dcd());
}

#[test]
fn loopback_round_trip_and_ordering() {
    let mut lb = Loopback::new();
    assert_eq!(lb.poll_rx(), None);

    lb.tx(0x41);
    lb.tx(0x42);
    lb.tx(0x43);
    assert_eq!(lb.pending(), 3);

    assert_eq!(lb.poll_rx(), Some(0x41));
    assert_eq!(lb.poll_rx(), Some(0x42));
    assert_eq!(lb.poll_rx(), Some(0x43));
    assert_eq!(lb.poll_rx(), None);
}

#[test]
fn loopback_inject_feeds_poll_rx_directly() {
    let mut lb = Loopback::new();
    lb.inject(0x55);
    assert_eq!(lb.poll_rx(), Some(0x55));
    assert_eq!(lb.poll_rx(), None);
}

#[test]
fn tcp_dcd_false_before_any_connection() {
    let mut ep = TCPEndpoint::bind("127.0.0.1:0").expect("bind");
    assert_eq!(ep.poll_rx(), None); // drives try_accept with no client waiting
    assert!(!ep.dcd());
}

#[test]
fn tcp_round_trip_dcd_and_disconnect() {
    let mut ep = TCPEndpoint::bind("127.0.0.1:0").expect("bind");
    let addr = ep.local_addr().expect("local_addr");

    let mut client = TcpStream::connect(addr).expect("connect");
    client.set_nonblocking(true).expect("client nonblocking");

    // Wait for the endpoint to accept the connection (each poll_rx drives try_accept).
    let connected = poll_until(TEST_TIMEOUT, || {
        ep.poll_rx();
        ep.dcd().then_some(())
    });
    assert!(connected.is_some(), "endpoint never saw the connection");
    assert!(ep.dcd());

    // Host -> guest.
    client.write_all(&[0x55]).expect("client write");
    let received = poll_until(TEST_TIMEOUT, || ep.poll_rx());
    assert_eq!(received, Some(0x55));

    // Guest -> host.
    ep.tx(0xAA);
    let mut buf = [0u8; 1];
    let echoed = poll_until(TEST_TIMEOUT, || match client.read(&mut buf) {
        Ok(1) => Some(buf[0]),
        _ => None,
    });
    assert_eq!(echoed, Some(0xAA));

    // Dropping the client should be observed as a read returning Ok(0), flipping dcd() false.
    drop(client);
    let disconnected = poll_until(TEST_TIMEOUT, || {
        ep.poll_rx();
        (!ep.dcd()).then_some(())
    });
    assert!(disconnected.is_some(), "endpoint never noticed disconnect");
    assert!(!ep.dcd());
}

#[test]
fn tcp_tx_with_no_client_is_silently_dropped() {
    let mut ep = TCPEndpoint::bind("127.0.0.1:0").expect("bind");
    // No client ever connects; tx must not panic or block.
    ep.tx(0x01);
    assert!(!ep.dcd());
}

#[cfg(unix)]
mod pty {
    use super::*;
    use std::fs::OpenOptions;
    use std::os::unix::fs::OpenOptionsExt;
    use std::os::unix::io::AsRawFd;

    #[test]
    fn path_is_nonempty_and_exists() {
        let pty = PTYEndpoint::new().expect("open pty");
        assert!(!pty.path().is_empty());
        assert!(std::path::Path::new(pty.path()).exists());
        assert!(pty.dcd());
    }

    /// Put `fd`'s termios into raw mode: without it, cooked-mode line
    /// buffering holds master->slave bytes until a newline, hiding them from `read`.
    fn set_raw(fd: libc::c_int) {
        // SAFETY: `fd` is a valid, open fd for the call; `term` is initialized by `tcgetattr` before use.
        unsafe {
            let mut term: libc::termios = std::mem::zeroed();
            assert_eq!(libc::tcgetattr(fd, &mut term), 0, "tcgetattr failed");
            libc::cfmakeraw(&mut term);
            assert_eq!(
                libc::tcsetattr(fd, libc::TCSANOW, &term),
                0,
                "tcsetattr failed"
            );
        }
    }

    #[test]
    fn round_trip_via_slave_device() {
        let mut pty = PTYEndpoint::new().expect("open pty");

        // Non-blocking slave: a read with nothing written yet returns EAGAIN instead of hanging.
        let mut slave = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(pty.path())
            .expect("open slave device");
        set_raw(slave.as_raw_fd());

        // Slave -> master.
        slave.write_all(&[0x5A]).expect("write to slave");
        let received = poll_until(TEST_TIMEOUT, || pty.poll_rx());
        assert_eq!(received, Some(0x5A));

        // Master -> slave.
        pty.tx(0xA5);
        let mut buf = [0u8; 1];
        let echoed = poll_until(TEST_TIMEOUT, || match slave.read(&mut buf) {
            Ok(1) => Some(buf[0]),
            _ => None,
        });
        assert_eq!(echoed, Some(0xA5));
    }
}
