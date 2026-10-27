//! Editing a flattened device tree: setting properties of nodes named by
//! their path, and writing the result compactly, without an allocator.
//!
//! Provenance (R7): the Devicetree Specification v0.4, chapter 5 (the
//! header, the memory reservation block, the structure block's tokens, the
//! strings block). QEMU builds the device tree it hands a guest with libfdt
//! and then sets `/chosen` and `/memory` the same way (`hw/arm/boot.c`
//! `arm_load_dtb`); this does that subset without libfdt.
//!
//! The rebuild streams the source tree into the destination twice — once to
//! measure, once to write — so it needs no scratch space beyond a stack
//! frame per level of nesting. The result is the tree libfdt makes of the
//! source when each edit, in order, is an `fdt_setprop`, with an
//! `fdt_add_subnode` before it for each node on its path that is missing,
//! laid out compactly: a property the node has is replaced where it stands
//! (`fdt_resize_property_`); one it lacks goes in front of the node's
//! properties (`fdt_add_property_`), and a node its parent lacks right after
//! the parent's properties (`fdt_add_subnode_namelen`), so of several, the
//! last added comes first. A property set twice keeps the place its first
//! setting gave it and the value of its last. Nothing else changes.

const FDT_MAGIC: u32 = 0xD00D_FEED;
const FDT_BEGIN_NODE: u32 = 1;
const FDT_END_NODE: u32 = 2;
const FDT_PROP: u32 = 3;
const FDT_NOP: u32 = 4;
const FDT_END: u32 = 9;
const HEADER_LEN: usize = 40;

/// How far below the root a node may sit: Linux's `unflatten_dt_nodes`
/// skips every node at a depth of `FDT_MAX_DEPTH - 1` (63) or more, the
/// root's depth being 0.
const MAX_DEPTH: usize = 62;

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

/// The path of a node below the root, as [`property`] reads one: its
/// components separated by `/` (`"chosen"`, `"cpus/cpu@0"`), none of them
/// empty; the empty path is the root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NodePath<'a>(&'a str);

impl<'a> NodePath<'a> {
    /// `path`, if no component of it is empty: not `"/chosen"`, `"chosen/"`
    /// or `"a//b"`.
    pub(crate) const fn new(path: &'a str) -> Option<Self> {
        let bytes = path.as_bytes();
        let mut i = 0;
        // Whether the byte at `i` starts a component.
        let mut component_starts = true;
        while i < bytes.len() {
            if bytes[i] == b'/' {
                if component_starts {
                    return None;
                }
                component_starts = true;
            } else {
                component_starts = false;
            }
            i += 1;
        }
        if component_starts && !bytes.is_empty() {
            None
        } else {
            Some(Self(path))
        }
    }

    /// The path a constant names: one with an empty component does not
    /// compile there.
    pub(crate) const fn of(path: &'a str) -> Self {
        match Self::new(path) {
            Some(path) => path,
            None => panic!("a node path has no empty component"),
        }
    }
}

/// Set `property` of the node at `node` to `value`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Edit<'a> {
    pub(crate) node: NodePath<'a>,
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
    /// A node sits deeper than a guest reads one: more than 62 levels below
    /// the root, where Linux's `unflatten_dt_nodes` skips it.
    TooDeep,
}

impl core::fmt::Display for DeviceTreeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::NotADeviceTree => "not a flattened device tree",
            Self::Malformed => "the device tree is malformed",
            Self::TooLarge => "the edited device tree does not fit",
            Self::TooDeep => "the device tree nests a node deeper than a guest reads",
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

/// The first offset in a strings block at which `name` and its terminating
/// NUL stand, the tail of a longer string included, as libfdt looks a name
/// up before it adds one (`fdt_find_string_len_`).
fn find_string(strings: &[u8], name: &str) -> Option<usize> {
    let name = name.as_bytes();
    strings
        .windows(name.len() + 1)
        .position(|w| w[..name.len()] == *name && w[name.len()] == 0)
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

/// Where each edit's property name lives in the rebuilt strings block, as
/// libfdt's `fdt_find_add_string_` places names edit by edit: where the
/// block already holds it, as a whole string or the tail of one, in the
/// source block or among the names earlier edits appended; otherwise
/// appended after them.
struct Names<'e, 'a> {
    source_len: usize,
    source: &'a [u8],
    edits: &'e [Edit<'a>],
}

impl Names<'_, '_> {
    /// Whether edit `i` appends its property's name. An earlier name that is
    /// not in the source block ends up in the block, appended or as the tail
    /// of one appended before it, so a name that is its tail is found.
    fn appends(&self, i: usize) -> bool {
        let name = self.edits[i].property;
        find_string(self.source, name).is_none()
            && !self.edits[..i]
                .iter()
                .any(|e| e.property.ends_with(name) && find_string(self.source, e.property).is_none())
    }

