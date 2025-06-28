pub type VmcsCache = BxVmcs;

#[derive(Debug, Default)]
pub struct VmcsMapping {}

// TODO: Implement this
#[derive(Debug, Default)] // Fixme: derive default by hand maybe
pub struct BxVmcs {
    // todo
    pub(crate) shadow_stack_prematurely_busy: bool,
}

pub type BxVmxCap = VmxCap;

#[derive(Debug, Default)]
pub struct VmxCap {
    // todo
}
