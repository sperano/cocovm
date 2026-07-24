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
    /// Read cartridge I/O: `$FF40–$FF5F` (the SCS* decode) plus
    /// `$FF60–$FF7E`, which real hardware doesn't strobe with SCS* but
    /// devices there (Deluxe RS-232, Orchestra-90) decode off the raw
    /// address bus — the port carries all 16 address lines.
    fn read(&mut self, addr: u16) -> u8;
    fn write(&mut self, addr: u16, val: u8);
    /// Read the external ROM window (the CTS* decode): `$C000–$FDFF`, or all
    /// of `$8000–$FDFF` when INIT0 MC1:MC0 selects the 32K-external map.
    fn rom_read(&mut self, _addr: u16) -> u8 {
        ROM_OPEN_BUS
    }
    /// Side-effect-free twin of [`Cartridge::rom_read`] for the debugger's
    /// disassembly/memory views ([`crate::SystemBus::peek`]). Overridden by
    /// cartridges whose ROM read is a pure array fetch (ROM paks, the FD-502
    /// controller ROM); the default is open bus so a device that can't read
    /// its ROM without side effects safely reports nothing rather than
    /// perturbing state.
    fn rom_peek(&self, _addr: u16) -> u8 {
        ROM_OPEN_BUS
    }
    /// Side-effect-free twin of [`Cartridge::read`] (the SCS* I/O window) for
    /// [`crate::SystemBus::peek`]. Defaults to open bus: most cartridge I/O
    /// reads (FDC status, etc.) mutate device state, so the debugger reports
    /// open bus rather than risk a side effect.
    fn peek(&self, _addr: u16) -> u8 {
        IO_OPEN_BUS
    }
    /// Side-effect-free twin of [`Cartridge::control_read`] (`$FF7F`) for
    /// [`crate::SystemBus::peek`].
    fn peek_control(&self) -> u8 {
        IO_OPEN_BUS
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
    /// Advance the cartridge's *audio* clocks by `dt` seconds of wall time.
    /// Separate from [`Cartridge::tick`] because sound chips (the GMC's
    /// SN76489A) run off their own crystal: a CPU-cycle timebase would let
    /// the GIME double-speed poke retune them. Called by the machine loop
    /// once per scanline, just before [`Cartridge::sound_level`] is sampled.
    fn audio_tick(&mut self, _dt: f64) {}
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
    /// Instantaneous audio level this cartridge drives onto the expansion
    /// port's analog SND pin, 0.0–1.0 (0.0 = silent, the default for carts
    /// with no audio hardware). Sampled by `SystemBus::sound_sample` once
    /// per scanline alongside the internal DAC/beeper sources; the caller
    /// applies its own gain before mixing.
    fn sound_level(&self) -> f32 {
        0.0
    }
    /// Side-effect-free peek at whether an NMI edge is currently latched,
    /// without consuming it (unlike [`Cartridge::take_nmi`]) — the
    /// debugger's hardware-state panel uses this to show the cart port's NMI
    /// line without perturbing the pending edge a running program still
    /// needs to see (`docs/plan-debugger.md` §3, "cart line states").
    fn nmi_pending(&self) -> bool {
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
    /// Downcast to the Orchestra-90, if that's what this cartridge is — how
    /// the frontend reads the DAC latches for its level meters behind the
    /// trait object.
    fn as_orch90(&mut self) -> Option<&mut crate::orch90::Orch90> {
        None
    }
    /// Downcast to the [`crate::ssc::Ssc`] Sound/Speech Cartridge, if that's
    /// what this cartridge is — mirrors [`Cartridge::as_disk_cart`]/
    /// [`Cartridge::as_multipak`]: tests and any future debug tooling reach
    /// direct AY-3-8913 register access
    /// ([`crate::ssc::Ssc::ay_write`]/[`crate::ssc::Ssc::ay_read`]) behind
    /// the trait object, bypassing the `$FF7D`/`$FF7E` host-byte protocol
    /// (see `crate::ssc`'s module doc comment).
    fn as_ssc(&mut self) -> Option<&mut crate::ssc::Ssc> {
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
    /// The cartridge's own analog audio output for this sample tick, in the
    /// same amplitude convention as
    /// [`SystemBus::sound_sample`](crate::bus::SystemBus::sound_sample) (0.0
    /// silence, full scale comparable to that method's other sources).
    /// Default: silent — most cartridges (ROM paks, the FD-502, the VHD
    /// interface) have no audio output of their own. The Sound/Speech
    /// Cartridge ([`crate::ssc::Ssc`]) is the one device that overrides
    /// this.
    ///
    /// Called exactly once per `sound_sample`, regardless of whether the
    /// CoCo's sound mux currently selects the cartridge input: some devices
    /// (the SSC's Sound Activity Circuit) need to observe their own output
    /// continuously, independent of what's actually reaching the speaker.
    /// Takes `&mut self` because that continuous observation is itself
    /// stateful (an envelope follower).
    fn audio_sample(&mut self) -> f32 {
        0.0
    }
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

        Ok(Self {
            image: mirror_fill(bytes, ROM_PAK_MAX_LEN),
            autostart,
        })
    }
}

/// Copy `bytes` into a `total_len` buffer and mirror-fill the rest with MAME
/// `cococart_slot_device::call_load`'s doubling loop:
///   while read_length < cart_length {
///       len = min(read_length, cart_length - read_length);
///       copy buffer[0..len] to buffer[read_length..];
///       read_length += len;
///   }
/// Each copy lands at a multiple of the image length and copies a prefix of
/// an already-periodic buffer, so the result is byte-identical to plain
/// repetition (`image[i % len]`) for every image size — the doubling is only
/// an efficiency trick, kept in MAME's shape so the provenance is obvious.
/// The slot device runs this same loop for every pak type, so [`RomPak`]
/// (32K) and [`BankedRomPak`] (128K) share it.
fn mirror_fill(bytes: &[u8], total_len: usize) -> Box<[u8]> {
    let mut image = vec![0u8; total_len].into_boxed_slice();
    image[..bytes.len()].copy_from_slice(bytes);
    let mut read_length = bytes.len();
    while read_length < total_len {
        let len = read_length.min(total_len - read_length);
        let (src, dst) = image.split_at_mut(read_length);
        dst[..len].copy_from_slice(&src[..len]);
        read_length += len;
    }
    image
}

impl Cartridge for RomPak {
    fn read(&mut self, _addr: u16) -> u8 {
        IO_OPEN_BUS
    }
    fn write(&mut self, _addr: u16, _val: u8) {}
    fn rom_read(&mut self, addr: u16) -> u8 {
        self.image[((addr - ROM_PAK_BASE) ^ ROM_PAK_HALF_SWAP) as usize]
    }
    fn rom_peek(&self, addr: u16) -> u8 {
        // Pure array fetch — identical to `rom_read`, no side effects.
        self.image[((addr - ROM_PAK_BASE) ^ ROM_PAK_HALF_SWAP) as usize]
    }
    fn cart_line_ties_q(&self) -> bool {
        self.autostart
    }
}

/// The banked pak's CTS window: the `$FF40` latch slides a **16K** view over
/// the image (MAME `coco_pak.cpp` `coco_pak_banked_device::get_cart_size()`
/// = `0x4000`), unlike the plain [`RomPak`]'s fixed 32K.
pub const BANKED_PAK_WINDOW_LEN: usize = 16 * 1024;

/// Largest banked-pak image: MAME's banked cart ROM region is 128K
/// (`coco_pak.cpp` `ROM_REGION(0x20000, ...)`), 8 banks of 16K. The bank
/// latch wraps modulo this, so undersized images (mirror-filled to 128K,
/// like every pak) repeat across the unused banks.
pub const BANKED_PAK_MAX_LEN: usize = 128 * 1024;

/// The bank latch address (MAME `coco_pak_banked_device::scs_write` case 0
/// of the SCS window).
const BANKED_PAK_BANK_REG: u16 = 0xFF40;

/// Error constructing a [`BankedRomPak`] from a raw image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BankedPakError {
    /// The image had zero bytes.
    Empty,
    /// The image exceeded [`BANKED_PAK_MAX_LEN`].
    TooLarge {
        /// The image's actual length, in bytes.
        len: usize,
    },
}

