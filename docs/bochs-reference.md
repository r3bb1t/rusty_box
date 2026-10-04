# The Bochs reference

`cpp_orig/bochs` is the Bochs source this port mirrors. It is a checkout of
the `r3bb1t/Bochs` fork whose `master` tracks `bochs-emu/Bochs`; it is not part
of this repository (`/cpp_orig` is ignored), so this file pins it.

| | |
|---|---|
| Commit | `f87c5e226add9a319902fd3fce5a11a38f22b3f9` |
| Upstream date | 2026-10-03 |
| Previous reference | `9cd6d6353b21a1faa736590c1708c70d6d6eb730` (2026-07-28) |
| Sync ledger | `docs/bochs-reference-sync-ledger.md` |

## Build options the port mirrors

The two Bochs builds kept beside the reference (`cpp_orig/bochs/build-mingw`,
`build-bench-nosmp`) are configured with `--enable-cpu-level=6
--enable-x86-64 --enable-vmx=2 --enable-svm --enable-avx --enable-evex
--enable-pci`. The port follows them, with six options ported ahead:

| Option | Reference build | Port |
|---|---|---|
| `BX_SUPPORT_EVEX` | 1 | implemented |
| `BX_SUPPORT_AMX` | 0 | not implemented; AMX and ACE encodings decode as #UD (`scripts/gen_opmap_evex.py` `REFERENCE_BUILD`, `scripts/gen_vex_slots.py`) |
| `BX_SUPPORT_CET` | 0 | ported ahead |
| `BX_SUPPORT_FRED` | 0 | ported ahead |
| `BX_SUPPORT_PKEYS` | 0 | ported ahead |
| `BX_SUPPORT_UINTR` | 0 | ported ahead |
| `BX_SUPPORT_SMP` | 0 | ported ahead (multiple processors) |
| `BX_SUPPORT_GEFORCE` | 0 | ported ahead (divergence D13) |

## Updating the reference

1. `git -C cpp_orig/bochs fetch https://github.com/bochs-emu/Bochs.git master`
   — fetch, never pull: a pull moves the tree before the impact is known.
2. Write the diff and the inventory for `HEAD..FETCH_HEAD` into a plan
   directory under `docs/superpowers/plans/`, and plan the sync. The script
   below is this sync's copy: before running it, set `OLD` and `NEW` to the
   two commits, `ROOT` to the repository root, and review its `NA`,
   `SHIPPED_MODELS`, `LATER` and `AREAS` tables against the port as it then
   stands; run it from the repository root with `PYTHONIOENCODING=utf-8`.
3. `git -C cpp_orig/bochs merge --ff-only <the inventoried commit>`.
4. Point every generator at the reference, regenerate, and adopt the changes
   area by area; record every commit in a ledger.

Numbers that move when the reference moves: the `X86Feature` order (it must
equal `cpu/decoder/features.h`, and `scripts/gen_opcode_isa.py` refuses to run
otherwise), the generated tables, and every snapshot field that stores a
feature bit — so a reference update that adds a feature bumps the snapshot
version.

## Inventory script

