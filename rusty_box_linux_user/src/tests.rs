use alloc::vec;
use alloc::vec::Vec;

use rusty_box::cpu::registers::{CS, EBP, EDI, EFLAGS, EIP, ESI, ESP};

use super::*;

const HEAD: u32 = 0x10000;
const CODE: u32 = 0x11000;
const HOLE: u32 = 0x12000;
const DATA: u32 = 0x13000;
const BLOCK: usize = 4096;
const PH: usize = 52;

fn put(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn elf(code: &[u8], data: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0; 2 * BLOCK + data.len()];
    bytes[..7].copy_from_slice(b"\x7fELF\x01\x01\x01");
    put(&mut bytes, 16, 2 | (3 << 16));
    put(&mut bytes, 20, 1);
    put(&mut bytes, 24, CODE);
    put(&mut bytes, 28, PH as u32);
    put(&mut bytes, 40, 52 | (32 << 16));
    put(&mut bytes, 44, 3);
    for (i, fields) in [
        [1, 0, HEAD, 0, 148, 148, 4, 4096],
        [
            1,
            BLOCK as u32,
            CODE,
            0,
            code.len() as u32,
            code.len() as u32,
            5,
            4096,
        ],
        [
            1,
            2 * BLOCK as u32,
            DATA,
            0,
            data.len() as u32,
            4096,
            6,
            4096,
        ],
    ]
    .iter()
    .enumerate()
    {
        for (offset, &word) in fields.iter().enumerate() {
            put(&mut bytes, PH + i * 32 + offset * 4, word);
        }
    }
    bytes[BLOCK..BLOCK + code.len()].copy_from_slice(code);
    bytes[2 * BLOCK..].copy_from_slice(data);
    bytes
}

fn call(number: u32, ebx: u32, ecx: u32, edx: u32) -> Vec<u8> {
    let mut code = Vec::new();
    for (mov, value) in [(0xB8, number), (0xBB, ebx), (0xB9, ecx), (0xBA, edx)] {
        code.push(mov);
        code.extend_from_slice(&value.to_le_bytes());
    }
    code.extend_from_slice(&[0xF9, 0xCD, 0x80]); // STC; INT 80h
    code
}

fn raw<T: Io>(code: &[u8], provider: T) -> LinuxI386User<T> {
    Image::parse(&elf(code, &[]))
        .unwrap()
        .launch(Launch::default(), provider)
        .unwrap()
}

fn word<I: Io>(process: &mut LinuxI386User<I>, at: u32) -> u32 {
    let mut bytes = [0; 4];
    process.read_memory(at, &mut bytes).unwrap();
    u32::from_le_bytes(bytes)
}

#[test]
fn validates_format_alignment_permissions_and_bounds() {
    let valid = elf(&[0x90], &[]);
    for (at, value, expected) in [
        (16, 3 | (3 << 16), ElfError::Format),
        (28, u32::MAX - 8, ElfError::Headers),
        (PH + 32, 3, ElfError::Runtime),
        (PH + 32 + 4, 4097, ElfError::Segment),
        (PH + 32 + 28, 3, ElfError::Segment),
        (PH + 32 + 16, 2, ElfError::Segment),
        (PH + 32 + 24, 7, ElfError::Permissions),
        (PH + 64 + 8, CODE, ElfError::Layout),
        (24, DATA, ElfError::Entry),
        (PH + 16, 147, ElfError::UnmappedHeaders),
    ] {
        let mut bytes = valid.clone();
        put(&mut bytes, at, value);
        assert_eq!(Image::parse(&bytes).err(), Some(expected), "field {at}");
    }
    assert_eq!(Image::parse(&valid[..51]).err(), Some(ElfError::Format));
    assert_eq!(Image::parse(&valid[..149]).err(), Some(ElfError::Segment));
}

