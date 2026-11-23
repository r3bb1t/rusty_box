# Host-side Linux observation

Milestone 8 starts from `e48b73c`. It samples the active Linux task and its main executable's mappings using an emulator pause, debugger registers, and guest RAM. The observer lives in the downstream execution-workflows crate. Consumers can use its owned snapshots with `App::route()` and `with_state()` or supply an `Emit` implementation. [Run the examples](../../examples/execution_workflows/README.md#sample-linux-tasks-from-the-emulator-host) · [Observer API](../../examples/execution_workflows/src/linux/host.rs)

## What changed since Milestone 7

| Evidence | Milestone 7 | Host-only observation |
| --- | --- | --- |
| Trigger | A tracee is held at `PTRACE_EVENT_EXEC`. The host breaks at the EIP reported by `PTRACE_GETREGS`. | The host breaks at the pinned kernel's return-to-user path, or samples at an existing ring-3 pause. |
| Process and file identity | Supervisor `waitpid` and `/proc/<pid>/exe` | CPU 0's Linux `current_task`, the task's raw TGID and TID, start time, `mm`, and `mm.exe_file` |
| Regions and entry | `/proc/<pid>/maps` and `/proc/<pid>/auxv` | The active `mm` Maple Tree, matching `vm_file` pointers and the saved auxiliary vector |
| CPU and address space | Live EIP, ESP and CR3 corroborate the record sent by the supervisor. | Live registers establish the pause. The translated `mm.pgd` must match the active CR3 page-directory base. |
| COM1 | Carries supervisor records, commands and output. `--elf` asks the supervisor to try to fault in the ELF header page. | Carries shell input, ordinary stdout and exit status. The host reads ELF headers only if explicitly asked and already resident. |

The older `/bin/cat` workflow uses a fixture-specific instruction sequence that leaves a `getpid` result in a register. Milestone 7 moved to a supervisor-selected runtime EIP, including for a relocated PIE. The host-only demonstration uses a single kernel breakpoint for both ordinary, independently linked programs. Its initramfs contains an ordinary BusyBox shell and the two programs. It contains neither the supervisor nor a procfs mount point. The shell sends no process metadata. [Milestone 7](linux-semantic-observation.md) · [Host-only fixture](../../examples/execution_workflows/guest/build_observation.sh)

## Design and scope

