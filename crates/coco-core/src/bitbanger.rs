//! Bit-banged serial printer port: PIA1 Data Register A bit 1 ($FF20, TX,
//! DIN pin 4) out, PIA1 Data Register B bit 0 ($FF22, BUSY, DIN pin 2) back
//! in. See `docs/bitbanger-spec.md` for the full verification trail (ROM
//! disassembly of the Color BASIC printer driver, MAME cross-check against
//! `src/devices/bus/rs232/printer.cpp`); every hardware fact cited here is
//! sourced from that document.
//!
//! Unlike the cassette deck (FSK tones demodulated by zero-crossing
//! threshold), the printer port is a plain async serial line: 1 start bit
//! (space) + 8 data bits (LSB-first) + 1 stop bit (mark), no parity
//! (`bitbanger-spec.md` "Framing"). [`BitBanger`] models the *receive* side
//! only — decoding what the ROM's bit-bang driver transmits on PA1 — since
//! that's the only direction a virtual printer needs.
//!
//! Decoding is an edge-triggered RX state machine, ticked once per
//! instruction from `Machine::run_cycles` with a CPU-cycle delta and PA1's
//! current level, the same shape as [`crate::cassette::Cassette::tick`]:
//! idle at mark, a mark→space transition is a start-bit candidate, and each
//! data/stop bit is sampled at its cell midpoint (`bitbanger-spec.md`
//! "Decoder spec"). Cycle-based timing (never wall time) is what makes the
//! CoCo 3 high-speed poke and BASIC's `POKE 150,n` baud changes fall out for
//! free: both just change how many CPU cycles a bit cell spans.
//!
//! BUSY (PIA1 PB0) is the mirror image of the cassette's PA0 input tap: an
//! externally-driven line the emulator feeds back into the PIA so a
//! (currently unimplemented) DMP-105 buffer model can pace BASIC's
//! poll-before/after-every-byte driver, matching `bitbanger-spec.md`
//! "Drive PB0 (BUSY) back into PIA1 as an input".

use std::cell::RefCell;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;
use std::rc::Rc;

use serde::{Deserialize, Serialize};

use crate::dmp105::Dmp105Handle;

/// PIA1 Port A bit 1 ($FF20): the TX line to the printer. 1 = mark/idle
/// (high), 0 = space. Only meaningful as an output when PIA1 DDRA bit 1 is
/// set (ROM init at `$A048` sets DDRA = $FE — `bitbanger-spec.md` "Register
/// map").
pub const TX_PIN: u8 = 0x02;

/// PIA1 Port B bit 0 ($FF22): the BUSY line back from the printer. 0 =
/// ready, 1 = busy (`bitbanger-spec.md` "Register map"; BASIC's driver at
/// `$A2C3`/`$A2F3` polls this bit before and after every byte).
pub const BUSY_PIN: u8 = 0x01;

/// Default bit period in CPU cycles: `cycles_per_bit = 78 + 16*N` with the
/// ROM's live `LPTBTD` default N = 88 (`$0058` at ROM init table `$A10D`,
/// file offset `0x210D`), giving 600 baud at the normal 0.894886 MHz CoCo 3
/// clock (`bitbanger-spec.md` "Baud timing"). A settable field on
/// [`BitBanger`] so `POKE 150,n` (a new N) or the `$FFD9` high-speed poke
/// (double clock, same N) can change the effective rate — both fall out of
/// counting CPU cycles rather than wall time.
pub const DEFAULT_BIT_PERIOD: u32 = 78 + 16 * 88;

/// Data bits per frame: 8, LSB-first, no parity (`bitbanger-spec.md`
/// "Framing").
const DATA_BITS: u8 = 8;

/// Sample index of the start-bit validation check, taken at the middle of
/// the start cell (0.5 bit-times after the falling edge). If the line has
/// already returned to mark by then, the edge was a sub-bit glitch, not a
/// start bit, and the frame is abandoned — standard UART false-start-bit
/// rejection. Without it, the ~30-cycle low pulse PA1 emits while the ROM's
/// boot code reconfigures DDRA (`$A02F`) free-runs into a phantom 0xFF.
const START_SAMPLE: u8 = 0;

/// Samples per frame: the start-bit validation at 0.5 bit-times, one
/// mid-cell sample per data bit at 1.5..8.5, and the stop-bit check at 9.5
/// (`bitbanger-spec.md` "Decoder spec").
const TOTAL_SAMPLES: u8 = DATA_BITS + 2;

