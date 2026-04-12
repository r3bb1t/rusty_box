use rusty_box_decoder::decoder::decode64::fetch_decode64;

#[test]
fn decode_montgomery_multiplication_loop() {
    // Bytes from libcrypto.so.3 bn_mulx4x_mont_gather5 inner loop
    let bytes: &[u8] = &[
        0xc4, 0xe2, 0xbb, 0xf6, 0x06, // mulx rax, r8, [rsi]
        0xc4, 0x62, 0xa3, 0xf6, 0x76, 0x08, // mulx r14, r11, [rsi+8]
        0x49, 0x01, 0xc3, // add r11, rax
        0x48, 0x89, 0x7c, 0x24, 0x08, // mov [rsp+8], rdi
        0xc4, 0x62, 0x9b, 0xf6, 0x6e, 0x10, // mulx r13, r12, [rsi+0x10]
        0x4d, 0x11, 0xf4, // adc r12, r14
        0x49, 0x83, 0xd5, 0x00, // adc r13, 0
        0x4c, 0x89, 0xc7, // mov rdi, r8
        0x4c, 0x0f, 0xaf, 0x44, 0x24, 0x18, // imul r8, [rsp+0x18]
        0x48, 0x31, 0xed, // xor rbp, rbp
        0xc4, 0x62, 0xfb, 0xf6, 0x76, 0x18, // mulx r14, rax, [rsi+0x18]
        0x4c, 0x89, 0xc2, // mov rdx, r8
        0x48, 0x8d, 0x76, 0x20, // lea rsi, [rsi+0x20]
        0x66, 0x4c, 0x0f, 0x38, 0xf6, 0xe8, // adcx r13, rax
        0x66, 0x4c, 0x0f, 0x38, 0xf6, 0xf5, // adcx r14, rbp
        0xc4, 0x62, 0xfb, 0xf6, 0x11, // mulx r10, rax, [rcx]
        0x66, 0x48, 0x0f, 0x38, 0xf6, 0xf8, // adcx rdi, rax
        0xf3, 0x4d, 0x0f, 0x38, 0xf6, 0xd3, // adox r10, r11
        0xc4, 0x62, 0xfb, 0xf6, 0x59, 0x08, // mulx r11, rax, [rcx+8]
        0x66, 0x4c, 0x0f, 0x38, 0xf6, 0xd0, // adcx r10, rax
        0xf3, 0x4d, 0x0f, 0x38, 0xf6, 0xdc, // adox r11, r12
    ];

    let expected: &[(usize, usize, &str)] = &[
        (0, 5, "MULX"),
        (5, 6, "MULX"),
        (11, 3, "ADD"),
        (14, 5, "MOV"),
        (19, 6, "MULX"),
        (25, 3, "ADC"),
        (28, 4, "ADC"),
        (32, 3, "MOV"),
        (35, 6, "IMUL"),
        (41, 3, "XOR"),
        (44, 6, "MULX"),
        (50, 3, "MOV"),
        (53, 4, "LEA"),
        (57, 6, "ADCX"),
        (63, 6, "ADCX"),
        (69, 5, "MULX"),
        (74, 6, "ADCX"),
        (80, 6, "ADOX"),
        (86, 6, "MULX"),
        (92, 6, "ADCX"),
        (98, 6, "ADOX"),
    ];

    let mut offset = 0;
    for (idx, &(exp_off, exp_ilen, exp_name)) in expected.iter().enumerate() {
        assert_eq!(offset, exp_off, "offset mismatch at instruction #{}", idx);
        let instr = fetch_decode64(&bytes[offset..]).unwrap_or_else(|e| {
            panic!("decode error at offset {} (#{} {}): {:?}", offset, idx, exp_name, e);
        });
        let ilen = instr.ilen() as usize;
        assert_eq!(ilen, exp_ilen,
            "ilen mismatch at #{}: got {} expected {} for {} (opcode={:?})",
            idx, ilen, exp_ilen, exp_name, instr.get_ia_opcode());

        let opcode_name = format!("{:?}", instr.get_ia_opcode());
        // Basic sanity: opcode name should contain the instruction name
        let name_upper = exp_name.to_uppercase();
        assert!(
            opcode_name.to_uppercase().contains(&name_upper)
            || (name_upper == "MOV" && (opcode_name.contains("Mov") || opcode_name.contains("mov")))
            || (name_upper == "ADD" && opcode_name.contains("Add"))
            || (name_upper == "ADC" && opcode_name.contains("Adc"))
            || (name_upper == "XOR" && opcode_name.contains("Xor"))
            || (name_upper == "LEA" && opcode_name.contains("Lea"))
            || (name_upper == "IMUL" && opcode_name.contains("Imul")),
            "opcode mismatch at #{}: {:?} doesn't match expected {}",
            idx, instr.get_ia_opcode(), exp_name
        );

        println!("OK #{:2}: offset={:3} ilen={} {:?}", idx, offset, ilen, instr.get_ia_opcode());
        offset += ilen;
    }
    println!("All {} instructions decoded correctly!", expected.len());
}
