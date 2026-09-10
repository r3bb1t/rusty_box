# What a machine may tell its guest it has

Measured on 2026-08-31 against the Windows Hypervisor Platform engine, on a
12th Gen Core i5-12450H. Companion to `docs/whp-platform-probe-2026-08-27.md`.

## The rule

**Advertise nothing the guest cannot execute.** A guest reads `CPUID` once and
commits: Linux picks its idle routine at boot and never reconsiders, and enables
`XCR0` components the moment it sees the feature bits. A capability that is
advertised and then faults does not produce a retry — it produces a dead guest,
usually far from the advertisement and looking like something else entirely.

This is not this port's rule. `CPUID` exits unconditionally under VMX and SVM, so
every hypervisor supplies the answer itself and every one of them narrows it.
KVM's `KVM_GET_SUPPORTED_CPUID` returns "the host cpuid ... with unknown or
unsupported features masked out"; VMware's EVC masks `CPUID` down to a cluster
baseline so a guest keeps running when the hardware under it changes. The second
is exactly this port's problem, because a guest here can be moved between the
interpreter and the hypervisor mid-run.

## Three terms, not one

What may be offered is the intersection of:

1. **What the host's silicon carries.** `CPUID.D:0` reports the extended-state
   components in `EDX:EAX`. This host answers `0x7` — x87, SSE, AVX. No AVX-512.
2. **What this port implements.** An instruction the host has and the interpreter
   does not works on hardware and faults the moment a burst or a trapped
   instruction lands on it. The two processors must agree.
3. **What the partition permits.** Neither of the above covers `MONITOR`/`MWAIT`:
   the host executes it, this port implements it, and the platform still cannot
   give it to a guest. `WHV_PROCESSOR_FEATURES` in the Windows SDK header
   (`WinHvPlatformDefs.h`, 10.0.26100.0) contains no `Mwait` or `Monitor` field
   at all.

Term 3 was found the hard way and is the one an "expose the host's processor"
design forgets.

## What went wrong, and how each looked

| Advertised | Guest did | Died as |
|---|---|---|
| AVX-512 (`CPUID.7:0`) | `XSETBV` set `XCR0=0xe7` | `WHV_E_INVALID_VP_STATE` on the next register write |
| VMX (`CPUID.1:ECX[5]`) | wrote `IA32_FEATURE_CONTROL` to lock it | triple fault in firmware with no IDT, at `0xE1E80` |
| `MONITOR` (`CPUID.1:ECX[3]`) | selected `mwait_idle` | `Oops: invalid opcode` in `swapper/0`, "Attempted to kill the idle task" |

None of the three names its own cause in the failure. That is the point of the
rule.

## Where the answer has to live

`CPUID` is answered in **two** places on this engine:

- the `Cpuid` exit, which `withhold_virtualisation_from` filters — and which the
  census shows **never fires** (`exits: cpuid 0`), so that filter is dead code;
- `burst_on_the_shadow` and `emulate_one`, which execute the guest's `CPUID` on
  the interpreter and answer from the model with no filter at all.

So a filter at the exit cannot work, and the withholding belongs to the processor
both paths share. It does: `cpu_include_features` / `cpu_exclude_features` on
`BxParams` — Bochs `cpuid: <feature>=0`, present in this tree with no consumer
until now — edit `ia_extensions_bitmask` at initialisation, and the affected
`CPUID` leaves derive from that bitmask rather than from the model's static table:

- leaf `D` subleaf 0 already did, through `xcr0_suppmask`;
- leaf 7 subleaf 0 withdraws the AVX-512 bits;
- leaf 1 withdraws `ECX[3]` when `IsaMonitorMwait` is absent.

`--cpu-capabilities host-shared` selects the narrowing; `preset` is the default
and leaves an interpreter run answering `CPUID` byte for byte as before.

**It is a machine setting, not an engine one.** A guest keeps what it enabled
across a switch from the hypervisor to the interpreter, so the two must offer the
same processor or the switch changes the hardware under a running guest.

## The platform API this should use instead

Read from `WinHvPlatformDefs.h`:

```
WHvPartitionPropertyCodeCpuidExitList     = 0x00001003
WHvPartitionPropertyCodeCpuidResultList   = 0x00001004
WHvPartitionPropertyCodeCpuidResultList2  = 0x0000100D   // + WHV_X64_CPUID_RESULT2_FLAGS
WHvPartitionPropertyCodeProcessorFeatures = 0x00001001   // no MWAIT bit
```

`CpuidResultList` registers answers **with the partition**, returned without an
exit. That is strictly better than what is built today: one answer, authoritative
on both execution paths, and no exits to pay for. It would also make `preset`
enforceable on the hardware path, which today it is not — the preset governs only
what the shadow answers.

Not yet used. The property codes and struct names are read; the semantics
(subleaf addressing, which functions are permitted, interaction with
`CpuidExitList`) are not yet checked.

## What other hypervisors do about these same features

- **`MONITOR`/`MWAIT`** — KVM hides it by default: "Nobody really wants to expose
  MONITOR/MWAIT features to the guest by default, as it would eat up all of the
  host CPU." Exposure is opt-in through `-overcommit cpu-pm=on`
  (`KVM_CAP_X86_DISABLE_EXITS`). QEMU has this port's exact bug on record: with
  that flag on a host lacking MWAIT, the feature is advertised and executing it
  raises `#UD`.
- KVM additionally emulates intercepted `MONITOR`/`MWAIT` as **NOPs** by default
  (`KVM_X86_QUIRK_MWAIT_NEVER_FAULTS`), so a guest that uses them anyway degrades
  rather than dying. **Not adopted here**: Bochs implements `MWAIT`, and turning
  it into a NOP would change guest-visible behaviour on the interpreter too. It
  is recorded as an option, not a plan.
- **AVX-512 and other XSAVE-bearing features** — masked to host support, KVM's
  `cpuid_mask` reading the host capability words and clearing what is absent.
- **The hypervisor-present bit** (`CPUID.1:ECX[31]`) — QEMU's WHPX sets it, because
  some Linux kernels otherwise touch MSRs that make no sense virtualised. **Not
  set here.** Every guest this port boots does so without it, and announcing
  virtualisation runs against the stealth this port's parity is for. Recorded so
  a future guest that needs it is not a mystery.

## Sources

- KVM API: <https://www.kernel.org/doc/html/v6.3/virt/kvm/api.html>
- Per-vCPU exit disable: <https://lwn.net/Articles/898612/>
- KVM MONITOR/MWAIT NOP quirk: <https://www.spinics.net/lists/kvm/msg279754.html>
- QEMU `-overcommit cpu-pm`: <https://lists.gnu.org/archive/html/qemu-devel/2018-06/msg06797.html>
- WHP partition property data types:
  <https://learn.microsoft.com/en-us/virtualization/api/hypervisor-platform/funcs/whvpartitionpropertydatatypes>
