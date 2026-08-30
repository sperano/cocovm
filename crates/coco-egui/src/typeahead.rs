use crate::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KbMode {
    Positional,
    Symbolic,
}

impl KbMode {
    pub(crate) fn label(self) -> &'static str {
        match self {
            KbMode::Positional => "Positional",
            KbMode::Symbolic => "Symbolic",
        }
    }
}

/// Fields one queued tap occupies in [`TypeAhead::advance`]: the hold and gap
/// counts plus the three phase-transition fields (pop, hold→gap, gap→idle).
pub(crate) const FIELDS_PER_TAP: u64 = TYPE_HOLD_FIELDS as u64 + TYPE_GAP_FIELDS as u64 + 3;

/// Symbolic-mode type-ahead: replays queued (key, shift) taps with hold/gap timing
/// so the ROM's 60 Hz keyboard scan registers each one.
#[derive(Default)]
pub(crate) struct TypeAhead {
    pub(crate) queue: VecDeque<(Pos, bool)>,
    pub(crate) phase: TypePhase,
    pub(crate) current: (Pos, bool),
}

#[derive(Default, Clone, Copy)]
pub(crate) enum TypePhase {
    #[default]
    Idle,
    Hold(u8),
    Gap(u8),
}

impl TypeAhead {
    pub(crate) fn clear(&mut self) {
        self.queue.clear();
        self.phase = TypePhase::Idle;
    }

    /// True while taps are still queued or a tap is mid hold/gap—that is, a paste or
    /// type-ahead burst is still draining and owns the keyboard matrix.
    pub(crate) fn is_active(&self) -> bool {
        !self.queue.is_empty() || !matches!(self.phase, TypePhase::Idle)
    }

    /// Advance one field, driving the CoCo matrix for the current tap.
    pub(crate) fn advance(&mut self, kb: &mut kbd::Keyboard) {
        match self.phase {
            TypePhase::Idle => {
                if let Some(entry) = self.queue.pop_front() {
                    self.current = entry;
                    kb.set(entry.0, true);
                    if entry.1 {
                        kb.set(kbd::SHIFT, true);
                    }
                    self.phase = TypePhase::Hold(TYPE_HOLD_FIELDS);
                }
            }
            TypePhase::Hold(0) => {
                kb.set(self.current.0, false);
                kb.set(kbd::SHIFT, false);
                self.phase = TypePhase::Gap(TYPE_GAP_FIELDS);
            }
            TypePhase::Hold(n) => {
                // Re-asserted every field: a focus-loss `release_all` between
                // fields must not cut the hold short.
                kb.set(self.current.0, true);
                if self.current.1 {
                    kb.set(kbd::SHIFT, true);
                }
                self.phase = TypePhase::Hold(n - 1);
            }
            TypePhase::Gap(0) => self.phase = TypePhase::Idle,
            TypePhase::Gap(n) => self.phase = TypePhase::Gap(n - 1),
        }
    }
}

#[cfg(test)]
#[path = "typeahead_test.rs"]
mod tests;
