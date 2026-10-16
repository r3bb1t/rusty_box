//! Decoding a Windows memory-manager region descriptor's flags, and the node
//! layouts that carry them.
//!
//! The descriptors (`MMVAD` / `MMVAD_SHORT`) form a balanced binary tree of
//! address ranges. Their node layout and their flag bit positions move
//! between Windows families, so both are selected by shape:
//!
//! - `AvlX64` — XP x64 / Server 2003 x64, Vista, Windows 7 (the `MM_AVL_TABLE`
//!   form). The node holds the tree pointers first, then 64-bit page numbers;
//!   the flags are a 64-bit word with `VadType` at bits 52..55 and
//!   `Protection` at bits 56..61.
//! - `BalancedNode` — Windows 8 through 11. The node is an
//!   `RTL_BALANCED_NODE` with 32-bit page numbers plus high bytes; the flags
//!   are a 32-bit word with `VadType` at bits 4..7 and `Protection` at bits
//!   7..12.
//!
//! The flag bits move within a family too, so without the kernel PDB they are
//! taken from [`FLAG_ERAS`], keyed by the kernel's `NtBuildNumber`: runs of
//! x64 releases whose `_MMVAD_FLAGS` the Vergilius Project shows alike. A
//! build no era covers reads its regions' ranges but not their kinds
//! ([`VadFlags::Unknown`]): the map never shows a label it cannot stand
//! behind. A guest whose kernel PDB is loaded is read by the layout the PDB
//! gives ([`VadLayout`]). `decode_flags` is pure, so it is tested without a
//! machine.

use rusty_box_symbols::units::BitRange;

/// Which Windows family's descriptor layout a guest uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum VadShape {
    AvlX64,
    BalancedNode,
}

/// What a region descriptor says a region is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum VadKind {
    /// Private committed/reserved memory (heap, stack).
    Private,
    /// A mapped file (a section view that is not an image).
    Mapped,
    /// A mapped image (an EXE or DLL).
    Image,
    /// Not read: no PDB names the flag bits, and no verified release covers
    /// the guest's build.
    Unknown,
}

/// A region's Windows protection, the base of the 5-bit `Protection` field.
/// The guard and no-cache modifiers in its high two bits are not shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WinProtection {
    NoAccess,
    ReadOnly,
    Execute,
    ExecuteRead,
    ReadWrite,
    WriteCopy,
    ExecuteReadWrite,
    ExecuteWriteCopy,
}

impl WinProtection {
    /// How it reads in the map's Access column.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::NoAccess => "---",
            Self::ReadOnly => "R",
            Self::Execute => "X",
            Self::ExecuteRead => "R X",
            Self::ReadWrite => "RW",
            Self::WriteCopy => "RWc",
            Self::ExecuteReadWrite => "RWX",
            Self::ExecuteWriteCopy => "RWXc",
        }
    }

    /// The base protection the low three bits of a `Protection` field name.
    fn from_bits(bits: u64) -> Self {
        match bits & 0x7 {
            0 => Self::NoAccess,
            1 => Self::ReadOnly,
            2 => Self::Execute,
            3 => Self::ExecuteRead,
            4 => Self::ReadWrite,
            5 => Self::WriteCopy,
            6 => Self::ExecuteReadWrite,
            _ => Self::ExecuteWriteCopy,
        }
    }
}

/// What a descriptor's flags say about its region.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct VadInfo {
    pub(crate) kind: VadKind,
    pub(crate) protection: Option<WinProtection>,
}

/// How many bytes a field takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FieldWidth {
    Dword,
    Qword,
}

impl FieldWidth {
    /// The width of a field `bytes` long, when it is one of the two.
    pub(crate) fn of_bytes(bytes: u64) -> Option<FieldWidth> {
        match bytes {
            4 => Some(FieldWidth::Dword),
            8 => Some(FieldWidth::Qword),
            _ => None,
        }
    }
}

/// What the `EPROCESS` field holding a process's descriptor tree is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum VadRootKind {
    /// An `MM_AVL_TABLE`, whose `BalancedRoot` sentinel anchors the tree.
    AvlTable,
    /// An `RTL_AVL_TREE`, whose `Root` is the root node.
    AvlTree,
}

