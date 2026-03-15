#!/usr/bin/env python3
"""Fix typed.rs by reconstructing from corrupted file."""

with open('rusty_box_decoder/src/typed.rs', 'r', encoding='utf-8') as f:
    lines = f.readlines()

with open('C:/Users/Aslan/AppData/Local/Temp/all_enum_variants.rs', 'r', encoding='utf-8') as f:
    enum_variants = f.readlines()

with open('C:/Users/Aslan/AppData/Local/Temp/all_match_arms.rs', 'r', encoding='utf-8') as f:
    match_arms = f.readlines()

# In the corrupted file:
# Lines 1-2168 (0-based: 0-2167): original enum variants (GOOD)
# Lines 2169-9438: corrupted middle (contains both enum variants and match arms mixed)
# Line 9438 (0-based: 9437): "}" closing the enum
# Lines 9439-9443 (0-based: 9438-9442): blank + separator
# Lines 9444-? (0-based: 9443-): impl Instruction block

# We need:
# Part A: lines 0-2167 (original enum)
# Part B: new enum variants + closing "}\n"
# Part C: from the impl block. But the impl block ALSO has the match arms
#         duplicated inside it because the script added them there too.
#         We need the impl block structure BUT with the match arms replaced.

# Find the impl block
impl_start = None
for i, line in enumerate(lines):
    if i > 9000 and line.strip() == 'impl Instruction {':
        impl_start = i
        break

print("Impl starts at line %d" % (impl_start + 1))

# Find the catch-all in the impl block (if any)
# Actually, in the corrupted file, the impl block has ALL the match arms from
# the original + the ones from the first insertion run.
# Let me find the old match arms boundary.
# Look for the SSE crossover section end, then find what follows
crossover_end = None
for i, line in enumerate(lines):
    if i > impl_start and 'Extension' in line and 'SSE crossover' in line:
        crossover_end = i
        break

if crossover_end is None:
    # Try to find the last manually written match arm
    for i, line in enumerate(lines):
        if i > impl_start and 'MovqVdqEq' in line and 'O::' in line:
            crossover_end = i
            break

print("Crossover section around line %d" % (crossover_end + 1 if crossover_end else 0))

# Find the end of the crossover match arms (last manually written arm before catch-all/new)
# We need to find where the MMX/SSE generated match arms start
mmx_match_start = None
for i, line in enumerate(lines):
    if i > impl_start and 'O::AddpdVpdWpd' in line:
        mmx_match_start = i
        break

if mmx_match_start is None:
    # Fallback: find "MMX / SSE" comment in match
    for i, line in enumerate(lines):
        if i > impl_start and 'MMX / SSE' in line:
            mmx_match_start = i
            break

print("MMX match starts at line %d" % (mmx_match_start + 1 if mmx_match_start else 0))

# The closing of the match/impl block is at the end of the file
# Lines 13978-13980: "        }\n    }\n}\n"
file_end_start = len(lines) - 3  # "        }", "    }", "}"

# Now reconstruct:
# 1. Original enum header (lines 0 to 2167)
# 2. New enum variants
# 3. Closing "}\n\n"
# 4. Separator + impl Instruction + macros + original match arms up to crossover
# 5. New generated match arms
# 6. Closing of match + method + impl

new_lines = []

# Part 1: Original enum (up to line 2167 inclusive)
new_lines.extend(lines[:2168])
print("Part 1: %d lines (original enum header)" % 2168)

# Part 2: New enum variants
new_lines.extend(enum_variants)
print("Part 2: %d lines (new enum variants)" % len(enum_variants))

# Part 3: Close enum
new_lines.append('}\n')
new_lines.append('\n')

# Part 4: From the impl block separator through the macros and original match arms
# Find where the original manually-written match arms end (just before generated ones)
# In the corrupted file, find the last MovqVdqEq match arm, then take up to there + a blank
last_manual_arm = None
for i, line in enumerate(lines):
    if i > impl_start and 'O::MovqVdqEq' in line:
        last_manual_arm = i

# Find the end of the MovqVdqEq block (the next blank line or comment after it)
last_manual_end = last_manual_arm
for i in range(last_manual_arm + 1, len(lines)):
    if lines[i].strip() == '' or lines[i].strip().startswith('//'):
        last_manual_end = i
        break
    last_manual_end = i + 1

print("Last manual arm at line %d, section ends at %d" % (last_manual_arm + 1, last_manual_end + 1))

# Separator through manual match arms (from line 9438 to last_manual_end)
# Line 9437 (0-based) in corrupted file was "}" but we already added it
# We want from the separator/blank after enum to the last manual arm
separator_start = None
for i in range(9437, len(lines)):
    if '// ==========' in lines[i]:
        separator_start = i
        break

print("Separator at line %d" % (separator_start + 1))

# Take from separator to last_manual_end
new_lines.extend(lines[separator_start:last_manual_end + 1])
print("Part 4: %d lines (impl + macros + manual arms)" % (last_manual_end + 1 - separator_start))

# Part 5: New generated match arms
new_lines.append('\n')
new_lines.extend(match_arms)
print("Part 5: %d lines (generated match arms)" % len(match_arms))

# Part 6: Close match + method + impl
new_lines.append('        }\n')
new_lines.append('    }\n')
new_lines.append('}\n')

with open('rusty_box_decoder/src/typed.rs', 'w', encoding='utf-8') as f:
    f.writelines(new_lines)

print("Total lines: %d" % len(new_lines))
