#!/usr/bin/env python3
"""Generate the host observer profile from matching Debian kernel ELFs."""

import argparse
import hashlib
from pathlib import Path

from elftools.elf.elffile import ELFFile


# The file and field paths are the ABI the observer actually reads. A missing
# or differently sized member is an unsupported build.
FIELDS = (
    ("TASK_PID", "task_struct", "pid", 4),
    ("TASK_TGID", "task_struct", "tgid", 4),
    ("TASK_START", "task_struct", "start_boottime", 8),
    ("TASK_COMM", "task_struct", "comm", 16),
    ("TASK_MM", "task_struct", "mm", 4),
    ("MM_PGD", "mm_struct", "pgd", 4),
    ("MM_EXE_FILE", "mm_struct", "exe_file", 4),
    ("MM_MT_ROOT", "mm_struct", "mm_mt.ma_root", 4),
    ("MAPLE_TREE_ROOT", "maple_tree", "ma_root", 4),
    ("MM_LOCK_COUNT", "mm_struct", "mmap_lock.count.counter", 4),
    ("MM_AUXV", "mm_struct", "saved_auxv", None),
    ("FILE_DENTRY", "file", "f_path.dentry", 4),
    ("DENTRY_NAME", "dentry", "d_name.name", 4),
    ("DENTRY_NAME_LEN", "dentry", "d_name.len", 4),
    ("VMA_START", "vm_area_struct", "vm_start", 4),
    ("VMA_END", "vm_area_struct", "vm_end", 4),
    ("VMA_FLAGS", "vm_area_struct", "vm_flags", 4),
    ("VMA_MM", "vm_area_struct", "vm_mm", 4),
    ("VMA_FILE", "vm_area_struct", "vm_file", 4),
    ("VMA_PGOFF", "vm_area_struct", "vm_pgoff", 4),
    ("PT_IP", "pt_regs", "ip", 4),
    ("PT_CS", "pt_regs", "cs", 2),
    ("MAPLE_PARENT", "maple_node", "parent", 4),
    ("MAPLE_RANGE_SLOT", "maple_node", "mr64.slot", None),
    ("MAPLE_RANGE_PIVOT", "maple_node", "mr64.pivot", None),
    ("MAPLE_RANGE_END", "maple_node", "mr64.meta.end", 1),
    ("MAPLE_ARANGE_SLOT", "maple_node", "ma64.slot", None),
    ("MAPLE_ARANGE_PIVOT", "maple_node", "ma64.pivot", None),
    ("MAPLE_ARANGE_END", "maple_node", "ma64.meta.end", 1),
)

SYMBOLS = {
    "PER_CPU_OFFSETS": "__per_cpu_offset",
    "CURRENT_TASK": "current_task",
    "RETURN_TO_USER": "restore_all_switch_stack",
}

MAPLE_TYPES = {
    "LEAF_KIND": "maple_leaf_64",
    "RANGE_KIND": "maple_range_64",
    "ARANGE_KIND": "maple_arange_64",
}


def member_type(die):
    t = die.get_DIE_from_attribute("DW_AT_type")
    while t.tag in ("DW_TAG_const_type", "DW_TAG_volatile_type", "DW_TAG_typedef"):
        t = t.get_DIE_from_attribute("DW_AT_type")
    return t


def size(die):
    if "DW_AT_byte_size" in die.attributes:
        return die.attributes["DW_AT_byte_size"].value
    if die.tag == "DW_TAG_pointer_type":
        return die.cu["address_size"]
    if die.tag == "DW_TAG_array_type":
        count = 1
        for subrange in die.iter_children():
            attrs = subrange.attributes
            count *= attrs["DW_AT_count"].value if "DW_AT_count" in attrs else attrs["DW_AT_upper_bound"].value + 1
        return count * size(member_type(die))
    raise ValueError(f"unknown size for {die.tag}")


def member(die, field):
    for child in die.iter_children():
        if child.tag != "DW_TAG_member":
            continue
        attrs = child.attributes
        location = attrs.get("DW_AT_data_member_location")
        offset = location.value if location else 0
        if "DW_AT_name" not in attrs:
            nested = member(member_type(child), field)
            if nested is not None:
                return offset + nested[0], nested[1]
        elif attrs["DW_AT_name"].value.decode() == field:
            if not isinstance(offset, int) or "DW_AT_bit_size" in attrs:
                raise ValueError(f"unsupported member layout for {field}")
            return offset, member_type(child)
    return None


def locate(die, path):
    offset = 0
    for field in path.split("."):
        found = member(die, field)
        if found is None:
            raise KeyError(path)
        step, die = found
        offset += step
    return offset, size(die)


