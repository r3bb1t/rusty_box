//! This integration crate owns its memory and decides where it is mapped.
#![cfg(feature = "alloc")]

use rusty_box::config::BxPhyAddress;
use rusty_box::cpu::builder::BxCpuBuilder;
use rusty_box::cpu::memory_session::{
    MemorySession, PhysicalOperation, PhysicalRequest, PhysicalResponse,
};
use rusty_box::cpu::{BxCpuC, CpuError, ResetReason};
use rusty_box::memory::mapped::{MappedMemory, MemoryMapError, PhysicalRegion, RegionKind};

const PAGE: usize = 4096;
const RESET: u64 = 0xFFFF_F000;
const fn region(guest: u64, slot: usize, kind: RegionKind) -> PhysicalRegion {
    PhysicalRegion {
        guest_start: BxPhyAddress::new(guest),
        arena_offset: slot * PAGE,
        len: PAGE,
        kind,
    }
}

/// This backend packs discontiguous guest pages in caller-owned storage.
/// Its reset ROM lives at the architectural reset address, independently of
/// chipset ROM allocation and the low-memory code mapping.
struct SparseBoard<const N: usize> {
    arena: Vec<u8>,
    stamps: [u32; N],
}

impl<const N: usize> SparseBoard<N> {
    fn new(reset_target: u16) -> Self {
        let mut board = Self {
            arena: vec![0; N * PAGE],
            stamps: [0; N],
        };
        let [lo, hi] = reset_target.to_le_bytes();
        board.put(N - 1, 0xFF0, &[0xEA, lo, hi, 0, 0]); // jmp 0000:target
        board
    }

    fn put(&mut self, slot: usize, offset: usize, data: &[u8]) {
        let start = slot * PAGE + offset;
        self.arena[start..start + data.len()].copy_from_slice(data);
    }

    fn memory<'a>(&'a mut self, map: &'a [PhysicalRegion]) -> MappedMemory<'a> {
        MappedMemory::new(&mut self.arena, &mut self.stamps, map).unwrap()
    }
}

fn cpu() -> Box<BxCpuC> {
    let mut cpu = BxCpuBuilder::new().build().unwrap();
    cpu.reset(ResetReason::Hardware);
    cpu
}

const DEVICE_PAGE: BxPhyAddress = BxPhyAddress::new(0x3000);
const DEVICE_MAP: [PhysicalRegion; 4] = [
    region(0x1000, 0, RegionKind::Ram),
    region(0x2000, 1, RegionKind::Ram),
    region(0x4000, 2, RegionKind::Ram),
    region(RESET, 3, RegionKind::Rom),
];

#[test]
fn external_scalar_split_store_preflights_and_retires_once() {
    let mut board = SparseBoard::<4>::new(0x1000);
    board.put(0, 0, &[0xB8, 0xCD, 0xAB, 0xA3, 0xFF, 0x2F]); // mov ax,0xabcd; mov [0x2fff],ax
    board.put(1, PAGE - 1, &[0x77]);
    let mut cpu = cpu();
    let request = PhysicalRequest {
        address: DEVICE_PAGE,
        operation: PhysicalOperation::Write {
            len: 1,
            value: 0xAB,
        },
    };
    {
        let mut memory = board.memory(&DEVICE_MAP);
        let mut session = MemorySession::new(&mut cpu, memory.loan());
        assert_eq!(session.run(2).unwrap(), 2);
        assert!(matches!(
            session.step_with_external_page(DEVICE_PAGE, |actual| {
                assert_eq!(actual, request);
                PhysicalResponse::Pending
            }),
            Err(CpuError::PhysicalAccessPending(actual)) if actual == request
        ));
        assert!(matches!(
            session.step_with_external_page(DEVICE_PAGE, |_| PhysicalResponse::Failed(41)),
            Err(CpuError::PhysicalBusFailure { request: actual, code: 41 }) if actual == request
        ));
        assert_eq!(session.cpu().rip(), 0x1003);
        assert_eq!(session.cpu().get_cr2_val(), 0);
    }
    assert_eq!(board.arena[2 * PAGE - 1], 0x77);
    let mut committed = 0;
    {
        let mut memory = board.memory(&DEVICE_MAP);
        let mut session = MemorySession::new(&mut cpu, memory.loan());
        let retired = session.step_with_external_page(DEVICE_PAGE, |actual| {
            assert_eq!(actual, request);
            committed += 1;
            PhysicalResponse::Complete(0)
        });
        assert_eq!(retired.unwrap(), 1);
        assert_eq!(session.cpu().rip(), 0x1006);
    }
    assert_eq!(committed, 1);
    assert_eq!(board.arena[2 * PAGE - 1], 0xCD);
}

