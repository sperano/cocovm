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

/// Fields one queued tap occupies in [`TypeAhead::advance`] under a target
/// that scans continuously: hold + gap minimums + three transition fields.
pub(crate) const FIELDS_PER_TAP: u64 = TYPE_HOLD_FIELDS as u64 + TYPE_GAP_FIELDS as u64 + 3;

/// Column reads that prove the target registered a held key: Color BASIC's
/// KEYIN scan, its debounce re-read, and the next scan.
pub(crate) const TAP_READS_TO_REGISTER: u32 = 3;
/// Column reads that prove the target saw the key released.
const TAP_READS_TO_RELEASE: u32 = 1;

/// Modifiers captured with a queued tap, independent of the host keyboard.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct KeyModifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct KeyTap {
    pub pos: Pos,
    pub modifiers: KeyModifiers,
}

impl From<(Pos, bool)> for KeyTap {
    fn from((pos, shift): (Pos, bool)) -> Self {
        Self {
            pos,
            modifiers: KeyModifiers {
                shift,
                ..Default::default()
            },
        }
    }
}

impl KeyTap {
    fn apply(self, kb: &mut kbd::Keyboard, down: bool) {
        kb.set(kbd::SHIFT, down && self.modifiers.shift);
        kb.set(kbd::CTRL, down && self.modifiers.ctrl);
        kb.set(kbd::ALT, down && self.modifiers.alt);
        kb.set(self.pos, down);
    }
}

/// Type-ahead: replays queued keys at the pace the target scans them, holding
/// and releasing each until the CPU has read its column (see `phase_done`).
#[derive(Default)]
pub(crate) struct TypeAhead {
    pub(crate) queue: VecDeque<KeyTap>,
    pub(crate) phase: TypePhase,
    pub(crate) current: KeyTap,
    /// The key's column-read count when the current phase began.
    reads_at_phase_start: u32,
}

/// Where the current tap is; the payload counts fields spent in the phase.
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
                    self.begin_phase(kb, true, TypePhase::Hold(0));
                }
            }
            TypePhase::Hold(fields) => {
                if self.phase_done(kb, fields, TYPE_HOLD_FIELDS, TAP_READS_TO_REGISTER) {
                    self.begin_phase(kb, false, TypePhase::Gap(0));
                } else {
                    // Re-asserted every field: a focus-loss `release_all` between
                    // fields must not cut the hold short.
                    self.current.apply(kb, true);
                    self.phase = TypePhase::Hold(fields + 1);
                }
            }
            TypePhase::Gap(fields) => {
                if self.phase_done(kb, fields, TYPE_GAP_FIELDS, TAP_READS_TO_RELEASE) {
                    self.phase = TypePhase::Idle;
                } else {
                    self.phase = TypePhase::Gap(fields + 1);
                }
            }
        }
    }

    /// Press or release the current tap and start counting the CPU's reads
    /// of its column from here.
    fn begin_phase(&mut self, kb: &mut kbd::Keyboard, down: bool, phase: TypePhase) {
        self.current.apply(kb, down);
        self.reads_at_phase_start = kb.column_reads(self.current.pos.1);
        self.phase = phase;
    }

    /// A phase ends once `min_fields` have elapsed and the CPU has read the
    /// key's column `reads` times since it began, or at the scan timeout.
    fn phase_done(&self, kb: &kbd::Keyboard, fields: u8, min_fields: u8, reads: u32) -> bool {
        let seen = kb
            .column_reads(self.current.pos.1)
            .wrapping_sub(self.reads_at_phase_start);
        fields >= min_fields && (seen >= reads || fields >= TYPE_SCAN_TIMEOUT_FIELDS)
    }
}

#[cfg(test)]
#[path = "typeahead_test.rs"]
pub(crate) mod tests;