/// A destination for decoded printer bytes. Deliberately minimal: this is
/// the seam for later tasks (text-capture-to-file, a DMP-105 command
/// interpreter — `bitbanger-spec.md` "Byte sink is pluggable") and shouldn't
/// grow beyond what the decoder itself needs.
pub trait PrinterSink {
    fn write_byte(&mut self, b: u8);

    /// Snapshot this sink's state for serialization (see the `sink_serde`
    /// module below) — the default, kept by every sink with no state worth
    /// carrying across a save-state (`NoopSink`, [`CaptureSink`]), is
    /// [`sink_serde::SinkState::Noop`].
    fn snapshot(&self) -> sink_serde::SinkState {
        sink_serde::SinkState::Noop
    }

    /// Downcast hook: `Some` only for a live [`Dmp105Handle`] sink, so the
    /// frontend can re-grab the restored handle for the paper window after
    /// `sink_serde::deserialize` rebuilds `sink` (see
    /// [`BitBanger::dmp105_handle`]). Default: not a DMP-105 sink.
    fn as_dmp105(&self) -> Option<&Dmp105Handle> {
        None
    }

    /// True only for [`StoppedFileCaptureSink`] — the marker
    /// `sink_serde::deserialize` installs in place of a live [`FileSink`]
    /// after a snapshot restore. Lets the save-state restore flow
    /// (`crate::snapshot::restore`) tell "print capture was active at save
    /// time, now stopped" apart from "print capture was never active", even
    /// though both restore to functionally the same no-op sink
    /// (`docs/plan-save-states.md`). Default: not that marker.
    fn was_file_capture_stopped_by_restore(&self) -> bool {
        false
    }
}

/// Sink used until something more interesting is plugged in via
/// [`BitBanger::set_sink`]: discards every byte.
struct NoopSink;

impl PrinterSink for NoopSink {
    fn write_byte(&mut self, _b: u8) {}
}

/// Marker sink `sink_serde::deserialize` installs when the snapshot recorded
/// [`sink_serde::SinkState::FileCapture`]: behaves exactly like [`NoopSink`]
/// (a restored file handle is frontend-owned and can't be reopened without
/// frontend involvement — `docs/plan-save-states.md` "on restore, capture is
/// simply stopped"), but is a distinct type so
/// [`PrinterSink::was_file_capture_stopped_by_restore`] can report that
/// capture *was* running, for the snapshot restore flow's standing notes.
struct StoppedFileCaptureSink;

impl PrinterSink for StoppedFileCaptureSink {
    fn write_byte(&mut self, _b: u8) {}
    fn was_file_capture_stopped_by_restore(&self) -> bool {
        true
    }
}

/// Test/diagnostic sink: appends every decoded byte to a shared buffer.
///
/// The buffer is an `Rc<RefCell<_>>` rather than a bare `Vec<u8>` because
/// [`BitBanger`] owns its sink as `Box<dyn PrinterSink>` — once moved into
/// [`BitBanger::set_sink`] a plain `Vec` would be unreachable from the
/// caller. Clone the sink (cheap: it's a refcounted handle to the same
/// buffer) before moving one half in, and read `bytes()` on the other half
/// afterward.
#[derive(Clone, Default)]
pub struct CaptureSink(Rc<RefCell<Vec<u8>>>);

impl CaptureSink {
    pub fn new() -> Self {
        Self::default()
    }

    /// Snapshot of every byte captured so far, in order.
    pub fn bytes(&self) -> Vec<u8> {
        self.0.borrow().clone()
    }
}

impl PrinterSink for CaptureSink {
    fn write_byte(&mut self, b: u8) {
        self.0.borrow_mut().push(b);
    }
}

/// "Print to text file" sink (`docs/printer-plan.md` T2): appends every
/// decoded byte to a file. By default bytes are written unmodified —
/// BASIC's line ending is a bare CR (`$0D`, `bitbanger-spec.md` "Framing")
/// and a faithful capture keeps it, so a captured `LLIST` reads back
/// exactly as the ROM sent it. Optionally (`translate_cr_to_lf`, the GUI's
/// "Translate CR to LF" checkbox) each CR is rewritten to LF so the file
/// reads as normal host text; a CoCo never sends CRLF pairs, so a plain
/// byte-for-byte swap is the whole job (a hypothetical CRLF in the stream
/// would come out LFLF — acceptable for a convenience mode).
///
/// Buffered via [`BufWriter`] so the decoder isn't paying one `write`
/// syscall per character, flushed to the OS whenever a (possibly
/// translated) line ending is seen — the natural granularity for "printer
/// output" (one line at a time), so a tail-follower on the capture file
/// sees whole lines as they print.
pub struct FileSink {
    file: BufWriter<File>,
    translate_cr_to_lf: bool,
}

