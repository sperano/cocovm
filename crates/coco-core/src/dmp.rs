//! Shared ownership and paper access for Tandy DMP printers.

use std::cell::RefCell;
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

    pub fn dots_in_range(&self, y0: u32, y1: u32) -> Vec<(u32, u32)> {
        self.0.borrow().paper().dots_in_range(y0, y1)
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
