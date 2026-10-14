//! A process's threads, read from its `ETHREAD` list, and the stack each one
//! runs on. The list walk and the stack labelling are pure here; the live
//! reads (offset discovery, `TEB` reads) are in `windows`.

use std::collections::HashSet;

use rusty_box::cpu::LinearRange;

/// The most threads a walk reports before giving up: a corrupt or cyclic list
/// never spins the run thread.
pub(crate) const MAX_THREADS: usize = 1 << 16;

/// One thread of a process. A kernel thread has no user `TEB`, so no stack.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WinThread {
    /// `ETHREAD.Cid.UniqueThread`.
    pub(crate) tid: u64,
    /// Where the thread began: `Win32StartAddress` when it is a user address,
    /// else the kernel `StartAddress`; 0 when neither field was found.
    pub(crate) start: u64,
    /// `KTHREAD.Teb`: the user thread environment block, when it has one.
    pub(crate) teb: Option<u64>,
    /// The user stack from the `TEB`'s `NtTib`: `StackLimit..StackBase`.
    pub(crate) stack: Option<LinearRange>,
    /// `TEB.NtTib.FiberData` holds a fibre's address, not the TEB version
    /// number it shares its slot with: the thread has become a fibre.
    pub(crate) uses_fibers: bool,
    /// Its user registers as the kernel saved them when it last entered the
    /// kernel, when its trap frame reads as its own (`frame_is_users`).
    pub(crate) saved: Option<SavedRegisters>,
}

/// The size of an x64 `KTRAP_FRAME` (Vergilius `_KTRAP_FRAME`, XP x64 SP2
/// and Windows 10 22H2 alike).
pub(crate) const TRAP_FRAME_BYTES: usize = 0x190;

/// A thread's user registers from its `KTRAP_FRAME`: what the kernel saved
/// when the thread entered it. RIP, RSP, RBP, RFLAGS, CS and SS are saved on
/// every entry; the others are filled as the kind of entry (system call,
/// interrupt, exception) requires, so they may not be the thread's own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SavedRegisters {
    pub(crate) rip: u64,
    pub(crate) rsp: u64,
    pub(crate) rbp: u64,
    pub(crate) rflags: u64,
    pub(crate) cs: u16,
    pub(crate) ss: u16,
    pub(crate) rax: u64,
    pub(crate) rcx: u64,
    pub(crate) rdx: u64,
    pub(crate) r8: u64,
    pub(crate) r9: u64,
    pub(crate) r10: u64,
    pub(crate) r11: u64,
    pub(crate) rbx: u64,
    pub(crate) rdi: u64,
    pub(crate) rsi: u64,
}

/// Reads the registers out of a `KTRAP_FRAME`'s bytes, at the offsets
/// Vergilius gives `_KTRAP_FRAME` (identical on XP x64 SP2 and Windows 10
/// 22H2): Rax 0x30 … R11 0x60, Rbx 0x140, Rdi 0x148, Rsi 0x150, Rbp 0x158,
/// Rip 0x168, SegCs 0x170 (u16), EFlags 0x178 (u32), Rsp 0x180, SegSs 0x188
/// (u16).
pub(crate) fn decode_trap_frame(frame: &[u8; TRAP_FRAME_BYTES]) -> SavedRegisters {
    let u64_at = |offset: usize| {
        let mut raw = [0u8; 8];
        raw.copy_from_slice(&frame[offset..offset + 8]);
        u64::from_le_bytes(raw)
    };
    let u16_at = |offset: usize| u16::from_le_bytes([frame[offset], frame[offset + 1]]);
    let u32_at = |offset: usize| {
        let mut raw = [0u8; 4];
        raw.copy_from_slice(&frame[offset..offset + 4]);
        u32::from_le_bytes(raw)
    };
    SavedRegisters {
        rip: u64_at(0x168),
        rsp: u64_at(0x180),
        rbp: u64_at(0x158),
        rflags: u64::from(u32_at(0x178)),
        cs: u16_at(0x170),
        ss: u16_at(0x188),
        rax: u64_at(0x30),
        rcx: u64_at(0x38),
        rdx: u64_at(0x40),
        r8: u64_at(0x48),
        r9: u64_at(0x50),
        r10: u64_at(0x58),
        r11: u64_at(0x60),
        rbx: u64_at(0x140),
        rdi: u64_at(0x148),
        rsi: u64_at(0x150),
    }
}

