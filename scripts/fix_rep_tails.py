#!/usr/bin/env python3
"""
Fix REP/REPE/REPNE loop tail semantics to match Bochs cpu.cc repeat()/repeat_ZF().

Bochs (cpu.cc:395-467 repeat, 470-602 repeat_ZF):
- clear_RF() at the top (already present in dispatchers — commit 1191210)
- Natural exit (ECX==0 or ZF mismatch): `return;` with NO assert_RF, NO STOP_TRACE
- Async break: fall through to tail: assert_RF() + RIP=prev_rip + async_event |= STOP_TRACE

This script transforms the current Rust pattern into the Bochs-exact pattern.
Regexes use explicit spaces to avoid backreference-vs-mixed-indent issues.
"""
import re
from pathlib import Path


BOCHS_COMMENT_SIMPLE = (
    "// Bochs cpu.cc:395-467 repeat(): natural exit returns; async break\n"
    "// falls through to assert_RF + RIP=prev_rip + STOP_TRACE tail.\n"
)
BOCHS_COMMENT_REPE = "// Bochs cpu.cc:470-602 repeat_ZF() rep==3 (F3/REPE): natural exit on !ZF||count==0.\n"
BOCHS_COMMENT_REPNE = "// Bochs cpu.cc:470-602 repeat_ZF() rep==2 (F2/REPNE): natural exit on ZF||count==0.\n"


def indent_block(text: str, n: int) -> str:
    pad = ' ' * n
    return ''.join(pad + line if line else line for line in text.splitlines(keepends=True))


def build_simple_body(op: str, var: str, setfn: str, width: str, trailing_rcx_sync: bool) -> str:
    """Build Bochs-exact loop body for simple REP handler.

    var: 'cx'/'ecx'/'rcx'; setfn: 'set_cx'/'set_ecx'/'set_rcx'."""
    trailing = f"self.set_rcx(self.ecx() as u64);\n" if trailing_rcx_sync else ""
    natural_exit_tail = f"self.set_rcx(self.ecx() as u64);\n        return Ok(());" if trailing_rcx_sync else "return Ok(());"
    return (
        f"{BOCHS_COMMENT_SIMPLE}"
        f"loop {{\n"
        f"    if {var} != 0 {{\n"
        f"        self.on_repeat_iteration(instr);\n"
        f"        self.{op}(instr)?;\n"
        f"        {var} = {var}.wrapping_sub(1);\n"
        f"        self.{setfn}({var});\n"
        f"    }}\n"
        f"    if {var} == 0 {{ {natural_exit_tail} }}\n"
        f"    if self.async_event != 0 {{ break; }}\n"
        f"    self.icount += 1;\n"
        f"}}\n"
        f"self.assert_rf();\n"
        f"self.set_rip(self.prev_rip);\n"
        f"{trailing}"
        f"self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;\n"
        f"Ok(())\n"
    )


def build_zf_body(op: str, var: str, setfn: str, repe: bool, trailing_rcx_sync: bool) -> str:
    cond = "!self.get_zf()" if repe else "self.get_zf()"
    comment = BOCHS_COMMENT_REPE if repe else BOCHS_COMMENT_REPNE
    trailing = "self.set_rcx(self.ecx() as u64);\n" if trailing_rcx_sync else ""
    natural_exit_body = (
        f"self.set_rcx(self.ecx() as u64);\n        return Ok(());"
        if trailing_rcx_sync else "return Ok(());"
    )
    return (
        f"{comment}"
        f"loop {{\n"
        f"    if {var} != 0 {{\n"
        f"        self.on_repeat_iteration(instr);\n"
        f"        self.{op}(instr)?;\n"
        f"        {var} = {var}.wrapping_sub(1);\n"
        f"        self.{setfn}({var});\n"
        f"    }}\n"
        f"    if {cond} || {var} == 0 {{ {natural_exit_body} }}\n"
        f"    if self.async_event != 0 {{ break; }}\n"
        f"    self.icount += 1;\n"
        f"}}\n"
        f"self.assert_rf();\n"
        f"self.set_rip(self.prev_rip);\n"
        f"{trailing}"
        f"self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;\n"
        f"Ok(())\n"
    )


