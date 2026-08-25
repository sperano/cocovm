use super::*;

/// Feed one bit cell's worth of a fixed PA1 level through `tick` in
/// `tick_size`-cycle chunks (non-divisor of the bit period, so a phase
/// error from chunking would show up).
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
/// bit belongs, then hold mark before returning control to the caller.
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

/// The decoder's mid-cell sampling must tolerate the *source*'s bit period
/// drifting a few percent from the *decoder*'s configured period (real-world
/// clock drift between two independent bit-banged devices).
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
/// discarded, and the decoder resyncs. A valid byte sent afterward must still decode correctly.
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

#[test]
fn a_fully_framed_byte_bumps_bytes_out() {
    let capture = CaptureSink::new();
    let mut bb = BitBanger::new();
    bb.set_sink(Box::new(capture.clone()));
    assert_eq!(bb.bytes_out(), 0);
    feed_byte(&mut bb, 0x55, DEFAULT_BIT_PERIOD, TICK_SIZE);
    assert_eq!(bb.bytes_out(), 1);
    feed_byte(&mut bb, 0x2A, DEFAULT_BIT_PERIOD, TICK_SIZE);
    assert_eq!(bb.bytes_out(), 2);
}

#[test]
fn a_framing_error_does_not_bump_bytes_out() {
    let capture = CaptureSink::new();
    let mut bb = BitBanger::new();
    bb.set_sink(Box::new(capture.clone()));
    feed_bad_frame(&mut bb, 0x2A, DEFAULT_BIT_PERIOD, TICK_SIZE);
    assert_eq!(bb.framing_errors(), 1);
    assert_eq!(bb.bytes_out(), 0);
}

/// The $FFD9 high-speed poke doubles the CPU clock with no change to the
/// ROM's cycle-counted delay loop, so it exactly doubles the effective baud.
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

/// LSB-first framing is unambiguous: 0x01 and 0x80 must decode to themselves,
/// not to each other or to a bit-reversed value.
#[test]
fn lsb_first_ordering() {
    let capture = CaptureSink::new();
    let mut bb = BitBanger::new();
    bb.set_sink(Box::new(capture.clone()));
    feed_byte(&mut bb, 0x01, DEFAULT_BIT_PERIOD, TICK_SIZE);
    feed_byte(&mut bb, 0x80, DEFAULT_BIT_PERIOD, TICK_SIZE);
    assert_eq!(capture.bytes(), vec![0x01, 0x80]);
}

/// A space pulse far shorter than half a bit cell must be rejected by the
/// start-bit validation sample: no byte, no framing error, and the next real frame still decodes.
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

/// A scratch path under the OS temp dir, unique per test run so parallel
/// `cargo test` runs of this file never collide on the same file.
fn scratch_path(name: &str) -> std::path::PathBuf {
    static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "cocovm-bitbanger-test-{}-{n}-{name}",
        std::process::id()
    ))
}

/// [`FileSink`] in faithful (default) mode must write decoded bytes to disk
/// unmodified — including a bare CR line ending, never rewritten to LF/CRLF.
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

/// [`BitBanger::start_file_capture`]/[`BitBanger::stop_capture`] must actually
/// swap the live sink: a decoded byte lands in the file only while capture is active.
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
