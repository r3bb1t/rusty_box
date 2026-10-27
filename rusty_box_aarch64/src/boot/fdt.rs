//! Editing a flattened device tree: setting properties on top-level nodes
//! and writing the result compactly, without an allocator.
//!
//! Provenance (R7): the Devicetree Specification v0.4, chapter 5 (the
//! header, the memory reservation block, the structure block's tokens, the
//! strings block). QEMU builds the device tree it hands a guest with libfdt
//! and then sets `/chosen` and `/memory` the same way (`hw/arm/boot.c`
//! `arm_load_dtb`); this does that subset without libfdt.
//!
//! The rebuild streams the source tree into the destination twice — once to
//! measure, once to write — so it needs no scratch space, and the result is
//! the tree with each edited node's named properties replaced (or created,
//! with the node itself if it is missing) and nothing else changed.

const FDT_MAGIC: u32 = 0xD00D_FEED;
const FDT_BEGIN_NODE: u32 = 1;
const FDT_END_NODE: u32 = 2;
const FDT_PROP: u32 = 3;
const FDT_NOP: u32 = 4;
const FDT_END: u32 = 9;
const HEADER_LEN: usize = 40;

/// What an edit sets a property to.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Value<'a> {
    /// Raw bytes, written as given.
    Bytes(&'a [u8]),
    /// A string, written with its terminating NUL.
    String(&'a str),
}

impl Value<'_> {
    fn len(&self) -> usize {
        match self {
            Self::Bytes(b) => b.len(),
            Self::String(s) => s.len() + 1,
        }
    }
}

/// Set `property` of the top-level node `node` to `value`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Edit<'a> {
    pub(crate) node: &'a str,
    pub(crate) property: &'a str,
    pub(crate) value: Value<'a>,
}

/// Why a device tree could not be edited.
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeviceTreeError {
    /// The bytes do not start with the device tree magic.
    NotADeviceTree,
    /// A block or a token runs past the end of the bytes, or a node or
    /// property is malformed.
    Malformed,
    /// The edited tree does not fit where it has to go.
    TooLarge,
}

impl core::fmt::Display for DeviceTreeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::NotADeviceTree => "not a flattened device tree",
            Self::Malformed => "the device tree is malformed",
            Self::TooLarge => "the edited device tree does not fit",
        })
    }
}

impl core::error::Error for DeviceTreeError {}

fn be32(bytes: &[u8], at: usize) -> Result<u32, DeviceTreeError> {
    let field: [u8; 4] = bytes
        .get(at..at.checked_add(4).ok_or(DeviceTreeError::Malformed)?)
        .and_then(|b| b.try_into().ok())
        .ok_or(DeviceTreeError::Malformed)?;
    Ok(u32::from_be_bytes(field))
}

const fn align4(n: usize) -> usize {
    (n + 3) & !3
}

/// The NUL-terminated name at `at` in a block.
fn name_at(block: &[u8], at: usize) -> Result<&[u8], DeviceTreeError> {
    let rest = block.get(at..).ok_or(DeviceTreeError::Malformed)?;
    let len = rest.iter().position(|&b| b == 0).ok_or(DeviceTreeError::Malformed)?;
    Ok(&rest[..len])
}

/// The offset of a whole string `name` in a strings block.
fn find_string(strings: &[u8], name: &str) -> Option<usize> {
    let mut at = 0;
    while at < strings.len() {
        let len = strings[at..].iter().position(|&b| b == 0).unwrap_or(strings.len() - at);
        if &strings[at..at + len] == name.as_bytes() {
            return Some(at);
        }
        at += len + 1;
    }
    None
}

/// A destination that measures when it has no buffer.
struct Out<'a> {
    buf: &'a mut [u8],
    measuring: bool,
    pos: usize,
}

impl Out<'_> {
    fn put(&mut self, bytes: &[u8]) -> Result<(), DeviceTreeError> {
        let end = self.pos.checked_add(bytes.len()).ok_or(DeviceTreeError::TooLarge)?;
        if !self.measuring {
            self.buf.get_mut(self.pos..end).ok_or(DeviceTreeError::TooLarge)?.copy_from_slice(bytes);
        }
        self.pos = end;
        Ok(())
    }

    fn put_u32(&mut self, v: u32) -> Result<(), DeviceTreeError> {
        self.put(&v.to_be_bytes())
    }

    fn pad4(&mut self) -> Result<(), DeviceTreeError> {
        while !self.pos.is_multiple_of(4) {
            self.put(&[0])?;
        }
        Ok(())
    }
}

