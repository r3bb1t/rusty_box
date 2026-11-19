//! Typed host register access for the x86 software engine.

use core::marker::PhantomData;

use super::decoder::BX_XMM_REGISTERS;
pub use super::eflags::EFlags;
use super::memory_session::scalar_io::PendingScalarIo;
use super::{BxCpuC, Instrumentation, X86Reg};
use crate::config::BxPhyAddress;

/// A register that has an infallible host-state write.
#[derive(Clone, Copy, Debug)]
pub struct ReadWrite;

/// A register exposed for inspection. Mode changes and checked control
/// operations have their own entry points.
#[derive(Clone, Copy, Debug)]
pub struct ReadOnly;

/// A descriptor names a register view, independent of any CPU instance.
/// Descriptors come from this module's constants or checked SIMD constructors.
#[derive(Clone, Copy)]
pub struct Register<V: RegisterValue, A = ReadWrite> {
    location: V::Location,
    access: PhantomData<A>,
}

impl<V: RegisterValue, A> Register<V, A> {
    const fn new(location: V::Location) -> Self {
        Self {
            location,
            access: PhantomData,
        }
    }
}

/// Value types carried by the register descriptors.
pub trait RegisterValue: value::Access {}
impl<V: value::Access> RegisterValue for V {}

mod value {
    use super::*;

    pub trait Access: Copy {
        type Location: Copy;
        fn read<T: Instrumentation, A>(cpu: &BxCpuC<T>, reg: Register<Self, A>) -> Self;
        fn write<T: Instrumentation>(cpu: &mut BxCpuC<T>, reg: Register<Self>, value: Self);
    }

    macro_rules! scalar_values {
        ($($width:ty),+ $(,)?) => {$(
            impl Access for $width {
                type Location = X86Reg;
                #[inline]
                fn read<T: Instrumentation, A>(cpu: &BxCpuC<T>, reg: Register<Self, A>) -> Self {
                    cpu.api_reg_read(reg.location) as Self
                }
                #[inline]
                fn write<T: Instrumentation>(cpu: &mut BxCpuC<T>, reg: Register<Self>, value: Self) {
                    write_scalar(cpu, reg.location, u64::from(value));
                }
            }
        )+};
    }
    scalar_values!(u8, u16, u32, u64);

    impl Access for EFlags {
        type Location = X86Reg;

        fn read<T: Instrumentation, A>(cpu: &BxCpuC<T>, reg: Register<Self, A>) -> Self {
            Self::from_bits_retain(cpu.api_reg_read(reg.location) as u32)
        }

        fn write<T: Instrumentation>(cpu: &mut BxCpuC<T>, reg: Register<Self>, value: Self) {
            write_scalar(cpu, reg.location, u64::from(value.bits()));
        }
    }

    macro_rules! vector_value {
        ($width:literal, $read:ident, $write:ident) => {
            impl Access for [u8; $width] {
                type Location = usize;

                #[inline]
                fn read<T: Instrumentation, A>(cpu: &BxCpuC<T>, reg: Register<Self, A>) -> Self {
                    cpu.$read(reg.location)
                }

                #[inline]
                fn write<T: Instrumentation>(
                    cpu: &mut BxCpuC<T>,
                    reg: Register<Self>,
                    value: Self,
                ) {
                    cpu.$write(reg.location, value);
                }
            }
        };
    }
    vector_value!(16, xmm_read_for_api, xmm_write_for_api);
    vector_value!(32, ymm_read_for_api, ymm_write_for_api);
    vector_value!(64, zmm_read_for_api, zmm_write_for_api);
}

macro_rules! scalar_registers {
    ($($width:ty {$($name:ident = $reg:ident),+ $(,)?}),+ $(,)?) => {
        $($(pub const $name: Register<$width> = Register::new(X86Reg::$reg);)+)+

        fn is_gpr(reg: X86Reg) -> bool {
            matches!(reg, $($(X86Reg::$reg)|+)|+)
        }
    };
}