/// Where a descriptor node's fields sit, for the tree walk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct VadNodeLayout {
    /// The tree pointers.
    pub(crate) left: u64,
    pub(crate) right: u64,
    /// The starting and ending page numbers (VPNs).
    pub(crate) start_vpn: u64,
    pub(crate) end_vpn: u64,
    /// A high byte extending each VPN (Windows 8.1+), or `None`.
    pub(crate) start_vpn_high: Option<u64>,
    pub(crate) end_vpn_high: Option<u64>,
    pub(crate) vpn_width: FieldWidth,
    /// The flags word.
    pub(crate) flags: u64,
    pub(crate) flags_width: FieldWidth,
}

/// Where the flags word keeps the bits the map reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct VadFlagBits {
    pub(crate) vad_type: BitRange,
    pub(crate) protection: BitRange,
    pub(crate) private_memory: BitRange,
}

/// Where a flags word keeps its bits, or that nothing vouches for a place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum VadFlags {
    Known(VadFlagBits),
    /// A build neither a PDB nor a verified release describes.
    Unknown,
}

impl VadFlags {
    /// What `flags` says about its region; neither kind nor protection when
    /// the bits are unknown.
    pub(crate) fn decode(self, flags: u64) -> VadInfo {
        match self {
            VadFlags::Known(bits) => decode_flags(flags, bits),
            VadFlags::Unknown => VadInfo { kind: VadKind::Unknown, protection: None },
        }
    }
}

/// A guest's descriptor tree, as the walk reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct VadLayout {
    pub(crate) root: VadRootKind,
    pub(crate) node: VadNodeLayout,
    pub(crate) flags: VadFlags,
}

/// A run of x64 releases whose `_MMVAD_FLAGS` keep the bits the map reads
/// in one place, from the first release's `NtBuildNumber` to the last's.
struct FlagEra {
    first: u32,
    last: u32,
    shape: VadShape,
    bits: VadFlagBits,
}

const fn bit_range(position: u32, length: u32) -> BitRange {
    BitRange { position, length }
}

/// The verified eras, oldest first, from the Vergilius Project's x64
/// `_MMVAD_FLAGS` pages. An era's range runs between releases read there;
/// a build outside every range reads kind unknown.
const FLAG_ERAS: [FlagEra; 5] = [
    // XP x64 SP2 (3790), Vista RTM (6000), 7 SP1 (7601): 64-bit flags,
    // CommitCharge:51, NoChange, VadType, MemCommit, Protection, Spare:2,
    // PrivateMemory.
    FlagEra {
        first: 3790,
        last: 7601,
        shape: VadShape::AvlX64,
        bits: VadFlagBits { vad_type: bit_range(52, 3), protection: bit_range(56, 5), private_memory: bit_range(63, 1) },
    },
    // 8 RTM (9200) through 10 1709 (16299): VadType, Protection,
    // PreferredNode:6, NoChange, PrivateMemory.
    FlagEra {
        first: 9200,
        last: 16299,
        shape: VadShape::BalancedNode,
        bits: VadFlagBits { vad_type: bit_range(0, 3), protection: bit_range(3, 5), private_memory: bit_range(15, 1) },
    },
    // 10 1803 (17134) and 1809 (17763): NoChange moved to the top of the
    // word, PrivateMemory follows PreferredNode:6.
    FlagEra {
        first: 17134,
        last: 17763,
        shape: VadShape::BalancedNode,
        bits: VadFlagBits { vad_type: bit_range(0, 3), protection: bit_range(3, 5), private_memory: bit_range(14, 1) },
    },
    // 10 1903 (18362) through 22H2 (19045): Lock, LockContended,
    // DeleteInProgress, NoChange, VadType, Protection, PreferredNode:6,
    // PageSize:2, PrivateMemory.
    FlagEra {
        first: 18362,
        last: 19045,
        shape: VadShape::BalancedNode,
        bits: VadFlagBits { vad_type: bit_range(4, 3), protection: bit_range(7, 5), private_memory: bit_range(20, 1) },
    },
    // Server 2022 (20348), 11 21H2 (22000) through 25H2 (26200):
    // PreferredNode grew to 7 bits.
    FlagEra {
        first: 20348,
        last: 26200,
        shape: VadShape::BalancedNode,
        bits: VadFlagBits { vad_type: bit_range(4, 3), protection: bit_range(7, 5), private_memory: bit_range(21, 1) },
    },
];

