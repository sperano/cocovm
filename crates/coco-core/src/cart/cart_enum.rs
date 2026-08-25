//! The [`Cart`] closed enum: dispatch, save-state plumbing, and the
//! trait-object-downcast-style accessors the frontend uses to reach a
//! specific cartridge type wherever it's plugged in.

use serde::{Deserialize, Serialize};

use super::Cartridge;
use super::empty::EmptySlot;
use super::gmc::GamesMasterCartridge;
use super::multipak::MultiPak;
use super::rompak::{BankedROMPak, ROMPak};

/// The cartridge in the expansion port (or in a [`MultiPak`] slot), as a
/// closed enum over every in-crate cartridge type. An enum rather than a
/// `Box<dyn Cartridge>` so `SystemBus`/`Machine` can derive serde for
/// save-states (`DESIGN.md` §9); dispatch to the concrete type is static
/// (see `with_each_cart!`) and the old trait-object downcast methods are
/// plain `match`es here ([`Cart::as_disk_cart`] etc.).
///
/// [`Cart::Custom`] is the one open variant: a boxed trait object for
/// out-of-crate [`Cartridge`] implementations (integration-test doubles).
/// It is excluded from serialization — a save-state with a custom cartridge
/// inserted is an error, which only test rigs can hit.
#[non_exhaustive]
#[derive(Serialize, Deserialize)]
pub enum Cart {
    /// Nothing in the port ([`EmptySlot`]).
    Empty(EmptySlot),
    /// Plain (up to 32K) game/utility ROM pak.
    ROMPak(ROMPak),
    /// RoboCop/Predator-style banked ROM pak.
    BankedROMPak(BankedROMPak),
    /// Games Master Cartridge: banked ROM + SN76489A PSG.
    GamesMasterCartridge(GamesMasterCartridge),
    /// FD-502 floppy disk controller. Boxed for size, like [`Cart::SoundSpeechCartridge`].
    DiskCart(Box<crate::fdc::DiskCart>),
    /// Multi-Pak Interface. Boxed to break the size recursion — a
    /// [`MultiPak`] holds four [`Cart`] slots of its own.
    MultiPak(Box<MultiPak>),
    /// Orchestra-90/CC stereo DAC cartridge.
    Orch90(crate::orch90::Orch90),
    /// Disto real-time clock.
    DistoRTC(crate::rtc::DistoRTC),
    /// Deluxe RS-232 Program Pak.
    DeluxeRS232(crate::rs232::DeluxeRS232),
    /// Sound/Speech Cartridge. Boxed for size: the AY + speech-engine state
    /// is by far the largest cartridge (clippy `large_enum_variant`).
    SoundSpeechCartridge(Box<crate::ssc::SoundSpeechCartridge>),
    /// Out-of-crate [`Cartridge`] implementation (test doubles) — see the
    /// type-level doc. Skipped: a boxed trait object has no serializable
    /// shape, so serializing a machine with one inserted is a hard error
    /// (`serde`'s generated `Err` for a skipped variant, not a panic) —
    /// documented intent, since only
    /// integration-test rigs can ever hit this variant.
    #[serde(skip)]
    Custom(Box<dyn Cartridge>),
}

/// Dispatch `$body` over the payload of every [`Cart`] variant — the match
/// each delegation method below expands to. Every arm resolves the
/// [`Cartridge`] method on the concrete type (static dispatch); only the
/// `Custom` arm stays a virtual call through the box.
macro_rules! with_each_cart {
    ($self:expr, $cart:ident => $body:expr) => {
        match $self {
            Cart::Empty($cart) => $body,
            Cart::ROMPak($cart) => $body,
            Cart::BankedROMPak($cart) => $body,
            Cart::GamesMasterCartridge($cart) => $body,
            Cart::DiskCart($cart) => $body,
            Cart::MultiPak($cart) => $body,
            Cart::Orch90($cart) => $body,
            Cart::DistoRTC($cart) => $body,
            Cart::DeluxeRS232($cart) => $body,
            Cart::SoundSpeechCartridge($cart) => $body,
            Cart::Custom($cart) => $body,
        }
    };
}

