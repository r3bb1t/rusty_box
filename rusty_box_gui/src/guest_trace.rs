//! Guest-death tracer for diagnosing services that crash-loop during boot
//! (e.g. Ubuntu's snapd) without any guest-side access.
//!
//! Built into the launcher with `--features guest-trace`; the emulator then
//! runs with instrumentation hooks and appends evidence lines to
//! `guest_trace.log` (override with `RUSTY_BOX_GUEST_TRACE_LOG`):
//!
//! - `EXECVE` — which binaries start (path + argv)
//! - `EXIT` — which address space died, with what status
//! - `WRITE` — stdout/stderr contents (Go panics and daemon errors land here)
//! - `KILL` — signals processes send (fatal ones pinpoint the killer)
//! - `MOUNT` — source/target/fstype (squashfs snap seeding visibility)
//! - `EXC` — CPU exceptions (#UD/#GP/...) with last-executed RIP and opcode
//!
//! Every line is stamped with the retired-instruction count and, for
//! syscalls, the CR3 of the calling address space — a stable per-process
//! tag that lets lines from the same process be correlated.
//!
//! How this discriminates root causes: a squashfs/loop `MOUNT` followed by
//! stderr complaints and clean `EXIT` codes points at disk/IO emulation;
//! an `EXC #UD` or a Go runtime panic on stderr right after `EXECVE`
//! points at a CPU emulation bug.

use rusty_box::cpu::decoder::{Instruction, Opcode};
use rusty_box::cpu::{HookCtx, HookMask, InstrAction, Instrumentation, X86Reg};
use std::fs::File;
use std::io::{BufWriter, Write};

// Linux x86-64 syscall numbers (arch/x86/entry/syscalls/syscall_64.tbl).
const SYS_WRITE: u64 = 1;
const SYS_WRITEV: u64 = 20;
const SYS_EXECVE: u64 = 59;
const SYS_EXIT: u64 = 60;
const SYS_KILL: u64 = 62;
const SYS_MOUNT: u64 = 165;
const SYS_TKILL: u64 = 200;
const SYS_EXIT_GROUP: u64 = 231;
const SYS_TGKILL: u64 = 234;
const SYS_EXECVEAT: u64 = 322;

/// Longest chunk of guest write() payload captured per syscall.
const WRITE_CAPTURE_CAP: usize = 256;
/// Longest guest string (paths, argv) captured.
const STR_CAP: usize = 128;

/// x86 exception vector mnemonics, index = vector.
const VECTOR_NAMES: [&str; 22] = [
    "#DE", "#DB", "NMI", "#BP", "#OF", "#BR", "#UD", "#NM", "#DF", "CSO", "#TS", "#NP", "#SS",
    "#GP", "#PF", "RSV", "#MF", "#AC", "#MC", "#XM", "#VE", "#CP",
];

/// Vectors that are architecturally routine during a Linux boot and would
/// flood the log: #DB, NMI, #BP (kernel static-key patching storms), #NM
/// (lazy FPU switch), #PF (demand paging).
const SKIP_VECTORS: u32 = (1 << 1) | (1 << 2) | (1 << 3) | (1 << 7) | (1 << 14);

/// Flight-recorder capacity (power of two). At ~10 bytes/entry this holds
/// the last 256k user-mode instructions — several scheduler quanta, enough
/// to cover a Go panic's full parse path.
const FLIGHT_CAP: usize = 1 << 18;
/// User/kernel split: record only canonical user-half RIPs.
const USER_RIP_LIMIT: u64 = 1 << 47;

pub struct GuestTracer {
    out: BufWriter<File>,
    /// Retired instruction count — the timestamp for every log line.
    icount: u64,
    /// RIP of the most recently executed instruction (exception context).
    last_rip: u64,
    /// Opcode of the most recently executed instruction (exception context).
    last_opcode: Option<Opcode>,
    /// Per-vector exception counts, for rate limiting.
    exc_counts: [u64; 32],
    /// Per-signal tgkill/tkill counts, for rate limiting (Go floods SIGURG).
    tkill_counts: [u64; 65],
    lines_since_flush: u32,
    /// Set after the first write error so the console warning fires once.
    write_error_reported: bool,
    /// Flight recorder: ring of (rip, opcode) for user-mode instructions,
    /// dumped when a guest process writes "panic: " to stderr.
    flight: Vec<(u64, u16)>,
    flight_pos: usize,
    /// Dumps are large; cap how many panics get one per boot.
    flight_dumps_left: u8,
}

