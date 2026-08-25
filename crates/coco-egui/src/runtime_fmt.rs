//! Humanizing a whole-seconds runtime total — shared by the detail pane's
//! Statistics block (`manager::detail`'s `draw_statistics`) and the VM
//! window's status bar (`chrome::status_bar`'s `runtime_status`), the two
//! places a cumulative powered-on runtime is shown to the user.

/// [`humanize_runtime`]'s unit breakpoints.
const SECS_PER_MINUTE: u64 = 60;
const SECS_PER_HOUR: u64 = 60 * SECS_PER_MINUTE;
const SECS_PER_DAY: u64 = 24 * SECS_PER_HOUR;

/// Humanize a whole-seconds duration as its two largest nonzero units —
/// `"2 d 3 h"`, `"3 h 12 m"`, `"12 m 5 s"` — or a single unit under a minute (`"42 s"`).
pub(crate) fn humanize_runtime(total_secs: u64) -> String {
    if total_secs >= SECS_PER_DAY {
        let days = total_secs / SECS_PER_DAY;
        let hours = (total_secs % SECS_PER_DAY) / SECS_PER_HOUR;
        format!("{days} d {hours} h")
    } else if total_secs >= SECS_PER_HOUR {
        let hours = total_secs / SECS_PER_HOUR;
        let minutes = (total_secs % SECS_PER_HOUR) / SECS_PER_MINUTE;
        format!("{hours} h {minutes} m")
    } else if total_secs >= SECS_PER_MINUTE {
        let minutes = total_secs / SECS_PER_MINUTE;
        let seconds = total_secs % SECS_PER_MINUTE;
        format!("{minutes} m {seconds} s")
    } else {
        format!("{total_secs} s")
    }
}

#[cfg(test)]
#[path = "runtime_fmt_test.rs"]
mod tests;
