use criterion::{black_box, criterion_group, criterion_main, Criterion};
use rusty_box_decoder::{fetch_decode32, fetch_decode64};

/// Representative x86 instruction byte sequences for benchmarking.

// Real-mode / protected-mode 32-bit instructions
const INSTR_32BIT: &[&[u8]] = &[
    &[0x90],                         // NOP
    &[0x89, 0xE5],                   // MOV EBP, ESP
    &[0x83, 0xEC, 0x10],             // SUB ESP, 0x10
    &[0x8B, 0x45, 0x08],             // MOV EAX, [EBP+8]
    &[0x01, 0xC8],                   // ADD EAX, ECX
    &[0x39, 0xC3],                   // CMP EBX, EAX
    &[0x0F, 0x84, 0x10, 0x00, 0x00, 0x00], // JE +0x10
    &[0xE8, 0x00, 0x10, 0x00, 0x00], // CALL +0x1000
    &[0xC3],                         // RET
    &[0x50],                         // PUSH EAX
    &[0x58],                         // POP EAX
    &[0xF3, 0xA4],                   // REP MOVSB
    &[0x0F, 0xB6, 0xC0],             // MOVZX EAX, AL
    &[0x0F, 0xBE, 0x45, 0xFC],       // MOVSX EAX, BYTE [EBP-4]
    &[0xC1, 0xE0, 0x03],             // SHL EAX, 3
    &[0x0F, 0xAF, 0xC1],             // IMUL EAX, ECX
];

// 64-bit long mode instructions
const INSTR_64BIT: &[&[u8]] = &[
    &[0x90],                             // NOP
    &[0x48, 0x89, 0xE5],                 // MOV RBP, RSP
    &[0x48, 0x83, 0xEC, 0x20],           // SUB RSP, 0x20
    &[0x48, 0x8B, 0x45, 0x08],           // MOV RAX, [RBP+8]
    &[0x48, 0x01, 0xC8],                 // ADD RAX, RCX
    &[0x48, 0x39, 0xC3],                 // CMP RBX, RAX
    &[0x0F, 0x84, 0x10, 0x00, 0x00, 0x00], // JE +0x10
    &[0xE8, 0x00, 0x10, 0x00, 0x00],     // CALL +0x1000
    &[0xC3],                             // RET
    &[0x50],                             // PUSH RAX
    &[0x58],                             // POP RAX
    &[0x48, 0x63, 0xC1],                 // MOVSXD RAX, ECX
    &[0x0F, 0xB6, 0xC0],                 // MOVZX EAX, AL
    &[0x48, 0xC1, 0xE0, 0x03],           // SHL RAX, 3
    &[0x48, 0x0F, 0xAF, 0xC1],           // IMUL RAX, RCX
    &[0x0F, 0x1F, 0x44, 0x00, 0x00],     // NOP DWORD [RAX+RAX+0]
];

fn bench_decode_32bit(c: &mut Criterion) {
    c.bench_function("decode_32bit_mix", |b| {
        b.iter(|| {
            for instr_bytes in INSTR_32BIT {
                let _ = black_box(fetch_decode32(black_box(instr_bytes), true));
            }
        })
    });
}

fn bench_decode_64bit(c: &mut Criterion) {
    c.bench_function("decode_64bit_mix", |b| {
        b.iter(|| {
            for instr_bytes in INSTR_64BIT {
                let _ = black_box(fetch_decode64(black_box(instr_bytes)));
            }
        })
    });
}

fn bench_decode_single_nop(c: &mut Criterion) {
    let nop = &[0x90u8];
    c.bench_function("decode_single_nop_64", |b| {
        b.iter(|| {
            let _ = black_box(fetch_decode64(black_box(nop)));
        })
    });
}

fn bench_decode_sse(c: &mut Criterion) {
    // SSE instructions common in Alpine/libcrypto workloads
    let sse_instrs: &[&[u8]] = &[
        &[0x66, 0x0F, 0xEF, 0xC0],       // PXOR XMM0, XMM0
        &[0x66, 0x0F, 0x6F, 0xC1],       // MOVDQA XMM0, XMM1
        &[0xF3, 0x0F, 0x6F, 0x03],       // MOVDQU XMM0, [RBX]
        &[0x66, 0x0F, 0xD4, 0xC1],       // PADDQ XMM0, XMM1
        &[0x66, 0x0F, 0xFB, 0xC1],       // PSUBQ XMM0, XMM1
        &[0x66, 0x0F, 0xF4, 0xC1],       // PMULUDQ XMM0, XMM1
    ];
    c.bench_function("decode_sse_mix", |b| {
        b.iter(|| {
            for instr_bytes in sse_instrs {
                let _ = black_box(fetch_decode64(black_box(*instr_bytes)));
            }
        })
    });
}

criterion_group!(benches, bench_decode_32bit, bench_decode_64bit, bench_decode_single_nop, bench_decode_sse);
criterion_main!(benches);