#[test]
fn external_scalar_load_restarts_and_split_fragments_keep_byte_order() {
    let mut board = SparseBoard::<4>::new(0x1000);
    board.put(
        0,
        0,
        &[
            0x66, 0xA1, 0xFD, 0x3F, // mov eax,[0x3ffd] (three device bytes, one RAM byte)
            0x66, 0xA3, 0xFD, 0x3F, // mov [0x3ffd],eax
            0xA3, 0x04, 0x30, // mov [0x3004],ax (contained word)
            0xA0, 0x04, 0x30, // mov al,[0x3004] (contained byte)
        ],
    );
    board.put(2, 0, &[0x44]);
    let mut cpu = cpu();
    let mut memory = board.memory(&DEVICE_MAP);
    let mut session = MemorySession::new(&mut cpu, memory.loan());
    session.run(1).unwrap();
    let original_rax = session.cpu().rax();
    let read = PhysicalRequest {
        address: BxPhyAddress::new(0x3FFD),
        operation: PhysicalOperation::Read { len: 3 },
    };
    assert!(matches!(
        session.step_with_external_page(DEVICE_PAGE, |_| PhysicalResponse::Pending),
        Err(CpuError::PhysicalAccessPending(request)) if request == read
    ));
    assert_eq!(session.cpu().rip(), 0x1000);
    assert_eq!(session.cpu().rax(), original_rax);
    assert_eq!(
        session
            .step_with_external_page(DEVICE_PAGE, |request| {
                assert_eq!(request, read);
                PhysicalResponse::Complete(0xFF33_2211) // high byte must come from RAM
            })
            .unwrap(),
        1
    );
    assert_eq!(session.cpu().rax(), 0x4433_2211);
    assert_eq!(
        session
            .step_with_external_page(DEVICE_PAGE, |request| {
                assert_eq!(request.address, read.address);
                assert_eq!(
                    request.operation,
                    PhysicalOperation::Write {
                        len: 3,
                        value: 0x33_2211
                    }
                );
                PhysicalResponse::Complete(0)
            })
            .unwrap(),
        1
    );
    assert_eq!(
        session
            .step_with_external_page(DEVICE_PAGE, |request| {
                assert_eq!(request.address, BxPhyAddress::new(0x3004));
                assert_eq!(
                    request.operation,
                    PhysicalOperation::Write {
                        len: 2,
                        value: 0x2211
                    }
                );
                PhysicalResponse::Complete(0)
            })
            .unwrap(),
        1
    );
    assert_eq!(
        session
            .step_with_external_page(DEVICE_PAGE, |request| {
                assert_eq!(request.address, BxPhyAddress::new(0x3004));
                assert_eq!(request.operation, PhysicalOperation::Read { len: 1 });
                PhysicalResponse::Complete(0xAA)
            })
            .unwrap(),
        1
    );
    assert_eq!(session.cpu().rax(), 0x4433_22AA);
}

