//! Tandy Multi-Pak Interface (MPI, 26-3024): a 4-slot passive expansion
//! adapter for the cartridge port.

use serde::{Deserialize, Serialize};

use super::{Cart, Cartridge, IO_OPEN_BUS};

/// Tandy Multi-Pak Interface (MPI, 26-3024): a 4-slot passive expansion
/// adapter for the cartridge port. The following facts are verified against MAME
/// `src/devices/bus/coco/coco_multi.cpp` (`coco_multipak_device`) and the
/// Lomont CoCo Hardware reference.
///
/// All expansion-port lines are shared across the 4 slots except SCS*, CTS*,
/// and CART* (MAME's `coco_multi.cpp` header comment): those three follow the
/// select register that follows, while `halt_asserted`/`take_nmi`/`tick` reach every
/// slot regardless of selection (a device doesn't stop because it isn't
/// currently addressed).
///
/// Two MAME facts are deliberately NOT modeled here, per spec: the CoCo 3
/// never delivers external-ROM-window *writes* to cartridges at all (already
/// true of `SystemBus`, independent of the MPI), and the field-mod some real
/// MPIs have that ties all 4 slots' CART* lines together (a hardware hack,
/// not stock behaviour) is not reproduced — CART* here strictly follows the
/// CTS select, as spec'd.
#[derive(Debug, Serialize, Deserialize)]
pub struct MultiPak {
    pub(super) slots: [Cart; mpi::SLOT_COUNT],
    /// The raw `$FF7F` select register (both used and forced-high unused
    /// bits — [`Cartridge::control_read`] applies [`mpi::READBACK_OR_MASK`]
    /// on the way out, so this can be compared directly against a switch
    /// value from [`mpi::SWITCH_VALUES`]).
    select: u8,
    /// Front-panel switch position (slot index, 0-3), moved by
    /// [`MultiPak::set_switch`].
    switch_slot: usize,
    /// Set by any software write to `$FF7F` (MAME `m_block`); while set,
    /// [`MultiPak::set_switch`] still records the new switch position but
    /// does not apply it to `select` — real hardware ignores the switch
    /// until the next reset once software has taken over slot selection.
    switch_blocked: bool,
}

/// `$FF7F` select-register bitfield constants and the front-panel switch
/// lookup (MAME `coco_multi.cpp`).
pub mod mpi {
    /// Number of physical cartridge slots.
    pub const SLOT_COUNT: usize = 4;

    /// SCS slot-select field (bits 1-0): the `$FF40-$FF5F` I/O window routes
    /// to this slot only.
    pub const SCS_MASK: u8 = 0x03;
    /// CTS slot-select field, bit position (bits 5-4): the external ROM
    /// window (`rom_read`) and the CART* line both follow this slot only —
    /// CART* is not independently selectable from CTS.
    pub const CTS_SHIFT: u8 = 4;
    /// CTS slot-select field, mask after shifting into position.
    pub const CTS_MASK: u8 = 0x03 << CTS_SHIFT;

    /// Bits 7, 6, 3, 2 are unused; a `$FF7F` read forces them high (MAME
    /// `coco_multi.cpp` `select_byte | 0xCC`). A write replaces the entire
    /// byte — there is no nibble merge on the way in, only this OR-mask on
    /// the way out.
    pub const READBACK_OR_MASK: u8 = 0xCC;

    /// Front-panel switch position -> power-on/reset `$FF7F` value, one
    /// entry per physical slot 1-4 (MAME `MULTI_SLOT_LOOKUP`). Both the SCS
    /// and CTS fields already point at the same slot, and the unused bits
    /// already read as the forced-high pattern, so these double as valid
    /// post-readback values too.
    pub const SWITCH_VALUES: [u8; SLOT_COUNT] = [0xCC, 0xDD, 0xEE, 0xFF];
}

impl MultiPak {
    /// Build an MPI with all 4 slots empty, select loaded from `switch_slot`
    /// (0-3; the conventional default is 3 — slot 4, the disk-controller
    /// slot).
    pub fn new(switch_slot: usize) -> Self {
        Self {
            slots: std::array::from_fn(|_| Cart::default()),
            select: mpi::SWITCH_VALUES[switch_slot],
            switch_slot,
            switch_blocked: false,
        }
    }