impl GuestTracer {
    /// Trace log destination: `RUSTY_BOX_GUEST_TRACE_LOG`, default
    /// `guest_trace.log` in the working directory.
    pub fn default_log_path() -> String {
        std::env::var("RUSTY_BOX_GUEST_TRACE_LOG").unwrap_or_else(|_| "guest_trace.log".to_owned())
    }

    /// Create the tracer writing to `path`, truncating any previous log.
    pub fn create(path: &str) -> std::io::Result<Self> {
        let mut out = BufWriter::new(File::create(path)?);
        writeln!(
            out,
            "# rusty_box guest trace — lines: [icount] KIND cr3=<addr-space tag> ...\n\
             # EXECVE/EXIT/WRITE(fd<=2)/KILL/MOUNT syscalls + CPU exceptions (#PF/#BP/#NM/#DB/NMI suppressed)"
        )?;
        out.flush()?;
        Ok(Self {
            out,
            icount: 0,
            last_rip: 0,
            last_opcode: None,
            exc_counts: [0; 32],
            tkill_counts: [0; 65],
            lines_since_flush: 0,
            write_error_reported: false,
            flight: vec![(0, 0); FLIGHT_CAP],
            flight_pos: 0,
            flight_dumps_left: 2,
        })
    }

    /// Dump the flight-recorder ring (oldest → newest) after a guest panic.
    /// Format: 8 `rip:opcode` hex pairs per line, then a decoded tail of the
    /// last 64 entries with opcode names for quick reading.
    fn dump_flight(&mut self) {
        if self.flight_dumps_left == 0 {
            return;
        }
        self.flight_dumps_left -= 1;
        let filled = self.flight_pos.min(FLIGHT_CAP);
        let start = if self.flight_pos > FLIGHT_CAP {
            self.flight_pos & (FLIGHT_CAP - 1)
        } else {
            0
        };
        self.emit(
            true,
            format_args!("FLIGHT dump: {filled} user-mode instructions, oldest first"),
        );
        let mut line = String::with_capacity(200);
        for i in 0..filled {
            let (rip, opc) = self.flight[(start + i) & (FLIGHT_CAP - 1)];
            use core::fmt::Write as _;
            let res = write!(line, "{rip:x}:{opc:x} ");
            if let Err(e) = res {
                // String formatting cannot fail; keep the contract explicit.
                self.emit(true, format_args!("FLIGHT format error: {e}"));
                return;
            }
            if i % 8 == 7 || i + 1 == filled {
                self.emit(false, format_args!("F {line}"));
                line.clear();
            }
        }
        let tail = filled.min(64);
        for i in (filled - tail)..filled {
            let (rip, opc) = self.flight[(start + i) & (FLIGHT_CAP - 1)];
            self.emit(false, format_args!("FTAIL {rip:#x} opc={opc:#x}"));
        }
        self.emit(true, format_args!("FLIGHT dump end"));
    }

    /// Append one line; flush immediately for rare/critical lines and
    /// periodically otherwise so a host crash loses little.
    fn emit(&mut self, flush_now: bool, line: core::fmt::Arguments) {
        let res = writeln!(self.out, "[{:>13}] {}", self.icount, line);
        self.lines_since_flush += 1;
        let res = res.and_then(|()| {
            if flush_now || self.lines_since_flush >= 64 {
                self.lines_since_flush = 0;
                self.out.flush()
            } else {
                Ok(())
            }
        });
        if let Err(e) = res {
            if !self.write_error_reported {
                self.write_error_reported = true;
                eprintln!("guest-trace: log write failed, tracing continues without log: {e}");
            }
        }
    }

    /// Read up to `cap` bytes of guest memory. Retries with shrinking sizes
    /// because a read fails wholesale when its tail crosses an unmapped page.
    fn read_guest(ctx: &HookCtx, addr: u64, cap: usize) -> Option<Vec<u8>> {
        if addr == 0 || addr >= 0x8000_0000_0000_0000 {
            return None;
        }
        let cr3 = ctx.cr3();
        let mut size = cap.min(WRITE_CAPTURE_CAP);
        while size > 0 {
            let mut buf = vec![0u8; size];
            if ctx.virt_read_with_cr3(addr, cr3, &mut buf) {
                return Some(buf);
            }
            size /= 4;
        }
        None
    }

    /// Read a NUL-terminated guest string, escaped for logging.
    fn read_cstr(ctx: &HookCtx, addr: u64) -> Option<String> {
        let buf = Self::read_guest(ctx, addr, STR_CAP)?;
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        Some(escape_bytes(&buf[..end]))
    }

