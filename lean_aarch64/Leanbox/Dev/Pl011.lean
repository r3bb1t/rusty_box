/-
  Arm PrimeCell PL011 UART: an exact port of
  rusty_box_aarch64/src/devices/pl011.rs (plus the board.rs glue around its
  MMIO accesses), for lockstep runs against the Rust machine.

  Timing model (pl011.rs module docs, "Time on the line"): characters take
  no time to cross the line. A `UARTDR` write while transmission is enabled
  (`UARTEN` and `TXE`) leaves the transmit FIFO at once, inside the same
  access, so `UARTFR.TXFE` is set and `BUSY`/`TXFF` clear when the guest next
  looks. The only timed interval is the receive timeout (`RTRIS`), 32 bit
  periods = 8 x divisor-in-64ths cycles of UARTCLK, converted to machine
  ticks rounded up.
-/

namespace Leanbox.Dev.Pl011

/-! ## Register offsets (pl011.rs `UARTDR` ... `UARTPCELLID3`) -/

def UARTDR : UInt64 := 0x000
def UARTRSR : UInt64 := 0x004
def UARTFR : UInt64 := 0x018
def UARTILPR : UInt64 := 0x020
def UARTIBRD : UInt64 := 0x024
def UARTFBRD : UInt64 := 0x028
def UARTLCR_H : UInt64 := 0x02C
def UARTCR : UInt64 := 0x030
def UARTIFLS : UInt64 := 0x034
def UARTIMSC : UInt64 := 0x038
def UARTRIS : UInt64 := 0x03C
def UARTMIS : UInt64 := 0x040
def UARTICR : UInt64 := 0x044
def UARTDMACR : UInt64 := 0x048
def UARTPERIPHID0 : UInt64 := 0xFE0
def UARTPCELLID3 : UInt64 := 0xFFC

-- pl011.rs PL011_ID
def PL011_ID : Array UInt32 := #[0x11, 0x10, 0x14, 0x00, 0x0D, 0xF0, 0x05, 0xB1]

/-! ## Bits (pl011.rs FLAG_*, DR_*, RSR_*, INT_*, LCR_*, CR_*, *_MASK) -/

def FLAG_RI : UInt32 := 0x100
def FLAG_TXFE : UInt32 := 0x80
def FLAG_RXFF : UInt32 := 0x40
def FLAG_TXFF : UInt32 := 0x20
def FLAG_RXFE : UInt32 := 0x10
def FLAG_BUSY : UInt32 := 0x08
def FLAG_DCD : UInt32 := 0x04
def FLAG_DSR : UInt32 := 0x02
def FLAG_CTS : UInt32 := 0x01
def FLAG_MODEM : UInt32 := FLAG_RI ||| FLAG_DCD ||| FLAG_DSR ||| FLAG_CTS
def FR_MASK : UInt32 := 0x1FF

def DR_OE : UInt32 := (1 : UInt32) <<< 11
def DR_BE : UInt32 := (1 : UInt32) <<< 10
def RSR_OE : UInt32 := (1 : UInt32) <<< 3
def RSR_CHARACTER_ERRORS : UInt32 := 0x7

def INT_OE : UInt32 := (1 : UInt32) <<< 10
def INT_BE : UInt32 := (1 : UInt32) <<< 9
def INT_RT : UInt32 := (1 : UInt32) <<< 6
def INT_TX : UInt32 := (1 : UInt32) <<< 5
def INT_RX : UInt32 := (1 : UInt32) <<< 4
def INT_DSR : UInt32 := (1 : UInt32) <<< 3
def INT_DCD : UInt32 := (1 : UInt32) <<< 2
def INT_CTS : UInt32 := (1 : UInt32) <<< 1
def INT_RI : UInt32 := 1

def LCR_WLEN_8 : UInt32 := (0b11 : UInt32) <<< 5
def LCR_FEN : UInt32 := (1 : UInt32) <<< 4
def LCR_BRK : UInt32 := 1

def CR_OUT2 : UInt32 := (1 : UInt32) <<< 13
def CR_OUT1 : UInt32 := (1 : UInt32) <<< 12
def CR_RTS : UInt32 := (1 : UInt32) <<< 11
def CR_DTR : UInt32 := (1 : UInt32) <<< 10
def CR_RXE : UInt32 := (1 : UInt32) <<< 9
def CR_TXE : UInt32 := (1 : UInt32) <<< 8
def CR_LBE : UInt32 := (1 : UInt32) <<< 7
def CR_UARTEN : UInt32 := 1