/// Delegation to the variant's [`Cartridge`] implementation — each method
/// here is the enum face of the same-named trait method; see the trait for
/// semantics.
impl Cart {
    /// See [`Cartridge::read`].
    pub fn read(&mut self, addr: u16) -> u8 {
        with_each_cart!(self, cart => cart.read(addr))
    }
    /// See [`Cartridge::write`].
    pub fn write(&mut self, addr: u16, val: u8) {
        with_each_cart!(self, cart => cart.write(addr, val))
    }
    /// See [`Cartridge::rom_read`].
    pub fn rom_read(&mut self, addr: u16) -> u8 {
        with_each_cart!(self, cart => cart.rom_read(addr))
    }
    /// See [`Cartridge::rom_peek`].
    pub fn rom_peek(&self, addr: u16) -> u8 {
        with_each_cart!(self, cart => cart.rom_peek(addr))
    }
    /// See [`Cartridge::peek`].
    pub fn peek(&self, addr: u16) -> u8 {
        with_each_cart!(self, cart => cart.peek(addr))
    }
    /// See [`Cartridge::peek_control`].
    pub fn peek_control(&self) -> u8 {
        with_each_cart!(self, cart => cart.peek_control())
    }
    /// See [`Cartridge::cart_line_ties_q`].
    pub fn cart_line_ties_q(&self) -> bool {
        with_each_cart!(self, cart => cart.cart_line_ties_q())
    }
    /// See [`Cartridge::cart_interrupt`].
    pub fn cart_interrupt(&mut self) -> bool {
        with_each_cart!(self, cart => cart.cart_interrupt())
    }
    /// See [`Cartridge::tick`].
    pub fn tick(&mut self, cycles: u32) {
        with_each_cart!(self, cart => cart.tick(cycles))
    }
    /// See [`Cartridge::generator_sample`].
    pub fn generator_sample(&mut self, dt: f64) -> (f32, f32) {
        with_each_cart!(self, cart => cart.generator_sample(dt))
    }
    /// See [`Cartridge::halt_asserted`].
    pub fn halt_asserted(&self) -> bool {
        with_each_cart!(self, cart => cart.halt_asserted())
    }
    /// See [`Cartridge::take_nmi`].
    pub fn take_nmi(&mut self) -> bool {
        with_each_cart!(self, cart => cart.take_nmi())
    }
    /// See [`Cartridge::nmi_pending`].
    pub fn nmi_pending(&self) -> bool {
        with_each_cart!(self, cart => cart.nmi_pending())
    }
    /// See [`Cartridge::sound_levels`].
    pub fn sound_levels(&self) -> (f32, f32) {
        with_each_cart!(self, cart => cart.sound_levels())
    }
    /// See [`Cartridge::audio_sample`].
    pub fn audio_sample(&mut self) -> f32 {
        with_each_cart!(self, cart => cart.audio_sample())
    }
    /// See [`Cartridge::control_read`].
    pub fn control_read(&mut self) -> u8 {
        with_each_cart!(self, cart => cart.control_read())
    }
    /// See [`Cartridge::control_write`].
    pub fn control_write(&mut self, val: u8) {
        with_each_cart!(self, cart => cart.control_write(val))
    }
    /// See [`Cartridge::reset`].
    pub fn reset(&mut self) {
        with_each_cart!(self, cart => cart.reset())
    }
    /// See [`Cartridge::after_restore`]. Called by
    /// [`crate::SystemBus::after_restore`] after a snapshot round-trip.
    pub fn after_restore(&mut self) {
        with_each_cart!(self, cart => cart.after_restore())
    }
    /// See [`Cartridge::validate_restored`]. Called by
    /// [`crate::snapshot::restore::validate_payload_shape`] against the whole cart tree.
    pub fn validate_restored(&self) -> Result<(), String> {
        with_each_cart!(self, cart => cart.validate_restored())
    }
}