#[test]
fn initializes_userspace_registers_stack_bss_and_mappings() {
    let bytes = elf(&[0x90], b"abc");
    let image = Image::parse(&bytes).unwrap();
    let mut process = image
        .launch(
            Launch {
                args: &[b"demo", b"\xFF"],
                env: &[b"X=2"],
            },
            BufferedIo::new(&[]),
        )
        .unwrap();
    let stack = process.started().initial_stack;
    assert_eq!(stack % 16, 0);
    let regs = process.cpu.regs();
    assert_eq!(regs.read(CS) & 3, 3);
    assert_eq!(regs.read(EIP), CODE);
    assert_eq!(regs.read(ESP), stack);
    assert_eq!(regs.read(EFLAGS).bits(), 0x202);
    for register in [EAX, EBX, ECX, EDX, EBP, ESI, EDI] {
        assert_eq!(regs.read(register), 0);
    }
    assert_eq!(word(&mut process, stack), 2);
    let argv0 = word(&mut process, stack + 4);
    let mut name = [0; 5];
    process.read_memory(argv0, &mut name).unwrap();
    assert_eq!(&name, b"demo\0");
    let argv1 = word(&mut process, stack + 8);
    let mut tail = [0; 2];
    process.read_memory(argv1, &mut tail).unwrap();
    assert_eq!(tail, [0xFF, 0]);
    assert_eq!(word(&mut process, stack + 12), 0);
    let env = word(&mut process, stack + 16);
    assert_eq!(word(&mut process, env).to_le_bytes(), *b"X=2\0");
    assert_eq!(word(&mut process, stack + 20), 0);
    assert_eq!(word(&mut process, stack + 24), 3);
    assert_eq!(word(&mut process, stack + 28), HEAD + PH as u32);
    assert_eq!(word(&mut process, HEAD + PH as u32), 1);
    assert_eq!(word(&mut process, DATA).to_le_bytes(), *b"abc\0");
    assert_eq!(word(&mut process, DATA + 4092), 0);
    let mut unchanged = [99; 2];
    assert!(matches!(
        process.read_memory(CODE + 4095, &mut unchanged),
        Err(MemoryError::Unmapped { address: HOLE })
    ));
    assert_eq!(unchanged, [99; 2]);
    assert!(matches!(
        process.read_memory(0x1000, &mut unchanged),
        Err(MemoryError::Unmapped { .. })
    ));
    assert!(matches!(
        process.read_memory(u32::MAX, &mut unchanged),
        Err(MemoryError::AddressOverflow)
    ));
    assert!(matches!(
        image.launch(
            Launch {
                args: &[b"a\0b"],
                env: &[]
            },
            BufferedIo::new(&[])
        ),
        Err(BuildError::Stack(StackError::InteriorNul))
    ));
    assert!(matches!(
        image.launch(
            Launch {
                args: &[&[b'x'; 65_536]],
                env: &[]
            },
            BufferedIo::new(&[])
        ),
        Err(BuildError::Stack(StackError::Full))
    ));
}

#[test]
fn interrupt_and_io_completion_are_distinct_budget_boundaries() {
    let mut code = call(SYS_WRITE, 1, DATA, 3);
    let return_ip = CODE + code.len() as u32;
    // Pass 0x103 to exit and verify that only the low status byte survives.
    code.extend([
        0x89, 0xC3, 0x81, 0xCB, 0, 1, 0, 0, 0xB8, 1, 0, 0, 0, 0xCD, 0x80,
    ]);
    let bytes = elf(&code, b"abc");
    let image = Image::parse(&bytes).unwrap();
    let mut process = image
        .launch(Launch::default(), BufferedIo::new(&[]))
        .unwrap();
    let first = process.run(6).unwrap();
    assert!(matches!(first.stop, Stop::Budget));
    assert_eq!((first.engine_count, first.syscalls), (6, 0));
    assert_eq!(process.cpu.regs().read(CS), 8);
    assert!(process.io().stdout().is_empty());
    assert_eq!(process.run(0).unwrap().engine_count, 0);
    assert!(process.io().stdout().is_empty());
    let handoff = process.run(1).unwrap();
    assert!(matches!(handoff.stop, Stop::Budget));
    assert_eq!((handoff.engine_count, handoff.syscalls), (1, 1));
    assert_eq!(process.io().stdout(), b"abc");
    assert_eq!(process.cpu.regs().read(CS), 8);
    assert_eq!(process.run(1).unwrap().syscalls, 0);
    let initial_stack = process.started().initial_stack;
    let regs = process.cpu.regs();
    assert_eq!(regs.read(CS) & 3, 3);
    assert_eq!(regs.read(EIP), return_ip);
    assert_eq!(regs.read(EAX), 3);
    assert_eq!(regs.read(EBX), 1);
    assert_eq!(regs.read(ECX), DATA);
    assert_eq!(regs.read(EDX), 3);
    assert_eq!(regs.read(ESP), initial_stack);
    assert_eq!(regs.read(EFLAGS).bits(), 0x203);
    let result = process.run(20).unwrap();
    assert!(matches!(
        result.stop,
        Stop::Exited(ProgramExited { status: 3 })
    ));
    assert!(matches!(process.run(20), Err(RunError::Finished)));
    assert_eq!(process.into_io().stdout(), b"abc");
}