def transform_string_rs(src: str):
    changes = 0

    # ----- Pattern A1: simple REP 16-bit (cx) -----
    # Matches the ENTIRE tail block starting from "while cx != 0 {" through "Ok(())" line.
    patA16 = re.compile(
        r"( +)while cx != 0 \{ self\.on_repeat_iteration\(instr\); self\.(\w+16)\(instr\)\?; cx = cx\.wrapping_sub\(1\);\n"
        r" +self\.set_cx\(cx\);\n"
        r" +if cx != 0 \{\n"
        r" +if self\.async_event != 0 \{\n"
        r" +self\.set_rip\(self\.prev_rip\);\n"
        r" +break;\n"
        r" +\}\n"
        r" +self\.icount \+= 1;\n"
        r" +\} \}\n"
        r" +self\.async_event \|= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;\n"
        r" +Ok\(\(\)\)(?P<tail> \})?\n"
    )
    def replA16(m):
        nonlocal changes; changes += 1
        indent = len(m.group(1))
        body = build_simple_body(m.group(2), 'cx', 'set_cx', '16', False)
        out = indent_block(body, indent)
        if m.group("tail"):
            # Function had inline opening brace with let; preserve the closing brace.
            out = out.rstrip(chr(10)) + " }" + chr(10)
        return out
    src = patA16.sub(replA16, src)

    # ----- Pattern A2: simple REP 32-bit (ecx, trailing set_rcx) -----
    patA32 = re.compile(
        r"( +)while ecx != 0 \{ self\.on_repeat_iteration\(instr\); self\.(\w+32)\(instr\)\?; ecx = ecx\.wrapping_sub\(1\);\n"
        r" +self\.set_ecx\(ecx\);\n"
        r" +if ecx != 0 \{\n"
        r" +if self\.async_event != 0 \{\n"
        r" +self\.set_rip\(self\.prev_rip\);\n"
        r" +break;\n"
        r" +\}\n"
        r" +self\.icount \+= 1;\n"
        r" +\} \}\n"
        r" +self\.set_rcx\(self\.ecx\(\) as u64\);\n"
        r" +self\.async_event \|= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;\n"
        r" +Ok\(\(\)\)(?P<tail> \})?\n"
    )
    def replA32(m):
        nonlocal changes; changes += 1
        indent = len(m.group(1))
        body = build_simple_body(m.group(2), 'ecx', 'set_ecx', '32', True)
        out = indent_block(body, indent)
        if m.group("tail"):
            # Function had inline opening brace with let; preserve the closing brace.
            out = out.rstrip(chr(10)) + " }" + chr(10)
        return out
    src = patA32.sub(replA32, src)

    # ----- Pattern A3: simple REP 64-bit (rcx) -----
    patA64 = re.compile(
        r"( +)while rcx != 0 \{ self\.on_repeat_iteration\(instr\); self\.(\w+64)\(instr\)\?; rcx = rcx\.wrapping_sub\(1\);\n"
        r" +self\.set_rcx\(rcx\);\n"
        r" +if rcx != 0 \{\n"
        r" +if self\.async_event != 0 \{\n"
        r" +self\.set_rip\(self\.prev_rip\);\n"
        r" +break;\n"
        r" +\}\n"
        r" +self\.icount \+= 1;\n"
        r" +\} \}\n"
        r" +self\.async_event \|= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;\n"
        r" +Ok\(\(\)\)(?P<tail> \})?\n"
    )
    def replA64(m):
        nonlocal changes; changes += 1
        indent = len(m.group(1))
        body = build_simple_body(m.group(2), 'rcx', 'set_rcx', '64', False)
        out = indent_block(body, indent)
        if m.group("tail"):
            # Function had inline opening brace with let; preserve the closing brace.
            out = out.rstrip(chr(10)) + " }" + chr(10)
        return out
    src = patA64.sub(replA64, src)

    # ----- Pattern B1: REPE 16-bit (!zf) -----
    patB16repe = re.compile(
        r"( +)while cx != 0 \{ self\.on_repeat_iteration\(instr\); self\.(\w+16)\(instr\)\?; cx = cx\.wrapping_sub\(1\);\n"
        r" +self\.set_cx\(cx\);\n"
        r" +if !self\.get_zf\(\) \|\| cx == 0 \{ break; \}\n"
        r" +if self\.async_event != 0 \{\n"
        r" +self\.set_rip\(self\.prev_rip\);\n"
        r" +break;\n"
        r" +\}\n"
        r" +self\.icount \+= 1; \}\n"
        r" +self\.async_event \|= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;\n"
        r" +Ok\(\(\)\)(?P<tail> \})?\n"
    )
    def replB16repe(m):
        nonlocal changes; changes += 1
        indent = len(m.group(1))
        body = build_zf_body(m.group(2), 'cx', 'set_cx', True, False)
        out = indent_block(body, indent)
        if m.group("tail"):
            # Function had inline opening brace with let; preserve the closing brace.
            out = out.rstrip(chr(10)) + " }" + chr(10)
        return out
    src = patB16repe.sub(replB16repe, src)

    # ----- Pattern B2: REPNE 16-bit -----
    patB16repne = re.compile(
        r"( +)while cx != 0 \{ self\.on_repeat_iteration\(instr\); self\.(\w+16)\(instr\)\?; cx = cx\.wrapping_sub\(1\);\n"
        r" +self\.set_cx\(cx\);\n"
        r" +if self\.get_zf\(\) \|\| cx == 0 \{ break; \}\n"
        r" +if self\.async_event != 0 \{\n"
        r" +self\.set_rip\(self\.prev_rip\);\n"
        r" +break;\n"
        r" +\}\n"
        r" +self\.icount \+= 1; \}\n"
        r" +self\.async_event \|= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;\n"
        r" +Ok\(\(\)\)(?P<tail> \})?\n"
    )
    def replB16repne(m):
        nonlocal changes; changes += 1
        indent = len(m.group(1))
        body = build_zf_body(m.group(2), 'cx', 'set_cx', False, False)
        out = indent_block(body, indent)
        if m.group("tail"):
            # Function had inline opening brace with let; preserve the closing brace.
            out = out.rstrip(chr(10)) + " }" + chr(10)
        return out
    src = patB16repne.sub(replB16repne, src)

    # ----- Pattern B3: REPE 32-bit -----
    patB32repe = re.compile(
        r"( +)while ecx != 0 \{ self\.on_repeat_iteration\(instr\); self\.(\w+32)\(instr\)\?; ecx = ecx\.wrapping_sub\(1\);\n"
        r" +self\.set_ecx\(ecx\);\n"
        r" +if !self\.get_zf\(\) \|\| ecx == 0 \{ break; \}\n"
        r" +if self\.async_event != 0 \{\n"
        r" +self\.set_rip\(self\.prev_rip\);\n"
        r" +break;\n"
        r" +\}\n"
        r" +self\.icount \+= 1; \}\n"
        r" +self\.set_rcx\(self\.ecx\(\) as u64\);\n"
        r" +self\.async_event \|= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;\n"
        r" +Ok\(\(\)\)(?P<tail> \})?\n"
    )
    def replB32repe(m):
        nonlocal changes; changes += 1
        indent = len(m.group(1))
        body = build_zf_body(m.group(2), 'ecx', 'set_ecx', True, True)
        out = indent_block(body, indent)
        if m.group("tail"):
            # Function had inline opening brace with let; preserve the closing brace.
            out = out.rstrip(chr(10)) + " }" + chr(10)
        return out
    src = patB32repe.sub(replB32repe, src)

    # ----- Pattern B4: REPNE 32-bit -----
    patB32repne = re.compile(
        r"( +)while ecx != 0 \{ self\.on_repeat_iteration\(instr\); self\.(\w+32)\(instr\)\?; ecx = ecx\.wrapping_sub\(1\);\n"
        r" +self\.set_ecx\(ecx\);\n"
        r" +if self\.get_zf\(\) \|\| ecx == 0 \{ break; \}\n"
        r" +if self\.async_event != 0 \{\n"
        r" +self\.set_rip\(self\.prev_rip\);\n"
        r" +break;\n"
        r" +\}\n"
        r" +self\.icount \+= 1; \}\n"
        r" +self\.set_rcx\(self\.ecx\(\) as u64\);\n"
        r" +self\.async_event \|= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;\n"
        r" +Ok\(\(\)\)(?P<tail> \})?\n"
    )
    def replB32repne(m):
        nonlocal changes; changes += 1
        indent = len(m.group(1))
        body = build_zf_body(m.group(2), 'ecx', 'set_ecx', False, True)
        out = indent_block(body, indent)
        if m.group("tail"):
            # Function had inline opening brace with let; preserve the closing brace.
            out = out.rstrip(chr(10)) + " }" + chr(10)
        return out
    src = patB32repne.sub(replB32repne, src)

    # ----- Pattern B5: REPE 64-bit -----
    patB64repe = re.compile(
        r"( +)while rcx != 0 \{ self\.on_repeat_iteration\(instr\); self\.(\w+64)\(instr\)\?; rcx = rcx\.wrapping_sub\(1\);\n"
        r" +self\.set_rcx\(rcx\);\n"
        r" +if !self\.get_zf\(\) \|\| rcx == 0 \{ break; \}\n"
        r" +if self\.async_event != 0 \{\n"
        r" +self\.set_rip\(self\.prev_rip\);\n"
        r" +break;\n"
        r" +\}\n"
        r" +self\.icount \+= 1; \}\n"
        r" +self\.async_event \|= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;\n"
        r" +Ok\(\(\)\)(?P<tail> \})?\n"
    )
    def replB64repe(m):
        nonlocal changes; changes += 1
        indent = len(m.group(1))
        body = build_zf_body(m.group(2), 'rcx', 'set_rcx', True, False)
        out = indent_block(body, indent)
        if m.group("tail"):
            # Function had inline opening brace with let; preserve the closing brace.
            out = out.rstrip(chr(10)) + " }" + chr(10)
        return out
    src = patB64repe.sub(replB64repe, src)

    # ----- Pattern B6: REPNE 64-bit -----
    patB64repne = re.compile(
        r"( +)while rcx != 0 \{ self\.on_repeat_iteration\(instr\); self\.(\w+64)\(instr\)\?; rcx = rcx\.wrapping_sub\(1\);\n"
        r" +self\.set_rcx\(rcx\);\n"
        r" +if self\.get_zf\(\) \|\| rcx == 0 \{ break; \}\n"
        r" +if self\.async_event != 0 \{\n"
        r" +self\.set_rip\(self\.prev_rip\);\n"
        r" +break;\n"
        r" +\}\n"
        r" +self\.icount \+= 1; \}\n"
        r" +self\.async_event \|= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;\n"
        r" +Ok\(\(\)\)(?P<tail> \})?\n"
    )
    def replB64repne(m):
        nonlocal changes; changes += 1
        indent = len(m.group(1))
        body = build_zf_body(m.group(2), 'rcx', 'set_rcx', False, False)
        out = indent_block(body, indent)
        if m.group("tail"):
            # Function had inline opening brace with let; preserve the closing brace.
            out = out.rstrip(chr(10)) + " }" + chr(10)
        return out
    src = patB64repne.sub(replB64repne, src)

    # ----- Pattern C1: fast-path ecx async exit — prepend assert_rf() -----
    patC_ecx = re.compile(
        r"( +)if ecx != 0 && self\.async_event != 0 \{\n"
        r" +self\.set_rip\(self\.prev_rip\);\n"
        r" +self\.set_rcx\(self\.ecx\(\) as u64\);\n"
        r" +self\.async_event \|= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;\n"
        r" +return Ok\(\(\)\);\n"
        r"\1\}"
    )
    def replC_ecx(m):
        nonlocal changes; changes += 1
        indent = m.group(1)
        return (
            f"{indent}if ecx != 0 && self.async_event != 0 {{\n"
            f"{indent}    // Bochs tail: assert_RF before stop (cpu.cc:462).\n"
            f"{indent}    self.assert_rf();\n"
            f"{indent}    self.set_rip(self.prev_rip);\n"
            f"{indent}    self.set_rcx(self.ecx() as u64);\n"
            f"{indent}    self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;\n"
            f"{indent}    return Ok(());\n"
            f"{indent}}}"
        )
    src = patC_ecx.sub(replC_ecx, src)

    # ----- Pattern C2: fast-path rcx async exit -----
    patC_rcx = re.compile(
        r"( +)if rcx != 0 && self\.async_event != 0 \{\n"
        r" +self\.set_rip\(self\.prev_rip\);\n"
        r" +self\.async_event \|= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;\n"
        r" +return Ok\(\(\)\);\n"
        r"\1\}"
    )
    def replC_rcx(m):
        nonlocal changes; changes += 1
        indent = m.group(1)
        return (
            f"{indent}if rcx != 0 && self.async_event != 0 {{\n"
            f"{indent}    // Bochs tail: assert_RF before stop (cpu.cc:462).\n"
            f"{indent}    self.assert_rf();\n"
            f"{indent}    self.set_rip(self.prev_rip);\n"
            f"{indent}    self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;\n"
            f"{indent}    return Ok(());\n"
            f"{indent}}}"
        )
    src = patC_rcx.sub(replC_rcx, src)

    return src, changes


