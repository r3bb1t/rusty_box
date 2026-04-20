#![allow(unused_assignments, dead_code)]

use crate::cpu::{cpuid::BxCpuIdTrait, BxCpuC};

use super::Result;

#[derive(Debug)]
pub struct BxCpuBuilder<I: BxCpuIdTrait> {
    cpuid: I,
}

impl<I: BxCpuIdTrait> Default for BxCpuBuilder<I> {
    fn default() -> Self {
        Self::new()
    }
}

impl<I: BxCpuIdTrait> BxCpuBuilder<I> {
    pub fn new() -> Self {
        let cpuid = I::new();
        Self { cpuid }
    }

    #[cfg(feature = "alloc")]
    pub fn build(self) -> Result<alloc::boxed::Box<BxCpuC<'static, I, ()>>> { self.build_with_tracer(()) }

    #[cfg(feature = "alloc")]
    pub fn build_with_tracer<T: super::instrumentation::Instrumentation>(self, tracer: T) -> Result<alloc::boxed::Box<BxCpuC<'static, I, T>>> {
        let cpuid = I::new();

        // BxCpuC is ~50MB (BxICache alone is ~19MB of fixed arrays).
        // Cannot construct on the stack. Allocate zeroed heap memory and
        // initialize field-by-field via raw pointer.
        let layout = alloc::alloc::Layout::new::<BxCpuC<'static, I, T>>();
        tracing::info!("CPU alloc: {} bytes (align={})", layout.size(), layout.align());
        let ptr = unsafe { alloc::alloc::alloc_zeroed(layout) } as *mut BxCpuC<'static, I, T>;
        if ptr.is_null() {
            alloc::alloc::handle_alloc_error(layout);
        }
        tracing::info!("CPU alloc OK at {:p}", ptr);

        unsafe {
            Self::init_cpu_fields(ptr, cpuid, tracer);
            let mut boxed = alloc::boxed::Box::from_raw(ptr);
            let config = Default::default();
            boxed.initialize(config)?;
            Ok(boxed)
        }
    }

    /// Initialize a BxCpuC at a caller-provided, zeroed memory location.
    ///
    /// # Safety
    /// - `ptr` must point to a valid, zeroed, properly aligned allocation of
    ///   `size_of::<BxCpuC<I, T>>()` bytes.
    /// - The allocation must outlive the returned reference.
    pub unsafe fn init_cpu_at<'a, T: super::instrumentation::Instrumentation>(
        ptr: *mut BxCpuC<'a, I, T>,
        tracer: T,
    ) -> Result<&'a mut BxCpuC<'a, I, T>> {
        let cpuid = I::new();
        Self::init_cpu_fields(ptr, cpuid, tracer);
        let cpu = &mut *ptr;
        cpu.initialize(Default::default())?;
        Ok(cpu)
    }

    /// Write essential fields into a zeroed BxCpuC allocation.
    ///
    /// # Safety
    /// `ptr` must be valid, zeroed, aligned for BxCpuC.
    unsafe fn init_cpu_fields<T: super::instrumentation::Instrumentation>(
        ptr: *mut BxCpuC<'_, I, T>,
        cpuid: I,
        tracer: T,
    ) {
        core::ptr::addr_of_mut!((*ptr).cpuid).write(cpuid);
        core::ptr::addr_of_mut!((*ptr).ignore_bad_msrs).write(true);
        core::ptr::addr_of_mut!((*ptr).a20_mask).write(0xFFFF_FFFF_FFFF_FFFF);
        core::ptr::addr_of_mut!((*ptr).last_exception_type).write(-1);
        core::ptr::addr_of_mut!((*ptr).instrumentation).write(
            super::instrumentation::InstrumentationRegistry::with_tracer(tracer),
        );
        core::ptr::addr_of_mut!((*ptr).mmio).write(crate::memory::mmio::MmioRegistry::new());
        (*ptr).dtlb.flush();
        (*ptr).itlb.flush();
    }
}