#[test]
fn external_scalar_guard_distinguishes_guest_fault_fetch_and_unsupported_opcodes() {
    let mut board = SparseBoard::<4>::new(0x1000);
    board.put(0, 0, &[0xA1, 0xFF, 0xFF]); // word read beyond the real-mode DS limit
    board.put(0, 0x10, &[0xF7, 0x16, 0, 0x30]); // not word [0x3000], an RMW
    board.put(0, 0x20, &[0xF3, 0xAA]); // rep stosb
    board.put(0, 0x30, &[0xEA, 0, 0x30, 0, 0]); // jump to the hole
    // Select each entry with a fresh CPU and a patched reset ROM far jump.
    for (start, expected) in [(0x1000, 0), (0x1010, 1), (0x1020, 1), (0x1030, 2)] {
        let mut cpu = cpu();
        let [lo, hi] = (start as u16).to_le_bytes();
        board.put(3, 0xFF0, &[0xEA, lo, hi, 0, 0]);
        let mut memory = board.memory(&DEVICE_MAP);
        let mut session = MemorySession::new(&mut cpu, memory.loan());
        session.run(if expected == 2 { 2 } else { 1 }).unwrap();
        let rip = session.cpu().rip();
        let result = session.step_with_external_page(
            if expected == 0 {
                BxPhyAddress::new(0xF000)
            } else {
                DEVICE_PAGE
            },
            |_| panic!("preflight must stop before calling the device"),
        );
        match expected {
            0 => assert!(
                matches!(result, Err(CpuError::GuestFaultBeforeDelivery { .. })),
                "{result:?}"
            ),
            1 => assert!(
                matches!(result, Err(CpuError::UnsupportedCpuOperation { .. })),
                "{result:?}"
            ),
            _ => assert!(
                matches!(result, Err(CpuError::VetoedDirectRead { p_addr: 0x3000 })),
                "{result:?}"
            ),
        }
        assert_eq!(session.cpu().rip(), rip);
        assert_eq!(session.cpu().get_cr2_val(), 0);
    }
}

#[test]
fn sparse_rom_and_open_bus_compose_with_real_loads_and_stores() {
    let mut board = SparseBoard::<4>::new(0x1000);
    let map = [
        region(0x1000, 2, RegionKind::Ram),
        region(0x2000, 1, RegionKind::Ram),
        region(0x3000, 0, RegionKind::Rom),
        region(RESET, 3, RegionKind::Rom),
    ];
    board.put(0, 0, &[0x12]);
    board.put(0, 0xFFF, &[0x34]);
    board.put(
        2,
        0,
        &[
            0xB8, 0xCD, 0xAB, // mov ax,0xabcd
            0xA3, 0xFF, 0x2F, // mov [0x2fff],ax (RAM then ROM)
            0xA1, 0xFF, 0x2F, // mov ax,[0x2fff]
            0x89, 0xC3, // mov bx,ax
            0xA1, 0xFF, 0x3F, // mov ax,[0x3fff] (ROM then hole)
            0xA3, 0x00, 0x40, // mov [0x4000],ax (discard)
            0xF4, // hlt
        ],
    );
    let mut cpu = cpu();
    {
        let mut memory = board.memory(&map);
        let mut execution = MemorySession::new(&mut cpu, memory.loan());
        assert_eq!(execution.run(8).unwrap(), 8);
        assert_eq!(execution.cpu().rbx(), 0x12CD);
        assert_eq!(execution.cpu().rax(), 0xFF34);
    }
    assert_eq!(board.arena[PAGE + 0xFFF], 0xCD);
    assert_eq!(board.arena[0], 0x12);
    assert_eq!(cpu.get_cr2_val(), 0);
}

