use crate::cpu::{
    cpu::{Exception, ExceptionClass, BX_CPU_HANDLED_EXCEPTIONS},
    decoder::features::X86Feature,
};

use super::{cpuid::BxCpuIdTrait, BxCpuC, Result};

/* Exception types.  These are used as indexes into the 'is_exception_OK'
 * array below, and are stored in the 'exception' array also
 */
enum ExceptionType {
    Benign = 0,
    Contributory = 1,
    PageFault = 2,
    DoubleFault = 10,
}

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
        let mut exception_type = ExceptionType::Benign;
        let mut exception_class = ExceptionClass::Fault;
        let mut push_error = false;

        if (vector.clone() as usize) < BX_CPU_HANDLED_EXCEPTIONS {
            push_error = self.exception_push_error(vector.clone() as usize);
        } else {
            return Err(super::error::CpuError::BadVector { vector });
        }
        /* Excluding page faults and double faults, error_code may not have the
        * least significant bit set correctly. This correction is applied first
             todo!()   * to make the change transparent to any instrumentation.
         }   */
        if push_error {
            if vector != Exception::Pf
                && vector != Exception::Df
                && vector != Exception::Cp
                && vector != Exception::Sx
            {
                error_code = (error_code & 0xfffe) | (self.ext as u16);
            }
        }

        tracing::debug!("exception({:?}): error_code={:#x}", vector, error_code);

        if self.real_mode() {
            push_error = false; // not INT, no error code pushed
            error_code = 0;
        }

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

        Ok(())
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
