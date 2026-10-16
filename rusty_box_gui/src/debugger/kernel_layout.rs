//! The kernel structure layouts the Windows walker reads, taken from the
//! kernel's own PDB: global variables by RVA, structure members by offset,
//! and the VAD flag bits. A member the PDB does not have (an older build
//! without it), or one whose lookup failed, is `None`, and the walker finds
//! that member by discovery instead.

use std::sync::Arc;

use rusty_box_symbols::identity::ModuleIdentity;
use rusty_box_symbols::tables::{MemberLocation, SymbolTables};
use rusty_box_symbols::units::{FieldOffset, Rva};

use super::drivers::ObjectHeaderEra;
use super::vad::{self, FieldWidth, VadFlagBits, VadFlags, VadLayout, VadNodeLayout, VadRootKind, VadShape};

/// The kernel variables the walker starts from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct KernelGlobals {
    pub(crate) initial_system_process: Option<Rva>,
    pub(crate) active_process_head: Option<Rva>,
    pub(crate) loaded_module_list: Option<Rva>,
    pub(crate) root_directory_object: Option<Rva>,
    pub(crate) service_descriptor_table: Option<Rva>,
}

/// `_EPROCESS` members, from the start of the `_EPROCESS`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ProcessFields {
    pub(crate) unique_process_id: Option<FieldOffset>,
    pub(crate) active_process_links: Option<FieldOffset>,
    pub(crate) image_file_name: Option<FieldOffset>,
    /// `Pcb.DirectoryTableBase`.
    pub(crate) directory_table_base: Option<FieldOffset>,
    pub(crate) peb: Option<FieldOffset>,
    pub(crate) inherited_from_unique_process_id: Option<FieldOffset>,
    pub(crate) wow64_process: Option<FieldOffset>,
    pub(crate) thread_list_head: Option<FieldOffset>,
    pub(crate) vad_root: Option<FieldOffset>,
}

/// `_ETHREAD` members, from the start of the `_ETHREAD` (its `Tcb`, the
/// `_KTHREAD`, starts it).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ThreadFields {
    pub(crate) thread_list_entry: Option<FieldOffset>,
    pub(crate) cid: Option<FieldOffset>,
    pub(crate) start_address: Option<FieldOffset>,
    pub(crate) win32_start_address: Option<FieldOffset>,
    /// `Tcb.Teb`.
    pub(crate) teb: Option<FieldOffset>,
    /// `Tcb.TrapFrame`.
    pub(crate) trap_frame: Option<FieldOffset>,
}

/// What one kernel build's PDB says about the structures the walker reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct KernelLayout {
    /// The kernel build the PDB describes.
    pub(crate) kernel: ModuleIdentity,
    pub(crate) globals: KernelGlobals,
    pub(crate) process: ProcessFields,
    pub(crate) thread: ThreadFields,
    /// The descriptor tree's layout, when the PDB gives its root, its node
    /// and its flags word; a flag bit it lacks is the family's.
    pub(crate) vad: Option<VadLayout>,
    pub(crate) object_era: Option<ObjectHeaderEra>,
    /// The lookups that failed rather than missed, each with its reason; the
    /// members they were for are discovered.
    pub(crate) failures: Vec<String>,
}

/// Whether two reads were made under the same layouts: the same handed-over
/// value, or none for both.
pub(crate) fn same_layouts(a: Option<&Arc<KernelLayout>>, b: Option<&Arc<KernelLayout>>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        (None, None) => true,
        (Some(_), None) | (None, Some(_)) => false,
    }
}

/// Member lookups against one file of tables, keeping what failed.
struct Lookups<'a> {
    tables: &'a SymbolTables,
    failures: Vec<String>,
}

impl Lookups<'_> {
    fn member(&mut self, type_name: &str, path: &[&str]) -> Option<MemberLocation> {
        match self.tables.member_location(type_name, path) {
            Ok(found) => found,
            Err(error) => {
                self.failures
                    .push(format!("{type_name}.{}: {error}", path.join(".")));
                None
            }
        }
    }

    fn offset(&mut self, type_name: &str, path: &[&str]) -> Option<FieldOffset> {
        self.member(type_name, path).map(|member| member.offset)
    }

    /// The first of `paths` the type has: a member renamed between builds.
    fn first(&mut self, type_name: &str, paths: &[&[&str]]) -> Option<MemberLocation> {
        paths.iter().find_map(|path| self.member(type_name, path))
    }
}

