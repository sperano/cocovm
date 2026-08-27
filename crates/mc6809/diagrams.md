# mc6809 crate — flow charts & sequence diagrams

## Flow Charts

### 1. `MC6809::step()` — main dispatch loop

```mermaid
flowchart TD
    S[step] --> R{Running?}
    R -- No: Syncing/Waiting --> IDLE[burn 1 idle cycle]
    IDLE --> DONE[return 1]

    R -- Yes --> OP[fetch_u8 → opcode]
    OP --> MATCH{opcode?}

    MATCH -- 0x12 --> NOP[return 2]
    MATCH -- 0x20..=0x2F --> SBR[short Bcc: fetch i8, eval branch_taken, return 3]
    MATCH -- 0x16 --> LBR[LBRA: fetch i16, add to PC, return 5]
    MATCH -- 0x10 --> P10[exec_page10: fetch op2, dispatch long LBcc / 16-bit ops / SWI2]
    MATCH -- 0x11 --> P11[exec_page11: CMPU/CMPS/SWI3]
    MATCH -- control xfer --> CTRL[exec_control_transfer: JMP/JSR/BSR/LBSR/RTS/TFR/EXG/PSH/PUL]
    MATCH -- misc inherent --> MISC[exec_misc_inherent: ORCC/ANDCC/SEX/ABX/MUL/DAA]
    MATCH -- interrupt --> INT[exec_interrupt_halt: SWI/RTI/CWAI/SYNC]
    MATCH -- load/store --> LS[exec_load_store: LDA/LDB/STA/STB/LDD/STD]
    MATCH -- 8-bit ALU --> ALU8[exec_alu8: ADD/ADC/SUB/SBC/CMP for A,B]
    MATCH -- indexed --> IDX[exec_indexed: LEA/LD/ST + ALU + ADC/SBC/CMP indexed]
    MATCH -- 8-bit logic --> LOG8[exec_logic8: AND/OR/EOR/BIT for A,B all modes]
    MATCH -- 16-bit ALU/LD/ST --> ALU16[exec_16bit: ADDD/SUBD/CMPX + LDX/STX/LDU/STU]
    MATCH -- 0x00-0x7F RMW --> RMW[exec_rmw: NEG/COM/LSR/ROR/ASR/ASL/ROL/DEC/INC/TST/CLR]
    MATCH -- other --> UNK[return 2]

    SBR --> UP[cycles += consumed, return cycles]
    LBR --> UP
    P10 --> UP
    P11 --> UP
    CTRL --> UP
    MISC --> UP
    INT --> UP
    LS --> UP
    ALU8 --> UP
    IDX --> UP
    LOG8 --> UP
    ALU16 --> UP
    RMW --> UP
    NOP --> UP
    UNK --> UP
```

### 2. Indexed Addressing: `ea_indexed()`

```mermaid
flowchart TD
    EAI[ea_indexed] --> PB[fetch postbyte]
    PB --> HI{bit 7 set?}
    HI -- No: 0rrnnnnn --> OF5["ea_indexed_offset5:<br/>extract reg sel, signed 5-bit offset,<br/>return reg+offset, cycles=1"]

    HI -- Yes: 1rri mmmm --> FULL["ea_indexed_full:<br/>extract reg sel, indirect bit, submode mmmm"]
    FULL --> SM[ea_indexed_submode dispatch on mmmm]

    SM -- 0000 --> RPLUS[ ,R+ : auto-inc by 1, return pre-inc value, +2cy ]
    SM -- 0001 --> RPLUS2[ ,R++ : auto-inc by 2, +3cy ]
    SM -- 0010 --> RMINUS[ ,-R : auto-dec by 1, return post-dec value, +2cy ]
    SM -- 0011 --> RMINUS2[ ,--R : auto-dec by 2, +3cy ]
    SM -- 0100 --> R0[ ,R : no offset, +0cy ]
    SM -- 0101 --> BR[B,R : signed B offset, +1cy ]
    SM -- 0110 --> AR[A,R : signed A offset, +1cy ]
    SM -- 1000 --> N8R[n,R 8-bit : signed byte offset, +1cy ]
    SM -- 1001 --> N16R[n,R 16-bit : word offset, +4cy ]
    SM -- 1011 --> DR[D,R : D as offset, +4cy ]
    SM -- 1100 --> N8PC[n,PCR 8-bit : PC-relative byte offset, +1cy ]
    SM -- 1101 --> N16PC[n,PCR 16-bit : PC-relative word offset, +5cy ]
    SM -- 1111 --> EXTIND["[n] extended indirect : fetch absolute addr, +2cy"]
    SM -- other --> ILL[illegal: plain register read, +0cy]

    RPLUS --> IND
    RPLUS2 --> IND
    RMINUS --> IND
    RMINUS2 --> IND
    R0 --> IND
    BR --> IND
    AR --> IND
    N8R --> IND
    N16R --> IND
    DR --> IND
    N8PC --> IND
    N16PC --> IND
    EXTIND --> IND
    ILL --> IND
    OF5 --> DONE[return ea, extra_cycles]

    IND{indirect?} -- Yes --> INDF[read_u16 at ea, extra += 3]
    INDF --> DONE
    IND -- No --> DONE
```