#[test]
fn split_instruction_fetch_retries_from_the_start_after_mapping_repair() {
    let mut board = SparseBoard::<3>::new(0x1FFF);
    let incomplete = [
        region(0x1000, 0, RegionKind::Ram),
        region(RESET, 2, RegionKind::Rom),
    ];
    board.put(0, 0xFFF, &[0xB8]); // mov ax,imm16 straddles the boundary
    let mut cpu = cpu();
    cpu.set_rax(0xBEEF);
    {
        let mut memory = board.memory(&incomplete);
        let mut execution = MemorySession::new(&mut cpu, memory.loan());
        assert_eq!(execution.run(1).unwrap(), 1); // completed reset jump
        for _ in 0..2 {
            assert!(matches!(
                execution.run(1),
                Err(CpuError::VetoedDirectRead { p_addr: 0x2000 })
            ));
            assert_eq!(execution.cpu().rip(), 0x1FFF);
            assert_eq!(execution.cpu().rax(), 0xBEEF);
            assert_eq!(execution.cpu().get_cr2_val(), 0);
        }
    }
    board.put(1, 0, &[0x78, 0x56, 0xF4]);
    let complete = [
        incomplete[0],
        region(0x2000, 1, RegionKind::Rom),
        incomplete[1],
    ];
    let mut memory = board.memory(&complete);
    let mut execution = MemorySession::new(&mut cpu, memory.loan());
    assert_eq!(execution.run(1).unwrap(), 1);
    assert_eq!(execution.cpu().rax(), 0x5678);
    assert_eq!(execution.cpu().rip(), 0x2002);
}

#[test]
fn sparse_code_stamps_cover_guest_and_host_writes_and_same_arena_remaps() {
    let mut board = SparseBoard::<3>::new(0x1000);
    let mut map = [
        region(0x1000, 1, RegionKind::Ram),
        region(RESET, 2, RegionKind::Rom),
    ];
    // mov byte [0x1006],0x22; mov ax,0x0011; jmp 0x1005
    board.put(
        1,
        0,
        &[0xC6, 0x06, 0x06, 0x10, 0x22, 0xB8, 0x11, 0, 0xEB, 0xFB],
    );
    board.put(0, 5, &[0xB8, 0x44, 0, 0xEB, 0xFB]);
    let mut cpu = cpu();
    {
        let mut memory = board.memory(&map);
        let mut execution = MemorySession::new(&mut cpu, memory.loan());
        assert_eq!(execution.run(4).unwrap(), 4);
        assert_eq!(execution.cpu().rax(), 0x22);
        execution
            .write_ram(BxPhyAddress::new(0x1006), &[0x33])
            .unwrap();
        assert_eq!(execution.run(2).unwrap(), 2);
        assert_eq!(execution.cpu().rax(), 0x33);
        assert!(
            execution
                .write_ram(BxPhyAddress::new(RESET + 0xFFF), &[1, 2])
                .is_err()
        );
    }
    map[0].arena_offset = 0;
    let mut memory = board.memory(&map);
    let mut execution = MemorySession::new(&mut cpu, memory.loan());
    assert_eq!(execution.run(2).unwrap(), 2);
    assert_eq!(execution.cpu().rax(), 0x44);
}

#[test]
fn mapped_memory_rejects_aliases_and_out_of_bounds_offsets() {
    let mut arena = [0; 2 * PAGE];
    let mut stamps = [0; 2];
    let first = region(0x1000, 0, RegionKind::Ram);
    let alias = [first, region(0x2000, 0, RegionKind::Rom)];
    assert!(matches!(
        MappedMemory::new(&mut arena, &mut stamps, &alias),
        Err(MemoryMapError::BackingOverlap { index: 1 })
    ));
    let overlap = [first, region(0x1000, 1, RegionKind::Ram)];
    assert!(matches!(
        MappedMemory::new(&mut arena, &mut stamps, &overlap),
        Err(MemoryMapError::GuestOrder { index: 1 })
    ));
    let out = [region(0x1000, 2, RegionKind::Ram)];
    assert!(matches!(
        MappedMemory::new(&mut arena, &mut stamps, &out),
        Err(MemoryMapError::InvalidRegion { index: 0 })
    ));
}

