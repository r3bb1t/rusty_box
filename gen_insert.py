#!/usr/bin/env python3
"""Insert generated enum variants and match arms into typed.rs"""

with open('rusty_box_decoder/src/typed.rs', 'r', encoding='utf-8') as f:
    lines = f.readlines()

with open('C:/Users/Aslan/AppData/Local/Temp/all_enum_variants.rs', 'r', encoding='utf-8') as f:
    enum_variants = f.readlines()

with open('C:/Users/Aslan/AppData/Local/Temp/all_match_arms.rs', 'r', encoding='utf-8') as f:
    match_arms = f.readlines()

# Find the catch-all enum section
enum_start = None
enum_end = None
for i, line in enumerate(lines):
    if 'Category catch-alls' in line:
        enum_start = i - 1
    if enum_start is not None and enum_end is None and 'Evex { opcode: Opcode, raw: Instruction },' in line:
        enum_end = i + 1
        break

print("Enum section: lines %d-%d" % (enum_start+1, enum_end+1))

# Find the catch-all match section
match_start = None
match_end = None
for i, line in enumerate(lines):
    if 'Catch-all (temporary' in line:
        match_start = i - 1
    if match_start is not None and match_end is None and line.strip() == '},' and i > match_start + 5:
        match_end = i + 1
        break

print("Match section: lines %d-%d" % (match_start+1, match_end+1))

# Build new file
new_lines = []
new_lines.extend(lines[:enum_start])
new_lines.extend(enum_variants)
new_lines.extend(lines[enum_end:match_start])
new_lines.extend(match_arms)
new_lines.extend(lines[match_end:])

with open('rusty_box_decoder/src/typed.rs', 'w', encoding='utf-8') as f:
    f.writelines(new_lines)

print("Total lines: %d" % len(new_lines))
