# WHP VMM-shape campaign — baseline at 7728086

Host: 12th Gen Intel Core i5-12450H (8 cores / 12 threads, hybrid: 4 performance +
4 efficiency), 15.7 GiB RAM. Windows 11 Home Single Language, build 26200.9168.
Defender real-time scanning: **on** (not disabled for these runs — the campaign's
later numbers are measured the same way, so the comparison holds). Hyper-V
hypervisor present: yes. Other hypervisor partitions during the runs: none.

**The host is shared, and it was not quiet.** Other projects are built on this
machine without warning, and a compile and link were observed running during the
sweep. Rather than claim a quiet host, every run below records the CPU load and the
number of live compiler and linker processes immediately before and after itself, so
contention is visible in the data. The two engines are therefore measured
**interleaved, round-robin, inside one session** — the project's standing rule
(memory `perf-measurement-methodology`), which exists because absolute wall-clock on
this laptop is not comparable across runs or days: thermal drift alone once produced
a phantom 8 % win that vanished under controlled measurement, and the noise floor
here is about 3.7 %.

The consequence for this campaign's gates is recorded rather than papered over: a
number in this document may not be compared against a Stage 1 number measured on a
different day. Stage 1's gate re-measures its comparison points in the same
interleaved sweep as the new engine.

The hybrid topology is recorded because it is a live source of variance: a vCPU
thread scheduled onto an efficiency core runs materially slower than one on a
performance core, and nothing in this campaign pins affinity. Where three runs
disagree by more than a few percent, suspect that before suspecting the change.

Measured 2026-09-04 with the step-driven harnesses (`alpine_probe`, `dlx_whp`);
no GUI. Three runs each unless stated.

ISO: `alpine-virt-3.24.1-x86_64.iso`, SHA-256
`e73a6241bd5f3c5c2d4d38c02cc52c378c0415a7c888bd292066bf36e0f41a39`, 69 206 016 bytes.
This is the original image with `quiet` intact on its APPEND line, verified by
reading the line out of the ISO. The no-quiet copy that disqualified the 2026-09-01
A/B no longer exists on this host; both copies present are byte-identical to the
hash above.

Gate suite at head: `cargo xtask ci` **green**, 22 steps, 1010.7 s. This discharges
the gate-hygiene debt carried by `374e803` and `7492d80`, neither of which had been
run through the gates.

## What these runs are, and are not

**They are functional evidence with indicative timings. They are not a benchmark.**
The host was shared and busy, and the sweep was stopped after one interleaved round
rather than spending another quarter-hour of a machine someone else is working on.
The findings below are the kind that survive that: a guest either reaches a login
prompt or it does not, and a ratio of 4.5× is nowhere near this machine's ~3.7 %
noise floor. Do not quote the second decimal of any wall time here, and do not
subtract these numbers from a Stage 1 result — Stage 1's gate re-measures both arms
in one interleaved sweep of its own.

| Guest | Engine | Result | Wall | Notes |
|---|---|---|---|---|
| Alpine | whp (slice engine) | **`login:` NOT reached** | 300 s patience exhausted | OpenRC 205.5 s, "Starting" 292.1 s |
| Alpine | interpreter | `login:` reached | 94.5 s | OpenRC 45.7 s, "Starting" 62.7 s |
| DLX | whp (slice engine) | **3 of 3 milestones**, login detected | 2.3 s | healthy; matches its historical ~2 s |
| Windows 7 | whp (GUI, user-timed) | owed | | |

**The hypervisor engine is roughly 4.5× slower than the interpreter on Alpine, and
does not finish.** The interpreter is at OpenRC by 45.7 s; the hypervisor engine
takes 205.5 s to reach the same point and never gets to a prompt inside five
minutes. A second, earlier run agrees: OpenRC at 167 s, no prompt.

**Where the time goes.** From `RESULT platform`, two runs:

| Run | VP runtime total | of which hypervisor overhead | guest execution | guest ÷ wall |
|---|---|---|---|---|
| 1 | 12 119 ms | 10 275 ms | 1 844 ms | 0.61 % |
| 2 | 12 440 ms | 10 417 ms | 2 023 ms | 0.67 % |

In a five-minute run the processor is scheduled inside the hypervisor for about
twelve seconds, and only about two of those execute guest instructions. In-run
share as the plan defines it — guest ÷ total VP runtime — is about 16 %.

The slice census names the mechanism: 2 191 772 slices in 300 s, of which
1 872 857 (**85.4 %**) produced no exit at all. The engine pays a full partition
round trip and gets nothing back. Exits in that run: memory 299 451, boundary
121 199, canceled 15 987, window 49 518, port 3 151; injections 212 002; nested
page faults 315 655.

This is the premise of the whole campaign, re-measured on this tree rather than
quoted from an earlier session.

## A defect this baseline found in the plan's own gate

The milestone `"Linux version"` was **not** recorded in any run, on either engine,
while `"OpenRC"` — which happens later — was. The harness samples the 80×25 text
screen, and that line had scrolled away before the next sample. The Stage 1 gate
defines its headline window as `"Linux version"` → `login:`, so **as written that
gate cannot be computed**. It is corrected in the plan: the window must start at a
milestone the scrape reliably catches, or the scrape must accumulate the distinct
lines it has seen so a scrolled-past milestone still registers — which Task 1.8
already requires for the fatal-signature scan, and which now serves both.

## G0 — VMware Workstation on this host (user-measured)

Procedure: new VM, 1 vCPU, 256 MiB, the same Alpine ISO attached as CD, boot;
stopwatch from "power on" to the `login:` prompt, three times. Same for the
Windows 7 ISO with 1024 MiB, to the edition-selection list.

| Guest | Milestone | Run 1 | Run 2 | Run 3 | Median |
|---|---|---|---|---|---|
| Alpine | `login:` | | | | |
| Windows 7 | edition list | | | | |

**Owed.** Gates G2 and G3 (within 2× of VMware) cannot be judged until these rows
and the two Windows 7 rows above are filled. Everything else in Stage 1's gate is
judgeable without them.