### 3. Interrupt Delivery

```mermaid
flowchart TD
    NMI[nmi] --> ARMED{nmi_armed?}
    ARMED -- No --> RET_N[return no-op]
    ARMED -- Yes --> TAKEN[take_interrupt: NMI vector, I+F set, entire=true]

    IRQ[irq] --> IMASK{I mask set?}
    IMASK -- Yes --> WK_S{Syncing?}
    WK_S -- Yes --> WAKE_S[wake to Running, return false]
    WK_S -- No --> RET_F[return false]
    IMASK -- No --> TAKEN_I[take_interrupt: IRQ vector, set I, clear F, entire=true, return true]

    FIRQ[firq] --> FMASK{F mask set?}
    FMASK -- Yes --> WK_F{Syncing?}
    WK_F -- Yes --> WAKE_F[wake to Running, return false]
    WK_F -- No --> RET_FF[return false]
    FMASK -- No --> TAKEN_F[take_interrupt: FIRQ vector, set I+F, entire=false, return true]

    TAKEN --> TAKE
    TAKEN_I --> TAKE
    TAKEN_F --> TAKE

    TAKE[take_interrupt: bus, vector, set_i, set_f, entire] --> COST{Waiting?}
    COST -- Yes --> WAKE_COST[entry cost = 4]
    COST -- No, entire --> FULL_COST[entry cost = 19]
    COST -- No, partial --> FAST_COST[entry cost = 10]
    WAKE_COST --> S{Waiting?}
    FULL_COST --> S
    FAST_COST --> S
    S -- No: not CWAI --> FRAME{entire?}
    FRAME -- Yes --> FULL[set E bit, psh full register mask to S]
    FRAME -- No --> FAST[clear E bit, psh PC+CC only to S]
    FULL --> MASKS
    FAST --> MASKS
    S -- Yes: CWAI already stacked --> MASKS[set I/F masks as requested]
    MASKS --> VEC[pc = read_u16 at vector]
    VEC --> RUN[state = Running]
    RUN --> CHARGE[add entry cost to cycles]
```

### 4. `psh()` / `pul()` — stack register-mask transfer

```mermaid
flowchart TD
    PSHS[pshs mask] --> PSH[psh: to_s=true]
    PSHU[pshu mask] --> PSH_U[psh: to_s=false]
    PSHS --> PSH
    PSHU --> PSH_U
    PSH --> PSP{to_s?}
    PSH_U --> PSP
    PSP -- true --> S_SP[sp = s, other = u]
    PSP -- false --> U_SP[sp = u, other = s]

    S_SP --> ORDER[push in order: PC → other SP → Y → X → DP → B → A → CC]
    U_SP --> ORDER
    ORDER --> CHK{mask bit set?}
    CHK --> PPC[push16 PC, bytes += 2]
    PPC --> PO[push16 other SP, bytes += 2]
    PO --> PY[push16 Y, bytes += 2]
    PY --> PX[push16 X, bytes += 2]
    PX --> PDP[push8 DP, bytes += 1]
    PDP --> PB[push8 B, bytes += 1]
    PB --> PA[push8 A, bytes += 1]
    PA --> PCC[push8 CC, bytes += 1]
    PCC --> SAVE[write back sp, return 5 + bytes]

    PULS[puls mask] --> PUL[pul: from_s=true]
    PULU[pulu mask] --> PUL_U[pul: from_s=false]
    PULS --> PUL
    PULU --> PUL_U
    PUL --> USP{from_s?}
    PUL_U --> USP
    USP -- true --> S_SP2[sp = s]
    USP -- false --> U_SP2[sp = u]

    S_SP2 --> PORDER[pull in order: CC → A → B → DP → X → Y → other SP → PC]
    U_SP2 --> PORDER
    PORDER --> PCC2[CC = pull8, bytes += 1]
    PCC2 --> PA2[A = pull8, bytes += 1]
    PA2 --> PB2[B = pull8, bytes += 1]
    PB2 --> PDP2[DP = pull8, bytes += 1]
    PDP2 --> PX2[X = pull16, bytes += 2]
    PX2 --> PY2[Y = pull16, bytes += 2]
    PY2 --> PO2[other SP = pull16, bytes += 2]
    PO2 --> PPC2[PC = pull16, bytes += 2]
    PPC2 --> SP2[write back sp, return 5 + bytes]
```