/// Firmware installs its own GDT, IDT and page tables through real guest
/// instructions. The second destination page is physically backed but
/// initially absent from the guest page tables.
fn paged_board(operation: &[u8]) -> SparseBoard<9> {
    let mut board = SparseBoard::new(0x1000);
    board.put(
        0,
        0,
        &[
            0x0F, 0x01, 0x16, 0xF0, 0x11, // lgdt [0x11f0]
            0x0F, 0x20, 0xC0, // mov eax,cr0
            0x83, 0xC8, 0x01, // or ax,1
            0x0F, 0x22, 0xC0, // mov cr0,eax
            0xEA, 0x40, 0x10, 0x08, 0x00, // jmp 8:0x1040
        ],
    );
    board.put(
        0,
        0x40,
        &[
            0x66, 0xB8, 0x10, 0x00, // mov ax,0x10
            0x8E, 0xD8, // mov ds,ax
            0x8E, 0xC0, // mov es,ax
            0x8E, 0xD0, // mov ss,ax
            0xBC, 0xFC, 0x3F, 0, 0, // mov esp,0x3ffc
            0x0F, 0x01, 0x1D, 0xF6, 0x11, 0, 0, // lidt [0x11f6]
            0xB8, 0x00, 0x40, 0, 0, // mov eax,0x4000
            0x0F, 0x22, 0xD8, // mov cr3,eax
            0x0F, 0x20, 0xC0, // mov eax,cr0
            0x0D, 0x00, 0, 0, 0x80, // or eax,0x80000000
            0x0F, 0x22, 0xC0, // mov cr0,eax
            0xE9, 0x92, 0x00, 0, 0, // jmp 0x1100 (relative to 0x106e)
        ],
    );
    board.put(0, 0x100, operation);
    board.put(
        0,
        0x180,
        &[
            0x0F, 0x20, 0xD0, // mov eax,cr2
            0x8B, 0x1C, 0x24, // mov ebx,[esp] (#PF error)
            0xC7, 0x05, 0x20, 0x50, 0, 0, 3, 0x80, 0, 0, // mov dword [0x5020],0x8003
            0x83, 0xC4, 0x04, // add esp,4
            0xCF, // iretd
        ],
    );
    board.put(0, 0x1F0, &[23, 0, 0x00, 0x12, 0, 0]); // GDTR
    board.put(0, 0x1F6, &[119, 0, 0x40, 0x12, 0, 0]); // IDTR
    board.put(0, 0x208, &0x00CF_9B00_0000_FFFFu64.to_le_bytes());
    board.put(0, 0x210, &0x00CF_9300_0000_FFFFu64.to_le_bytes());
    board.put(0, 0x240 + 14 * 8, &[0x80, 0x11, 8, 0, 0, 0x8E, 0, 0]); // #PF gate
    board.put(1, 0, &0x5003u32.to_le_bytes());
    for page in [1u32, 3, 4, 5, 6, 7] {
        board.put(2, page as usize * 4, &((page << 12) | 3).to_le_bytes());
    }
    board
}

const PAGED_MAP: [PhysicalRegion; 8] = [
    region(0x1000, 0, RegionKind::Ram),
    region(0x3000, 7, RegionKind::Ram),
    region(0x4000, 1, RegionKind::Ram),
    region(0x5000, 2, RegionKind::Ram),
    region(0x6000, 5, RegionKind::Ram),
    region(0x7000, 6, RegionKind::Ram),
    region(0x8000, 3, RegionKind::Ram),
    region(RESET, 8, RegionKind::Rom),
];

fn run_to(execution: &mut MemorySession<'_>, rip: u64) {
    for _ in 0..40 {
        if execution.cpu().rip() == rip {
            break;
        }
        execution.run(1).unwrap();
    }
    assert_eq!(execution.cpu().rip(), rip);
}

