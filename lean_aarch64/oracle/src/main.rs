//! rusty_box as the lockstep oracle for the Lean machine.
//!   oracle edk2 dump                  -> dtb.bin, init.txt (start state)
//!   oracle edk2 check K MAX           -> one line per K retired instructions
//!   oracle edk2 detail FROM COUNT     -> full state after each instruction in [FROM, FROM+COUNT)
//!   oracle linux dump|check|detail ... -> the same for a direct kernel boot of
//!     $LINUX_IMAGE (an arm64 Image) with $LINUX_INITRD, the serial console
//!     command line; `dump` writes ram.bin (all of RAM at the first instruction)
use rusty_box_aarch64::boot::{LinuxImage, SERIAL_CONSOLE_COMMAND_LINE};
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
    if a[1] == "linux" { return linux(&a); }
    if a[1] == "bare" { return bare(&a[2]); }
    if a[1] == "iso" { return iso(&a[2], &a[3..]); }
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

/// Every word executed whose class the Lean machine may lack: SIMD&FP data
/// processing and loads/stores, system words, and the exceptions taken.
#[derive(Default)]
struct Probe {
    words: std::collections::HashMap<u32, u64>,
    exc: std::collections::HashMap<String, u64>,
}
impl rusty_box_aarch64::Instrumentation for Probe {
    fn before_execution(&mut self, _pc: u64, insn: u32) {
        let op0 = (insn >> 25) & 0xF;
        let simd_dp = op0 & 0b0111 == 0b0111;
        let simd_ldst = op0 & 0b0101 == 0b0100 && (insn >> 26) & 1 == 1;
        let system = insn & 0xFFC0_0000 == 0xD500_0000 || insn & 0xFF00_0000 == 0xD400_0000;
        if simd_dp || simd_ldst || system { *self.words.entry(insn).or_default() += 1; }
    }
    fn exception(&mut self, t: rusty_box_aarch64::instrumentation::ExceptionTaken) {
        *self.exc.entry(format!("{:?} {:?}->{:?}", t.kind, t.from, t.to)).or_default() += 1;
    }
}