    /// The offset of `name`, which an edit sets, in the rebuilt block: the
    /// first place it stands, in the source block or in the first name
    /// appended that ends with it.
    fn offset(&self, name: &str) -> usize {
        if let Some(at) = find_string(self.source, name) {
            return at;
        }
        let mut appended = 0;
        for (i, e) in self.edits.iter().enumerate() {
            if !self.appends(i) {
                continue;
            }
            if e.property.ends_with(name) {
                return self.source_len + appended + e.property.len() - name.len();
            }
            appended += e.property.len() + 1;
        }
        self.source_len + appended
    }

    /// The bytes appended after the source block.
    fn appended_len(&self) -> usize {
        (0..self.edits.len())
            .filter(|&i| self.appends(i))
            .map(|i| self.edits[i].property.len() + 1)
            .sum()
    }
}

/// Write a property holding `edit`'s value, its name at `name_off` in the
/// strings block, followed by `padding` when that is as long as the padding
/// the value needs and by zeros otherwise.
fn put_prop(out: &mut Out<'_>, name_off: usize, edit: &Edit<'_>, padding: &[u8]) -> Result<(), DeviceTreeError> {
    let len = edit.value.len();
    out.put_u32(FDT_PROP)?;
    out.put_u32(u32::try_from(len).map_err(|_| DeviceTreeError::TooLarge)?)?;
    out.put_u32(u32::try_from(name_off).map_err(|_| DeviceTreeError::TooLarge)?)?;
    match edit.value {
        Value::Bytes(b) => out.put(b)?,
        Value::String(s) => {
            out.put(s.as_bytes())?;
            out.put(&[0])?;
        }
    }
    if padding.len() == align4(len) - len {
        out.put(padding)
    } else {
        out.pad4()
    }
}

/// A node being written and the nodes it is in, innermost first: its path,
/// read from the end. The root has no parent and is at depth 0.
struct Chain<'c, 'n> {
    name: &'n [u8],
    parent: Option<&'c Chain<'c, 'n>>,
    depth: usize,
}

impl<'c, 'n> Chain<'c, 'n> {
    /// The node `name` in `parent`, or the root when there is no parent;
    /// [`DeviceTreeError::TooDeep`] below [`MAX_DEPTH`].
    fn inside(parent: Option<&'c Chain<'c, 'n>>, name: &'n [u8]) -> Result<Self, DeviceTreeError> {
        let depth = parent.map_or(0, |p| p.depth + 1);
        if depth > MAX_DEPTH {
            return Err(DeviceTreeError::TooDeep);
        }
        Ok(Self { name, parent, depth })
    }

    /// What is left of `path` below this node, if `path` runs through it:
    /// `Some("")` when `path` names the node itself.
    fn below<'p>(&self, path: NodePath<'p>) -> Option<&'p str> {
        let Some(parent) = self.parent else {
            return Some(path.0);
        };
        let rest = parent.below(path)?;
        let (first, rest) = rest.split_once('/').unwrap_or((rest, ""));
        (!first.is_empty() && first.as_bytes() == self.name).then_some(rest)
    }

    /// Whether `path` names this node.
    fn is(&self, path: NodePath<'_>) -> bool {
        self.below(path) == Some("")
    }

    /// The child of this node that `path` runs through, if `path` runs
    /// below it.
    fn child_toward<'p>(&self, path: NodePath<'p>) -> Option<&'p str> {
        match self.below(path)? {
            "" => None,
            rest => Some(rest.split_once('/').map_or(rest, |(first, _)| first)),
        }
    }
}

/// Write the structure block with the edits applied: its one root node,
/// then `FDT_END`.
fn structure(source: &[u8], strings: &[u8], names: &Names<'_, '_>, out: &mut Out<'_>) -> Result<(), DeviceTreeError> {
    let root = skip_nops(source, 0)?;
    if be32(source, root)? != FDT_BEGIN_NODE {
        return Err(DeviceTreeError::Malformed);
    }
    let after = skip_nops(source, node(source, root, None, strings, names, out)?)?;
    if be32(source, after)? != FDT_END {
        return Err(DeviceTreeError::Malformed);
    }
    out.put_u32(FDT_END)
}

/// The offset of the first token at or after `at` that is not `FDT_NOP`.
fn skip_nops(source: &[u8], mut at: usize) -> Result<usize, DeviceTreeError> {
    while be32(source, at)? == FDT_NOP {
        at += 4;
    }
    Ok(at)
}

/// A node of the source: the source's structure and strings blocks, and the
/// offset of the node's first token after its name.
#[derive(Clone, Copy)]
struct SourceNode<'s> {
    structure: &'s [u8],
    strings: &'s [u8],
    body: usize,
}

