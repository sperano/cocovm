//! Bit-banger print capture to a host file, and the paper window.

use crate::*;

impl CocoApp {
    /// Start "print to text file" capture at `path` (create/truncate —
    /// [`coco_core::bitbanger::BitBanger::start_file_capture`]). Failures
    /// (e.g. an unwritable path) land in [`Self::cart_error`] and leave any
    /// previous capture running.
    ///
    /// Symmetric with [`Self::toggle_paper_window`]: if the paper window
    /// currently owns the bit-banger's sink, starting file capture yanks it
    /// out from under the window, so the window is detached (and closed)
    /// rather than left showing stale content.
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

    /// View-menu "Printer Paper" checkbox handler: on closed->open,
    /// attaches a DMP-105 to the bit-banger if the paper window doesn't
    /// already have a live handle (stopping any active print-file-capture
    /// first, since only one sink is live at a time). Closing just hides
    /// the window — the handle stays attached so it keeps accumulating
    /// output in the background (see `paper_view`'s module doc comment).
    pub(crate) fn toggle_paper_window(&mut self) {
        if self.paper_window.open {
            self.paper_window.open = false;
            return;
        }
        if self.paper_window.handle.is_none() {
            if self.print_capture_path.is_some() {
                self.stop_print_capture();
            }
            self.paper_window.handle = Some(self.machine.bus.bitbanger.start_dmp105());
        }
        self.paper_window.open = true;
    }
}
