//! Supervisor setup and architectural interrupt/frame handling.

use rusty_box::cpu::memory_session::SessionStop;
use rusty_box::cpu::registers::{CR0, CR2, CR3, CR4, CS, EAX, EFER, EIP, ESP, SS};
use rusty_box::cpu::{CpuEmulator, CpuError};
use rusty_box::iodev::IoPort;

use crate::memory::{PAGE, PDPT, physical};
use crate::{BuildError, GuestFault};

pub(crate) const BOOT: u32 = PAGE;
const KERNEL_STACK_TOP: u32 = 3 * PAGE;
pub(crate) const BOOT_STACK: u32 = KERNEL_STACK_TOP - 20;
const KERNEL_CODE: u16 = 8;
const KERNEL_DATA: u16 = 16;
const USER_CODE: u16 = 0x1B;
const USER_DATA: u16 = 0x23;
const TSS: u32 = 0x500;
const GDT: u32 = 0x800;
const GDTR: u32 = 0x840;
const IDTR: u32 = 0x848;
const SYSCALL_VECTOR: u32 = 0x80;
const SYSCALL_GATE: u32 = BOOT + 0x100;
const FAULT_GATES: u32 = BOOT + 0x200;
const SYSCALL_PORT: u8 = 0xE0;
const FAULT_PORT: u8 = 0xE1;

// Intel SDM Vol. 3A, protected-mode initialization. The CPU-only builder has
// already loaded flat ring-0 segments. Run this code through the same x86
// engine as the application so control-register and descriptor caches agree.
const BOOTSTRAP: &[u8] = &[
    0x0F, 0x01, 0x15, 0x40, 0x08, 0, 0, // lgdt [GDTR]
    0x0F, 0x01, 0x1D, 0x48, 0x08, 0, 0, // lidt [IDTR]
    0x66, 0xB8, 0x28, 0, // mov ax, TSS selector
    0x0F, 0x00, 0xD8, // ltr ax
    0x0F, 0x20, 0xE0, // mov eax, cr4
    0x83, 0xC8, 0x20, // or eax, PAE
    0x0F, 0x22, 0xE0, // mov cr4, eax
    0xB9, 0x80, 0, 0, 0xC0, // mov ecx, IA32_EFER
    0x0F, 0x32, // rdmsr
    0x0D, 0, 8, 0, 0, // or eax, NXE
    0x0F, 0x30, // wrmsr
    0xB8, 0, 0x30, 0, 0, // mov eax, PDPT
    0x0F, 0x22, 0xD8, // mov cr3, eax
    0x0F, 0x20, 0xC0, // mov eax, cr0
    0x0D, 0, 0, 1, 0x80, // or eax, PG | WP
    0x0F, 0x22, 0xC0, // mov cr0, eax
    0x66, 0xB8, 0x23, 0, // mov ax, user data selector
    0x8E, 0xD8, // mov ds, ax
    0x8E, 0xC0, // mov es, ax
    0x8E, 0xE0, // mov fs, ax
    0x8E, 0xE8, // mov gs, ax
    0x31, 0xC0, // xor eax, eax
    0x31, 0xDB, // xor ebx, ebx
    0x31, 0xC9, // xor ecx, ecx
    0x31, 0xD2, // xor edx, edx
    0x31, 0xF6, // xor esi, esi
    0x31, 0xFF, // xor edi, edi
    0x31, 0xED, // xor ebp, ebp
    0xCF, // iretd
];
const BOOT_DISPATCHES: u64 = 29;

