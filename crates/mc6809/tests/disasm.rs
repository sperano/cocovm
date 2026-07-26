//! Test-driven coverage for the static disassembler (`mc6809::disasm`):
//! immediate/direct/extended/inherent addressing, short/long branches
//! (offset resolved to an absolute address), PSHS/PULS/PSHU/PULU register
//! masks, TFR/EXG register pairs, both prefix pages ($10/$11), illegal
//! opcodes in all three pages, a known byte sequence from the real ROM, and
//! a forward `len`-sum walk that must never land mid-instruction.
//!
//! Indexed-addressing postbyte coverage lives in `disasm_indexed.rs`.

#[path = "disasm/common.rs"]
mod common;

#[path = "disasm/branches.rs"]
mod branches;
#[path = "disasm/direct_extended_lea.rs"]
mod direct_extended_lea;
#[path = "disasm/inherent_immediate.rs"]
mod inherent_immediate;
#[path = "disasm/prefix_pages.rs"]
mod prefix_pages;
#[path = "disasm/rom_and_scan.rs"]
mod rom_and_scan;
#[path = "disasm/stack_transfer.rs"]
mod stack_transfer;
