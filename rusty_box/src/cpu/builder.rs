#![allow(unused_assignments, dead_code)]

use crate::{
    cpu::{cpudb::CpuModel, BxCpuC},
    params::CpuTopology,
};

use super::Result;

#[derive(Debug, Default)]
pub struct BxCpuBuilder {
    model: CpuModel,
}

impl BxCpuBuilder {
    /// Builder for the default model (Skylake-X).
    pub fn new() -> Self {
        Self::default()
    }

    /// Builder for an explicitly selected CPU model.
    pub fn new_with_model(model: CpuModel) -> Self {
        Self { model }
    }

    #[cfg(feature = "alloc")]
    pub fn build(self) -> Result<alloc::boxed::Box<BxCpuC<()>>> {
        self.build_with_tracer(())
    }

    #[cfg(feature = "alloc")]
    pub fn build_with_tracer<T: super::instrumentation::Instrumentation>(
        self,
        tracer: T,
    ) -> Result<alloc::boxed::Box<BxCpuC<T>>> {
        // BxCpuC is ~50MB (BxICache alone is ~19MB of fixed arrays).
        // Cannot construct on the stack. Allocate zeroed heap memory and
        // initialize field-by-field via raw pointer.
        let layout = alloc::alloc::Layout::new::<BxCpuC<T>>();
        // Host allocator internals — no Bochs counterpart, and the address
        // below is a HOST pointer, so neither belongs in a guest boot log.
        tracing::debug!(
            "CPU alloc: {} bytes (align={})",
            layout.size(),
            layout.align()
        );
        let ptr = unsafe { alloc::alloc::alloc_zeroed(layout) } as *mut BxCpuC<T>;
        if ptr.is_null() {
            return Err(
                crate::memory::MemoryError::UnableToAllocateGuestMemory(layout.size()).into(),
            );
        }
        tracing::debug!("CPU alloc OK at {:p}", ptr);

        unsafe {
            Self::init_cpu_fields(ptr, self.model, tracer);
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
    ///   `size_of::<BxCpuC<T>>()` bytes.
    /// - The allocation must outlive the returned reference.
    pub unsafe fn init_cpu_at<'a, T: super::instrumentation::Instrumentation>(
        self,
        ptr: *mut BxCpuC<T>,
        tracer: T,
    ) -> Result<&'a mut BxCpuC<T>> {
        Self::init_cpu_fields(ptr, self.model, tracer);
        let cpu = &mut *ptr;
        cpu.initialize(Default::default())?;
        Ok(cpu)
    }

    /// Write essential fields into a zeroed BxCpuC allocation.
    ///
    /// # Safety
    /// `ptr` must be valid, zeroed, aligned for BxCpuC.
    unsafe fn init_cpu_fields<T: super::instrumentation::Instrumentation>(
        ptr: *mut BxCpuC<T>,
        cpuid: CpuModel,
        tracer: T,
    ) {
        core::ptr::addr_of_mut!((*ptr).cpuid).write(cpuid);
        core::ptr::addr_of_mut!((*ptr).ignore_bad_msrs).write(true);
        core::ptr::addr_of_mut!((*ptr).cpu_topology).write(CpuTopology::default());
        core::ptr::addr_of_mut!((*ptr).a20_mask).write(0xFFFF_FFFF_FFFF_FFFF);
        core::ptr::addr_of_mut!((*ptr).last_exception_type).write(-1);
        core::ptr::addr_of_mut!((*ptr).instrumentation)
            .write(super::instrumentation::InstrumentationRegistry::with_tracer(tracer));
        core::ptr::addr_of_mut!((*ptr).mmio).write(crate::memory::mmio::MmioRegistry::new());
        (*ptr).dtlb.flush();
        (*ptr).itlb.flush();
        // The allocation arrives zeroed, and zero is a MEANINGFUL value for both
        // of the icache's validity guards — an entry's `p_addr` of 0 is a real
        // physical address that `find_entry` will match, and a link timestamp of
        // 0 equals the stamp every zeroed `TraceLink` carries, which is exactly
        // what `BxICache::new` starts at 1 to prevent. Establish the flushed
        // state the type defines, as the TLBs above already do.
        (*ptr).i_cache.flush_all();
    }
}