impl KernelLayout {
    /// The layouts `tables`, the PDB of `kernel`, give.
    pub(crate) fn from_tables(kernel: ModuleIdentity, tables: &SymbolTables) -> KernelLayout {
        let mut lookups = Lookups {
            tables,
            failures: Vec::new(),
        };
        let globals = KernelGlobals {
            initial_system_process: tables.address_of("PsInitialSystemProcess"),
            active_process_head: tables.address_of("PsActiveProcessHead"),
            loaded_module_list: tables.address_of("PsLoadedModuleList"),
            root_directory_object: tables.address_of("ObpRootDirectoryObject"),
            service_descriptor_table: tables.address_of("KeServiceDescriptorTable"),
        };
        const EPROCESS: &str = "_EPROCESS";
        let process = ProcessFields {
            unique_process_id: lookups.offset(EPROCESS, &["UniqueProcessId"]),
            active_process_links: lookups.offset(EPROCESS, &["ActiveProcessLinks"]),
            image_file_name: lookups.offset(EPROCESS, &["ImageFileName"]),
            directory_table_base: lookups.offset(EPROCESS, &["Pcb", "DirectoryTableBase"]),
            peb: lookups.offset(EPROCESS, &["Peb"]),
            inherited_from_unique_process_id: lookups
                .offset(EPROCESS, &["InheritedFromUniqueProcessId"]),
            wow64_process: lookups
                .first(EPROCESS, &[&["WoW64Process"], &["Wow64Process"]])
                .map(|member| member.offset),
            thread_list_head: lookups.offset(EPROCESS, &["ThreadListHead"]),
            vad_root: lookups.offset(EPROCESS, &["VadRoot"]),
        };
        const ETHREAD: &str = "_ETHREAD";
        let thread = ThreadFields {
            thread_list_entry: lookups.offset(ETHREAD, &["ThreadListEntry"]),
            cid: lookups.offset(ETHREAD, &["Cid"]),
            start_address: lookups.offset(ETHREAD, &["StartAddress"]),
            win32_start_address: lookups.offset(ETHREAD, &["Win32StartAddress"]),
            teb: lookups.offset(ETHREAD, &["Tcb", "Teb"]),
            trap_frame: lookups.offset(ETHREAD, &["Tcb", "TrapFrame"]),
        };
        let vad = vad_layout(&mut lookups);
        let object_era = if lookups.member("_OBJECT_HEADER", &["InfoMask"]).is_some() {
            Some(ObjectHeaderEra::Modern)
        } else if lookups.member("_OBJECT_HEADER", &["NameInfoOffset"]).is_some() {
            Some(ObjectHeaderEra::Legacy)
        } else {
            None
        };
        KernelLayout {
            kernel,
            globals,
            process,
            thread,
            vad,
            object_era,
            failures: lookups.failures,
        }
    }
}