/// The flag bits of `build` when the guest's tree has `shape`: those of the
/// era covering it, else unknown (an unread build, a build between eras or
/// after the last, or a shape its era does not use).
pub(crate) fn build_flags(shape: VadShape, build: Option<u32>) -> VadFlags {
    let Some(build) = build else {
        return VadFlags::Unknown;
    };
    FLAG_ERAS
        .iter()
        .find(|era| era.shape == shape && (era.first..=era.last).contains(&build))
        .map_or(VadFlags::Unknown, |era| VadFlags::Known(era.bits))
}

/// The layout a shape's family uses, for a guest whose kernel PDB is not
/// loaded, its flag bits those of `build`'s era.
pub(crate) fn shape_layout(shape: VadShape, build: Option<u32>) -> VadLayout {
    match shape {
        // MMVAD (x64 AVL): Balance/Parent@0, LeftChild@8, RightChild@0x10,
        // StartingVpn@0x18, EndingVpn@0x20, u (flags)@0x28.
        VadShape::AvlX64 => VadLayout {
            root: VadRootKind::AvlTable,
            node: VadNodeLayout {
                left: 0x08,
                right: 0x10,
                start_vpn: 0x18,
                end_vpn: 0x20,
                start_vpn_high: None,
                end_vpn_high: None,
                vpn_width: FieldWidth::Qword,
                flags: 0x28,
                flags_width: FieldWidth::Qword,
            },
            flags: build_flags(shape, build),
        },
        // MMVAD_SHORT (x64, Win8.1+): VadNode (RTL_BALANCED_NODE) Left@0,
        // Right@8; StartingVpn@0x18, EndingVpn@0x1C, StartingVpnHigh@0x20,
        // EndingVpnHigh@0x21; u (flags)@0x30.
        VadShape::BalancedNode => VadLayout {
            root: VadRootKind::AvlTree,
            node: VadNodeLayout {
                left: 0x00,
                right: 0x08,
                start_vpn: 0x18,
                end_vpn: 0x1C,
                start_vpn_high: Some(0x20),
                end_vpn_high: Some(0x21),
                vpn_width: FieldWidth::Dword,
                flags: 0x30,
                flags_width: FieldWidth::Dword,
            },
            flags: build_flags(shape, build),
        },
    }
}

/// The flag bits a shape's family uses where a kernel PDB names its tree but
/// not one of the bits: Windows 7's, and Windows 10 1903 through 22H2's.
pub(crate) fn flag_bits(shape: VadShape) -> VadFlagBits {
    let bits = |position, length| BitRange { position, length };
    match shape {
        // 64-bit flags: CommitCharge:51, NoChange, VadType(52..54), MemCommit,
        // Protection(56..60), Spare, PrivateMemory(63).
        VadShape::AvlX64 => VadFlagBits {
            vad_type: bits(52, 3),
            protection: bits(56, 5),
            private_memory: bits(63, 1),
        },
        // 32-bit flags: Lock, LockContended, DeleteInProgress, NoChange,
        // VadType(4..6), Protection(7..11), PreferredNode, PageSize,
        // PrivateMemory(20).
        VadShape::BalancedNode => VadFlagBits {
            vad_type: bits(4, 3),
            protection: bits(7, 5),
            private_memory: bits(20, 1),
        },
    }
}

/// Decodes a descriptor's flags word by `bits`.
pub(crate) fn decode_flags(flags: u64, bits: VadFlagBits) -> VadInfo {
    // VadType 2 is VadImageMap.
    let kind = if bits.vad_type.extract(flags) == 2 {
        VadKind::Image
    } else if bits.private_memory.extract(flags) != 0 {
        VadKind::Private
    } else {
        VadKind::Mapped
    };
    VadInfo {
        kind,
        protection: Some(WinProtection::from_bits(bits.protection.extract(flags))),
    }
}

/// One region Windows' records describe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct VadRegion {
    /// First address, inclusive.
    pub(crate) start: u64,
    /// Last address plus one.
    pub(crate) end: u64,
    pub(crate) info: VadInfo,
}

