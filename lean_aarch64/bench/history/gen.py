"""Write Arm<Variant>.lean from Arm.lean.tmpl: only the register file differs.
struct: x0..x30 as unboxed UInt64 fields of Cpu (31-arm match per access)
array : one `Array UInt64` (each element boxed; a write allocates)"""
import sys
import sys
src, prefix = sys.argv[1], sys.argv[2]
tmpl = open(src).read()

struct_fields = "\n".join(f"  x{i} : UInt64 := 0" for i in range(31))
struct_get = "@[inline] def Cpu.getX (c : Cpu) (i : UInt8) : UInt64 :=\n  match i with\n" + \
    "\n".join(f"  | {i} => c.x{i}" for i in range(31)) + "\n  | _ => 0\n"
struct_set = "@[inline] def Cpu.setX (c : Cpu) (i : UInt8) (v : UInt64) : Cpu :=\n  match i with\n" + \
    "\n".join(f"  | {i} => {{ c with x{i} := v }}" for i in range(31)) + "\n  | _ => c\n"

array_fields = "  regs : Array UInt64"
array_getset = """@[inline] def Cpu.getX (c : Cpu) (i : UInt8) : UInt64 := c.regs[i.toNat]!
@[inline] def Cpu.setX (c : Cpu) (i : UInt8) (v : UInt64) : Cpu := { c with regs := c.regs.set! i.toNat v }
"""
def fused(field_upd, extra):
    return field_upd

struct_set += "\n@[inline] def Cpu.setXN (c : Cpu) (i : UInt8) (v next : UInt64) : Cpu :=\n  match i with\n" + \
    "\n".join(f"  | {i} => {{ c with x{i} := v, pc := next, icount := c.icount + 1 }}" for i in range(31)) + \
    "\n  | _ => { c with pc := next, icount := c.icount + 1 }\n"
struct_set += "\n@[inline] def Cpu.setXF (c : Cpu) (i : UInt8) (v f next : UInt64) : Cpu :=\n  match i with\n" + \
    "\n".join(f"  | {i} => {{ c with x{i} := v, nzcv := f, pc := next, icount := c.icount + 1 }}" for i in range(31)) + \
    "\n  | _ => { c with nzcv := f, pc := next, icount := c.icount + 1 }\n"
array_getset += """@[inline] def Cpu.setXN (c : Cpu) (i : UInt8) (v next : UInt64) : Cpu :=
  if i == 31 then { c with pc := next, icount := c.icount + 1 }
  else { c with regs := c.regs.set! i.toNat v, pc := next, icount := c.icount + 1 }
@[inline] def Cpu.setXF (c : Cpu) (i : UInt8) (v f next : UInt64) : Cpu :=
  if i == 31 then { c with nzcv := f, pc := next, icount := c.icount + 1 }
  else { c with regs := c.regs.set! i.toNat v, nzcv := f, pc := next, icount := c.icount + 1 }
"""
variants = {
    "Struct": (struct_fields, struct_get + "\n" + struct_set, ""),
    "Array": (array_fields, array_getset, ", regs := Array.replicate 31 0"),
}
for name, (fields, getset, init) in variants.items():
    out = (tmpl.replace("{{REGFIELDS}}", fields).replace("{{GETSET}}", getset)
               .replace("{{INITREGS}}", init).replace("{{VARIANT}}", name.lower()))
    open(f"{prefix}{name}.lean", "w").write(out)