def ILPR_MASK : UInt32 := 0xFF
def IBRD_MASK : UInt32 := 0xFFFF
def FBRD_MASK : UInt32 := 0x3F
def LCR_MASK : UInt32 := 0xFF
def CR_MASK : UInt32 := 0xFF87
def IFLS_MASK : UInt32 := 0x3F
def IMSC_MASK : UInt32 := 0x7FF
def DMACR_MASK : UInt32 := 0x7

-- pl011.rs RX_FIFO_DEPTH / TX_FIFO_DEPTH
def RX_FIFO_DEPTH : UInt32 := 16
def TX_FIFO_DEPTH : UInt32 := 16
-- pl011.rs OVERRUN_PENDING (kept in bit 31 of `flags`)
def OVERRUN_PENDING : UInt32 := (1 : UInt32) <<< 31
-- pl011.rs TX_CAPACITY (the host-side RingBuffer of transmitted bytes)
def TX_CAPACITY : Nat := 64 * 1024

-- board.rs UART_CLOCK
def UART_CLOCK : Nat := 24_000_000
-- rusty_box_core time.rs ClockHz::BOCHS_DEFAULT
def MACHINE_RATE : Nat := 50_000_000

/-! ## Time helpers (rusty_box_core/src/time.rs) -/

-- time.rs `saturate` (u128 -> u64, clamping)
def saturate (n : Nat) : UInt64 :=
  if n > UInt64.size - 1 then UInt64.ofNat (UInt64.size - 1) else UInt64.ofNat n

-- time.rs ClockHz::ticks_from_cycles_ceil: ceil(cycles * rate / cycle_rate), saturating
def ticksFromCyclesCeil (rate cycles cycleRate : Nat) : UInt64 :=
  saturate ((cycles * rate + cycleRate - 1) / cycleRate)

-- time.rs VmInstant::add (saturating_add)
def instantAdd (at_ : UInt64) (span : UInt64) : UInt64 :=
  saturate (at_.toNat + span.toNat)

-- pl011.rs UartClocks { machine: VmClock { now, rate }, uartclk }
structure Clocks where
  now : UInt64
  rate : Nat := MACHINE_RATE
  uartclk : Nat := UART_CLOCK
deriving Inhabited

-- board.rs uart_clocks(clock) for a machine at tick `now`, 50 MHz
def Clocks.atTick (now : UInt64) : Clocks := { now := now }

/-! ## Baud divisor (pl011.rs BaudDivisor) -/

-- BaudDivisor::SLOWEST
def DIVISOR_SLOWEST : UInt32 := IBRD_MASK <<< 6
-- BaudDivisor::of_registers
def divisorOfRegisters (ibrd fbrd : UInt32) : UInt32 :=
  ((ibrd &&& IBRD_MASK) <<< 6) ||| (fbrd &&& FBRD_MASK)
-- BaudDivisor::for_rate (u64 arithmetic: uartclk*4/baud, then u32::try_from)
def divisorForRate (uartclk : Nat) (baud : Nat) : UInt32 :=
  let sixtyFourths := (uartclk * 4) / baud
  if sixtyFourths ≤ DIVISOR_SLOWEST.toNat then UInt32.ofNat sixtyFourths else DIVISOR_SLOWEST
-- BaudDivisor::integer
def divisorInteger (d : UInt32) : UInt32 := d >>> 6
-- BaudDivisor::fraction
def divisorFraction (d : UInt32) : UInt32 := d &&& FBRD_MASK

/-! ## State (pl011.rs `struct Pl011`) -/

structure State where
  /-- `tx`: bytes transmitted, not yet drained by the host. Unbounded here;
  `takeOutput` returns only the newest `TX_CAPACITY`, which is exactly what
  draining the Rust 64 KiB drop-oldest ring yields. -/
  out : ByteArray
  /-- `tx_fifo` (16 slots; `tx_fifo[..tx_count]` waiting). -/
  txFifo : Array UInt8
  txCount : UInt8
  /-- `read_fifo` (16 slots; stale slots are guest-visible on empty reads). -/
  readFifo : Array UInt32
  readPos : UInt8
  readCount : UInt32
  /-- `divisor` as latched by the last `UARTLCR_H` write. -/
  divisor : UInt32
  /-- `rx_timeout`, in machine ticks. -/
  rxTimeout : Option UInt64
  flags : UInt32
  rsr : UInt32
  ilpr : UInt32
  ibrd : UInt32
  fbrd : UInt32
  lcr : UInt32
  cr : UInt32
  ifl : UInt32
  intEnabled : UInt32
  intLevel : UInt32
  dmacr : UInt32
deriving Inhabited

