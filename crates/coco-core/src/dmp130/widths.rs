// Visually transcribed DMP-130 Operation Manual Appendix A pp82–83.
// DOT-COLUMN WIDTHS, not automatically the advance for monospaced modes:
// monospaced cell width is fixed12/14/24 (p44). Tables label these
// "Standard and proportional characters" and "Correspondence and
// proportional characters". Their exact blank-column accounting relative
// to proportional advances is not separately explained. Do not claim
// hardware-measured advances. Last entry0x7F undefined -> zero sentinel.
pub const STANDARD_ASCII_WIDTHS: [u8; 96] = [
    12, 9, 9, 11, 11, 11, 11, 9, 8, 9, 11, 11, 9, 11, 9, 10, 11, 11, 11, 11, 11, 11, 11, 11, 11,
    11, 9, 9, 10, 11, 10, 10, 11, 11, 11, 11, 11, 11, 11, 11, 11, 9, 11, 11, 11, 11, 11, 11, 11,
    11, 11, 11, 11, 11, 11, 11, 10, 11, 11, 9, 10, 9, 9, 11, 8, 11, 11, 9, 11, 11, 9, 11, 11, 9, 8,
    9, 9, 11, 11, 11, 11, 11, 10, 11, 11, 11, 11, 11, 11, 11, 11, 9, 9, 9, 10, 0,
];
pub const CORRESPONDENCE_ASCII_WIDTHS: [u8; 96] = [
    24, 13, 17, 23, 21, 23, 23, 13, 20, 16, 22, 22, 13, 21, 13, 23, 21, 20, 21, 21, 21, 21, 21, 21,
    21, 21, 13, 13, 19, 21, 19, 21, 21, 23, 23, 23, 23, 23, 23, 23, 23, 18, 19, 23, 22, 24, 24, 23,
    22, 23, 23, 23, 23, 23, 23, 24, 23, 23, 22, 13, 23, 13, 19, 24, 17, 22, 23, 22, 23, 21, 21, 23,
    23, 20, 16, 23, 20, 24, 23, 22, 23, 23, 22, 23, 21, 23, 23, 24, 23, 23, 21, 21, 13, 16, 21, 0,
];
