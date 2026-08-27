/// Which host backend the Deluxe RS-232 pak's serial line is plugged into
/// (status bar; the live endpoint object lives inside the core's
/// [`coco_core::rs232::DeluxeRS232`]).
pub(crate) enum RS232Endpoint {
    /// TX loops straight back to RX — the pak's inert power-on default.
    Loopback,
    /// TCP listener at this address; a host terminal connects with
    /// `nc`/`telnet`.
    TCP(String),
    /// Unix pseudo-terminal; the string is the slave device path a host
    /// terminal program opens (e.g. `screen /dev/ttys009 9600`). No PTYs on
    /// Windows, so the variant only exists on Unix.
    #[cfg(unix)]
    PTY(String),
}

impl RS232Endpoint {
    /// Short status-bar description of where the wire goes.
    pub(crate) fn label(&self) -> String {
        match self {
            RS232Endpoint::Loopback => "loopback".to_string(),
            RS232Endpoint::TCP(addr) => format!("tcp {addr}"),
            #[cfg(unix)]
            RS232Endpoint::PTY(path) => format!("pty {path}"),
        }
    }
}

/// Default listen address for the RS-232 pak's TCP endpoint: localhost, port
/// 6551 after the ACIA part number.
pub(crate) const RS232_TCP_DEFAULT_ADDR: &str = "127.0.0.1:6551";

/// Launch-time request handed to [`crate::CocoApp::rs232_set_endpoint`] — the
/// *request* (bind parameters live in the app state), as opposed to
/// [`RS232Endpoint`], the record of what's actually bound. Loopback needs no
/// request of its own: it's [`crate::CocoApp::insert_rs232`]'s own default.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum RS232EndpointKind {
    TCP,
    #[cfg(unix)]
    PTY,
}
