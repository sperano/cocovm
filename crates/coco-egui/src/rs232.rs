/// Which host backend the Deluxe RS-232 pak's serial line is plugged into
/// (menu labels / status bar; the live endpoint object lives inside the
/// core's [`coco_core::rs232::DeluxeRs232`]).
pub(crate) enum Rs232Endpoint {
    /// TX loops straight back to RX — the pak's inert power-on default.
    Loopback,
    /// TCP listener at this address; a host terminal connects with
    /// `nc`/`telnet`.
    Tcp(String),
    /// Unix pseudo-terminal; the string is the slave device path a host
    /// terminal program opens (e.g. `screen /dev/ttys009 9600`).
    Pty(String),
}

impl Rs232Endpoint {
    /// Short status-bar/menu description of where the wire goes.
    pub(crate) fn label(&self) -> String {
        match self {
            Rs232Endpoint::Loopback => "loopback".to_string(),
            Rs232Endpoint::Tcp(addr) => format!("tcp {addr}"),
            Rs232Endpoint::Pty(path) => format!("pty {path}"),
        }
    }
}

/// Default listen address for the RS-232 pak's TCP endpoint: localhost, port
/// 6551 after the ACIA part number.
pub(crate) const RS232_TCP_DEFAULT_ADDR: &str = "127.0.0.1:6551";

/// Menu selection handed to [`CocoApp::rs232_set_endpoint`] — the *request*
/// (bind parameters live in the app state), as opposed to
/// [`Rs232Endpoint`], the record of what's actually bound.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Rs232EndpointKind {
    Loopback,
    Tcp,
    Pty,
}
