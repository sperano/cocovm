use mc6809::disasm::disassemble;

/// Disassemble `bytes` (padded with zeros) as if loaded at `addr`.
pub fn disasm_at(addr: u16, bytes: &[u8]) -> mc6809::disasm::Insn {
    let mem = bytes.to_vec();
    disassemble(
        &mut |a: u16| {
            let idx = a.wrapping_sub(addr) as usize;
            mem.get(idx).copied().unwrap_or(0)
        },
        addr,
    )
}
