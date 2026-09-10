//! Local CI gate suite.
//!
//! `cargo xtask ci` runs the doctrine ratchets, then a fixed matrix of `cargo`
//! steps, every one but the last a release build: tests for rusty_box_core and
//! rusty_box_devices (their no_std + no_alloc builds included) and bare-metal
//! (`x86_64-unknown-none`) checks of both, devices with alloc; tests for the
//! three WHP crates, the engine's with every target, and a check of
//! rusty_box_whp's examples; tests for the decoder and the rusty_box library;
//! checks of rusty_box as no_std with and without alloc, for
//! `x86_64-unknown-none`, and for `wasm32-unknown-unknown` with alloc; the UEFI
//! application build; the public-API, doc-example and doctrine compile-fail
//! tests; checks of the GUI for wasm and for the host (its tests compiled, not
//! run); an all-features check; and a debug-assertions check. `--full` then
//! adds the whole rusty_box test suite and the GUI release build, and the DLX
//! Linux headless boot gate runs last unless `--skip-boot` is given.
//!
//! It is not a copy of `.github/workflows/ci.yml`. That workflow runs debug
//! builds, and among its steps are several no step here runs: the tests of
//! rusty_box_bximage and rusty_box_gui, the egui serial-path tests, and checks
//! of rusty_box_no_alloc_smoke, of the decoder for `x86_64-unknown-none`, of
//! rusty_box for `x86_64-unknown-uefi`, and of the GUI without its default
//! features.
//!
//! `cargo xtask perf-baseline` builds the release perfbench binary and archives it
//! (with the git revision) under `target/perf-baselines/<rev>/` for interleaved
//! A/B comparison against later phases.