impl FileSink {
    /// Open `path` for capture: create it if it doesn't exist, truncate it if
    /// it does (a fresh capture session always starts from an empty file).
    /// `translate_cr_to_lf` fixes the sink's line-ending mode for its whole
    /// lifetime — it's a property of the capture session, not a live toggle.
    pub fn create(path: impl AsRef<Path>, translate_cr_to_lf: bool) -> io::Result<Self> {
        Ok(Self {
            file: BufWriter::new(File::create(path)?),
            translate_cr_to_lf,
        })
    }
}

impl PrinterSink for FileSink {
    fn write_byte(&mut self, b: u8) {
        let b = if self.translate_cr_to_lf && b == b'\r' {
            b'\n'
        } else {
            b
        };
        // Best-effort: a full disk or a revoked permission has no useful
        // recovery path from inside the decoder's per-instruction tick, and
        // the alternative (propagating an error out of
        // `PrinterSink::write_byte`) would infect the hot CPU loop with I/O
        // error handling for a side-channel that was never guaranteed to
        // succeed on real hardware either (a jammed printer just eats bytes).
        let _ = self.file.write_all(&[b]);
        if b == b'\r' || b == b'\n' {
            let _ = self.file.flush();
        }
    }

    /// `docs/plan-save-states.md`: "on restore, capture is simply stopped" —
    /// the open file handle is frontend-owned and doesn't survive a
    /// snapshot, but `sink_serde::deserialize` still needs to know a file
    /// capture *was* active so the paper/text distinction isn't lost on the
    /// wire (even though both currently restore to a no-op sink).
    fn snapshot(&self) -> sink_serde::SinkState {
        sink_serde::SinkState::FileCapture
    }
}

/// RX state machine driven by [`BitBanger::tick`].
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
enum RxState {
    /// Idle at mark, hunting for the next mark→space edge (a start-bit
    /// candidate).
    Idle,
    /// Mid-frame: `elapsed` CPU cycles since the falling edge that started
    /// this frame, `sample` samples taken so far ([`START_SAMPLE`] = start
    /// validation, then 8 data bits LSB-first, then the stop-bit check),
    /// and the data bits assembled so far.
    Receiving { elapsed: u32, sample: u8, bits: u8 },
}

/// Async RX decoder for the CoCo's bit-banged serial printer port.
///
/// Receive-only: models what a virtual printer sees on PA1, plus the BUSY
/// line it can drive back. See the module doc comment for the design
/// rationale (cycle-timed ticking, mid-cell sampling, pluggable sink).
#[derive(Serialize, Deserialize)]
pub struct BitBanger {
    /// Cycles per bit cell — see [`DEFAULT_BIT_PERIOD`].
    bit_period: u32,
    /// Count of stop bits that read space instead of mark: a framing
    /// error. The offending byte is discarded, never delivered to the sink
    /// (`bitbanger-spec.md` "Decoder spec": "verify stop bit (mark) else
    /// framing error").
    framing_errors: u32,
    /// BUSY (PIA1 PB0) as asserted by the sink. Defaults to not-busy/ready
    /// (`bitbanger-spec.md`: "0=ready normally; a sink may assert 1=busy").
    busy: bool,
    /// PA1 level seen on the previous tick, so a mark→space transition can
    /// be told apart from a repeated level (idle mark defaults to true,
    /// matching the line's idle-high convention).
    last_mark: bool,
    state: RxState,
    /// The trait object is serialized through the small state enum in
    /// [`sink_serde`], not directly — the DMP-105/paper state must survive
    /// a snapshot even though the sink itself doesn't own a serializable
    /// shape (`docs/plan-save-states.md`).
    #[serde(with = "sink_serde")]
    sink: Box<dyn PrinterSink>,
}