impl std::fmt::Display for BankedPakError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BankedPakError::Empty => write!(f, "banked ROM pak image is empty"),
            BankedPakError::TooLarge { len } => write!(
                f,
                "banked ROM pak image is {len} bytes, larger than the {BANKED_PAK_MAX_LEN}-byte banked address space"
            ),
        }
    }
}

impl std::error::Error for BankedPakError {}

/// A banked ROM pak: the RoboCop/Predator bank-switch circuit that the Games
/// Master Cartridge reuses (MAME `coco_pak.cpp` `coco_pak_banked_device`).
/// A whole-byte write to `$FF40` selects which 16K page of the (up to 128K)
/// image the external ROM window shows; the effective bank is
/// `(latch * 16K) mod 128K` (MAME `cts_read`'s
/// `(m_pos * 0x4000) % m_eprom->bytes()`), and the latch resets to bank 0 on
/// the RESET* line (`device_reset`).
pub struct BankedRomPak {
    image: Box<[u8]>,
    /// The raw `$FF40` latch byte (`m_pos`) — masking happens at read time,
    /// via the modulo above.
    bank: u8,
    /// Whether this pak ties the CART* line to Q (see
    /// [`Cartridge::cart_line_ties_q`]).
    autostart: bool,
}

impl std::fmt::Debug for BankedRomPak {
    /// Elides the 128K image body.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BankedRomPak")
            .field("bank", &self.bank)
            .field("autostart", &self.autostart)
            .finish()
    }
}