/// Where each edit's property name lives in the rebuilt strings block: in
/// the source block if it is there, otherwise appended after it in the order
/// edits first name it.
struct Names<'e, 'a> {
    source_len: usize,
    source: &'a [u8],
    edits: &'e [Edit<'a>],
}

impl Names<'_, '_> {
    fn offset(&self, name: &str) -> usize {
        if let Some(at) = find_string(self.source, name) {
            return at;
        }
        let mut appended = 0;
        for (i, e) in self.edits.iter().enumerate() {
            if find_string(self.source, e.property).is_some() || self.first_named(i).is_some() {
                continue;
            }
            if e.property == name {
                break;
            }
            appended += e.property.len() + 1;
        }
        self.source_len + appended
    }

    /// The earlier edit that already names edit `i`'s property, if any.
    fn first_named(&self, i: usize) -> Option<usize> {
        let name = self.edits[i].property;
        self.edits[..i].iter().position(|e| e.property == name)
    }

    /// The bytes appended after the source block.
    fn appended_len(&self) -> usize {
        (0..self.edits.len())
            .filter(|&i| find_string(self.source, self.edits[i].property).is_none() && self.first_named(i).is_none())
            .map(|i| self.edits[i].property.len() + 1)
            .sum()
    }
}

fn put_prop(out: &mut Out<'_>, names: &Names<'_, '_>, edit: &Edit<'_>) -> Result<(), DeviceTreeError> {
    out.put_u32(FDT_PROP)?;
    out.put_u32(u32::try_from(edit.value.len()).map_err(|_| DeviceTreeError::TooLarge)?)?;
    out.put_u32(u32::try_from(names.offset(edit.property)).map_err(|_| DeviceTreeError::TooLarge)?)?;
    match edit.value {
        Value::Bytes(b) => out.put(b)?,
        Value::String(s) => {
            out.put(s.as_bytes())?;
            out.put(&[0])?;
        }
    }
    out.pad4()
}

/// Write the structure block with the edits applied.
fn structure(source: &[u8], strings: &[u8], names: &Names<'_, '_>, out: &mut Out<'_>) -> Result<(), DeviceTreeError> {
    let edits = names.edits;
    let mut depth = 0usize;
    // The top-level node being copied, while its properties are.
    let mut editing: Option<&[u8]> = None;
    let mut at = 0;
    loop {
        let token = be32(source, at)?;
        match token {
            FDT_BEGIN_NODE => {
                let name = name_at(source, at + 4)?;
                let end = at + 4 + align4(name.len() + 1);
                out.put(source.get(at..end).ok_or(DeviceTreeError::Malformed)?)?;
                depth += 1;
                editing = None;
                if depth == 2 && edits.iter().any(|e| e.node.as_bytes() == name) {
                    editing = Some(name);
                    for e in edits.iter().filter(|e| e.node.as_bytes() == name) {
                        put_prop(out, names, e)?;
                    }
                }
                at = end;
            }
            FDT_END_NODE => {
                if depth == 1 {
                    // Closing the root: create each edited node it lacked.
                    for (i, e) in edits.iter().enumerate() {
                        let first_of_node = edits[..i].iter().all(|o| o.node != e.node);
                        if first_of_node && !node_exists(source, e.node)? {
                            out.put_u32(FDT_BEGIN_NODE)?;
                            out.put(e.node.as_bytes())?;
                            out.put(&[0])?;
                            out.pad4()?;
                            for same in edits.iter().filter(|o| o.node == e.node) {
                                put_prop(out, names, same)?;
                            }
                            out.put_u32(FDT_END_NODE)?;
                        }
                    }
                }
                depth = depth.checked_sub(1).ok_or(DeviceTreeError::Malformed)?;
                editing = None;
                out.put_u32(token)?;
                at += 4;
            }
            FDT_PROP => {
                let len = be32(source, at + 4)? as usize;
                let name_off = be32(source, at + 8)? as usize;
                let end = at + 12 + align4(len);
                let prop_name = name_at(strings, name_off)?;
                let replaced = match editing {
                    Some(node) => edits
                        .iter()
                        .any(|e| e.node.as_bytes() == node && e.property.as_bytes() == prop_name),
                    None => false,
                };
                if !replaced {
                    out.put(source.get(at..end).ok_or(DeviceTreeError::Malformed)?)?;
                }
                at = end;
            }
            FDT_NOP => at += 4,
            FDT_END => {
                out.put_u32(token)?;
                return Ok(());
            }
            _ => return Err(DeviceTreeError::Malformed),
        }
    }
}