impl Default for BitBanger {
    fn default() -> Self {
        Self {
            bit_period: DEFAULT_BIT_PERIOD,
            framing_errors: 0,
            busy: false,
            last_mark: true,
            state: RxState::Idle,
            sink: Box::new(NoopSink),
        }
    }
}

impl BitBanger {
    pub fn new() -> Self {
        Self::default()
    }

    /// Bit period in CPU cycles (see [`DEFAULT_BIT_PERIOD`]).
    pub fn bit_period(&self) -> u32 {
        self.bit_period
    }

    /// Set the bit period in CPU cycles — e.g. to model `POKE 150,n`
    /// (`cycles_per_bit = 78 + 16*n`) or a non-default `LPTBTD`.
    pub fn set_bit_period(&mut self, cycles: u32) {
        self.bit_period = cycles;
    }

    /// Count of framing errors seen (stop bit read space, not mark) since
    /// construction.
    pub fn framing_errors(&self) -> u32 {
        self.framing_errors
    }

    /// Current BUSY (PIA1 PB0) level: false = ready, true = busy.
    pub fn busy(&self) -> bool {
        self.busy
    }

    /// Assert or clear BUSY, e.g. from a printer-buffer model.
    pub fn set_busy(&mut self, busy: bool) {
        self.busy = busy;
    }

    /// Swap in a new decoded-byte destination (text capture, a DMP-105
    /// interpreter, …). Does not reset in-flight frame state.
    pub fn set_sink(&mut self, sink: Box<dyn PrinterSink>) {
        self.sink = sink;
    }

    /// Start "print to text file" capture at `path` (create/truncate — see
    /// [`FileSink::create`]), so both the CLI (`--print-capture`) and the GUI
    /// (the Machine menu's "Start Print Capture…") can drive it through the
    /// same call. `translate_cr_to_lf` picks the session's line-ending mode
    /// (see [`FileSink`]). Leaves any in-flight frame untouched, like
    /// [`Self::set_sink`].
    pub fn start_file_capture(
        &mut self,
        path: impl AsRef<Path>,
        translate_cr_to_lf: bool,
    ) -> io::Result<()> {
        self.sink = Box::new(FileSink::create(path, translate_cr_to_lf)?);
        Ok(())
    }

    /// Stop capture, restoring the no-op sink (decoded bytes are discarded
    /// again until something else is plugged in).
    pub fn stop_capture(&mut self) {
        self.sink = Box::new(NoopSink);
    }

    /// Attach a [`Dmp105`](crate::dmp105::Dmp105) interpreter as the live
    /// sink (`docs/printer-plan.md` T4): same shape as
    /// [`Self::start_file_capture`], but returns a cloned
    /// [`Dmp105Handle`] (the `CaptureSink` `Rc<RefCell<_>>` pattern) rather
    /// than nothing, since — unlike text capture — the frontend needs to
    /// read the accumulating paper back out while the bus owns the other
    /// half as its sink.
    pub fn start_dmp105(&mut self) -> Dmp105Handle {
        let handle = Dmp105Handle::new();
        self.sink = Box::new(handle.clone());
        handle
    }

    /// The live sink's [`Dmp105Handle`], if it is one — how the frontend
    /// re-grabs the paper-window handle after a snapshot restore rebuilds
    /// `sink` from `sink_serde::SinkState::Dmp105` (a `Dmp105Handle` is a
    /// cheap `Rc` clone, so this is fine to call every frame).
    pub fn dmp105_handle(&self) -> Option<Dmp105Handle> {
        self.sink.as_dmp105().cloned()
    }

    /// True if this `BitBanger` just came back from a snapshot restore whose
    /// sink was a live file capture at save time (`crate::snapshot::restore`'s
    /// standing-notes step, `docs/plan-save-states.md`).
    pub fn capture_was_stopped_on_restore(&self) -> bool {
        self.sink.was_file_capture_stopped_by_restore()
    }