-- pl011.rs Pl011::new. This is also the power-on state on the firmware
-- boot path: board.rs VirtBoard::new builds `Pl011::new()`, machine.rs
-- `start` then calls `reset_devices` (Pl011::reset on a fresh device: a no-op)
-- and pumps host input (none); only the direct-kernel path calls
-- set_up_console.
def State.new : State :=
  { out := ByteArray.empty
    txFifo := Array.replicate 16 0
    txCount := 0
    readFifo := Array.replicate 16 0
    readPos := 0
    readCount := 0
    divisor := divisorOfRegisters 0 0
    rxTimeout := none
    flags := FLAG_RXFE ||| FLAG_TXFE
    rsr := 0
    ilpr := 0
    ibrd := 0
    fbrd := 0
    lcr := 0
    cr := 0x300
    ifl := 0x12
    intEnabled := 0
    intLevel := 0
    dmacr := 0 }

namespace State

-- pl011.rs Pl011::reset: registers to reset values, keeping the transmitted
-- bytes for the host and the receive FIFO's slot contents.
def reset (s : State) : State :=
  { State.new with out := s.out, readFifo := s.readFifo }

-- pl011.rs Pl011::irq
def irq (s : State) : Bool := s.intLevel &&& s.intEnabled != 0

-- pl011.rs `self.tx.push_back(byte)` (RingBuffer<u8, TX_CAPACITY>).
-- Amortised trimming keeps memory bounded without changing what a drain
-- returns (the newest TX_CAPACITY bytes).
def pushOut (s : State) (b : UInt8) : State :=
  let o := s.out.push b
  let o := if o.size ≥ 2 * TX_CAPACITY then o.extract (o.size - TX_CAPACITY) o.size else o
  { s with out := o }

-- pl011.rs Pl011::drain_tx / machine.rs Serial::take_output
def takeOutput (s : State) : State × ByteArray :=
  let o := s.out
  let o := if o.size > TX_CAPACITY then o.extract (o.size - TX_CAPACITY) o.size else o
  ({ s with out := ByteArray.empty }, o)

-- pl011.rs Pl011::fifo_depth
def fifoDepth (s : State) : UInt32 :=
  if s.lcr &&& LCR_FEN != 0 then RX_FIFO_DEPTH else 1

-- the eighths table shared by rx_trigger / tx_trigger
def eighthsOf (sel : UInt32) : UInt32 :=
  if sel == 0 then 1 else if sel == 1 then 2 else if sel == 3 then 6
  else if sel == 4 then 7 else 4

-- pl011.rs Pl011::rx_trigger
def rxTrigger (s : State) : UInt32 :=
  if s.lcr &&& LCR_FEN == 0 then 1
  else RX_FIFO_DEPTH * eighthsOf ((s.ifl >>> 3) &&& 7) / 8

-- pl011.rs Pl011::tx_trigger
def txTrigger (s : State) : UInt32 :=
  if s.lcr &&& LCR_FEN == 0 then 0
  else TX_FIFO_DEPTH * eighthsOf (s.ifl &&& 7) / 8

-- pl011.rs Pl011::tx_level_moved
def txLevelMoved (s : State) (before : UInt32) : State :=
  let level := s.txTrigger
  let after := s.txCount.toUInt32
  if before > level && after ≤ level then { s with intLevel := s.intLevel ||| INT_TX }
  else if before ≤ level && after > level then { s with intLevel := s.intLevel &&& ~~~INT_TX }
  else s

-- pl011.rs Pl011::loopback
def loopback (s : State) : Bool := s.cr &&& CR_LBE != 0
-- pl011.rs Pl011::receiving
def receiving (s : State) : Bool := s.cr &&& (CR_UARTEN ||| CR_RXE) == CR_UARTEN ||| CR_RXE
-- pl011.rs Pl011::transmitting
def transmitting (s : State) : Bool := s.cr &&& (CR_UARTEN ||| CR_TXE) == CR_UARTEN ||| CR_TXE

-- pl011.rs Pl011::tx_flags
def txFlags (s : State) : UInt32 :=
  if s.txCount == 0 then FLAG_TXFE
  else if s.txCount.toUInt32 == s.fifoDepth then FLAG_TXFF ||| FLAG_BUSY
  else FLAG_BUSY

-- pl011.rs Pl011::timeout_cycles (UARTCLK cycles for 32 bit periods)
def timeoutCycles (s : State) : Option Nat :=
  let ibrd := divisorInteger s.divisor
  if ibrd == 0 || (ibrd == IBRD_MASK && divisorFraction s.divisor != 0) then none
  else some (8 * s.divisor.toNat)

-- pl011.rs Pl011::start_rx_timeout
def startRxTimeout (s : State) (c : Clocks) : State :=
  { s with rxTimeout := s.timeoutCycles.map fun cyc =>
      instantAdd c.now (ticksFromCyclesCeil c.rate cyc c.uartclk) }

