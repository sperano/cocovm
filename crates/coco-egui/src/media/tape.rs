//! The cassette deck: mounting, creating, rewinding and ejecting tapes,
//! and writing recordings back as `.cas` (and optionally `.wav`).

use crate::*;

impl CocoApp {
    /// Mount the tape at `path` (.cas decoded bytes, or a .wav recording
    /// demodulated via [`coco_core::cassette_wav::decode_wav`] — sniffed by
    /// the `RIFF` magic on the loaded bytes, not the file extension, since a
    /// picked file's extension isn't authoritative), writing back whatever
    /// was in the deck first. A write-back failure that leaves the old tape
    /// still dirty (the canonical `.cas` never landed) aborts the mount,
    /// preserving the old tape, its dirty flag, and its tracked path for a
    /// later retry; a failure that hits only the optional `.wav` sibling
    /// (the `.cas` already landed and the tape is clean — see
    /// [`Self::save_tape_bytes`]) reports through [`Self::cart_error`] but
    /// proceeds, since there is nothing left to retry.
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

    /// Create a brand-new blank tape at `path` and mount it, ready for CSAVE.
    /// Refuses to overwrite an existing file (mirrors [`Self::new_blank_disk`]).
    /// Whatever was in the deck first is flushed before the new file is even
    /// created. A write-back failure that leaves the old tape still dirty
    /// aborts the whole operation, leaving neither a stray empty file on
    /// disk nor the old tape disturbed; a failure that hits only the
    /// optional `.wav` sibling (the tape is already clean — see
    /// [`Self::save_tape_bytes`]) reports through [`Self::cart_error`] but
    /// proceeds, since there is nothing left to retry.
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

    /// Eject the tape, saving an unfinished recording back to its file
    /// first. A write-back failure that leaves the tape still dirty (the
    /// canonical `.cas` never landed) aborts the eject — the tape stays
    /// mounted, dirty, and tracked at its path, and the error lands in
    /// [`Self::cart_error`] so a later retry can succeed; a failure that
    /// hits only the optional `.wav` sibling (the tape is already clean —
    /// see [`Self::save_tape_bytes`]) reports through [`Self::cart_error`]
    /// but still ejects, since there is nothing left to retry.
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

    /// Finalize any pending recording and save it back
    /// ([`Self::save_tape_bytes`]) — for callers that are themselves ending a
    /// capture (eject, rewind, seek). The per-frame auto-save hook
    /// (`step_emulation`) calls [`Self::save_tape_bytes`] directly instead,
    /// since by the time it runs the core's own idle auto-finalize has
    /// already landed the recording (see that hook's doc comment).
    pub(crate) fn write_back_tape(&mut self) -> Result<(), String> {
        self.machine.bus.cassette.finalize_recording();
        self.save_tape_bytes()
    }

    /// Save the mounted tape back to disk if it changed (like
    /// [`Self::write_back_disk`]; on failure the tape stays mounted and
    /// dirty so a later retry can succeed) — the save half of
    /// [`Self::write_back_tape`], split out so the per-frame auto-save hook
    /// can save a recording the core already finalized without also
    /// re-finalizing (which would fold any new in-flight capture into it).
    ///
    /// The canonical save is always a `.cas` — `tape_path` with its
    /// extension forced to `.cas` (a no-op if it already was one, e.g. a
    /// tape mounted from `.cas` to begin with; `foo.wav` becomes `foo.cas`).
    /// On success, `tape_path` is updated to that `.cas` path so a tape
    /// originally mounted from a `.wav` is never silently overwritten
    /// again — from then on the app tracks the `.cas` sibling. When
    /// [`Self::save_tape_wav`] is on, a `.wav` of the tape audio
    /// ([`coco_core::cassette_wav::synthesize_wav`]) is additionally
    /// written alongside it, next to (not instead of) the `.cas` — a
    /// failure on that second write is also an `Err`, but unlike the `.cas`
    /// half it is NOT retried by a later flush: `mark_saved` has already
    /// run (the `.cas`, the canonical data, did land and is not rolled
    /// back), so the next flush of a still-clean tape is a no-op and the
    /// `.wav` stays missing until the tape dirties again.
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
