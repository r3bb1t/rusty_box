//! What a module is, wherever it is loaded: the exact build a file of debug
//! information must come from. A PE image names its PDB by GUID and age (its
//! RSDS record) and itself by TimeDateStamp and SizeOfImage; an ELF file
//! names itself by its GNU build-id.

use std::fmt;

/// A Windows GUID in its field form. RSDS records and PDB info streams store
/// it as three little-endian fields followed by eight bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Guid {
    pub data1: u32,
    pub data2: u16,
    pub data3: u16,
    pub data4: [u8; 8],
}

impl Guid {
    /// The GUID whose on-disk bytes are `bytes`.
    #[must_use]
    pub fn from_le_bytes(bytes: [u8; 16]) -> Guid {
        let [a0, a1, a2, a3, b0, b1, c0, c1, d0, d1, d2, d3, d4, d5, d6, d7] = bytes;
        Guid {
            data1: u32::from_le_bytes([a0, a1, a2, a3]),
            data2: u16::from_le_bytes([b0, b1]),
            data3: u16::from_le_bytes([c0, c1]),
            data4: [d0, d1, d2, d3, d4, d5, d6, d7],
        }
    }

    /// Thirty-two uppercase hex digits, each field in numeric order and no
    /// dashes: the symbol server's spelling.
    #[must_use]
    pub fn symbol_server_hex(&self) -> String {
        let [d0, d1, d2, d3, d4, d5, d6, d7] = self.data4;
        format!(
            "{:08X}{:04X}{:04X}{d0:02X}{d1:02X}{d2:02X}{d3:02X}{d4:02X}{d5:02X}{d6:02X}{d7:02X}",
            self.data1, self.data2, self.data3
        )
    }
}

impl fmt::Display for Guid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [d0, d1, d2, d3, d4, d5, d6, d7] = self.data4;
        write!(
            f,
            "{{{:08X}-{:04X}-{:04X}-{d0:02X}{d1:02X}-{d2:02X}{d3:02X}{d4:02X}{d5:02X}{d6:02X}{d7:02X}}}",
            self.data1, self.data2, self.data3
        )
    }
}

/// The PDB a PE image was linked with, from its RSDS record.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PdbId {
    /// The file-name part of the recorded path (`ntkrnlmp.pdb`).
    pub name: String,
    pub guid: Guid,
    pub age: u32,
}

impl PdbId {
    /// `<GUID><age>`: the folder a symbol store keeps this PDB under. The age
    /// is lowercase hex without padding, as dbghelp's symsrv writes it.
    #[must_use]
    pub fn store_key(&self) -> String {
        format!("{}{:x}", self.guid.symbol_server_hex(), self.age)
    }
}

/// A PE image's own identity, readable from its headers alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PeImageId {
    /// On reproducible builds this is a hash of the image, not a time.
    pub time_date_stamp: u32,
    pub size_of_image: u32,
}

impl PeImageId {
    /// `<TimeDateStamp %08X><SizeOfImage %x>`: the folder a symbol store keeps
    /// this binary under.
    #[must_use]
    pub fn store_key(&self) -> String {
        format!("{:08X}{:x}", self.time_date_stamp, self.size_of_image)
    }
}

/// An ELF file's GNU build-id (`NT_GNU_BUILD_ID`).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BuildId(pub Vec<u8>);

impl BuildId {
    /// Lowercase hex, as debuginfod and `.build-id` folders spell it.
    #[must_use]
    pub fn to_hex(&self) -> String {
        let mut out = String::new();
        for byte in &self.0 {
            out.push_str(&format!("{byte:02x}"));
        }
        out
    }
}

/// A loaded module's identity.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ModuleIdentity {
    Pe {
        /// The module's file name (`ntoskrnl.exe`).
        file_name: String,
        image: PeImageId,
        /// `None` while the image's debug directory has not been read: in a
        /// guest it may be paged out.
        pdb: Option<PdbId>,
    },
    Elf {
        file_name: String,
        build_id: BuildId,
    },
}

/// Whether a file of debug information belongs to a module.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub enum IdentityMatch {
    Same,
    Different,
    /// The module's half of the comparison is not known yet: a PE image whose
    /// PDB identity did not read, or an ELF file without a build-id.
    Unknown,
}

