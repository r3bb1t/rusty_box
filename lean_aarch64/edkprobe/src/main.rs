//! Boot EDK2 to `Shell>` on rusty_box and record what a Lean core must cover.
use std::collections::HashMap;
use disarm64_defn::defn::InsnOpcode;
use rusty_box_aarch64::instrumentation::{ExceptionKind, ExceptionTaken, MemoryAccess};
use rusty_box_aarch64::{Instrumentation, MachineBuilder, RunBudget, VirtConfig};

#[derive(Default)]
struct Probe {
    words: HashMap<u32, u64>,
    exc: HashMap<String, u64>,
    dev: HashMap<u64, u64>, // device accesses per 64 KiB window below RAM
}
impl Instrumentation for Probe {
    fn before_execution(&mut self, _pc: u64, insn: u32) { *self.words.entry(insn).or_default() += 1; }
    fn exception(&mut self, t: ExceptionTaken) {
        let k = match t.kind { ExceptionKind::Synchronous(c) => format!("sync {c:?} {:?}->{:?}", t.from, t.to),
                               k => format!("{k:?} {:?}->{:?}", t.from, t.to) };
        *self.exc.entry(k).or_default() += 1;
    }
    fn memory_access(&mut self, a: MemoryAccess) {
        let p = a.address.get();
        if p < 0x4000_0000 { *self.dev.entry(p & !0xFFFF).or_default() += 1; }
    }
}

fn main() {
    let code = std::fs::read("/usr/share/AAVMF/AAVMF_CODE.no-secboot.fd").unwrap();
    let vars = std::fs::read("/usr/share/AAVMF/AAVMF_VARS.fd").unwrap();
    let mut m = MachineBuilder::new(VirtConfig::default()).instrumentation(Probe::default())
        .firmware(&code).variables(&vars).build().unwrap();
    let mut window: Vec<u8> = Vec::new();
    loop {
        let out = m.step(RunBudget::Ticks(4_000_000));
        if let Some(mut s) = m.serial(0) { window.extend(s.take_output()); }
        if window.windows(7).any(|w| w == b"Shell> ") || out.is_terminal() || m.ticks() > 20_000_000_000 { break; }
    }
    let ticks = m.ticks();
    let p = m.instrumentation();
    let total: u64 = p.words.values().sum();
    // classify every distinct word
    let mut by_mn: HashMap<String, u64> = HashMap::new();
    let (mut simd, mut fpldst, mut undecoded) = (0u64, 0u64, 0u64);
    let mut sysregs: HashMap<u32, u64> = HashMap::new();
    for (&w, &c) in &p.words {
        let op0 = (w >> 25) & 0xF;
        if op0 & 0b0111 == 0b0111 { simd += c; }
        if op0 & 0b0101 == 0b0100 && (w >> 26) & 1 == 1 { fpldst += c; }
        if w & 0xFFD0_0000 == 0xD510_0000 { *sysregs.entry((w >> 5) & 0x7FFF).or_default() += c; }
        match disarm64::decoder::decode(w) {
            Some(o) => *by_mn.entry(o.definition().mnemonic.to_string()).or_default() += c,
            None => undecoded += c,
        }
    }
    let mut v: Vec<_> = by_mn.into_iter().collect(); v.sort_by(|a, b| b.1.cmp(&a.1));
    println!("ticks {ticks}  executed {total}  distinct words {}  mnemonics {}", p.words.len(), v.len());
    println!("SIMD/FP data-processing {simd} ({:.3}%)  FP/SIMD loads/stores {fpldst} ({:.3}%)  undecoded {undecoded}",
             100.0 * simd as f64 / total as f64, 100.0 * fpldst as f64 / total as f64);
    println!("top mnemonics (share of executed):");
    for (mn, c) in v.iter().take(30) { println!("  {mn:10} {:6.2}%", 100.0 * *c as f64 / total as f64); }
    println!("all mnemonics: {}", v.iter().map(|x| x.0.clone()).collect::<Vec<_>>().join(" "));
    println!("distinct system registers via MRS/MSR: {}", sysregs.len());
    let mut e: Vec<_> = p.exc.iter().collect(); e.sort_by(|a, b| b.1.cmp(a.1));
    println!("exceptions: {:?}", e);
    let mut d: Vec<_> = p.dev.iter().collect(); d.sort();
    println!("device windows touched: {:?}", d.iter().map(|(a, c)| format!("{a:#x}:{c}")).collect::<Vec<_>>());
}
