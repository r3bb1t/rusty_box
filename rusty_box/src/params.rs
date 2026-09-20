#![allow(unused_assignments, dead_code)]

use crate::cpu::decoder::features::X86Feature;

const MAX_FEATURES: usize = 64;
pub const BX_CPU_PROCESSORS_LIMIT: u32 = 255;
pub const BX_CPU_CORES_LIMIT: u32 = 8;
pub const BX_CPU_HT_THREADS_LIMIT: u32 = 4;
pub const BX_MAX_SMP_THREADS_SUPPORTED: u32 = 254;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuTopology {
    n_processors: u32,
    n_cores: u32,
    n_threads: u32,
}

impl CpuTopology {
    #[inline]
    pub const fn new_unchecked(n_processors: u32, n_cores: u32, n_threads: u32) -> Self {
        Self {
            n_processors,
            n_cores,
            n_threads,
        }
    }

    #[inline]
    pub const fn n_processors(self) -> u32 {
        self.n_processors
    }

    #[inline]
    pub const fn n_cores(self) -> u32 {
        self.n_cores
    }

    #[inline]
    pub const fn n_threads(self) -> u32 {
        self.n_threads
    }

    #[inline]
    pub const fn package_logical_count(self) -> u32 {
        self.n_cores * self.n_threads
    }

    #[inline]
    pub const fn cpu_count(self) -> u32 {
        self.n_processors * self.n_cores * self.n_threads
    }
}

impl Default for CpuTopology {
    fn default() -> Self {
        Self::new_unchecked(1, 1, 1)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BxParamError {
    TopologyComponentOutOfRange {
        component: &'static str,
        value: u32,
        min: u32,
        max: u32,
    },
    TooManyLogicalProcessors {
        count: u32,
        max: u32,
    },
}

/// Fixed-capacity list of CPU features, replacing `Vec<X86Feature>` for no-alloc support.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeatureList {
    features: [X86Feature; MAX_FEATURES],
    len: usize,
}

impl FeatureList {
    pub const fn new() -> Self {
        // X86Feature is repr-safe; use first variant as filler (never read past `len`).
        Self {
            features: [X86Feature::Isa386; MAX_FEATURES],
            len: 0,
        }
    }

    pub fn push(&mut self, feature: X86Feature) {
        assert!(self.len < MAX_FEATURES, "FeatureList overflow");
        self.features[self.len] = feature;
        self.len += 1;
    }

    pub fn iter(&self) -> impl Iterator<Item = &X86Feature> {
        self.features[..self.len].iter()
    }

    pub fn contains(&self, feature: &X86Feature) -> bool {
        self.features[..self.len].contains(feature)
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn len(&self) -> usize {
        self.len
    }
}

impl Default for FeatureList {
    fn default() -> Self {
        Self::new()
    }
}

/// What a triple fault does to the machine — Bochs `cpu: reset_on_triple_fault`.
///
/// Whichever is chosen, a processor running a VMX guest takes a triple-fault
/// VM exit first, and one running an SVM guest that intercepts `SHUTDOWN`
/// takes that exit; only a fault that reaches the host processor gets here.
///
/// Left unchosen ([`BxParams::on_triple_fault`] never called), a machine that
/// boots firmware resets, as Bochs does, and a machine set up without
/// firmware — [`Emulator::new_with_mode`](crate::emulator::Emulator::new_with_mode)
/// and [`Emulator::setup_cpu_mode`](crate::emulator::Emulator::setup_cpu_mode)
/// — shuts the processor down, because a reset would start it at a reset
/// vector nothing was loaded at. Bochs always boots firmware, so the second
/// default is this port's own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum OnTripleFault {
    /// Reset the whole machine, processors and devices, as a hardware reset
    /// does. Bochs's default (`reset_on_triple_fault=1`), and what a guest
    /// that reboots by triple-faulting relies on.
    ResetTheMachine = 0,
    /// Leave the processor in the shutdown state, where only NMI, SMI and
    /// INIT reach it.
    ShutDown = 1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BxParams {
    pub(crate) cpu_nthreads: u32,
    pub(crate) cpu_ncores: u32,
    pub(crate) cpu_nprocessors: u32,

    pub(crate) cpu_include_features: FeatureList,
    pub(crate) cpu_exclude_features: FeatureList,

    /// `None` until a caller chooses; see [`OnTripleFault`] for what the
    /// machine does then.
    pub(crate) cpu_on_triple_fault: Option<OnTripleFault>,
}

impl Default for BxParams {
    fn default() -> Self {
        Self {
            cpu_nthreads: 1,
            cpu_ncores: 1,
            cpu_nprocessors: 1,
            cpu_include_features: FeatureList::new(),
            cpu_exclude_features: FeatureList::new(),
            cpu_on_triple_fault: None,
        }
    }
}

impl BxParams {
    #[inline]
    pub fn with_topology(
        mut self,
        n_processors: u32,
        n_cores: u32,
        n_threads: u32,
    ) -> core::result::Result<Self, BxParamError> {
        validate_component("n_processors", n_processors, 1, BX_CPU_PROCESSORS_LIMIT)?;
        validate_component("n_cores", n_cores, 1, BX_CPU_CORES_LIMIT)?;
        validate_component("n_threads", n_threads, 1, BX_CPU_HT_THREADS_LIMIT)?;

        let count = n_processors * n_cores * n_threads;
        if count > BX_MAX_SMP_THREADS_SUPPORTED {
            return Err(BxParamError::TooManyLogicalProcessors {
                count,
                max: BX_MAX_SMP_THREADS_SUPPORTED,
            });
        }

        self.cpu_nprocessors = n_processors;
        self.cpu_ncores = n_cores;
        self.cpu_nthreads = n_threads;
        Ok(self)
    }

