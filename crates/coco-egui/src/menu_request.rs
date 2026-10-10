//! A request raised from outside the UI, such as the macOS application
//! menu (`macos_menu.rs`), for the manager to act on at its next update.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// One pending menu action: About, or Check for Updates.
#[derive(Clone, Default)]
pub(crate) struct MenuRequest(Arc<AtomicBool>);

impl MenuRequest {
    /// Ask for the action; the manager performs it on its next update.
    #[cfg(any(target_os = "macos", test))]
    pub(crate) fn raise(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    /// Whether a request was pending; clears it.
    pub(crate) fn take(&self) -> bool {
        self.0.swap(false, Ordering::Relaxed)
    }
}
