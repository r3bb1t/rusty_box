//! Diagnostic tripwire for guest null-page write faults.
//!
//! Systematic-debugging instrumentation for the intermittent Ubuntu userspace
//! segfault (`logger[...]: segfault at 0 ... error 6`, kernel `Code:` dump
//! architecturally inconsistent with the reported RIP). A write fault inside
//! the guest's low 64 KiB (`vm.mmap_min_addr` keeps it unmapped on Linux) is
//! never part of a healthy boot, so when one is about to be delivered this
//! module captures everything needed to discriminate the two live hypothesis
//! families:
//!
//! * **stale trace** (icache/SMC invalidation gap): a cached trace on the
//!   faulting RIP's physical page no longer matches a fresh decode of guest
//!   memory — the dump shows exactly which cached instruction diverges;
//! * **genuine execution** (data-side corruption or #PF RIP mis-report): every
//!   cached trace matches memory, so suspicion moves to operand/state bugs.
//!
//! Read-only with respect to guest-visible state; gated on the
//! `RUSTY_BOX_PF_DIAG` environment variable (its value is the report path,
//! appended per hit). Compiled only with the `std` feature.

use crate::cpu::decoder::{decode64, BX_64BIT_REG_RIP};
use crate::cpu::{instrumentation::Instrumentation, BxCpuC, BxCpuIdTrait};
use std::fmt::Write as _;
use std::io::Write as _;

/// Matches icache.rs `BX_ICACHE_INVALID_PHY_ADDRESS` (`BxPhyAddress::MAX`).
const INVALID_PHY: u64 = u64::MAX;

