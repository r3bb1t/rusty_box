import Leanbox.Dev.Pl011
/-!
# PL011 checks

The pl011.rs unit tests as `#guard`s, checked when this module is built (`lake test`).
-/
namespace Leanbox.Dev.Pl011

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