/// Generates one device accessor below: the `$variant` payload if that's
/// what this cart is, else searching a [`MultiPak`]'s slots through its
/// paired `MultiPak::$finder`. The `let` rebind deref-coerces boxed payloads
/// ([`Cart::DiskCart`], [`Cart::SoundSpeechCartridge`]) and reborrows plain ones alike.
macro_rules! cart_accessor {
    ($(#[$doc:meta])* $name:ident, $finder:ident, $variant:ident, $ty:ty) => {
        $(#[$doc])*
        pub fn $name(&mut self) -> Option<&mut $ty> {
            match self {
                Cart::$variant(cart) => {
                    let cart: &mut $ty = cart;
                    Some(cart)
                }
                Cart::MultiPak(mp) => mp.$finder(),
                _ => None,
            }
        }
    };
}

/// The old `Cartridge` trait-object downcasts, now plain matches. Each
/// searches through a [`MultiPak`]'s slots too, so the frontend reaches a
/// device the same way whether it sits in the port directly or in an MPI
/// slot.
impl Cart {
    /// Wrap an out-of-crate [`Cartridge`] implementation (a test double) in
    /// the [`Cart::Custom`] variant.
    pub fn custom(cart: impl Cartridge + 'static) -> Self {
        Cart::Custom(Box::new(cart))
    }

    /// The [`MultiPak`], if that's what is inserted. Not recursive: real
    /// MPIs cannot nest.
    pub fn as_multipak(&mut self) -> Option<&mut MultiPak> {
        match self {
            Cart::MultiPak(mp) => Some(mp),
            _ => None,
        }
    }

    cart_accessor!(
        /// The FD-502 disk controller, if one is inserted — how the frontend
        /// reaches drive slots to insert/eject a floppy while running.
        as_disk_cart, find_disk_cart, DiskCart, crate::fdc::DiskCart
    );

    cart_accessor!(
        /// The Deluxe RS-232 pak, if one is inserted — how the frontend
        /// swaps host endpoints and reads the TX/RX activity counters.
        as_deluxe_rs232, find_deluxe_rs232, DeluxeRS232, crate::rs232::DeluxeRS232
    );

    cart_accessor!(
        /// The Disto real-time clock, if one is inserted — how the frontend
        /// reaches the clock chip (sync to host time).
        as_disto_rtc, find_disto_rtc, DistoRTC, crate::rtc::DistoRTC
    );

    cart_accessor!(
        /// The Orchestra-90, if one is inserted — how the frontend reads the
        /// DAC latches for its level meters.
        as_orch90, find_orch90, Orch90, crate::orch90::Orch90
    );

    cart_accessor!(
        /// The [`crate::ssc::SoundSpeechCartridge`], if one is inserted — lets
        /// tests and debug tooling reach direct AY-3-8913 register access,
        /// bypassing the `$FF7D`/`$FF7E` host-byte protocol.
        as_ssc, find_ssc, SoundSpeechCartridge, crate::ssc::SoundSpeechCartridge
    );
}

impl Default for Cart {
    /// An empty expansion port.
    fn default() -> Self {
        Cart::Empty(EmptySlot)
    }
}

/// Save-state support: walking every cartridge
/// reachable from the expansion port and detecting the one variant that
/// can't be snapshotted at all.
impl Cart {
    /// `(mpi_slot, &mut Cart)` pairs reachable from this cart: itself with
    /// `mpi_slot: None` if this isn't a [`MultiPak`], or its four slots
    /// otherwise. One level is always enough since a [`MultiPak`] slot is
    /// never itself a `MultiPak`.
    pub fn slots_mut(&mut self) -> Vec<(Option<u8>, &mut Cart)> {
        match self {
            Cart::MultiPak(mp) => mp
                .slots
                .iter_mut()
                .enumerate()
                .map(|(i, cart)| (Some(i as u8), cart))
                .collect(),
            other => vec![(None, other)],
        }
    }

    /// True if this cart (or, for a [`MultiPak`], any of its slots) is
    /// [`Cart::Custom`] — an out-of-crate test double with no serializable
    /// shape. [`crate::snapshot::save`] checks this to surface a clean
    /// `SnapshotError::CustomCartNotSnapshotable` instead of a raw serde error.
    pub fn contains_custom(&self) -> bool {
        match self {
            Cart::Custom(_) => true,
            Cart::MultiPak(mp) => mp.slots.iter().any(Cart::contains_custom),
            _ => false,
        }
    }

    /// True if this cart is a [`MultiPak`] with a `Cart::MultiPak` nested in
    /// one of its own slots — not valid hardware, reachable only from a
    /// hand-crafted payload. Checked by
    /// [`crate::snapshot::restore::validate_payload_shape`] before any
    /// [`Cart::slots_mut`] walk, since that method only descends one level.
    pub fn contains_nested_multipak(&self) -> bool {
        match self {
            Cart::MultiPak(mp) => mp.slots.iter().any(|s| matches!(s, Cart::MultiPak(_))),
            _ => false,
        }
    }
}

/// Manual rather than derived only because [`Cart::Custom`]'s trait object
/// (and a few payloads that elide huge internal buffers from their own
/// `Debug`) can't satisfy a derive bound; every payload that has a `Debug`
/// is printed through it.
impl std::fmt::Debug for Cart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Cart::Empty(slot) => f.debug_tuple("Empty").field(slot).finish(),
            Cart::ROMPak(pak) => f.debug_tuple("ROMPak").field(pak).finish(),
            Cart::BankedROMPak(pak) => f.debug_tuple("BankedROMPak").field(pak).finish(),
            Cart::GamesMasterCartridge(gmc) => {
                f.debug_tuple("GamesMasterCartridge").field(gmc).finish()
            }
            Cart::DiskCart(disk) => f.debug_tuple("DiskCart").field(disk).finish(),
            Cart::MultiPak(mp) => f.debug_tuple("MultiPak").field(mp).finish(),
            Cart::Orch90(orch) => f.debug_tuple("Orch90").field(orch).finish(),
            Cart::DistoRTC(rtc) => f.debug_tuple("DistoRTC").field(rtc).finish(),
            Cart::DeluxeRS232(_) => f.write_str("DeluxeRS232"),
            Cart::SoundSpeechCartridge(_) => f.write_str("SoundSpeechCartridge"),
            Cart::Custom(_) => f.write_str("Custom"),
        }
    }
}