-- pl011.rs Pl011::update
def updateC (s : State) (now : UInt64) : State :=
  match s.rxTimeout with
  | some at_ => if now ≥ at_ then { s with rxTimeout := none, intLevel := s.intLevel ||| INT_RT } else s
  | none => s

-- pl011.rs Pl011::update(clock) (only `clock.now()` is used)
def update (s : State) (now : UInt64) : State := s.updateC now

-- pl011.rs Pl011::next_deadline (ticks)
def nextDeadline (s : State) : Option UInt64 := s.rxTimeout

-- pl011.rs Pl011::rx_room
def rxRoom (s : State) : UInt32 :=
  if !s.loopback && !s.receiving then 0
  else if s.fifoDepth ≥ s.readCount then s.fifoDepth - s.readCount else 0

-- pl011.rs Pl011::can_receive
def canReceive (s : State) : Bool := s.rxRoom > 0

-- pl011.rs Pl011::receive_character
def receiveCharacter (s : State) (value : UInt32) (c : Clocks) : State :=
  let s := if value &&& DR_BE != 0 then { s with intLevel := s.intLevel ||| INT_BE } else s
  let s := if s.intEnabled &&& INT_RT != 0 then s.startRxTimeout c else s
  let depth := s.fifoDepth
  if s.readCount == depth then
    -- note_overrun
    { s with flags := s.flags ||| OVERRUN_PENDING
             rsr := s.rsr ||| RSR_OE
             intLevel := s.intLevel ||| INT_OE }
  else
    -- take_overrun
    let pending := s.flags &&& OVERRUN_PENDING != 0
    let s := { s with flags := s.flags &&& ~~~OVERRUN_PENDING }
    let entry := if pending then value ||| DR_OE else value
    let slot := (s.readPos.toUInt32 + s.readCount) &&& (depth - 1)
    let s := { s with readFifo := s.readFifo.set! slot.toNat entry
                      readCount := s.readCount + 1
                      flags := s.flags &&& ~~~FLAG_RXFE }
    let s := if s.readCount == depth then { s with flags := s.flags ||| FLAG_RXFF } else s
    if s.readCount == s.rxTrigger then { s with intLevel := s.intLevel ||| INT_RX } else s

-- pl011.rs Pl011::receive (true = Received::Taken, false = Received::NoRoom).
-- Only host input reaches this; our runs have none, so it is used by tests.
def receive (s : State) (byte : UInt8) (c : Clocks) : State × Bool :=
  if !s.canReceive then (s, false)
  else if !s.loopback then (s.receiveCharacter byte.toUInt32 c, true)
  else (s, true)

-- pl011.rs Pl011::loopback_tx
def loopbackTx (s : State) (value : UInt32) (c : Clocks) : State :=
  if s.loopback && s.receiving then s.receiveCharacter value c else s

-- pl011.rs Pl011::transmit
def transmit (s : State) (c : Clocks) : State :=
  let s :=
    if s.transmitting && s.txCount > 0 then
      let before := s.txCount.toUInt32
      let s := (List.range s.txCount.toNat).foldl
        (fun s i =>
          let byte := s.txFifo[i]!
          let s := s.pushOut byte
          s.loopbackTx byte.toUInt32 c) s
      let s := { s with txCount := 0 }
      s.txLevelMoved before
    else s
  let s := { s with flags := s.flags &&& ~~~(FLAG_TXFE ||| FLAG_TXFF ||| FLAG_BUSY) }
  { s with flags := s.flags ||| s.txFlags }

-- pl011.rs Pl011::write_txdata
def writeTxdata (s : State) (byte : UInt8) (c : Clocks) : State :=
  if s.txCount.toUInt32 == s.fifoDepth then s
  else
    let before := s.txCount.toUInt32
    let s := { s with txFifo := s.txFifo.set! s.txCount.toNat byte, txCount := s.txCount + 1 }
    let s := s.txLevelMoved before
    s.transmit c

-- pl011.rs Pl011::read_rxdata
def readRxdata (s : State) : State × UInt32 :=
  let depth := s.fifoDepth
  let s := { s with flags := s.flags &&& ~~~FLAG_RXFF }
  let ch := s.readFifo[s.readPos.toNat]!
  let s := if s.readCount > 0 then
      { s with readCount := s.readCount - 1
               readPos := (s.readPos + 1) &&& (depth.toUInt8 - 1) }
    else s
  let s := if s.readCount == 0 then
      { s with flags := s.flags ||| FLAG_RXFE
               intLevel := s.intLevel &&& ~~~INT_RT
               rxTimeout := none }
    else s
  let s := if s.readCount < s.rxTrigger then { s with intLevel := s.intLevel &&& ~~~INT_RX } else s
  let s := { s with rsr := (s.rsr &&& RSR_OE) ||| ((ch >>> 8) &&& RSR_CHARACTER_ERRORS) }
  (s, ch)

