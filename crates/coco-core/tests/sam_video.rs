//! SAM V0–V2 VDG-mode strobes ($FFC0–$FFC5): latch the legacy-graphics vertical
//! cadence bits (`GIME::sam_video`), same clear/set strobe-pair pattern as the
//! F0–F6 page bits and R1 speed bit (SEB Unravelled II; MAME `6883sam.cpp`).

use coco_core::{MachineVariant, MemorySize, SystemBus};
use mc6809::Bus;

const V0_CLEAR: u16 = 0xFFC0;
const V0_SET: u16 = 0xFFC1;
const V1_CLEAR: u16 = 0xFFC2;
const V1_SET: u16 = 0xFFC3;
const V2_CLEAR: u16 = 0xFFC4;
const V2_SET: u16 = 0xFFC5;

fn bus() -> SystemBus {
    SystemBus::new(
        MachineVariant::Coco3,
        MemorySize::K512,
        vec![0u8; 32 * 1024].into_boxed_slice(),
    )
}

#[test]
fn vdg_strobes_latch_sam_video_bits_independently() {
    let mut b = bus();
    assert_eq!(b.gime.sam_video, 0, "V bits reset to 0");

    b.write(V0_SET, 0); // written data is ignored — only the address matters
    assert_eq!(b.gime.sam_video, 0b001);

    b.write(V2_SET, 0xFF);
    assert_eq!(b.gime.sam_video, 0b101);

    b.write(V1_SET, 0);
    assert_eq!(b.gime.sam_video, 0b111);

    b.write(V0_CLEAR, 0);
    assert_eq!(b.gime.sam_video, 0b110);

    b.write(V2_CLEAR, 0);
    b.write(V1_CLEAR, 0);
    assert_eq!(b.gime.sam_video, 0);
}

#[test]
fn vdg_strobes_do_not_disturb_the_adjacent_page_bits() {
    // F0 ($FFC6) is the very next strobe pair after V2 ($FFC4/$FFC5) — confirm
    // the ranges don't bleed into each other.
    let mut b = bus();
    b.write(V0_SET, 0);
    b.write(V1_SET, 0);
    b.write(V2_SET, 0);
    assert_eq!(b.gime.sam_video, 0b111);
    assert_eq!(b.gime.sam_page, 0, "page bits untouched by V strobes");

    b.write(0xFFC7, 0); // F0 set
    assert_eq!(b.gime.sam_page, 0b01);
    assert_eq!(b.gime.sam_video, 0b111, "V bits untouched by a page strobe");
}

