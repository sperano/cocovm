//! Opt-in bounded frontend measurements. Without `perf`, calls are empty.

#[derive(Clone, Copy)]
pub(crate) enum Stage {
    ManagerUpdate,
    VmUiUpdate,
    FieldExecution,
    DisplayConversion,
    TextureEnqueue,
    AudioPush,
    #[cfg(feature = "perf")]
    AudioCallbackLockWait,
}

#[cfg(feature = "perf")]
mod enabled;
#[cfg(feature = "perf")]
pub(crate) use enabled::*;

#[cfg(not(feature = "perf"))]
mod disabled {
    use super::Stage;

    pub(crate) struct Span;
    impl Drop for Span {
        #[inline(always)]
        fn drop(&mut self) {}
    }
    #[inline(always)]
    pub(crate) fn span(_: Stage) -> Span {
        Span
    }
    #[inline(always)]
    pub(crate) fn initialize() {}
    #[inline(always)]
    pub(crate) fn texture_enqueue(_: usize) {}
    #[inline(always)]
    pub(crate) fn audio_queue(_: usize, _: usize) {}
}
#[cfg(not(feature = "perf"))]
pub(crate) use disabled::*;