-- pl011.rs Pl011::reset_fifos (note: uses the OLD lcr for tx_trigger)
def resetFifos (s : State) : State :=
  let s := { s with readCount := 0, readPos := 0 }
  let before := s.txCount.toUInt32
  let s := { s with txCount := 0 }
  let s := s.txLevelMoved before
  let s := { s with flags := s.flags &&& ~~~(FLAG_RXFF ||| FLAG_TXFF ||| FLAG_BUSY) }
  { s with flags := s.flags ||| FLAG_RXFE ||| FLAG_TXFE
           intLevel := s.intLevel &&& ~~~(INT_RT ||| INT_RX)
           rxTimeout := none }

-- pl011.rs Pl011::loopback_mdmctrl
def loopbackMdmctrl (s : State) : State :=
  if !s.loopback then s
  else
    let cr := s.cr
    let pairs : List (UInt32 × UInt32) :=
      [(CR_OUT2, FLAG_RI), (CR_OUT1, FLAG_DCD), (CR_RTS, FLAG_CTS), (CR_DTR, FLAG_DSR)]
    let status := pairs.foldl (fun st (ctl, inp) => if cr &&& ctl != 0 then st ||| inp else st) 0
    let changed := (s.flags ^^^ status) &&& FLAG_MODEM
    let ints : List (UInt32 × UInt32) :=
      [(FLAG_DSR, INT_DSR), (FLAG_DCD, INT_DCD), (FLAG_CTS, INT_CTS), (FLAG_RI, INT_RI)]
    let lvl := ints.foldl (fun l (inp, i) => if changed &&& inp != 0 then l ||| i else l) s.intLevel
    { s with intLevel := lvl, flags := (s.flags &&& ~~~FLAG_MODEM) ||| status }

-- pl011.rs Pl011::set_interrupt_mask
def setInterruptMask (s : State) (v : UInt32) (c : Clocks) : State :=
  let mask := v &&& IMSC_MASK
  let s :=
    if mask &&& INT_RT == 0 then { s with intLevel := s.intLevel &&& ~~~INT_RT, rxTimeout := none }
    else if s.intEnabled &&& INT_RT == 0 && s.readCount > 0 then s.startRxTimeout c
    else s
  { s with intEnabled := mask }

-- pl011.rs register_at
def registerAt (offset : UInt64) : UInt64 := offset &&& ~~~3

-- pl011.rs Pl011::read_register (QEMU pl011_read)
def readRegister (s : State) (offset : UInt64) : State × UInt32 :=
  let r := registerAt offset
  if r == UARTDR then s.readRxdata
  else if r == UARTRSR then (s, s.rsr)
  else if r == UARTFR then (s, s.flags &&& FR_MASK)
  else if r == UARTILPR then (s, s.ilpr)
  else if r == UARTIBRD then (s, s.ibrd)
  else if r == UARTFBRD then (s, s.fbrd)
  else if r == UARTLCR_H then (s, s.lcr)
  else if r == UARTCR then (s, s.cr)
  else if r == UARTIFLS then (s, s.ifl)
  else if r == UARTIMSC then (s, s.intEnabled)
  else if r == UARTRIS then (s, s.intLevel)
  else if r == UARTMIS then (s, s.intLevel &&& s.intEnabled)
  else if r == UARTDMACR then (s, s.dmacr)
  else if UARTPERIPHID0 ≤ r && r ≤ UARTPCELLID3 then
    (s, PL011_ID[((r - UARTPERIPHID0) >>> 2).toNat]!)
  else (s, 0)

-- pl011.rs Pl011::write_register (QEMU pl011_write)
def writeRegister (s : State) (offset : UInt64) (v : UInt32) (c : Clocks) : State :=
  let r := registerAt offset
  if r == UARTDR then s.writeTxdata v.toUInt8 c
  else if r == UARTRSR then { s with rsr := 0 }
  else if r == UARTILPR then { s with ilpr := v &&& ILPR_MASK }
  else if r == UARTIBRD then { s with ibrd := v &&& IBRD_MASK }
  else if r == UARTFBRD then { s with fbrd := v &&& FBRD_MASK }
  else if r == UARTLCR_H then
    let v := v &&& LCR_MASK
    let s := if (s.lcr ^^^ v) &&& LCR_FEN != 0 then s.resetFifos else s
    -- pl011_loopback_break, judged with the old lcr still in place
    let s := if (s.lcr ^^^ v) &&& LCR_BRK != 0 && v &&& LCR_BRK != 0 then s.loopbackTx DR_BE c else s
    let s := { s with lcr := v }
    { s with divisor := divisorOfRegisters s.ibrd s.fbrd }
  else if r == UARTCR then
    let s := { s with cr := v &&& CR_MASK }
    let s := s.loopbackMdmctrl
    s.transmit c
  else if r == UARTIFLS then { s with ifl := v &&& IFLS_MASK }
  else if r == UARTIMSC then s.setInterruptMask v c
  else if r == UARTICR then { s with intLevel := s.intLevel &&& ~~~v }
  else if r == UARTDMACR then { s with dmacr := v &&& DMACR_MASK }
  else s