/// Copy the node whose `FDT_BEGIN_NODE` is at `at` in `source`, in
/// `parent`, with the edits applied: the properties edits add, its own with
/// the edited ones replaced in place, the nodes edits create in it, then its
/// own children. Returns the offset after its `FDT_END_NODE`. `FDT_NOP`s
/// are dropped.
fn node(
    source: &[u8],
    at: usize,
    parent: Option<&Chain<'_, '_>>,
    strings: &[u8],
    names: &Names<'_, '_>,
    out: &mut Out<'_>,
) -> Result<usize, DeviceTreeError> {
    let name = name_at(source, at + 4)?;
    let body = at + 4 + align4(name.len() + 1);
    out.put(source.get(at..body).ok_or(DeviceTreeError::Malformed)?)?;
    let here = Chain::inside(parent, name)?;
    let contents = SourceNode {
        structure: source,
        strings,
        body,
    };
    added_properties(Some(contents), &here, names, out)?;
    let mut children_created = false;
    let mut at = body;
    loop {
        match be32(source, at)? {
            FDT_BEGIN_NODE => {
                if !children_created {
                    created_children(Some(contents), &here, names, out)?;
                    children_created = true;
                }
                at = node(source, at, Some(&here), strings, names, out)?;
            }
            FDT_PROP => {
                let len = be32(source, at + 4)? as usize;
                let name_off = be32(source, at + 8)? as usize;
                let end = at + 12 + align4(len);
                let prop_name = name_at(strings, name_off)?;
                // Replaced where it stands, under the name it had; a value of
                // the length it had keeps the padding it had, as
                // `fdt_setprop` writes such a value over the old one alone.
                // A property after a child, where the Devicetree
                // Specification (5.4.2) puts none, libfdt does not find
                // (`nextprop_` stops at a node), and it stays as it is.
                let setting = if children_created {
                    None
                } else {
                    last_setting(&here, prop_name, names.edits)
                };
                match setting {
                    Some(edit) if edit.value.len() == len => {
                        let padding = source.get(at + 12 + len..end).ok_or(DeviceTreeError::Malformed)?;
                        put_prop(out, name_off, edit, padding)?;
                    }
                    Some(edit) => put_prop(out, name_off, edit, &[])?,
                    None => out.put(source.get(at..end).ok_or(DeviceTreeError::Malformed)?)?,
                }
                at = end;
            }
            FDT_NOP => at += 4,
            FDT_END_NODE => {
                if !children_created {
                    created_children(Some(contents), &here, names, out)?;
                }
                out.put_u32(FDT_END_NODE)?;
                return Ok(at + 4);
            }
            _ => return Err(DeviceTreeError::Malformed),
        }
    }
}

/// The last edit that sets `property` of `here`: the value the property
/// ends with.
fn last_setting<'e, 'a>(here: &Chain<'_, '_>, property: &[u8], edits: &'e [Edit<'a>]) -> Option<&'e Edit<'a>> {
    edits.iter().rev().find(|e| e.property.as_bytes() == property && here.is(e.node))
}

/// Write the properties edits give `here` that its source node, `contents`,
/// lacks (a node being created has none): each with its last value, the one
/// first set last first, as `fdt_add_property_` puts each new property in
/// front of the others.
fn added_properties(
    contents: Option<SourceNode<'_>>,
    here: &Chain<'_, '_>,
    names: &Names<'_, '_>,
    out: &mut Out<'_>,
) -> Result<(), DeviceTreeError> {
    let edits = names.edits;
    for (i, e) in edits.iter().enumerate().rev() {
        let first = !edits[..i].iter().any(|o| o.property == e.property && here.is(o.node));
        if !here.is(e.node) || !first {
            continue;
        }
        let present = match contents {
            Some(node) => has_property(node, e.property)?,
            None => false,
        };
        if !present {
            if let Some(last) = last_setting(here, e.property.as_bytes(), edits) {
                put_prop(out, names.offset(e.property), last, &[])?;
            }
        }
    }
    Ok(())
}

/// Create in `here` each child an edit's path runs through that its source
/// node, `contents`, lacks (a node being created has none): the one first
/// named last first, as `fdt_add_subnode_namelen` puts each new node in
/// front of the others, right after the parent's properties.
fn created_children(
    contents: Option<SourceNode<'_>>,
    here: &Chain<'_, '_>,
    names: &Names<'_, '_>,
    out: &mut Out<'_>,
) -> Result<(), DeviceTreeError> {
    let edits = names.edits;
    for (i, e) in edits.iter().enumerate().rev() {
        let Some(child) = here.child_toward(e.node) else {
            continue;
        };
        if edits[..i].iter().any(|o| here.child_toward(o.node) == Some(child)) {
            continue;
        }
        let exists = match contents {
            Some(node) => has_child(node.structure, node.body, child)?,
            None => false,
        };
        if !exists {
            create(child, here, names, out)?;
        }
    }
    Ok(())
}