scalar_registers! {
    u8 {
        AL = Al, CL = Cl, DL = Dl, BL = Bl,
        AH = Ah, CH = Ch, DH = Dh, BH = Bh,
        SPL = Spl, BPL = Bpl, SIL = Sil, DIL = Dil,
        R8B = R8b, R9B = R9b, R10B = R10b, R11B = R11b, R12B = R12b, R13B = R13b, R14B = R14b, R15B = R15b,
    },
    u16 {
        AX = Ax, CX = Cx, DX = Dx, BX = Bx, SP = Sp, BP = Bp, SI = Si, DI = Di,
        R8W = R8w, R9W = R9w, R10W = R10w, R11W = R11w, R12W = R12w, R13W = R13w, R14W = R14w, R15W = R15w,
    },
    u32 {
        EAX = Eax, ECX = Ecx, EDX = Edx, EBX = Ebx, ESP = Esp, EBP = Ebp, ESI = Esi, EDI = Edi,
        R8D = R8d, R9D = R9d, R10D = R10d, R11D = R11d, R12D = R12d, R13D = R13d, R14D = R14d, R15D = R15d,
    },
    u64 {
        RAX = Rax, RCX = Rcx, RDX = Rdx, RBX = Rbx, RSP = Rsp, RBP = Rbp, RSI = Rsi, RDI = Rdi,
        R8 = R8, R9 = R9, R10 = R10, R11 = R11, R12 = R12, R13 = R13, R14 = R14, R15 = R15,
    },
}

pub const RIP: Register<u64> = Register::new(X86Reg::Rip);
pub const EIP: Register<u32> = Register::new(X86Reg::Eip);
pub const IP: Register<u16> = Register::new(X86Reg::Ip);
/// Reading returns the current flags. Writing updates the status flags and
/// DF, IF, TF, RF and AC, preserving privilege and mode fields.
pub const EFLAGS: Register<EFlags> = Register::new(X86Reg::Eflags);
pub const CR0: Register<u32, ReadOnly> = Register::new(X86Reg::Cr0);
pub const CR2: Register<u64> = Register::new(X86Reg::Cr2);
pub const CR3: Register<u64, ReadOnly> = Register::new(X86Reg::Cr3);
pub const CR4: Register<u32, ReadOnly> = Register::new(X86Reg::Cr4);
pub const EFER: Register<u64, ReadOnly> = Register::new(X86Reg::Efer);
pub const CS: Register<u16, ReadOnly> = Register::new(X86Reg::Cs);
pub const DS: Register<u16, ReadOnly> = Register::new(X86Reg::Ds);
pub const ES: Register<u16, ReadOnly> = Register::new(X86Reg::Es);
pub const SS: Register<u16, ReadOnly> = Register::new(X86Reg::Ss);
pub const FS: Register<u16, ReadOnly> = Register::new(X86Reg::Fs);
pub const GS: Register<u16, ReadOnly> = Register::new(X86Reg::Gs);

/// Select a modeled XMM register (0–31). The guest's instruction feature
/// checks still govern access from guest code.
pub fn xmm(index: usize) -> Option<Register<[u8; 16]>> {
    (index < BX_XMM_REGISTERS).then(|| Register::new(index))
}

/// Select a modeled YMM register (0–31).
pub fn ymm(index: usize) -> Option<Register<[u8; 32]>> {
    (index < BX_XMM_REGISTERS).then(|| Register::new(index))
}

/// Select a modeled ZMM register (0–31).
pub fn zmm(index: usize) -> Option<Register<[u8; 64]>> {
    (index < BX_XMM_REGISTERS).then(|| Register::new(index))
}

/// The low 128 bits. A host write preserves the upper bits.
pub const XMM0: Register<[u8; 16]> = Register::new(0);
/// The low 256 bits. A host write clears the upper 256 bits.
pub const YMM0: Register<[u8; 32]> = Register::new(0);
pub const ZMM0: Register<[u8; 64]> = Register::new(0);

/// Short mutable view of the processor's register state.
pub struct Registers<'a, T: Instrumentation = ()> {
    cpu: &'a mut BxCpuC<T>,
    pending: Option<&'a mut Option<PendingScalarIo>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum RegisterRefusal {
    #[error("{0:?} is outside the session's scalar register view")]
    Unsupported(X86Reg),
    #[error("{0:?} is read-only through the session register API")]
    ReadOnly(X86Reg),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum PageTableRootError {
    #[error(
        "this page-table root API requires legacy non-PAE execution outside virtualization and SMM"
    )]
    Mode,
    #[error("the legacy page-table root must be 4 KiB aligned and fit in 32 physical address bits")]
    Address,
}

impl<'a, T: Instrumentation> Registers<'a, T> {
    pub(crate) fn for_processor(cpu: &'a mut BxCpuC<T>) -> Self {
        Self { cpu, pending: None }
    }