-- pl011.rs Pl011::read via bus.rs read_by_words: an 8-byte access (log2 = 3)
-- is `offset` then `offset + 4`; any other width is one register, unshifted.
def readC (s : State) (off : UInt64) (log2 : UInt8) : State × UInt64 :=
  if log2 == 3 then
    let (s, lo) := s.readRegister off
    let (s, hi) := s.readRegister (off + 4)
    (s, lo.toUInt64 ||| (hi.toUInt64 <<< 32))
  else
    let (s, v) := s.readRegister off
    (s, v.toUInt64)

-- pl011.rs Pl011::write via bus.rs write_by_words (`word as u32`)
def writeC (s : State) (off : UInt64) (log2 : UInt8) (v : UInt64) (c : Clocks) : State :=
  if log2 == 3 then
    let s := s.writeRegister off (v &&& 0xFFFF_FFFF).toUInt32 c
    s.writeRegister (off + 4) (v >>> 32).toUInt32 c
  else
    s.writeRegister off v.toUInt32 c

-- board.rs VirtBoard::pump_serial_input_since moves bytes from the host
-- input buffer (`serial_input`), which is the board's, into the receive FIFO:
-- the board does it (Cpu/Bus.lean `pumpSerialSince`) after these, so the
-- device's own step is the identity.
def pumpSerialInput (s : State) (_now : UInt64) : State := s

-- board.rs read_outside_ram, UART arm: uart.read, pump_serial_input_since,
-- sync_device_lines (the line is `irq`).
def read (s : State) (off : UInt64) (log2 : UInt8) (now : UInt64) : State × UInt64 :=
  let (s, v) := s.readC off log2
  (s.pumpSerialInput now, v)

-- board.rs write_outside_ram, UART arm: uart.write(uart_clocks(clock)),
-- pump_serial_input_since, sync_device_lines.
def write (s : State) (off : UInt64) (log2 : UInt8) (v : UInt64) (now : UInt64) : State :=
  (s.writeC off log2 v (Clocks.atTick now)).pumpSerialInput now

-- pl011.rs Pl011::set_up_console (direct-kernel boot only; NOT in State.new)
def setUpConsoleC (s : State) (baud : Nat) (c : Clocks) : State :=
  let d := divisorForRate c.uartclk baud
  let s := s.writeRegister UARTIBRD (divisorInteger d) c
  let s := s.writeRegister UARTFBRD (divisorFraction d) c
  let s := s.writeRegister UARTLCR_H (LCR_WLEN_8 ||| LCR_FEN) c
  s.writeRegister UARTCR (CR_UARTEN ||| CR_TXE ||| CR_RXE) c

-- board.rs VirtBoard::set_up_console at tick `now` (boot.rs: 115200 baud)
def setUpConsole (s : State) (baud : Nat) (now : UInt64) : State :=
  (s.setUpConsoleC baud (Clocks.atTick now)).pumpSerialInput now

end State

/-! ## Checks reproducing pl011.rs unit tests -/

namespace Tests
open State

-- pl011.rs tests `at(ticks)`: a machine clock as fast as UARTCLK (24 MHz)
def at24 (t : UInt64) : Clocks := { now := t, rate := UART_CLOCK }
def wr (s : State) (n : UInt64) (v : UInt32) (t : UInt64 := 0) : State :=
  s.writeC (n * 4) 2 v.toUInt64 (at24 t)
def rd (s : State) (n : UInt64) : State × UInt32 :=
  let (s, v) := s.readC (n * 4) 2
  (s, v.toUInt32)
def rdv (s : State) (n : UInt64) : UInt32 := (rd s n).2