    /// Advance the decoder by `cycles` CPU cycles with PA1 held at
    /// `pa1_mark` (true = mark/high, false = space/low) for that whole
    /// span. Called once per instruction from `Machine::run_cycles`,
    /// alongside `bus.cassette.tick` — cycle-timestamped, not wall-clock,
    /// so `POKE 150,n` and the high-speed poke change the effective rate
    /// for free (module doc comment).
    pub fn tick(&mut self, cycles: u32, pa1_mark: bool) {
        self.state = match self.state {
            RxState::Idle => {
                if self.last_mark && !pa1_mark {
                    // Falling edge: mark -> space, a start-bit candidate.
                    RxState::Receiving {
                        elapsed: cycles,
                        sample: 0,
                        bits: 0,
                    }
                } else {
                    RxState::Idle
                }
            }
            RxState::Receiving {
                mut elapsed,
                mut sample,
                mut bits,
            } => {
                elapsed += cycles;
                let mut false_start = false;
                while sample < TOTAL_SAMPLES && elapsed >= self.sample_threshold(sample) {
                    if sample == START_SAMPLE {
                        if pa1_mark {
                            // Line back at mark mid-start-cell: the falling
                            // edge was a glitch, not a start bit. Abandon
                            // the frame silently (not a framing error — no
                            // frame ever began).
                            false_start = true;
                            break;
                        }
                    } else if sample <= DATA_BITS {
                        bits |= u8::from(pa1_mark) << (sample - 1);
                    } else if pa1_mark {
                        self.sink.write_byte(bits);
                    } else {
                        // Stop bit read space: framing error. Discard the
                        // byte and resync — go back to Idle and hunt for
                        // the next mark->space edge, rather than assuming
                        // the following bits are frame-aligned
                        // (`bitbanger-spec.md` "Decoder spec").
                        self.framing_errors += 1;
                    }
                    sample += 1;
                }
                if false_start || sample >= TOTAL_SAMPLES {
                    RxState::Idle
                } else {
                    RxState::Receiving {
                        elapsed,
                        sample,
                        bits,
                    }
                }
            }
        };
        self.last_mark = pa1_mark;
    }

    /// CPU-cycle offset of sample `sample` (0-indexed) after the start-bit
    /// edge: sample times are 0.5 (start validation), 1.5, …, 8.5 (data),
    /// 9.5 (stop) bit-times (`bitbanger-spec.md` "Decoder spec"), so sample
    /// `k` sits at `(0.5 + k)` bit-times = `bit_period * (2k + 1) / 2`.
    fn sample_threshold(&self, sample: u8) -> u32 {
        let scaled = u64::from(self.bit_period) * (2 * u64::from(sample) + 1);
        (scaled / 2) as u32
    }
}

