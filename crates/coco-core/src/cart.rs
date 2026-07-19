//! Cartridge port devices (FDC, ROM pack, Multi-Pak). See `DESIGN.md` §7.
//!
//! NOTE: a trait object (`Box<dyn Cartridge>`) is not `Serialize`, so `SystemBus`
//! and `Machine` don't yet derive serde. For save-states (`DESIGN.md` §9) this
//! becomes an enum or uses typetag — deferred.

/// Value read from the external ROM window when nothing drives the bus:
/// $00, matching real hardware / MAME coco3 (verified by MAME trace-diff,
/// 2026-07-02 — an empty slot's `LDD $C000` yields $0000, not $FFFF).
pub const ROM_OPEN_BUS: u8 = 0x00;

/// Value read from the cartridge I/O window ($FF40–$FF5F, SCS*) when nothing
/// drives the bus: floats high, like an unstrobed PIA input pin.
pub const IO_OPEN_BUS: u8 = 0xFF;

/// A device on the cartridge port.
pub trait Cartridge {
    /// Read cartridge I/O ($FF40–$FF5F, the SCS* decode).
    fn read(&mut self, addr: u16) -> u8;
    fn write(&mut self, addr: u16, val: u8);
    /// Read the external ROM window (the CTS* decode): `$C000–$FDFF`, or all
    /// of `$8000–$FDFF` when INIT0 MC1:MC0 selects the 32K-external map.
    fn rom_read(&mut self, _addr: u16) -> u8 {
        ROM_OPEN_BUS
    }
    /// True while this cartridge ties the expansion-port CART* line to the Q
    /// clock (~895 kHz): auto-start game paks do this so edges arrive
    /// continuously, which drives both the legacy PIA1 CB1 FIRQ path and the
    /// GIME EI0 input the same physical pin feeds. `SystemBus::hsync` polls
    /// this once per scanline — plenty to model a ~895 kHz signal, since the
    /// PIA/GIME only care that *an* edge keeps arriving. Disk-BASIC-style
    /// paks (no autostart) leave this false; they rely on BASIC's cold-start
    /// probe of `$C000`/`$C001` for `'D'`,`'K'` instead.
    fn cart_line_ties_q(&self) -> bool {
        false
    }
    /// Current level of the CART* interrupt line as driven by the device:
    /// true = asserted (the active-low pin held low). The level counterpart
    /// to [`Cartridge::cart_line_ties_q`]'s Q-burst: a cartridge with a real
    /// interrupt source — the Deluxe RS-232's 6551 ACIA IRQ output — holds
    /// this while its interrupt condition stands, and
    /// `SystemBus::poll_cart_interrupt` converts the transitions into the
    /// PIA1 CB1 edge and GIME EI0 raise that the shared physical pin feeds.
    /// Default: never asserted.
    fn cart_interrupt(&mut self) -> bool {
        false
    }
    /// Advance the cartridge's internal clocks by `cycles` CPU cycles. Called
    /// by the machine loop after every instruction (and once per burned cycle
    /// while the CPU is halted, so a device can pace work — the FDC's DRQ
    /// cadence — while it holds the HALT line).
    fn tick(&mut self, _cycles: u32) {}
    /// True while the cartridge holds the CPU HALT* line low (the FD-502's
    /// transfer handshake). Sampled at instruction boundaries.
    fn halt_asserted(&self) -> bool {
        false
    }
    /// Consume a pending NMI edge from the cartridge (the FD-502 gates the
    /// FDC's INTRQ onto the CPU NMI). Returns true at most once per edge.
    fn take_nmi(&mut self) -> bool {
        false
    }
    /// Downcast to the FD-502 disk controller, if that's what this cartridge
    /// is — how the frontend reaches drive slots (insert/eject a floppy while
    /// the machine runs, as on real hardware) behind the trait object.
    fn as_disk_cart(&mut self) -> Option<&mut crate::fdc::DiskCart> {
        None
    }
    /// Downcast to the [`MultiPak`], if that's what this cartridge is — how
    /// the frontend reaches individual slots (insert/eject/switch) behind the
    /// trait object.
    fn as_multipak(&mut self) -> Option<&mut MultiPak> {
        None
    }
    /// Downcast to the Deluxe RS-232 pak, if that's what this cartridge is —
    /// how the frontend swaps host endpoints and reads the TX/RX activity
    /// counters behind the trait object (same pattern as
    /// [`Cartridge::as_disk_cart`]).
    fn as_deluxe_rs232(&mut self) -> Option<&mut crate::rs232::DeluxeRs232> {
        None
    }
    /// Downcast to the Disto real-time clock, if that's what this cartridge
    /// is — how the frontend reaches the clock chip (sync to host time)
    /// behind the trait object.
    fn as_disto_rtc(&mut self) -> Option<&mut crate::rtc::DistoRtc> {
        None
    }
    /// Read the Multi-Pak Interface's own select register (`$FF7F`). Not
    /// routed through [`Cartridge::read`]/[`Cartridge::write`]: those carry
    /// the SCS I/O window ($FF40-$FF5F), and `$FF7F` must reach the MPI
    /// itself even when a `DiskCart` (whose own register decode could alias
    /// it) is plugged into the MPI's selected SCS slot. Every cartridge other
    /// than [`MultiPak`] leaves this at the default open-bus/no-op.
    fn control_read(&mut self) -> u8 {
        IO_OPEN_BUS
    }
    /// Write the Multi-Pak Interface's own select register (`$FF7F`). See
    /// [`Cartridge::control_read`].
    fn control_write(&mut self, _val: u8) {}
    /// Re-run this cartridge's reset sequence (the CoCo's RESET* line, which
    /// the expansion port shares). [`MultiPak`] reloads its select register
    /// from the front-panel switch and forwards the reset to all 4 slots;
    /// every other cartridge has nothing reset-sensitive to do.
    fn reset(&mut self) {}
}

