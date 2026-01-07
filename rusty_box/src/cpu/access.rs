use crate::config::BxAddress;

use super::{rusty_box::MemoryAccessType, BxCpuC, BxCpuIdTrait};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    pub(super) fn is_canonical_access(
        &self,
        laddr: BxAddress,
        rw: MemoryAccessType,
        user: bool,
    ) -> bool {
        if !self.is_canonical(laddr) {
            return false;
        }

        if self.long64_mode() && self.cr4.lass() {
            // laddr[63] == 0 user, laddr[63] == 1 supervisor
            let access_user_space = (laddr >> 63) == 0;

            if user {
                // When LASS is enabled, linear user accesses to supervisor space are blocked
                if !access_user_space {
                    tracing::error!(
                        "User access LASS canonical violation for address {laddr:#x} rw={rw:?}"
                    );
                    return false;
                }
                return true;
            }

            // A supervisor-mode instruction fetch causes a LASS violation if it would accesses a linear address[63] == 0
            // A supervisor-mode data access causes a LASS violation only if supervisor-mode access protection is enabled
            // (CR4.SMAP = 1) and RFLAGS.AC = 0 or the access implicitly accesses a system data structure.
            if (rw == MemoryAccessType::Execute || (self.cr4.smap() && !self.get_ac() != 0))
                && access_user_space
            {
                tracing::error!(
                    "Supervisor access LASS canonical violation for address {laddr:#x} rw={rw:?}"
                );
                return false;
            }
        }

        true
    }
}
