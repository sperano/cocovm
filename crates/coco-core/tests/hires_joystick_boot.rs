//! Boots hand-assembled 6809 programs on a CoCo 3 that drive each hi-res
//! joystick interface's trigger exactly like real software would, then poll
//! PIA0 PA7 in a tight loop until it reads high, counting iterations.
//! Cross-checks the measured elapsed-cycle count against
//! `hires_joystick::duration_cycles` end to end through the bus/run-loop
//! wiring, not just the state machine in isolation (`hires_joystick_test.rs`).

#[path = "hires_joystick_boot/common.rs"]
mod common;

#[path = "hires_joystick_boot/cocomax3.rs"]
mod cocomax3;
#[path = "hires_joystick_boot/tandy.rs"]
mod tandy;