impl BankedRomPak {
    /// Build a banked ROM pak from a raw image. Rejects empty images and
    /// images larger than [`BANKED_PAK_MAX_LEN`]; anything in between is
    /// mirror-filled to the full 128K (MAME loads every pak type through the
    /// same doubling loop — see [`mirror_fill`]).
    pub fn from_bytes(bytes: &[u8], autostart: bool) -> Result<Self, BankedPakError> {
        if bytes.is_empty() {
            return Err(BankedPakError::Empty);
        }
        if bytes.len() > BANKED_PAK_MAX_LEN {
            return Err(BankedPakError::TooLarge { len: bytes.len() });
        }
        Ok(Self {
            image: mirror_fill(bytes, BANKED_PAK_MAX_LEN),
            bank: 0,
            autostart,
        })
    }
}

impl Cartridge for BankedRomPak {
    fn read(&mut self, _addr: u16) -> u8 {
        IO_OPEN_BUS
    }
    fn write(&mut self, addr: u16, val: u8) {
        if addr == BANKED_PAK_BANK_REG {
            self.bank = val;
        }
    }
    fn rom_read(&mut self, addr: u16) -> u8 {
        // The 16K window mirrors identically into both halves of the 32K
        // external map, so the GIME half-swap (see [`ROM_PAK_HALF_SWAP`])
        // flips a bit the window mask discards — it drops out entirely.
        let window = usize::from(addr - ROM_PAK_BASE) & (BANKED_PAK_WINDOW_LEN - 1);
        let base = usize::from(self.bank) * BANKED_PAK_WINDOW_LEN % BANKED_PAK_MAX_LEN;
        self.image[base + window]
    }
    fn cart_line_ties_q(&self) -> bool {
        self.autostart
    }
    fn reset(&mut self) {
        self.bank = 0;
    }
}

/// The SN76489A data port: `$FF41` (MAME `coco_gmc.cpp` `scs_write` case 1).
///
/// ⚠ This address collides with the DriveWire Becker port's data register.
/// MAME resolves it by intercepting Becker *ahead* of the cartridge decode,
/// shadowing the GMC's PSG; when the Becker port lands here
/// (`docs/plan-drivewire-becker.md`), the bus must keep that precedence and
/// the UI must refuse to enable both at once.
const GMC_PSG_REG: u16 = 0xFF41;

/// The PSG's crystal on the GMC: 4 MHz (MAME `coco_gmc.cpp`
/// `SN76489A(config, m_psg, 4_MHz_XTAL)`).
const GMC_PSG_CRYSTAL_HZ: f64 = 4_000_000.0;

/// John Linville's Games Master Cartridge (MAME `coco_gmc.cpp`): a
/// [`BankedRomPak`] plus a TI SN76489A PSG for game music. `$FF40` is the
/// ROM bank latch (inherited from the banked pak), `$FF41` writes the PSG's
/// single command port; nothing is readable back (MAME's `scs_read` is the
/// do-nothing base), so reads stay at the I/O window's open-bus value.
///
/// Audio mixes into the speaker unconditionally, not through the analog
/// mux's SEL=10 cartridge-sound input: MAME routes the GMC's PSG to a
/// dedicated speaker device ignoring SNDEN entirely (its mux cart-sound path
/// is an explicit "NYI" stub), and no independent schematic settles what the
/// real cart's SND-pin wiring expects — so we keep MAME's behaviour.
pub struct Gmc {
    rom: BankedRomPak,
    psg: crate::sn76489::SN76489A,
    /// Mean PSG level over the last [`Cartridge::audio_tick`] interval —
    /// what [`Cartridge::sound_level`] reports.
    level: f32,
}