#[test]
fn faults_are_reported_by_the_cpu_before_user_code_or_io_can_continue() {
    let cases: &[(&[u8], u8, Option<u32>)] = &[
        (&[0xA1, 0, 0x20, 1, 0], 14, Some(HOLE)), // mov eax,[unmapped]
        (&[0xA3, 0, 0x10, 1, 0], 14, Some(CODE)), // mov [read-only code],eax
        (&[0xB8, 0, 0x30, 1, 0, 0xFF, 0xE0], 14, Some(DATA)), // jump to NX data
        (&[0xA1, 0, 0x10, 0, 0], 14, Some(0x1000)), // read supervisor code
        (&[0xE7, 0xE0], 13, None),                // try to write supervisor port from ring 3
        (&[0xCD, 0x81], 13, None),                // vector outside this IDT
        (&[0xF0, 0xCD, 0x80], 6, None),           // LOCK prefix invalid on INT
    ];
    for &(code, vector, address) in cases {
        let mut process = raw(code, BufferedIo::new(b"unchanged"));
        let run = process.run(50).unwrap();
        let Stop::Fault(fault) = run.stop else {
            std::panic!("expected vector {vector}, got {run:?}");
        };
        assert_eq!((fault.vector, fault.address), (vector, address));
        match address {
            Some(HOLE) => assert_eq!(fault.error_code, Some(4)),
            Some(CODE) => assert_eq!(fault.error_code, Some(7)),
            Some(DATA) => assert_eq!(fault.error_code, Some(0x15)),
            Some(0x1000) => assert_eq!(fault.error_code, Some(5)),
            _ => {}
        }
        assert_eq!(
            fault.instruction,
            if address == Some(DATA) { DATA } else { CODE }
        );
        assert_eq!(run.syscalls, 0);
        assert_eq!(process.io().input_consumed(), 0);
        assert!(process.io().stdout().is_empty());
    }
}

#[test]
fn returns_guest_errors_without_consuming_external_input() {
    for (number, fd, address, len, expected) in [
        (SYS_READ, 0, HOLE, 1, -EFAULT),
        (SYS_READ, 0, CODE, 1, -EFAULT),
        (SYS_READ, 0, DATA + 4095, 2, -EFAULT),
        (SYS_READ, 0, u32::MAX, 2, -EFAULT),
        (SYS_READ, 2, DATA, 3, -EBADF),
        (SYS_WRITE, 1, HOLE, 5, -EFAULT),
        (SYS_WRITE, 0, DATA, 3, -EBADF),
        (SYS_READ, 0, HOLE, 0, 0),
    ] {
        let mut process = raw(&call(number, fd, address, len), BufferedIo::new(b"abc"));
        assert!(matches!(process.run(8).unwrap().stop, Stop::Budget));
        assert_eq!(process.cpu.regs().read(EAX) as i32, expected);
        assert_eq!(process.io().input_consumed(), 0);
        assert!(process.io().stdout().is_empty());
    }
}

#[test]
fn stops_explicitly_for_unsupported_calls_and_bounds_nonterminating_code() {
    let mut process = raw(&call(20, 0, 0, 0), BufferedIo::new(&[]));
    assert!(matches!(
        process.run(20).unwrap().stop,
        Stop::UnsupportedSystemCall { number: 20 }
    ));
    assert!(matches!(process.run(1), Err(RunError::Finished)));
    let mut looped = raw(&[0xEB, 0xFE], BufferedIo::new(&[]));
    for budget in [0, 1, 1000, 7] {
        let run = looped.run(budget).unwrap();
        assert!(matches!(run.stop, Stop::Budget));
        assert_eq!(run.engine_count, budget);
        assert_eq!(looped.cpu.regs().read(EIP), CODE);
    }
}

struct BadIo(usize);

impl Io for BadIo {
    fn read_stdin(&mut self, buffer: &mut [u8]) -> Result<usize, IoError> {
        self.0 += 1;
        Ok(buffer.len() + 1)
    }

    fn write(&mut self, _stream: Output, _buffer: &[u8]) -> Result<usize, IoError> {
        Err(IoError::BrokenPipe)
    }
}

#[test]
fn handles_explicit_io_errors_and_ends_on_provider_protocol_errors() {
    let mut write = raw(&call(SYS_WRITE, 2, DATA, 1), BadIo(0));
    assert!(matches!(write.run(8).unwrap().stop, Stop::Budget));
    assert_eq!(write.cpu.regs().read(EAX) as i32, -32);
    let mut read = raw(&call(SYS_READ, 0, DATA, 2), BadIo(0));
    assert!(matches!(
        read.run(20),
        Err(RunError::IoCount {
            offered: 2,
            transferred: 3
        })
    ));
    assert!(matches!(read.run(20), Err(RunError::Finished)));
    assert_eq!(read.io().0, 1);
    assert_eq!(word(&mut read, DATA), 0);
}