def definitions(dwarf):
    desired = {struct for _, struct, _, _ in FIELDS} | {"maple_type"}
    found = {}
    preferred = ("init/main.c", "kernel/fork.c", "mm/mmap.c", "lib/maple_tree.c", "arch/x86/kernel/process_32.c")
    for scan_all in (False, True):
        for unit in dwarf.iter_CUs():
            name = unit.get_top_DIE().attributes.get("DW_AT_name")
            if not scan_all and not (name and name.value.decode().endswith(preferred)):
                continue
            for die in unit.iter_DIEs():
                if die.tag not in ("DW_TAG_structure_type", "DW_TAG_enumeration_type"):
                    continue
                attrs = die.attributes
                if "DW_AT_name" not in attrs or "DW_AT_byte_size" not in attrs:
                    continue
                symbol = attrs["DW_AT_name"].value.decode()
                if symbol in desired and symbol not in found:
                    found[symbol] = die
                    if found.keys() == desired:
                        return found
    raise ValueError(f"missing DWARF definitions {sorted(desired - found.keys())}")


def build_id(elf):
    section = elf.get_section_by_name(".notes")
    note = next(section.iter_notes())
    if (
        note["n_name"] != "GNU"
        or note["n_type"] != "NT_GNU_BUILD_ID"
        or note["n_namesz"] != 4
        or note["n_descsz"] != 20
    ):
        raise ValueError("expected the GNU build ID as the first ELF note")
    return section["sh_addr"] + 16, note["n_desc"]


def generate(bzimage, boot, debug):
    for elf in (boot, debug):
        if elf.elfclass != 32 or not elf.little_endian or elf["e_machine"] != "EM_386" or elf["e_type"] != "ET_EXEC":
            raise ValueError("expected a little-endian i386 kernel ELF")
    address, identity = build_id(boot)
    if (address, identity) != build_id(debug):
        raise ValueError("debug ELF build ID or note address disagrees with the boot ELF")
    text = boot.get_section_by_name(".text")["sh_addr"]
    load = next(s for s in boot.iter_segments() if s["p_type"] == "PT_LOAD" and s["p_vaddr"] <= text < s["p_vaddr"] + s["p_memsz"])

    print("// Generated by guest/extract_i386_layout.py against the Debian boot and debug ELFs.")
    print("pub(super) const KERNEL_SHA256: &str =")
    print(f'    "{hashlib.sha256(bzimage.read_bytes()).hexdigest()}";')
    as_literal = "".join(f"\\x{byte:02x}" for byte in bytes.fromhex(identity))
    print("pub(super) const BUILD_ID: &[u8; 20] =")
    print(f'    b"{as_literal}";')
    print(f"pub(super) const BUILD_ID_ADDRESS: u32 = 0x{address:X};")
    print(f"pub(super) const KERNEL_BASE: u32 = 0x{load['p_vaddr'] - load['p_paddr']:X};")
    symtab = debug.get_section_by_name(".symtab")
    for name, symbol in SYMBOLS.items():
        values = {sym["st_value"] for sym in (symtab.get_symbol_by_name(symbol) or [])}
        if len(values) != 1:
            raise ValueError(f"expected one address for {symbol}")
        print(f"pub(super) const {name}: u32 = 0x{values.pop():X};")

    # The linked ET_EXEC already carries the resolved DWARF offsets, even
    # though the kernel debug ELF retains its relocation tables.
    definitions_by_name = definitions(debug.get_dwarf_info(relocate_dwarf_sections=False))
    for name, struct, field, expected_size in FIELDS:
        offset, length = locate(definitions_by_name[struct], field)
        if expected_size is not None and length != expected_size:
            raise ValueError(f"unexpected size for {struct}.{field}: {length}")
        print(f"pub(super) const {name}: u32 = 0x{offset:X};")
        if expected_size is None:
            print(f"pub(super) const {name}_SIZE: usize = {length};")
    print(f"pub(super) const MAPLE_NODE_SIZE: usize = {size(definitions_by_name['maple_node'])};")
    variants = {
        die.attributes["DW_AT_name"].value.decode(): die.attributes["DW_AT_const_value"].value
        for die in definitions_by_name["maple_type"].iter_children()
        if die.tag == "DW_TAG_enumerator"
    }
    for name, symbol in MAPLE_TYPES.items():
        print(f"pub(super) const {name}: u32 = {variants[symbol]};")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("bzimage", type=Path)
    parser.add_argument("boot_elf", type=Path)
    parser.add_argument("debug_elf", type=Path)
    args = parser.parse_args()
    with args.boot_elf.open("rb") as boot, args.debug_elf.open("rb") as debug:
        generate(args.bzimage, ELFFile(boot), ELFFile(debug))


if __name__ == "__main__":
    main()
