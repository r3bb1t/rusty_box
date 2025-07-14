// pub type BxPhyAddress = u64;
pub type BxPhyAddress = u64;

pub type BxAddress = u64;

#[cfg(target_pointer_width = "32")]
pub type BxPtrEquiv = u32;

#[cfg(target_pointer_width = "64")]
pub type BxPtrEquiv = u64;

#[cfg(not(any(target_pointer_width = "32", target_pointer_width = "64")))]
compile_error!("could not define BxPtrEquivT to size of pointer");
