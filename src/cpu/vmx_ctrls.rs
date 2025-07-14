#[derive(Debug, Default)]
pub(super) struct VmxVmexec1Controls {
    pub(super) vmexec_ctrls: u32,
}

#[derive(Debug, Default)]
pub(super) struct VmxPinBasedVmexecControls {
    pub(super) pin_vmexec_ctrls: u32,
}

#[derive(Debug, Default)]
pub(super) struct VmxVmexec2Controls {
    vmexec_ctrls: u32,
}
