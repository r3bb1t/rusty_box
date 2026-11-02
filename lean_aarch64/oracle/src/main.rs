//! rusty_box as the lockstep oracle for the Lean machine.
//!   oracle edk2 dump                  -> dtb.bin, init.txt (start state)
//!   oracle edk2 check K MAX           -> one line per K retired instructions
//!   oracle edk2 detail FROM COUNT     -> full state after each instruction in [FROM, FROM+COUNT)
use rusty_box_aarch64::{MachineBuilder, Reg, RunBudget, VirtConfig};
use std::io::Write;

const FNV_OFF: u64 = 0xcbf29ce484222325;
fn fnv(h: u64, v: u64) -> u64 { let mut h = h; for b in v.to_le_bytes() { h ^= b as u64; h = h.wrapping_mul(0x100000001b3); } h }

macro_rules! state_hash { ($m:expr) => {{
    let mut h = FNV_OFF;
    for i in 0..31 { h = fnv(h, $m.reg_read(Reg::x(i).unwrap())); }
    for r in [Reg::SpEl0, Reg::SpEl1, Reg::Nzcv, Reg::Daif] { h = fnv(h, $m.reg_read(r)); }
    h
}}}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let code = std::fs::read("/usr/share/AAVMF/AAVMF_CODE.no-secboot.fd").unwrap();
    let vars = std::fs::read("/usr/share/AAVMF/AAVMF_VARS.fd").unwrap();
    let mut m = MachineBuilder::new(VirtConfig::default()).firmware(&code).variables(&vars).build().unwrap();
    let mut out = std::io::BufWriter::new(std::io::stdout().lock());
    let mut console: Vec<u8> = Vec::new();
    let mut retired: u64 = 0;
    match a[2].as_str() {
        "dump" => {
            let mut ram = vec![0u8; 1 << 20];
            m.mem_read(0x4000_0000, &mut ram).unwrap();
            let mut dtb_at = None;
            for off in (0..ram.len() - 8).step_by(8) { if ram[off..off + 4] == [0xd0, 0x0d, 0xfe, 0xed] { dtb_at = Some(off); break; } }
            let off = dtb_at.expect("no dtb in the first MiB of RAM");
            let size = u32::from_be_bytes(ram[off + 4..off + 8].try_into().unwrap()) as usize;
            std::fs::write("dtb.bin", &ram[off..off + size]).unwrap();
            writeln!(out, "dtb_at {:#x} size {size}", 0x4000_0000u64 + off as u64).unwrap();
            writeln!(out, "pc {:#x}", m.reg_read(Reg::Pc)).unwrap();
            for i in 0..31 { let v = m.reg_read(Reg::x(i).unwrap()); if v != 0 { writeln!(out, "x{i} {v:#x}").unwrap(); } }
            writeln!(out, "pstate {:?}", m.pstate()).unwrap();
            writeln!(out, "nzcv {:#x} daif {:#x}", m.reg_read(Reg::Nzcv), m.reg_read(Reg::Daif)).unwrap();
            // non-zero RAM outside the DTB
            let mut nz = 0usize; for (i, b) in ram.iter().enumerate() { if *b != 0 && (i < off || i >= off + size) { nz += 1; } }
            writeln!(out, "other non-zero RAM bytes in first MiB: {nz}").unwrap();
        }
        "check" => {
            let k: u64 = a[3].parse().unwrap();
            let max: u64 = a[4].parse().unwrap();
            while retired < max {
                let o = m.step(RunBudget::Instructions(k));
                retired += o.progress.instructions().unwrap_or(0);
                if let Some(mut s) = m.serial(0) { console.extend(s.take_output()); }
                writeln!(out, "C {retired} {} {:x} {:016x} {}", m.ticks(), m.reg_read(Reg::Pc), state_hash!(m), console.len()).unwrap();
                if o.is_terminal() { writeln!(out, "STOP {:?}", o.stop).unwrap(); break; }
                if console.windows(7).any(|w| w == b"Shell> ") { writeln!(out, "SHELL").unwrap(); break; }
            }
            std::fs::write("console.txt", &console).unwrap();
        }
        "detail" => {
            let from: u64 = a[3].parse().unwrap();
            let count: u64 = a[4].parse().unwrap();
            while retired < from {
                let o = m.step(RunBudget::Instructions((from - retired).min(1 << 24)));
                retired += o.progress.instructions().unwrap_or(0);
                if o.is_terminal() { break; }
            }
            for _ in 0..count {
                let pc = m.reg_read(Reg::Pc);
                let mut w = [0u8; 4]; let insn = if m.mem_read(pc, &mut w).is_ok() { u32::from_le_bytes(w) } else { 0 };
                let o = m.step(RunBudget::Instructions(1));
                retired += o.progress.instructions().unwrap_or(0);
                write!(out, "D {retired} t{} pc {pc:x} insn {insn:08x} ->", m.ticks()).unwrap();
                for i in 0..31 { write!(out, " {:x}", m.reg_read(Reg::x(i).unwrap())).unwrap(); }
                writeln!(out, " sp0 {:x} sp1 {:x} nzcv {:x} daif {:x}", m.reg_read(Reg::SpEl0), m.reg_read(Reg::SpEl1), m.reg_read(Reg::Nzcv), m.reg_read(Reg::Daif)).unwrap();
            }
        }
        _ => panic!("mode"),
    }
}
