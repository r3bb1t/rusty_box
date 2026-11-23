//! The pinned Linux 6.1 Maple Tree representation for `mm_struct.mm_mt`.

use std::collections::BTreeSet;

use rusty_box::cpu::Instrumentation;

use super::layout::*;
use super::{GuestMemory, ObservationError, kernel_pointer};
use crate::linux::MappedRegion;

const VM_READ: u32 = 1;
const VM_EXEC: u32 = 4;
const NODE_MASK: u32 = MAPLE_NODE_SIZE as u32 - 1;
const NODE_TYPE_SHIFT: u32 = 3;
const NODE_TYPE_MASK: u32 = 0xF;
const ROOT_TAG: u32 = 2;
const MAX_NODES: usize = 128;
const MAX_VMAS: usize = 512;

#[derive(Default)]
pub(super) struct Ranges {
    pub(super) executable: Vec<MappedRegion>,
    pub(super) header: Option<MappedRegion>,
}

struct NodeRange {
    encoded: u32,
    low: u32,
    high: u32,
    parent: Option<u32>,
}

struct NodeLayout {
    slot: u32,
    pivot: u32,
    pivots: usize,
    slots: usize,
    end: usize,
}

const RANGE: NodeLayout = NodeLayout {
    slot: MAPLE_RANGE_SLOT,
    pivot: MAPLE_RANGE_PIVOT,
    pivots: MAPLE_RANGE_PIVOT_SIZE / 4,
    slots: MAPLE_RANGE_SLOT_SIZE / 4,
    end: MAPLE_RANGE_END as usize,
};
const ALLOCATION_RANGE: NodeLayout = NodeLayout {
    slot: MAPLE_ARANGE_SLOT,
    pivot: MAPLE_ARANGE_PIVOT,
    pivots: MAPLE_ARANGE_PIVOT_SIZE / 4,
    slots: MAPLE_ARANGE_SLOT_SIZE / 4,
    end: MAPLE_ARANGE_END as usize,
};

fn malformed() -> ObservationError {
    ObservationError::Invalid("Maple Tree node or range")
}

fn node_word(raw: &[u8; MAPLE_NODE_SIZE], offset: u32, index: usize) -> u32 {
    // The generated arrays and element sizes fix each indexed read in the
    // node. Callers validate the slot or pivot index before reading it.
    let start = offset as usize + index * 4;
    u32::from_le_bytes([raw[start], raw[start + 1], raw[start + 2], raw[start + 3]])
}

fn last_slot(
    raw: &[u8; MAPLE_NODE_SIZE],
    layout: &NodeLayout,
    kind: u32,
    high: u32,
) -> Result<usize, ObservationError> {
    // Linux lib/maple_tree.c ma_data_end. In a full range node the metadata
    // shares storage with the last slot, so the final pivot selects the form.
    let last_pivot = layout.pivots - 1;
    let pivot = node_word(raw, layout.pivot, last_pivot);
    let last = if kind == ARANGE_KIND || pivot == 0 {
        usize::from(raw[layout.end])
    } else if pivot == high {
        last_pivot
    } else {
        layout.pivots
    };
    if last >= layout.slots {
        return Err(malformed());
    }
    Ok(last)
}

