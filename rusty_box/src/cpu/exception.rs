use crate::cpu::{
    cpu::{Exception, ExceptionClass, BX_CPU_HANDLED_EXCEPTIONS},
    decoder::features::X86Feature,
};

use super::{cpuid::BxCpuIdTrait, BxCpuC, Result};

/// Interrupt type, based on BX_INTERRUPT_TYPE in Bochs
#[derive(Debug, Clone, Copy)]
pub(super) enum InterruptType {
    SoftwareInterrupt = 0,
    PrivilegedSoftwareInterrupt = 1,
    SoftwareException = 2,
    ExternalInterrupt = 3,
    Nmi = 4,
    HardwareException = 5,
}

/* Exception types.  These are used as indexes into the 'is_exception_OK'
 * array below, and are stored in the 'exception' array also
 */
#[derive(Clone, Copy)]
enum ExceptionType {
    Benign = 0,
    Contributory = 1,
    PageFault = 2,
    DoubleFault = 10,
}

// Match Bochs `is_exception_OK[3][3]` (cpu/exception.cc:851..855).
// Indexes are {Benign, Contributory, PageFault}.
const IS_EXCEPTION_OK: [[bool; 3]; 3] = [
    [true, true, true],   // 1st exception is BENIGN
    [true, false, true],  // 1st exception is CONTRIBUTORY
    [true, false, false], // 1st exception is PAGE_FAULT
];

struct BxExceptionInfo {
    exception_type: ExceptionType,
    exception_class: ExceptionClass,
    push_error: bool,
}

