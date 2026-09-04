//! The cartridge board around the TMS7040: what its four ports are wired to
//! (MAME `coco_ssc.cpp`'s port handlers, in their statement order).
//!
//! ```text
//! Port A: input — the host byte latched by a `$FF7E` write
//! Port B: output — A0-A7 of the 2 KB static RAM
//! Port C: output — bit 0 A8 / AY BC1, 1 A9, 2 A10, 3 RAM R/W* / AY BDIR,
//!                  4 RAM CS*, 5 SP0256 ALD*, 6 AY CS*, 7 BUSY* to the host
//! Port D: bidirectional — the shared data bus (RAM, AY, SP0256 address)
//! ```

use serde::{Deserialize, Serialize};
use tms7000::{Bus, Port};

use crate::ay8913::AY8913;
use crate::sp0256::SP0256;

/// The board's static RAM (MAME `RAM(config, "staticram").set_default_size("2K")`).
pub(super) const RAM_SIZE: usize = 2048;
/// Address bits the RAM decodes: A0-A7 from port B, A8-A10 from port C.
const RAM_ADDRESS_MASK: u16 = 0x7FF;

/// Port C bit assignments (MAME `C_*`; the `*` lines are active low).
mod pc {
    pub const A8_BC1: u8 = 0x01;
    pub const RAM_RW_BDIR: u8 = 0x08;
    pub const RAM_CS: u8 = 0x10;
    pub const ALD: u8 = 0x20;
    pub const AY_CS: u8 = 0x40;
    pub const BUSY: u8 = 0x80;
}

/// Allophone addresses the board passes to the SP0256: ALD only strobes
/// for port D values below 64 (MAME `ssc_port_c_w`: `m_tms7000_portd < 64`),
/// matching the AL2 ROM's 64-entry jump table.
pub const ALLOPHONE_COUNT: u8 = 64;

/// Latches and RAM on the board; the chips themselves live on the cartridge.
#[derive(Serialize, Deserialize)]
pub(super) struct Board {
    /// Static RAM: zero at power-on, kept across resets (MAME
    /// `set_default_value(0)`; the cartridge's reset doesn't touch it).
    #[serde(with = "crate::serde_util::byte_array")]
    pub(super) ram: [u8; RAM_SIZE],
    /// The host byte the last `$FF7E` write latched.
    pub(super) port_a: u8,
    pub(super) port_b: u8,
    /// Port C as last written; edge detection compares against it.
    pub(super) port_c: u8,
    /// The data bus as last driven, by the firmware or a RAM/AY read.
    pub(super) port_d: u8,
    /// INT3 to the TMS7040: raised by a `$FF7E` write, dropped when the
    /// firmware reads port A (MAME `ssc_port_a_r`).
    pub(super) int3: bool,
    /// The host's BUSY* flag: set by a `$FF7E` write, cleared by a rising
    /// edge on port C bit 7.
    pub(super) busy: bool,
}

impl Default for Board {
    fn default() -> Self {
        Self {
            ram: [0; RAM_SIZE],
            port_a: 0,
            port_b: 0,
            port_c: 0,
            port_d: 0,
            int3: false,
            busy: false,
        }
    }
}

impl Board {
    fn ram_address(&self, port_c: u8) -> usize {
        usize::from((u16::from(port_c) << 8 | u16::from(self.port_b)) & RAM_ADDRESS_MASK)
    }
}

/// The board plus the chips it wires the ports to, borrowed for one
/// [`tms7000::TMS7040::step`].
pub(super) struct BoardView<'a> {
    pub(super) board: &'a mut Board,
    pub(super) ay: &'a mut AY8913,
    pub(super) sp0256: &'a mut SP0256,
}

impl Bus for BoardView<'_> {
    fn read_port(&mut self, port: Port) -> u8 {
        match port {
            Port::A => {
                self.board.int3 = false;
                self.board.port_a
            }
            Port::B => 0xFF,
            Port::C => self.board.port_c,
            Port::D => self.read_port_d(),
        }
    }

    fn write_port(&mut self, port: Port, val: u8) {
        match port {
            Port::A => {}
            Port::B => self.board.port_b = val,
            Port::C => self.write_port_c(val),
            Port::D => self.board.port_d = val,
        }
    }
}

impl BoardView<'_> {
    /// MAME `ssc_port_d_r`: a RAM read or an AY register read, else the bus
    /// as last driven.
    fn read_port_d(&mut self) -> u8 {
        let c = self.board.port_c;
        if c & pc::RAM_CS == 0 && c & pc::RAM_RW_BDIR != 0 {
            self.board.port_d = self.board.ram[self.board.ram_address(c)];
        }
        if c & pc::AY_CS == 0 && c & pc::RAM_RW_BDIR == 0 && c & pc::A8_BC1 != 0 {
            self.board.port_d = self.ay.read_data();
        }
        self.board.port_d
    }

    /// MAME `ssc_port_c_w`: every strobe decodes on a port C write, using
    /// the *new* bits for the RAM address and the data bus as last driven.
    fn write_port_c(&mut self, val: u8) {
        let old = self.board.port_c;
        let data = self.board.port_d;
        if val & pc::RAM_CS == 0 && val & pc::RAM_RW_BDIR == 0 {
            let addr = self.board.ram_address(val);
            self.board.ram[addr] = data;
        }
        if val & pc::AY_CS == 0 {
            let bdir = val & pc::RAM_RW_BDIR != 0;
            let bc1 = val & pc::A8_BC1 != 0;
            if bdir && bc1 {
                self.ay.write_address(data);
            }
            if bdir && !bc1 {
                self.ay.write_data(data);
            }
        }
        if old & pc::ALD != 0 && val & pc::ALD == 0 && data < ALLOPHONE_COUNT {
            self.sp0256.ald_write(data);
        }
        if old & pc::BUSY == 0 && val & pc::BUSY != 0 {
            self.board.busy = false;
        }
        self.board.port_c = val;
    }
}