use std::{
    env,
    path::PathBuf,
    process::{Command, Stdio},
    time::Instant,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CiCommand {
    /// Also run the slow steps: full `cargo test -p rusty_box` (integration
    /// tests included) and the release GUI build.
    pub full: bool,
    /// Skip the DLX boot gate (needs the DLX disk image and several minutes).
    pub skip_boot: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PerfBaselineCommand {}

/// The stdout marker the DLX headless boot prints when the guest reaches the
/// login prompt (see `rusty_box/examples/dlxlinux/dlxlinux.rs`).
const DLX_LOGIN_MARKER: &str = "*** LOGIN DETECTED ***";
/// Instruction budget that reliably reaches the DLX login prompt headlessly
/// (LILO Enter injected ~130M, `dlx login:` appears ~364M, margin on top).
const DLX_BOOT_BUDGET: &str = "450000000";

/// Doctrine R1/R6 ratchet baselines (docs/safety-doctrine.md). Counts may only
/// DECREASE; a commit that removes unsafe tightens the matching constant in the
/// same commit. An increase fails ci.
const UNSAFE_TOKEN_BASELINES: &[(&str, usize)] = &[
    // 254 -> 249: the memory stub's swap-file `UnsafeCell` is gone, and with it
    // the four `&self`-to-`&mut File` launderings plus the three test pokes.
    // 249 -> 222: the `CpuTlbPin` sidecar is gone — its `UnsafeCell` state, the
    // publication paths that wrote through it, and the `from_raw_parts`
    // reconstructions that rebuilt the pin slice at every scheduler slice.
    // 222 -> 212: the fetch and async-event entry points hold memory as a
    // borrow, so the raw `*mut BxMemC` re-borrows that fed `prefetch`,
    // `serve_icache_miss` and the icache tests are gone.
    // 212 -> 176: `BxCpuC` no longer stores its bus at all. The `mem_bus`,
    // `io_bus` and `pc_system_ptr` pointers, the accessors that laundered
    // each deref, and the wiring that installed and cleared them are gone;
    // an `ExecCtx` holds the machine as borrows instead.
    // 176 -> 116: `run_cpu_batch` and `inject_interrupt` are safe fns. Their
    // bodies' remaining raw wiring is device-side and named by SAFETY blocks
    // that own it; the ~58 `unsafe { … }` wrappers at their call sites, and
    // two `'a: 'static` bounds justified by a deleted helper, are gone.
    // 116 -> 115: the device manager reaches port and MMIO dispatch as a
    // borrow on `ExecCtx`, so `BxDevicesC`'s stored pointer, its set/clear
    // pair and the accessor that laundered each deref are gone.
    // 115 -> 103: the machine holds no pointer to any of its own parts. A port
    // write carries the memory borrow fw_cfg's DMA descriptor needs, the PIC
    // and DMA controllers come off `ExecCtx` instead of aliasing the device
    // manager the slice already borrows, and the SMC drain reads memory
    // through a context. `Emulator` derives `Send`.
    // 103 -> 97: the machine's CPUs live in one store, boot processor at index
    // 0. `BspCpu`'s pointer newtype and its two `Deref` launderings are gone,
    // as is the no-alloc `[*mut BxCpuC; 253]` and the three dereferences that
    // read it. Every build's machine now derives `Send`.
    ("rusty_box/src", 97),
    ("rusty_box_decoder/src", 0),
    // Zero, and structurally so: the crate carries `#![forbid(unsafe_code)]`,
    // which the workspace lint table backs with a `deny` any new crate inherits.
    ("rusty_box_core/src", 0),
    // Zero, and structurally so, for the same reason as core: the crate carries
    // `#![forbid(unsafe_code)]`. A device model has no business with raw memory
    // — it is handed what it may touch.
    ("rusty_box_devices/src", 0),
    // The host-FFI leaf, and the one crate whose baseline is not zero and is
    // not aiming there: calling the WinHvPlatform C API is its whole purpose.
    // What the ratchet enforces here is CONFINEMENT — every host call lives in
    // `windows.rs`, the only file that lifts the workspace's
    // `deny(unsafe_code)` wholesale, and each block names the invariant it
    // rests on. The remaining tokens are signature markers, and every one of
    // them lifts the `deny` for a single item so the ratchet's own directory
    // total cannot hide unsafe migrating between files. A rise means either a
    // new platform call, a new obligation stated in a signature, or unsafe that
    // escaped the seam.
    //
    // 31 -> 34: reading a segment or a descriptor-table register back out of
    // the platform's value union. The union is how `WHV_REGISTER_VALUE` is
    // defined, so a read of any member is unsafe by construction — `as_word`
    // was already one — and these three are the export half of a state
    // exchange that previously only wrote. Still confined to `windows.rs`.
    //
    // 34 -> 35: `Partition::map_borrowed` is `unsafe fn`, and it is the ONE
    // token outside the platform seam. It performs no unsafe operation of its
    // own — the `unsafe` is the signature, carrying a contract no lifetime can
    // express: the host bytes must outlive the mapping, and a partition stored
    // beside the memory it maps cannot borrow its sibling. Stating it here is
    // what lets the crate that discharges it hold exactly one `unsafe` call.
    //
    // 35 -> 36: `WHvGetVirtualProcessorCounters`, the platform's own per-class
    // intercept and runtime accounting. A new platform call is the one reason
    // this crate's count rises, and this one is confined the same way as the
    // rest: the block is in `windows.rs`, the buffer it hands over is a
    // borrowed `u64` slice whose length is what bounds the write, and the
    // structure is parsed in safe code above the seam.
    //
    // 36 -> 38: `WHvGetVirtualProcessorXsaveState` and its `Set` counterpart —
    // the platform's only window onto the x87 and vector file, which its
    // register names cannot address past the XMM halves. Confined as ever:
    // both blocks in `windows.rs`, each handing over a borrowed byte
    // slice whose length bounds the transfer, with the area parsed in safe
    // code above the seam (R1).
    //
    // 38 -> 37 + 1: the seam is its own crate. The sum is unchanged, and so is
    // every block; what changed is that the 37 platform calls are now confined
    // by a crate boundary as well as by a file, which is what lets the wrapper
    // bind new entry points without either crate's count moving.
    //
    // 37 -> 43: three verbs became `unsafe fn`, and a signature costs a token
    // wherever it is written. The count of unsafe OPERATIONS is unchanged — the
    // same 37 platform calls, still all in `windows.rs`, and `windows.rs` is
    // still 37, because each of the three lost the block it used to wrap (the
    // body of an `unsafe fn` needs none) exactly as it gained the marker. The 6
    // new tokens are 3 signatures in `unsupported.rs`, which performs no unsafe
    // operation at all and carries a targeted `#[expect]` on each, and the 3
    // `unsafe fn` field types in `_IMP_IS_COMPLETE` that pin them for both
    // targets. The obligations are `map_gpa`, which leaves the hypervisor
    // holding a host address past the borrow, and `delete_partition` /
    // `delete_vp`, which release a resource a `Copy` handle cannot stop anyone
    // releasing twice. A `pub` seam cannot name a particular caller, so each
    // states the obligation as the caller's.
    //
    // NINE more for the platform facilities the VMM shape needs, every one a
    // block in `windows.rs` around a single C call whose buffer the line above
    // it sizes: the two partition-property verbs (`WHvGetPartitionProperty` and
    // the byte-buffer `WHvSetPartitionProperty`), the two that stop and start
    // partition time, the two that move a processor's interrupt-controller
    // state page, the banked capability read, and the two places the 128-bit
    // member of the register union is touched — reading it back out of a
    // `RegVal`, and reading the APIC-write context off an exit. `unsupported.rs`
    // and `lib.rs` do not move: the new verbs carry no obligation a caller must
    // discharge, so their signatures are safe `fn`s on both targets.
    //
    // 52 -> 62: the high-resolution waitable timer a device deadline is waited
    // on. Not partition calls — host WAIT objects, and the ones a machine's
    // device deadlines cannot do without: measured on this host, the condition
    // variable behind `Condvar::wait_timeout` returns after the system's
    // ~15.6 ms tick whatever it asks for, and every device deadline on a PC is
    // nearer than that (a PIT at 1 kHz is 1 ms away, the 8042's serial delay
    // 150 µs). A timer created with `CREATE_WAITABLE_TIMER_HIGH_RESOLUTION`
    // answers a 500 µs request in 0.752 ms.
    //
    // Deliberately NOT `timeBeginPeriod`, which was tried here and measured to
    // work: it raises a setting for the whole process, which QEMU may do
    // (`os-win32.c os_setup_early_signal_handling`) because QEMU is the
    // application and this is a library; it floors at 1 ms so the 8042's
    // deadline is inexpressible through it; and Windows 11 revokes the grant
    // for an occluded process. QEMU has no high-resolution waitable timer path
    // at all, so this is ahead of it rather than level with it.
    //
    // Ten tokens: five blocks in `windows.rs` (create the timer, create the
    // event, arm, ring, wait) plus the close, which is TWO handles in one
    // block and an `unsafe fn` marker — `RawDeadline` is `Copy`, so the type
    // cannot stop a caller closing twice and the obligation is stated in the
    // signature. The remaining three are that signature written again in
    // `unsupported.rs` (which performs no unsafe operation and carries a
    // targeted `#[expect]`), the `unsafe fn` field type in `_IMP_IS_COMPLETE`
    // that pins it for both targets, and its `#[expect]`. The obligation is
    // discharged once, by `rusty_box_whp::DeadlineTimer`, which owns the
    // handles and closes them in `Drop` (R1).
    ("rusty_box_whp_sys/src", 62),
    // The safe wrapper over that leaf. FOUR: one signature and three blocks,
    // and the split is the point.
    //
    // The signature is `Partition::map_borrowed`, which performs no unsafe
    // operation of its own — it states a contract no lifetime can express, that
    // the host bytes outlive the mapping, and deleting it would delete the
    // obligation while the hazard remained.
    //
    // The three blocks are where this crate DISCHARGES what the seam asks of a
    // caller, and each is somewhere a signature cannot go. Two are destructors:
    // `Drop::drop` is not an `unsafe fn` and cannot be made one, so
    // `OwnedPartition` and `Partition` state their one-delete guarantee in a
    // block instead. The third is `map_range`, the crate's single door onto the
    // platform's one retaining verb (R5) — `map` and `remap` discharge it by
    // owning the pages, `map_borrowed` forwards it to its own caller.
    //
    // A rise means a fourth place reaching an unsafe operation DIRECTLY, or
    // unsafe that escaped the seam crate for some other reason. It is not a
    // count of who discharges the mapping obligation: `map_range` is a safe
    // `fn`, so a fourth caller of it needs no marker and moves this number not
    // at all — R5's single door, not this baseline, is what holds that
    // obligation in one place.
    //
    // 4 -> 5: a THIRD destructor, and it is the same shape as the other two.
    // `DeadlineTimer` owns the pair of host wait objects a device deadline is
    // waited on, and the seam's `close_deadline` is an `unsafe fn` because
    // `RawDeadline` is `Copy` and nothing in the type system stops a caller
    // closing one twice. `Drop::drop` cannot be an `unsafe fn`, so the
    // guarantee — the pair is taken out of the value before it is closed, and
    // a `&mut self` drop means no waiter can be inside a call on it — is
    // stated in a block. That is exactly the split this baseline exists to
    // describe: the seam states the obligation, this crate discharges it once,
    // and every caller of `DeadlineTimer` needs no marker at all.
    ("rusty_box_whp/src", 5),
    // The adapter between the machine and the leaf. ONE: installing the
    // machine's memory into a partition hands the hypervisor host addresses
    // that outlive the borrow they came from, and no lifetime can say
    // otherwise. The leaf puts that contract in a signature; this crate is
    // where it is discharged, because this is the crate that knows the machine
    // owns both the allocation and the engine. A rise means a second place
    // making that claim, which is exactly what must not happen.
    ("rusty_box_whp_engine/src", 1),
];
/// `unsafe impl … Send/Sync` lines in rusty_box/src. Zero, permanently: thread
/// safety is derived from ownership, and `Emulator`'s `const` assertion in
/// emulator/mod.rs fails the build if a field ever takes that away (R6).
const UNSAFE_IMPL_SEND_BASELINE: usize = 0;

/// Files, per crate, carrying a crate-inner `allow` or `expect` that switches
/// `dead_code` off for the whole file — and, in a `mod.rs`, for every module
/// beneath it. The lint counts when it is named directly or through a group
/// that holds it (`unused`, `warnings`), however many lines the attribute
/// spans. Ratcheted for the same reason as unsafe: not because the items it
/// hides are wrong, but because while it is on, nothing tells you a NEW one
/// appeared. A scanned crate with no entry is held at zero.
///
/// Measured with `RUSTFLAGS="--force-warn dead_code"`, these hide ~2450 items.
/// Most are Bochs-parity constants and helpers ported ahead of their callers —
/// `tables.rs` alone accounts for ~141 — so the backlog needs per-item
/// judgement and cannot be swept. The count going down is what makes that judgement happen file by
/// file; replace a blanket allow with targeted ones that name their provenance.
// 73 -> 70: `memory/mod.rs` (whose inner attribute covered the whole memory
// subtree, `residency.rs` included), `memory/memory_stub.rs`, and `cpu/tlb.rs`.
// Between them they hid a callerless `Tlb::pinned_alloc_offset` left over from
// the deleted pin sidecar, a 4 KiB never-read `apic_scratch` buffer, and a
// forwarder with no callers.
const BLANKET_DEAD_CODE_BASELINES: &[(&str, usize)] = &[
    ("rusty_box/src", 68),
    // `decoder/tables.rs` names `dead_code`; the three opcode maps (`opmap.rs`,
    // `opmap_0f38.rs`, `opmap_0f3a.rs`) switch it off through `unused`.
    ("rusty_box_decoder/src", 4),
];

/// `.unwrap()` / `.expect(…)` outside test code, in each crate
/// `UNSAFE_TOKEN_BASELINES` names: rusty_box, the decoder, core, devices and
/// the three WHP crates.
/// Zero, and it is to stay zero: a library that panics on a condition it could
/// have returned is a library its caller cannot contain.
///
/// The rule is about WHERE, not about the call. Inside `#[cfg(test)]`, a
/// fixture that cannot be built should fail loudly and immediately — so the
/// scan stops at a file's first `#[cfg(test)]` line, skips whole test files,
/// and skips doc comments, whose examples are tests too.
///
/// 52 -> 0. Every one of the 52 was a claim about a value, and none of them
/// paid for itself: the largest group, 37 in `cpu/string.rs`, asserted at run
/// time that a page-bounded count fits a `u32` — which the byte-length check
/// beside it had already established. Four in `iodev/{hpet,ioapic}.rs` said
/// "len checked" about a length nobody had checked, one line below the slice
/// index that would have panicked first. Five in `cpu/svm.rs` stood in for a
/// CPU model's SVM capability, which CPUID already answers.
const PRODUCTION_PANIC_BASELINE: usize = 0;

/// Count occurrences of a bare `unsafe` token per crate, comment lines
/// stripped, against the ratchet baselines.
fn doctrine_ratchets(root: &PathBuf) -> Result<(), String> {
    let started = Instant::now();
    println!("==> doctrine ratchets");

    fn is_ident(b: u8) -> bool {
        b == b'_' || b.is_ascii_alphanumeric()
    }
    /// Occurrences of `needle` as a whole word in `line`, ignoring anything
    /// after `//`. Good enough for a monotone ratchet; not a parser.
    fn word_count(line: &str, needle: &str) -> usize {
        let code = line.split("//").next().unwrap_or("");
        let bytes = code.as_bytes();
        let mut n = 0;
        let mut at = 0;
        while let Some(pos) = code[at..].find(needle) {
            let start = at + pos;
            let end = start + needle.len();
            let left_ok = start == 0 || !is_ident(bytes[start - 1]);
            let right_ok = end >= bytes.len() || !is_ident(bytes[end]);
            if left_ok && right_ok {
                n += 1;
            }
            at = end;
        }
        n
    }
    /// Whether a file carries a crate-inner attribute switching `dead_code` off
    /// for all of it: an `allow` or `expect` naming the lint, or a group that
    /// holds it (`unused`, `warnings`). An inner attribute runs to its closing
    /// bracket, however many lines that takes. A targeted `#[allow(dead_code)]`
    /// on one item is the shape this ratchet is pushing the tree towards, so it
    /// is not counted.
    fn hides_dead_code(text: &str) -> bool {
        let mut lines = text.lines();
        while let Some(line) = lines.next() {
            if !line.trim_start().starts_with("#![") {
                continue;
            }
            let mut attr = String::new();
            let mut depth: i32 = 0;
            let mut current = Some(line);
            while let Some(part) = current {
                let code = part.split("//").next().unwrap_or("");
                depth += code.matches('[').count() as i32 - code.matches(']').count() as i32;
                attr.push_str(code);
                attr.push('\n');
                if depth <= 0 {
                    break;
                }
                current = lines.next();
            }
            let switches_lints_off =
                word_count(&attr, "allow") + word_count(&attr, "expect") > 0;
            let covers_dead_code = ["dead_code", "unused", "warnings"]
                .iter()
                .any(|lint| word_count(&attr, lint) > 0);
            if switches_lints_off && covers_dead_code {
                return true;
            }
        }
        false
    }

    /// Whether a file is entirely test code, by the naming this tree uses for
    /// one: `tests.rs`, or `<subject>_tests.rs`. Such a file carries no
    /// `#[cfg(test)]` of its own — the `mod` that declares it does.
    fn is_test_file(path: &std::path::Path) -> bool {
        path.file_stem()
            .and_then(|stem| stem.to_str())
            .is_some_and(|stem| stem == "tests" || stem.ends_with("_tests"))
    }

    /// Whether `line` is a `#[cfg(test)]` / `#[cfg(all(test, …))]` attribute.
    fn is_cfg_test_attr(line: &str) -> bool {
        let trimmed = line.trim_start();
        trimmed.starts_with("#[cfg(test)") || trimmed.starts_with("#[cfg(all(test")
    }

    /// A line with string literals blanked out, so brace counting is not
    /// thrown off by a `{` inside one. Handles `\"` escapes; a raw string
    /// carrying an unbalanced brace would still fool it, and there are none.
    fn without_strings(code: &str) -> String {
        let mut out = String::with_capacity(code.len());
        let mut in_string = false;
        let mut escaped = false;
        for ch in code.chars() {
            match (in_string, escaped, ch) {
                (false, _, '"') => {
                    in_string = true;
                    out.push(' ');
                }
                (false, _, c) => out.push(c),
                (true, true, _) => {
                    escaped = false;
                    out.push(' ');
                }
                (true, false, '\\') => {
                    escaped = true;
                    out.push(' ');
                }
                (true, false, '"') => {
                    in_string = false;
                    out.push(' ');
                }
                (true, false, _) => out.push(' '),
            }
        }
        out
    }

    /// `.unwrap()` / `.expect(` in the production part of one file.
    ///
    /// Test code is skipped wherever it is, not merely from a file's first
    /// `#[cfg(test)]` onwards. That distinction is the whole difficulty: the
    /// gate sits on an inline `mod tests {` in most files, but on a plain
    /// `mod tests;` DECLARATION in `memory/mod.rs`, and on a bare struct
    /// forty lines further down the same file. Stopping at the first one seen
    /// skips the rest of the file and reports a tree that is clean because it
    /// was not looked at.
    ///
    /// So a `#[cfg(test)]` starts a skipped region only when the item it gates
    /// opens a block, and that region ends when the braces close it. A gated
    /// declaration or statement — anything ending in `;` — skips nothing.
    ///
    /// Doc comments are skipped throughout: `///` examples are compiled and
    /// run as tests, and `unwrap` is how an example says "assume this worked".
    fn production_panics(text: &str) -> usize {
        let mut n = 0;
        let mut depth: i32 = 0;
        // Depth the innermost `#[cfg(test)]` item was opened at, if any.
        let mut test_region: Option<i32> = None;
        // A `#[cfg(test)]` has been seen and its item not yet identified.
        let mut pending_gate = false;

        for line in text.lines() {
            let trimmed = line.trim_start();
            let is_doc_or_comment = trimmed.starts_with("//");
            let code = if is_doc_or_comment {
                String::new()
            } else {
                without_strings(line.split("//").next().unwrap_or(""))
            };
            let opens = code.matches('{').count() as i32;
            let closes = code.matches('}').count() as i32;

            if !is_doc_or_comment && is_cfg_test_attr(line) {
                pending_gate = true;
            } else if pending_gate && !code.trim().is_empty() && !trimmed.starts_with("#[") {
                if opens > closes {
                    // The gated item opens a block: skip until it closes.
                    test_region = test_region.or(Some(depth));
                    pending_gate = false;
                } else if code.trim_end().ends_with(';') {
                    // A declaration or statement — `mod tests;`, a `use`. It
                    // gates one line and nothing follows it into a block.
                    pending_gate = false;
                }
                // Otherwise the item's signature is still open across lines
                // (a multi-line `fn` header); keep looking for its brace.
            }

            if test_region.is_none() && !is_doc_or_comment {
                n += code.matches(".unwrap()").count() + code.matches(".expect(").count();
            }

            depth += opens - closes;
            if let Some(started_at) = test_region {
                if depth <= started_at {
                    test_region = None;
                }
            }
        }
        n
    }

    fn scan_dir(
        dir: &std::path::Path,
        unsafe_tokens: &mut usize,
        unsafe_impl_send: &mut usize,
        blanket_dead_code: &mut usize,
        panics: &mut usize,
    ) -> Result<(), String> {
        let entries = std::fs::read_dir(dir)
            .map_err(|err| format!("doctrine ratchets: read_dir {}: {err}", dir.display()))?;
        for entry in entries {
            let entry =
                entry.map_err(|err| format!("doctrine ratchets: dir entry: {err}"))?;
            let path = entry.path();
            if path.is_dir() {
                scan_dir(&path, unsafe_tokens, unsafe_impl_send, blanket_dead_code, panics)?;
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&path).map_err(|err| {
                    format!("doctrine ratchets: read {}: {err}", path.display())
                })?;
                if !is_test_file(&path) {
                    *panics += production_panics(&text);
                }
                // One per FILE, not per line: a file may carry several inner
                // attributes, and what is being counted is files whose dead
                // code is invisible.
                if hides_dead_code(&text) {
                    *blanket_dead_code += 1;
                }
                for line in text.lines() {
                    let trimmed = line.trim_start();
                    if trimmed.starts_with("//") {
                        continue;
                    }
                    *unsafe_tokens += word_count(line, "unsafe");
                    let code = line.split("//").next().unwrap_or("");
                    if code.contains("unsafe impl")
                        && (code.contains("Send") || code.contains("Sync"))
                    {
                        *unsafe_impl_send += 1;
                    }
                }
            }
        }
        Ok(())
    }

    let mut total_impl_send = 0usize;
    let mut total_panics = 0usize;
    for (rel, baseline) in UNSAFE_TOKEN_BASELINES {
        let mut tokens = 0usize;
        let mut impl_send = 0usize;
        let mut blanket_dead_code = 0usize;
        let mut panics = 0usize;
        scan_dir(
            &root.join(rel),
            &mut tokens,
            &mut impl_send,
            &mut blanket_dead_code,
            &mut panics,
        )?;
        if panics > PRODUCTION_PANIC_BASELINE {
            return Err(format!(
                "doctrine ratchets: {rel} has {panics} `.unwrap()`/`.expect(…)` outside test \
                 code, baseline is {PRODUCTION_PANIC_BASELINE}. A library crate returns its \
                 failures; it does not abort its caller's process. If the value truly cannot \
                 be absent, say so in a type — the 52 removed to reach this baseline were all \
                 claims some nearby code had already proved."
            ));
        }
        total_panics += panics;
        if *rel == "rusty_box/src" {
            total_impl_send = impl_send;
        }
        let dead_code_baseline = BLANKET_DEAD_CODE_BASELINES
            .iter()
            .find(|(dir, _)| dir == rel)
            .map_or(0, |(_, baseline)| *baseline);
        if blanket_dead_code > dead_code_baseline {
            return Err(format!(
                "doctrine ratchets: {rel} has {blanket_dead_code} files whose inner \
                 `allow`/`expect` covers `dead_code`, baseline is {dead_code_baseline} — a new \
                 one hides every unused item in its file (and, from a mod.rs, in every module \
                 below it). Put the allow on the item that needs it and say why."
            ));
        }
        if blanket_dead_code < dead_code_baseline {
            println!(
                "    {rel}: {blanket_dead_code} blanket dead_code allows (< baseline \
                 {dead_code_baseline} — tighten BLANKET_DEAD_CODE_BASELINES in this commit)"
            );
        }
        if tokens > *baseline {
            return Err(format!(
                "doctrine ratchets: {rel} has {tokens} `unsafe` tokens, baseline is {baseline} \
                 (R1: unsafe only ratchets down — remove the new unsafe or justify + re-baseline \
                 in this commit)"
            ));
        }
        if tokens < *baseline {
            println!(
                "    {rel}: {tokens} unsafe tokens (< baseline {baseline} — tighten \
                 UNSAFE_TOKEN_BASELINES in this commit)"
            );
        }
    }
    if total_impl_send > UNSAFE_IMPL_SEND_BASELINE {
        return Err(format!(
            "doctrine ratchets: {total_impl_send} `unsafe impl Send/Sync` lines, baseline is \
             {UNSAFE_IMPL_SEND_BASELINE} (R6: Send derives, never promised)"
        ));
    }
    println!("    production `.unwrap()`/`.expect(…)`: {total_panics}");
    println!(
        "<== doctrine ratchets ok ({:.1}s)",
        started.elapsed().as_secs_f32()
    );
    Ok(())
}

fn repo_root() -> Result<PathBuf, String> {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| "xtask manifest directory has no parent".to_string())
}