fn linux(a: &[String]) {
    if a[2] == "probe" {
        let image = std::env::var("LINUX_IMAGE").unwrap_or_else(|_| "/tmp/alp/Image".into());
        let initrd = std::env::var("LINUX_INITRD").unwrap_or_else(|_| "/tmp/alp/initramfs-virt".into());
        let kernel = std::fs::read(image).unwrap();
        let initrd = std::fs::read(initrd).unwrap();
        let mut m = MachineBuilder::new(VirtConfig::default())
            .instrumentation(Probe::default())
            .linux(LinuxImage::parse(&kernel).unwrap())
            .initrd(&initrd)
            .command_line(SERIAL_CONSOLE_COMMAND_LINE)
            .build()
            .unwrap();
        let until = a[3].as_bytes().to_vec();
        let mut console: Vec<u8> = Vec::new();
        loop {
            let o = m.step(RunBudget::Instructions(1_000_000));
            if let Some(mut s) = m.serial(0) { console.extend(s.take_output()); }
            if m.instructions() % 100_000_000 < 1_000_000 { eprintln!("{} {} {:?}", m.instructions(), m.ticks(), o.stop); }
            if o.is_terminal() || console.windows(until.len()).any(|w| w == until.as_slice()) { break; }
        }
        let p = m.instrumentation();
        let mut v: Vec<_> = p.words.iter().collect();
        v.sort();
        for (w, c) in v { println!("W {w:08x} {c}"); }
        for (k, c) in &p.exc { println!("E {k} {c}"); }
        return;
    }
    let image = std::env::var("LINUX_IMAGE").unwrap_or_else(|_| "/tmp/alp/Image".into());
    let initrd = std::env::var("LINUX_INITRD").unwrap_or_else(|_| "/tmp/alp/initramfs-virt".into());
    let kernel = std::fs::read(image).unwrap();
    let initrd = std::fs::read(initrd).unwrap();
    let mut m = MachineBuilder::new(VirtConfig::default())
        .linux(LinuxImage::parse(&kernel).unwrap())
        .initrd(&initrd)
        .command_line(SERIAL_CONSOLE_COMMAND_LINE)
        .build()
        .unwrap();
    let mut out = std::io::BufWriter::new(std::io::stdout().lock());
    let mut console: Vec<u8> = Vec::new();
    let mut retired: u64 = 0;
    match a[2].as_str() {
        "dump" => {
            std::fs::write("ram.bin", m.ram()).unwrap();
            std::fs::write("dtb.bin", m.device_tree()).unwrap();
            writeln!(out, "pc {:#x}", m.reg_read(Reg::Pc)).unwrap();
            for i in 0..31 { let v = m.reg_read(Reg::x(i).unwrap()); if v != 0 { writeln!(out, "x{i} {v:#x}").unwrap(); } }
            writeln!(out, "pstate {:?}", m.pstate()).unwrap();
            writeln!(out, "ram {} bytes, device tree {} bytes", m.ram().len(), m.device_tree().len()).unwrap();
        }
        "check" => {
            let k: u64 = a[3].parse().unwrap();
            let max: u64 = a[4].parse().unwrap();
            // The console script: `until:TEXT` waits for TEXT on the console
            // after the previous item, `type:TEXT` sends TEXT; plain TEXT is
            // `until:`; `\r` and `\n` are CR and LF. Acted on after each line.
            let script: Vec<(bool, Vec<u8>)> = a[5..].iter().map(|s| {
                let un = |t: &str| t.replace("\\r", "\r").replace("\\n", "\n").into_bytes();
                if let Some(t) = s.strip_prefix("type:") { (true, un(t)) }
                else if let Some(t) = s.strip_prefix("until:") { (false, un(t)) }
                else { (false, un(s)) }
            }).collect();
            let (mut pos, mut mark) = (0usize, 0usize);
            while retired < max {
                let o = m.step(RunBudget::Instructions(k));
                retired += o.progress.instructions().unwrap_or(0);
                if let Some(mut s) = m.serial(0) { console.extend(s.take_output()); }
                writeln!(out, "C {retired} {} {:x} {:016x} {}", m.ticks(), m.reg_read(Reg::Pc), state_hash!(m), console.len()).unwrap();
                if o.is_terminal() { writeln!(out, "STOP {:?}", o.stop).unwrap(); break; }
                while pos < script.len() {
                    let (is_type, text) = &script[pos];
                    if *is_type {
                        let sent = m.serial(0).map_or(0, |mut s| s.send(text));
                        if sent != text.len() { writeln!(out, "SHORT {sent} of {}", text.len()).unwrap(); }
                    } else if !console[mark..].windows(text.len()).any(|w| w == text.as_slice()) {
                        break;
                    }
                    mark = console.len();
                    pos += 1;
                }
                if !script.is_empty() && pos >= script.len() { writeln!(out, "UNTIL").unwrap(); break; }
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
                let mut w = [0u8; 4]; let insn = if m.virt_read(pc, &mut w).is_ok() { u32::from_le_bytes(w) } else { 0 };
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

/// The differential test: each line of `path` is an instruction word and a
/// start state (`insn sp nzcv x0..x30 v0lo v0hi .. v31lo v31hi`, hex); each
/// runs alone on a bare machine at EL1h with interrupts masked, the MMU off,
/// VBAR_EL1 at RAM base + 2 MiB and SIMD&FP enabled, its word at RAM base +
/// 1 MiB + 4 * line, and 8 KiB of RAM at RAM base + 4 KiB filled with a
/// pattern of the line's number, FPSR and FPCR zero. One line out per test:
/// the state after.
fn bare(path: &str) {
    use rusty_box_aarch64::{Pstate, SysReg, VReg};
    const RAM: u64 = 0x4000_0000;
    let mut m = MachineBuilder::new(VirtConfig::default()).bare().build().unwrap();
    let text = std::fs::read_to_string(path).unwrap();
    let mut out = std::io::BufWriter::new(std::io::stdout().lock());
    for (idx, line) in text.lines().enumerate() {
        let f: Vec<u64> = line.split_whitespace().map(|x| u64::from_str_radix(x, 16).unwrap()).collect();
        let pc = RAM + 0x10_0000 + 4 * idx as u64;
        m.mem_write(pc, &(f[0] as u32).to_le_bytes()).unwrap();
        let mut pat = vec![0u8; 0x2000];
        let mut h = 0x9E37_79B9_7F4A_7C15u64 ^ idx as u64;
        for b in pat.iter_mut() { h ^= h << 13; h ^= h >> 7; h ^= h << 17; *b = h as u8; }
        m.mem_write(RAM + 0x1000, &pat).unwrap();
        let mut ps = Pstate::RESET;
        ps.n = f[2] & 8 != 0; ps.z = f[2] & 4 != 0; ps.c = f[2] & 2 != 0; ps.v = f[2] & 1 != 0;
        m.set_pstate(ps).unwrap();
        for i in 0..31 { m.reg_write(Reg::x(i).unwrap(), f[3 + i as usize]); }
        m.reg_write(Reg::SpEl1, f[1]);
        for i in 0..32u8 {
            let lo = f[34 + 2 * i as usize] as u128; let hi = f[35 + 2 * i as usize] as u128;
            m.vreg_write(VReg::new(i).unwrap(), (hi << 64) | lo);
        }
        for (r, v) in [(SysReg::VBAR_EL1, RAM + 0x20_0000), (SysReg::CPACR_EL1, 0x30_0000),
                       (SysReg::ESR_EL1, 0), (SysReg::FAR_EL1, 0), (SysReg::ELR_EL1, 0), (SysReg::SPSR_EL1, 0),
                       (SysReg::FPSR, 0), (SysReg::FPCR, 0)] {
            m.sysreg_write(r, v).unwrap();
        }
        m.reg_write(Reg::Pc, pc);
        let o = m.step(RunBudget::Instructions(1));
        let mut mem = vec![0u8; 0x2000];
        m.mem_read(RAM + 0x1000, &mut mem).unwrap();
        let mut mh = FNV_OFF; for b in &mem { mh ^= *b as u64; mh = mh.wrapping_mul(0x100000001b3); }
        write!(out, "T {idx} {:?} pc {:x} nzcv {:x}", o.stop, m.reg_read(Reg::Pc), m.reg_read(Reg::Nzcv) >> 28).unwrap();
        for i in 0..31 { write!(out, " {:x}", m.reg_read(Reg::x(i).unwrap())).unwrap(); }
        write!(out, " sp {:x}", m.reg_read(Reg::SpEl1)).unwrap();
        for i in 0..32u8 { let v = m.vreg_read(VReg::new(i).unwrap()); write!(out, " {:x}:{:x}", v as u64, (v >> 64) as u64).unwrap(); }
        let r = |r| m.sysreg_read(r).unwrap();
        writeln!(out, " esr {:x} far {:x} elr {:x} spsr {:x} fpsr {:x} mem {mh:x}", r(SysReg::ESR_EL1), r(SysReg::FAR_EL1), r(SysReg::ELR_EL1), r(SysReg::SPSR_EL1), r(SysReg::FPSR)).unwrap();
    }
}

/// Boot the firmware with `path` (an ISO image) on a read-only virtio-blk
/// disk, as rusty_box does with a `[virt] disk`, and report the retired
/// instruction and tick counts at which each of `marks` first appears on the
/// console, and the SIMD&FP words executed (all, then those not in the set the
/// Lean machine runs: `oracle linux probe` lists that set).
fn iso(path: &str, marks: &[String]) {
    let code = std::fs::read("/usr/share/AAVMF/AAVMF_CODE.no-secboot.fd").unwrap();
    let vars = std::fs::read("/usr/share/AAVMF/AAVMF_VARS.fd").unwrap();
    let image = std::fs::read(path).unwrap();
    let mut m = MachineBuilder::new(VirtConfig::default())
        .instrumentation(Probe::default())
        .firmware(&code)
        .variables(&vars)
        .virtio_disk(rusty_box_aarch64::Disk::memory(image).read_only())
        .build()
        .unwrap();
    let mut console: Vec<u8> = Vec::new();
    let mut next = 0usize;
    let t0 = std::time::Instant::now();
    while next < marks.len() {
        let o = m.step(RunBudget::Instructions(10_000_000));
        if let Some(mut s) = m.serial(0) { console.extend(s.take_output()); }
        while next < marks.len() && console.windows(marks[next].len()).any(|w| w == marks[next].as_bytes()) {
            println!("MARK {:?} retired {} ticks {} host {:.1}s", marks[next], m.instructions(), m.ticks(), t0.elapsed().as_secs_f64());
            next += 1;
        }
        if o.is_terminal() { println!("STOP {:?}", o.stop); break; }
        if m.instructions() > 20_000_000_000 { println!("GAVE UP at {}", m.instructions()); break; }
    }
    std::fs::write("console.txt", &console).unwrap();
    let p = m.instrumentation();
    let mut v: Vec<_> = p.words.iter().collect();
    v.sort();
    for (w, c) in v { println!("W {w:08x} {c}"); }
}