    /// Read a guest u64 (e.g. an argv slot or iovec field).
    fn read_u64(ctx: &HookCtx, addr: u64) -> Option<u64> {
        let buf = Self::read_guest(ctx, addr, 8)?;
        if buf.len() < 8 {
            return None;
        }
        Some(u64::from_le_bytes([
            buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7],
        ]))
    }

    fn log_execve(&mut self, ctx: &HookCtx, path_ptr: u64, argv_ptr: u64) {
        let path = Self::read_cstr(ctx, path_ptr).unwrap_or_else(|| "<unreadable>".to_owned());
        let mut argv = String::new();
        for i in 0..4u64 {
            let Some(slot) = Self::read_u64(ctx, argv_ptr.wrapping_add(i * 8)) else {
                break;
            };
            if slot == 0 {
                break;
            }
            let Some(arg) = Self::read_cstr(ctx, slot) else {
                break;
            };
            argv.push(' ');
            argv.push_str(&arg);
        }
        let cr3 = ctx.cr3();
        self.emit(true, format_args!("EXECVE cr3={cr3:#012x} {path}{argv}"));
    }

    /// write(fd, buf, count) with fd 0..=2 — capture the payload. Under
    /// systemd a service's fd 1/2 is the journal socket, so daemon stderr
    /// (Go panics included) still flows through here.
    fn log_write(&mut self, ctx: &HookCtx, fd: u64, buf_ptr: u64, count: u64) {
        let raw = Self::read_guest(ctx, buf_ptr, count as usize);
        let is_panic = fd == 2
            && raw
                .as_deref()
                .is_some_and(|b| b.starts_with(b"panic: ") || b.starts_with(b"fatal error:"));
        let data = raw
            .map(|b| escape_bytes(&b))
            .unwrap_or_else(|| "<unreadable>".to_owned());
        let cr3 = ctx.cr3();
        self.emit(
            false,
            format_args!("WRITE cr3={cr3:#012x} fd={fd} len={count} \"{data}\""),
        );
        // A Go panic is being reported — freeze-frame the instruction trail
        // that led here.
        if is_panic {
            self.dump_flight();
        }
    }

    /// writev(fd, iov, iovcnt) with fd 0..=2 — gather the first iovecs.
    fn log_writev(&mut self, ctx: &HookCtx, fd: u64, iov_ptr: u64, iovcnt: u64) {
        let mut data = String::new();
        let mut total = 0u64;
        let mut captured = 0usize;
        for i in 0..iovcnt.min(4) {
            let base = Self::read_u64(ctx, iov_ptr.wrapping_add(i * 16));
            let len = Self::read_u64(ctx, iov_ptr.wrapping_add(i * 16 + 8));
            let (Some(base), Some(len)) = (base, len) else {
                break;
            };
            total += len;
            let room = WRITE_CAPTURE_CAP.saturating_sub(captured);
            if room == 0 || len == 0 {
                continue;
            }
            if let Some(chunk) = Self::read_guest(ctx, base, (len as usize).min(room)) {
                captured += chunk.len();
                data.push_str(&escape_bytes(&chunk));
            }
        }
        let cr3 = ctx.cr3();
        self.emit(
            false,
            format_args!("WRITE cr3={cr3:#012x} fd={fd} len={total} iov={iovcnt} \"{data}\""),
        );
    }

    fn log_kill(&mut self, ctx: &HookCtx, which: &str, target: u64, tid: u64, sig: u64) {
        // Go's async preemption floods tgkill(SIGURG=23); rate-limit per signal.
        let n = {
            let slot = &mut self.tkill_counts[(sig as usize).min(64)];
            *slot += 1;
            *slot
        };
        if n > 50 && n % 1000 != 0 {
            return;
        }
        let cr3 = ctx.cr3();
        self.emit(
            true,
            format_args!("KILL cr3={cr3:#012x} {which} target={target} tid={tid} sig={sig} n={n}"),
        );
    }

    fn log_mount(&mut self, ctx: &HookCtx, src: u64, target: u64, fstype: u64) {
        let src = Self::read_cstr(ctx, src).unwrap_or_else(|| "<none>".to_owned());
        let target = Self::read_cstr(ctx, target).unwrap_or_else(|| "<none>".to_owned());
        let fstype = Self::read_cstr(ctx, fstype).unwrap_or_else(|| "<none>".to_owned());
        let cr3 = ctx.cr3();
        self.emit(
            true,
            format_args!("MOUNT cr3={cr3:#012x} src={src} target={target} fstype={fstype}"),
        );
    }
}