struct Step {
    name: &'static str,
    args: &'static [&'static str],
    /// Extra environment variables for the child process.
    envs: &'static [(&'static str, &'static str)],
    /// Substring that must appear on stdout for the step to pass (in addition
    /// to a zero exit status). Steps with a marker run with captured output.
    stdout_marker: Option<&'static str>,
}

const MATRIX: &[Step] = &[
    // The core crate is proven on its own, in every configuration it ships,
    // BEFORE anything that depends on it: a foundation that only compiles as
    // part of its consumer is not a foundation. Its default build is the
    // strictest one (no_std, no alloc), so the additive features are checked
    // for what they add rather than for making it work.
    Step {
        name: "core tests (no_std + no_alloc)",
        args: &["test", "--release", "-p", "rusty_box_core"],
        envs: &[],
        stdout_marker: None,
    },
    Step {
        name: "core tests (alloc)",
        args: &["test", "--release", "-p", "rusty_box_core", "--features", "alloc"],
        envs: &[],
        stdout_marker: None,
    },
    Step {
        name: "core tests (std)",
        args: &["test", "--release", "-p", "rusty_box_core", "--features", "std"],
        envs: &[],
        stdout_marker: None,
    },
    Step {
        name: "core bare-metal target check",
        args: &[
            "check",
            "--release",
            "-p",
            "rusty_box_core",
            "--target",
            "x86_64-unknown-none",
        ],
        envs: &[],
        stdout_marker: None,
    },
    // The device models, under the same rule and for the same reason: their
    // whole claim is that they need no host and no execution engine, and a
    // build that proves it has to happen without one in the room.
    Step {
        name: "devices tests (no_std + no_alloc)",
        args: &["test", "--release", "-p", "rusty_box_devices"],
        envs: &[],
        stdout_marker: None,
    },
    Step {
        name: "devices tests (std)",
        args: &["test", "--release", "-p", "rusty_box_devices", "--features", "std"],
        envs: &[],
        stdout_marker: None,
    },
    Step {
        name: "devices bare-metal target check",
        args: &[
            "check",
            "--release",
            "-p",
            "rusty_box_devices",
            "--features",
            "alloc",
            "--target",
            "x86_64-unknown-none",
        ],
        envs: &[],
        stdout_marker: None,
    },
    // The Windows Hypervisor Platform seam. Its tests need no hypervisor —
    // they cover the bitfield layouts transcribed from the SDK, which is
    // exactly the part a reader cannot check by eye — so this step runs
    // everywhere, including on a host where `hypervisor_present()` is false.
    // A separate step from the wrapper's because it is a separate crate, and
    // a crate no step names is a crate whose tests stop running.
    Step {
        name: "WHP sys tests",
        args: &["test", "--release", "-p", "rusty_box_whp_sys"],
        envs: &[],
        stdout_marker: None,
    },
    // The safe wrapper: the two lifecycle types, the counters and the page
    // arithmetic. Building the probe too, because an example outside the gate
    // is an example that rots.
    Step {
        name: "WHP leaf tests",
        args: &["test", "--release", "-p", "rusty_box_whp"],
        envs: &[],
        stdout_marker: None,
    },
    Step {
        name: "WHP probe builds",
        args: &["check", "--release", "-p", "rusty_box_whp", "--examples"],
        envs: &[],
        stdout_marker: None,
    },
    // The adapter between the machine and the leaf, and the only crate that
    // depends on both. Its state-exchange and time-conversion tests need no
    // hypervisor; the ones that start a machine on hardware skip with a reason
    // when there is none, so this runs everywhere and proves the wiring still
    // compiles even where it cannot be exercised.
    //
    // `--all-targets` because this crate's EXAMPLES are its measurement
    // harnesses, and without it nothing in the suite compiles them: the
    // neighbouring `rusty_box_whp` step checks its own examples, this one did
    // not check these, and deleting the slice census broke five of them where
    // no gate could see it.
    Step {
        name: "WHP engine tests",
        args: &["test", "--release", "-p", "rusty_box_whp_engine", "--all-targets"],
        envs: &[],
        stdout_marker: None,
    },
    Step {
        name: "decoder tests",
        args: &["test", "--release", "-p", "rusty_box_decoder"],
        envs: &[],
        stdout_marker: None,
    },
    Step {
        name: "lib tests (default features)",
        args: &["test", "--release", "-p", "rusty_box", "--lib"],
        envs: &[],
        stdout_marker: None,
    },
    Step {
        name: "no_std + no_alloc check",
        args: &["check", "--release", "-p", "rusty_box", "--no-default-features"],
        envs: &[],
        stdout_marker: None,
    },
    Step {
        name: "no_std + alloc check",
        args: &[
            "check",
            "--release",
            "-p",
            "rusty_box",
            "--no-default-features",
            "--features",
            "alloc",
        ],
        envs: &[],
        stdout_marker: None,
    },
    Step {
        name: "bare-metal target check",
        args: &[
            "check",
            "--release",
            "-p",
            "rusty_box",
            "--no-default-features",
            "--target",
            "x86_64-unknown-none",
        ],
        envs: &[],
        stdout_marker: None,
    },
    Step {
        name: "UEFI example build",
        args: &[
            "build",
            "--release",
            "-p",
            "rusty_box_uefi",
            "--target",
            "x86_64-unknown-uefi",
        ],
        envs: &[],
        stdout_marker: None,
    },
    Step {
        name: "wasm target check (lib, alloc)",
        args: &[
            "check",
            "--release",
            "-p",
            "rusty_box",
            "--no-default-features",
            "--features",
            "alloc",
            "--target",
            "wasm32-unknown-unknown",
        ],
        envs: &[],
        stdout_marker: None,
    },
    Step {
        // The lib tests reach into the crate's own internals; this compiles
        // and runs the public API the way a consumer writes it, which is the
        // only thing that catches a surface that is correct inside and
        // unusable outside.
        name: "public API examples",
        args: &[
            "test",
            "--release",
            "-p",
            "rusty_box",
            "--features",
            "std",
            "--test",
            "api_examples",
        ],
        envs: &[],
        stdout_marker: None,
    },
    Step {
        // Every other `rusty_box` test step is `--lib`, which runs no doc
        // example. A `no_run` example still has to compile, and one that
        // names a path the crate no longer exports fails only here.
        name: "doc examples",
        args: &["test", "--release", "-p", "rusty_box", "--doc"],
        envs: &[],
        stdout_marker: None,
    },
    Step {
        // The doctrine's compile-fail fixtures: each proves a rule by the
        // exact diagnostic that enforces it (R2, R3). The goldens are rustc's
        // rendered output, so they move when the code they quote moves —
        // regenerate with `TRYBUILD=overwrite` once the rule is confirmed to
        // still hold. The registry is `#![cfg(feature = "std")]`, so the
        // feature is named here rather than inherited from the defaults: a
        // default set without it would compile the registry to nothing and
        // pass.
        name: "doctrine compile-fail fixtures",
        args: &[
            "test",
            "--release",
            "-p",
            "rusty_box",
            "--features",
            "std",
            "--test",
            "compile_fail",
        ],
        envs: &[],
        stdout_marker: None,
    },
    Step {
        // The GUI's browser front end is the only consumer of several
        // `#[cfg(target_arch = "wasm32")]` code paths, so no other step in
        // this suite compiles them. One such path went stale unnoticed.
        name: "GUI wasm target check",
        args: &[
            "check",
            "--release",
            "-p",
            "rusty_box_gui",
            "--target",
            "wasm32-unknown-unknown",
        ],
        envs: &[],
        stdout_marker: None,
    },
    Step {
        // The step above compiles the GUI's LIB for wasm, and `--full`'s
        // release build compiles its BINARY for the host. Neither compiles its
        // test targets, so the fixtures in `app.rs` and `runner.rs` that build
        // a `ResolvedConfig` field by field were free to rot — and did, for two
        // fields, across four sites. `--all-targets` is what sees them.
        name: "GUI host check, tests included",
        args: &["check", "--release", "-p", "rusty_box_gui", "--all-targets"],
        envs: &[],
        stdout_marker: None,
    },
    Step {
        name: "all-features check",
        args: &["check", "--release", "-p", "rusty_box", "--all-features"],
        envs: &[],
        stdout_marker: None,
    },
    Step {
        // Every other step passes `--release`, so this is the only one that
        // compiles `#[cfg(debug_assertions)]` code: the debug-only diagnostic
        // counters and the tests that assert on them, all in `rusty_box`.
        // GitHub CI's `cargo test` is a debug build and compiles every such
        // block, so one this suite never compiles is one that fails there.
        // `check` gives the compiler's verdict without building debug
        // binaries; `--all-targets --all-features` reaches every test, example
        // and feature-gated block.
        name: "debug-assertions check (all targets, all features)",
        args: &["check", "-p", "rusty_box", "--all-targets", "--all-features"],
        envs: &[],
        stdout_marker: None,
    },
];