/// Whether `saved` is the user state of the thread whose user stack is
/// `stack`: it came from user mode (CS at privilege 3 — 0x33 for 64-bit code,
/// 0x23 for a WOW64 thread's 32-bit code), it left off at a user address, and
/// its stack pointer lies in that stack. A frame failing any of these is some
/// other frame, or none, and is not shown as the thread's registers.
pub(crate) fn frame_is_users(saved: &SavedRegisters, stack: LinearRange) -> bool {
    const USER_PRIVILEGE: u16 = 3;
    const USER_SPACE_END: u64 = 0x0000_8000_0000_0000;
    saved.cs & 3 == USER_PRIVILEGE
        && saved.rip != 0
        && saved.rip < USER_SPACE_END
        && (stack.start..=stack.end).contains(&saved.rsp)
}

/// Walks the `LIST_ENTRY` ring at `head` by its `Flink` (offset 0), returning
/// each node's address, up to `cap`. A `Flink` chain that loops without
/// passing back through `head` is stopped by the visited set, so a corrupt
/// list ends the walk instead of spinning.
pub(crate) fn walk_thread_list(
    head: u64,
    mut read_flink: impl FnMut(u64) -> Option<u64>,
    cap: usize,
) -> Vec<u64> {
    let mut entries = Vec::new();
    let mut visited = HashSet::new();
    let Some(mut node) = read_flink(head) else {
        return entries;
    };
    while node != head && entries.len() < cap {
        if !visited.insert(node) {
            break;
        }
        entries.push(node);
        let Some(next) = read_flink(node) else {
            break;
        };
        node = next;
    }
    entries
}

