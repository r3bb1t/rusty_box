#![allow(unused_assignments, dead_code)]

use crate::cpu::vmx_ctrls::{VmxPinBasedVmexecControls, VmxVmexec1Controls, VmxVmexec2Controls};

pub type VmcsCache = BxVmcs;

#[derive(Debug, Default)]
pub struct VmcsMapping {}

// TODO: Implement this
// TODO: Add fire_vmexit callsite when VMexit function is ported (C++ vmx.cc:2862)
#[derive(Debug, Default)] // Fixme: derive default by hand maybe
pub struct BxVmcs {
    pin_vmexec_ctrls: VmxPinBasedVmexecControls,

    vmexec_ctrls1: VmxVmexec1Controls,

    vmexec_ctrls2: VmxVmexec2Controls,
    // todo
    pub(crate) shadow_stack_prematurely_busy: bool,
}

pub type BxVmxCap = VmxCap;

#[derive(Debug, Default)]
pub struct VmxCap {
    // todo
}
