//! The control-protocol connection a tool call goes out over.

use coco_control::{ControlClient, ControlError, Reply, Request};

/// Sends one control-protocol request and waits for its reply. Exists so
/// `tools::call` can be tested against a mock instead of a real app.
pub trait Backend {
    fn call(&mut self, req: &Request) -> Result<Reply, ControlError>;
}

/// A [`Backend`] over the real control port. Connects lazily on first use
/// and reconnects once on I/O failure — the app may not have been started
/// yet, or may have been restarted since the last call.
pub struct AppBackend {
    port: u16,
    client: Option<ControlClient>,
}

impl AppBackend {
    pub fn new(port: u16) -> Self {
        Self { port, client: None }
    }

    fn connected(&mut self) -> Result<&mut ControlClient, ControlError> {
        if self.client.is_none() {
            self.client = Some(ControlClient::connect(self.port)?);
        }
        Ok(self.client.as_mut().expect("just set"))
    }
}

impl Backend for AppBackend {
    fn call(&mut self, req: &Request) -> Result<Reply, ControlError> {
        match self.connected()?.call(req) {
            Err(ControlError::Io(_) | ControlError::Disconnected) => {
                // The connection may be stale (app restarted, etc.); drop it
                // and try exactly once more on a fresh one.
                self.client = None;
                self.connected()?.call(req)
            }
            // The app may still answer this request later; a fresh connection
            // keeps that late reply from being read as the next call's.
            Err(ControlError::Timeout) => {
                self.client = None;
                Err(ControlError::Timeout)
            }
            other => other,
        }
    }
}

/// Message for a tool result when the control connection can't be made at
/// all — as opposed to the app answering with an error, which is
/// [`ControlError::Remote`] and carries its own message.
pub fn unreachable_message(port: u16) -> String {
    format!(
        "cocovm isn't reachable on 127.0.0.1:{port}. Start the cocovm app (it listens on this \
         control port by default; override with --control-port / COCOVM_CONTROL_PORT on both sides)."
    )
}

#[cfg(test)]
pub(crate) struct MockBackend {
    pub(crate) responses: std::collections::VecDeque<Result<Reply, ControlError>>,
    pub(crate) calls: Vec<Request>,
}

#[cfg(test)]
impl MockBackend {
    pub(crate) fn new(responses: Vec<Result<Reply, ControlError>>) -> Self {
        Self {
            responses: responses.into(),
            calls: Vec::new(),
        }
    }
}

#[cfg(test)]
impl Backend for MockBackend {
    fn call(&mut self, req: &Request) -> Result<Reply, ControlError> {
        self.calls.push(req.clone());
        self.responses
            .pop_front()
            .unwrap_or(Err(ControlError::Remote("mock exhausted".into())))
    }
}

#[cfg(test)]
#[path = "backend_test.rs"]
mod tests;