impl Instrumentation for GuestTracer {
    fn active_hooks(&self) -> HookMask {
        HookMask::EXEC | HookMask::EXCEPTION
    }

    fn before_execution(&mut self, rip: u64, instr: &Instruction) {
        self.icount = self.icount.wrapping_add(1);
        self.last_rip = rip;
        let opcode = instr.get_ia_opcode();
        self.last_opcode = Some(opcode);
        // Flight recorder: user-mode instructions only (kernel noise would
        // drown the trail; the panicking process's last quanta dominate).
        if rip < USER_RIP_LIMIT {
            self.flight[self.flight_pos & (FLIGHT_CAP - 1)] = (rip, opcode as u16);
            self.flight_pos = self.flight_pos.wrapping_add(1);
        }
    }

    fn exception(&mut self, vector: u8, error_code: u32) {
        let v = (vector as usize) & 31;
        if v < 32 && SKIP_VECTORS & (1 << v) != 0 {
            return;
        }
        self.exc_counts[v] += 1;
        let n = self.exc_counts[v];
        // #GP bursts (kernel rdmsr_safe probing) and #UD via WARN() are
        // normal in small numbers; log the first 50 then sample.
        if n > 50 && n % 500 != 0 {
            return;
        }
        let name = VECTOR_NAMES.get(v).copied().unwrap_or("#??");
        let rip = self.last_rip;
        let opcode = self.last_opcode;
        self.emit(
            true,
            format_args!(
                "EXC {name} vector={v} err={error_code:#x} last_rip={rip:#x} last_op={opcode:?} n={n}"
            ),
        );
    }

    fn pre_syscall(&mut self, ctx: &mut HookCtx) -> InstrAction {
        let nr = ctx.reg_read(X86Reg::Rax);
        let a0 = ctx.reg_read(X86Reg::Rdi);
        let a1 = ctx.reg_read(X86Reg::Rsi);
        let a2 = ctx.reg_read(X86Reg::Rdx);
        match nr {
            SYS_WRITE if a0 <= 2 => self.log_write(ctx, a0, a1, a2),
            SYS_WRITEV if a0 <= 2 => self.log_writev(ctx, a0, a1, a2),
            SYS_EXECVE => self.log_execve(ctx, a0, a1),
            SYS_EXECVEAT => self.log_execve(ctx, a1, a2),
            SYS_EXIT | SYS_EXIT_GROUP => {
                let cr3 = ctx.cr3();
                let kind = if nr == SYS_EXIT { "EXIT" } else { "EXIT_GROUP" };
                self.emit(true, format_args!("{kind} cr3={cr3:#012x} code={a0}"));
            }
            SYS_KILL => self.log_kill(ctx, "kill", a0, 0, a1),
            SYS_TKILL => self.log_kill(ctx, "tkill", a0, a0, a1),
            SYS_TGKILL => self.log_kill(ctx, "tgkill", a0, a1, a2),
            SYS_MOUNT => self.log_mount(ctx, a0, a1, a2),
            _ => {}
        }
        InstrAction::Continue
    }
}

impl Drop for GuestTracer {
    fn drop(&mut self) {
        if let Err(e) = self.out.flush() {
            if !self.write_error_reported {
                eprintln!("guest-trace: final log flush failed: {e}");
            }
        }
    }
}

/// Escape guest bytes for a single log line (`\n`, `\t`, `\xNN`, `\\`, `\"`).
fn escape_bytes(bytes: &[u8]) -> String {
    bytes.escape_ascii().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_keeps_text_readable_and_one_line() {
        let escaped = escape_bytes(b"panic: oops\ngoroutine 1 [running]\x00\xff");
        assert_eq!(escaped, "panic: oops\\ngoroutine 1 [running]\\x00\\xff");
        assert!(!escaped.contains('\n'));
    }

    #[test]
    fn tracer_writes_header_and_flushes_on_drop() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("rusty-box-guest-trace-{}.log", std::process::id()));
        let path_str = path.to_str().unwrap();
        {
            let mut tracer = GuestTracer::create(path_str).unwrap();
            tracer.emit(false, format_args!("TEST line"));
        }
        let contents = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert!(contents.starts_with("# rusty_box guest trace"));
        assert!(contents.contains("TEST line"));
    }
}