/// Write the node `name`, which the source lacks, in `parent`: the
/// properties edits give it and the nodes they create in it.
fn create(name: &str, parent: &Chain<'_, '_>, names: &Names<'_, '_>, out: &mut Out<'_>) -> Result<(), DeviceTreeError> {
    let here = Chain::inside(Some(parent), name.as_bytes())?;
    out.put_u32(FDT_BEGIN_NODE)?;
    out.put(name.as_bytes())?;
    out.put(&[0])?;
    out.pad4()?;
    added_properties(None, &here, names, out)?;
    created_children(None, &here, names, out)?;
    out.put_u32(FDT_END_NODE)
}

/// Whether the source node `node` has a property named `name` before its
/// first child, where libfdt looks (`nextprop_`).
fn has_property(node: SourceNode<'_>, name: &str) -> Result<bool, DeviceTreeError> {
    let mut at = node.body;
    loop {
        match be32(node.structure, at)? {
            FDT_PROP => {
                let len = be32(node.structure, at + 4)? as usize;
                if name_at(node.strings, be32(node.structure, at + 8)? as usize)? == name.as_bytes() {
                    return Ok(true);
                }
                at += 12 + align4(len);
            }
            FDT_NOP => at += 4,
            FDT_BEGIN_NODE | FDT_END_NODE => return Ok(false),
            _ => return Err(DeviceTreeError::Malformed),
        }
    }
}

/// Whether the node whose first token after its name is at `body` in
/// `source` has a child named `name`.
fn has_child(source: &[u8], body: usize, name: &str) -> Result<bool, DeviceTreeError> {
    // Levels below the node.
    let mut depth = 0usize;
    let mut at = body;
    loop {
        match be32(source, at)? {
            FDT_BEGIN_NODE => {
                let child = name_at(source, at + 4)?;
                if depth == 0 && child == name.as_bytes() {
                    return Ok(true);
                }
                depth += 1;
                at += 4 + align4(child.len() + 1);
            }
            FDT_END_NODE => {
                if depth == 0 {
                    return Ok(false);
                }
                depth -= 1;
                at += 4;
            }
            FDT_PROP => at += 12 + align4(be32(source, at + 4)? as usize),
            FDT_NOP => at += 4,
            _ => return Err(DeviceTreeError::Malformed),
        }
    }
}

/// Write `source` with `edits` applied into `dst`, returning the edited
/// tree's length. The result is compact: no slack after the strings block.
/// A tree, or an edit's path, with a node deeper than [`MAX_DEPTH`] is
/// refused with [`DeviceTreeError::TooDeep`].
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
        if names.appends(i) {
            out.put(e.property.as_bytes())?;
            out.put(&[0])?;
        }
    }
    Ok(total)
}

