//! Allophone-stream playback: walks a buffer-RAM window handing each
//! allophone address to the SP0256 as soon as it will take one (LRQ high),
//! the way the TMS7040 firmware services the chip's load-request interrupt
//! (MAME `coco_ssc.cpp` wires DRQ to `TMS7000_INT1_LINE`).

use serde::{Deserialize, Serialize};

use super::SoundSpeechCartridge;
use super::protocol::terminator;

/// Allophone addresses the cartridge passes to the SP0256: the board's
/// glue only strobes ALD for values below 64 (MAME `coco_ssc.cpp`
/// `ssc_port_c_w`: `m_tms7000_portd < 64`), matching the AL2 ROM's 64-entry
/// jump table. Larger bytes in a stream are skipped.
pub const ALLOPHONE_COUNT: u8 = 64;

/// Cursor through one allophone stream. A new EXECUTE replaces whatever
/// stream was running; the allophone already latched in the chip finishes
/// regardless (the firmware has no way to cancel it — the SP0256's RESET
/// pin is wired to `$FF7D`, not to the TMS7040).
#[derive(Clone, Copy, Default, Serialize, Deserialize)]
pub(super) struct Speech {
    pub(super) active: bool,
    /// Next RAM offset to read an allophone address from.
    pub(super) cursor: usize,
    /// One past the highest offset this stream may read.
    pub(super) cap: usize,
}

impl SoundSpeechCartridge {
    /// Starts an allophone-stream EXECUTE over `start..cap` and hands the
    /// chip its first allophone right away.
    pub(super) fn start_allophone_execute(&mut self, start: usize, cap: usize) {
        self.speech = Speech {
            active: true,
            cursor: start,
            cap,
        };
        self.feed_speech();
    }

    /// `$C7` abort-all-speech (and `$00`'s speech half): stop feeding the
    /// chip. Whatever it has already latched plays out.
    pub(super) fn stop_all_speech(&mut self) {
        self.speech = Speech::default();
    }

    /// Hand the SP0256 allophones for as long as it accepts them. Ends the
    /// stream at the `$FF` terminator or the window's cap.
    pub(super) fn feed_speech(&mut self) {
        while self.speech.active && self.sp0256.lrq() {
            if self.speech.cursor >= self.speech.cap {
                self.speech.active = false;
                break;
            }
            let byte = self.ram[self.speech.cursor];
            if byte == terminator::SOUND {
                self.speech.active = false;
                break;
            }
            self.speech.cursor += 1;
            if byte < ALLOPHONE_COUNT {
                self.sp0256.ald_write(byte);
            }
        }
    }
}
