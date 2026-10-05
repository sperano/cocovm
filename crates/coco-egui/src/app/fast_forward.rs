//! Unthrottled fast-forward: running a VM's fields as fast as the host
//! allows, for an MCP `wait`/`wait_for_text` that asked for it, instead of
//! at the wall clock's pace.
//!
//! The wall-clock loop (`app/frame.rs`) credits the emulation clock from
//! elapsed host time, caps the fields per update, and feeds every field's
//! audio to the output ring. A fast-forward replaces all three for as long
//! as it lasts: each `update()` runs fields back to back for one
//! [`FAST_FORWARD_SLICE`] of host time, the audio those fields produce is
//! dropped, and the wall clock is neither consulted nor owed anything when
//! the run ends. Slicing keeps the manager and the other VM windows
//! responsive: between slices egui still draws every window, and the
//! other VMs still run their own owed fields.
//!
//! A future `run_until_break` tool needs nothing more than
//! [`CocoApp::start_fast_forward`] with a field budget: a breakpoint clears
//! `running`, which ends the run the same way it ends a wall-clock one.

use std::time::{Duration, Instant};

use crate::CocoApp;
use crate::control::TextMatcher;

use super::control::PAUSED_ERROR;

/// Host time one `update()` spends running fast-forwarded fields before it
/// returns to egui. Long enough that a slice's fields dwarf the manager's
/// per-update overhead, short enough that the manager window, the VM
/// window's chrome, and the other running VMs still update several times
/// a second.
pub(crate) const FAST_FORWARD_SLICE: Duration = Duration::from_millis(50);

/// Why a second fast-forward is refused while one is in progress: the two
/// requests would disagree about when real-time pacing resumes.
pub(crate) const FAST_FORWARD_BUSY_ERROR: &str = "a fast-forward is already in progress";

/// An unthrottled run in progress on a [`CocoApp`].
pub(crate) struct FastForward {
    /// The `CocoApp::fields_run` count that ends the run.
    target_field: u64,
    /// `wait_for_text`'s pattern: the run also ends, before the target, on
    /// the first field after which the decoded screen matches. Checked once
    /// per field so the reply's screen is the one that matched, not one a
    /// slice's worth of fields later.
    until_text: Option<TextMatcher>,
}

/// Why a fast-forward slice stopped running fields.
enum SliceEnd {
    /// The run reached its target field or its screen text.
    Finished,
    /// The VM stopped advancing (a breakpoint tripped during the slice).
    Paused,
    /// The slice's host-time budget ran out with the run still going.
    Budget,
}

impl CocoApp {
    /// Begin running unthrottled until [`Self::fields_run`] reaches
    /// `target_field`, or until the screen matches `until_text`. A paused
    /// VM is refused, as is one already fast-forwarding. A target at or
    /// below the current count starts nothing: the caller's wait is
    /// already satisfied.
    pub(crate) fn start_fast_forward(
        &mut self,
        target_field: u64,
        until_text: Option<TextMatcher>,
    ) -> Result<(), String> {
        if !self.running {
            return Err(PAUSED_ERROR.to_string());
        }
        if self.fast_forward.is_some() {
            return Err(FAST_FORWARD_BUSY_ERROR.to_string());
        }
        if target_field <= self.fields_run {
            return Ok(());
        }
        self.fast_forward = Some(FastForward {
            target_field,
            until_text,
        });
        Ok(())
    }

    /// End an unthrottled run early, whatever its progress. The emulation
    /// clock restarts from scratch, as after a pause, so the wall clock is
    /// owed nothing for the host time the run took. A no-op when no run is
    /// in progress.
    pub(crate) fn stop_fast_forward(&mut self) {
        if self.fast_forward.take().is_some() {
            self.reset_emulation_clock();
        }
    }

    pub(crate) fn is_fast_forwarding(&self) -> bool {
        self.fast_forward.is_some()
    }

    /// Run one slice of the fast-forward in progress: fields back to back
    /// for [`FAST_FORWARD_SLICE`] of host time, or until the run ends.
    /// Returns whether a run is still in progress afterwards. Credits
    /// [`Self::total_runtime`] with the emulated time, not the host time,
    /// since the machine was powered on for that long by its own clock.
    pub(super) fn run_fast_forward_slice(&mut self) -> bool {
        self.run_fast_forward_slice_until(Instant::now() + FAST_FORWARD_SLICE)
    }

    fn run_fast_forward_slice_until(&mut self, deadline: Instant) -> bool {
        let before = self.fields_run;
        let end = self.run_fast_forward_fields(deadline);
        let fields = self.fields_run - before;
        let field_rate_hz = self.machine.config.video.field_rate_hz();
        self.total_runtime += Duration::from_secs_f64(fields as f64 / field_rate_hz);
        match end {
            SliceEnd::Budget => true,
            SliceEnd::Finished | SliceEnd::Paused => {
                self.stop_fast_forward();
                false
            }
        }
    }

    /// The field loop of [`Self::run_fast_forward_slice`]: one field at a
    /// time, with the screen-text check after each.
    fn run_fast_forward_fields(&mut self, deadline: Instant) -> SliceEnd {
        loop {
            let Some(run) = self.fast_forward.as_ref() else {
                return SliceEnd::Finished;
            };
            let target_field = run.target_field;
            self.run_fields(1);
            if !self.running {
                return SliceEnd::Paused;
            }
            if self.fields_run >= target_field || self.fast_forward_text_matches() {
                return SliceEnd::Finished;
            }
            if Instant::now() >= deadline {
                return SliceEnd::Budget;
            }
        }
    }

    /// Whether the run's `until_text` matches the screen now. Decodes the
    /// screen only when a pattern is set; a plain `wait` pays nothing.
    fn fast_forward_text_matches(&mut self) -> bool {
        let has_pattern = self
            .fast_forward
            .as_ref()
            .is_some_and(|run| run.until_text.is_some());
        if !has_pattern {
            return false;
        }
        let snapshot = self.screen_snapshot();
        self.fast_forward
            .as_ref()
            .and_then(|run| run.until_text.as_ref())
            .is_some_and(|matcher| matcher.is_match(&snapshot))
    }
}

#[cfg(test)]
#[path = "fast_forward_test.rs"]
mod tests;
