//! The cassette deck: mounting, creating, rewinding and ejecting tapes,
//! and writing recordings back as `.cas` (and optionally `.wav`).

use crate::*;

impl CocoApp {
    /// Mounts the tape at `path` (.cas or .wav, sniffed by the `RIFF` magic, not the
    /// extension), writing back the old tape first. Aborts on a failed `.cas` write-back; a `.wav`-only failure reports but proceeds — see [`Self::save_tape_bytes`].
    pub(crate) fn insert_tape(&mut self, path: PathBuf) {
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) => {
                self.cart_error = Some(format!("could not read {}: {e}", path.display()));
                return;
            }
        };
        let tape = if bytes.starts_with(b"RIFF") {
            match coco_core::cassette_wav::decode_wav(&bytes, self.machine.cpu_hz()) {
                Ok(tape) => tape,
                Err(e) => {
                    self.cart_error = Some(format!("could not decode {}: {e}", path.display()));
                    return;
                }
            }
        } else {
            bytes
        };
        if let Err(e) = self.write_back_tape() {
            self.cart_error = Some(e);
            if self.machine.bus.cassette.dirty() {
                return;
            }
        }
        self.machine.bus.cassette.insert_tape(tape);
        self.tape_path = Some(path);
    }

    /// Creates a brand-new blank tape at `path` and mounts it, ready for CSAVE. Refuses to
    /// overwrite an existing file. Aborts on a failed `.cas` write-back of the old tape; a `.wav`-only failure reports but proceeds.
    pub(crate) fn new_tape(&mut self, path: PathBuf) {
        if let Err(e) = self.write_back_tape() {
            self.cart_error = Some(e);
            if self.machine.bus.cassette.dirty() {
                return;
            }
        }
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(_) => {
                self.machine.bus.cassette.insert_tape(Vec::new());
                self.tape_path = Some(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                self.cart_error = Some(format!(
                    "{} already exists; use Insert Tape to mount an existing tape, or \
                     choose a different name",
                    path.display()
                ));
            }
            Err(e) => {
                self.cart_error = Some(format!("could not create {}: {e}", path.display()));
            }
        }
    }

    /// Ejects the tape, saving an unfinished recording back to its file first. Aborts on a
    /// failed `.cas` write-back, leaving the tape mounted and dirty; a `.wav`-only failure reports but still ejects.
    pub(crate) fn eject_tape(&mut self) {
        if let Err(e) = self.write_back_tape() {
            self.cart_error = Some(e);
            if self.machine.bus.cassette.dirty() {
                return;
            }
        }
        self.machine.bus.cassette.eject_tape();
        self.tape_path = None;
    }

    /// Finalizes any pending recording and saves it back ([`Self::save_tape_bytes`]) — for
    /// callers ending a capture (eject, rewind, seek). The per-frame auto-save hook calls `save_tape_bytes` directly since finalize already ran.
    pub(crate) fn write_back_tape(&mut self) -> Result<(), String> {
        self.machine.bus.cassette.finalize_recording();
        self.save_tape_bytes()
    }

    /// Saves the mounted tape to disk if dirty; the canonical save is always `.cas`
    /// (extension forced, `tape_path` updated to match). If [`Self::save_tape_wav`] is on, a `.wav` sibling is also written — its failure isn't retried since the `.cas` already landed.
    pub(crate) fn save_tape_bytes(&mut self) -> Result<(), String> {
        let Some(path) = self.tape_path.clone() else {
            return Ok(());
        };
        if !self.machine.bus.cassette.dirty() {
            return Ok(());
        }
        let cas_path = path.with_extension("cas");
        std::fs::write(&cas_path, self.machine.bus.cassette.tape_bytes())
            .map_err(|e| format!("could not save {}: {e}", cas_path.display()))?;
        self.machine.bus.cassette.mark_saved();
        if cas_path != path {
            self.tape_path = Some(cas_path.clone());
        }
        if self.save_tape_wav {
            let wav_path = cas_path.with_extension("wav");
            let wav = coco_core::cassette_wav::synthesize_wav(
                self.machine.bus.cassette.tape_bytes(),
                self.machine.cpu_hz(),
            );
            std::fs::write(&wav_path, wav)
                .map_err(|e| format!("could not save {}: {e}", wav_path.display()))?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "tape_test.rs"]
mod tests;