const FULL_MATRIX: &[Step] = &[
    Step {
        name: "full test suite (integration included)",
        args: &["test", "--release", "-p", "rusty_box"],
        envs: &[],
        stdout_marker: None,
    },
    Step {
        name: "GUI release build",
        args: &["build", "-p", "rusty_box_gui", "--release"],
        envs: &[],
        stdout_marker: None,
    },
];

const DLX_BOOT_GATE: Step = Step {
    name: "DLX Linux headless boot gate",
    args: &[
        "run",
        "--release",
        "-p",
        "rusty_box",
        "--example",
        "dlxlinux",
        "--features",
        "std",
    ],
    envs: &[
        ("RUSTY_BOX_HEADLESS", "1"),
        ("MAX_INSTRUCTIONS", DLX_BOOT_BUDGET),
    ],
    stdout_marker: Some(DLX_LOGIN_MARKER),
};

fn run_step(root: &PathBuf, step: &Step) -> Result<(), String> {
    let started = Instant::now();
    println!("==> {}", step.name);

    let mut command = Command::new("cargo");
    command.args(step.args).current_dir(root);
    for (key, value) in step.envs {
        command.env(key, value);
    }

    match step.stdout_marker {
        None => {
            let status = command
                .status()
                .map_err(|err| format!("{}: failed to spawn cargo: {err}", step.name))?;
            if !status.success() {
                return Err(format!("{}: cargo exited with {status}", step.name));
            }
        }
        Some(marker) => {
            command.stdout(Stdio::piped()).stderr(Stdio::inherit());
            let output = command
                .output()
                .map_err(|err| format!("{}: failed to spawn cargo: {err}", step.name))?;
            let stdout = String::from_utf8_lossy(&output.stdout);
            if !output.status.success() {
                let tail: Vec<&str> = stdout.lines().rev().take(20).collect();
                let tail: Vec<&str> = tail.into_iter().rev().collect();
                return Err(format!(
                    "{}: cargo exited with {}\nlast output:\n{}",
                    step.name,
                    output.status,
                    tail.join("\n")
                ));
            }
            if !stdout.contains(marker) {
                let tail: Vec<&str> = stdout.lines().rev().take(20).collect();
                let tail: Vec<&str> = tail.into_iter().rev().collect();
                return Err(format!(
                    "{}: expected marker {marker:?} not found in output\nlast output:\n{}",
                    step.name,
                    tail.join("\n")
                ));
            }
        }
    }

    println!("<== {} ok ({:.1}s)", step.name, started.elapsed().as_secs_f32());
    Ok(())
}