/// The value of `property` of the node at `path` in `tree`, if both exist —
/// for a host that wants to check what a machine was given
/// ([`VirtMachine::device_tree`](crate::machine::VirtMachine::device_tree)).
/// `path` names the node below the root, its components separated by `/`
/// (`"chosen"`, `"cpus/cpu@0"`); the empty path is the root.
///
/// # Errors
///
/// [`DeviceTreeError`] when `tree` is not a well-formed device tree.
pub fn property<'t>(tree: &'t [u8], path: &str, property: &str) -> Result<Option<&'t [u8]>, DeviceTreeError> {
    if be32(tree, 0).ok() != Some(FDT_MAGIC) {
        return Err(DeviceTreeError::NotADeviceTree);
    }
    let off_struct = be32(tree, 8)? as usize;
    let off_strings = be32(tree, 12)? as usize;
    let strings = tree.get(off_strings..).ok_or(DeviceTreeError::Malformed)?;
    let want = if path.is_empty() { 0 } else { path.split('/').count() };
    // The root is at depth 1; the node at depth `d` matches `path`'s
    // component `d - 2`. `matched` counts the open nodes that match, which
    // are always the outermost ones below the root.
    let mut depth = 0usize;
    let mut matched = 0usize;
    let mut at = off_struct;
    loop {
        match be32(tree, at)? {
            FDT_BEGIN_NODE => {
                let name = name_at(tree, at + 4)?;
                depth += 1;
                if depth >= 2
                    && matched == depth - 2
                    && path.split('/').nth(matched).filter(|_| matched < want).map(str::as_bytes) == Some(name)
                {
                    matched += 1;
                }
                at += 4 + align4(name.len() + 1);
            }
            FDT_END_NODE => {
                if depth >= 2 && matched == depth - 1 {
                    matched -= 1;
                }
                depth = depth.checked_sub(1).ok_or(DeviceTreeError::Malformed)?;
                at += 4;
            }
            FDT_PROP => {
                let len = be32(tree, at + 4)? as usize;
                let name = name_at(strings, be32(tree, at + 8)? as usize)?;
                if matched == want && depth == want + 1 && name == property.as_bytes() {
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
                node: NodePath::of("memory@40000000"),
                property: "reg",
                value: Value::Bytes(&reg),
            },
            Edit {
                node: NodePath::of("chosen"),
                property: "bootargs",
                value: Value::String("console=ttyAMA0"),
            },
            Edit {
                node: NodePath::of("rusty-box-test"),
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
    fn a_property_is_found_by_its_node_path() {
        assert_eq!(property(QEMU_DTB, "", "compatible"), Ok(Some(&b"linux,dummy-virt\0"[..])));
        assert_eq!(property(QEMU_DTB, "", "dma-coherent"), Ok(Some(&[][..])));
        // `cpu@0` follows its sibling `cpu-map`, whose nested subtree matches
        // no further than `cpus`.
        assert_eq!(property(QEMU_DTB, "cpus/cpu@0", "compatible"), Ok(Some(&b"arm,cortex-a57\0"[..])));
        assert_eq!(property(QEMU_DTB, "aliases", "serial0"), Ok(Some(&b"/pl011@9000000\0"[..])));
        assert_eq!(
            property(QEMU_DTB, "cpus/cpu-map/socket0/cluster0/core0", "cpu"),
            Ok(Some(&0x8001u32.to_be_bytes()[..]))
        );
        // A path names nodes from the root down, and only the node at its end.
        assert_eq!(property(QEMU_DTB, "cpu@0", "compatible"), Ok(None));
        assert_eq!(property(QEMU_DTB, "cpus", "compatible"), Ok(None));
        assert_eq!(property(QEMU_DTB, "cpus/cpu@0/l2-cache", "compatible"), Ok(None));
        assert_eq!(property(QEMU_DTB, "cpus/core0", "cpu"), Ok(None));
        assert_eq!(property(QEMU_DTB, "chosen", "compatible"), Ok(None));
    }

    /// An edit names a node by its path at any depth: an existing node's
    /// property is replaced and its others kept, a missing node is created
    /// with each missing node on its path, once however many edits run
    /// through it, and the root's own properties can be set.
    #[test]
    fn edits_reach_nodes_at_any_depth() {
        let mut dst = [0u8; 64 * 1024];
        let edits = [
            Edit {
                node: NodePath::of("cpus/cpu@0"),
                property: "compatible",
                value: Value::String("arm,armv8"),
            },
            Edit {
                node: NodePath::of("cpus/cpu@1"),
                property: "reg",
                value: Value::Bytes(&[0, 0, 0, 1]),
            },
            Edit {
                node: NodePath::of("rusty/box/a"),
                property: "made-here",
                value: Value::Bytes(&[0, 0, 0, 2]),
            },
            Edit {
                node: NodePath::of("rusty/box/b"),
                property: "made-here",
                value: Value::Bytes(&[0, 0, 0, 3]),
            },
            Edit {
                node: NodePath::of(""),
                property: "model",
                value: Value::String("rusty_box virt"),
            },
        ];
        let len = rebuild(QEMU_DTB, &edits, &mut dst).expect("rebuild");
        let tree = &dst[..len];
        assert_eq!(property(tree, "cpus/cpu@0", "compatible"), Ok(Some(&b"arm,armv8\0"[..])));
        for kept in ["reg", "device_type", "phandle", "d-cache-size"] {
            assert_eq!(property(tree, "cpus/cpu@0", kept), property(QEMU_DTB, "cpus/cpu@0", kept), "{kept}");
            assert!(property(tree, "cpus/cpu@0", kept).is_ok_and(|v| v.is_some()), "{kept}");
        }
        assert_eq!(
            property(tree, "cpus/cpu-map/socket0/cluster0/core0", "cpu"),
            property(QEMU_DTB, "cpus/cpu-map/socket0/cluster0/core0", "cpu")
        );
        assert_eq!(property(tree, "cpus", "#address-cells"), property(QEMU_DTB, "cpus", "#address-cells"));
        assert_eq!(property(tree, "cpus/cpu@1", "reg"), Ok(Some(&[0u8, 0, 0, 1][..])));
        assert_eq!(property(tree, "rusty/box/a", "made-here"), Ok(Some(&[0u8, 0, 0, 2][..])));
        assert_eq!(property(tree, "rusty/box/b", "made-here"), Ok(Some(&[0u8, 0, 0, 3][..])));
        assert_eq!(property(tree, "", "model"), Ok(Some(&b"rusty_box virt\0"[..])));
        assert_eq!(property(tree, "", "compatible"), property(QEMU_DTB, "", "compatible"));
        // `rusty` and `box` are each created once: one FDT_BEGIN_NODE apiece.
        let begins = |name: &[u8]| {
            let mut token = [0u8; 12];
            token[..4].copy_from_slice(&FDT_BEGIN_NODE.to_be_bytes());
            token[4..4 + name.len()].copy_from_slice(name);
            let token = &token[..4 + align4(name.len() + 1)];
            tree.windows(token.len()).filter(|w| *w == token).count()
        };
        assert_eq!((begins(b"rusty"), begins(b"box"), begins(b"cpus")), (1, 1, 1));
    }

    /// The names of the properties of the node at `path` (as [`property`]
    /// reads one), in order, then of its children.
    fn layout(tree: &[u8], path: &str) -> (std::vec::Vec<std::string::String>, std::vec::Vec<std::string::String>) {
        use std::string::String;
        use std::vec::Vec;
        let word = |at: usize| u32::from_be_bytes([tree[at], tree[at + 1], tree[at + 2], tree[at + 3]]);
        let text = |at: usize| {
            let end = at + tree[at..].iter().position(|&b| b == 0).expect("a NUL");
            String::from_utf8(tree[at..end].to_vec()).expect("UTF-8")
        };
        let (structure, strings) = (word(8) as usize, word(12) as usize);
        let (mut open, mut properties, mut children) = (Vec::<String>::new(), Vec::new(), Vec::new());
        let mut at = structure;
        loop {
            let here = open.get(1..).map(|o| o.join("/")).unwrap_or_default();
            match word(at) {
                FDT_BEGIN_NODE => {
                    let name = text(at + 4);
                    if !open.is_empty() && here == path {
                        children.push(name.clone());
                    }
                    at += 4 + align4(name.len() + 1);
                    open.push(name);
                }
                FDT_END_NODE => {
                    open.pop();
                    at += 4;
                }
                FDT_PROP => {
                    if !open.is_empty() && here == path {
                        properties.push(text(strings + word(at + 8) as usize));
                    }
                    at += 12 + align4(word(at + 4) as usize);
                }
                FDT_NOP => at += 4,
                _ => return (properties, children),
            }
        }
    }

    /// The edits land where libfdt's `fdt_setprop` and `fdt_add_subnode`
    /// put them, so the board's tree is laid out as QEMU's: QEMU 11.1.2,
    /// `-M virt,dumpdtb -cpu cortex-a57 -kernel -initrd -append`, gives
    /// `/chosen` as `linux,initrd-end`, `linux,initrd-start`, `bootargs`,
    /// then the properties it had, having set them (`arm_load_dtb`) in
    /// the opposite order; and it keeps `/memory`'s `reg` before its
    /// `device_type`, set in that opposite order too. An edit that sets a
    /// property to the value it has changes nothing, and a property set
    /// twice keeps its first place and its last value.
    #[test]
    fn edits_land_where_libfdt_puts_them() {
        let mut dst = [0u8; 64 * 1024];
        let reg = [0, 0, 0, 0, 0x40, 0, 0, 0, 0, 0, 0, 0, 0x20, 0, 0, 0];
        let edits = [
            Edit {
                node: NodePath::of("memory@40000000"),
                property: "device_type",
                value: Value::String("memory"),
            },
            Edit {
                node: NodePath::of("memory@40000000"),
                property: "reg",
                value: Value::Bytes(&reg),
            },
            Edit {
                node: NodePath::of("chosen"),
                property: "bootargs",
                value: Value::String("console=ttyS0"),
            },
            Edit {
                node: NodePath::of("chosen"),
                property: "linux,initrd-start",
                value: Value::Bytes(&[0, 0, 0, 0, 0x48, 0, 0, 0]),
            },
            Edit {
                node: NodePath::of("chosen"),
                property: "linux,initrd-end",
                value: Value::Bytes(&[0, 0, 0, 0, 0x48, 0, 0x10, 0]),
            },
            Edit {
                node: NodePath::of("chosen"),
                property: "bootargs",
                value: Value::String("console=ttyAMA0"),
            },
            Edit {
                node: NodePath::of("made/first"),
                property: "n",
                value: Value::Bytes(&[1]),
            },
            Edit {
                node: NodePath::of("made/second"),
                property: "n",
                value: Value::Bytes(&[2]),
            },
        ];
        let len = rebuild(QEMU_DTB, &edits, &mut dst).expect("rebuild");
        let tree = &dst[..len];
        let (chosen, _) = layout(tree, "chosen");
        let (source_chosen, _) = layout(QEMU_DTB, "chosen");
        let mut want = std::vec!["linux,initrd-end", "linux,initrd-start", "bootargs"];
        want.extend(source_chosen.iter().map(|p| p.as_str()));
        assert_eq!(chosen, want);
        assert_eq!(property(tree, "chosen", "bootargs"), Ok(Some(&b"console=ttyAMA0\0"[..])));
        assert_eq!(layout(tree, "memory@40000000"), layout(QEMU_DTB, "memory@40000000"));
        assert_eq!(layout(QEMU_DTB, "memory@40000000").0, ["reg", "device_type"]);
        assert_eq!(property(tree, "memory@40000000", "reg"), Ok(Some(&reg[..])));
        // `made` is the root's first child, its children last made first.
        let (_, root_children) = layout(tree, "");
        let (_, source_root_children) = layout(QEMU_DTB, "");
        assert_eq!(root_children[0], "made");
        assert_eq!(root_children[1..], source_root_children[..]);
        assert_eq!(layout(tree, "made").1, ["second", "first"]);

        // The processor node set to what it is: nothing moves.
        let same = [
            Edit {
                node: NodePath::of("cpus/cpu@0"),
                property: "compatible",
                value: Value::String("arm,cortex-a57"),
            },
            Edit {
                node: NodePath::of("cpus/cpu@0"),
                property: "reg",
                value: Value::Bytes(&[0, 0, 0, 0]),
            },
        ];
        let mut unedited = [0u8; 64 * 1024];
        let unedited_len = rebuild(QEMU_DTB, &[], &mut unedited).expect("rebuild");
        let same_len = rebuild(QEMU_DTB, &same, &mut dst).expect("rebuild");
        assert_eq!(&dst[..same_len], &unedited[..unedited_len]);
    }

    /// A property after a child, where the Devicetree Specification puts
    /// none, is copied as it stands: libfdt does not find it, so setting it
    /// adds another in front of the node's properties, as here.
    #[test]
    fn a_property_after_a_child_stays_where_it_is() {
        let mut tree = std::vec::Vec::new();
        let header_len = HEADER_LEN + 16;
        let mut structure = std::vec::Vec::new();
        let p_is_1 = u32::from_be_bytes([1, 0, 0, 0]);
        for token in [FDT_BEGIN_NODE, 0, FDT_BEGIN_NODE, u32::from_be_bytes(*b"n\0\0\0"), FDT_END_NODE, FDT_PROP, 1, 0, p_is_1, FDT_END_NODE, FDT_END] {
            structure.extend(token.to_be_bytes());
        }
        let strings_at = header_len + structure.len();
        for field in [FDT_MAGIC, strings_at as u32 + 2, header_len as u32, strings_at as u32, HEADER_LEN as u32, 17, 16, 0, 2, structure.len() as u32] {
            tree.extend(field.to_be_bytes());
        }
        tree.extend([0u8; 16]);
        tree.extend(structure);
        tree.extend(b"p\0");
        let mut dst = [0u8; 1024];
        assert_eq!(rebuild(&tree, &[], &mut dst), Ok(tree.len()));
        assert_eq!(&dst[..tree.len()], &tree[..]);
        let edit = Edit {
            node: NodePath::of(""),
            property: "p",
            value: Value::Bytes(&[2]),
        };
        let len = rebuild(&tree, &[edit], &mut dst).expect("rebuild");
        assert_eq!(layout(&dst[..len], ""), (std::vec!["p".into(), "p".into()], std::vec!["n".into()]));
        assert_eq!(property(&dst[..len], "", "p"), Ok(Some(&[2u8][..])), "the one set comes first");
        // The one after the child still holds 1.
        let mut trailing = std::vec::Vec::new();
        for token in [FDT_END_NODE, FDT_PROP, 1, 0, p_is_1, FDT_END_NODE] {
            trailing.extend(token.to_be_bytes());
        }
        assert!(dst[..len].windows(trailing.len()).any(|w| w == &trailing[..]));
    }

    /// A path names no node through an empty component; the root is the
    /// empty path.
    #[test]
    fn a_node_path_has_no_empty_component() {
        for refused in ["/", "/chosen", "chosen/", "cpus//cpu@0"] {
            assert_eq!(NodePath::new(refused), None, "{refused}");
        }
        for path in ["", "chosen", "cpus/cpu@0"] {
            assert_eq!(NodePath::new(path), Some(NodePath(path)));
        }
    }

    /// A name a new property needs is found where libfdt finds it, the tail
    /// of a name an earlier edit appended included; so is one the source
    /// holds as the tail of another (`#address-cells` holds `cells`). A
    /// name that only begins another (`compat`, of `compatible`) is not
    /// there, and is appended.
    #[test]
    fn a_new_name_is_found_in_the_names_before_it() {
        let edits = [
            Edit {
                node: NodePath::of("chosen"),
                property: "compat",
                value: Value::Bytes(&[4]),
            },
            Edit {
                node: NodePath::of("chosen"),
                property: "rusty,box",
                value: Value::Bytes(&[1]),
            },
            Edit {
                node: NodePath::of("chosen"),
                property: "box",
                value: Value::Bytes(&[2]),
            },
            Edit {
                node: NodePath::of("chosen"),
                property: "cells",
                value: Value::Bytes(&[3]),
            },
        ];
        let mut dst = [0u8; 64 * 1024];
        let len = rebuild(QEMU_DTB, &edits, &mut dst).expect("rebuild");
        let tree = &dst[..len];
        let source_strings = be32(QEMU_DTB, 32).expect("a header") as usize;
        let appended = "compat".len() + 1 + "rusty,box".len() + 1;
        assert_eq!(be32(tree, 32), Ok((source_strings + appended) as u32), "two names appended");
        assert_eq!(property(tree, "chosen", "compat"), Ok(Some(&[4u8][..])));
        assert_eq!(property(tree, "chosen", "compatible"), Ok(None));
        assert_eq!(property(tree, "chosen", "rusty,box"), Ok(Some(&[1u8][..])));
        assert_eq!(property(tree, "chosen", "box"), Ok(Some(&[2u8][..])));
        assert_eq!(property(tree, "chosen", "cells"), Ok(Some(&[3u8][..])));
    }

    /// A node 62 levels below the root is the deepest a guest reads, so an
    /// edit may create one; one level more is refused, as is a source tree
    /// that nests one.
    #[test]
    fn a_node_deeper_than_a_guest_reads_is_refused() {
        use std::string::String;
        use std::vec::Vec;

        fn path(levels: usize) -> String {
            let mut path = String::from("n");
            for _ in 1..levels {
                path.push_str("/n");
            }
            path
        }
        fn edit(node: &str) -> Edit<'_> {
            Edit {
                node: NodePath::new(node).expect("a path"),
                property: "deep",
                value: Value::Bytes(&[]),
            }
        }
        // A tree of nothing but a chain of `levels` nodes named `n` below
        // the root.
        fn nested(levels: usize) -> Vec<u8> {
            let mut structure = Vec::new();
            for name in core::iter::once(&b""[..]).chain(core::iter::repeat_n(&b"n"[..], levels)) {
                structure.extend(FDT_BEGIN_NODE.to_be_bytes());
                structure.extend(name);
                structure.resize(structure.len() + 4 - name.len() % 4, 0);
            }
            for _ in 0..=levels {
                structure.extend(FDT_END_NODE.to_be_bytes());
            }
            structure.extend(FDT_END.to_be_bytes());
            let struct_at = HEADER_LEN + 16;
            let strings_at = struct_at + structure.len();
            let header = [
                FDT_MAGIC,
                strings_at as u32,
                struct_at as u32,
                strings_at as u32,
                HEADER_LEN as u32,
                17,
                16,
                0,
                0,
                structure.len() as u32,
            ];
            let mut tree: Vec<u8> = header.iter().flat_map(|f| f.to_be_bytes()).collect();
            tree.extend([0u8; 16]);
            tree.extend(structure);
            tree
        }

        // Linux skips a node at depth FDT_MAX_DEPTH - 1 = 63 or more.
        let deepest_read = 62;
        let mut dst = [0u8; 64 * 1024];
        let deepest = path(deepest_read);
        let len = rebuild(QEMU_DTB, &[edit(&deepest)], &mut dst).expect("62 levels");
        assert_eq!(property(&dst[..len], &deepest, "deep"), Ok(Some(&[][..])));
        let too_deep = path(deepest_read + 1);
        assert_eq!(rebuild(QEMU_DTB, &[edit(&too_deep)], &mut dst), Err(DeviceTreeError::TooDeep));

        let tree = nested(deepest_read);
        assert_eq!(rebuild(&tree, &[], &mut dst), Ok(tree.len()));
        assert_eq!(&dst[..tree.len()], &tree[..], "an unedited tree is copied as it is");
        assert_eq!(rebuild(&tree, &[edit(&too_deep)], &mut dst), Err(DeviceTreeError::TooDeep));
        assert_eq!(rebuild(&nested(deepest_read + 1), &[], &mut dst), Err(DeviceTreeError::TooDeep));
    }

    #[test]
    fn a_tree_that_does_not_fit_is_refused_whole() {
        let mut dst = [0u8; 256];
        assert_eq!(rebuild(QEMU_DTB, &[], &mut dst), Err(DeviceTreeError::TooLarge));
        assert_eq!(rebuild(b"not a tree at all, honestly....", &[], &mut dst), Err(DeviceTreeError::NotADeviceTree));
    }
}
