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

    #[test]
    fn a_thread_without_a_teb_lists_without_a_stack() {
        // A kernel thread: an ID and a start address, no TEB or stack.
        let kernel_thread = WinThread {
            tid: 0x44,
            start: 0xFFFF_F800_0100_0000,
            teb: None,
            stack: None,
            uses_fibers: false,
        };
        // It is never a stack owner, whatever region is asked.
        assert_eq!(stack_owner(0, u64::MAX, std::slice::from_ref(&kernel_thread)), None);
        assert!(kernel_thread.stack.is_none());
    }
}