/// `#[serde(with = "sink_serde")]` for [`BitBanger::sink`]: the trait object
/// itself isn't `Serialize`/`Deserialize` (and shouldn't be — a serialized
/// `Box<dyn PrinterSink>` would either need typetag machinery for a
/// two-implementation seam or leak host file handles into the snapshot), so
/// this maps it to and from the small [`SinkState`] enum instead
/// (`docs/plan-save-states.md`).
// `pub`, not `pub(crate)`: `PrinterSink` itself is public API (implemented
// by `coco-egui`), and its `snapshot` method's return type must be at least
// as visible as the trait or rustc's `private_interfaces` lint fires.
pub mod sink_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use super::{NoopSink, PrinterSink};
    use crate::dmp105::{Dmp105, Dmp105Handle};

    /// What actually needs to survive a snapshot, per live sink kind: a
    /// no-op sink and a file capture both restore to [`NoopSink`] (`FileSink`
    /// holds an open host file handle, frontend-owned — "on restore, capture
    /// is simply stopped", `docs/plan-save-states.md`), while a DMP-105 sink
    /// carries its whole interpreter/paper state across.
    #[derive(Serialize, Deserialize)]
    pub enum SinkState {
        Noop,
        FileCapture,
        Dmp105(Dmp105),
    }

    // `&Box<dyn PrinterSink>`, not `&dyn PrinterSink`: this is what the
    // `#[serde(with = "sink_serde")]` codegen actually calls with (the
    // field's declared type is `Box<dyn PrinterSink>`) — `&Box<T> -> &dyn
    // Trait` isn't a coercion rustc applies at a plain call site, only at
    // method-call receiver position, so narrowing the parameter here would
    // fail to compile.
    #[allow(clippy::borrowed_box)]
    pub(crate) fn serialize<S: Serializer>(
        sink: &Box<dyn PrinterSink>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        sink.snapshot().serialize(serializer)
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Box<dyn PrinterSink>, D::Error> {
        Ok(match SinkState::deserialize(deserializer)? {
            SinkState::Noop => Box::new(NoopSink),
            // Distinct from `SinkState::Noop`, even though both currently
            // behave identically: see `StoppedFileCaptureSink`'s doc comment.
            SinkState::FileCapture => Box::new(super::StoppedFileCaptureSink),
            SinkState::Dmp105(state) => Box::new(Dmp105Handle::from_state(state)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feed one bit cell's worth of a fixed PA1 level through `tick` in
    /// `tick_size`-cycle chunks (a deliberately non-divisor of the bit
    /// period, like the cassette tests' `TICK_CYCLES = 7`, so any phase
    /// error from chunking would accumulate and show up).
    fn feed_bit(bb: &mut BitBanger, level: bool, cycles_total: u32, tick_size: u32) {
        let mut remaining = cycles_total;
        while remaining > 0 {
            let step = remaining.min(tick_size);
            bb.tick(step, level);
            remaining -= step;
        }
    }

    /// Feed one full frame (start + 8 data bits LSB-first + stop) built at
    /// `period` cycles/bit.
    fn feed_byte(bb: &mut BitBanger, byte: u8, period: u32, tick_size: u32) {
        feed_bit(bb, false, period, tick_size); // start bit: space
        for i in 0..8 {
            let bit = (byte >> i) & 1 == 1;
            feed_bit(bb, bit, period, tick_size);
        }
        feed_bit(bb, true, period, tick_size); // stop bit: mark
    }

    /// Feed a malformed frame: start + 8 data bits + a SPACE where the stop
    /// bit belongs (framing violation), then hold mark for a while before
    /// returning control to the caller.
    fn feed_bad_frame(bb: &mut BitBanger, byte: u8, period: u32, tick_size: u32) {
        feed_bit(bb, false, period, tick_size); // start bit: space
        for i in 0..8 {
            let bit = (byte >> i) & 1 == 1;
            feed_bit(bb, bit, period, tick_size);
        }
        feed_bit(bb, false, period, tick_size); // bad stop bit: space
        feed_bit(bb, true, period, tick_size); // line returns to idle mark
    }

    const TICK_SIZE: u32 = 7;

    #[test]
    fn decodes_0x55() {
        let capture = CaptureSink::new();
        let mut bb = BitBanger::new();
        bb.set_sink(Box::new(capture.clone()));
        feed_byte(&mut bb, 0x55, DEFAULT_BIT_PERIOD, TICK_SIZE);
        assert_eq!(capture.bytes(), vec![0x55]);
        assert_eq!(bb.framing_errors(), 0);
    }

    #[test]
    fn decodes_0x00() {
        let capture = CaptureSink::new();
        let mut bb = BitBanger::new();
        bb.set_sink(Box::new(capture.clone()));
        feed_byte(&mut bb, 0x00, DEFAULT_BIT_PERIOD, TICK_SIZE);
        assert_eq!(capture.bytes(), vec![0x00]);
    }

    #[test]
    fn decodes_0xff() {
        let capture = CaptureSink::new();
        let mut bb = BitBanger::new();
        bb.set_sink(Box::new(capture.clone()));
        feed_byte(&mut bb, 0xFF, DEFAULT_BIT_PERIOD, TICK_SIZE);
        assert_eq!(capture.bytes(), vec![0xFF]);
    }

    #[test]
    fn decodes_ascii_a() {
        let capture = CaptureSink::new();
        let mut bb = BitBanger::new();
        bb.set_sink(Box::new(capture.clone()));
        feed_byte(&mut bb, b'A', DEFAULT_BIT_PERIOD, TICK_SIZE);
        assert_eq!(capture.bytes(), vec![b'A']);
    }

    #[test]
    fn multi_byte_stream_decodes_in_order() {
        let capture = CaptureSink::new();
        let mut bb = BitBanger::new();
        bb.set_sink(Box::new(capture.clone()));
        let bytes = [b'H', b'e', b'l', b'l', b'o', 0x0D];
        for &b in &bytes {
            feed_byte(&mut bb, b, DEFAULT_BIT_PERIOD, TICK_SIZE);
        }
        assert_eq!(capture.bytes(), bytes);
        assert_eq!(bb.framing_errors(), 0);
    }

    /// The decoder's mid-cell sampling must tolerate the *source*'s bit
    /// period drifting a few percent from the *decoder*'s configured
    /// period — real-world clock drift between two independent bit-banged
    /// devices (`bitbanger-spec.md` "Decoder spec": "A robust decoder
    /// should tolerate a few percent deviation"). Build the waveform at a
    /// period 2% slower than the decoder's default and confirm it still
    /// decodes: by bit 9 (9.5 bit-times in) accumulated drift is under
    /// 0.19 bit-times, well inside the 0.5 bit-time margin each mid-cell
    /// sample has.
    #[test]
    fn timing_slop_plus_two_percent_still_decodes() {
        let source_period = DEFAULT_BIT_PERIOD + DEFAULT_BIT_PERIOD / 50; // +2%
        let capture = CaptureSink::new();
        let mut bb = BitBanger::new(); // decoder stays at the default period
        bb.set_sink(Box::new(capture.clone()));
        feed_byte(&mut bb, 0x41, source_period, TICK_SIZE);
        assert_eq!(capture.bytes(), vec![0x41]);
        assert_eq!(bb.framing_errors(), 0);
    }

    #[test]
    fn timing_slop_minus_two_percent_still_decodes() {
        let source_period = DEFAULT_BIT_PERIOD - DEFAULT_BIT_PERIOD / 50; // -2%
        let capture = CaptureSink::new();
        let mut bb = BitBanger::new();
        bb.set_sink(Box::new(capture.clone()));
        feed_byte(&mut bb, 0x41, source_period, TICK_SIZE);
        assert_eq!(capture.bytes(), vec![0x41]);
        assert_eq!(bb.framing_errors(), 0);
    }

    /// A stop bit that reads space is a framing error: counted, the byte
    /// discarded (never reaches the sink), and the decoder resyncs by
    /// going back to hunting for the next mark->space edge rather than
    /// assuming the following bits are frame-aligned. A valid byte sent
    /// afterward, starting from a fresh edge, must still decode correctly.
    #[test]
    fn framing_error_counts_and_resyncs() {
        let capture = CaptureSink::new();
        let mut bb = BitBanger::new();
        bb.set_sink(Box::new(capture.clone()));

        feed_bad_frame(&mut bb, 0x2A, DEFAULT_BIT_PERIOD, TICK_SIZE);
        assert_eq!(bb.framing_errors(), 1);
        assert!(capture.bytes().is_empty());

        feed_byte(&mut bb, 0x2A, DEFAULT_BIT_PERIOD, TICK_SIZE);
        assert_eq!(capture.bytes(), vec![0x2A]);
        assert_eq!(bb.framing_errors(), 1);
    }

    /// The $FFD9 high-speed poke doubles the CPU clock with no change to
    /// the ROM's cycle-counted delay loop, so it exactly doubles the
    /// effective baud (`bitbanger-spec.md` "Baud timing"). Configuring the
    /// decoder at half the default period and decoding a byte sent at that
    /// rate proves the speed-poke/`POKE 150,n` relationship falls out of
    /// pure cycle counting, with no special-cased "fast mode".
    #[test]
    fn double_rate_bit_period_decodes() {
        const DOUBLE_RATE_PERIOD: u32 = DEFAULT_BIT_PERIOD / 2; // 743
        let capture = CaptureSink::new();
        let mut bb = BitBanger::new();
        bb.set_bit_period(DOUBLE_RATE_PERIOD);
        bb.set_sink(Box::new(capture.clone()));
        feed_byte(&mut bb, 0x41, DOUBLE_RATE_PERIOD, TICK_SIZE);
        assert_eq!(capture.bytes(), vec![0x41]);
    }

    /// LSB-first framing is unambiguous: 0x01 and 0x80 must decode to
    /// themselves, not to each other or to a bit-reversed value, proving
    /// bit order is handled correctly in both directions.
    #[test]
    fn lsb_first_ordering() {
        let capture = CaptureSink::new();
        let mut bb = BitBanger::new();
        bb.set_sink(Box::new(capture.clone()));
        feed_byte(&mut bb, 0x01, DEFAULT_BIT_PERIOD, TICK_SIZE);
        feed_byte(&mut bb, 0x80, DEFAULT_BIT_PERIOD, TICK_SIZE);
        assert_eq!(capture.bytes(), vec![0x01, 0x80]);
    }

    /// A space pulse far shorter than half a bit cell — like the ~30-cycle
    /// PA1 glitch the ROM's boot-time DDRA reconfiguration produces — must
    /// be rejected by the start-bit validation sample at 0.5 bit-times:
    /// no byte, no framing error, and the next real frame still decodes.
    #[test]
    fn sub_bit_glitch_is_rejected_as_false_start() {
        const GLITCH_CYCLES: u32 = 30;
        let capture = CaptureSink::new();
        let mut bb = BitBanger::new();
        bb.set_sink(Box::new(capture.clone()));

        feed_bit(&mut bb, false, GLITCH_CYCLES, TICK_SIZE);
        feed_bit(&mut bb, true, DEFAULT_BIT_PERIOD * 2, TICK_SIZE);
        assert!(capture.bytes().is_empty());
        assert_eq!(bb.framing_errors(), 0);

        feed_byte(&mut bb, b'A', DEFAULT_BIT_PERIOD, TICK_SIZE);
        assert_eq!(capture.bytes(), vec![b'A']);
        assert_eq!(bb.framing_errors(), 0);
    }

    #[test]
    fn busy_defaults_to_ready() {
        let bb = BitBanger::new();
        assert!(!bb.busy());
    }

    #[test]
    fn busy_is_settable() {
        let mut bb = BitBanger::new();
        bb.set_busy(true);
        assert!(bb.busy());
    }

    /// A scratch path under the OS temp dir, unique per test run (`std::env`
    /// PID + a per-call counter) so parallel `cargo test` runs of this file
    /// never collide on the same file.
    fn scratch_path(name: &str) -> std::path::PathBuf {
        static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "coco-rs-bitbanger-test-{}-{n}-{name}",
            std::process::id()
        ))
    }

    /// [`FileSink`] in faithful (default) mode must write decoded bytes to
    /// disk unmodified — including a bare CR line ending, never rewritten
    /// to LF/CRLF (struct doc comment).
    #[test]
    fn file_sink_writes_bytes_unmodified() {
        let path = scratch_path("writes-bytes");
        let mut sink = FileSink::create(&path, false).expect("create scratch file");
        for &b in b"HELLO\r" {
            sink.write_byte(b);
        }
        drop(sink);
        let contents = std::fs::read(&path).expect("read scratch file");
        assert_eq!(contents, b"HELLO\r");
        let _ = std::fs::remove_file(&path);
    }

    /// With `translate_cr_to_lf` set, every CR comes out as LF and every
    /// other byte is untouched (the GUI's "Translate CR to LF" checkbox).
    #[test]
    fn file_sink_translates_cr_to_lf_when_asked() {
        let path = scratch_path("cr-to-lf");
        let mut sink = FileSink::create(&path, true).expect("create scratch file");
        for &b in b"HELLO\rWORLD\r" {
            sink.write_byte(b);
        }
        drop(sink);
        let contents = std::fs::read(&path).expect("read scratch file");
        assert_eq!(contents, b"HELLO\nWORLD\n");
        let _ = std::fs::remove_file(&path);
    }

    /// [`FileSink::create`] truncates an existing file rather than appending,
    /// so a fresh capture session never inherits a previous run's tail.
    #[test]
    fn file_sink_create_truncates_existing_file() {
        let path = scratch_path("truncates");
        std::fs::write(&path, b"stale content that must be gone").unwrap();
        let mut sink = FileSink::create(&path, false).expect("create scratch file");
        sink.write_byte(b'X');
        drop(sink);
        let contents = std::fs::read(&path).expect("read scratch file");
        assert_eq!(contents, b"X");
        let _ = std::fs::remove_file(&path);
    }

    /// [`BitBanger::start_file_capture`]/[`BitBanger::stop_capture`] must
    /// actually swap the live sink: a decoded byte lands in the file only
    /// while capture is active, and `stop_capture` restores the no-op sink
    /// (`bitbanger-spec.md`'s `NoopSink`) so nothing after that point is ever
    /// written.
    #[test]
    fn start_and_stop_file_capture_gate_decoded_bytes() {
        let path = scratch_path("start-stop");
        let mut bb = BitBanger::new();
        bb.start_file_capture(&path, false).expect("start capture");
        feed_byte(&mut bb, b'A', DEFAULT_BIT_PERIOD, TICK_SIZE);
        bb.stop_capture();
        feed_byte(&mut bb, b'B', DEFAULT_BIT_PERIOD, TICK_SIZE);
        let contents = std::fs::read(&path).expect("read scratch file");
        assert_eq!(contents, b"A");
        let _ = std::fs::remove_file(&path);
    }
}