#[test]
fn paged_sparse_warm_dtlb_store_invalidates_code_at_its_guest_address() {
    let mut board = paged_board(&[
        0xC6, 0x05, 1, 0x70, 0, 0, 0x11, // mov byte [0x7001],0x11 (warms writable DTLB)
        0x31, 0xC0, // xor eax,eax
        0xE8, 0xF2, 0x5E, 0, 0, // call 0x7000 (caches old code)
        0xC6, 0x05, 1, 0x70, 0, 0, 0x22, // mov byte [0x7001],0x22 (warm DTLB hit)
        0xE8, 0xE6, 0x5E, 0, 0, // call 0x7000 (must decode new code)
        0xF4,
    ]);
    // Physical page 7 is in arena slot 6. Looking up stamp 7 would miss.
    board.put(6, 0, &[0xB8, 0x11, 0, 0, 0, 0xC3]); // mov eax,0x11; ret
    let mut cpu = cpu();
    let mut memory = board.memory(&PAGED_MAP);
    let mut session = MemorySession::new(&mut cpu, memory.loan());
    run_to(&mut session, 0x1100);
    assert_eq!(session.run(5).unwrap(), 5);
    assert_eq!(session.cpu().rax(), 0x11);
    assert_eq!(session.run(4).unwrap(), 4);
    assert_eq!(session.cpu().rax(), 0x22);
}

#[test]
fn guarded_scalar_step_refuses_paged_mode_before_touching_the_operand() {
    let mut board = paged_board(&[0xA1, 0, 0x80, 0, 0]); // mov eax,[0x8000], absent PTE
    let mut cpu = cpu();
    let mut memory = board.memory(&PAGED_MAP);
    let mut session = MemorySession::new(&mut cpu, memory.loan());
    run_to(&mut session, 0x1100);
    assert!(matches!(
        session.step_with_external_page(BxPhyAddress::new(0x9000), |_| panic!("paged step")),
        Err(CpuError::UnsupportedCpuOperation { .. })
    ));
    assert_eq!(session.cpu().rip(), 0x1100);
    assert_eq!(session.cpu().get_cr2_val(), 0);
}

#[test]
fn guest_page_fault_restarts_rep_from_the_remaining_elements() {
    let mut board = paged_board(&[
        0xBE, 0x00, 0x60, 0, 0, // mov esi,0x6000
        0xBF, 0xFE, 0x7F, 0, 0, // mov edi,0x7ffe
        0xB9, 0x04, 0, 0, 0, // mov ecx,4
        0xF3, 0xA4, // rep movsb
        0xF4, // hlt
    ]);
    board.put(5, 0, &[1, 2, 3, 4]);
    let mut cpu = cpu();
    {
        let mut memory = board.memory(&PAGED_MAP);
        let mut execution = MemorySession::new(&mut cpu, memory.loan());
        run_to(&mut execution, 0x110F);
        execution.run(1).unwrap();
        assert_eq!(execution.cpu().rip(), 0x1180);
        assert_eq!(execution.cpu().get_cr2_val(), 0x8000);
        assert_eq!(execution.cpu().rcx(), 2);
        assert_eq!(execution.cpu().rdi(), 0x8000);
        assert_eq!(execution.cpu().rsi(), 0x6002);
        assert_eq!(execution.cpu().rsp(), 0x3FEC);
        // Change a byte in the already completed portion. IRET must resume
        // with guest-saved indices and leave this byte alone.
        execution
            .write_ram(BxPhyAddress::new(0x7FFE), &[0x42])
            .unwrap();
        execution.run(12).unwrap();
        assert_eq!(execution.cpu().rip(), 0x1112);
        assert_eq!(execution.cpu().rbx(), 2); // Write to a nonpresent PTE
        assert_eq!(execution.cpu().rcx(), 0);
        assert_eq!(execution.cpu().rsp(), 0x3FFC);
    }
    assert_eq!(&board.arena[7 * PAGE - 2..7 * PAGE], &[0x42, 2]);
    assert_eq!(&board.arena[3 * PAGE..3 * PAGE + 2], &[3, 4]);
    assert_eq!(&board.arena[2 * PAGE + 32..2 * PAGE + 34], &[0x63, 0x80]); // A/D
}

