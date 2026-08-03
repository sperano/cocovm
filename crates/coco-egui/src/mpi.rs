use crate::*;

/// Number of physical slots on a Multi-Pak Interface — re-exported from the
/// core crate's own constant so the frontend's slot arrays can't drift from
/// [`coco_core::cart::MultiPak`]'s.
pub(crate) const MPI_SLOT_COUNT: usize = coco_core::cart::mpi::SLOT_COUNT;

/// Front-panel MPI switch position an [`MultiPak`] starts on
/// ([`CocoApp::insert_multipak`]): slot 4, the conventional disk-controller
/// default (also MAME's default — `coco_multi.cpp` `MULTI_SLOT_LOOKUP`).
pub(crate) const DEFAULT_MPI_SWITCH_SLOT: usize = MPI_SLOT_COUNT - 1;

/// MPI slot the RTC defaults to (slot 3): a cartridge takes slot 1 and the
/// FD-502 slot 4, the conventional layout the manager's "New…" dialog builds.
pub(crate) const DEFAULT_RTC_SLOT: usize = 2;

/// What occupies one Multi-Pak Interface slot, tracked by the frontend so a
/// cold restart (or just the status bar / menu labels) can describe it
/// without having to match on the core's [`Cart`] enum. The FD-502 doesn't
/// carry its own disk paths here — those stay in [`CocoApp::disk_paths`]
/// exactly as they do without an MPI, since [`Cart::as_disk_cart`]
/// forwarding already makes the drive UI transparent to whether the
/// controller lives at the top level or nested in a slot.
///
/// [`Cart`]: coco_core::cart::Cart
/// [`Cart::as_disk_cart`]: coco_core::cart::Cart::as_disk_cart
pub(crate) enum MPISlot {
    Empty,
    ROMPak(PathBuf),
    FD502,
    DistoRTC,
    GamesMasterCartridge(PathBuf),
    Orch90(PathBuf),
    SoundSpeechCartridge,
}

/// Frontend-tracked state of an inserted [`MultiPak`]: which slot the
/// front-panel switch points at (mirrors [`MultiPak::set_switch`]) and what's
/// plugged into each of its 4 slots ([`MPISlot`]).
pub(crate) struct MPIState {
    pub(crate) switch: usize,
    pub(crate) slots: [MPISlot; MPI_SLOT_COUNT],
}