const EXCEPTIONS_INFO: [BxExceptionInfo; BX_CPU_HANDLED_EXCEPTIONS as _] = [
    /* DE */
    BxExceptionInfo {
        exception_type: ExceptionType::Contributory,
        exception_class: ExceptionClass::Fault,
        push_error: false,
    },
    /* DB */
    BxExceptionInfo {
        exception_type: ExceptionType::Benign,
        exception_class: ExceptionClass::Fault,
        push_error: false,
    },
    /* 02 */
    BxExceptionInfo {
        exception_type: ExceptionType::Benign,
        exception_class: ExceptionClass::Fault,
        push_error: false,
    }, // NMI
    /* BP */
    BxExceptionInfo {
        exception_type: ExceptionType::Benign,
        exception_class: ExceptionClass::Trap,
        push_error: false,
    },
    /* OF */
    BxExceptionInfo {
        exception_type: ExceptionType::Benign,
        exception_class: ExceptionClass::Trap,
        push_error: false,
    },
    /* BR */
    BxExceptionInfo {
        exception_type: ExceptionType::Benign,
        exception_class: ExceptionClass::Fault,
        push_error: false,
    },
    /* UD */
    BxExceptionInfo {
        exception_type: ExceptionType::Benign,
        exception_class: ExceptionClass::Fault,
        push_error: false,
    },
    /* NM */
    BxExceptionInfo {
        exception_type: ExceptionType::Benign,
        exception_class: ExceptionClass::Fault,
        push_error: false,
    },
    /* DF */
    BxExceptionInfo {
        exception_type: ExceptionType::DoubleFault,
        exception_class: ExceptionClass::Fault,
        push_error: true,
    },
    // coprocessor segment overrun (286,386 only)
    /* 09 */
    BxExceptionInfo {
        exception_type: ExceptionType::Benign,
        exception_class: ExceptionClass::Fault,
        push_error: false,
    },
    /* TS */
    BxExceptionInfo {
        exception_type: ExceptionType::Contributory,
        exception_class: ExceptionClass::Fault,
        push_error: true,
    },
    /* NP */
    BxExceptionInfo {
        exception_type: ExceptionType::Contributory,
        exception_class: ExceptionClass::Fault,
        push_error: true,
    },
    /* SS */
    BxExceptionInfo {
        exception_type: ExceptionType::Contributory,
        exception_class: ExceptionClass::Fault,
        push_error: true,
    },
    /* GP */
    BxExceptionInfo {
        exception_type: ExceptionType::Contributory,
        exception_class: ExceptionClass::Fault,
        push_error: true,
    },
    /* PF */
    BxExceptionInfo {
        exception_type: ExceptionType::PageFault,
        exception_class: ExceptionClass::Fault,
        push_error: true,
    },
    /* 15 */
    BxExceptionInfo {
        exception_type: ExceptionType::Benign,
        exception_class: ExceptionClass::Fault,
        push_error: false,
    }, // reserved
    /* MF */
    BxExceptionInfo {
        exception_type: ExceptionType::Benign,
        exception_class: ExceptionClass::Fault,
        push_error: false,
    },
    /* AC */
    BxExceptionInfo {
        exception_type: ExceptionType::Benign,
        exception_class: ExceptionClass::Fault,
        push_error: true,
    },
    /* MC */
    BxExceptionInfo {
        exception_type: ExceptionType::Benign,
        exception_class: ExceptionClass::Abort,
        push_error: false,
    },
    /* XM */
    BxExceptionInfo {
        exception_type: ExceptionType::Benign,
        exception_class: ExceptionClass::Fault,
        push_error: false,
    },
    /* VE */
    BxExceptionInfo {
        exception_type: ExceptionType::PageFault,
        exception_class: ExceptionClass::Fault,
        push_error: false,
    },
    /* CP */
    BxExceptionInfo {
        exception_type: ExceptionType::Contributory,
        exception_class: ExceptionClass::Fault,
        push_error: true,
    },
    /* 22 */
    BxExceptionInfo {
        exception_type: ExceptionType::Benign,
        exception_class: ExceptionClass::Fault,
        push_error: false,
    },
    /* 23 */
    BxExceptionInfo {
        exception_type: ExceptionType::Benign,
        exception_class: ExceptionClass::Fault,
        push_error: false,
    },
    /* 24 */
    BxExceptionInfo {
        exception_type: ExceptionType::Benign,
        exception_class: ExceptionClass::Fault,
        push_error: false,
    },
    /* 25 */
    BxExceptionInfo {
        exception_type: ExceptionType::Benign,
        exception_class: ExceptionClass::Fault,
        push_error: false,
    },
    /* 26 */
    BxExceptionInfo {
        exception_type: ExceptionType::Benign,
        exception_class: ExceptionClass::Fault,
        push_error: false,
    },
    /* 27 */
    BxExceptionInfo {
        exception_type: ExceptionType::Benign,
        exception_class: ExceptionClass::Fault,
        push_error: false,
    },
    /* 28 */
    BxExceptionInfo {
        exception_type: ExceptionType::Benign,
        exception_class: ExceptionClass::Fault,
        push_error: false,
    },
    /* 29 */
    BxExceptionInfo {
        exception_type: ExceptionType::Benign,
        exception_class: ExceptionClass::Fault,
        push_error: false,
    },
    /* SX */
    BxExceptionInfo {
        exception_type: ExceptionType::Contributory,
        exception_class: ExceptionClass::Fault,
        push_error: true,
    }, // SVM #SX is here and pushes error code
    /* 31 */
    BxExceptionInfo {
        exception_type: ExceptionType::Benign,
        exception_class: ExceptionClass::Fault,
        push_error: false,
    },
];

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // vector:     0..255: vector in IDT
    // error_code: if exception generates and error, push this error code
    pub(super) fn exception(&mut self, vector: Exception, mut error_code: u16) -> Result<()> {
        let mut push_error = if (vector as usize) < BX_CPU_HANDLED_EXCEPTIONS {
            self.exception_push_error(vector as usize)
        } else {
            return Err(super::error::CpuError::BadVector { vector });
        };
        /* Excluding page faults and double faults, error_code may not have the
         * least significant bit set correctly. This correction is applied first
         * to make the change transparent to any instrumentation.
         */
        if push_error {
            if vector != Exception::Pf
                && vector != Exception::Df
                && vector != Exception::Cp
                && vector != Exception::Sx
            {
                // Bochs ORs in EXT (0/1) into bit0 of the error code.
                // Our `ext` is a bool, so convert explicitly.
                error_code = (error_code & 0xfffe) | (u16::from(self.ext));
            }
        }

        // Reduce verbosity for common exceptions (#GP(0) is very common during boot)
        if vector != Exception::Gp || error_code != 0 {
            tracing::debug!("exception({:?}): error_code={:#x}", vector, error_code);
        }

        if self.real_mode() {
            push_error = false; // not INT, no error code pushed
            error_code = 0;
        }

        // Mirror Bochs cpu/exception.cc:984..1052.
        let info = &EXCEPTIONS_INFO[vector as usize];
        let exception_type = info.exception_type as u32;
        let exception_class = info.exception_class;

        if matches!(exception_class, ExceptionClass::Fault) {
            // restore RIP/RSP to value before error occurred
            self.set_rip(self.prev_rip);
            if self.speculative_rsp {
                self.set_rsp(self.prev_rsp);
                self.set_ssp(self.prev_ssp);
            }
            self.speculative_rsp = false;

            // Bochs: if (vector != #DB) assert_RF();
            if vector != Exception::Db {
                self.eflags |= 1 << 16; // RF bit
            }

            // Triple fault: 3rd exception with no resolution after #DF.
            if self.last_exception_type == ExceptionType::DoubleFault as u32 {
                eprintln!("TRIPLE FAULT at RIP={:#x} CS={:#x} vector={:?} error_code={:#x} icount={} CR0={:#x} CR3={:#x} IDTR.base={:#x} IDTR.limit={:#x}",
                    self.rip(), self.sregs[super::decoder::BxSegregs::Cs as usize].selector.value,
                    vector, error_code, self.icount,
                    self.cr0.get32(), self.cr3, self.idtr.base, self.idtr.limit);
                self.debug_puts(b"[TRIPLE_FAULT]\n");
                self.activity_state = super::cpu::CpuActivityState::Shutdown;
                self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
                return Err(super::error::CpuError::CpuLoopRestart);
            }
        }

        // Bochs: EXT = 1 for exceptions.
        self.ext = true;

        // If we've already had 1st exception, see if 2nd causes a Double Fault.
        if exception_type != ExceptionType::DoubleFault as u32 {
            let last = self.last_exception_type as usize;
            let newt = exception_type as usize;
            if last < 3 && newt < 3 && !IS_EXCEPTION_OK[last][newt] {
                eprintln!("DOUBLE FAULT: 1st exception type={} 2nd={:?}(type={}) at RIP={:#x} error_code={:#x} icount={}",
                    last, vector, newt, self.rip(), error_code, self.icount);
                return self.exception(Exception::Df, 0);
            }
        }

        self.last_exception_type = exception_type;

        // #if BX_DEBUGGER
        // if (bx_dbg.debugger_active)
        // bx_dbg_exception(BX_CPU_ID, vector, error_code);
        // #endif

        // #if BX_SUPPORT_VMX
        // VMexit_Event(BX_HARDWARE_EXCEPTION, vector, error_code, push_error);
        // #endif

        // #if BX_SUPPORT_SVM
        // SvmInterceptException(BX_HARDWARE_EXCEPTION, vector, error_code, push_error);
        // #endif

        // Call interrupt handler based on CPU mode
        let vector_u8 = vector as u8;
        
        // Invalidate prefetch queue
        self.eip_fetch_ptr = None;
        self.eip_page_window_size = 0;

        if self.real_mode() {
            // Real mode interrupt handling (already implemented in soft_int.rs)
            self.interrupt_real_mode(vector_u8);
        } else {
            // Protected mode interrupt handling
            self.protected_mode_int(vector_u8, false, push_error, error_code)?;
        }

        // error resolved
        self.last_exception_type = 0;

        // Bochs longjmps back to the main decode loop after delivering the exception.
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Err(super::error::CpuError::CpuLoopRestart)
    }

    fn exception_push_error(&mut self, vector: usize) -> bool {
        if vector < BX_CPU_HANDLED_EXCEPTIONS {
            if vector == Exception::Cp as usize {
                if !self.bx_cpuid_support_isa_extension(X86Feature::IsaCET) {
                    return false;
                }
            } else if vector == Exception::Sx as usize
                && !self.bx_cpuid_support_isa_extension(X86Feature::IsaSVM)
            {
                return false;
            }
            //return self.ex
            EXCEPTIONS_INFO[vector].push_error
        } else {
            false
        }
    }
}