    /// Plug a cartridge into `slot` (0-3).
    pub fn insert(&mut self, slot: usize, cart: impl Into<Cart>) {
        self.slots[slot] = cart.into();
    }

    /// Remove whatever is in `slot`, restoring the empty slot.
    pub fn eject(&mut self, slot: usize) {
        self.slots[slot] = Cart::default();
    }

    /// Model moving the physical front-panel switch to `slot` (0-3). Updates
    /// the live select register immediately unless a software write to
    /// `$FF7F` has taken over selection since the last reset (see
    /// [`MultiPak::switch_blocked`]); the switch position itself is always
    /// recorded, so the next reset picks it up regardless.
    pub fn set_switch(&mut self, slot: usize) {
        self.switch_slot = slot;
        if !self.switch_blocked {
            self.select = mpi::SWITCH_VALUES[slot];
        }
    }

    /// The current front-panel switch position (0-3), for UI display —
    /// independent of whether it's currently controlling `select` (see
    /// [`MultiPak::switch_blocked`]).
    pub fn switch_slot(&self) -> usize {
        self.switch_slot
    }

    /// True if a software write to `$FF7F` is overriding the front-panel
    /// switch (cleared on the next reset).
    pub fn switch_blocked(&self) -> bool {
        self.switch_blocked
    }

    /// The slot currently selected for the SCS I/O window (`$FF40-$FF5F`).
    pub fn scs_slot(&self) -> usize {
        (self.select & mpi::SCS_MASK) as usize
    }

    /// The slot currently selected for the CTS ROM window and the CART* line.
    pub fn cts_slot(&self) -> usize {
        ((self.select & mpi::CTS_MASK) >> mpi::CTS_SHIFT) as usize
    }
}

/// Standard SCS* window (`$FF40-$FF5F`): routed only to the SCS-selected
/// slot, same as `CART*`/`CTS*` follow the CTS-selected slot. The `$FF60-
/// $FF7E` extension some carts decode (`docs/cartridges.md` "Carts can
/// decode addresses outside SCS") is NOT switched by the MPI — the address
/// and data buses are common to every slot, only SCS*/CTS*/CART* are
/// per-slot — so it's handled separately in the following code.
const SCS_BASE: u16 = 0xFF40;
const SCS_LAST: u16 = 0xFF5F;

impl Cartridge for MultiPak {
    /// `$FF40-$FF5F` (SCS*) routes to the SCS-selected slot only; `$FF60-$FF7E`
    /// is broadcast to every slot instead — a real MPI switches only
    /// SCS*/CTS*/CART*, not the shared address/data bus (`coco_multi.cpp:9-19`),
    /// so a device decoding raw addresses there (such as the Deluxe RS-232 Pak's
    /// ACIA at `$FF68-$FF6B`, `coco_rs232.cpp:57-62`) answers from any slot.
    fn read(&mut self, addr: u16) -> u8 {
        if (SCS_BASE..=SCS_LAST).contains(&addr) {
            return self.slots[self.scs_slot()].read(addr);
        }
        // $FF60-$FF7E: broadcast to every slot and return the first
        // non-open-bus response. Real hardware would bus-fight if two
        // plugged-in carts both decoded the same extension address; in
        // practice at most one ever does.
        self.slots
            .iter_mut()
            .map(|slot| slot.read(addr))
            .find(|&val| val != IO_OPEN_BUS)
            .unwrap_or(IO_OPEN_BUS)
    }

    fn write(&mut self, addr: u16, val: u8) {
        if (SCS_BASE..=SCS_LAST).contains(&addr) {
            self.slots[self.scs_slot()].write(addr, val);
            return;
        }
        // $FF60-$FF7E: every slot sees the write (see `read`'s comment) —
        // whichever cart(s) decode this address react to it.
        for slot in &mut self.slots {
            slot.write(addr, val);
        }
    }

    fn rom_read(&mut self, addr: u16) -> u8 {
        self.slots[self.cts_slot()].rom_read(addr)
    }

    fn rom_peek(&self, addr: u16) -> u8 {
        self.slots[self.cts_slot()].rom_peek(addr)
    }