### 5. `branch_taken()` — branch condition evaluation

```mermaid
flowchart TD
    BT[branch_taken: cond, cc] --> EX[extract C,Z,N,V from cc]
    EX --> SW{cond and 0x0F}
    SW -- 0x0 --> T[true - BRA]
    SW -- 0x1 --> F[false - BRN]
    SW -- 0x2 --> H[!C and !Z - BHI]
    SW -- 0x3 --> LS[C or Z - BLS]
    SW -- 0x4 --> CC[!C - BCC/BHS]
    SW -- 0x5 --> CS[C - BCS/BLO]
    SW -- 0x6 --> NE[!Z - BNE]
    SW -- 0x7 --> EQ[Z - BEQ]
    SW -- 0x8 --> VC[!V - BVC]
    SW -- 0x9 --> VS[V - BVS]
    SW -- 0xA --> PL[!N - BPL]
    SW -- 0xB --> MI[N - BMI]
    SW -- 0xC --> GE[N == V - BGE]
    SW -- 0xD --> LT[N != V - BLT]
    SW -- 0xE --> GT[!Z and N == V - BGT]
    SW -- 0xF --> LE[Z or N != V - BLE]
```

---

## Sequence Diagrams

### 1. CPU Emulation Loop

Sequence of the caller (e.g. coco-core) driving the CPU:

```mermaid
sequenceDiagram
    participant Emu as "Emulator (coco-core)"
    participant CPU as MC6809
    participant Bus as Bus impl

    Emu->>+CPU: reset(bus)
    CPU->>Bus: read_u16(VECTOR_RESET = $FFFE)
    Bus-->>CPU: 16-bit address
    CPU->>CPU: dp = 0, cc |= I|F, pc = vector
    CPU-->>-Emu: (pc set, nmi_armed = false)

    loop each frame
        Emu->>CPU: step(bus)
        CPU->>Bus: read(pc) → opcode
        Bus-->>CPU: opcode byte
        Note over CPU: dispatch on opcode

        alt Immediate operand
            CPU->>Bus: read(pc) → imm8
        else Direct addressing
            CPU->>Bus: read(pc) → dp_lo
            CPU->>Bus: read(DP:dp_lo) → operand
        else Extended addressing
            CPU->>Bus: read(pc), read(pc+1) → addr
            CPU->>Bus: read(addr) → operand
        else Indexed addressing
            CPU->>Bus: read(pc) → postbyte
            loop decode postbyte
                CPU->>Bus: read(pc) → offset bytes
            end
            CPU->>Bus: read(ea) → operand
        end

        CPU->>CPU: execute (ALU, load, store, branch...)
        CPU->>CPU: cycles += consumed
        CPU-->>Emu: return cycles

        Emu->>CPU: nmi(bus) / irq(bus) / firq(bus)
        opt Interrupt accepted
            CPU->>Bus: PUSH via write() calls (stack frame)
            CPU->>Bus: read_u16(vector) → new PC
        end
    end
```

### 2. JSR/BSR → RTS Subroutine Call

```mermaid
sequenceDiagram
    participant CPU as MC6809
    participant Bus as Bus

    Note over CPU: PC = $C000, S = $0100

    CPU->>Bus: read($C000) → $BD (JSR extended)
    CPU->>Bus: read($C001) → $E0 (high addr)
    CPU->>Bus: read($C002) → $00 (low addr)
    Note over CPU: target = $E000, PC now = $C003
    CPU->>Bus: write($00FF) = $03 (PC lo)
    CPU->>Bus: write($00FE) = $C0 (PC hi)
    Note over CPU: S = $00FE, PC = $E000

    Note over CPU: ...subroutine body...

    CPU->>Bus: read($E050) → $39 (RTS)
    CPU->>Bus: read($00FE) → $C0 (return hi)
    CPU->>Bus: read($00FF) → $03 (return lo)
    Note over CPU: S = $0100, PC = $C003
```

### 3. Interrupt Handling (IRQ)

