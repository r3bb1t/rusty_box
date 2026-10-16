//! A loaded module's identity, read from its image in guest memory: the
//! TimeDateStamp and SizeOfImage in its headers, and the PDB its CodeView
//! record names. The record may lie on a page the guest paged out; the
//! module then keeps its image identity, which finds its binary.

use rusty_box_symbols::identity::ModuleIdentity;
use rusty_box_symbols::pe::{read_pe_identity, HeaderError, ImageRead};
use rusty_box_symbols::units::Rva;

use super::guest::{GuestMemory, GuestRead, Root};

/// What identifies a module, as far as guest memory says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum GuestIdentity {
    Known(ModuleIdentity),
    /// Its headers did not read: nothing says which build it is.
    HeadersUnreadable(HeaderError),
    /// The loader names no file for it, and stores keep files by name.
    Unnamed,
}

impl GuestIdentity {
    pub(crate) fn known(&self) -> Option<&ModuleIdentity> {
        match self {
            GuestIdentity::Known(identity) => Some(identity),
            GuestIdentity::HeadersUnreadable(_) | GuestIdentity::Unnamed => None,
        }
    }
}

/// The identity of the image at `base`, read under `root`, for a module the
/// loader names `name`. The engine's header reader fills buffers it owns
/// and bounds itself; an RVA past the top of memory wraps, as the
/// processor's addresses do, and fails to translate.
pub(crate) fn read_module_identity<G: GuestMemory>(
    mem: &mut G,
    root: Root,
    base: u64,
    name: Option<&str>,
) -> GuestIdentity {
    let Some(name) = name else {
        return GuestIdentity::Unnamed;
    };
    let mut read = |rva: Rva, buf: &mut [u8]| match mem.read_linear(root, base.wrapping_add(rva.0), buf) {
        GuestRead::Read => ImageRead::Read,
        GuestRead::Unreadable => ImageRead::Unreadable,
    };
    match read_pe_identity(&mut read) {
        Ok(identity) => GuestIdentity::Known(identity.module_identity(name.to_owned())),
        Err(error) => GuestIdentity::HeadersUnreadable(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debugger::fake_windows::{
        FakeWindows, FakeWindowsSpec, DEBUG_DIRECTORY_RVA, KERNEL_BASE, LIVE_CR3,
    };
    use rusty_box_symbols::identity::{Guid, PdbId, PeImageId};

    #[test]
    fn a_kernel_image_names_its_build_and_its_pdb() {
        let spec = FakeWindowsSpec::typical();
        let pdb = spec.kernel.pdb.clone();
        let mut fake = FakeWindows::build(spec);
        let read = read_module_identity(&mut fake.guest, Root::Live, KERNEL_BASE, Some("ntoskrnl.exe"));
        assert_eq!(
            read,
            GuestIdentity::Known(ModuleIdentity::Pe {
                file_name: "ntoskrnl.exe".to_owned(),
                image: PeImageId {
                    time_date_stamp: 0x5E1D_2A4B,
                    size_of_image: 0x10_0000,
                },
                pdb: Some(PdbId {
                    name: "ntkrnlmp.pdb".to_owned(),
                    guid: Guid::from_le_bytes(pdb.guid),
                    age: 1,
                }),
            })
        );
    }

    /// Review focus 4: the CodeView record's page is paged out; the image
    /// identity still finds the binary.
    #[test]
    fn a_paged_out_codeview_record_leaves_the_image_identity() {
        let mut fake = FakeWindows::build(FakeWindowsSpec::typical());
        fake.guest.page_out(LIVE_CR3, KERNEL_BASE + DEBUG_DIRECTORY_RVA);
        let read = read_module_identity(&mut fake.guest, Root::Live, KERNEL_BASE, Some("ntoskrnl.exe"));
        let Some(ModuleIdentity::Pe { image, pdb, .. }) = read.known() else {
            panic!("the headers read, so the image is known: {read:?}");
        };
        assert_eq!(image.store_key(), "5E1D2A4B100000");
        assert_eq!(*pdb, None);
    }

    #[test]
    fn a_module_whose_headers_do_not_read_has_no_identity() {
        let mut fake = FakeWindows::build(FakeWindowsSpec::typical());
        fake.guest.page_out(LIVE_CR3, KERNEL_BASE);
        let read = read_module_identity(&mut fake.guest, Root::Live, KERNEL_BASE, Some("ntoskrnl.exe"));
        assert!(
            matches!(read, GuestIdentity::HeadersUnreadable(HeaderError::Unreadable { .. })),
            "{read:?}"
        );
        assert_eq!(
            read_module_identity(&mut fake.guest, Root::Live, KERNEL_BASE, None),
            GuestIdentity::Unnamed
        );
    }
}