pub(super) fn walk<T: Instrumentation>(
    memory: &mut GuestMemory<'_, T>,
    mm: u32,
    file: u32,
    phdr: u64,
) -> Result<Ranges, ObservationError> {
    let root = memory.word(mm, MM_MT_ROOT)?;
    let mut ranges = Ranges::default();
    if root == 0 {
        return Ok(ranges);
    }
    if root & 3 != ROOT_TAG {
        return Err(ObservationError::Invalid(
            "unsupported direct Maple root entry",
        ));
    }
    let tree = mm
        .checked_add(MM_MT_ROOT - MAPLE_TREE_ROOT)
        .ok_or_else(malformed)?;
    let mut pending = vec![NodeRange {
        encoded: root,
        low: 0,
        high: u32::MAX,
        parent: None,
    }];
    let mut nodes = BTreeSet::new();
    let mut vmas = BTreeSet::new();

    while let Some(range) = pending.pop() {
        let address = kernel_pointer(range.encoded & !NODE_MASK)?;
        if nodes.len() == MAX_NODES || !nodes.insert(address) {
            return Err(malformed());
        }
        let kind = (range.encoded >> NODE_TYPE_SHIFT) & NODE_TYPE_MASK;
        let layout = match kind {
            LEAF_KIND | RANGE_KIND => &RANGE,
            ARANGE_KIND => &ALLOCATION_RANGE,
            _ => return Err(ObservationError::Invalid("unsupported Maple node type")),
        };
        let raw = memory.bytes::<MAPLE_NODE_SIZE>(address, 0)?;
        let parent = node_word(&raw, MAPLE_PARENT, 0);
        let linked = match range.parent {
            None => parent == (tree | 1),
            Some(expected) => parent & 1 == 0 && parent & !NODE_MASK == expected,
        };
        if !linked {
            return Err(malformed());
        }
        let last = last_slot(&raw, layout, kind, range.high)?;
        let mut low = range.low;
        for index in 0..=last {
            let high = if index == last {
                range.high
            } else {
                node_word(&raw, layout.pivot, index)
            };
            if high < low || high > range.high || (index != last && high == range.high) {
                return Err(malformed());
            }
            let slot = node_word(&raw, layout.slot, index);
            if kind == LEAF_KIND {
                if slot != 0 {
                    let vma = kernel_pointer(slot)?;
                    let start = memory.word(vma, VMA_START)?;
                    let end = memory.word(vma, VMA_END)?;
                    if start & 0xFFF != 0
                        || end & 0xFFF != 0
                        || start > low
                        || end <= high
                        || end > KERNEL_BASE
                        || memory.word(vma, VMA_MM)? != mm
                    {
                        return Err(ObservationError::Invalid(
                            "VMA range or owner disagrees with leaf",
                        ));
                    }
                    if vmas.insert(vma) {
                        if vmas.len() > MAX_VMAS {
                            return Err(ObservationError::Invalid("VMA scan bound"));
                        }
                        if memory.word(vma, VMA_FILE)? == file {
                            let flags = memory.word(vma, VMA_FLAGS)?;
                            let mapping = MappedRegion {
                                start: u64::from(start),
                                end: u64::from(end),
                                file_offset: u64::from(memory.word(vma, VMA_PGOFF)?) << 12,
                            };
                            if flags & VM_EXEC != 0 {
                                ranges.executable.push(mapping);
                            }
                            if flags & VM_READ != 0
                                && mapping.file_offset == 0
                                && (mapping.start..mapping.end).contains(&phdr)
                                && ranges.header.replace(mapping).is_some()
                            {
                                return Err(ObservationError::Invalid(
                                    "ambiguous mapped ELF headers",
                                ));
                            }
                        }
                    }
                }
            } else {
                if slot == 0 || pending.len() >= MAX_NODES {
                    return Err(malformed());
                }
                pending.push(NodeRange {
                    encoded: slot,
                    low,
                    high,
                    parent: Some(address),
                });
            }
            if index != last {
                low = high + 1;
            }
        }
    }
    ranges
        .executable
        .sort_unstable_by_key(|mapping| mapping.start);
    Ok(ranges)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maple_range_does_not_read_past_its_valid_slots() {
        let mut node = [0_u8; MAPLE_NODE_SIZE];
        node[RANGE.end] = RANGE.slots as u8;
        assert_eq!(
            last_slot(&node, &RANGE, LEAF_KIND, u32::MAX),
            Err(malformed())
        );
        node[RANGE.end] = 3;
        assert_eq!(last_slot(&node, &RANGE, LEAF_KIND, u32::MAX), Ok(3));
        let pivot = RANGE.pivot as usize + (RANGE.pivots - 1) * 4;
        node[pivot..pivot + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            last_slot(&node, &RANGE, LEAF_KIND, u32::MAX),
            Ok(RANGE.pivots - 1)
        );
    }
}