pub(crate) fn enter(cpu: &mut CpuEmulator, entry: u32, stack: u32) -> Result<(), BuildError> {
    let mut memory = cpu.memory();
    // The builder installed the first three GDT entries. Extend that table
    // with the DPL3 flat code and data segments and the available 32-bit TSS.
    let descriptors = [
        0x00CF_FA00_0000_FFFFu64,
        0x00CF_F200_0000_FFFF,
        103 | (u64::from(TSS) << 16) | (0x89 << 40),
    ];
    for (index, descriptor) in descriptors.iter().enumerate() {
        memory.write_phys(
            physical(GDT + 24 + index as u32 * 8),
            &descriptor.to_le_bytes(),
        )?;
    }
    memory.write_phys(physical(GDTR), &table_pointer(GDT, 47))?;
    memory.write_phys(
        physical(IDTR),
        &table_pointer(0, (SYSCALL_VECTOR as u16 + 1) * 8 - 1),
    )?;
    memory.write_phys(physical(TSS + 4), &KERNEL_STACK_TOP.to_le_bytes())?;
    memory.write_phys(physical(TSS + 8), &KERNEL_DATA.to_le_bytes())?;
    // No I/O bitmap fits inside the TSS. All ring-3 port accesses fail their
    // ordinary CPU privilege check before the CPU-only provider boundary.
    memory.write_phys(physical(TSS + 102), &104u16.to_le_bytes())?;
    memory.write_phys(physical(BOOT), BOOTSTRAP)?;
    memory.write_phys(
        physical(SYSCALL_VECTOR * 8),
        &interrupt_gate(SYSCALL_GATE, 0xEE),
    )?;
    // Supervisor handler: one port handoff and the real architectural IRET.
    memory.write_phys(physical(SYSCALL_GATE), &[0xE7, SYSCALL_PORT, 0xCF])?;
    for vector in 0..32 {
        let gate = FAULT_GATES + vector * 8;
        memory.write_phys(physical(vector * 8), &interrupt_gate(gate, 0x8E))?;
        memory.write_phys(
            physical(gate),
            &[0xB8, vector as u8, 0, 0, 0, 0xE7, FAULT_PORT, 0xF4],
        )?;
    }
    for (index, word) in [
        entry,
        u32::from(USER_CODE),
        0x202,
        stack,
        u32::from(USER_DATA),
    ]
    .iter()
    .enumerate()
    {
        memory.write_phys(physical(BOOT_STACK + index as u32 * 4), &word.to_le_bytes())?;
    }
    let run = cpu.run(BOOT_DISPATCHES);
    let regs = cpu.regs();
    if !matches!(run.stop, SessionStop::Budget)
        || run.instructions != BOOT_DISPATCHES
        || regs.read(EIP) != entry
        || regs.read(ESP) != stack
        || regs.read(CS) != USER_CODE
        || regs.read(SS) != USER_DATA
        || regs.read(CR0) & 0x8001_0001 != 0x8001_0001
        || regs.read(CR4) & 0x20 == 0
        || regs.read(EFER) & 0x800 == 0
        || regs.read(CR3) != u64::from(PDPT)
    {
        return Err(BuildError::Bootstrap {
            ip: regs.read(EIP),
            stop: run.stop,
        });
    }
    Ok(())
}

fn table_pointer(base: u32, limit: u16) -> [u8; 6] {
    let mut bytes = [0; 6];
    bytes[..2].copy_from_slice(&limit.to_le_bytes());
    bytes[2..].copy_from_slice(&base.to_le_bytes());
    bytes
}

fn interrupt_gate(address: u32, attr: u8) -> [u8; 8] {
    let [a, b, c, d] = address.to_le_bytes();
    [a, b, KERNEL_CODE as u8, 0, 0, attr, c, d]
}

#[derive(Clone, Copy)]
pub(crate) enum Boundary {
    SystemCall,
    Fault(u8),
}

pub(crate) fn boundary(cpu: &mut CpuEmulator, stop: &SessionStop) -> Option<Boundary> {
    let SessionStop::Exit(CpuError::IoUnavailable {
        port,
        len: 4,
        direction_in: false,
        string: false,
    }) = stop
    else {
        return None;
    };
    let regs = cpu.regs();
    if regs.read(CS) != KERNEL_CODE {
        return None;
    }
    if *port == IoPort::new(u16::from(SYSCALL_PORT)) {
        return Some(Boundary::SystemCall);
    }
    if *port == IoPort::new(u16::from(FAULT_PORT)) && regs.read(EAX) < 32 {
        return Some(Boundary::Fault(regs.read(EAX) as u8));
    }
    None
}

pub(crate) fn fault(cpu: &mut CpuEmulator, vector: u8) -> rusty_box::Result<GuestFault> {
    let regs = cpu.regs();
    let stack = regs.read(ESP);
    let address = (vector == 14).then(|| regs.read(CR2) as u32);
    let pushes_error = matches!(vector, 8 | 10..=14 | 17 | 21 | 29 | 30);
    let mut words = [0; 8];
    // Supervisor stack is identity-mapped. The engine created this frame,
    // including the error word for the specified architectural exceptions.
    let len = if pushes_error { 8 } else { 4 };
    cpu.memory().read_phys(physical(stack), &mut words[..len])?;
    let first = u32::from_le_bytes([words[0], words[1], words[2], words[3]]);
    let second = u32::from_le_bytes([words[4], words[5], words[6], words[7]]);
    Ok(GuestFault {
        vector,
        instruction: if pushes_error { second } else { first },
        error_code: pushes_error.then_some(first),
        address,
    })
}