#[test]
fn buffered_streams_are_independent_and_refuse_excess_output() {
    let mut streams = BufferedIo::new(&[]).output_limit(2);
    assert_eq!(streams.write(Output::Stdout, b"abc"), Ok(2));
    assert_eq!(streams.write(Output::Stdout, b"d"), Err(IoError::NoSpace));
    assert_eq!(streams.write(Output::Stderr, b"de"), Ok(2));
    assert_eq!(streams.stdout(), b"ab");
    assert_eq!(streams.stderr(), b"de");

    let mut process = raw(&call(SYS_WRITE, 2, DATA, 1), BufferedIo::new(&[]));
    let run = process.run(8).unwrap();
    assert!(matches!(run.stop, Stop::Budget));
    assert_eq!(process.cpu.regs().read(EAX), 1);
    assert_eq!(process.io().stderr(), &[0]);
    assert!(process.io().stdout().is_empty());
}

#[test]
#[ignore = "release benchmark, requires compiled guest fixtures"]
fn measure_user_mode() {
    use std::hint::black_box;
    use std::path::PathBuf;
    use std::time::Instant;

    use rusty_box::events::{App, Emit, ExecutionRequest};

    fn time_each(count: u32, mut operation: impl FnMut()) -> f64 {
        let start = Instant::now();
        for _ in 0..count {
            operation();
        }
        start.elapsed().as_nanos() as f64 / f64::from(count)
    }

    fn report(label: &str, samples: &mut [f64]) {
        samples.sort_unstable_by(f64::total_cmp);
        std::println!(
            "{label}: {:.2} ns [{:.2}..{:.2}]",
            samples[samples.len() / 2],
            samples[0],
            samples[samples.len() - 1]
        );
    }

    let fixtures = PathBuf::from(std::env::var_os("RB_USER_FIXTURES").expect("fixture directory"));
    let compute = std::fs::read(fixtures.join("compute")).unwrap();
    let filter = std::fs::read(fixtures.join("filter")).unwrap();
    let image = Image::parse(&compute).unwrap();
    let filter = Image::parse(&filter).unwrap();
    let config = Launch {
        args: &[b"compute", b"7"],
        env: &[b"MODE=fixture"],
    };
    let mut loaded = image.launch(config, BufferedIo::new(&[])).unwrap();
    let start_event = loaded.started();
    let Stop::Exited(exit_event) = loaded.run(50_000).unwrap().stop else {
        std::panic!("compute exit");
    };
    let mut parse = [0.0; 7];
    let mut install = [0.0; 7];
    let mut launch = [0.0; 7];
    let mut compute_runtime = [0.0; 7];
    let mut filter_runtime = [0.0; 7];
    let mut events = [0.0; 7];
    let mut abi = [0.0; 7];
    let mut flat = [0.0; 7];
    let mut paged = [0.0; 7];
    let mut read_physical = [0.0; 7];
    let mut read_guest = [0.0; 7];

    // inc dword ptr [DATA]; jmp back to it. The same bytes and addresses run
    // over the CPU-only dense backing and the process's paged backing.
    let loop_code = [0xFF, 0x05, 0x00, 0x30, 0x01, 0, 0xEB, 0xF8];
    let mut user = raw(&loop_code, BufferedIo::new(&[]));
    let mut direct = CpuEmulator::builder()
        .ram((DATA + 4096) as usize)
        .entry(CODE)
        .build()
        .unwrap();
    direct
        .memory()
        .write_phys(memory::physical(CODE), &loop_code)
        .unwrap();
    const COUNT: u64 = 2_000_000;
    assert_eq!(direct.run(COUNT).instructions, COUNT);
    assert_eq!(user.run(COUNT).unwrap().engine_count, COUNT);

    // read(0,...,0) avoids buffer copies and callbacks. Measure the full kernel
    // gate, host handoff, ABI selection and IRET together with this user loop.
    let mut trap = raw(
        &[
            0xB8, 3, 0, 0, 0, 0x31, 0xDB, 0x31, 0xD2, 0xCD, 0x80, 0xEB, 0xF3,
        ],
        BufferedIo::new(&[]),
    );
    assert!(trap.run(10_000).unwrap().syscalls > 0);
    let input = [b'x'; 4096];
    for round in 0..7 {
        parse[round] = time_each(10_000, || {
            black_box(Image::parse(black_box(&compute)).unwrap());
        });
        install[round] = time_each(10_000, || {
            loaded
                .space
                .load(black_box(&mut loaded.cpu), &image)
                .unwrap()
        });
        launch[round] = time_each(25, || {
            black_box(image.launch(config, BufferedIo::new(&[])).unwrap());
        });
        let mut compute_process = image.launch(config, BufferedIo::new(&[])).unwrap();
        let start = Instant::now();
        let run = compute_process.run(50_000).unwrap();
        compute_runtime[round] = start.elapsed().as_nanos() as f64;
        assert!(matches!(
            run.stop,
            Stop::Exited(ProgramExited { status: 0 })
        ));
        let mut filter_process = filter
            .launch(Launch::default(), BufferedIo::new(&input))
            .unwrap();
        let start = Instant::now();
        let run = filter_process.run(1_000_000).unwrap();
        filter_runtime[round] = start.elapsed().as_nanos() as f64 / input.len() as f64;
        assert!(matches!(
            run.stop,
            Stop::Exited(ProgramExited { status: 0 })
        ));
        assert_eq!(filter_process.io().stdout(), &[b'X'; 4096]);

        let measure_direct = |direct: &mut CpuEmulator| {
            let start = Instant::now();
            let run = direct.run(COUNT);
            let elapsed = start.elapsed().as_nanos() as f64;
            assert_eq!(run.instructions, COUNT);
            elapsed / COUNT as f64
        };
        let measure_user = |user: &mut LinuxI386User<_>| {
            let start = Instant::now();
            let run = user.run(COUNT).unwrap();
            let elapsed = start.elapsed().as_nanos() as f64;
            assert_eq!(run.engine_count, COUNT);
            elapsed / COUNT as f64
        };
        if round % 2 == 0 {
            flat[round] = measure_direct(&mut direct);
            paged[round] = measure_user(&mut user);
        } else {
            paged[round] = measure_user(&mut user);
            flat[round] = measure_direct(&mut direct);
        }
        let mut out = [0; 64];
        read_physical[round] = time_each(50_000, || {
            direct
                .memory()
                .read_phys(black_box(memory::physical(DATA)), black_box(&mut out))
                .unwrap()
        });
        read_guest[round] = time_each(50_000, || {
            user.read_memory(black_box(DATA), black_box(&mut out))
                .unwrap()
        });

        let start = Instant::now();
        let run = trap.run(350_000).unwrap();
        abi[round] = start.elapsed().as_nanos() as f64 / run.syscalls as f64;
        assert!(matches!(run.stop, Stop::Budget));
        assert_eq!(run.engine_count, 350_000);
        assert_eq!(run.syscalls, 50_000);

        let mut app = App::<u64>::new()
            .on(|event: &ProgramStarted, state: &mut u64| *state += u64::from(event.entry))
            .on(|event: &ProgramExited, state: &mut u64| *state += u64::from(event.status))
            .with_state(0);
        events[round] = time_each(100_000, || {
            assert_eq!(
                black_box(&mut app).emit(black_box(&start_event), &mut ()),
                Ok(ExecutionRequest::None)
            );
            assert_eq!(
                black_box(&mut app).emit(black_box(&exit_event), &mut ()),
                Ok(ExecutionRequest::None)
            );
        });
        assert_eq!(
            app.into_state(),
            100_000 * (u64::from(start_event.entry) + u64::from(exit_event.status))
        );
    }
    let mut direct_data = [0; 4];
    direct
        .memory()
        .read_phys(memory::physical(DATA), &mut direct_data)
        .unwrap();
    assert_eq!(u32::from_le_bytes(direct_data), word(&mut user, DATA));
    assert_eq!(u32::from_le_bytes(direct_data), (COUNT / 2 * 8) as u32);

    for (label, mut samples) in [
        ("ELF validation per compute ELF", parse),
        ("ELF install into allocated RAM per compute ELF", install),
        ("fresh launch from parsed compute ELF", launch),
        ("compute execution from ready user entry", compute_runtime),
        (
            "filter execution per input byte (4096 bytes)",
            filter_runtime,
        ),
        ("CPU-only unpaged loop per engine count", flat),
        ("user-mode paged loop per engine count", paged),
        ("checked physical read per 64 bytes", read_physical),
        ("checked user read per 64 bytes", read_guest),
        ("int80 read(0 bytes) including user loop per call", abi),
        ("optional typed application start and exit pair", events),
    ] {
        report(label, &mut samples);
    }
}
