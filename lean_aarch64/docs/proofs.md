# What is proved, and what will be

A proof is only as good as the statement it proves, so this page is the
statements: what "correct" means for Leanbox, layer by layer, each layer a
different guarantee. A statement is listed here before its proof exists,
the way a test is written before the code. Each entry is marked **proved**
(a theorem in the library, kernel-checked, passing the L0 audit), **checked**
(tested in lockstep or by `#guard`, not yet proved) or **open**.

The type doctrine (`type-doctrine.md`) makes wrong programs unwritable;
this page proves that the written program is right.

## P1. The fast machine is the slow machine

The engine exists only to be fast; these say it changes no answer.

| statement | status |
|---|---|
| CLZ, RBIT, REV16/32/64's loop-free forms equal the Arm ARM's loops (`Isa.Alu`, `@[csimp]`) | proved, but by `bv_decide`, which trusts the compiler; to be reproven for the kernel (L0) |
| `execOp (Op.lower i) = execInstr i`: the flat form an instruction is lowered to means what the instruction means (L15) | open |
| `decode (encode i) = .instr i`, and `decode` is total over the 2³² words (L15) | open |
| **Trace-cache coherence**: every cached trace is the decoding of the words now in memory at its address. A store that may reach code (`Cpu.Memory.noteStore`) preserves it, which is the theorem behind self-modifying code | open |
| **Poll fast-forward is exact**: `Engine.Poll.pollSkip` reaches the same state as stepping the loop's instructions one by one, the iteration count it skips included | checked |
| **The engine refines the reference**: `run` on a trace equals `execOp` folded over its ops, burst limits and interrupts included (L9) | checked (lockstep, both boots); open until `run` is total (L11) |
| The micro-TLBs only ever hold a translation the full walk would give for the current regime | checked |

## P2. The processor obeys the architecture's rules

Facts the Arm ARM states that hold in every state, made invariants of the
state's type where they can be (L1, L4), and theorems where they cannot.

| statement | status |
|---|---|
| XZR reads zero in every reachable state: the register file's type carries `read ZR = 0`, and `RegFile.write` keeps it because a write slot is never ZR (L4) | open |
| A W-register write zeroes bits 63:32 (`Val .w32`, L3) | open |
| Taking an exception to EL1 sets PSTATE.DAIF, saves the return address in ELR_EL1 and the PSTATE in SPSR_EL1, and branches to VBAR_EL1 plus the vector offset (Arm ARM `TakeException`) | checked |
| ERET to an illegal state sets PSTATE.IL, and the next instruction then takes an Illegal Execution State exception (L16) | checked |
| The flags of ADDS/SUBS/ADCS are `AddWithCarry`'s, for every width and operand (L10, over `BitVec`) | open |
| `DecodeBitMasks` accepts exactly the encodings the Arm ARM defines, and `logical` immediates are its results (L15) | open |
| A load returns what the last store to the same bytes wrote: `ld64 (st64 m i v) i = v`, and a store leaves every byte outside it unchanged (`Bytes`) | open |

## P3. User code cannot reach the kernel's state

How a computer keeps programs apart, stated as theorems: privilege and
memory protection.

| statement | status |
|---|---|
| **Privilege isolation**: an instruction executed at EL0 either leaves every EL1-only register unchanged or takes an exception to EL1 (no path from EL0 writes SCTLR_EL1, VBAR_EL1, TTBR*, ...) | open |
| **Memory protection**: a store executed at EL0 changes only bytes in pages whose descriptor grants EL0 write, under the current translation, or it faults | open |
| A fetch executes only from a page the descriptor makes executable at the current exception level (UXN/PXN, WXN) | open |
| Interrupts masked by PSTATE.I are never taken, whatever the GIC signals | checked |

## P4. The devices behave as their manuals say

| statement | status |
|---|---|
| GICv2 signals the highest-priority pending enabled interrupt above the priority mask, and acknowledge/EOI move it through active and back (IHI 0048B) | checked (`LeanboxTest/Dev/Gic`) |
| The PL011's FIFOs never hold more than 32 entries (the bound in their type) and keep order (DDI 0183F) | checked |
| A timer's interrupt is raised at the first tick its condition holds, not before and not after (Arm ARM, generic timer) | checked |
| PL031's count advances by one per second of machine time, from the epoch it was set to (DDI 0224) | checked |

## P5. Programs running on it are correct

Every instruction is a Lean function, so `simp` can run a guest program on
symbolic inputs, the method of LNSym (a Lean model of Arm used to prove
cryptographic code at AWS). This is the payoff: proofs about the software
the machine runs, not only about the machine.

| statement | status |
|---|---|
| A guest `memcpy` loop copies `n` bytes from `src` to `dst` and changes nothing else, for every `n` and non-overlapping buffers | open |
| The perfbench `fnv` loop computes FNV-1a of its buffer | open |

## P6. The specification is Arm's

| statement | status |
|---|---|
| Every encoding `decode` accepts, with its fields, agrees with the Arm ARM's machine-readable A64 encodings (`cpp_orig/arm-docs/isa/`) (L17) | open |
| `Cpu.Step` refines Arm's ASL pseudocode, through Sail's ASL-generated model, once Sail's Lean backend reaches Arm | open (horizon) |

## Features that come from being pure

The machine is a pure function from a state to a state. That makes some
things nearly free that are hard in an emulator written in C or Rust.

- **Time travel.** A machine state is a value, and an old state shares
  everything it has in common with the new one, so keeping a checkpoint is
  keeping a reference. Stepping backwards is a bisection between
  checkpoints, and replay is exact because the function is deterministic.
- **One statement, three uses.** A property is written once as a decidable
  proposition. It runs as a `#guard` on chosen states, as a property test on
  random states, and is proved when the proof exists. The statement never
  changes as it moves from checked to proved.
- **An assembler in Lean.** `encode : Instr → UInt32` makes `Instr` values
  guest code. Test programs and the P5 programs are written as Lean terms,
  type-checked by L15's rules, and assembled with the proven round trip.
- **Hooks with typed conditions** (L18): instrumentation that receives
  exactly what its condition selects, decided once per decode.
- **Proof-carrying traces.** A trace can carry the proof that it is the
  decoding of the memory it came from (P1's coherence), so the engine
  holds evidence for its cache, not hope.
