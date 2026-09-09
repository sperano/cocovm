//! Tandy DMP-130 serial printer, including Tandy and IBM command grammars.
//! See `docs/dmp130-protocol.md` for sources and rendering approximations.
//! Text remains buffered until a print-triggering control code arrives.
use crate::{
    bitbanger::PrinterSink,
    printer::{Paper, X_UNITS_PER_INCH, Y_UNITS_PER_INCH},
};
use serde::{Deserialize, Serialize};
mod codes;
mod font;
mod ibm;
mod motion;
mod parser;
mod raster;
mod tandy;
mod widths;
// Software denominator: condensed 959 POS columns and NLQ half-unit dots.
const X_FRACTION: u64 = 1918;
const X_INCH: u64 = X_UNITS_PER_INCH as u64 * X_FRACTION;
const PRINT_WIDTH: u64 = 8 * X_INCH;
const FULL_FEED: i32 = (Y_UNITS_PER_INCH / 6) as i32;
const GRAPHICS_FEED: i32 = (7 * Y_UNITS_PER_INCH / 72) as i32;
const DEFAULT_FORM: u32 = 11 * Y_UNITS_PER_INCH;
const TAB_INTERVAL: u32 = 8;
const MAX_TABS: usize = 28;
const MAX_COLUMNS: u32 = 137;
const GRAPHICS_COLUMNS: u32 = 480;
const DOUBLE_WIDTH: u64 = 2;
const USA: u8 = 32;
const LAST_COUNTRY: u8 = 42;
const MAX_FORM_INCHES: u8 = 22;
const MAX_FORM_LINES: u8 = 127;
const MAX_STAGED_FEED: u8 = 85;
const MIN_TANDY_FORM_LINES: u8 = 2;
const GRAPHICS_DPI: u64 = 60;
const IBM_DOUBLE_DPI: u64 = 120;
const IBM_QUADRUPLE_DPI: u64 = 240;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Grammar {
    Tandy,
    Ibm,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Mode {
    DataProcessing,
    WordProcessing,
    Graphics,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Pitch {
    Pica,
    Elite,
    Condensed,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Charset {
    Tandy,
    Ibm1,
    Ibm2,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Script {
    Super,
    Sub,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Style {
    pitch: Pitch,
    nlq: bool,
    proportional: bool,
    condensed: bool,
    underline: bool,
    bold: bool,
    double_strike: bool,
    italic: bool,
    script: Option<Script>,
    micro: bool,
    wide: bool,
    transient_wide: bool,
}
impl Default for Style {
    fn default() -> Self {
        Self {
            pitch: Pitch::Pica,
            nlq: false,
            proportional: false,
            condensed: false,
            underline: false,
            bold: false,
            double_strike: false,
            italic: false,
            script: None,
            micro: false,
            wide: false,
            transient_wide: false,
        }
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
enum Pending {
    #[default]
    None,
    Escape,
    Operands {
        selector: u8,
        bytes: Vec<u8>,
        need: usize,
    },
    RepeatCount,
    RepeatData(u8),
    Backspace,
    Tabs {
        stops: Vec<u8>,
        valid: bool,
    },
    FormInches,
    Graphics {
        remaining: u16,
        step: u64,
    },
}
/// DMP-130 interpreter with factory DIP-switch defaults and continuous paper.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DMP130 {
    grammar: Grammar,
    mode: Mode,
    text_mode: Mode,
    graphics_wide: bool,
    style: Style,
    charset: Charset,
    country: u8,
    x: u64,
    y: u32,
    left: u64,
    right: u64,
    feed: i32,
    staged_feed: i32,
    cr_lf: bool,
    bidirectional: bool,
    home_next_line: bool,
    paper_out: bool,
    form_length: u32,
    form_top: u32,
    skip: u32,
    tabs: Vec<u64>,
    pending: Pending,
    buffered: Vec<(u32, u32)>,
    paper: Paper,
}
impl Default for DMP130 {
    fn default() -> Self {
        let mut printer = Self {
            grammar: Grammar::Tandy,
            mode: Mode::DataProcessing,
            text_mode: Mode::DataProcessing,
            graphics_wide: false,
            style: Style::default(),
            charset: Charset::Tandy,
            country: USA,
            x: 0,
            y: 0,
            left: 0,
            right: PRINT_WIDTH,
            feed: FULL_FEED,
            staged_feed: FULL_FEED,
            cr_lf: true,
            bidirectional: true,
            home_next_line: false,
            paper_out: true,
            form_length: DEFAULT_FORM,
            form_top: 0,
            skip: 0,
            tabs: Vec::new(),
            pending: Pending::None,
            buffered: Vec::new(),
            paper: Paper::new(),
        };
        printer.reset_tabs();
        printer
    }
}
impl DMP130 {
    pub fn new() -> Self {
        Self::default()
    }
    /// Restore power-on state while retaining impressions already on paper.
    pub fn reset(&mut self) {
        let paper = std::mem::take(&mut self.paper);
        *self = Self::default();
        self.paper = paper;
    }
    pub fn paper(&self) -> &Paper {
        &self.paper
    }
    pub fn paper_mut(&mut self) -> &mut Paper {
        &mut self.paper
    }
    fn flush(&mut self) {
        for (x, y) in self.buffered.drain(..) {
            self.paper.mark(x, y);
        }
    }
    fn switch_grammar(&mut self) {
        self.flush();
        let grammar = if self.grammar == Grammar::Tandy {
            Grammar::Ibm
        } else {
            Grammar::Tandy
        };
        let y = self.y;
        self.reset();
        self.grammar = grammar;
        self.y = y;
        self.form_top = y;
        if grammar == Grammar::Ibm {
            self.charset = Charset::Ibm1;
        }
    }
}
impl PrinterSink for DMP130 {
    fn write_byte(&mut self, byte: u8) {
        self.feed_byte(byte);
    }
}
#[cfg(test)]
#[path = "dmp130_test.rs"]
mod tests;