/// No cartridge inserted.
#[derive(Debug, Clone, Default)]
pub struct EmptySlot;

impl Cartridge for EmptySlot {
    fn read(&mut self, _addr: u16) -> u8 {
        IO_OPEN_BUS
    }
    fn write(&mut self, _addr: u16, _val: u8) {}
}

/// Largest image a ROM pak can hold: the external ROM window is 32K
/// (`$8000–$FFFF` under INIT0 MC1:MC0 = `11`).
pub const ROM_PAK_MAX_LEN: usize = 32 * 1024;

/// Base of the logical addresses `rom_read` receives (`$8000-$FDFF`).
const ROM_PAK_BASE: u16 = 0x8000;

/// Pak images are dumped CTS-window-first: file offset 0 is the byte at
/// `$C000`, and (for 32K carts) offset `$4000` is the byte at `$8000` once
/// INIT0 MC1:MC0 = `11` maps the second half in. The GIME routes cart banks
/// as `((bank & 3) ^ 2) * 0x2000` (MAME `gime.cpp` `update_memory`), i.e. the
/// two 16K halves are swapped relative to a flat `addr - $8000` view, so we
/// XOR the half-select bit when indexing. Invisible for ≤16K paks (the
/// mirror-fill makes both halves identical); load-bearing for 32K carts like
/// Arkanoid, whose entry code lives at file offset 0 expecting to be fetched
/// at `$C000`.
const ROM_PAK_HALF_SWAP: u16 = 0x4000;

/// Error constructing a [`RomPak`] from a raw image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RomPakError {
    /// The image had zero bytes.
    Empty,
    /// The image exceeded [`ROM_PAK_MAX_LEN`].
    TooLarge {
        /// The image's actual length, in bytes.
        len: usize,
    },
}

impl std::fmt::Display for RomPakError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RomPakError::Empty => write!(f, "ROM pak image is empty"),
            RomPakError::TooLarge { len } => write!(
                f,
                "ROM pak image is {len} bytes, larger than the {ROM_PAK_MAX_LEN}-byte external ROM window"
            ),
        }
    }
}

impl std::error::Error for RomPakError {}

/// A cartridge ROM pak: a headerless raw dump (`.rom`/`.ccc`/`.bin`, 2K–32K) of
/// the kind sold for CoCo game/utility cartridges.
///
/// Undersized images are mirror-filled to the full 32K exactly as MAME's
/// `coco_pak_device::call_load` does, so `rom_read` needs no bounds logic
/// regardless of the original image size (fact 5). Indexing swaps the 16K
/// halves — see [`ROM_PAK_HALF_SWAP`].
pub struct RomPak {
    image: Box<[u8]>,
    /// Whether this pak ties the CART* line to Q (see
    /// [`Cartridge::cart_line_ties_q`]).
    autostart: bool,
}

impl std::fmt::Debug for RomPak {
    /// Elides the 32K image body; only its length and the autostart flag are
    /// interesting for debugging.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RomPak")
            .field("image_len", &self.image.len())
            .field("autostart", &self.autostart)
            .finish()
    }
}

impl RomPak {
    /// Build a ROM pak from a raw image. Rejects empty images and images
    /// larger than [`ROM_PAK_MAX_LEN`]; anything in between is mirror-filled
    /// to 32K.
    pub fn from_bytes(bytes: &[u8], autostart: bool) -> Result<Self, RomPakError> {
        if bytes.is_empty() {
            return Err(RomPakError::Empty);
        }
        if bytes.len() > ROM_PAK_MAX_LEN {
            return Err(RomPakError::TooLarge { len: bytes.len() });
        }

        let mut image = vec![0u8; ROM_PAK_MAX_LEN].into_boxed_slice();
        image[..bytes.len()].copy_from_slice(bytes);

        // Mirror-fill the rest with MAME `coco_pak_device`'s doubling loop:
        //   while read_length < cart_length {
        //       len = min(read_length, cart_length - read_length);
        //       copy buffer[0..len] to buffer[read_length..];
        //       read_length += len;
        //   }
        // Each copy lands at a multiple of the image length and copies a
        // prefix of an already-periodic buffer, so the result is byte-identical
        // to plain repetition (`image[i % len]`) for every image size — the
        // doubling is only an efficiency trick, kept in MAME's shape so the
        // provenance is obvious.
        let mut read_length = bytes.len();
        while read_length < ROM_PAK_MAX_LEN {
            let len = read_length.min(ROM_PAK_MAX_LEN - read_length);
            let (src, dst) = image.split_at_mut(read_length);
            dst[..len].copy_from_slice(&src[..len]);
            read_length += len;
        }

        Ok(Self { image, autostart })
    }
}

