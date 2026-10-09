//! End-to-end regression for the bit-banger printer port under NitrOS-9:
//! boot the real EOU 1.0.1 Level 2 disk images
//! to a shell (same asset pattern as `tests/vhd_boot.rs`), run `echo hello
//! >/p`, and assert the bytes reaching a [`CaptureSink`] match exactly what
//! the shell's `/p` redirection sent — proving the bit-banger decoder works
//! against a second, independently-written driver (NitrOS-9's own bit-bang
//! code), not only Color BASIC's. Skips gracefully if `roms/`/`tests/`
//! assets aren't present, matching `tests/vhd_boot.rs`.
//!
//! ## Bit rate: NitrOS-9 is not Color BASIC's 600-baud constant
//!
//! EOU boots the CoCo 3 GIME straight into high-speed mode (`$FFD9`,
//! `GIME::cpu_fast == true` — confirmed live, not assumed) and its `/p`
//! driver does **not** behave like Color BASIC's, which busy-waits a fixed
//! cycle count that the speed poke exactly doubles the effective baud of
//! (wiki `cocovm/bitbanger-spec` "Baud timing"). Direct instrumentation of
//! `BitBanger::tick`'s raw PA1 edge intervals during a live boot (decoding a
//! 619-byte `dir /dd >/p` listing byte-for-byte against the known directory
//! contents) found the bit-cell quantum is **twice**
//! [`bitbanger::DEFAULT_BIT_PERIOD`] (1486 cycles): NitrOS-9's driver holds
//! true wall-clock baud at 600 regardless of `cpu_fast` by doubling its own
//! delay-loop cycle count to compensate for the doubled clock, the opposite
//! of BASIC's speed-oblivious driver. See wiki `cocovm/bitbanger-spec`'s
//! "NitrOS-9 `/p` driver (T3 finding, empirical, not ROM-disassembled)" for
//! the full derivation — this is a measured fact from unmodified EOU code,
//! not a disassembly of the `/p` driver's source.

use coco_core::bitbanger::CaptureSink;
mod printer_boot;
use printer_boot::*;

/// Boot NitrOS-9 EOU to a shell, `echo hello >/p`, and check the bit-banger
/// decoder (retuned to [`OS9_PRINTER_BIT_PERIOD`]) captured exactly what the
/// shell sent, with no framing errors.
#[test]
fn os9_echo_redirected_to_printer_is_captured() {
    /// Generous upper bound on `echo hello >/p` finishing and the shell
    /// prompt returning: a handful of bytes at 600 baud (14,860 cycles/byte
    /// at [`OS9_PRINTER_BIT_PERIOD`]-per-bit) is nowhere near this budget
    /// even accounting for OS-9 scheduling overhead.
    const MAX_PRINT_FIELDS: usize = 6_000;

    let Some((mut m, vhd_copy)) = boot_eou_shell("bitbanger-os9", |_| {}) else {
        return;
    };

    let capture = CaptureSink::new();
    m.bus.bitbanger.set_sink(Box::new(capture.clone()));
    m.bus.bitbanger.set_bit_period(OS9_PRINTER_BIT_PERIOD);
    let baseline_prompt = shell_prompt_count(&mut m);

    type_str(&mut m, "echo hello >/p");
    tap_char(&mut m, '\r');

    let screen = wait_for_new_shell_prompt(&mut m, baseline_prompt, MAX_PRINT_FIELDS);
    assert_eq!(
        m.bus.bitbanger.framing_errors(),
        0,
        "bit-banger decoder saw framing errors decoding OS-9's /p output; screen:\n{screen}"
    );
    assert_eq!(
        capture.bytes(),
        b"hello \r",
        "captured /p bytes did not match `echo hello`'s expected output; got {:?}",
        String::from_utf8_lossy(&capture.bytes())
    );

    let _ = std::fs::remove_file(&vhd_copy);
}
