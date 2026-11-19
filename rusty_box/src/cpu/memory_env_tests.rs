//! Real decode/execute/MMU regressions: Bochs cpu.cc, access2.cc, paging.cc,
//! exception.cc and icache.cc. Dense/open-bus policy is this host's choice.
use alloc::boxed::Box;

use super::api_bridge::SegmentSize;
use super::builder::BxCpuBuilder;
use super::decoder::BxSegregs;
use super::exec_ctx::ExecCtx;
use super::{BxCpuC, CpuError, X86Reg};
use crate::config::{A20Mask, BxLinAddress, BxPhyAddress};
use crate::memory::flat::FlatMemory;
use crate::memory::{BxMemC, BxMemoryStubC};
const MIB: usize = 1 << 20;
const CODE: u64 = 0x2000;
fn at(addr: u64) -> BxPhyAddress {
    BxPhyAddress::new(addr)
}
fn cpu() -> Box<BxCpuC<()>> {
    let mut cpu = BxCpuBuilder::new().build().unwrap();
    cpu.reset(super::ResetReason::Hardware);
    for reg in [
        X86Reg::Cs,
        X86Reg::Ss,
        X86Reg::Ds,
        X86Reg::Es,
        X86Reg::Fs,
        X86Reg::Gs,
    ] {
        cpu.set_seg_for_api(
            reg,
            if reg == X86Reg::Cs { 8 } else { 16 },
            0,
            u32::MAX,
            SegmentSize::Bits32,
        );
    }
    cpu.enter_protected_mode_for_api();
    cpu.set_rip(CODE);
    cpu.prev_rip = CODE;
    cpu.set_rsp(0xD000);
    cpu.set_rflags_for_api(2);
    cpu
}
fn pc_memory() -> BxMemC {
    let mut memory = BxMemC::new(
        BxMemoryStubC::create_and_init(2 * MIB, 2 * MIB, 4096).unwrap(),
        false,
    );
    memory.set_a20_mask(A20Mask::OPEN);
    memory
}
fn environment(flat: bool, f: impl FnOnce(&mut ExecCtx<'_, ()>)) {
    environment_storage(flat, false, f);
}
fn environment_storage(flat: bool, aligned: bool, f: impl FnOnce(&mut ExecCtx<'_, ()>)) {
    let mut cpu = cpu();
    if flat {
        let mut bytes = alloc::vec![0;2*MIB+4096+3];
        let mut stamps = alloc::vec![0;FlatMemory::required_stamps(2*MIB+3)];
        let offset = if aligned {
            bytes.as_ptr().align_offset(4096)
        } else {
            1
        };
        // Neither host alignment nor page-multiple length may be assumed.
        let mut memory =
            FlatMemory::new(&mut bytes[offset..offset + 2 * MIB + 3], &mut stamps).unwrap();
        f(&mut ExecCtx::flat(&mut cpu, &mut memory));
    } else {
        let mut memory = pc_memory();
        f(&mut ExecCtx::cpu_only(&mut cpu, &mut memory).unwrap());
    }
}
fn put(ctx: &mut ExecCtx<'_, ()>, address: u64, bytes: &[u8]) {
    assert_eq!(
        ctx.memory.write_ram(at(address), bytes).unwrap(),
        bytes.len()
    );
}
fn get<const N: usize>(ctx: &mut ExecCtx<'_, ()>, address: u64) -> [u8; N] {
    let mut bytes = [0; N];
    assert_eq!(ctx.memory.read_ram(at(address), &mut bytes).unwrap(), N);
    bytes
}
fn run(ctx: &mut ExecCtx<'_, ()>, count: u64) {
    assert_eq!(ctx.cpu_loop_n_slice(count, true, 1).unwrap(), count);
}
fn paging(ctx: &mut ExecCtx<'_, ()>, table: u64) {
    put(ctx, 0x1000, &(table as u32 | 3).to_le_bytes());
    for page in 0..16u32 {
        put(
            ctx,
            table + u64::from(page) * 4,
            &(page * 4096 | 3).to_le_bytes(),
        );
    }
    put(ctx, 0x8008, &0x00CF_9B00_0000_FFFFu64.to_le_bytes());
    put(ctx, 0x8010, &0x00CF_9300_0000_FFFFu64.to_le_bytes());
    ctx.gdtr.base = BxLinAddress::new(0x8000);
    ctx.gdtr.limit = 0x17;
    ctx.idtr.base = BxLinAddress::new(0x9000);
    ctx.idtr.limit = 14 * 8 + 7;
    for vector in [13u64, 14] {
        put(ctx, 0x9000 + vector * 8, &[0, 0x30, 8, 0, 0, 0x8E, 0, 0]);
    }
    put(ctx, 0x3000, &[0x83, 0xC4, 4, 0xCF]); // add esp,4; iretd
    ctx.api_reg_write(X86Reg::Cr3, 0x1000);
    let cr0 = ctx.cr0.get32() | 0x8001_0001;
    ctx.api_reg_write(X86Reg::Cr0, u64::from(cr0));
}
fn long_paging(ctx: &mut ExecCtx<'_, ()>) {
    put(ctx, 0x1000, &0x4003u64.to_le_bytes());
    put(ctx, 0x4000, &0x5003u64.to_le_bytes());
    put(ctx, 0x5000, &0x83u64.to_le_bytes());
    ctx.set_seg_for_api(X86Reg::Cs, 8, 0, u32::MAX, SegmentSize::Long64);
    ctx.enter_long_mode_for_api(0x1000);
}

#[test]
fn both_maps_run_arithmetic_data_and_long_mode_paging() {
    for flat in [false, true] {
        environment(flat, |ctx| {
            put(
                ctx,
                CODE,
                &[
                    0xB8, 7, 0, 0, 0, 0x83, 0xC0, 5, 0xA3, 0, 0x60, 0, 0, 0xA1, 0, 0x60, 0, 0,
                ],
            );
            run(ctx, 4);
            assert_eq!(ctx.eax(), 12);
            assert_eq!(get::<4>(ctx, 0x6000), 12u32.to_le_bytes());
            long_paging(ctx);
            put(
                ctx,
                CODE,
                &[
                    0x48, 0x8B, 3, 0x48, 0x83, 0xC0, 5, 0x48, 0x89, 3, 0xEB, 0xF4,
                ],
            );
            ctx.set_rip(CODE);
            ctx.set_rbx(0x6000);
            run(ctx, 8);
            assert_eq!(ctx.rax(), 22);
            assert_eq!(get::<8>(ctx, 0x6000), 22u64.to_le_bytes());
        });
    }
}
#[test]
fn flat_legacy_aperture_is_ram_and_a20_is_open() {
    environment(true, |ctx| {
        paging(ctx, 0xA0000);
        put(ctx, 0xA0000 + 0xE0 * 4, &0xE0003u32.to_le_bytes());
        put(ctx, 0xA0000 + 0x100 * 4, &0x10_0003u32.to_le_bytes());
        put(
            ctx,
            0xE0000,
            &[
                0xB8, 0x78, 0x56, 0x34, 0x12, 0xA3, 0, 0, 0x10, 0, 0xA1, 0, 0, 0x10, 0,
            ],
        );
        ctx.set_rip(0xE0000);
        run(ctx, 3);
        assert_eq!(ctx.eax(), 0x1234_5678);
        assert_eq!(get::<4>(ctx, 0x10_0000), 0x1234_5678u32.to_le_bytes());
        assert_eq!(get::<4>(ctx, 0), [0; 4]);
        assert_eq!(
            u32::from_le_bytes(get::<4>(ctx, 0xA0400)) & 0x60,
            0x60,
            "walker wrote A/D bits in VGA-addressed RAM"
        );
    });
}
#[test]
fn both_maps_host_dma_and_guest_writes_invalidate_warm_traces() {
    for flat in [false, true] {
        environment(flat, |ctx| {
            put(ctx, CODE, &[0xB8, 1, 0, 0, 0, 0xEB, 0xF9]);
            run(ctx, 2);
            assert_eq!(ctx.eax(), 1);
            put(ctx, CODE + 1, &9u32.to_le_bytes());
            run(ctx, 2);
            assert_eq!(ctx.eax(), 9);
            put(
                ctx,
                CODE,
                &[0xC6, 0x05, 8, 0x20, 0, 0, 7, 0xB8, 1, 0, 0, 0, 0xEB, 0xF2],
            );
            run(ctx, 3);
            assert_eq!(ctx.eax(), 7);
            assert_eq!(ctx.rip(), CODE);
        });
    }
}
#[test]
fn both_maps_split_store_page_fault_is_atomic_and_iret_retries_once() {
    for flat in [false, true] {
        for width in [2usize, 4, 8] {
            environment(flat, |ctx| {
                paging(ctx, 0x4000);
                put(ctx, 0x401C, &0u32.to_le_bytes());
                let dest = 0x7000 - width as u64 / 2;
                put(ctx, dest, &[0xAA; 8][..width]);
                let mut code = match width {
                    2 => alloc::vec![0x66, 0xA3],
                    4 => alloc::vec![0xA3],
                    _ => alloc::vec![
                        0xF3, 0x0F, 0x7E, 0x05, 0, 0x60, 0, 0, 0x66, 0x0F, 0xD6, 0x05
                    ],
                };
                code.extend_from_slice(&(dest as u32).to_le_bytes());
                put(ctx, CODE, &code);
                ctx.set_eax(0x1234_5678);
                if width == 8 {
                    ctx.api_reg_write(X86Reg::Cr4, 1 << 9);
                    put(ctx, 0x6000, &0x1122_3344_5566_7788u64.to_le_bytes());
                    run(ctx, 1);
                }
                let fault_ip = ctx.rip();
                let before = ctx.icount;
                run(ctx, 1);
                assert_eq!(ctx.rip(), 0x3000);
                assert_eq!(ctx.cr2, 0x7000);
                assert_eq!(get::<8>(ctx, dest)[..width], [0xAA; 8][..width]);
                assert_eq!(get::<4>(ctx, 0xCFF0), 2u32.to_le_bytes());
                assert_eq!(get::<4>(ctx, 0xCFF4), (fault_ip as u32).to_le_bytes());
                assert_eq!(
                    ctx.icount,
                    before + 1,
                    "existing loop charges exception delivery, not successful store retirement"
                );
                put(ctx, 0x401C, &0x7003u32.to_le_bytes());
                run(ctx, 2);
                assert_eq!(ctx.rip(), fault_ip);
                run(ctx, 1);
                assert_eq!(ctx.rip(), CODE + code.len() as u64);
                let expected = if width == 8 {
                    0x1122_3344_5566_7788u64
                } else {
                    0x1234_5678
                };
                assert_eq!(
                    get::<8>(ctx, dest)[..width],
                    expected.to_le_bytes()[..width]
                );
            });
        }
    }
}
#[test]
fn both_maps_cross_page_fault_precedes_mmio_callbacks_and_permissions() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    for flat in [false, true] {
        for write in [false, true] {
            for host_permission in [false, true] {
                environment(flat, |ctx| {
                    paging(ctx, 0x4000);
                    if host_permission {
                        let mut perms =
                            crate::memory::permissions::PagePermissions::new(2 * MIB as u64);
                        perms.set(at(0x7000), 4096, super::MemPerms::empty());
                        ctx.page_permissions = Some(perms);
                    } else {
                        put(ctx, 0x401C, &0u32.to_le_bytes());
                    }
                    let calls = Arc::new(AtomicUsize::new(0));
                    let reads = calls.clone();
                    let writes = calls.clone();
                    ctx.mmio.map(
                        at(0x6FFE),
                        2,
                        Box::new(move |_, _| {
                            reads.fetch_add(1, Ordering::Relaxed);
                            0
                        }),
                        Box::new(move |_, _, _| {
                            writes.fetch_add(1, Ordering::Relaxed);
                        }),
                    );
                    put(
                        ctx,
                        CODE,
                        &[if write { 0xA3 } else { 0xA1 }, 0xFE, 0x6F, 0, 0],
                    );
                    run(ctx, 1);
                    assert_eq!(ctx.rip(), 0x3000);
                    assert_eq!(calls.load(Ordering::Relaxed), 0);
                    if host_permission {
                        assert_eq!(
                            get::<4>(ctx, 0xCFF0),
                            [0; 4],
                            "host protection remains #GP(0)"
                        );
                    } else {
                        assert_eq!(ctx.cr2, 0x7000);
                    }
                });
            }
        }
    }
}
#[test]
fn both_maps_warm_data_tlb_does_not_bypass_callback_mmio() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    for flat in [false, true] {
        environment(flat, |ctx| {
            paging(ctx, 0x4000);
            let calls = Arc::new(AtomicUsize::new(0));
            let reads = calls.clone();
            ctx.mmio.map(
                at(0x6080),
                4,
                Box::new(move |_, size| {
                    assert_eq!(size, 4);
                    reads.fetch_add(1, Ordering::Relaxed);
                    77
                }),
                Box::new(|_, _, _| {}),
            );
            put(ctx, CODE, &[0xA1, 0x80, 0x60, 0, 0, 0xEB, 0xF9]);
            run(ctx, 4);
            assert_eq!(ctx.eax(), 77);
            assert_eq!(calls.load(Ordering::Relaxed), 2);
        });
    }
}
#[test]
fn both_maps_guest_remap_and_invlpg_replace_a_warm_data_translation() {
    for flat in [false, true] {
        environment(flat, |ctx| {
            paging(ctx, 0x4000);
            put(ctx, CODE, &[0xA1, 0, 0x60, 0, 0, 0xEB, 0xF9]);
            put(ctx, 0x6000, &11u32.to_le_bytes());
            put(ctx, 0xA000, &22u32.to_le_bytes());
            run(ctx, 2);
            assert_eq!(ctx.eax(), 11);
            put(ctx, 0x4018, &0xA003u32.to_le_bytes());
            put(
                ctx,
                0x2100,
                &[
                    0x0F, 0x01, 0x3D, 0, 0x60, 0, 0, 0xE9, 0xF4, 0xFE, 0xFF, 0xFF,
                ],
            );
            ctx.set_rip(0x2100);
            ctx.invalidate_prefetch_q();
            run(ctx, 4);
            assert_eq!(ctx.eax(), 22);
            assert_eq!(ctx.rip(), CODE);
        });
    }
}
#[test]
fn both_maps_nx_fault_after_invlpg_and_iret_restart() {
    for flat in [false, true] {
        environment(flat, |ctx| {
            put(ctx, 0x1000, &0x4003u64.to_le_bytes());
            put(ctx, 0x4000, &0x5003u64.to_le_bytes());
            put(ctx, 0x5000, &0x6003u64.to_le_bytes());
            for page in 0..16u64 {
                put(ctx, 0x6000 + page * 8, &(page * 4096 | 3).to_le_bytes());
            }
            put(ctx, 0x6058, &0xA003u64.to_le_bytes());
            put(ctx, 0xA000, &[0x90, 0xEB, 0xFD]);
            put(ctx, 0x8008, &0x00AF_9B00_0000_FFFFu64.to_le_bytes());
            put(ctx, 0x8010, &0x00CF_9300_0000_FFFFu64.to_le_bytes());
            ctx.gdtr.base = BxLinAddress::new(0x8000);
            ctx.gdtr.limit = 0x17;
            ctx.idtr.base = BxLinAddress::new(0x9000);
            ctx.idtr.limit = 14 * 16 + 15;
            put(
                ctx,
                0x9000 + 14 * 16,
                &[0, 0x30, 8, 0, 0, 0x8E, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            );
            put(ctx, 0x3000, &[0x48, 0x83, 0xC4, 8, 0x48, 0xCF]);
            ctx.efer.insert(super::crregs::BxEfer::NXE);
            ctx.set_seg_for_api(X86Reg::Cs, 8, 0, u32::MAX, SegmentSize::Long64);
            ctx.enter_long_mode_for_api(0x1000);
            ctx.set_rip(0xB000);
            run(ctx, 2);
            put(ctx, 0x6058, &(0xA003u64 | 1 << 63).to_le_bytes());
            ctx.tlb_invlpg(BxLinAddress::new(0xB000));
            run(ctx, 1);
            assert_eq!(ctx.rip(), 0x3000);
            assert_eq!(ctx.cr2, 0xB000);
            assert_eq!(get::<8>(ctx, 0xCFD0), 0x11u64.to_le_bytes());
            assert_eq!(get::<8>(ctx, 0xCFD8), 0xB000u64.to_le_bytes());
            put(ctx, 0x6058, &0xA003u64.to_le_bytes());
            ctx.tlb_invlpg(BxLinAddress::new(0xB000));
            run(ctx, 2);
            assert_eq!(ctx.rip(), 0xB000);
            run(ctx, 1);
            assert_eq!(ctx.rip(), 0xB001);
        });
    }
}
#[test]
fn flat_same_address_replacement_discards_backing_caches() {
    let mut cpu = cpu();
    let mut bytes = alloc::vec![0;MIB];
    let mut stamps = alloc::vec![0;MIB/4096];
    for value in [11u32, 22, 33] {
        bytes[CODE as usize..CODE as usize + 7].copy_from_slice(&[0xA1, 0, 0x60, 0, 0, 0xEB, 0xF9]);
        bytes[0x6000..0x6004].copy_from_slice(&value.to_le_bytes());
        let mut memory = FlatMemory::new(&mut bytes, &mut stamps).unwrap();
        let mut ctx = ExecCtx::flat(&mut cpu, &mut memory);
        paging(&mut ctx, 0x4000);
        run(&mut ctx, 2);
        assert_eq!(ctx.eax(), value);
    }
    cpu.set_rip(CODE);
    let mut short = alloc::vec![0;CODE as usize];
    let mut memory = FlatMemory::new(&mut short, &mut stamps).unwrap();
    let mut ctx = ExecCtx::flat(&mut cpu, &mut memory);
    ctx.api_reg_write(X86Reg::Cr0, 1);
    let before = ctx.icount;
    assert!(ctx.cpu_loop_n_slice(1, true, 1).is_err());
    assert_eq!(ctx.icount, before);
    assert_eq!(ctx.rip(), CODE);
}
#[test]
fn pc_rebind_after_flat_on_same_bytes_invalidates_policy_and_smc_watermark() {
    let mut cpu = cpu();
    let mut memory = pc_memory();
    let (base, len) = memory.allocation_span();
    let mut stamps = alloc::vec![0;FlatMemory::required_stamps(len)];
    {
        let bytes = memory.allocation_slice(0, len as u64).unwrap();
        let mut flat = FlatMemory::new(bytes, &mut stamps).unwrap();
        flat.write(at(CODE), &[0xB8, 11, 0, 0, 0, 0xEB, 0xF9])
            .unwrap();
        let mut ctx = ExecCtx::flat(&mut cpu, &mut flat);
        run(&mut ctx, 2);
        for value in 30u32..40 {
            put(&mut ctx, CODE + 1, &value.to_le_bytes());
            run(&mut ctx, 2);
        }
    }
    assert_eq!(memory.allocation_span(), (base, len));
    memory
        .load_RAM(&[0xB8, 22, 0, 0, 0, 0xEB, 0xF9], at(CODE))
        .unwrap();
    let mut ctx = ExecCtx::cpu_only(&mut cpu, &mut memory).unwrap();
    run(&mut ctx, 2);
    assert_eq!(ctx.eax(), 22);
    put(&mut ctx, CODE + 1, &88u32.to_le_bytes());
    run(&mut ctx, 2);
    assert_eq!(ctx.eax(), 88);
}
#[test]
fn flat_partial_final_page_open_bus_and_fetch_exit_are_bounded() {
    environment(true, |ctx| {
        let end = ctx.memory.get_memory_len() as u64;
        put(ctx, end - 2, &[0x34, 0x12]);
        put(ctx, CODE, &[0xA1, 1, 0, 0x20, 0]);
        run(ctx, 1);
        assert_eq!(ctx.eax(), 0xFFFF_1234);
        put(ctx, end - 1, &[0x90]);
        ctx.set_rip(end - 1);
        ctx.invalidate_prefetch_q();
        run(ctx, 1);
        let before = ctx.icount;
        assert!(ctx.cpu_loop_n_slice(1, true, 1).is_err());
        assert_eq!(ctx.icount, before);
        assert_eq!(ctx.rip(), end);
    });
}
#[test]
fn both_maps_lapic_relocation_exits_after_retirement_and_invalidates_ram() {
    for flat in [false, true] {
        environment(flat, |ctx| {
            paging(ctx, 0x4000);
            put(ctx, 0x10020, &0xDEAD_BEEFu32.to_le_bytes());
            put(ctx, 0x4040, &0x10003u32.to_le_bytes());
            put(ctx, CODE, &[0xA1, 0x20, 0, 1, 0, 0xEB, 0xF9]);
            run(ctx, 2);
            assert_eq!(ctx.eax(), 0xDEAD_BEEF);
            let old = ctx.msr.apicbase;
            ctx.set_ecx(0x1B);
            ctx.set_eax(0x10000 | (old as u32 & 0xFFF));
            ctx.set_edx(0);
            put(ctx, 0x2100, &[0x0F, 0x30, 0xA1, 0x20, 0, 1, 0]);
            ctx.set_rip(0x2100);
            ctx.invalidate_prefetch_q();
            let before = ctx.icount;
            assert!(matches!(
                ctx.cpu_loop_n_slice(2, true, 1),
                Err(CpuError::MachineBoundaryUnavailable)
            ));
            assert_eq!(ctx.icount, before + 1);
            assert_eq!(ctx.rip(), 0x2102);
            assert!(ctx.wants_a_machine_boundary());
            assert_eq!(
                ctx.read_linear_dword(BxSegregs::Ds, BxLinAddress::new(0x10020))
                    .unwrap(),
                0
            );
        });
    }
}
#[test]
fn both_maps_smc_queue_overflow_invalidates_affected_traces() {
    for flat in [false, true] {
        environment(flat, |ctx| {
            for page in 16..86u64 {
                put(ctx, page * 4096, &[0xB8, 1, 0, 0, 0, 0xEB, 0xF9]);
                ctx.set_rip(page * 4096);
                ctx.invalidate_prefetch_q();
                run(ctx, 2);
            }
            for page in 16..86u64 {
                put(ctx, page * 4096 + 1, &9u32.to_le_bytes());
            }
            for page in 16..86u64 {
                ctx.set_rip(page * 4096);
                ctx.invalidate_prefetch_q();
                run(ctx, 2);
                assert_eq!(ctx.eax(), 9);
            }
        });
    }
}
#[test]
fn both_maps_subpage_mmio_boundary_splits_without_ram_aliasing() {
    for flat in [false, true] {
        environment(flat, |ctx| {
            paging(ctx, 0x4000);
            put(ctx, 0x607F, &[0x11, 0xAA, 0xBB, 0x44]);
            ctx.mmio.map(
                at(0x6080),
                2,
                Box::new(|addr, size| {
                    assert_eq!(size, 1);
                    if addr.get() == 0x6080 { 0x22 } else { 0x33 }
                }),
                Box::new(|_, size, _| assert_eq!(size, 1)),
            );
            put(ctx, CODE, &[0xA1, 0x7F, 0x60, 0, 0, 0xA3, 0x7F, 0x60, 0, 0]);
            run(ctx, 1);
            assert_eq!(ctx.eax(), 0x4433_2211);
            ctx.set_eax(0x8877_6655);
            run(ctx, 1);
            assert_eq!(get::<4>(ctx, 0x607F), [0x55, 0xAA, 0xBB, 0x88]);
        });
    }
}
#[test]
fn both_maps_fetch_stops_at_subpage_mmio_without_false_retirement() {
    for flat in [false, true] {
        environment(flat, |ctx| {
            put(ctx, CODE, &[0xB8, 7, 0, 0, 0, 0xEB, 0xF9]);
            ctx.mmio.map(
                at(CODE + 16),
                4,
                Box::new(|_, _| panic!("fetch consumed a data callback")),
                Box::new(|_, _, _| {}),
            );
            run(ctx, 2);
            assert_eq!(ctx.eax(), 7);
            ctx.set_rip(CODE + 16);
            ctx.invalidate_prefetch_q();
            let before = ctx.icount;
            assert!(ctx.cpu_loop_n_slice(1, true, 1).is_err());
            assert_eq!(ctx.icount, before);
            assert_eq!(ctx.rip(), CODE + 16);
        });
    }
}
#[test]
fn both_maps_typed_rep_stos_handles_unaligned_host_and_guest_addresses() {
    for flat in [false, true] {
        for long in [false, true] {
            for width in [2usize, 4, 8] {
                if width == 8 && !long {
                    continue;
                }
                for dest in [0x6000u64, 0x6001, 0x6FFE] {
                    environment(flat, |ctx| {
                        if long {
                            long_paging(ctx);
                        }
                        let code = match width {
                            2 => &[0xF3, 0x66, 0xAB][..],
                            4 => &[0xF3, 0xAB][..],
                            _ => &[0xF3, 0x48, 0xAB][..],
                        };
                        put(ctx, CODE, code);
                        ctx.set_rax(0x8877_6655_4433_2211);
                        ctx.set_rdi(dest);
                        ctx.set_rcx(4);
                        // The budget counts handler dispatches. Slow REP
                        // charges element iterations to icount; FastRep puts
                        // their surplus in ticks. Both complete this one REP.
                        // RIP and total ticks pin identical guest work here.
                        let before_ticks = ctx.cpu_ticks();
                        let before_count = ctx.icount;
                        let charged = ctx.cpu_loop_n_slice(1, true, 1).unwrap();
                        assert!((1..=4).contains(&charged));
                        assert_eq!(ctx.icount - before_count, charged);
                        assert_eq!(ctx.cpu_ticks() - before_ticks, 4);
                        assert_eq!(ctx.rip(), CODE + code.len() as u64);
                        assert_eq!(ctx.rcx(), 0);
                        assert_eq!(ctx.rdi(), dest + 4 * width as u64);
                        let mut actual = [0; 32];
                        ctx.memory
                            .read_ram(at(dest), &mut actual[..4 * width])
                            .unwrap();
                        for bytes in actual[..4 * width].chunks_exact(width) {
                            assert_eq!(bytes, &0x8877_6655_4433_2211u64.to_le_bytes()[..width]);
                        }
                    });
                }
            }
        }
    }
}

/// Manual diagnostic: construction excluded, one warm execution loan.
#[test]
#[ignore = "manual memory timing"]
fn memory_environment_perf() {
    let flat = std::env::var("MEMBENCH_BACKEND").unwrap_or_default() == "flat";
    let mode = std::env::var("MEMBENCH_MODE").unwrap_or_else(|_| "mixed".into());
    let instructions: u64 = std::env::var("MEMBENCH_INSN")
        .unwrap_or_else(|_| "200000000".into())
        .parse()
        .unwrap();
    environment_storage(
        flat,
        std::env::var_os("MEMBENCH_ALIGNED").is_some(),
        |ctx| {
            long_paging(ctx);
            let mixed = &[
                0x48, 0x01, 0xC8, 0x48, 0x31, 0xD2, 0x48, 0x89, 0x03, 0x48, 0x8B, 0x33, 0x48, 0x01,
                0xF0, 0x48, 0xFF, 0xC9, 0x75, 0xEC,
            ][..];
            let alu = &[
                0x48, 0x01, 0xC8, 0x48, 0x31, 0xD2, 0x48, 0x01, 0xC2, 0x48, 0x01, 0xD6, 0x48, 0x01,
                0xF0, 0x48, 0xFF, 0xC9, 0x75, 0xEC,
            ][..];
            let string = &[
                0xBE, 0, 0, 1, 0, 0xBF, 0, 0x10, 1, 0, 0xB9, 0x40, 0, 0, 0, 0xF3, 0xA4, 0xBF, 0,
                0x20, 1, 0, 0xB9, 0x40, 0, 0, 0, 0xB0, 0x5A, 0xF3, 0xAA, 0xEB, 0xDF,
            ][..];
            put(
                ctx,
                CODE,
                match mode.as_str() {
                    "alu" => alu,
                    "string" => string,
                    _ => mixed,
                },
            );
            ctx.set_rbx(0x10000);
            ctx.set_rcx(0x0000_FFFF_FFFF_FFFF);
            let start = std::time::Instant::now();
            run(ctx, instructions);
            println!(
                "MEMBENCH backend={} mode={} instructions={} ns={:.4}",
                if flat { "flat" } else { "pc" },
                mode,
                instructions,
                start.elapsed().as_secs_f64() * 1e9 / instructions as f64
            );
        },
    );
}
#[test]
#[ignore = "manual flat rebind timing"]
fn flat_rebind_perf() {
    let mut cpu = cpu();
    let mut bytes = alloc::vec![0;MIB];
    let mut stamps = alloc::vec![0;MIB/4096];
    let mut memory = FlatMemory::new(&mut bytes, &mut stamps).unwrap();
    memory.write(at(CODE), &[0xEB, 0xFE]).unwrap();
    let rounds = 200;
    let start = std::time::Instant::now();
    for _ in 0..rounds {
        let mut ctx = ExecCtx::flat(&mut cpu, &mut memory);
        run(&mut ctx, 1);
    }
    println!(
        "FLAT_REBIND rounds={} us_per_bind_and_run={:.3}",
        rounds,
        start.elapsed().as_secs_f64() * 1e6 / rounds as f64
    );
}
