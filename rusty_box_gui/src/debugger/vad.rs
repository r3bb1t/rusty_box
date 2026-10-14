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
//! Bit positions are from the Vergilius Project's `_MMVAD_FLAGS` pages for
//! each family. `decode_flags` is pure, so it is tested without a machine.

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

/// Where a descriptor node's fields sit, for the tree walk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct VadNodeLayout {
    /// The tree pointers.
    pub(crate) left: u64,
    pub(crate) right: u64,
    /// The starting and ending page numbers (VPNs).
    pub(crate) start_vpn: u64,
    pub(crate) end_vpn: u64,
    /// A high byte extending each VPN (Windows 8+), or `None`.
    pub(crate) start_vpn_high: Option<u64>,
    pub(crate) end_vpn_high: Option<u64>,
    /// The flags word.
    pub(crate) flags: u64,
    /// Whether the VPNs are 8 bytes (`AvlX64`) or 4 bytes.
    pub(crate) vpn_is_u64: bool,
    /// Whether the flags word is 8 bytes (`AvlX64`) or 4 bytes.
    pub(crate) flags_is_u64: bool,
}

/// The node layout for a shape.
pub(crate) fn node_layout(shape: VadShape) -> VadNodeLayout {
    match shape {
        // MMVAD (x64 AVL): Balance/Parent@0, LeftChild@8, RightChild@0x10,
        // StartingVpn@0x18, EndingVpn@0x20, u (flags)@0x28.
        VadShape::AvlX64 => VadNodeLayout {
            left: 0x08,
            right: 0x10,
            start_vpn: 0x18,
            end_vpn: 0x20,
            start_vpn_high: None,
            end_vpn_high: None,
            flags: 0x28,
            vpn_is_u64: true,
            flags_is_u64: true,
        },
        // MMVAD_SHORT (x64, Win8+): VadNode (RTL_BALANCED_NODE) Left@0,
        // Right@8; StartingVpn@0x18, EndingVpn@0x1C, StartingVpnHigh@0x20,
        // EndingVpnHigh@0x21; u (flags)@0x30.
        VadShape::BalancedNode => VadNodeLayout {
            left: 0x00,
            right: 0x08,
            start_vpn: 0x18,
            end_vpn: 0x1C,
            start_vpn_high: Some(0x20),
            end_vpn_high: Some(0x21),
            flags: 0x30,
            vpn_is_u64: false,
            flags_is_u64: false,
        },
    }
}

/// Decodes a descriptor's flags word for the given shape.
pub(crate) fn decode_flags(flags: u64, shape: VadShape) -> VadInfo {
    let (vad_type, protection_bits, private) = match shape {
        // 64-bit flags: CommitCharge:51, NoChange, VadType(52..54), MemCommit,
        // Protection(56..60), Spare, PrivateMemory(63).
        VadShape::AvlX64 => ((flags >> 52) & 0x7, (flags >> 56) & 0x1F, (flags >> 63) & 1 != 0),
        // 32-bit flags: Lock, LockContended, DeleteInProgress, NoChange,
        // VadType(4..6), Protection(7..11), PreferredNode, PageSize,
        // PrivateMemory(20).
        VadShape::BalancedNode => ((flags >> 4) & 0x7, (flags >> 7) & 0x1F, (flags >> 20) & 1 != 0),
    };
    // VadType 2 is VadImageMap.
    let kind = if vad_type == 2 {
        VadKind::Image
    } else if private {
        VadKind::Private
    } else {
        VadKind::Mapped
    };
    VadInfo {
        kind,
        protection: Some(WinProtection::from_bits(protection_bits)),
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
        // AVL x64: VadType 2 (image), Protection = ReadOnly (1) at bit 56.
        let img = decode_flags((2u64 << 52) | (1u64 << 56), VadShape::AvlX64);
        assert_eq!(img.kind, VadKind::Image);
        assert_eq!(img.protection, Some(WinProtection::ReadOnly));

        // AVL x64: VadType 0, PrivateMemory (bit 63) → private.
        let priv_x64 = decode_flags((4u64 << 56) | (1u64 << 63), VadShape::AvlX64);
        assert_eq!(priv_x64.kind, VadKind::Private);
        assert_eq!(priv_x64.protection, Some(WinProtection::ReadWrite));

        // Win8–11: VadType 1 (mapped), Protection = ReadWrite (4) at bit 7.
        let bn = decode_flags((1u64 << 4) | (4u64 << 7), VadShape::BalancedNode);
        assert_eq!(bn.kind, VadKind::Mapped);
        assert_eq!(bn.protection, Some(WinProtection::ReadWrite));
    }

    #[test]
    fn protection_covers_every_base_value() {
        let labels: Vec<&str> = (0..8).map(|b| WinProtection::from_bits(b).label()).collect();
        assert_eq!(labels, ["---", "R", "X", "R X", "RW", "RWc", "RWX", "RWXc"]);
    }
}
