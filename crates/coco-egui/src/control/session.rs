//! MCP sessions (`Mcp-Session-Id`), shared by every connection the listener
//! accepts. A session expires after [`CONTROL_SESSION_IDLE_TIMEOUT`] with no
//! request in flight; a long `tools/call` (an `enter_basic` listing can type
//! for minutes) holds it open through an [`ActiveSession`].

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use super::mcp::ProtocolVersion;
use super::{CONTROL_SESSION_IDLE_TIMEOUT, MAX_CONTROL_SESSIONS};

/// Hex digits `generate_session_id` produces — 64 bits of a counter plus 64
/// bits of wall-clock nanoseconds, so distinct only across connections, not
/// cryptographically unguessable; the `Origin` check is the real defense.
const SESSION_ID_HEX_CHARS: usize = 32;

/// MCP sessions, each with its negotiated protocol version.
#[derive(Default)]
pub(super) struct SessionStore {
    sessions: HashMap<String, Session>,
}

struct Session {
    /// When the last request started, or the last in-flight one finished.
    last_used: Instant,
    protocol_version: ProtocolVersion,
    /// Requests started but not yet answered; a session with any never
    /// expires.
    in_flight: usize,
}

impl SessionStore {
    pub(super) fn create(
        &mut self,
        now: Instant,
        protocol_version: ProtocolVersion,
    ) -> Option<String> {
        self.expire_idle(now);
        if self.sessions.len() >= MAX_CONTROL_SESSIONS {
            return None;
        }
        let id = generate_session_id();
        self.sessions.insert(
            id.clone(),
            Session {
                last_used: now,
                protocol_version,
                in_flight: 0,
            },
        );
        Some(id)
    }

    pub(super) fn remove(&mut self, id: &str) {
        self.sessions.remove(id);
    }

    /// Look up `id` for a request starting at `now` and count it in flight
    /// until [`Self::finish`].
    pub(super) fn start(&mut self, id: &str, now: Instant) -> Option<ProtocolVersion> {
        self.expire_idle(now);
        let session = self.sessions.get_mut(id)?;
        session.last_used = now;
        session.in_flight += 1;
        Some(session.protocol_version)
    }

    /// The request [`Self::start`] counted was answered at `now`; the idle
    /// timeout runs from here. A no-op if the session was deleted meanwhile.
    pub(super) fn finish(&mut self, id: &str, now: Instant) {
        if let Some(session) = self.sessions.get_mut(id) {
            session.in_flight = session.in_flight.saturating_sub(1);
            session.last_used = now;
        }
    }

    fn expire_idle(&mut self, now: Instant) {
        self.sessions.retain(|_, session| {
            session.in_flight > 0
                || now.saturating_duration_since(session.last_used) < CONTROL_SESSION_IDLE_TIMEOUT
        });
    }
}

/// A request's hold on its session from [`ActiveSession::start`] until it
/// drops, after the response is written: the session can't expire meanwhile.
pub(super) struct ActiveSession<'a> {
    sessions: &'a Mutex<SessionStore>,
    id: String,
}

impl<'a> ActiveSession<'a> {
    /// [`SessionStore::start`] under the lock; `None` for an unknown or
    /// expired `id`.
    pub(super) fn start(
        sessions: &'a Mutex<SessionStore>,
        id: &str,
        now: Instant,
    ) -> Option<(ProtocolVersion, Self)> {
        let version = sessions
            .lock()
            .expect("sessions mutex poisoned")
            .start(id, now)?;
        let active = Self {
            sessions,
            id: id.to_string(),
        };
        Some((version, active))
    }
}

impl Drop for ActiveSession<'_> {
    fn drop(&mut self) {
        // A poisoned lock means another connection thread already panicked;
        // panicking again here could abort mid-unwind.
        if let Ok(mut store) = self.sessions.lock() {
            store.finish(&self.id, Instant::now());
        }
    }
}

fn generate_session_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    // Truncated to 64 bits: still ~16 significant hex digits of wall-clock
    // nanoseconds, plenty to keep this side of the id from repeating.
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    let id = format!("{counter:016x}{nanos:016x}");
    debug_assert_eq!(id.len(), SESSION_ID_HEX_CHARS);
    id
}

#[cfg(test)]
#[path = "session_test.rs"]
mod tests;