pub fn execute_ci(command: CiCommand) -> Result<(), String> {
    let root = repo_root()?;
    let started = Instant::now();
    let mut ran = 0usize;

    doctrine_ratchets(&root)?;
    ran += 1;

    for step in MATRIX {
        run_step(&root, step)?;
        ran += 1;
    }
    if command.full {
        for step in FULL_MATRIX {
            run_step(&root, step)?;
            ran += 1;
        }
    }
    if !command.skip_boot {
        run_step(&root, &DLX_BOOT_GATE)?;
        ran += 1;
    }

    println!(
        "ci: {ran} steps passed in {:.1}s",
        started.elapsed().as_secs_f32()
    );
    Ok(())
}

pub fn execute_perf_baseline(_command: PerfBaselineCommand) -> Result<(), String> {
    let root = repo_root()?;

    let rev = {
        let output = Command::new("git")
            .args(["rev-parse", "--short=12", "HEAD"])
            .current_dir(&root)
            .output()
            .map_err(|err| format!("failed to run git rev-parse: {err}"))?;
        if !output.status.success() {
            return Err(format!("git rev-parse exited with {}", output.status));
        }
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    };

    let status = Command::new("cargo")
        .args([
            "build", "--release", "-p", "rusty_box", "--example", "perfbench", "--features", "std",
        ])
        .current_dir(&root)
        .status()
        .map_err(|err| format!("failed to spawn cargo: {err}"))?;
    if !status.success() {
        return Err(format!("perfbench build exited with {status}"));
    }

    let exe = if cfg!(windows) {
        "perfbench.exe"
    } else {
        "perfbench"
    };
    let built = root
        .join("target")
        .join("release")
        .join("examples")
        .join(exe);
    if !built.exists() {
        return Err(format!("built perfbench not found at {}", built.display()));
    }

    let dest_dir = root.join("target").join("perf-baselines").join(&rev);
    std::fs::create_dir_all(&dest_dir)
        .map_err(|err| format!("failed to create {}: {err}", dest_dir.display()))?;
    let dest = dest_dir.join(exe);
    std::fs::copy(&built, &dest)
        .map_err(|err| format!("failed to copy perfbench to {}: {err}", dest.display()))?;

    println!("perf baseline archived: {}", dest.display());
    println!("A/B usage: alternate baseline and candidate binaries >= 10 pairs, compare medians.");
    Ok(())
}

pub fn parse_ci_args(args: &[String]) -> Result<CiCommand, String> {
    let mut command = CiCommand::default();
    for arg in args {
        match arg.as_str() {
            "--full" => command.full = true,
            "--skip-boot" => command.skip_boot = true,
            "--help" | "-h" => return Err(ci_usage()),
            other => return Err(format!("unknown ci option {other:?}\n{}", ci_usage())),
        }
    }
    Ok(command)
}

pub fn ci_usage() -> String {
    [
        "usage: cargo xtask ci [--full] [--skip-boot]",
        "  --full       also run the full integration test suite and GUI release build",
        "  --skip-boot  skip the DLX Linux headless boot gate",
    ]
    .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ci_defaults() {
        assert_eq!(parse_ci_args(&[]).unwrap(), CiCommand::default());
    }

    #[test]
    fn parse_ci_flags() {
        let parsed = parse_ci_args(&["--full".into(), "--skip-boot".into()]).unwrap();
        assert_eq!(
            parsed,
            CiCommand {
                full: true,
                skip_boot: true,
            }
        );
    }

    #[test]
    fn parse_ci_rejects_unknown() {
        assert!(parse_ci_args(&["--bogus".into()]).is_err());
    }
}
