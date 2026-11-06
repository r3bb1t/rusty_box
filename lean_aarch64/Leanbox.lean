import Leanbox.Bytes
import Leanbox.Isa.Pseudocode
import Leanbox.Isa.Op
import Leanbox.Isa.Decode
import Leanbox.Isa.Alu
import Leanbox.Dev.Flash
import Leanbox.Dev.Gic
import Leanbox.Dev.Pl011
import Leanbox.Dev.Pl031
import Leanbox.Dev.Pl061
import Leanbox.Dev.VirtioMmio
import Leanbox.Board.Map
import Leanbox.Board.Basic
import Leanbox.Cpu.State
import Leanbox.Cpu.Timer
import Leanbox.Cpu.Mmu
import Leanbox.Cpu.Exception
import Leanbox.Cpu.Memory
import Leanbox.Cpu.Sysreg
import Leanbox.Cpu.System
import Leanbox.Cpu.Simd
import Leanbox.Cpu.Step
import Leanbox.Engine.Poll
import Leanbox.Engine.Trace
import Leanbox.Engine.Run
import Leanbox.Engine.Machine
import Leanbox.Boot
import Leanbox.Lockstep
/-!
# Leanbox: an Arm computer in Lean

Leanbox is a whole computer written in Lean 4: a 64-bit Arm processor (a
Cortex-A57, Armv8.0-A) on the `virt` board, with the interrupt controller,
the serial port, the clock, the GPIO and the flash. It boots real software,
EDK2's UEFI firmware to its shell and Alpine Linux to a root shell, and at
every step it is in exactly the state rusty_box_aarch64, the Rust machine it
is checked against, would be in.

The library is meant to be read in the order of the imports above, each part
resting only on the parts before it:

* **Bytes** (`Leanbox.Bytes`). A computer's memory is a row of bytes. How
  words are read from and written to one, with every index proven in
  bounds, so the program cannot read outside an array.
* **The instruction set** (`Leanbox.Isa`). What an instruction *means*,
  before any machine exists: the Arm ARM's shared pseudocode
  (`Isa.Pseudocode`), instructions as data (`Isa.Op`), the decoder from a
  32-bit word to one (`Isa.Decode`), and the values of arithmetic and logic
  (`Isa.Alu`), where the fast forms of bit operations are proven equal to
  their definitions.
* **The board** (`Leanbox.Dev`, `Leanbox.Board`). The devices around the
  processor, each a pure state machine with its registers (`Dev`), and the
  board that places them at addresses and wires their interrupts (`Board`).
* **The processor** (`Leanbox.Cpu`). Its state, time, address translation,
  exceptions, loads and stores, system registers and instructions, SIMD,
  and finally `Cpu.Step.execOp`: one instruction, completely. This is the
  reference: whatever else runs must agree with it.
* **The engine** (`Leanbox.Engine`). How the reference is made fast without
  changing an answer: instructions decoded once into cached traces
  (`Engine.Trace`), one tail-recursive interpreter loop that runs them
  (`Engine.Run`), polls of the clock skipped exactly (`Engine.Poll`), and the
  run loop that keeps devices and time up to date (`Engine.Machine`).
* **Running it** (`Leanbox.Boot`, `Leanbox.Lockstep`). The machine's start
  states, and the lines it prints so that a run can be compared, line by
  line, with rusty_box's.

Everything is pure Lean: no `@[extern]`, `implemented_by`, `unsafe` or
`sorry`, so every function is either proved or part of Lean's own runtime.
Importing this module imports the whole library, so `lake build` checks
every proof in it; `lake test` runs the devices' tests (`LeanboxTest`).
-/