def DR : UInt64 := 0
def RSR : UInt64 := 1
def FR : UInt64 := 6
def LCR_H : UInt64 := 11
def CR : UInt64 := 12
def IFLS : UInt64 := 13
def IMSC : UInt64 := 14
def RIS : UInt64 := 15
def MIS : UInt64 := 16
def ICR : UInt64 := 17
def ENABLED : UInt32 := 0x300 ||| CR_UARTEN
def TXBITS : UInt32 := FLAG_TXFE ||| FLAG_TXFF ||| FLAG_BUSY
def TIMEOUT_13_1 : UInt64 := 512 * 13 + 512 / 64

def enabled : State := wr State.new CR ENABLED
def timed : State := wr (wr (wr enabled 9 13) 10 1) LCR_H LCR_FEN

-- reset values (Pl011::new) and
-- a_narrower_access_reaches_the_register_that_holds_it
#guard rdv State.new CR == 0x300
#guard rdv State.new FR == 0x90
#guard rdv State.new IFLS == 0x12
#guard ((List.range 8).map fun i =>
    (State.new.read (0xFE0 + 4 * i.toUInt64 + 3) 0 0).2) ==
  [0x11, 0x10, 0x14, 0x00, 0x0D, 0xF0, 0x05, 0xB1]
#guard (State.new.read 0xFDC 2 0).2 == 0
#guard ((State.new.write 0x31 0 ENABLED.toUInt64 0).read 0x32 1 0).2 == ENABLED.toUInt64

-- the_transmit_fifo_holds_what_is_written_until_uarten_and_txe_are_set
def tTxFifoHolds (off : UInt32) : Bool := Id.run do
  let mut u := wr (wr (wr State.new LCR_H LCR_FEN) CR off) IMSC INT_TX
  let mut ok := rdv u FR &&& TXBITS == FLAG_TXFE
  for i in [0:16] do
    u := wr u DR (0x61 + i.toUInt32)
    let full := if i + 1 == 16 then FLAG_TXFF else 0
    ok := ok && rdv u FR &&& TXBITS == FLAG_BUSY ||| full
  u := wr u DR 0x21
  let (u1, o1) := u.takeOutput
  u := u1
  ok := ok && o1.size == 0 && rdv u RIS &&& INT_TX == 0
  u := wr u CR ENABLED
  let (u2, o2) := u.takeOutput
  u := u2
  ok := ok && o2.toList == "abcdefghijklmnop".toUTF8.toList
  ok := ok && rdv u FR &&& TXBITS == FLAG_TXFE && rdv u RIS &&& INT_TX == INT_TX && u.irq
  u := wr u DR 0x7A
  let (_, o3) := u.takeOutput
  return ok && o3.toList == [0x7A]
#guard tTxFifoHolds 0x300
#guard tTxFifoHolds (CR_UARTEN ||| CR_RXE)
#guard tTxFifoHolds (CR_TXE ||| CR_RXE)