CPU state gives the current instruction, privilege level, and page-table base. The Linux profile lets the host interpret kernel RAM as an active task and address space. The return breakpoint is at `restore_all_switch_stack`, before the i386 kernel moves the saved register frame to the entry stack. The observer checks the saved user CS and IP for a pending userspace return. A ring-3 pause supplies the live PC instead. The breakpoint covers this `iret` path. The fast `sysexit` path uses another address. Syscall entry alone establishes a request, which may fail. CR3 can change without an exec, or be shared or reused across tasks. The selected boundary samples the task currently selected to run in userspace. [Return path in Linux 6.1.176](https://github.com/gregkh/linux/blob/v6.1.176/arch/x86/entry/entry_32.S) · [Linux `current_task`](https://github.com/gregkh/linux/blob/v6.1.176/arch/x86/include/asm/current.h)

Each pause resolves `current_task` again. The observer reads the per-CPU address for CPU 0 using `__per_cpu_offset[0]`, validates the task's `mm`, and compares its translated page-directory root to CR3 with the hardware flags masked. `TaskIdentity` holds the raw initial-namespace process ID (TGID), thread ID, start boottime and task address. A new task can reuse a PID, task address, `mm` address or physical page-directory root. The observer keeps no cache keyed by any of them. Its identity is scoped to one boot and forward execution, and `mm` and file addresses are current snapshot values. The fixture uses the display basename to select which samples to publish. Task birth, exit, namespace-local IDs, and generations across exec and snapshot restore require additional evidence.

The executable snapshot walks `mm.mm_mt` while the machine is paused. It checks the pinned `mmap_lock` writer bit, validates supported Maple node types, links, pivots and VMA ownership, and bounds the walk to 128 nodes and 512 VMAs. It keeps VMAs with `VM_EXEC` and the same `struct file` pointer as `mm.exe_file`, with half-open ranges and file offsets. This covers the active main file and can omit other mappings of the same inode opened through a different `struct file`. Shared libraries, anonymous executable pages and interpreter VMAs are outside that filter. The optional entry is the saved `AT_ENTRY`. It may differ from a live PC, an interpreter's initial PC, or an address that's still mapped. The fixture verifies that its entry falls in one reported executable VMA. [Bounded Maple walk](../../examples/execution_workflows/src/linux/host/maps.rs) · [Linux Maple implementation](https://github.com/gregkh/linux/blob/v6.1.176/lib/maple_tree.c) · [Linux rwsem state](https://github.com/gregkh/linux/blob/v6.1.176/kernel/locking/rwsem.c)

The supported setup is single-vCPU Debian i386 `6.1.0-50-686` (`6.1.176-1`), non-PAE and booted with `nokaslr`, running ordinary protected-mode user programs. The observer checks the boot image SHA-256 and verifies the running kernel's build ID in guest RAM. Checked debugger translation and physical RAM reads back every field. A missing page, missing user `mm`, inconsistent page-directory root, active mapping writer, unsupported boundary or unrecognized structure is a recoverable observation error. The embedding application chooses when to resume, discard a partial sample or stop. A broader observer would need matching layouts and validated entry paths for each kernel and paging configuration. SMP would require a coherent pause plus rules for concurrent guest writers. Complete process and exec tracking would require separate, verified lifecycle boundaries.

## Application composition and optional metadata

`Observer::at_pause` borrows the machine exclusively and exposes a `ProcessSample`. `into_sample()` keeps only that process sample and releases the borrow without walking the VMAs. Call `executable()` to walk the same `mm` while the machine remains borrowed. Call `into_observation()` to release the borrow and keep an owned process and executable pair. Delivery uses `Emit` in process-then-executable order. Stops accumulate through the pair and the first handler error ends delivery, with no implicit retry. The downstream binary registers both types through `.on()`, supplies non-`Clone` state with `.with_state()`, and checks its recovered state after both executions. [Downstream application](../../examples/execution_workflows/src/bin/linux_host_observe.rs)

`inspect_elf()` is an optional stage. It uses the same bounded ELF32 parser as Milestone 7 and checks saved auxiliary-vector dimensions, header placement and executable segment ranges against the host-observed VMA data. A readable file VMA may lack a resident header page. In that case the probe returns `AnalysisError::Missing` without faulting the guest page in. Metadata describes the ELF headers present at the pause and supplies no digest or residency proof for executable segments. The basic path skips the ELF probe entirely. [Shared ELF parser](../../examples/execution_workflows/src/linux/metadata.rs) · [Checked debugger interfaces](../../rusty_box/src/emulator/debug.rs)

## Reproduce the pinned layout

Use the versioned installer kernel from the [downstream instructions](../../examples/execution_workflows/README.md#boot-linux-and-replace-bincat). Boot image SHA-256 is `c77cd9c37609b8b1b1cdfbb91b644a44d606d3d75da8656a0c79635be0b89c21`. The matching GNU build ID is `e53bebc4fd9b0992c212d7b0f7f84237a970890d`. The checked-in profile is generated from this boot ELF's note and load addresses and the matching Debian debug ELF's symbols and DWARF member offsets. The generator checks x86 ELF class, endianness, type, note identity and address, unique symbol values and field sizes before accepting the layout. [Layout generator](../../examples/execution_workflows/guest/extract_i386_layout.py) · [Generated profile](../../examples/execution_workflows/src/linux/host/layout.rs)

```sh
curl -fL 'https://deb.debian.org/debian/pool/main/l/linux/linux-image-6.1.0-50-686-dbg_6.1.176-1_i386.deb' -o "$RB_WORK/kernel-dbg.deb"
printf '%s  %s\n' '4625e3cd446ff2ade7776492bfdecd60b2fb6a72b8b99a97e04f0e472dfeeb84' "$RB_WORK/kernel-dbg.deb" | sha256sum -c -
dpkg-deb --fsys-tarfile "$RB_WORK/kernel-dbg.deb" |
  tar -xO --wildcards '*/vmlinux-6.1.0-50-686' > "$RB_WORK/vmlinux"
curl -fL 'https://raw.githubusercontent.com/gregkh/linux/v6.1.176/scripts/extract-vmlinux' -o "$RB_WORK/extract-vmlinux"
sh "$RB_WORK/extract-vmlinux" "$RB_WORK/linux" > "$RB_WORK/boot.elf"
python3 -m venv "$RB_WORK/layout-tools"
"$RB_WORK/layout-tools/bin/pip" install 'pyelftools==0.32'
"$RB_WORK/layout-tools/bin/python" examples/execution_workflows/guest/extract_i386_layout.py \
  "$RB_WORK/linux" "$RB_WORK/boot.elf" "$RB_WORK/vmlinux" > "$RB_WORK/layout.rs"
diff -u examples/execution_workflows/src/linux/host/layout.rs "$RB_WORK/layout.rs"
```

## Live checks and paused-read costs

The host-only guest ran `program_sum` and `program_text` from its shell. They returned status 0 and printed `sum=22` and `argv0=/bin/program_text`. Both produced `PendingUserReturn` snapshots. An example run captured task IDs 82 and 83 with CR3 values `0x2cc6000` and `0x2ca9000`, respectively. The static executable entry was `0x8049000` and the relocated static PIE entry was `0xb7f2d000`. Each entry fell in a file-backed executable VMA with offset `0x1000`. These addresses are observations from that run.

The `--benchmark` measurements are medians of 256 in-process repetitions at each already-stopped boundary, release build on an AMD EPYC 9V74 with Rust 1.100.0. They exclude guest execution, breakpoint delivery, kernel image hashing, event dispatch and COM1 work. The two fixture processes had small address spaces, so these are paused-read costs for the demonstrated workload.

| Fixture | Task and CR3 verification | VMA walk and auxv | Optional ELF attempt |
| --- | ---: | ---: | ---: |
| `program_sum` | 961 ns | 2,093 ns | 50 ns, header page absent |
| `program_text` | 961 ns | 2,413 ns | 50 ns, header page absent |

A separate boot with the Milestone 7 supervisor compared the host's ring-3 snapshots against ptrace and procfs. PID, executable entry and executable ranges agreed for both programs. Both observers saw live CR3 `0x2cc7000` for PID 84 (`program_sum`) and again for PID 86 (`program_text`). The host resolved distinct task identities at the two pauses. The supervisor selected the ring-3 breakpoint and prepared header residency in this run. The new observer derived its process and executable data independently from registers and RAM. The shared ELF parser agreed on the resident metadata and took medians of 441 ns and 701 ns across 256 reads at the respective pauses. [Oracle comparison](../../examples/execution_workflows/src/linux/host.rs)

The downstream Rust tests cover unknown profiles, unsupported CPU state, missing memory, malformed auxiliary vectors, bounded Maple slots, and typed ordering with stop and error propagation. The two guest programs exercise small VMA trees. The automated host-only boot, the independent oracle comparison, the original two-mode Milestone 7 boot regression, and downstream Clippy passed. The existing compiler fixtures, composed-hardware tests, and no-default-feature core and device suites passed. Portability checks covered x86 `no_std` with and without `alloc`, a bare-metal `no_alloc` link without allocator symbols, WebAssembly with `alloc`, and bare-metal AArch64 with and without `alloc`.