impl ModuleIdentity {
    #[must_use]
    pub fn file_name(&self) -> &str {
        match self {
            ModuleIdentity::Pe { file_name, .. } | ModuleIdentity::Elf { file_name, .. } => {
                file_name
            }
        }
    }

    /// Whether tables that declare `found` describe this module.
    pub fn matches(&self, found: &TablesIdentity) -> IdentityMatch {
        match (self, found) {
            (ModuleIdentity::Pe { pdb: Some(pdb), .. }, TablesIdentity::Pdb { guid, age }) => {
                if pdb.guid == *guid && pdb.age == *age {
                    IdentityMatch::Same
                } else {
                    IdentityMatch::Different
                }
            }
            (ModuleIdentity::Pe { pdb: None, .. }, TablesIdentity::Pdb { .. }) => {
                IdentityMatch::Unknown
            }
            (ModuleIdentity::Elf { build_id, .. }, TablesIdentity::Elf { build_id: Some(found) }) => {
                if build_id == found {
                    IdentityMatch::Same
                } else {
                    IdentityMatch::Different
                }
            }
            (ModuleIdentity::Elf { .. }, TablesIdentity::Elf { build_id: None }) => {
                IdentityMatch::Unknown
            }
            (ModuleIdentity::Pe { .. }, TablesIdentity::Elf { .. })
            | (ModuleIdentity::Elf { .. }, TablesIdentity::Pdb { .. }) => IdentityMatch::Different,
        }
    }

    /// This module with its PDB identity learned from its binary. An ELF
    /// module has no PDB, so it is returned unchanged.
    #[must_use]
    pub fn with_pdb(&self, pdb: PdbId) -> ModuleIdentity {
        match self {
            ModuleIdentity::Pe { file_name, image, .. } => ModuleIdentity::Pe {
                file_name: file_name.clone(),
                image: *image,
                pdb: Some(pdb),
            },
            ModuleIdentity::Elf { .. } => self.clone(),
        }
    }
}

impl fmt::Display for ModuleIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModuleIdentity::Pe { file_name, pdb: Some(pdb), .. } => {
                write!(f, "{file_name} ({} {} age {})", pdb.name, pdb.guid, pdb.age)
            }
            ModuleIdentity::Pe { file_name, image, pdb: None } => {
                write!(f, "{file_name} (image {})", image.store_key())
            }
            ModuleIdentity::Elf { file_name, build_id } => {
                write!(f, "{file_name} (build-id {})", build_id.to_hex())
            }
        }
    }
}

/// The build a file of debug information says it describes.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TablesIdentity {
    Pdb { guid: Guid, age: u32 },
    Elf { build_id: Option<BuildId> },
}