    /// [`MultiPak::read`]'s side-effect-free twin: same SCS-slot/broadcast split.
    fn peek(&self, addr: u16) -> u8 {
        if (SCS_BASE..=SCS_LAST).contains(&addr) {
            return self.slots[self.scs_slot()].peek(addr);
        }
        self.slots
            .iter()
            .map(|slot| slot.peek(addr))
            .find(|&val| val != IO_OPEN_BUS)
            .unwrap_or(IO_OPEN_BUS)
    }

    fn peek_control(&self) -> u8 {
        self.select | mpi::READBACK_OR_MASK
    }

    /// `$FF90-$FF97` (the CoCo Max module's ADC window) is broadcast to
    /// every slot, same as the `$FF60-$FF7E` extension — the module has no
    /// SCS* wiring of its own; like the Deluxe RS-232 Pak's ACIA, it decodes
    /// the raw address bus, which the MPI does not switch (MAME's
    /// `coco_multipak_device::cartridge_space()` forwards straight to the
    /// parent bus for every slot, not just the one `$FF7F` selects).
    fn upper_io_read(&mut self, addr: u16) -> u8 {
        self.slots
            .iter_mut()
            .map(|slot| slot.upper_io_read(addr))
            .find(|&val| val != IO_OPEN_BUS)
            .unwrap_or(IO_OPEN_BUS)
    }

    /// See [`MultiPak::upper_io_read`]'s doc: broadcast, not SCS-selected.
    fn upper_io_write(&mut self, addr: u16, val: u8) {
        for slot in &mut self.slots {
            slot.upper_io_write(addr, val);
        }
    }

    /// [`MultiPak::upper_io_read`]'s side-effect-free twin.
    fn upper_io_peek(&self, addr: u16) -> u8 {
        self.slots
            .iter()
            .map(|slot| slot.upper_io_peek(addr))
            .find(|&val| val != IO_OPEN_BUS)
            .unwrap_or(IO_OPEN_BUS)
    }

    fn cart_line_ties_q(&self) -> bool {
        self.slots[self.cts_slot()].cart_line_ties_q()
    }

    /// CART* follows the CTS slot select, same as [`MultiPak::rom_read`] and
    /// [`MultiPak::cart_line_ties_q`] — the three lines the MPI switches
    /// together (MAME `coco_multi.cpp` header comment).
    fn cart_interrupt(&mut self) -> bool {
        self.slots[self.cts_slot()].cart_interrupt()
    }

    /// Every slot's clock runs regardless of selection (MAME ticks all 4
    /// devices every call), so this advances all 4 rather than the
    /// selected one(s).
    fn tick(&mut self, cycles: u32) {
        for slot in &mut self.slots {
            slot.tick(cycles);
        }
    }

    /// Like [`Cartridge::tick`], audio clocks run in every slot regardless
    /// of selection — a sound chip's crystal doesn't stop when the slot
    /// isn't addressed — and the outputs wire-sum on the MPI's shared
    /// analog bus.
    fn generator_sample(&mut self, dt: f64) -> (f32, f32) {
        self.slots.iter_mut().fold((0.0, 0.0), |(l, r), slot| {
            let (sl, sr) = slot.generator_sample(dt);
            (l + sl, r + sr)
        })
    }

    /// Wire-OR of all 4 slots: any device, such as an FD-502 in a
    /// non-selected slot — can hold HALT* regardless of SCS/CTS selection.
    fn halt_asserted(&self) -> bool {
        self.slots.iter().any(|slot| slot.halt_asserted())
    }

    /// Polls (and consumes edges from) every slot, OR-ing the results —
    /// never short-circuits, so a pending edge in a later slot isn't left
    /// stranded behind an earlier slot's `false`.
    fn take_nmi(&mut self) -> bool {
        let mut any = false;
        for slot in &mut self.slots {
            if slot.take_nmi() {
                any = true;
            }
        }
        any
    }

    /// Wire-OR of all 4 slots, mirroring [`MultiPak::take_nmi`] but without
    /// consuming the edge.
    fn nmi_pending(&self) -> bool {
        self.slots.iter().any(|slot| slot.nmi_pending())
    }