/// One decoded tree node: its children's pointers (0 for none) and its
/// region.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct VadNode {
    pub(crate) left: u64,
    pub(crate) right: u64,
    pub(crate) region: VadRegion,
}

/// The most nodes a walk reads, so a corrupt or cyclic tree cannot spin.
pub(crate) const MAX_VAD_NODES: usize = 1 << 16;

/// Walks the tree rooted at `root`, reading each node through `read_node`,
/// and returns every region. A node already seen is not followed (cycles
/// terminate), and the walk stops after `cap` nodes however the tree reads.
pub(crate) fn walk<F>(root: u64, mut read_node: F, cap: usize) -> Vec<VadRegion>
where
    F: FnMut(u64) -> Option<VadNode>,
{
    let mut regions = Vec::new();
    let mut visited = std::collections::HashSet::new();
    let mut stack = vec![root];
    while let Some(pointer) = stack.pop() {
        if pointer == 0 || regions.len() >= cap || !visited.insert(pointer) {
            continue;
        }
        let Some(node) = read_node(pointer) else {
            continue;
        };
        regions.push(node.region);
        stack.push(node.left);
        stack.push(node.right);
    }
    regions
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tree whose nodes never repeat (each points at two fresh children)
    /// is stopped by the cap, not run forever.
    #[test]
    fn a_cyclic_or_endless_tree_stops_at_the_cap() {
        let region = VadRegion {
            start: 0,
            end: 0x1000,
            info: VadInfo {
                kind: VadKind::Private,
                protection: Some(WinProtection::ReadWrite),
            },
        };
        // Every pointer yields a node with two never-before-seen children.
        let regions = walk(
            1,
            |pointer| {
                Some(VadNode {
                    left: pointer * 2,
                    right: pointer * 2 + 1,
                    region,
                })
            },
            100,
        );
        assert_eq!(regions.len(), 100);
    }

    /// A small tree is walked whole; a child pointing back at the root is a
    /// cycle that terminates.
    #[test]
    fn a_small_tree_is_walked_and_a_cycle_terminates() {
        let node = |start: u64, left: u64, right: u64| VadNode {
            left,
            right,
            region: VadRegion {
                start,
                end: start + 0x1000,
                info: VadInfo {
                    kind: VadKind::Private,
                    protection: None,
                },
            },
        };
        // 1 -> (2, 3); 3 -> (1, 0): the back-edge to 1 is a cycle.
        let regions = walk(
            1,
            |pointer| match pointer {
                1 => Some(node(0x1000, 2, 3)),
                2 => Some(node(0x2000, 0, 0)),
                3 => Some(node(0x3000, 1, 0)),
                _ => None,
            },
            MAX_VAD_NODES,
        );
        let mut starts: Vec<u64> = regions.iter().map(|r| r.start).collect();
        starts.sort_unstable();
        assert_eq!(starts, [0x1000, 0x2000, 0x3000]);
    }

    #[test]
    fn flags_decode_per_shape() {
        let avl = flag_bits(VadShape::AvlX64);
        // AVL x64: VadType 2 (image), Protection = ReadOnly (1) at bit 56.
        let img = decode_flags((2u64 << 52) | (1u64 << 56), avl);
        assert_eq!(img.kind, VadKind::Image);
        assert_eq!(img.protection, Some(WinProtection::ReadOnly));

        // AVL x64: VadType 0, PrivateMemory (bit 63) → private.
        let priv_x64 = decode_flags((4u64 << 56) | (1u64 << 63), avl);
        assert_eq!(priv_x64.kind, VadKind::Private);
        assert_eq!(priv_x64.protection, Some(WinProtection::ReadWrite));

        // Win8.1–11: VadType 1 (mapped), Protection = ReadWrite (4) at bit 7.
        let bn = decode_flags((1u64 << 4) | (4u64 << 7), flag_bits(VadShape::BalancedNode));
        assert_eq!(bn.kind, VadKind::Mapped);
        assert_eq!(bn.protection, Some(WinProtection::ReadWrite));
    }

    /// One verified release, and where its era keeps two of the bits.
    struct Release {
        build: u32,
        shape: VadShape,
        vad_type: u32,
        private_memory: u32,
    }

    fn release(build: u32, shape: VadShape, vad_type: u32, private_memory: u32) -> Release {
        Release { build, shape, vad_type, private_memory }
    }

    /// Every release whose `_MMVAD_FLAGS` was read from the Vergilius Project
    /// decodes by its own bits.
    #[test]
    fn each_verified_release_reads_by_its_eras_bits() {
        use VadShape::{AvlX64, BalancedNode};
        let releases = [
            release(3790, AvlX64, 52, 63),
            release(6000, AvlX64, 52, 63),
            release(7601, AvlX64, 52, 63),
            release(9200, BalancedNode, 0, 15),
            release(9600, BalancedNode, 0, 15),
            release(10240, BalancedNode, 0, 15),
            release(10586, BalancedNode, 0, 15),
            release(14393, BalancedNode, 0, 15),
            release(15063, BalancedNode, 0, 15),
            release(16299, BalancedNode, 0, 15),
            release(17134, BalancedNode, 0, 14),
            release(17763, BalancedNode, 0, 14),
            release(18362, BalancedNode, 4, 20),
            release(18363, BalancedNode, 4, 20),
            release(19041, BalancedNode, 4, 20),
            release(19045, BalancedNode, 4, 20),
            release(20348, BalancedNode, 4, 21),
            release(22000, BalancedNode, 4, 21),
            release(22621, BalancedNode, 4, 21),
            release(22631, BalancedNode, 4, 21),
            release(26100, BalancedNode, 4, 21),
            release(26200, BalancedNode, 4, 21),
        ];
        for release in releases {
            let VadFlags::Known(bits) = build_flags(release.shape, Some(release.build)) else {
                panic!("build {} has no era", release.build);
            };
            assert_eq!(bits.vad_type.position, release.vad_type, "build {}", release.build);
            assert_eq!(bits.private_memory.position, release.private_memory, "build {}", release.build);
        }
    }

    /// Windows 10 1809 keeps `PrivateMemory` at bit 14: its private regions
    /// read private, where 1903's bits would call them mapped.
    #[test]
    fn a_1809_private_region_reads_private() {
        let flags = (4u64 << 3) | (1 << 14);
        let VadFlags::Known(bits) = build_flags(VadShape::BalancedNode, Some(17763)) else {
            panic!("1809 has an era");
        };
        assert_eq!(decode_flags(flags, bits), VadInfo { kind: VadKind::Private, protection: Some(WinProtection::ReadWrite) });
    }

    /// A build no era covers — unread, between eras, newer than the last,
    /// or on a tree shape its era does not use — reads no kind and no
    /// protection.
    #[test]
    fn a_build_no_era_covers_reads_kind_unknown() {
        let unknown = [
            build_flags(VadShape::BalancedNode, None),
            build_flags(VadShape::BalancedNode, Some(17000)),
            build_flags(VadShape::BalancedNode, Some(19500)),
            build_flags(VadShape::BalancedNode, Some(27000)),
            build_flags(VadShape::AvlX64, Some(19045)),
            build_flags(VadShape::BalancedNode, Some(7601)),
        ];
        for flags in unknown {
            assert_eq!(flags, VadFlags::Unknown);
            assert_eq!(flags.decode(u64::MAX), VadInfo { kind: VadKind::Unknown, protection: None });
        }
    }

    /// A build whose `PrivateMemory` moved (Windows 11 and Server 2022 keep
    /// it at bit 21) decodes right once its PDB says where.
    #[test]
    fn flags_decode_where_the_layout_puts_them() {
        let moved = VadFlagBits {
            private_memory: BitRange { position: 21, length: 1 },
            ..flag_bits(VadShape::BalancedNode)
        };
        let flags = (4u64 << 7) | (1 << 21);
        assert_eq!(decode_flags(flags, moved).kind, VadKind::Private);
        assert_eq!(decode_flags(flags, flag_bits(VadShape::BalancedNode)).kind, VadKind::Mapped);
    }

    #[test]
    fn protection_covers_every_base_value() {
        let labels: Vec<&str> = (0..8).map(|b| WinProtection::from_bits(b).label()).collect();
        assert_eq!(labels, ["---", "R", "X", "R X", "RW", "RWc", "RWX", "RWXc"]);
    }
}