/// The descriptor tree's layout: its root's kind from `_EPROCESS.VadRoot`,
/// its node from `_MMVAD_SHORT`, its flag bits from `_MMVAD_FLAGS`.
fn vad_layout(lookups: &mut Lookups<'_>) -> Option<VadLayout> {
    const NODE: &str = "_MMVAD_SHORT";
    let root = if lookups
        .member("_EPROCESS", &["VadRoot", "BalancedRoot"])
        .is_some()
    {
        VadRootKind::AvlTable
    } else if lookups.member("_EPROCESS", &["VadRoot", "Root"]).is_some() {
        VadRootKind::AvlTree
    } else {
        return None;
    };
    // A flag bit the PDB does not name is where the root's family keeps it.
    let family = match root {
        VadRootKind::AvlTable => VadShape::AvlX64,
        VadRootKind::AvlTree => VadShape::BalancedNode,
    };
    let left = lookups.first(NODE, &[&["VadNode", "Left"], &["VadNode", "LeftChild"], &["LeftChild"]])?;
    let right = lookups.first(NODE, &[&["VadNode", "Right"], &["VadNode", "RightChild"], &["RightChild"]])?;
    let start_vpn = lookups.member(NODE, &["StartingVpn"])?;
    let end_vpn = lookups.member(NODE, &["EndingVpn"])?;
    let flags = lookups.first(NODE, &[&["u", "VadFlags"], &["u", "LongFlags"]])?;
    let node = VadNodeLayout {
        left: left.offset.0,
        right: right.offset.0,
        start_vpn: start_vpn.offset.0,
        end_vpn: end_vpn.offset.0,
        start_vpn_high: lookups
            .member(NODE, &["StartingVpnHigh"])
            .map(|member| member.offset.0),
        end_vpn_high: lookups
            .member(NODE, &["EndingVpnHigh"])
            .map(|member| member.offset.0),
        vpn_width: start_vpn.size.and_then(FieldWidth::of_bytes)?,
        flags: flags.offset.0,
        flags_width: flags.size.and_then(FieldWidth::of_bytes)?,
    };
    let family_bits = vad::flag_bits(family);
    let mut bit = |name: &str, fallback| {
        lookups
            .member("_MMVAD_FLAGS", &[name])
            .and_then(|member| member.bits)
            .unwrap_or(fallback)
    };
    let flags = VadFlagBits {
        vad_type: bit("VadType", family_bits.vad_type),
        protection: bit("Protection", family_bits.protection),
        private_memory: bit("PrivateMemory", family_bits.private_memory),
    };
    Some(VadLayout { root, node, flags: VadFlags::Known(flags) })
}

/// A Windows 10 22H2 kernel's PDB as the fixture builder writes it: the
/// members the walker reads, at that build's offsets (Vergilius), matching
/// `FakeWindowsSpec::typical`.
#[cfg(test)]
pub(crate) mod fixture {
    use rusty_box_symbols_fixtures::pdb::{
        FixtureField, FixturePublic, FixtureType, FixtureUdt, PdbFixture, PublicKind, UdtKind,
    };

    fn field(name: &str, offset: u64, ty: FixtureType) -> FixtureField {
        FixtureField {
            name: name.to_owned(),
            offset,
            ty,
        }
    }

    fn named(name: &str) -> FixtureType {
        FixtureType::Named(name.to_owned())
    }

    fn bits(position: u8, length: u8) -> FixtureType {
        FixtureType::Bitfield {
            storage: Box::new(FixtureType::U32),
            position,
            length,
        }
    }

    fn udt(name: &str, kind: UdtKind, size: u64, fields: Vec<FixtureField>) -> FixtureUdt {
        FixtureUdt {
            name: name.to_owned(),
            kind,
            size,
            fields,
        }
    }

    fn data(name: &str, offset: u32) -> FixturePublic {
        FixturePublic {
            name: name.to_owned(),
            section: 2,
            offset,
            kind: PublicKind::Data,
        }
    }