impl std::fmt::Debug for Gmc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Gmc")
            .field("rom", &self.rom)
            .field("psg", &self.psg)
            .finish()
    }
}

impl Gmc {
    /// Build a GMC from a raw banked-ROM image (same size rules as
    /// [`BankedRomPak::from_bytes`]).
    pub fn from_bytes(bytes: &[u8], autostart: bool) -> Result<Self, BankedPakError> {
        Ok(Self {
            rom: BankedRomPak::from_bytes(bytes, autostart)?,
            psg: crate::sn76489::SN76489A::new(GMC_PSG_CRYSTAL_HZ),
            level: 0.0,
        })
    }
}

impl Cartridge for Gmc {
    fn read(&mut self, _addr: u16) -> u8 {
        IO_OPEN_BUS
    }
    fn write(&mut self, addr: u16, val: u8) {
        match addr {
            GMC_PSG_REG => self.psg.write(val),
            _ => self.rom.write(addr, val),
        }
    }
    fn rom_read(&mut self, addr: u16) -> u8 {
        self.rom.rom_read(addr)
    }
    fn cart_line_ties_q(&self) -> bool {
        self.rom.cart_line_ties_q()
    }
    fn audio_tick(&mut self, dt: f64) {
        self.level = self.psg.sample(dt);
    }
    fn sound_level(&self) -> f32 {
        self.level
    }
    /// Only the bank latch resets — the SN76489A has no reset pin, so the
    /// PSG plays on through a warm reset until software reprograms it, as on
    /// the real cartridge.
    fn reset(&mut self) {
        self.rom.reset();
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

/// Standard SCS* window (`$FF40-$FF5F`): routed only to the SCS-selected
/// slot, same as `CART*`/`CTS*` follow the CTS-selected slot. The `$FF60-
/// $FF7E` extension some carts decode (`docs/cartridges.md` "Carts can
/// decode addresses outside SCS") is NOT switched by the MPI — the address
/// and data buses are common to every slot, only SCS*/CTS*/CART* are
/// per-slot — so it's handled separately below.
const SCS_BASE: u16 = 0xFF40;
const SCS_LAST: u16 = 0xFF5F;

impl Cartridge for MultiPak {
    /// Routes the whole I/O window ($FF40-$FF5F and $FF60-$FF7E) to the
    /// SCS-selected slot. Approximation for the $FF60-$FF7E part: a real MPI
    /// switches only SCS*, so a device decoding raw addresses there (e.g. an
    /// Orchestra-90) responds from any slot — here it must be the SCS slot.
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

    fn peek(&self, addr: u16) -> u8 {
        self.slots[self.scs_slot()].peek(addr)
    }

    fn peek_control(&self) -> u8 {
        self.select | mpi::READBACK_OR_MASK
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

    /// Like [`Cartridge::tick`], audio clocks run in every slot regardless
    /// of selection — a sound chip's crystal doesn't stop when the slot
    /// isn't addressed.
    fn audio_tick(&mut self, dt: f64) {
        for slot in &mut self.slots {
            slot.audio_tick(dt);
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

    /// Wire-OR of all 4 slots, mirroring [`MultiPak::take_nmi`] but without
    /// consuming the edge.
    fn nmi_pending(&self) -> bool {
        self.slots.iter().any(|slot| slot.nmi_pending())
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

    fn as_orch90(&mut self) -> Option<&mut crate::orch90::Orch90> {
        self.slots.iter_mut().find_map(|slot| slot.as_orch90())
    }

    /// Sum of all 4 slots: the analog SND pin is common to every slot on a
    /// real MPI (only SCS*/CTS*/CART* are switched), so slot outputs mix on
    /// the wire regardless of selection.
    fn sound_level(&self) -> f32 {
        self.slots.iter().map(|slot| slot.sound_level()).sum()
    }

    fn as_ssc(&mut self) -> Option<&mut crate::ssc::Ssc> {
        self.slots.iter_mut().find_map(|slot| slot.as_ssc())
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
    /// analog bus, same as a real passive backplane — every slot, not just
    /// the SCS/CTS-selected one(s).
    fn audio_sample(&mut self) -> f32 {
        self.slots.iter_mut().map(|slot| slot.audio_sample()).sum()
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
