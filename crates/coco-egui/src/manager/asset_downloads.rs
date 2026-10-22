//! The downloads behind the first-run asset dialog (`manager::assets`):
//! one background thread per file set ([`Source`]), all running at once,
//! each drawn as a labelled progress bar.

use std::path::Path;
use std::sync::{Arc, mpsc};
use std::time::Duration;

use eframe::egui;

use crate::download::Progress;

/// Bytes per megabyte in the progress bars' text (decimal, like the size a
/// browser or file manager shows for the download).
const BYTES_PER_MB: f64 = 1_000_000.0;

/// How often a progress bar redraws while its download runs.
pub(crate) const PROGRESS_REPAINT_INTERVAL: Duration = Duration::from_millis(100);

/// One set of files the asset dialog downloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Source {
    /// The app's asset bundle (`--assets-url`): ROMs, cartridges, and the
    /// welcome images.
    Bundle,
    /// The NitrOS-9 EOU images the NitrOS-9 EOU template copies
    /// (`nitros9.rs`).
    Eou,
}

impl Source {
    /// Every source, in the order the dialog lists them.
    pub(crate) const ALL: [Self; 2] = [Self::Bundle, Self::Eou];

    /// The progress bar's label.
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Bundle => "ROMs, cartridges, and images",
            Self::Eou => crate::nitros9::EOU_TITLE,
        }
    }

    /// Whether the manager needs this source's files to open. The EOU
    /// images only back one template, and they come from a third-party
    /// site, so the dialog never waits for them: a failed download is
    /// retried from the New Machine dialog.
    pub(crate) const fn required(self) -> bool {
        match self {
            Self::Bundle => true,
            Self::Eou => false,
        }
    }

    /// Display paths of this source's files absent on disk.
    pub(crate) fn missing(self) -> Vec<String> {
        match self {
            Self::Bundle => crate::startup::missing_assets(),
            Self::Eou => crate::startup::missing_eou_assets(),
        }
    }

    /// Download this source's files from `url` and install them under the
    /// assets directory `install_dir`.
    pub(crate) fn fetch(
        self,
        url: &str,
        install_dir: &Path,
        progress: Arc<Progress>,
    ) -> Result<(), String> {
        match self {
            Self::Bundle => crate::startup::download_and_unpack_assets(url, install_dir, progress),
            Self::Eou => crate::nitros9::download_eou(
                url,
                &install_dir.join(crate::nitros9::EOU_DIR_NAME),
                progress,
            ),
        }
        .map_err(|e| e.to_string())
    }
}

/// Where each [`Source`] downloads from.
#[derive(Debug, Clone)]
pub(crate) struct SourceUrls {
    /// `--assets-url` (`cli.rs`).
    pub(crate) bundle: String,
    /// `--eou-url` (`cli.rs`).
    pub(crate) eou: String,
}

impl SourceUrls {
    /// `source`'s URL.
    pub(crate) fn get(&self, source: Source) -> &str {
        match source {
            Source::Bundle => &self.bundle,
            Source::Eou => &self.eou,
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

    pub(crate) fn source(&self) -> Source {
        self.source
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
