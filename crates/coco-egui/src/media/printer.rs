//! Bit-banger print capture to a host file, and the paper window.

use crate::*;

impl CocoApp {
    /// Starts "print to text file" capture at `path` (create/truncate); failures
    /// land in [`Self::cart_error`]. Detaches the paper window first so it
    /// doesn't show stale content.
    pub(crate) fn start_print_capture(&mut self, path: PathBuf) {
        match self
            .machine
            .bus
            .bitbanger
            .start_file_capture(&path, self.print_capture_lf)
        {
            Ok(()) => {
                self.print_capture_path = Some(path);
                self.paper_window.detach();
            }
            Err(e) => self.cart_error = Some(format!("could not open {}: {e}", path.display())),
        }
    }

    /// Stop capture, restoring the bit-banger's no-op sink.
    pub(crate) fn stop_print_capture(&mut self) {
        self.machine.bus.bitbanger.stop_capture();
        self.print_capture_path = None;
    }

    /// Attaches a DMP-105 to the bit-banger, stopping any active print-file
    /// capture first — only one sink is live at a time. Does not open the paper window.
    pub(crate) fn attach_dmp105(&mut self) {
        if self.print_capture_path.is_some() {
            self.stop_print_capture();
        }
        self.paper_window.handle = Some(self.machine.bus.bitbanger.start_dmp105());
    }

    /// View-menu "Printer Paper" checkbox handler: opening attaches a DMP-105 if
    /// none is live yet; closing just hides the window — capture keeps running in the background.
    pub(crate) fn toggle_paper_window(&mut self) {
        if self.paper_window.open {
            self.paper_window.open = false;
            return;
        }
        if self.paper_window.handle.is_none() {
            self.attach_dmp105();
        }
        self.paper_window.open = true;
    }
}