impl fmt::Display for TablesIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TablesIdentity::Pdb { guid, age } => write!(f, "PDB {guid} age {age}"),
            TablesIdentity::Elf { build_id: Some(id) } => write!(f, "build-id {}", id.to_hex()),
            TablesIdentity::Elf { build_id: None } => f.write_str("no build-id"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// dotnet/symstore `SSQP_Key_Conventions.md`, "PDB-Signature-Age":
    /// GUID {497B72F6-390A-44FC-878E-5A2D63B6CC4B}, age 1. The bytes are the
    /// GUID as an RSDS record stores it: three little-endian fields.
    const SSQP_RSDS_GUID: [u8; 16] = [
        0xF6, 0x72, 0x7B, 0x49, 0x0A, 0x39, 0xFC, 0x44, 0x87, 0x8E, 0x5A, 0x2D, 0x63, 0xB6, 0xCC,
        0x4B,
    ];

    #[test]
    fn the_symbol_server_key_swaps_the_first_three_guid_fields() {
        let guid = Guid::from_le_bytes(SSQP_RSDS_GUID);
        assert_eq!(guid.to_string(), "{497B72F6-390A-44FC-878E-5A2D63B6CC4B}");
        let pdb = PdbId { name: "foo.pdb".to_owned(), guid, age: 1 };
        assert_eq!(pdb.store_key(), "497B72F6390A44FC878E5A2D63B6CC4B1");
    }

    #[test]
    fn the_age_is_lowercase_hex_without_padding() {
        let guid = Guid::from_le_bytes(SSQP_RSDS_GUID);
        let at = |age| PdbId { name: "foo.pdb".to_owned(), guid, age }.store_key();
        assert_eq!(at(0x1A), "497B72F6390A44FC878E5A2D63B6CC4B1a");
        assert_eq!(at(16), "497B72F6390A44FC878E5A2D63B6CC4B10");
    }

    /// The raw RSDS bytes of a Windows 11 `ntoskrnl.exe` (file offset
    /// 0x5BD00), cross-checked with `llvm-readobj --coff-debug-directory`.
    #[test]
    fn a_kernel_rsds_record_gives_its_published_store_key() {
        let guid = Guid::from_le_bytes([
            0xFE, 0xC6, 0x61, 0xB6, 0x99, 0x72, 0x16, 0x1D, 0x0F, 0xE3, 0x7C, 0x48, 0xB8, 0xBC,
            0x54, 0x70,
        ]);
        let pdb = PdbId { name: "ntkrnlmp.pdb".to_owned(), guid, age: 1 };
        assert_eq!(pdb.store_key(), "B661C6FE72991D160FE37C48B8BC54701");
    }

    /// SSQP "PE-timestamp-filesize": Timestamp 0x542d574e, SizeOfImage
    /// 0xc2000 -> `542D574Ec2000`.
    #[test]
    fn the_binary_key_is_the_timestamp_then_the_image_size() {
        let image = PeImageId { time_date_stamp: 0x542D_574E, size_of_image: 0xC_2000 };
        assert_eq!(image.store_key(), "542D574Ec2000");
        let small = PeImageId { time_date_stamp: 0x1234, size_of_image: 0x1000 };
        assert_eq!(small.store_key(), "000012341000");
    }

    #[test]
    fn a_build_id_is_lowercase_hex() {
        assert_eq!(BuildId(vec![0xAB, 0x01, 0xFF]).to_hex(), "ab01ff");
        assert_eq!(BuildId(Vec::new()).to_hex(), "");
    }

    #[test]
    fn a_module_matches_only_tables_of_its_own_build() {
        let guid = Guid::from_le_bytes(SSQP_RSDS_GUID);
        let image = PeImageId { time_date_stamp: 1, size_of_image: 0x1000 };
        let pdb = PdbId { name: "a.pdb".to_owned(), guid, age: 3 };
        let module = ModuleIdentity::Pe { file_name: "a.dll".to_owned(), image, pdb: Some(pdb) };
        assert_eq!(module.matches(&TablesIdentity::Pdb { guid, age: 3 }), IdentityMatch::Same);
        assert_eq!(module.matches(&TablesIdentity::Pdb { guid, age: 4 }), IdentityMatch::Different);
        assert_eq!(
            module.matches(&TablesIdentity::Elf { build_id: Some(BuildId(vec![1])) }),
            IdentityMatch::Different
        );
        let unread = ModuleIdentity::Pe { file_name: "a.dll".to_owned(), image, pdb: None };
        assert_eq!(unread.matches(&TablesIdentity::Pdb { guid, age: 3 }), IdentityMatch::Unknown);
        let elf = ModuleIdentity::Elf { file_name: "a.so".to_owned(), build_id: BuildId(vec![1, 2]) };
        assert_eq!(
            elf.matches(&TablesIdentity::Elf { build_id: Some(BuildId(vec![1, 2])) }),
            IdentityMatch::Same
        );
        assert_eq!(elf.matches(&TablesIdentity::Elf { build_id: None }), IdentityMatch::Unknown);
    }

    #[test]
    fn learning_a_modules_pdb_keeps_its_image_identity() {
        let guid = Guid::from_le_bytes(SSQP_RSDS_GUID);
        let image = PeImageId { time_date_stamp: 7, size_of_image: 0x2000 };
        let module = ModuleIdentity::Pe { file_name: "n.dll".to_owned(), image, pdb: None };
        let pdb = PdbId { name: "n.pdb".to_owned(), guid, age: 1 };
        assert_eq!(
            module.with_pdb(pdb.clone()),
            ModuleIdentity::Pe { file_name: "n.dll".to_owned(), image, pdb: Some(pdb) }
        );
    }
}