/// The thread whose user stack fully contains `start..end`, if any: a private
/// region that sits inside one thread's stack is that thread's stack. A thread
/// with no user stack never owns a region.
pub(crate) fn stack_owner(start: u64, end: u64, threads: &[WinThread]) -> Option<u64> {
    threads.iter().find_map(|thread| {
        thread
            .stack
            .filter(|stack| stack.start <= start && end <= stack.end)
            .map(|_| thread.tid)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thread(tid: u64, stack: Option<LinearRange>) -> WinThread {
        WinThread {
            tid,
            start: 0,
            teb: stack.map(|_| 0x7FF0_0000),
            stack,
            uses_fibers: false,
            saved: None,
        }
    }

    #[test]
    fn a_cyclic_thread_list_stops_at_the_cap() {
        // A ring of nodes 1..=N whose last node points back to the first,
        // never to the head: the visited set, then the cap, must stop it.
        let ring = |count: u64| {
            move |node: u64| -> Option<u64> {
                if node == 0 {
                    Some(1)
                } else {
                    Some(node % count + 1)
                }
            }
        };
        // A short cycle ends at the visited set, well under the cap.
        assert_eq!(walk_thread_list(0, ring(5), MAX_THREADS).len(), 5);
        // A head-terminated list (1 → 2 → head) ends normally.
        let flat = |node: u64| Some(match node { 0 => 1, 1 => 2, _ => 0 });
        assert_eq!(walk_thread_list(0, flat, MAX_THREADS), [1, 2]);
        // A cap below the cycle length is honoured.
        assert_eq!(walk_thread_list(0, ring(100), 8).len(), 8);
        // A self-loop at the first node is one entry, not a spin.
        assert_eq!(walk_thread_list(0, |_| Some(7), MAX_THREADS), [7]);
    }

    #[test]
    fn a_private_region_in_a_thread_stack_is_labelled() {
        let threads = [
            thread(0x1A4, Some(LinearRange { start: 0x5B_0000, end: 0x60_0000 })),
            thread(0x2B8, Some(LinearRange { start: 0x70_0000, end: 0x74_0000 })),
        ];
        // A region wholly inside the first stack is that thread's.
        assert_eq!(stack_owner(0x5C_0000, 0x5E_0000, &threads), Some(0x1A4));
        // A region touching the second stack's exact bounds counts.
        assert_eq!(stack_owner(0x70_0000, 0x74_0000, &threads), Some(0x2B8));
        // A region that spills past a stack's end belongs to neither.
        assert_eq!(stack_owner(0x5F_0000, 0x61_0000, &threads), None);
        // A region in no stack is unowned.
        assert_eq!(stack_owner(0x10_0000, 0x11_0000, &threads), None);
    }

    /// A `KTRAP_FRAME` with `value` at `offset`, `width` bytes wide.
    fn put(frame: &mut [u8; TRAP_FRAME_BYTES], offset: usize, value: u64, width: usize) {
        frame[offset..offset + width].copy_from_slice(&value.to_le_bytes()[..width]);
    }

    /// Each register comes from the offset Vergilius gives `_KTRAP_FRAME` on
    /// both XP x64 SP2 and Windows 10 22H2.
    #[test]
    fn a_trap_frame_decodes_at_the_offsets_windows_uses() {
        let mut frame = [0u8; TRAP_FRAME_BYTES];
        for (offset, value) in [
            (0x30, 0xA0), (0x38, 0xC0), (0x40, 0xD0), (0x48, 0x08), (0x50, 0x09),
            (0x58, 0x10), (0x60, 0x11), (0x140, 0xB0), (0x148, 0xD1), (0x150, 0x51),
            (0x158, 0x7FF0_0000_8000), (0x168, 0x7FF7_1234_5678), (0x180, 0x7FF0_0000_7F00),
        ] {
            put(&mut frame, offset, value, 8);
        }
        put(&mut frame, 0x170, 0x33, 2);
        put(&mut frame, 0x178, 0x246, 4);
        put(&mut frame, 0x188, 0x2B, 2);
        let saved = decode_trap_frame(&frame);
        assert_eq!(
            saved,
            SavedRegisters {
                rip: 0x7FF7_1234_5678,
                rsp: 0x7FF0_0000_7F00,
                rbp: 0x7FF0_0000_8000,
                rflags: 0x246,
                cs: 0x33,
                ss: 0x2B,
                rax: 0xA0,
                rcx: 0xC0,
                rdx: 0xD0,
                r8: 0x08,
                r9: 0x09,
                r10: 0x10,
                r11: 0x11,
                rbx: 0xB0,
                rdi: 0xD1,
                rsi: 0x51,
            }
        );
    }

    fn user_frame() -> SavedRegisters {
        let mut saved = decode_trap_frame(&[0u8; TRAP_FRAME_BYTES]);
        saved.cs = 0x33;
        saved.ss = 0x2B;
        saved.rip = 0x7FF7_0000_1000;
        saved.rsp = 0x1F_F000;
        saved
    }

    /// A frame is the thread's own user state only when it came from user
    /// mode (CS at privilege 3), left off at a user address, and its stack
    /// pointer lies in the thread's own stack.
    #[test]
    fn a_frame_is_a_threads_own_only_with_a_user_cs_rip_and_stack() {
        let stack = LinearRange { start: 0x10_0000, end: 0x20_0000 };
        assert!(frame_is_users(&user_frame(), stack));
        // A WOW64 thread entered from 32-bit code (CS 0x23).
        assert!(frame_is_users(&SavedRegisters { cs: 0x23, ..user_frame() }, stack));
        // Entered from the kernel (CS 0x10): not user state.
        assert!(!frame_is_users(&SavedRegisters { cs: 0x10, ..user_frame() }, stack));
        // A kernel RIP, or none.
        assert!(!frame_is_users(&SavedRegisters { rip: 0xFFFF_F800_0100_0000, ..user_frame() }, stack));
        assert!(!frame_is_users(&SavedRegisters { rip: 0, ..user_frame() }, stack));
        // An RSP outside the thread's stack belongs to some other frame.
        assert!(!frame_is_users(&SavedRegisters { rsp: 0x30_0000, ..user_frame() }, stack));
    }

    #[test]
    fn a_thread_without_a_teb_lists_without_a_stack() {
        // A kernel thread: an ID and a start address, no TEB or stack.
        let kernel_thread = WinThread {
            tid: 0x44,
            start: 0xFFFF_F800_0100_0000,
            teb: None,
            stack: None,
            uses_fibers: false,
            saved: None,
        };
        // It is never a stack owner, whatever region is asked.
        assert_eq!(stack_owner(0, u64::MAX, std::slice::from_ref(&kernel_thread)), None);
        assert!(kernel_thread.stack.is_none());
    }
}
