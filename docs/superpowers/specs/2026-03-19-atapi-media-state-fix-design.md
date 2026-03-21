# ATAPI Media State Machine + Stat Error Investigation

## Problem

Alpine modloop mount stalls at 1-2 MIPS with no ATA I/O. Bochs handles this in ~1 second. Additionally, `stat: can't stat '68608'` errors appear before OpenRC that don't appear in Bochs.

## Root Cause

1. **Missing `status_changed` field** — Bochs tracks a 3-state FSM (1→-1→0) for media change detection. We don't track it at all.
2. **TEST_UNIT_READY** always returns ready/not-ready. Bochs returns NOT_READY→UNIT_ATTENTION→ready sequence on media change.
3. **GET_EVENT_STATUS_NOTIFICATION** hardcodes event code 0. Bochs returns dynamic codes (0/3/4) based on status_changed.
4. **Stat errors** — unknown command produces "68608", "0", "100%" as output, parsed as filenames. Needs investigation.

## Design: Exact Bochs Parity

### S1: Add `status_changed` field to AtaDrive

Match Bochs `harddrv.h:258`: `int status_changed`.

- Type: `i32` (-1, 0, 1)
- Init in `new()`: 0 (matches Bochs zero-init)
- Set to 1 in `attach_cdrom()` (simulates Bochs `cdrom_status_handler` line 3871)
- Reset to 0 in `reset()`

### S2: Fix TEST_UNIT_READY (opcode 0x00)

Match Bochs `harddrv.cc:1335-1352` exactly:

```
status_changed == 1:  atapi_cmd_error(NOT_READY, MEDIUM_NOT_PRESENT); status_changed = -1
status_changed == -1: atapi_cmd_error(UNIT_ATTENTION, MEDIUM_MAY_HAVE_CHANGED); status_changed = 0
status_changed == 0:  existing logic (ready → nop, not ready → error)
```

### S3: Fix GET_EVENT_STATUS_NOTIFICATION (opcode 0x4A)

Match Bochs `harddrv.cc:1902-1903`:

```
buffer[4] = if status_changed == 0 { 0 }
            else if inserted { 4 }
            else { 3 };
```

### S4: Investigate stat errors

Add `debug_init=1` to kernel cmdline and trace which command produces "68608", "0", "100%". Compare with Bochs. This likely reveals a functional emulator bug (wrong instruction output, broken BusyBox command, etc.).

## Verification

1. DLX regression: boot to `dlx:~#`
2. Alpine: modloop mount completes (not stalls at 1-2 MIPS)
3. Alpine: no stat errors before OpenRC
4. Alpine: OpenRC services start normally

## Files Modified

- `rusty_box/src/iodev/harddrv.rs` — status_changed field + TUR/GESN fixes
