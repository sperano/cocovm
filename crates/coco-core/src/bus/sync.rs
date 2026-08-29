//! Interrupt-line sampling and the per-scanline/per-field sync pulses: IRQ/
//! FIRQ/HALT/NMI aggregation, the cartridge CART* interrupt edge detector,
//! and `hsync`/`fs_falling`/`fs_rising`.

use crate::config::MachineVariant;
use crate::gime;

use super::SystemBus;

impl SystemBus {
    /// True when any interrupt source is holding the CPU IRQ line low: PIA0's
    /// output wired-OR with the GIME's IRQ output (`DESIGN.md` §4).
    pub fn irq_asserted(&self) -> bool {
        self.pia0.irq() || self.gime.irq_asserted()
    }

    /// True when any interrupt source is holding the CPU FIRQ line low: PIA1
    /// wired-OR with the GIME's FIRQ output.
    pub fn firq_asserted(&self) -> bool {
        self.pia1.irq() || self.gime.firq_asserted()
    }

    /// True while the cartridge holds the CPU HALT* line low (the FD-502's
    /// sector-transfer handshake) — the only HALT source on a stock CoCo 3.
    pub fn halt_asserted(&self) -> bool {
        self.cart.halt_asserted()
    }

    /// Consume a pending NMI edge (the FD-502 gates FDC INTRQ onto NMI).
    pub fn take_nmi(&mut self) -> bool {
        self.cart.take_nmi()
    }

    /// Power-on: forget the sampled keyboard-line and CART* levels so an
    /// input still asserted from before power-off fires on its first sample.
    pub(crate) fn reset_edge_history(&mut self) {
        self.kbd_line_low = false;
        self.prev_cart_int = false;
    }

    /// Sample the level-driven CART* interrupt ([`Cartridge::cart_interrupt`])
    /// and convert transitions into PIA1 CB1 (level, active-low) and a GIME
    /// EI0 raise on the falling edge only (GIME sources are hardwired
    /// falling-edge — Lomont). Polled per-instruction so latency isn't
    /// quantized to scanlines.
    ///
    /// [`Cartridge::cart_interrupt`]: crate::cart::Cartridge::cart_interrupt
    pub fn poll_cart_interrupt(&mut self) {
        let level = self.cart.cart_interrupt();
        if level == self.prev_cart_int {
            return;
        }
        self.prev_cart_int = level;
        self.pia1.b.set_c1(!level);
        if level && self.variant == MachineVariant::Coco3 {
            self.gime.raise(gime::intr::EI0);
        }
    }

    /// Horizontal-sync line: GIME HS pulses low for 16 of 228 pixel clocks at
    /// line end (~4.5 µs; MAME `mc6847.cpp` `TIMER_HSYNC_OFF_TIME`=212/
    /// `ON_TIME`=228). Modeled per-line (no sub-scanline resolution), so both
    /// edges fire back-to-back here; each latches PIA0 CA1/PIA1 CB1 only if it
    /// matches that side's selected edge, so either polling style still gets
    /// one flag per line. GIME HBORD is hardwired falling-edge only (Lomont),
    /// raised once per line regardless of edge selection.
    ///
    /// Also the per-scanline sample point for GIME EI1 (keyboard: a zero on
    /// any PA0–PA6 row while some column is strobed, falling edge — SEB
    /// Unravelled II) and for the CART* line (autostart paks tie it to the Q
    /// clock, ~895 kHz, so this pulses PIA1 CB1 and raises GIME EI0 every
    /// scanline while one's inserted).
    pub fn hsync(&mut self) {
        // No GIME on the plain-SAM path: PIA0/PIA1 Cx1 pulses still fire,
        // but the following GIME sources don't raise.
        let is_gime = self.variant == MachineVariant::Coco3;
        self.pia0.a.set_c1(false);
        if is_gime {
            self.gime.raise(gime::intr::HBORD);
        }
        self.pia0.a.set_c1(true);
        // Buttons are included: SEB warns joystick fire buttons always trip EI1.
        let line_low = self.pia0_pa_pins() & 0x7F != 0x7F;
        if is_gime && line_low && !self.kbd_line_low {
            self.gime.raise(gime::intr::EI1);
        }
        self.kbd_line_low = line_low;
        if self.cart.cart_line_ties_q() {
            self.pia1.b.set_c1(false);
            if is_gime {
                self.gime.raise(gime::intr::EI0);
            }
            self.pia1.b.set_c1(true);
        }
    }

    /// Field-sync falling edge (~60/50 Hz vertical, at
    /// [`crate::config::VideoStandard::fs_falling_line`] scanlines into the
    /// field): latches PIA0 CB1 per its selected edge and raises the GIME
    /// VBORD source (Lomont: "VBORD generated on falling edge of VSYNC").
    pub fn fs_falling(&mut self) {
        self.pia0.b.set_c1(false);
        // No GIME on the plain-SAM path, so no VBORD source.
        if self.variant == MachineVariant::Coco3 {
            self.gime.raise(gime::intr::VBORD);
        }
    }

    /// Field-sync rising edge, at
    /// [`crate::config::VideoStandard::fs_rising_line`]: latches PIA0 CB1 per
    /// its selected edge. No GIME border source is tied to this edge.
    pub fn fs_rising(&mut self) {
        self.pia0.b.set_c1(true);
    }
}
