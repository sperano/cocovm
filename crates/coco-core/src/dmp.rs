//! Shared ownership and paper access for Tandy DMP printers.

use std::cell::RefCell;
use std::fmt;
use std::ops::ControlFlow;
use std::rc::Rc;

use serde::{Deserialize, Serialize};

use crate::bitbanger::{PrinterSink, sink_serde::SinkState};
use crate::dmp105::DMP105;
use crate::dmp130::DMP130;
use crate::printer::{Paper, PaperExtent};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DmpModel {
    #[default]
    Dmp105,
    Dmp130,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DmpPrinter {
    Dmp105(Box<DMP105>),
    Dmp130(Box<DMP130>),
}

impl DmpPrinter {
    fn paper(&self) -> &Paper {
        match self {
            Self::Dmp105(printer) => printer.paper(),
            Self::Dmp130(printer) => printer.paper(),
        }
    }

    fn paper_mut(&mut self) -> &mut Paper {
        match self {
            Self::Dmp105(printer) => printer.paper_mut(),
            Self::Dmp130(printer) => printer.paper_mut(),
        }
    }

    fn reset(&mut self) {
        match self {
            Self::Dmp105(printer) => printer.reset(),
            Self::Dmp130(printer) => printer.reset(),
        }
    }

    fn write_byte(&mut self, byte: u8) {
        match self {
            Self::Dmp105(printer) => printer.write_byte(byte),
            Self::Dmp130(printer) => printer.write_byte(byte),
        }
    }
}

/// The serial sink and frontend share the same interpreter and paper.
#[derive(Clone)]
pub struct DmpHandle(Rc<RefCell<DmpPrinter>>);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaperSnapshotTooLarge {
    pub estimated_bytes: usize,
    pub limit_bytes: usize,
}

impl fmt::Display for PaperSnapshotTooLarge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "printer paper snapshot needs about {} bytes, exceeding the {}-byte export limit",
            self.estimated_bytes, self.limit_bytes
        )
    }
}

impl std::error::Error for PaperSnapshotTooLarge {}

impl Default for DmpHandle {
    fn default() -> Self {
        Self::with_model(DmpModel::default())
    }
}

impl DmpHandle {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_model(model: DmpModel) -> Self {
        Self::from_state(match model {
            DmpModel::Dmp105 => DmpPrinter::Dmp105(Box::default()),
            DmpModel::Dmp130 => DmpPrinter::Dmp130(Box::default()),
        })
    }

    pub(crate) fn from_state(state: DmpPrinter) -> Self {
        Self(Rc::new(RefCell::new(state)))
    }

    pub fn model(&self) -> DmpModel {
        match &*self.0.borrow() {
            DmpPrinter::Dmp105(_) => DmpModel::Dmp105,
            DmpPrinter::Dmp130(_) => DmpModel::Dmp130,
        }
    }

    pub fn paper_extent(&self) -> PaperExtent {
        self.0.borrow().paper().extent()
    }

    /// Clones the paper into an immutable snapshot suitable for background rendering.
    pub fn paper_snapshot(&self) -> Paper {
        self.0.borrow().paper().clone()
    }

    /// Clones a paper snapshot only when its estimated owned storage fits the limit.
    pub fn paper_snapshot_with_limit(
        &self,
        limit_bytes: usize,
    ) -> Result<Paper, PaperSnapshotTooLarge> {
        let printer = self.0.borrow();
        let paper = printer.paper();
        let estimated_bytes = paper.estimated_owned_bytes();
        if estimated_bytes > limit_bytes {
            return Err(PaperSnapshotTooLarge {
                estimated_bytes,
                limit_bytes,
            });
        }
        Ok(paper.clone())
    }

    pub fn dots_in_range(&self, y0: u32, y1: u32) -> Vec<(u32, u32)> {
        self.0.borrow().paper().dots_in_range(y0, y1)
    }

    /// Visits dots without allocating a page-local coordinate copy.
    pub fn visit_dots_in_range(&self, y0: u32, y1: u32, visit: &mut dyn FnMut(u32, u32)) {
        self.0.borrow().paper().visit_dots_in_range(y0, y1, visit);
    }

    /// Visits dots until `visit` requests an early break.
    pub fn try_visit_dots_in_range(
        &self,
        y0: u32,
        y1: u32,
        visit: &mut dyn FnMut(u32, u32) -> ControlFlow<()>,
    ) -> ControlFlow<()> {
        self.0
            .borrow()
            .paper()
            .try_visit_dots_in_range(y0, y1, visit)
    }

    pub fn take_dirty(&self) -> Option<(u32, u32)> {
        self.0.borrow_mut().paper_mut().take_dirty()
    }

    pub fn tear_off(&self) {
        self.0.borrow_mut().paper_mut().clear();
    }

    pub fn reset(&self) {
        self.0.borrow_mut().reset();
    }
}

impl PrinterSink for DmpHandle {
    fn write_byte(&mut self, byte: u8) {
        self.0.borrow_mut().write_byte(byte);
    }

    fn snapshot(&self) -> SinkState {
        SinkState::Dmp(self.0.borrow().clone())
    }

    fn as_printer(&self) -> Option<&DmpHandle> {
        Some(self)
    }
}

#[cfg(test)]
#[path = "dmp_test.rs"]
mod tests;
