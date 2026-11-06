import Leanbox.Dev.Pl061
/-!
# PL061 checks

The pl061.rs unit tests as `#guard`s, checked when this module is built (`lake test`).
-/
namespace Leanbox.Dev.Pl061

namespace Tests
open State

-- pl061.rs tests LINE
def LINE : UInt8 := (1 : UInt8) <<< POWER_BUTTON_LINE
def wr (s : State) (off : UInt64) (v : UInt8) : State := s.write off 2 v.toUInt64
def rd (s : State) (off : UInt64) : UInt8 := (s.read off 2).2.toUInt8

-- an_eight_byte_access_is_two_word_accesses
def tDword : Bool :=
  let v : UInt64 := (0x12 : UInt64) ||| ((0x34 : UInt64) <<< 32)
  let s := State.new.write GPIOIBE 3 v
  rd s GPIOIBE == 0x12 && rd s GPIOIEV == 0x34 && (s.read GPIOIBE 3).2 == v
#guard tDword

-- a_power_button_press_and_release_each_interrupt_on_both_edges
def tPowerButton : Bool :=
  let s := wr (wr State.new GPIOIBE LINE) GPIOIE LINE
  let ok := !s.irq
  let s := s.setInput POWER_BUTTON_LINE true
  let ok := ok && s.irq && rd s GPIODATA_LAST &&& LINE == LINE
  let s := wr s GPIOIC LINE
  let ok := ok && !s.irq
  let s := s.setInput POWER_BUTTON_LINE false
  ok && s.irq
#guard tPowerButton

-- an_output_reads_what_it_drives_and_an_input_what_the_outside_drives
def tOutputInput : Bool :=
  let s := State.new.setInput POWER_BUTTON_LINE true
  let s := wr (wr s GPIODIR LINE) GPIODATA_LAST 0
  let ok := rd s GPIODATA_LAST == 0
  let s := wr s GPIODIR 0
  ok && rd s GPIODATA_LAST == LINE
#guard tOutputInput

-- a_write_to_an_input_reaches_the_line_once_it_is_an_output
def tWriteToInput : Bool :=
  let s := wr State.new GPIODATA_LAST 0x81
  let ok := rd s GPIODATA_LAST == 0
  let s := wr s GPIODIR 0xFF
  ok && rd s GPIODATA_LAST == 0x81
#guard tWriteToInput

-- the_address_masks_data_bits
def tAddressMask : Bool :=
  let s := wr (wr State.new GPIODIR 0xFF) 0x098 0xFB
  rd s GPIODATA_LAST == 0b0010_0010 && rd s 0x080 == 0b0010_0000
#guard tAddressMask

-- an_output_changing_level_is_an_edge
def tOutputEdge : Bool :=
  let s := wr (wr (wr State.new GPIODIR 0x01) GPIOIEV 0x01) GPIOIE 0x01
  let ok := !s.irq
  let s := wr s 0x004 0x01
  ok && s.irq && rd s GPIORIS == 0x01
#guard tOutputEdge

-- a_level_interrupt_follows_the_line
def tLevel : Bool :=
  let s := wr (wr (wr State.new GPIOIS LINE) GPIOIEV LINE) GPIOIE LINE
  let s := s.setInput POWER_BUTTON_LINE true
  let ok := s.irq
  let s := wr s GPIOIC LINE
  let ok := ok && s.irq
  let s := s.setInput POWER_BUTTON_LINE false
  ok && !s.irq && rd s GPIORIS == 0
#guard tLevel

-- a_masked_edge_shows_in_the_raw_status_only
def tMaskedEdge : Bool :=
  let s := (wr State.new GPIOIEV LINE).setInput POWER_BUTTON_LINE true
  rd s GPIORIS == LINE && rd s GPIOMIS == 0 && !s.irq
#guard tMaskedEdge

-- a_line_under_hardware_control_reads_the_outside
def tHardwareControl : Bool :=
  let s := wr (wr (wr State.new GPIODIR LINE) GPIODATA_LAST LINE) GPIOAFSEL LINE
  let ok := rd s GPIODATA_LAST == 0
  let s := s.setInput POWER_BUTTON_LINE true
  ok && rd s GPIODATA_LAST == LINE
#guard tHardwareControl

-- a_reset_clears_the_registers_and_keeps_the_outside
def tReset : Bool :=
  let s := wr (wr State.new GPIOIBE LINE) GPIOIE LINE
  let s := s.setInput POWER_BUTTON_LINE true
  let ok := s.irq
  let s := s.reset
  ok && !s.irq && rd s GPIORIS == 0 && rd s GPIOIE == 0 && rd s GPIODATA_LAST == LINE
#guard tReset

-- a_narrow_access_reaches_the_register_that_holds_it
def tNarrow : Bool :=
  let s := wr State.new GPIODIR 0x5A
  (s.read 0x401 0).2 == 0x5A && (s.read 0xFE2 0).2 == 0x61
#guard tNarrow

end Tests

end Leanbox.Dev.Pl061