def transform_io_rs(src: str):
    """io.rs has three tail shapes:
       (a) Simple 16/32/64 handler (no fast path): while loop with `cx -= 1` no set_cx.
       (b) 32-bit INSW/INSD fast-path + per-word fallback tail.
       (c) Inside a fast-path, async break sets STOP_TRACE — add assert_rf.
    """
    changes = 0

    # ----- Pattern IO-a16: simple 16-bit INS/OUTS -----
    patIOa16 = re.compile(
        r"( +)let mut cx = self\.cx\(\);\n"
        r" +while cx != 0 \{ self\.on_repeat_iteration\(instr\); self\.(\w+16)\(instr\)\?; cx -= 1;\n"
        r" +self\.set_cx\(cx\); \}\n"
        r" +self\.async_event \|= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;\n"
        r" +Ok\(\(\)\)(?P<tail> \})?\n"
    )
    def replIOa16(m):
        nonlocal changes; changes += 1
        indent = len(m.group(1))
        body = (
            f"let mut cx = self.cx();\n"
            f"{BOCHS_COMMENT_SIMPLE}"
            f"loop {{\n"
            f"    if cx != 0 {{\n"
            f"        self.on_repeat_iteration(instr);\n"
            f"        self.{m.group(2)}(instr)?;\n"
            f"        cx = cx.wrapping_sub(1);\n"
            f"        self.set_cx(cx);\n"
            f"    }}\n"
            f"    if cx == 0 {{ return Ok(()); }}\n"
            f"    if self.async_event != 0 {{ break; }}\n"
            f"    self.icount += 1;\n"
            f"}}\n"
            f"self.assert_rf();\n"
            f"self.set_rip(self.prev_rip);\n"
            f"self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;\n"
            f"Ok(())\n"
        )
        out = indent_block(body, indent)
        if m.group("tail"):
            # Function had inline opening brace with let; preserve the closing brace.
            out = out.rstrip(chr(10)) + " }" + chr(10)
        return out
    src = patIOa16.sub(replIOa16, src)

    # ----- Pattern IO-a32 simple: rep_insb32, rep_outsb32, rep_outsw32, rep_outsd32 -----
    patIOa32 = re.compile(
        r"( +)let mut ecx = self\.ecx\(\);\n"
        r" +while ecx != 0 \{ self\.on_repeat_iteration\(instr\); self\.(\w+32)\(instr\)\?; ecx -= 1;\n"
        r" +self\.set_ecx\(ecx\); \}\n"
        r" +self\.async_event \|= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;\n"
        r" +Ok\(\(\)\)(?P<tail> \})?\n"
    )
    def replIOa32(m):
        nonlocal changes; changes += 1
        indent = len(m.group(1))
        body = (
            f"let mut ecx = self.ecx();\n"
            f"{BOCHS_COMMENT_SIMPLE}"
            f"loop {{\n"
            f"    if ecx != 0 {{\n"
            f"        self.on_repeat_iteration(instr);\n"
            f"        self.{m.group(2)}(instr)?;\n"
            f"        ecx = ecx.wrapping_sub(1);\n"
            f"        self.set_ecx(ecx);\n"
            f"    }}\n"
            f"    if ecx == 0 {{\n"
            f"        self.set_rcx(self.ecx() as u64);\n"
            f"        return Ok(());\n"
            f"    }}\n"
            f"    if self.async_event != 0 {{ break; }}\n"
            f"    self.icount += 1;\n"
            f"}}\n"
            f"self.assert_rf();\n"
            f"self.set_rip(self.prev_rip);\n"
            f"self.set_rcx(self.ecx() as u64);\n"
            f"self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;\n"
            f"Ok(())\n"
        )
        out = indent_block(body, indent)
        if m.group("tail"):
            # Function had inline opening brace with let; preserve the closing brace.
            out = out.rstrip(chr(10)) + " }" + chr(10)
        return out
    src = patIOa32.sub(replIOa32, src)

    # ----- Pattern IO-a64 simple -----
    patIOa64 = re.compile(
        r"( +)let mut rcx = self\.rcx\(\);\n"
        r" +while rcx != 0 \{ self\.on_repeat_iteration\(instr\); self\.(\w+64)\(instr\)\?; rcx -= 1;\n"
        r" +self\.set_rcx\(rcx\); \}\n"
        r" +self\.async_event \|= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;\n"
        r" +Ok\(\(\)\)(?P<tail> \})?\n"
    )
    def replIOa64(m):
        nonlocal changes; changes += 1
        indent = len(m.group(1))
        body = (
            f"let mut rcx = self.rcx();\n"
            f"{BOCHS_COMMENT_SIMPLE}"
            f"loop {{\n"
            f"    if rcx != 0 {{\n"
            f"        self.on_repeat_iteration(instr);\n"
            f"        self.{m.group(2)}(instr)?;\n"
            f"        rcx = rcx.wrapping_sub(1);\n"
            f"        self.set_rcx(rcx);\n"
            f"    }}\n"
            f"    if rcx == 0 {{ return Ok(()); }}\n"
            f"    if self.async_event != 0 {{ break; }}\n"
            f"    self.icount += 1;\n"
            f"}}\n"
            f"self.assert_rf();\n"
            f"self.set_rip(self.prev_rip);\n"
            f"self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;\n"
            f"Ok(())\n"
        )
        out = indent_block(body, indent)
        if m.group("tail"):
            # Function had inline opening brace with let; preserve the closing brace.
            out = out.rstrip(chr(10)) + " }" + chr(10)
        return out
    src = patIOa64.sub(replIOa64, src)

    # ----- Pattern IO-fast: per-word fallback inside fast-path INSW/INSD handlers -----
    # Lines like:
    #   self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
    #   return Ok(());
    # within a fast-path async-exit. Add assert_rf before set_rip (which is above).
    # The specific pattern:
    #     if self.async_event != 0 {
    #         ...
    #         self.async_event |= STOP_TRACE;
    #         return Ok(());
    #     }
    # We locate the `STOP_TRACE; \n <indent> return Ok(());` pair inside a block.
    # This pattern is rare; handled separately per site if needed.

    # ----- Pattern IO-B: per-word fallback tail in rep_insw32/insd32 with fast path -----
    # Per-word loops that end in:
    #   while ecx != 0 { self.on_repeat_iteration(instr); self.insw32(instr)?; ecx -= 1;
    #       self.set_ecx(ecx); }
    #       self.async_event |= STOP_TRACE;
    #       Ok(())
    # These exist AFTER a fast path block closes. The tail appears identical
    # to IO-a32 but is inside the same function's fall-through after the fast path.
    # IO-a32 regex above already matches this form.

    # ----- Pattern IO-C: 32-bit per-word fallback in fast path: transferred case -----
    # if self.async_event != 0 {
    #     <compute transferred>
    #     ...
    #     self.async_event |= STOP_TRACE;
    #     return Ok(());
    # }
    # We add assert_rf() right before the STOP_TRACE line (the tail of Bochs repeat()).
    patIOc = re.compile(
        r"( +)self\.async_event \|= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;\n"
        r"( +)return Ok\(\(\)\);"
    )
    # Only apply when the preceding code did not already include assert_rf.
    # We'll use a marker-based check to avoid double-inserting.
    def replIOc(m):
        # Don't touch the simple IO-a rewrites (already final, non-nested)
        # We identify fast-path positions by checking that before the STOP_TRACE is a set_rip or similar.
        return m.group(0)  # default: no change; we'll do fast-path sites manually
    # Skipping auto - handle fast paths manually below.

    return src, changes