    /// The kernel's PDB, its globals in `.data` (RVA 0x3000) where the fake
    /// kernel keeps them.
    pub(crate) fn windows10_kernel(guid: [u8; 16], age: u32) -> PdbFixture {
        use FixtureType::{Pointer, U32, U64, U8};
        use UdtKind::{Struct, Union};
        let mut pdb = PdbFixture::new(guid, age);
        pdb.publics = vec![
            data("PsActiveProcessHead", 0x00),
            data("PsInitialSystemProcess", 0x10),
            data("PsLoadedModuleList", 0x20),
            data("ObpRootDirectoryObject", 0x30),
            data("KeServiceDescriptorTable", 0x40),
        ];
        pdb.types = vec![
            udt("_LIST_ENTRY", Struct, 0x10, vec![
                field("Flink", 0, Pointer),
                field("Blink", 8, Pointer),
            ]),
            udt("_CLIENT_ID", Struct, 0x10, vec![
                field("UniqueProcess", 0, Pointer),
                field("UniqueThread", 8, Pointer),
            ]),
            udt("_KPROCESS", Struct, 0x438, vec![field("DirectoryTableBase", 0x28, U64)]),
            udt("_RTL_AVL_TREE", Struct, 8, vec![field("Root", 0, Pointer)]),
            udt("_EPROCESS", Struct, 0xA40, vec![
                field("Pcb", 0, named("_KPROCESS")),
                field("UniqueProcessId", 0x440, Pointer),
                field("ActiveProcessLinks", 0x448, named("_LIST_ENTRY")),
                field("InheritedFromUniqueProcessId", 0x540, Pointer),
                field("Peb", 0x550, Pointer),
                field("WoW64Process", 0x580, Pointer),
                field("ImageFileName", 0x5A8, FixtureType::Array { element: Box::new(U8), count: 15 }),
                field("ThreadListHead", 0x5E0, named("_LIST_ENTRY")),
                field("VadRoot", 0x7D8, named("_RTL_AVL_TREE")),
            ]),
            udt("_KTHREAD", Struct, 0x430, vec![
                field("TrapFrame", 0x90, Pointer),
                field("Teb", 0xF0, Pointer),
            ]),
            udt("_ETHREAD", Struct, 0x500, vec![
                field("Tcb", 0, named("_KTHREAD")),
                field("StartAddress", 0x450, Pointer),
                field("Cid", 0x478, named("_CLIENT_ID")),
                field("Win32StartAddress", 0x4D0, Pointer),
                field("ThreadListEntry", 0x4E8, named("_LIST_ENTRY")),
            ]),
            udt("_RTL_BALANCED_NODE", Struct, 0x18, vec![
                field("Left", 0, Pointer),
                field("Right", 8, Pointer),
                field("ParentValue", 0x10, U64),
            ]),
            udt("_MMVAD_FLAGS", Struct, 4, vec![
                field("Lock", 0, bits(0, 1)),
                field("VadType", 0, bits(4, 3)),
                field("Protection", 0, bits(7, 5)),
                field("PrivateMemory", 0, bits(20, 1)),
            ]),
            udt("_MMVAD_SHORT", Struct, 0x40, vec![
                field("VadNode", 0, named("_RTL_BALANCED_NODE")),
                field("StartingVpn", 0x18, U32),
                field("EndingVpn", 0x1C, U32),
                field("StartingVpnHigh", 0x20, U8),
                field("EndingVpnHigh", 0x21, U8),
                field("u", 0x30, FixtureType::Nested(Box::new(udt("", Union, 4, vec![
                    field("LongFlags", 0, U32),
                    field("VadFlags", 0, named("_MMVAD_FLAGS")),
                ])))),
            ]),
            udt("_OBJECT_HEADER", Struct, 0x38, vec![
                field("PointerCount", 0, U64),
                field("HandleCount", 8, U64),
                field("TypeIndex", 0x18, U8),
                field("InfoMask", 0x1A, U8),
                field("Body", 0x30, U64),
            ]),
        ];
        pdb
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_box_symbols::identity::{Guid, PdbId, PeImageId};
    use rusty_box_symbols::units::BitRange;
    use rusty_box_symbols_fixtures::pdb::{build_pdb, FixtureType, PdbFixture};

    const GUID: [u8; 16] = [0x42; 16];

    fn kernel() -> ModuleIdentity {
        ModuleIdentity::Pe {
            file_name: "ntoskrnl.exe".to_owned(),
            image: PeImageId {
                time_date_stamp: 0x5E1D_2A4B,
                size_of_image: 0x10_0000,
            },
            pdb: Some(PdbId {
                name: "ntkrnlmp.pdb".to_owned(),
                guid: Guid::from_le_bytes(GUID),
                age: 1,
            }),
        }
    }

    fn layout_of(pdb: &PdbFixture) -> KernelLayout {
        let tables = SymbolTables::parse(&build_pdb(pdb)).expect("the fixture parses");
        KernelLayout::from_tables(kernel(), &tables)
    }

    #[test]
    fn a_kernel_pdb_gives_every_member_the_walker_reads() {
        let layout = layout_of(&fixture::windows10_kernel(GUID, 1));
        assert_eq!(layout.globals.initial_system_process, Some(Rva(0x3010)));
        assert_eq!(layout.globals.service_descriptor_table, Some(Rva(0x3040)));
        let offset = |value| Some(FieldOffset(value));
        assert_eq!(
            layout.process,
            ProcessFields {
                unique_process_id: offset(0x440),
                active_process_links: offset(0x448),
                image_file_name: offset(0x5A8),
                directory_table_base: offset(0x28),
                peb: offset(0x550),
                inherited_from_unique_process_id: offset(0x540),
                wow64_process: offset(0x580),
                thread_list_head: offset(0x5E0),
                vad_root: offset(0x7D8),
            }
        );
        assert_eq!(
            layout.thread,
            ThreadFields {
                thread_list_entry: offset(0x4E8),
                cid: offset(0x478),
                start_address: offset(0x450),
                win32_start_address: offset(0x4D0),
                teb: offset(0xF0),
                trap_frame: offset(0x90),
            }
        );
        // The fixture is a Windows 10 22H2 kernel: build 19045.
        assert_eq!(layout.vad, Some(vad::shape_layout(VadShape::BalancedNode, Some(19045))));
        assert_eq!(layout.object_era, Some(ObjectHeaderEra::Modern));
        assert!(layout.failures.is_empty(), "{:?}", layout.failures);
    }

    /// A build that moved a flag bit is read where its PDB puts it, and a
    /// bit the PDB does not name is the family's.
    #[test]
    fn vad_flag_bits_come_from_the_pdb_and_fall_back_one_by_one() {
        let mut pdb = fixture::windows10_kernel(GUID, 1);
        let flags = pdb
            .types
            .iter_mut()
            .find(|udt| udt.name == "_MMVAD_FLAGS")
            .expect("the flags type");
        flags.fields.retain(|field| field.name != "VadType");
        if let Some(private) = flags.fields.iter_mut().find(|field| field.name == "PrivateMemory") {
            private.ty = FixtureType::Bitfield {
                storage: Box::new(FixtureType::U32),
                position: 21,
                length: 1,
            };
        }
        let VadFlags::Known(bits) = layout_of(&pdb).vad.expect("the tree's layout").flags else {
            panic!("a PDB's flag bits are known");
        };
        assert_eq!(bits.private_memory, BitRange { position: 21, length: 1 });
        assert_eq!(bits.vad_type, vad::flag_bits(VadShape::BalancedNode).vad_type);
    }

    /// An older build without a member leaves just that member to discovery.
    #[test]
    fn a_member_the_pdb_lacks_is_left_to_discovery() {
        let mut pdb = fixture::windows10_kernel(GUID, 1);
        if let Some(eprocess) = pdb.types.iter_mut().find(|udt| udt.name == "_EPROCESS") {
            eprocess.fields.retain(|field| field.name != "WoW64Process");
        }
        pdb.types.retain(|udt| udt.name != "_MMVAD_SHORT");
        let layout = layout_of(&pdb);
        assert_eq!(layout.process.wow64_process, None);
        assert_eq!(layout.process.peb, Some(FieldOffset(0x550)));
        assert_eq!(layout.vad, None);
        assert!(layout.failures.is_empty(), "a miss is not a failure: {:?}", layout.failures);
    }

    /// Review focus 2: type records this reader cannot read make every miss
    /// a failure, kept with its reason; those members are discovered.
    #[test]
    fn unreadable_type_records_turn_misses_into_failures() {
        let mut pdb = fixture::windows10_kernel(GUID, 1);
        pdb.types.retain(|udt| udt.name != "_ETHREAD");
        pdb.broken_udt_records = 1;
        let layout = layout_of(&pdb);
        assert_eq!(layout.thread, ThreadFields::default());
        assert_eq!(layout.process.unique_process_id, Some(FieldOffset(0x440)));
        assert!(
            layout.failures.iter().any(|failure| failure.starts_with("_ETHREAD.Cid:")),
            "{:?}",
            layout.failures
        );
    }
}
