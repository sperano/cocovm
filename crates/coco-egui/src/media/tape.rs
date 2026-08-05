//! The cassette deck: mounting, creating, rewinding and ejecting tapes,
//! and writing recordings back as `.cas` (and optionally `.wav`).

use crate::*;

impl CocoApp {
    /// Mount the tape at `path` (.cas decoded bytes, or a .wav recording
    /// demodulated via [`coco_core::cassette_wav::decode_wav`] — sniffed by
    /// the `RIFF` magic on the loaded bytes, not the file extension, since a
    /// picked file's extension isn't authoritative), writing back whatever
    /// was in the deck first. Failures land in [`Self::cart_error`] and
    /// leave the currently mounted tape untouched.
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
        self.write_back_tape();
        self.machine.bus.cassette.insert_tape(tape);
        self.tape_path = Some(path);
    }

    /// Create a brand-new blank tape at `path` and mount it, ready for CSAVE.
    /// Refuses to overwrite an existing file (mirrors [`Self::new_blank_disk`]).
    pub(crate) fn new_tape(&mut self, path: PathBuf) {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(_) => {
                self.write_back_tape();
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

    /// Eject the tape, saving an unfinished recording back to its file first.
    pub(crate) fn eject_tape(&mut self) {
        self.write_back_tape();
        self.machine.bus.cassette.eject_tape();
        self.tape_path = None;
    }

    /// Finalize any pending recording and save it back
    /// ([`Self::save_tape_bytes`]) — for callers that are themselves ending a
    /// capture (eject, rewind, seek). The per-frame auto-save hook
    /// (`step_emulation`) calls [`Self::save_tape_bytes`] directly instead,
    /// since by the time it runs the core's own idle auto-finalize has
    /// already landed the recording (see that hook's doc comment).
    pub(crate) fn write_back_tape(&mut self) {
        self.machine.bus.cassette.finalize_recording();
        self.save_tape_bytes();
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
    /// written alongside it, next to (not instead of) the `.cas`.
    pub(crate) fn save_tape_bytes(&mut self) {
        let Some(path) = self.tape_path.clone() else {
            return;
        };
        if !self.machine.bus.cassette.dirty() {
            return;
        }
        let cas_path = path.with_extension("cas");
        match std::fs::write(&cas_path, self.machine.bus.cassette.tape_bytes()) {
            Ok(()) => {
                self.machine.bus.cassette.mark_saved();
                if cas_path != path {
                    self.tape_path = Some(cas_path.clone());
                }
            }
            Err(e) => {
                self.cart_error = Some(format!("could not save {}: {e}", cas_path.display()));
                return;
            }
        }
        if self.save_tape_wav {
            let wav_path = cas_path.with_extension("wav");
            let wav = coco_core::cassette_wav::synthesize_wav(
                self.machine.bus.cassette.tape_bytes(),
                self.machine.cpu_hz(),
            );
            if let Err(e) = std::fs::write(&wav_path, wav) {
                self.cart_error = Some(format!("could not save {}: {e}", wav_path.display()));
            }
        }
    }
}