    /// Sum of all 4 slots: the analog bus is common to every slot on a
    /// real MPI (only SCS*/CTS*/CART* are switched), so slot outputs mix on
    /// the wire regardless of selection.
    fn sound_levels(&self) -> (f32, f32) {
        self.slots.iter().fold((0.0, 0.0), |(l, r), slot| {
            let (sl, sr) = slot.sound_levels();
            (l + sl, r + sr)
        })
    }

    fn control_read(&mut self) -> u8 {
        self.select | mpi::READBACK_OR_MASK
    }

    fn control_write(&mut self, val: u8) {
        // A write replaces the entire byte — no nibble merge (spec).
        self.select = val;
        self.switch_blocked = true;
    }

    /// All 4 slots' audio outputs are wire-summed through the MPI's shared
    /// analog bus, same as a real passive backplane — every slot, not
    /// the SCS/CTS-selected one(s).
    fn audio_sample(&mut self) -> f32 {
        self.slots.iter_mut().map(|slot| slot.audio_sample()).sum()
    }

    /// Reloads `select` from the front-panel switch and lifts any software
    /// override (MAME `device_reset`), then forwards the reset to every
    /// slot's own cartridge — real hardware's RESET* line reaches the whole
    /// expansion bus, not the MPI itself.
    fn reset(&mut self) {
        self.select = mpi::SWITCH_VALUES[self.switch_slot];
        self.switch_blocked = false;
        for slot in &mut self.slots {
            slot.reset();
        }
    }

    /// Recurses into every slot — each slot's own cartridge rebuilds its own
    /// skipped scratch, if any.
    fn after_restore(&mut self) {
        for slot in &mut self.slots {
            slot.after_restore();
        }
    }

    /// Recurses into every slot, prefixing whichever slot's own check fails
    /// with its index — nesting itself (a `Cart::MultiPak` slot holding
    /// another `Cart::MultiPak`) is rejected earlier, structurally, by
    /// [`Cart::contains_nested_multipak`], before [`Cart::slots_mut`]/this
    /// walk ever runs.
    fn validate_restored(&self) -> Result<(), String> {
        for (i, slot) in self.slots.iter().enumerate() {
            slot.validate_restored()
                .map_err(|e| format!("Multi-Pak slot {i}: {e}"))?;
        }
        Ok(())
    }
}

/// Slot searches behind the [`Cart::as_disk_cart`]-family accessors: each
/// finds the device in whichever slot holds it. The frontend can also call
/// these directly on a [`MultiPak`] it already has a `&mut` to.
impl MultiPak {
    /// The FD-502 disk controller in any slot, if one is plugged in.
    pub fn find_disk_cart(&mut self) -> Option<&mut crate::fdc::DiskCart> {
        self.slots.iter_mut().find_map(Cart::as_disk_cart)
    }

    /// The Deluxe RS-232 pak in any slot, if one is plugged in.
    pub fn find_deluxe_rs232(&mut self) -> Option<&mut crate::rs232::DeluxeRS232> {
        self.slots.iter_mut().find_map(Cart::as_deluxe_rs232)
    }

    /// The Disto real-time clock in any slot, if one is plugged in.
    pub fn find_disto_rtc(&mut self) -> Option<&mut crate::rtc::DistoRTC> {
        self.slots.iter_mut().find_map(Cart::as_disto_rtc)
    }

    /// The Orchestra-90 in any slot, if one is plugged in.
    pub fn find_orch90(&mut self) -> Option<&mut crate::orch90::Orch90> {
        self.slots.iter_mut().find_map(Cart::as_orch90)
    }

    /// The Sound/Speech Cartridge in any slot, if one is plugged in.
    pub fn find_ssc(&mut self) -> Option<&mut crate::ssc::SoundSpeechCartridge> {
        self.slots.iter_mut().find_map(Cart::as_ssc)
    }

    /// The CoCo Max Hi-Res Input Module in any slot, if one is plugged in.
    pub fn find_cocomax(&mut self) -> Option<&mut super::cocomax::CoCoMaxModule> {
        self.slots.iter_mut().find_map(Cart::as_cocomax)
    }
}