def patch_io_fast_paths(src: str):
    """Manually patch fast-path STOP_TRACE sites in io.rs to add assert_rf()."""
    changes = 0
    # The fast-path INSW/INSD sites have:
    #   self.set_rdi(new_edi as u64);
    #   ecx -= transferred;
    #   ...
    #   self.async_event |= STOP_TRACE;
    #   return Ok(());
    # We want to insert `self.assert_rf();\n<indent>self.set_rip(self.prev_rip);` before STOP_TRACE.
    # Bochs matches if async_event occurs mid-fast-path on INSW.
    # However the current fast-path code sets set_rip indirectly? Let me inspect.
    # Actually: the fast paths set ecx/edi AFTER the transfer and then just stop tracing.
    # They don't set RIP=prev_rip, which means after restart the instruction does NOT re-execute.
    # That matches "partial completion" semantics, not the Bochs repeat() retry model.
    # This is a deeper question. For now, we match Bochs only in that we add assert_rf().
    #
    # Safer: don't touch io.rs fast paths automatically — the plan says "every async exit path
    # must do assert_rf() before setting STOP_TRACE". Do this via regex.
    pat = re.compile(
        r"(                                )self\.async_event \|= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;\n"
        r"( +)return Ok\(\(\)\);",
    )
    # Match any depth — use a general one.
    pat = re.compile(
        r"^( +)self\.async_event \|= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;\n"
        r"( +)return Ok\(\(\)\);",
        re.M
    )
    # Unsafe — would match all. Instead, mark sites by unique preceding pattern:
    # Scan for the fast-path blocks and only edit those within them.
    # For now do nothing here; rely on plan verification step and manual edits.
    return src, 0


def main():
    total = 0
    for name, transform in [('rusty_box/src/cpu/string.rs', transform_string_rs),
                             ('rusty_box/src/cpu/io.rs', transform_io_rs)]:
        path = Path(name)
        raw = path.read_bytes()
        use_crlf = b'\r\n' in raw[:4096]
        src = raw.decode('utf-8')
        if use_crlf:
            src = src.replace('\r\n', '\n')
        new_src, n = transform(src)
        if n > 0:
            if use_crlf:
                new_src = new_src.replace('\n', '\r\n')
            path.write_bytes(new_src.encode('utf-8'))
            print(f"{name}: {n} rewrites")
            total += n
        else:
            print(f"{name}: no matches")
    print(f"TOTAL: {total}")


if __name__ == '__main__':
    main()