/// Whether the source structure block has a top-level node named `node`.
fn node_exists(source: &[u8], node: &str) -> Result<bool, DeviceTreeError> {
    let mut depth = 0usize;
    let mut at = 0;
    loop {
        match be32(source, at)? {
            FDT_BEGIN_NODE => {
                let name = name_at(source, at + 4)?;
                depth += 1;
                if depth == 2 && name == node.as_bytes() {
                    return Ok(true);
                }
                at += 4 + align4(name.len() + 1);
            }
            FDT_END_NODE => {
                depth = depth.checked_sub(1).ok_or(DeviceTreeError::Malformed)?;
                at += 4;
            }
            FDT_PROP => at += 12 + align4(be32(source, at + 4)? as usize),
            FDT_NOP => at += 4,
            FDT_END => return Ok(false),
            _ => return Err(DeviceTreeError::Malformed),
        }
    }
}

/// Write `source` with `edits` applied into `dst`, returning the edited
/// tree's length. The result is compact: no slack after the strings block.
pub(crate) fn rebuild(source: &[u8], edits: &[Edit<'_>], dst: &mut [u8]) -> Result<usize, DeviceTreeError> {
    if be32(source, 0).ok() != Some(FDT_MAGIC) {
        return Err(DeviceTreeError::NotADeviceTree);
    }
    let off_struct = be32(source, 8)? as usize;
    let off_strings = be32(source, 12)? as usize;
    let off_rsvmap = be32(source, 16)? as usize;
    let boot_cpuid = be32(source, 28)?;
    let size_strings = be32(source, 32)? as usize;
    let size_struct = be32(source, 36)? as usize;
    let structure_src = source
        .get(off_struct..off_struct.checked_add(size_struct).ok_or(DeviceTreeError::Malformed)?)
        .ok_or(DeviceTreeError::Malformed)?;
    let strings = source
        .get(off_strings..off_strings.checked_add(size_strings).ok_or(DeviceTreeError::Malformed)?)
        .ok_or(DeviceTreeError::Malformed)?;
    // The reservation block runs to its all-zero terminating entry.
    let mut rsv_end = off_rsvmap;
    loop {
        let entry = source.get(rsv_end..rsv_end + 16).ok_or(DeviceTreeError::Malformed)?;
        rsv_end += 16;
        if entry.iter().all(|&b| b == 0) {
            break;
        }
    }
    let rsvmap = &source[off_rsvmap..rsv_end];
    let names = Names {
        source_len: strings.len(),
        source: strings,
        edits,
    };

    let mut measure = Out {
        buf: &mut [],
        measuring: true,
        pos: 0,
    };
    structure(structure_src, strings, &names, &mut measure)?;
    let new_struct_len = measure.pos;
    let new_strings_len = strings.len() + names.appended_len();
    let struct_at = HEADER_LEN + rsvmap.len();
    let strings_at = struct_at + new_struct_len;
    let total = strings_at + new_strings_len;
    if total > dst.len() {
        return Err(DeviceTreeError::TooLarge);
    }

    let mut out = Out {
        buf: dst,
        measuring: false,
        pos: 0,
    };
    let header = [
        FDT_MAGIC,
        total as u32,
        struct_at as u32,
        strings_at as u32,
        HEADER_LEN as u32,
        17,
        16,
        boot_cpuid,
        new_strings_len as u32,
        new_struct_len as u32,
    ];
    for field in header {
        out.put_u32(field)?;
    }
    out.put(rsvmap)?;
    structure(structure_src, strings, &names, &mut out)?;
    out.put(strings)?;
    for (i, e) in edits.iter().enumerate() {
        if find_string(strings, e.property).is_none() && names.first_named(i).is_none() {
            out.put(e.property.as_bytes())?;
            out.put(&[0])?;
        }
    }
    Ok(total)
}

/// The value of `property` of the top-level node `node` in `tree`, if both
/// exist — for a host that wants to check what a machine was given
/// ([`VirtMachine::device_tree`](crate::machine::VirtMachine::device_tree)).
///
/// # Errors
///
/// [`DeviceTreeError`] when `tree` is not a well-formed device tree.
pub fn property<'t>(tree: &'t [u8], node: &str, property: &str) -> Result<Option<&'t [u8]>, DeviceTreeError> {
    if be32(tree, 0).ok() != Some(FDT_MAGIC) {
        return Err(DeviceTreeError::NotADeviceTree);
    }
    let off_struct = be32(tree, 8)? as usize;
    let off_strings = be32(tree, 12)? as usize;
    let strings = tree.get(off_strings..).ok_or(DeviceTreeError::Malformed)?;
    let mut depth = 0usize;
    let mut in_node = false;
    let mut at = off_struct;
    loop {
        match be32(tree, at)? {
            FDT_BEGIN_NODE => {
                let name = name_at(tree, at + 4)?;
                depth += 1;
                in_node = depth == 2 && name == node.as_bytes();
                at += 4 + align4(name.len() + 1);
            }
            FDT_END_NODE => {
                depth = depth.checked_sub(1).ok_or(DeviceTreeError::Malformed)?;
                in_node = false;
                at += 4;
            }
            FDT_PROP => {
                let len = be32(tree, at + 4)? as usize;
                let name = name_at(strings, be32(tree, at + 8)? as usize)?;
                if in_node && name == property.as_bytes() {
                    return Ok(Some(tree.get(at + 12..at + 12 + len).ok_or(DeviceTreeError::Malformed)?));
                }
                at += 12 + align4(len);
            }
            FDT_NOP => at += 4,
            FDT_END => return Ok(None),
            _ => return Err(DeviceTreeError::Malformed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const QEMU_DTB: &[u8] = include_bytes!("../../assets/qemu-virt-a57-256m.dtb");

    #[test]
    fn edits_replace_create_and_leave_the_rest_alone() {
        let mut dst = [0u8; 64 * 1024];
        let size = 0x2000_0000u64;
        let mut reg = [0u8; 16];
        reg[..8].copy_from_slice(&0x4000_0000u64.to_be_bytes());
        reg[8..].copy_from_slice(&size.to_be_bytes());
        let edits = [
            Edit {
                node: "memory@40000000",
                property: "reg",
                value: Value::Bytes(&reg),
            },
            Edit {
                node: "chosen",
                property: "bootargs",
                value: Value::String("console=ttyAMA0"),
            },
            Edit {
                node: "rusty-box-test",
                property: "made-here",
                value: Value::Bytes(&[0, 0, 0, 1]),
            },
        ];
        let len = rebuild(QEMU_DTB, &edits, &mut dst).expect("rebuild");
        let tree = &dst[..len];
        assert_eq!(property(tree, "memory@40000000", "reg"), Ok(Some(&reg[..])));
        assert_eq!(property(tree, "chosen", "bootargs"), Ok(Some(&b"console=ttyAMA0\0"[..])));
        assert_eq!(property(tree, "rusty-box-test", "made-here"), Ok(Some(&[0u8, 0, 0, 1][..])));
        // Untouched: the UART's compatible string and chosen's stdout-path.
        assert_eq!(
            property(tree, "pl011@9000000", "compatible"),
            property(QEMU_DTB, "pl011@9000000", "compatible")
        );
        assert_eq!(property(tree, "chosen", "stdout-path"), property(QEMU_DTB, "chosen", "stdout-path"));
        assert!(len < 16 * 1024, "the rebuild is compact: {len} bytes");
    }

    #[test]
    fn a_tree_that_does_not_fit_is_refused_whole() {
        let mut dst = [0u8; 256];
        assert_eq!(rebuild(QEMU_DTB, &[], &mut dst), Err(DeviceTreeError::TooLarge));
        assert_eq!(rebuild(b"not a tree at all, honestly....", &[], &mut dst), Err(DeviceTreeError::NotADeviceTree));
    }
}