#[test]
fn split_scalar_store_does_not_commit_before_a_guest_page_fault() {
    let mut board = paged_board(&[
        0xB8, 0x11, 0x22, 0x33, 0x44, // mov eax,0x44332211
        0xA3, 0xFE, 0x7F, 0, 0,    // mov [0x7ffe],eax
        0xF4, // hlt
    ]);
    board.put(6, PAGE - 2, &[0xAA, 0xBB]);
    board.put(3, 0, &[0xCC, 0xDD]);
    // This handler repairs the page table without changing the store value.
    board.put(
        0,
        0x180,
        &[
            0xC7, 0x05, 0x20, 0x50, 0, 0, 3, 0x80, 0, 0, // repair PTE
            0x83, 0xC4, 0x04, // add esp,4
            0xCF, // iretd
        ],
    );
    let mut cpu = cpu();
    {
        let mut memory = board.memory(&PAGED_MAP);
        let mut execution = MemorySession::new(&mut cpu, memory.loan());
        run_to(&mut execution, 0x1105);
        execution.run(1).unwrap();
        assert_eq!(execution.cpu().rip(), 0x1180);
        assert_eq!(execution.cpu().get_cr2_val(), 0x8000);
    }
    assert_eq!(&board.arena[7 * PAGE - 2..7 * PAGE], &[0xAA, 0xBB]);
    assert_eq!(&board.arena[3 * PAGE..3 * PAGE + 2], &[0xCC, 0xDD]);
    {
        let mut memory = board.memory(&PAGED_MAP);
        let mut execution = MemorySession::new(&mut cpu, memory.loan());
        execution.run(12).unwrap();
        assert_eq!(execution.cpu().rip(), 0x110B);
    }
    assert_eq!(&board.arena[7 * PAGE - 2..7 * PAGE], &[0x11, 0x22]);
    assert_eq!(&board.arena[3 * PAGE..3 * PAGE + 2], &[0x33, 0x44]);
}

// Opt-in benchmark for the paged data path, with equality of guest code and
// translations across dense and sparse placements. The reset/boot and a warm
// run are outside the timed interval.
#[test]
#[ignore = "manual performance workload"]
fn timed_borrowed_memory() {
    let mut board = paged_board(&[
        0x40, // inc eax
        0xA3, 0, 0x70, 0, 0, // mov [0x7000],eax
        0x8B, 0x1D, 0, 0x70, 0, 0, // mov ebx,[0x7000]
        0xEB, 0xF2, // jmp 0x1100
    ]);
    let mut cpu = cpu();
    {
        let mut memory = board.memory(&PAGED_MAP);
        let mut execution = MemorySession::new(&mut cpu, memory.loan());
        run_to(&mut execution, 0x1100);
    }
    let count = std::env::var("BORROWED_BENCH_COUNT")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(120_000_000);
    let measure = |mut execution: MemorySession<'_>| {
        execution.run(4096).unwrap();
        let start = std::time::Instant::now();
        let executed = execution.run(count).unwrap();
        let elapsed = start.elapsed();
        assert_eq!(executed, count);
        println!(
            "borrowed-memory: {executed} counts in {:.3}s ({:.3} ns/count)",
            elapsed.as_secs_f64(),
            elapsed.as_secs_f64() * 1e9 / executed as f64
        );
    };
    if std::env::var("BORROWED_BENCH_MAP").as_deref() == Ok("dense") {
        let mut dense = vec![0; 9 * PAGE];
        let mut stamps = [0; 9];
        for region in &PAGED_MAP[..PAGED_MAP.len() - 1] {
            let start = region.guest_start.get() as usize;
            dense[start..start + PAGE]
                .copy_from_slice(&board.arena[region.arena_offset..region.arena_offset + PAGE]);
        }
        let mut memory = rusty_box::memory::flat::FlatMemory::new(&mut dense, &mut stamps).unwrap();
        measure(MemorySession::new(&mut cpu, memory.loan()));
    } else {
        let mut memory = board.memory(&PAGED_MAP);
        measure(MemorySession::new(&mut cpu, memory.loan()));
    }
}