```mermaid
sequenceDiagram
    participant Emu as Emulator
    participant CPU as MC6809
    participant Bus as Bus

    Note over CPU: CC has I mask clear, state = Running, nmi_armed = true

    Emu->>CPU: irq(bus)
    CPU->>CPU: I mask check → not set
    CPU->>CPU: take_interrupt: entire=true, set I, clear F
    CPU->>CPU: cc |= ENTIRE
    CPU->>Bus: write(S-1) = PC lo
    CPU->>Bus: write(S-2) = PC hi
    CPU->>Bus: write(S-3) = U lo
    CPU->>Bus: write(S-4) = U hi
    CPU->>Bus: write(S-5) = Y lo
    CPU->>Bus: write(S-6) = Y hi
    CPU->>Bus: write(S-7) = X lo
    CPU->>Bus: write(S-8) = X hi
    CPU->>Bus: write(S-9) = DP
    CPU->>Bus: write(S-10) = B
    CPU->>Bus: write(S-11) = A
    CPU->>Bus: write(S-12) = CC
    Note over CPU: S -= 12, cc |= IRQ_MASK
    CPU->>Bus: read_u16(VECTOR_IRQ = $FFF8) → handler addr
    CPU->>CPU: pc = handler, state = Running, cycles += 19
    CPU-->>Emu: true (serviced)
```

### 4. FIRQ — Fast Interrupt (partial frame)

```mermaid
sequenceDiagram
    participant Emu as Emulator
    participant CPU as MC6809
    participant Bus as Bus

    Note over CPU: CC has F mask clear, state = Running

    Emu->>CPU: firq(bus)
    CPU->>CPU: F mask check → not set
    CPU->>CPU: take_interrupt: entire=false, set I, set F
    CPU->>CPU: cc &= ~ENTIRE
    CPU->>Bus: write(S-1) = PC lo
    CPU->>Bus: write(S-2) = PC hi
    CPU->>Bus: write(S-3) = CC
    Note over CPU: S -= 3, cc |= IRQ_MASK | FIRQ_MASK
    CPU->>Bus: read_u16(VECTOR_FIRQ = $FFF6) → handler addr
    CPU->>CPU: pc = handler, state = Running, cycles += 10
    CPU-->>Emu: true (serviced)
```

### 5. CWAI → Interrupt (combined stack+hlt)

```mermaid
sequenceDiagram
    participant Emu as Emulator
    participant CPU as MC6809
    participant Bus as Bus

    Note over CPU: step() encounters opcode $3C (CWAI)

    CPU->>Bus: read(pc) → imm8 AND mask
    CPU->>CPU: cc &= imm8 (clear selected bits)
    CPU->>CPU: cc |= ENTIRE
    CPU->>CPU: psh full register set (PC,U/S,Y,X,DP,B,A,CC)
    CPU->>CPU: state = Waiting
    Note over CPU: CPU halted — no further steps

    Emu->>CPU: irq(bus)
    CPU->>CPU: I mask check
    alt I mask clear (CWAI opened the IRQ door)
        Note over CPU: take_interrupt: state == Waiting → skip re-stacking
        CPU->>CPU: set I/F, pc = read_u16(VECTOR_IRQ)
        CPU->>CPU: state = Running, cycles += 4
        CPU-->>Emu: true
    else I mask still set
        CPU-->>Emu: false (ignored)
    end
```

### 6. SYNC → Interrupt Wake

```mermaid
sequenceDiagram
    participant Emu as Emulator
    participant CPU as MC6809
    participant Bus as Bus

    Note over CPU: step() encounters opcode $13 (SYNC)

    CPU->>CPU: state = Syncing

    Emu->>CPU: step(bus)
    CPU->>CPU: state != Running → burn 1 idle cycle
    CPU-->>Emu: 1

    Emu->>CPU: irq(bus)
    CPU->>CPU: I mask check
    alt I mask set (IRQ masked)
        CPU->>CPU: state == Syncing → wake to Running
        CPU-->>Emu: false (not serviced)
        Note over CPU: Next step() will execute the instruction after SYNC
    else I mask clear
        CPU->>CPU: take_interrupt → stack frame, vector, resume
        CPU-->>Emu: true (serviced)
    end
```

### 7. TFR / EXG Register Transfer

```mermaid
sequenceDiagram
    participant CPU as MC6809

    Note over CPU: TFR A,B (opcode $1F, postbyte $89)
    CPU->>CPU: fetch postbyte $89 → src=$8(A), dst=$9(B)
    CPU->>CPU: tfr_value: reg_read(src=A) → $12 (A's value)
    CPU->>CPU: tfr_value: src=8bit, dst=8bit → pass through as $0012
    CPU->>CPU: reg_write(dst=B, $0012) → B = $12

    Note over CPU: TFRA,X (opcode $1F, postbyte $81)
    CPU->>CPU: src=$8(A)=$FF, dst=$1(X)
    CPU->>CPU: tfr_value: A→16 → $FFFF (high=$FF, low=A)
    CPU->>CPU: reg_write(dst=X, $FFFF) → X = $FFFF

    Note over CPU: EXG D,X (opcode $1E, postbyte $01)
    CPU->>CPU: reg_read($0=D) → d1, reg_read($1=X) → x1
    CPU->>CPU: reg_write($0=D, x1), reg_write($1=X, d1)
```