```python
"""Read-only: build the A3 hunk inventory for the Bochs reference sync.

For every hunk of `git diff -U0 OLD NEW` in cpp_orig/bochs: the area, the
Bochs function it touches, +/- lines, the commits that touched the file, and
where the port cites that symbol. Prints Markdown to stdout.
"""
import re, subprocess, collections, sys
from pathlib import Path

ROOT = Path(r"C:\Users\olegg\Desktop\rusty_box")
REF = ROOT / "cpp_orig" / "bochs"
OLD, NEW = "9cd6d6353b21a1faa736590c1708c70d6d6eb730", "f87c5e226add9a319902fd3fce5a11a38f22b3f9"

def git(*a):
    return subprocess.run(["git", "-C", str(REF), *a], capture_output=True, text=True,
                          encoding="utf-8", errors="replace").stdout

# ---- what does not apply, and why --------------------------------------
NA = [
    (r"^\.github/", "upstream CI"),
    (r"^bochs/(gui|bx_debug|build|doc|docs-html|misc)/", "host GUI / debugger / packaging / docs"),
    (r"^bochs/(configure|configure\.ac|Makefile\.in|config\.h\.in|osdep\.(cc|h)|plugin\.(cc|h)|main\.cc|logio\.cc|bxthread\.h|win32usbres\.rc|bxdisasm\.cc|README|CHANGES|PARAM_TREE\.txt|\.bochsrc|config\.cc|param_names\.h|cpudb\.h)$", "host build / config / launcher"),
    (r"^bochs/cpu/decoder/disasm\.cc$", "disassembler (the port has none)"),
    (r"^bochs/iodev/(usb|sound|network)/", "device the port does not have"),
    (r"^bochs/iodev/(floppy|gameport|parallel)\.(cc|h)$|^bochs/iodev/display/svga_cirrus\.(cc|h)$|^bochs/iodev/hdimage/vvfat\.(cc|h)$",
     "device the port does not have"),
    (r"^bochs/iodev/extfpuirq\.(cc|h)$", "device the port does not have (pre-existing gap: no IRQ 13 FPU-error routing)"),
    (r"^bochs/memory/Makefile\.in$", "build file"),
    (r"^bochs/cpu/avx/(amx|amx_\w+|ace_\w+|fp8|bf8|hf8)\.(cc|h)$", "AMX/ACE (BX_SUPPORT_AMX 0 in the reference build)"),
    (r"^bochs/cpu/(Makefile\.in|todo)$", "build file / notes"),
    (r"^bochs/cpu/avx/Makefile\.in$|^bochs/cpu/fpu/Makefile\.in$|^bochs/cpu/softfloat3e/Makefile\.in$|^bochs/cpu/cpudb/Makefile\.in$", "build file"),
]
SHIPPED_MODELS = {"bochs/cpu/cpudb/intel/corei7_skylake-x.cc", "bochs/cpu/cpudb/amd/ryzen.cc"}
LATER = {"bochs/cpu/cpudb/intel/arrow_lake.cc": "step D (Arrow Lake model)",
         "bochs/cpu/uintr.cc": "step B (UINTR) for the functions the port lacks"}

def na_reason(path):
    for pat, why in NA:
        if re.search(pat, path):
            return why
    if path.startswith("bochs/cpu/cpudb/") and path not in SHIPPED_MODELS and not path.endswith(("Makefile.in",)) and path not in LATER:
        return "CPU model the port does not ship"
    return None

AREAS = [
    ("decoder", r"^bochs/cpu/decoder/"),
    ("cpu-models", r"^bochs/cpu/cpudb/|^bochs/cpu/cpuid\.(cc|h)$"),
    ("fpu", r"^bochs/cpu/fpu/|^bochs/cpu/i387\.h$"),
    ("softfloat", r"^bochs/cpu/softfloat3e/"),
    ("avx", r"^bochs/cpu/avx/|^bochs/cpu/simd_\w+\.h$|^bochs/cpu/xmm\.h$|^bochs/cpu/sse_\w+\.cc$|^bochs/cpu/cpu_templates_pfp\.h$|^bochs/cpu/wide_int\.(cc|h)$"),
    ("icache-trace", r"^bochs/cpu/icache\.(cc|h)$|^bochs/cpu/idiom\.cc$"),
    ("vmx-svm", r"^bochs/cpu/(vmx|vmcs|vmexit|vapic|svm)\.(cc|h)$"),
    ("cpu-core", r"^bochs/cpu/"),
    ("devices", r"^bochs/iodev/|^bochs/memory/|^bochs/pc_system\.(cc|h)$|^bochs/bochs\.h$"),
    ("bios", r"^bochs/bios/"),
    ("instrument", r"^bochs/instrument/"),
]
def area(path):
    for name, pat in AREAS:
        if re.search(pat, path):
            return name
    return "other"

# ---- commits per file ----------------------------------------------------
commits_of = collections.defaultdict(list)
subject = {}
cur = None
for line in git("log", "--reverse", "--name-only", "--format=== %h %s", f"{OLD}..{NEW}").splitlines():
    if line.startswith("== "):
        sha, _, subj = line[3:].partition(" ")
        cur = sha
        subject[sha] = subj
    elif line.strip() and cur:
        commits_of[line.strip()].append(cur)

# ---- the port's citations of Bochs symbols --------------------------------
rust_files = [p for d in ("rusty_box/src", "rusty_box_decoder/src", "rusty_box_devices/src", "rusty_box_core/src")
              for p in (ROOT / d).rglob("*.rs") if ".tmp." not in p.name]
word_at = collections.defaultdict(list)
for p in rust_files:
    rel = p.relative_to(ROOT).as_posix()
    for n, text in enumerate(p.read_text(encoding="utf-8", errors="replace").splitlines(), 1):
        for w in set(re.findall(r"[A-Za-z_]\w{3,}", text)):
            if len(word_at[w]) < 3 and not any(h.startswith(rel + ":") for h in word_at[w]):
                word_at[w].append(f"{rel}:{n}")
def cites(symbol):
    if not symbol or len(symbol) < 4:
        return []
    return word_at.get(symbol, [])

FUNC = re.compile(r"(?:BX_CPU_C|bx_\w+_c|bx_\w+_t|\w+_t)::(~?\w+)|^\s*(?:static\s+)?(?:BX_CPP_INLINE\s+)?[\w:<>\*\s]+?\b(\w+)\s*\(")
def func_of(text):
    m = FUNC.search(text or "")
    if not m:
        return ""
    return m.group(1) or m.group(2) or ""

# ---- hunks ---------------------------------------------------------------
rows = collections.defaultdict(list)
na_files = collections.defaultdict(list)
diff = git("diff", "-U0", "--no-color", OLD, NEW)
path = None
binary = set()
for line in diff.splitlines():
    if line.startswith("diff --git "):
        path = line.split(" b/", 1)[1]
        continue
    if line.startswith("Binary files"):
        binary.add(path)
        continue
    m = re.match(r"^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@ ?(.*)$", line)
    if m and path:
        rem = int(m.group(2) or 1) if m.group(2) != "0" else 0
        add = int(m.group(4) or 1) if m.group(4) != "0" else 0
        rows[path].append([int(m.group(3)), func_of(m.group(5)), add, rem, []])
        continue
    if path and rows.get(path) and line[:1] in "+-" and not line.startswith(("+++", "---")):
        fn = func_of(line[1:])
        if fn and fn not in rows[path][-1][4] and re.search(r"\(", line):
            rows[path][-1][4].append(fn)

out = []
out.append("# Bochs reference sync — hunk inventory\n")
out.append(f"Upstream `{OLD[:9]}..{NEW[:9]}` (cpp_orig/bochs). Generated by a read-only script; "
           "the line numbers are in the NEW tree. `cites` = where the port names that Bochs symbol.\n")
summary = collections.Counter()
for p in sorted(set(list(rows) + list(binary))):
    why = na_reason(p)
    if why:
        na_files[why].append(p)
        continue
    summary[area(p)] += 1

out.append("## Not applicable (by path)\n")
for why, files in sorted(na_files.items()):
    out.append(f"- **{why}** ({len(files)} files): " + ", ".join(f"`{f}`" for f in files))
out.append("")
for name, _ in AREAS + [("other", "")]:
    files = [p for p in sorted(set(list(rows) + list(binary))) if not na_reason(p) and area(p) == name]
    if not files:
        continue
    out.append(f"## Area: {name} ({len(files)} files)\n")
    for p in files:
        later = LATER.get(p)
        cs = commits_of.get(p, [])
        out.append(f"### `{p}`" + (f" — also feeds {later}" if later else ""))
        out.append(f"Commits ({len(cs)}): " + ", ".join(cs))
        if p in binary:
            out.append("- binary file changed\n")
            continue
        out.append("")
        out.append("| new line | context | defines/changes | +/- | port cites |")
        out.append("|---|---|---|---|---|")
        for ln, ctx, add, rem, fns in rows[p]:
            sym = (fns[0] if fns else ctx)
            c = cites(sym)
            out.append(f"| {ln} | {ctx} | {', '.join(fns[:3])} | +{add}/-{rem} | {'; '.join(c) if c else '—'} |")
        out.append("")
out.append("## Commits\n")
out.append("| commit | subject | files | status |")
out.append("|---|---|---|---|")
files_of = collections.defaultdict(list)
for f, cs in commits_of.items():
    for c in cs:
        files_of[c].append(f)
for sha in subject:
    fs = files_of.get(sha, [])
    if fs and all(na_reason(f) for f in fs):
        status = "N/A: " + "; ".join(sorted({na_reason(f) for f in fs}))
    else:
        status = "adopt: " + ", ".join(sorted({area(f) for f in fs if not na_reason(f)}))
    out.append(f"| {sha} | {subject[sha].replace('|', '/')[:120]} | {len(fs)} | {status} |")
print("\n".join(out))
print(f"\nSUMMARY applicable files per area: {dict(summary)}; N/A files: {sum(len(v) for v in na_files.values())}", file=sys.stderr)
```
