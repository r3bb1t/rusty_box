/// memory access type (read/write/execute/rw)
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum MemoryAccessType {
    Read = 0,
    Write = 1,
    Execute = 2,
    RW = 3,
    ShadowStackRead = 4,
    ShadowStackWrite = 5,
    ShadowStackInvalid = 6, // can't execute shadow stack
    ShadowStackRw = 7,
}