/// `From` impls so `Machine::insert_cartridge`/[`MultiPak::insert`] accept
/// the concrete cartridge types directly (`impl Into<Cart>`).
macro_rules! impl_from_cart {
    ($($ty:ty => $variant:ident),* $(,)?) => {
        $(impl From<$ty> for Cart {
            fn from(cart: $ty) -> Self {
                Cart::$variant(cart)
            }
        })*
    };
}

impl_from_cart!(
    EmptySlot => Empty,
    ROMPak => ROMPak,
    BankedROMPak => BankedROMPak,
    GamesMasterCartridge => GamesMasterCartridge,
    crate::orch90::Orch90 => Orch90,
    crate::rtc::DistoRTC => DistoRTC,
    crate::rs232::DeluxeRS232 => DeluxeRS232,
);

impl From<crate::fdc::DiskCart> for Cart {
    /// Boxes the disk controller — see [`Cart::DiskCart`].
    fn from(disk: crate::fdc::DiskCart) -> Self {
        Cart::DiskCart(Box::new(disk))
    }
}

impl From<crate::ssc::SoundSpeechCartridge> for Cart {
    /// Boxes the SSC — see [`Cart::SoundSpeechCartridge`].
    fn from(ssc: crate::ssc::SoundSpeechCartridge) -> Self {
        Cart::SoundSpeechCartridge(Box::new(ssc))
    }
}

impl From<MultiPak> for Cart {
    /// Boxes the MPI — see [`Cart::MultiPak`].
    fn from(mp: MultiPak) -> Self {
        Cart::MultiPak(Box::new(mp))
    }
}
