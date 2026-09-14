use std::ops::ControlFlow;

/// A source of already-printed dot impressions using the paper model's fixed-point units.
pub trait DotSource {
    /// Visits dots in `y0..=y1` until the callback requests an early break.
    fn try_visit_dots_in_range(
        &self,
        y0: u32,
        y1: u32,
        visit: &mut dyn FnMut(u32, u32) -> ControlFlow<()>,
    ) -> ControlFlow<()>;
}

impl DotSource for coco_core::printer::Paper {
    fn try_visit_dots_in_range(
        &self,
        y0: u32,
        y1: u32,
        visit: &mut dyn FnMut(u32, u32) -> ControlFlow<()>,
    ) -> ControlFlow<()> {
        coco_core::printer::Paper::try_visit_dots_in_range(self, y0, y1, visit)
    }
}

impl DotSource for coco_core::dmp::DmpHandle {
    fn try_visit_dots_in_range(
        &self,
        y0: u32,
        y1: u32,
        visit: &mut dyn FnMut(u32, u32) -> ControlFlow<()>,
    ) -> ControlFlow<()> {
        coco_core::dmp::DmpHandle::try_visit_dots_in_range(self, y0, y1, visit)
    }
}
