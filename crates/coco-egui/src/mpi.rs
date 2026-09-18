use crate::*;

/// Number of physical slots on a Multi-Pak Interface — re-exported from the
/// core crate's own constant so the frontend's slot arrays can't drift from
/// [`coco_core::cart::MultiPak`]'s.
pub(crate) const MPI_SLOT_COUNT: usize = coco_core::cart::mpi::SLOT_COUNT;

/// Front-panel MPI switch position an [`MultiPak`] starts on
/// ([`CocoApp::insert_multipak`]): slot 4, the conventional disk-controller
/// default (also MAME's default — `coco_multi.cpp` `MULTI_SLOT_LOOKUP`).
pub(crate) const DEFAULT_MPI_SWITCH_SLOT: usize = MPI_SLOT_COUNT - 1;

/// What occupies one Multi-Pak Interface slot, tracked by the frontend so a
/// cold restart (or the status bar) can describe it without having to
/// match on the core's [`Cart`] enum. The FD-502 doesn't carry its own disk
/// paths here — those stay in [`CocoApp::disk_paths`]
/// exactly as they do without an MPI, since [`Cart::as_disk_cart`]
/// forwarding already makes the drive UI transparent to whether the
/// controller lives at the top level or nested in a slot.
///
/// [`Cart`]: coco_core::cart::Cart
/// [`Cart::as_disk_cart`]: coco_core::cart::Cart::as_disk_cart
pub(crate) enum MPISlot {
    Empty,
    ROMPak(PathBuf),
    BankedROMPak(PathBuf),
    FD502,
    DistoRTC,
    /// Deluxe RS-232 Pak; `None` if it has no EPROM dump installed (it's
    /// fully usable ROM-less — CTS reads answer open-bus). At most one
    /// across the whole machine — two would fight over the shared ACIA at
    /// `$FF68`, reachable from any slot regardless of switch/`$FF7F`
    /// selection (the pak decodes the full address bus itself).
    DeluxeRS232(Option<PathBuf>),
    GamesMasterCartridge(PathBuf),
    Orch90,
    SoundSpeechCartridge,
    CoCoMax,
}

/// Frontend-tracked state of an inserted [`MultiPak`]: which slot the
/// front-panel switch points at (mirrors [`MultiPak::set_switch`]) and what's
/// plugged into each of its 4 slots ([`MPISlot`]).
pub(crate) struct MPIState {
    pub(crate) switch: usize,
    pub(crate) slots: [MPISlot; MPI_SLOT_COUNT],
}