impl Cartridge for RomPak {
    fn read(&mut self, _addr: u16) -> u8 {
        IO_OPEN_BUS
    }
    fn write(&mut self, _addr: u16, _val: u8) {}
    fn rom_read(&mut self, addr: u16) -> u8 {
        self.image[((addr - ROM_PAK_BASE) ^ ROM_PAK_HALF_SWAP) as usize]
    }
    fn cart_line_ties_q(&self) -> bool {
        self.autostart
    }
}

/// Tandy Multi-Pak Interface (MPI, 26-3024): a 4-slot passive expansion
/// adapter for the cartridge port. Facts below are verified against MAME
/// `src/devices/bus/coco/coco_multi.cpp` (`coco_multipak_device`) and the
/// Lomont CoCo Hardware reference.
///
/// All expansion-port lines are shared across the 4 slots except SCS*, CTS*,
/// and CART* (MAME's `coco_multi.cpp` header comment): those three follow the
/// select register below, while `halt_asserted`/`take_nmi`/`tick` reach every
/// slot regardless of selection (a device doesn't stop just because it isn't
/// currently addressed).
///
/// Two MAME facts are deliberately NOT modeled here, per spec: the CoCo 3
/// never delivers external-ROM-window *writes* to cartridges at all (already
/// true of `SystemBus`, independent of the MPI), and the field-mod some real
/// MPIs have that ties all 4 slots' CART* lines together (a hardware hack,
/// not stock behaviour) is not reproduced — CART* here strictly follows the
/// CTS select, as spec'd.
pub struct MultiPak {
    slots: [Box<dyn Cartridge>; mpi::SLOT_COUNT],
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
            slots: [
                Box::new(EmptySlot),
                Box::new(EmptySlot),
                Box::new(EmptySlot),
                Box::new(EmptySlot),
            ],
            select: mpi::SWITCH_VALUES[switch_slot],
            switch_slot,
            switch_blocked: false,
        }
    }

    /// Plug a cartridge into `slot` (0-3).
    pub fn insert(&mut self, slot: usize, cart: Box<dyn Cartridge>) {
        self.slots[slot] = cart;
    }

    /// Remove whatever is in `slot`, restoring the empty slot.
    pub fn eject(&mut self, slot: usize) {
        self.slots[slot] = Box::new(EmptySlot);
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

impl Cartridge for MultiPak {
    fn read(&mut self, addr: u16) -> u8 {
        self.slots[self.scs_slot()].read(addr)
    }

    fn write(&mut self, addr: u16, val: u8) {
        self.slots[self.scs_slot()].write(addr, val);
    }

    fn rom_read(&mut self, addr: u16) -> u8 {
        self.slots[self.cts_slot()].rom_read(addr)
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
    /// devices every call), so this advances all 4 rather than just the
    /// selected one(s).
    fn tick(&mut self, cycles: u32) {
        for slot in &mut self.slots {
            slot.tick(cycles);
        }
    }

    /// Wire-OR of all 4 slots: any device — e.g. an FD-502 in a
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

    fn as_disk_cart(&mut self) -> Option<&mut crate::fdc::DiskCart> {
        self.slots.iter_mut().find_map(|slot| slot.as_disk_cart())
    }

    fn as_multipak(&mut self) -> Option<&mut MultiPak> {
        Some(self)
    }

    fn as_deluxe_rs232(&mut self) -> Option<&mut crate::rs232::DeluxeRs232> {
        self.slots
            .iter_mut()
            .find_map(|slot| slot.as_deluxe_rs232())
    }

    fn as_disto_rtc(&mut self) -> Option<&mut crate::rtc::DistoRtc> {
        self.slots.iter_mut().find_map(|slot| slot.as_disto_rtc())
    }

    fn control_read(&mut self) -> u8 {
        self.select | mpi::READBACK_OR_MASK
    }

    fn control_write(&mut self, val: u8) {
        // A write replaces the entire byte — no nibble merge (spec).
        self.select = val;
        self.switch_blocked = true;
    }

    /// Reloads `select` from the front-panel switch and lifts any software
    /// override (MAME `device_reset`), then forwards the reset to every
    /// slot's own cartridge — real hardware's RESET* line reaches the whole
    /// expansion bus, not just the MPI itself.
    fn reset(&mut self) {
        self.select = mpi::SWITCH_VALUES[self.switch_slot];
        self.switch_blocked = false;
        for slot in &mut self.slots {
            slot.reset();
        }
    }
}