    pub(super) fn with_pending(
        cpu: &'a mut BxCpuC<T>,
        pending: &'a mut Option<PendingScalarIo>,
    ) -> Self {
        Self {
            cpu,
            pending: Some(pending),
        }
    }

    fn edited(&mut self) {
        if let Some(pending) = self.pending.as_deref_mut() {
            *pending = None;
        }
    }

    #[inline]
    pub fn read<V: RegisterValue, A>(&self, register: Register<V, A>) -> V {
        V::read(self.cpu, register)
    }

    #[inline]
    pub fn write<V: RegisterValue>(&mut self, register: Register<V>, value: V) {
        V::write(self.cpu, register, value);
        self.edited();
    }

    /// Runtime-selected scalar read. SIMD values use their own descriptors.
    pub fn read_dynamic(&self, register: X86Reg) -> Result<u64, RegisterRefusal> {
        read_dynamic(self.cpu, register)
    }

    /// Runtime-selected scalar write with the legacy scalar width conventions.
    /// Use the checked root setter for CR3.
    pub fn write_dynamic(&mut self, register: X86Reg, value: u64) -> Result<(), RegisterRefusal> {
        use X86Reg::*;
        match register {
            Rip | Eip | Ip | Rflags | Eflags | Flags | Cr2 => {}
            Cs | Ss | Ds | Es | Fs | Gs | Cr0 | Cr3 | Cr4 | Efer => {
                return Err(RegisterRefusal::ReadOnly(register));
            }
            _ if is_gpr(register) => {}
            _ => return Err(RegisterRefusal::Unsupported(register)),
        }
        write_scalar(self.cpu, register, value);
        self.edited();
        Ok(())
    }

    /// Install an aligned legacy page-directory root and flush translations.
    /// Preserve PWT and PCD. Ordinary guest walks check the table contents
    /// and backing when they are used.
    pub fn set_page_table_root(&mut self, root: BxPhyAddress) -> Result<(), PageTableRootError> {
        if self.cpu.long_mode()
            || self.cpu.cr4.pae()
            || self.cpu.in_vmx
            || self.cpu.in_svm_guest
            || self.cpu.in_smm
        {
            return Err(PageTableRootError::Mode);
        }
        if root.page_offset() != 0 || root.get() > u64::from(u32::MAX) {
            return Err(PageTableRootError::Address);
        }
        let cache_flags = self.cpu.get_cr3_val() & 0x18;
        self.cpu.set_cr3_raw_for_api(root.get() | cache_flags);
        self.edited();
        Ok(())
    }
}

pub(super) fn read_dynamic<T: Instrumentation>(
    cpu: &BxCpuC<T>,
    reg: X86Reg,
) -> Result<u64, RegisterRefusal> {
    use X86Reg::*;
    if is_gpr(reg)
        || matches!(
            reg,
            Rip | Eip
                | Ip
                | Rflags
                | Eflags
                | Flags
                | Cs
                | Ss
                | Ds
                | Es
                | Fs
                | Gs
                | Cr0
                | Cr2
                | Cr3
                | Cr4
                | Efer
        )
    {
        Ok(cpu.api_reg_read(reg))
    } else {
        Err(RegisterRefusal::Unsupported(reg))
    }
}

pub(super) fn set_ip<T: Instrumentation>(cpu: &mut BxCpuC<T>, address: u64) {
    cpu.set_rip(address);
    cpu.prev_rip = address;
    cpu.invalidate_prefetch_q();
    let _ = cpu.code_breakpoints.take_hit();
    cpu.code_breakpoints.forget_reported();
}

fn write_scalar<T: Instrumentation>(cpu: &mut BxCpuC<T>, reg: X86Reg, value: u64) {
    use X86Reg::*;
    match reg {
        Rip => set_ip(cpu, value),
        Eip => set_ip(cpu, value & u64::from(u32::MAX)),
        Ip => set_ip(cpu, (cpu.rip() & !0xFFFF) | (value & 0xFFFF)),
        Rflags | Eflags | Flags => {
            let width = if reg == Flags { 0xFFFF } else { u32::MAX };
            let mask =
                EFlags::OSZAPC | EFlags::DF | EFlags::IF_ | EFlags::TF | EFlags::RF | EFlags::AC;
            cpu.write_eflags(value as u32, width & mask.bits());
        }
        _ => cpu.api_reg_write(reg, value),
    }
}
