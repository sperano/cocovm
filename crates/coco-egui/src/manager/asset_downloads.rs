//! The downloads behind the first-run asset dialog (`manager::assets`) and
//! the New Machine dialog's NitrOS-9 EOU entry (`manager::eou_menu`): one
//! background thread per file set ([`Source`]), drawn as a progress bar,
//! labelled where the surrounding text does not already name the files.

use std::sync::{Arc, mpsc};
use std::time::Duration;

use eframe::egui;

use crate::download::Progress;

/// Bytes per megabyte in the progress bars' text (decimal, like the size a
/// browser or file manager shows for the download).
const BYTES_PER_MB: f64 = 1_000_000.0;

/// How often a progress bar redraws while its download runs.
pub(crate) const PROGRESS_REPAINT_INTERVAL: Duration = Duration::from_millis(100);

/// One set of files the manager downloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Source {
    /// The app's asset bundle (`--assets-url`), from the asset dialog: ROMs,
    /// cartridges, and the welcome images.
    Bundle,
    /// The NitrOS-9 EOU images the NitrOS-9 EOU template copies
    /// (`nitros9.rs`), from the New Machine dialog.
    Eou,
}

impl Source {
    /// What the files are called: the label [`Download::draw`] puts above
    /// the bar, and the prefix of the asset dialog's failure details.
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Bundle => "ROMs, cartridges, and images",
            Self::Eou => crate::nitros9::EOU_TITLE,
        }
    }
}

/// Where one download stands.
enum State {
    /// The thread is still working; its result arrives on the channel.
    Running(mpsc::Receiver<Result<(), String>>),
    Done,
    Failed(String),
}

/// One source's download: its thread's progress and outcome.
pub(crate) struct Download {
    source: Source,
    progress: Arc<Progress>,
    state: State,
}

impl Download {
    /// Run `fetch`, which downloads `source` reporting into the
    /// [`Progress`] it is given, on a new thread that repaints `ctx` when it
    /// ends.
    pub(crate) fn start(
        source: Source,
        ctx: &egui::Context,
        fetch: impl FnOnce(Arc<Progress>) -> Result<(), String> + Send + 'static,
    ) -> Self {
        let progress = Arc::new(Progress::default());
        let (tx, rx) = mpsc::channel();
        let thread_progress = Arc::clone(&progress);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(fetch(thread_progress));
            ctx.request_repaint();
        });
        Self::running(source, progress, rx)
    }

    /// A download whose result arrives on `rx`, reporting into `progress`.
    pub(crate) fn running(
        source: Source,
        progress: Arc<Progress>,
        rx: mpsc::Receiver<Result<(), String>>,
    ) -> Self {
        Self {
            source,
            progress,
            state: State::Running(rx),
        }
    }

    pub(crate) fn is_running(&self) -> bool {
        matches!(self.state, State::Running(_))
    }

    /// Why the download failed, once it has.
    pub(crate) fn error(&self) -> Option<&str> {
        match &self.state {
            State::Failed(error) => Some(error),
            State::Running(_) | State::Done => None,
        }
    }

    /// Take the thread's result, if it has sent one.
    pub(crate) fn poll(&mut self) {
        let State::Running(rx) = &self.state else {
            return;
        };
        self.state = match rx.try_recv() {
            Ok(Ok(())) => State::Done,
            Ok(Err(error)) => State::Failed(error),
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                State::Failed("download thread exited unexpectedly".to_string())
            }
        };
    }

    /// The source's label above a full-width progress bar.
    pub(crate) fn draw(&self, ui: &mut egui::Ui) {
        ui.label(self.source.label());
        self.draw_bar(ui);
    }

    /// The full-width progress bar alone, for a dialog whose own text
    /// already names what is downloading.
    pub(crate) fn draw_bar(&self, ui: &mut egui::Ui) {
        let fraction = self.progress.fraction();
        let bar = match &self.state {
            State::Running(_) => match fraction {
                Some(fraction) => egui::ProgressBar::new(fraction).text(self.size_text()),
                None => egui::ProgressBar::new(0.0)
                    .animate(true)
                    .text(megabytes(self.progress.received())),
            },
            State::Done => egui::ProgressBar::new(1.0).text("Done"),
            State::Failed(_) => egui::ProgressBar::new(fraction.unwrap_or(0.0)).text("Failed"),
        };
        ui.add(bar.desired_width(ui.available_width()));
    }

    /// "12.3 MB of 58.2 MB", or just the received size while the total is
    /// unknown.
    fn size_text(&self) -> String {
        let received = megabytes(self.progress.received());
        match self.progress.total() {
            Some(total) => format!("{received} of {}", megabytes(total)),
            None => received,
        }
    }
}

/// `bytes` as megabytes with one decimal: "58.2 MB".
pub(crate) fn megabytes(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / BYTES_PER_MB)
}

#[cfg(test)]
#[path = "asset_downloads_test.rs"]
mod tests;