impl<I: BxCpuIdTrait, T: Instrumentation> BxCpuC<'_, I, T> {
    /// Capture a full diagnostic report for an imminent null-page write #PF.
    ///
    /// Called from `page_fault` (paging.rs) before the exception is raised;
    /// `self.cr2` is already set to `laddr` and RIP has not yet been restored
    /// to `prev_rip`, so both the architectural fault RIP (`prev_rip`) and the
    /// in-flight RIP are observable.
    #[cold]
    pub(super) fn null_write_fault_diag(&mut self, laddr: u64, error_code: u32, user: bool) {
        let Some(path) = std::env::var_os("RUSTY_BOX_PF_DIAG") else {
            return;
        };

        let fault_rip = self.prev_rip;
        let in_flight_rip = self.gen_reg[BX_64BIT_REG_RIP].rrx();
        let rip_phys = self.translate_linear_for_diag(fault_rip);

        let mut report = String::new();
        // String's fmt::Write never fails; a helper macro keeps the Result
        // handling honest without drowning the report in match arms.
        macro_rules! out {
            ($($arg:tt)*) => {
                if writeln!(report, $($arg)*).is_err() {
                    // fmt::Write for String is infallible; nothing to react to.
                }
            };
        }

        out!("==== RUSTY_BOX_PF_DIAG null-page write fault ====");
        out!(
            "icount={} user={} error_code={:#x} CR2={:#x} CR3={:#x}",
            self.icount,
            user,
            error_code,
            laddr,
            self.cr3
        );
        out!(
            "fault RIP (prev_rip)={:#x} in-flight RIP={:#x}{}",
            fault_rip,
            in_flight_rip,
            if in_flight_rip != fault_rip {
                "  <RIP advanced mid-instruction>"
            } else {
                ""
            }
        );
        // The 16 architectural GPRs (gen_reg also holds RIP/NIL/temp slots).
        for n in 0..16 {
            if writeln!(
                report,
                "  r{:02}={:#018x}",
                n,
                self.gen_reg[n].rrx()
            )
            .is_err()
            {
                // infallible for String
            }
        }

        // Recent async history — interrupt-timing-dependent corruption shows
        // up as a delivery immediately before the fault icount.
        out!("recent external interrupts (icount, vector, rip), oldest first:");
        for k in 0..32usize {
            let slot = (self.irq_diag_idx + k) % 32;
            let (ic, vec, rip) = self.irq_diag_ring[slot];
            if ic != 0 || vec != 0 || rip != 0 {
                out!("  ic={ic} vec={vec:#04x} rip={rip:#x}");
            }
        }
        out!("recent exceptions (icount, vector, err, prev_rip), oldest first:");
        for k in 0..32usize {
            let slot = (self.exc_diag_idx + k) % 32;
            let (ic, vec, err, rip) = self.exc_diag_ring[slot];
            if ic != 0 || vec != 0 || err != 0 || rip != 0 {
                out!("  ic={ic} vec={vec:#04x} err={err:#06x} prev_rip={rip:#x}");
            }
        }

        let Some(mem_bus) = self.mem_bus else {
            out!("mem_bus UNWIRED — cannot inspect guest memory");
            append_report(&path, &report);
            return;
        };
        // SAFETY: mem_bus is wired for the duration of CPU execution (the same
        // invariant smc_write_check relies on); BxCpuC and BxMemC are distinct
        // objects, so this temporary &mut never aliases self.
        let mem = unsafe { &mut *mem_bus.as_ptr() };

        // SMC bookkeeping at fault time. Unapplied events here would mean this
        // CPU could have looked up stale traces since the write that queued
        // them — the smoking gun for a drain-window bug.
        let seq_next = mem.smc_seq_next();
        out!(
            "SMC: seq_next={} seq_seen={}{}",
            seq_next,
            self.smc_seq_seen,
            if seq_next > self.smc_seq_seen {
                "  <UNAPPLIED SMC EVENTS AT FAULT TIME>"
            } else {
                ""
            }
        );

        let Some(rip_phys) = rip_phys else {
            out!("fault RIP untranslatable — no physical-page inspection possible");
            append_report(&path, &report);
            return;
        };
        let page = rip_phys & !0xfff;
        out!(
            "fault RIP phys={:#x} (page {:#x}); page has SMC stamps: {}",
            rip_phys,
            page,
            mem.smc_range_has_stamps(page, 4096)
        );

        // Current guest memory around the fault RIP, and a fresh decode at it.
        let dump_start = rip_phys.saturating_sub(32).max(page);
        let mut window = [0u8; 64];
        let got = match mem.read_ram(&[], dump_start, &mut window) {
            Ok(n) => n,
            Err(_) => 0,
        };
        let mut hex = String::new();
        for (i, b) in window[..got].iter().enumerate() {
            let addr = dump_start + i as u64;
            if writeln_hex(&mut hex, addr, *b, addr == rip_phys).is_err() {
                // infallible for String
            }
        }
        out!("memory @ [{:#x}..+{}] (>> marks RIP):\n{}", dump_start, got, hex);

        let mut rip_bytes = [0u8; 16];
        let rip_got = match mem.read_ram(&[], rip_phys, &mut rip_bytes) {
            Ok(n) => n,
            Err(_) => 0,
        };
        match decode64::fetch_decode64(&rip_bytes[..rip_got]) {
            Ok(fresh) => out!(
                "fresh decode at fault RIP: {:?} ilen={}",
                fresh.get_ia_opcode(),
                fresh.ilen()
            ),
            Err(e) => out!("fresh decode at fault RIP failed: {e:?}"),
        }

        // Compare every cached trace on the faulting physical page against a
        // fresh decode of current memory. Any mismatch proves the icache holds
        // stale code for this page.
        let mut entries = 0usize;
        let mut compared = 0usize;
        let mut mismatches = 0usize;
        for (idx, entry) in self.i_cache.entry.iter().enumerate() {
            if entry.p_addr == INVALID_PHY || entry.p_addr & !0xfff != page {
                continue;
            }
            entries += 1;
            out!(
                "icache entry #{idx}: p_addr={:#x} tlen={} mpool_start={}",
                entry.p_addr,
                entry.tlen,
                entry.mpool_start_idx
            );
            let mut addr = entry.p_addr;
            for k in 0..entry.tlen as usize {
                let Some(cached) = self.i_cache.mpool.get(entry.mpool_start_idx + k) else {
                    out!("  [{k}] mpool index out of range — walk aborted");
                    break;
                };
                let cached_ilen = cached.ilen();
                if cached_ilen == 0 {
                    // End-of-trace dummy — nothing behind it corresponds to
                    // guest bytes.
                    break;
                }
                let mut bytes = [0u8; 16];
                let n = match mem.read_ram(&[], addr, &mut bytes) {
                    Ok(n) => n,
                    Err(_) => 0,
                };
                match decode64::fetch_decode64(&bytes[..n]) {
                    Ok(fresh) => {
                        compared += 1;
                        let stale = fresh.get_ia_opcode() as u32
                            != cached.get_ia_opcode() as u32
                            || fresh.ilen() != cached_ilen;
                        if stale {
                            mismatches += 1;
                            out!(
                                "  [{k}] @{addr:#x} cached {:?} ilen={} | fresh {:?} ilen={}  <== STALE",
                                cached.get_ia_opcode(),
                                cached_ilen,
                                fresh.get_ia_opcode(),
                                fresh.ilen()
                            );
                        }
                    }
                    Err(e) => {
                        out!(
                            "  [{k}] @{addr:#x} cached {:?} ilen={} | fresh decode failed: {e:?} (page split?) — walk stopped",
                            cached.get_ia_opcode(),
                            cached_ilen
                        );
                        break;
                    }
                }
                addr += u64::from(cached_ilen);
                if addr & !0xfff != page {
                    // Trace continues on the next physical page (page-split);
                    // out of scope for this per-page comparison.
                    break;
                }
            }
        }
        out!(
            "icache page audit: {entries} entries, {compared} instructions compared, {mismatches} MISMATCHES"
        );
        out!("==== end ====\n");

        tracing::error!(
            "PF_DIAG tripwire: null-page write fault RIP={fault_rip:#x} CR2={laddr:#x} \
             ({entries} cached entries on page, {mismatches} stale) — report appended"
        );
        append_report(&path, &report);
    }
}

/// One hex-dump line: `   0x1234: ab` with a `>>` marker on the RIP byte.
fn writeln_hex(
    dst: &mut String,
    addr: u64,
    byte: u8,
    is_rip: bool,
) -> core::fmt::Result {
    writeln!(
        dst,
        "  {}{addr:#x}: {byte:02x}",
        if is_rip { ">>" } else { "  " }
    )
}

fn append_report(path: &std::ffi::OsStr, report: &str) {
    match std::fs::OpenOptions::new().create(true).append(true).open(path) {
        Ok(mut f) => {
            if let Err(e) = f.write_all(report.as_bytes()) {
                tracing::warn!("PF_DIAG: writing the report failed: {e}");
            }
        }
        Err(e) => tracing::warn!("PF_DIAG: opening {path:?} failed: {e}"),
    }
}