    #[inline]
    /// Hold a feature back from the processor this machine offers its guest.
    ///
    /// Bochs `cpuid: <feature>=0`. The feature leaves the ISA bitmask at
    /// initialisation, which is what every decode gate, `XSETBV` and the
    /// `CPUID` leaves derived from it all read — so a guest is never told it
    /// has something one of them would refuse.
    ///
    /// A machine setting rather than an engine one, deliberately: a guest that
    /// enabled a register file keeps it across a switch between engines, so
    /// the two must offer the same processor or the switch changes the
    /// hardware under a running guest.
    #[must_use]
    pub fn excluding(mut self, feature: X86Feature) -> Self {
        self.cpu_exclude_features.push(feature);
        self
    }

    /// Offer a feature the model does not carry — Bochs `cpuid: <feature>=1`.
    #[must_use]
    pub fn including(mut self, feature: X86Feature) -> Self {
        self.cpu_include_features.push(feature);
        self
    }

    /// Choose what a triple fault does — Bochs `cpu: reset_on_triple_fault`.
    /// See [`OnTripleFault`] for what happens when nothing is chosen.
    #[must_use]
    pub fn on_triple_fault(mut self, action: OnTripleFault) -> Self {
        self.cpu_on_triple_fault = Some(action);
        self
    }

    /// What a triple fault does on a machine that boots firmware: the
    /// caller's choice, or Bochs's default reset.
    pub(crate) fn on_triple_fault_with_firmware(&self) -> OnTripleFault {
        self.cpu_on_triple_fault.unwrap_or(OnTripleFault::ResetTheMachine)
    }

    /// What a triple fault does on a machine set up without firmware: the
    /// caller's choice, or shutdown, since nothing waits at its reset vector.
    pub(crate) fn on_triple_fault_without_firmware(&self) -> OnTripleFault {
        self.cpu_on_triple_fault.unwrap_or(OnTripleFault::ShutDown)
    }

    pub fn cpu_topology(&self) -> CpuTopology {
        CpuTopology::new_unchecked(self.cpu_nprocessors, self.cpu_ncores, self.cpu_nthreads)
    }

    #[inline]
    pub fn cpu_count(&self) -> u32 {
        self.cpu_topology().cpu_count()
    }
}

#[inline]
fn validate_component(
    component: &'static str,
    value: u32,
    min: u32,
    max: u32,
) -> core::result::Result<(), BxParamError> {
    if value < min || value > max {
        return Err(BxParamError::TopologyComponentOutOfRange {
            component,
            value,
            min,
            max,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_topology_is_uniprocessor() {
        let params = BxParams::default();

        assert_eq!(params.cpu_topology(), CpuTopology::new_unchecked(1, 1, 1));
        assert_eq!(params.cpu_count(), 1);
    }

    #[test]
    fn topology_uses_bochs_smp_limits_and_xapic_cap() {
        let params = BxParams::default()
            .with_topology(2, 4, 2)
            .expect("valid Bochs SMP topology");

        assert_eq!(params.cpu_topology(), CpuTopology::new_unchecked(2, 4, 2));
        assert_eq!(params.cpu_count(), 16);
        let four_thread_core = BxParams::default()
            .with_topology(1, 1, 4)
            .expect("Bochs allows up to four hardware threads per core");
        assert_eq!(
            four_thread_core.cpu_topology(),
            CpuTopology::new_unchecked(1, 1, 4)
        );
        assert_eq!(four_thread_core.cpu_count(), 4);

        assert_eq!(
            BxParams::default().with_topology(0, 1, 1),
            Err(BxParamError::TopologyComponentOutOfRange {
                component: "n_processors",
                value: 0,
                min: 1,
                max: BX_CPU_PROCESSORS_LIMIT,
            })
        );
        assert_eq!(
            BxParams::default().with_topology(BX_CPU_PROCESSORS_LIMIT + 1, 1, 1),
            Err(BxParamError::TopologyComponentOutOfRange {
                component: "n_processors",
                value: BX_CPU_PROCESSORS_LIMIT + 1,
                min: 1,
                max: BX_CPU_PROCESSORS_LIMIT,
            })
        );
        assert_eq!(
            BxParams::default().with_topology(1, BX_CPU_CORES_LIMIT + 1, 1),
            Err(BxParamError::TopologyComponentOutOfRange {
                component: "n_cores",
                value: BX_CPU_CORES_LIMIT + 1,
                min: 1,
                max: BX_CPU_CORES_LIMIT,
            })
        );
        assert_eq!(
            BxParams::default().with_topology(1, 1, BX_CPU_HT_THREADS_LIMIT + 1),
            Err(BxParamError::TopologyComponentOutOfRange {
                component: "n_threads",
                value: BX_CPU_HT_THREADS_LIMIT + 1,
                min: 1,
                max: BX_CPU_HT_THREADS_LIMIT,
            })
        );
        assert_eq!(
            BxParams::default().with_topology(BX_CPU_PROCESSORS_LIMIT, 1, 1),
            Err(BxParamError::TooManyLogicalProcessors {
                count: BX_CPU_PROCESSORS_LIMIT,
                max: BX_MAX_SMP_THREADS_SUPPORTED,
            })
        );
    }
}