-- a_burst_longer_than_the_fifo_goes_out_whole (TXFF never seen, TXFE after)
def tBurst : Bool := Id.run do
  let mut u := wr (wr (wr State.new LCR_H (LCR_WLEN_8 ||| LCR_FEN)) IMSC INT_TX) CR ENABLED
  let mut ok := true
  for i in [0:64] do
    ok := ok && rdv u FR &&& FLAG_TXFF == 0
    u := wr u DR (0x41 + (i % 26).toUInt32)
  let (u', o) := u.takeOutput
  return ok && o.size == 64 && rdv u' FR &&& TXBITS == FLAG_TXFE
#guard tBurst

-- with_the_fifo_disabled_the_transmit_interrupt_marks_the_holding_register_emptying
def tHoldingTx : Bool := Id.run do
  let mut u := wr (wr State.new IMSC INT_TX) CR ENABLED
  let mut ok := rdv u RIS &&& INT_TX == 0
  u := wr u DR 0x31
  ok := ok && rdv u RIS &&& INT_TX == INT_TX && u.irq
  u := wr (wr u ICR INT_TX) DR 0x32
  ok := ok && rdv u RIS &&& INT_TX == INT_TX
  u := wr (wr u CR 0x300) DR 0x33
  ok := ok && rdv u RIS &&& INT_TX == 0
  u := wr u CR ENABLED
  ok := ok && rdv u RIS &&& INT_TX == INT_TX
  return ok && u.takeOutput.2.toList == "123".toUTF8.toList
#guard tHoldingTx

-- a_console_set_up_runs_at_the_rate_asked_with_everything_enabled
def tConsole : Bool := Id.run do
  let u := State.new.setUpConsoleC 115200 (at24 0)
  let slow := State.new.setUpConsoleC 1 (at24 0)
  let ok := [9, 10, LCR_H, CR].map (rdv u) == [13, 1, 0x70, 0x301]
    && [9, 10].map (rdv slow) == [0xFFFF, 0]
  let u := wr u DR 0x6B
  let (u, o) := u.takeOutput
  let (u, taken) := u.receive 0x72 (at24 0)
  return ok && o.toList == [0x6B] && taken && (rd u DR).2 == 0x72
#guard tConsole

-- the_receive_timeout_expires_32_bit_periods_after_the_last_arrival
def tRxTimeout : Bool := Id.run do
  let mut u := wr timed IMSC INT_RT
  let (u1, t1) := u.receive 0x61 (at24 100)
  u := u1
  let mut ok := t1 && u.nextDeadline == some (100 + TIMEOUT_13_1)
  u := u.update (100 + TIMEOUT_13_1 - 1)
  ok := ok && rdv u RIS &&& INT_RT == 0
  let (u2, t2) := u.receive 0x62 (at24 1000)
  u := u2
  ok := ok && t2 && u.nextDeadline == some (1000 + TIMEOUT_13_1)
  u := u.update (100 + TIMEOUT_13_1)
  ok := ok && rdv u RIS &&& INT_RT == 0
  u := u.update (1000 + TIMEOUT_13_1)
  ok := ok && rdv u RIS &&& INT_RT == INT_RT && rdv u MIS == INT_RT && u.irq
    && u.nextDeadline == none
  let (u3, a) := rd u DR
  u := u3
  ok := ok && a == 0x61 && rdv u RIS &&& INT_RT == INT_RT
  let (u4, b) := rd u DR
  return ok && b == 0x62 && rdv u4 RIS &&& INT_RT == 0
#guard tRxTimeout

-- the_receive_timeout_ends_at_the_first_whole_tick_after_32_bit_periods
-- (machine at 50 MHz: 6664 UARTCLK cycles = 13883.3 ticks -> 13884)
def tRxTimeout50 : Bool :=
  let u := wr timed IMSC INT_RT
  let (u, taken) := u.receive 0x61 { now := 7 }
  taken && u.nextDeadline == some (7 + 13884)
    && rdv (u.update (7 + 13883)) RIS &&& INT_RT == 0
    && rdv (u.update (7 + 13884)) RIS &&& INT_RT == INT_RT
#guard tRxTimeout50

-- a_full_fifo_keeps_its_entries_and_flags_the_overrun (loopback)
def tOverrun : Bool := Id.run do
  let mut u := wr (wr (wr enabled LCR_H LCR_FEN) CR (ENABLED ||| CR_LBE)) IMSC INT_OE
  for i in [0:16] do
    u := wr u DR i.toUInt32
  let mut ok := rdv u RIS &&& INT_OE == 0
  u := wr u DR 0x40
  ok := ok && rdv u RSR == RSR_OE && rdv u RIS &&& INT_OE == INT_OE && u.irq
    && rdv u FR &&& (FLAG_RXFF ||| FLAG_RXFE) == FLAG_RXFF
  let (u1, first) := rd u DR
  u := u1
  ok := ok && first == 0 && rdv u RSR == RSR_OE
  u := wr u DR 0x41
  for i in [1:16] do
    let (u2, e) := rd u DR
    u := u2
    ok := ok && e == i.toUInt32
  let (u3, last) := rd u DR
  u := u3
  ok := ok && last == DR_OE ||| 0x41 && rdv u FR &&& FLAG_RXFE == FLAG_RXFE
  return ok
#guard tOverrun

-- an_eight_byte_access_is_two_registers
def tDword : Bool :=
  let u := wr (wr enabled CR (ENABLED ||| CR_LBE)) LCR_H LCR_BRK
  let (u, pair) := u.readC 0 3
  let u := u.writeC (13 * 4) 3 0x0000_0050_0000_0012 (at24 0)
  pair == DR_BE.toUInt64 ||| ((DR_BE >>> 8).toUInt64 <<< 32)
    && rdv u 13 == 0x12 && rdv u IMSC == 0x50
#guard tDword

-- reserved_register_bits_read_as_zero
def tReserved : Bool := Id.run do
  let mut u := State.new
  let mut ok := true
  for (n, defined) in [(8, 0xFF), (9, 0xFFFF), (10, 0x3F), (11, 0xFF), (12, 0xFF87),
                       (13, 0x3F), (14, 0x7FF), (18, 0x7)] do
    u := wr u n 0xFFFF_FFFF
    ok := ok && rdv u n == defined
  u := wr (wr (wr u ICR 0xFFFF_FFFF) LCR_H 0) LCR_H LCR_BRK
  let (u', d) := rd u DR
  return ok && d == 0x400 && rdv u' RSR == 0x4 && rdv u' RIS &&& ~~~0x7FF == 0
#guard tReserved

end Tests

end Leanbox.Dev.Pl011
