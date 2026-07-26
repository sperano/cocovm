//! Interrupt-line sampling and the per-scanline/per-field sync pulses: IRQ/
//! FIRQ/HALT/NMI aggregation, the cartridge CART* interrupt edge detector,
//! and `hsync`/`fs_falling`/`fs_rising`.

use crate::config::MachineVariant;
use crate::gime;

use super::SystemBus;

impl SystemBus {
    /// True when any interrupt source is holding the CPU IRQ line low.
    ///
    /// PIA0's output and the GIME's IRQ output are wired-OR on the CPU pin: at
    /// the BASIC prompt the stock ROM runs the legacy PIA path (INIT0 IEN=0)
    /// off PIA0's field/horizontal sync, while GIME-native software enables
    /// IEN and the $FF92 sources instead (`DESIGN.md` §4).
    pub fn irq_asserted(&self) -> bool {
        self.pia0.irq() || self.gime.irq_asserted()
    }

    /// True when any interrupt source is holding the CPU FIRQ line low: PIA1
    /// (the legacy cartridge FIRQ path) wired-OR with the GIME's FIRQ output
    /// ($FF93 sources gated by INIT0 FEN).
    pub fn firq_asserted(&self) -> bool {
        self.pia1.irq() || self.gime.firq_asserted()
    }

    /// True while the cartridge holds the CPU HALT* line low (the FD-502's
    /// sector-transfer handshake). The only HALT source on a stock CoCo 3 is
    /// the expansion port.
    pub fn halt_asserted(&self) -> bool {
        self.cart.halt_asserted()
    }

    /// Consume a pending NMI edge (the FD-502 gates FDC INTRQ onto NMI).
    pub fn take_nmi(&mut self) -> bool {
        self.cart.take_nmi()
    }

    /// Sample the level-driven CART* interrupt ([`Cartridge::cart_interrupt`],
    /// e.g. the Deluxe RS-232's 6551 ACIA IRQ) and convert transitions into
    /// what the shared physical pin feeds: PIA1 CB1 sees the line level itself
    /// — CART* is active-low, so asserted = CB1 low, and the PIA latches
    /// whichever edge its control register selects — while the GIME EI0
    /// source is raised on the falling (assert) edge only, its hardwired
    /// trigger (GIME border/cart sources are falling-edge, per Lomont; same
    /// treatment as the `cart_line_ties_q` Q-burst in [`SystemBus::hsync`]).
    /// Polled per-instruction from `Machine::run_cycles` so serial-interrupt
    /// latency isn't quantized to scanlines.
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

    /// Horizontal-sync line: the GIME HS pin idles high and pulses low for 16
    /// of 228 pixel clocks at line end (~4.5 µs; MAME `mc6847.cpp`
    /// `TIMER_HSYNC_OFF_TIME`=212/`ON_TIME`=228). Our per-line model has no
    /// resolution below one scanline, so both the falling and rising edges
    /// are emitted back-to-back here rather than timed within the line; each
    /// still only latches PIA0 CA1 (control reg $FF01, port A) and PIA1 CB1
    /// (see below) if it matches that side's selected edge
    /// ([`crate::pia::cr::C1_EDGE_HIGH`]), so software polling either edge —
    /// stock BASIC's falling-edge CRA or NitrOS-9's rising-edge one — still
    /// gets exactly one flag per line. The GIME HBORD source is raised once
    /// per line regardless of edge selection: GIME border sources are
    /// hardwired falling-edge only, not selectable (Lomont).
    ///
    /// Also the GIME's per-scanline sample point for the EI1
    /// keyboard-interrupt input (a zero on any PA0–PA6 row while some column
    /// is strobed — SEB Unravelled II), which fires on falling edge.
    ///
    /// Also the sample point for the expansion-port CART* line: auto-start
    /// game paks tie it to the Q clock (~895 kHz), so while one is inserted
    /// this pulses PIA1 CB1 (the legacy FIRQ cart-boot path) and raises the
    /// GIME EI0 source (the same physical pin) every scanline, which is more
    /// than enough cadence to keep either interrupt path continuously fed.
    pub fn hsync(&mut self) {
        // No GIME on the plain-SAM path (CoCo 1/2): the PIA0/PIA1 Cx1 pulses
        // below stay exactly as-is, but the GIME border/keyboard/cart-EI0
        // interrupt sources it would also raise here don't exist — the GIME
        // struct must stay completely inert on that path
        // (`docs/coco12-plan.md` Phase 4).
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
    /// field, not at end-of-field): latches PIA0 CB1 (control reg $FF03, port
    /// B) — the interrupt that drives BASIC's housekeeping loop — per its
    /// selected edge, and raises the GIME VBORD source (Lomont: "VBORD
    /// generated on falling edge of VSYNC").
    pub fn fs_falling(&mut self) {
        self.pia0.b.set_c1(false);
        // No GIME (hence no VBORD source) on the plain-SAM path — Phase 4.
        if self.variant == MachineVariant::Coco3 {
            self.gime.raise(gime::intr::VBORD);
        }
    }

    /// Field-sync rising edge, at
    /// [`crate::config::VideoStandard::fs_rising_line`] scanlines into the
    /// field: latches PIA0 CB1 per its selected edge (e.g. NitrOS-9-style
    /// rising-edge polling). No GIME border source is tied to this edge.
    pub fn fs_rising(&mut self) {
        self.pia0.b.set_c1(true);
    }
}
