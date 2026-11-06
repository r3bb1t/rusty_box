/-!
# The memory map

Where the `virt` board puts RAM and each device (board.rs), the interrupt
each device raises, and the machine's RAM size and clock rates.
-/
namespace Leanbox

-- board.rs memory map
def FLASH0_BASE : UInt64 := 0x0000_0000
def FLASH_BANK_SIZE : UInt64 := 0x0400_0000
def FLASH1_BASE : UInt64 := FLASH0_BASE + FLASH_BANK_SIZE
def GIC_DIST_BASE : UInt64 := 0x0800_0000
def GIC_DIST_SIZE : UInt64 := 0x1000
def GIC_CPU_BASE : UInt64 := 0x0801_0000
def GIC_CPU_SIZE : UInt64 := 0x2000
def GIC_V2M_BASE : UInt64 := 0x0802_0000
def GIC_V2M_SIZE : UInt64 := 0x1000
def UART0_BASE : UInt64 := 0x0900_0000
def RTC_BASE : UInt64 := 0x0901_0000
def GPIO_BASE : UInt64 := 0x0903_0000
def PRIMECELL_SIZE : UInt64 := 0x1000
def VIRTIO_MMIO_BASE : UInt64 := 0x0A00_0000
def PCIE_MMIO_BASE : UInt64 := 0x1000_0000
def PCIE_MMIO_SIZE : UInt64 := 0x2EFF_0000
def PCIE_PIO_BASE : UInt64 := 0x3EFF_0000
def PCIE_PIO_SIZE : UInt64 := 0x1_0000
def RAM_BASE : UInt64 := 0x4000_0000
def PCIE_ECAM_BASE : UInt64 := 0x40_1000_0000
def PCIE_ECAM_SIZE : UInt64 := 0x1000_0000
def PCIE_MMIO_HIGH_BASE : UInt64 := 0x80_0000_0000
def PCIE_MMIO_HIGH_SIZE : UInt64 := 0x80_0000_0000
-- board.rs IRQ_* / PPI_*
def IRQ_UART0 : Nat := 32 + 1
def IRQ_RTC : Nat := 32 + 2
def IRQ_GPIO : Nat := 32 + 7
def PPI_VIRTUAL_TIMER : Nat := 16 + 11
def PPI_PHYSICAL_TIMER : Nat := 16 + 14
def PPI_HYPERVISOR_TIMER : Nat := 16 + 10

/-- machine.rs `RamSize::DEFAULT`. -/
def RAM_SIZE : UInt64 := 256 * 1024 * 1024

/-- rusty_box_core `ClockHz::BOCHS_DEFAULT`, and features.rs
`QEMU_BACKCOMPAT_COUNTER_HZ`. -/
def IPS : Nat := 50_000_000
def COUNTER_HZ : Nat := 62_500_000

end Leanbox
