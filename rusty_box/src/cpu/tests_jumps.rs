#[cfg(test)]
mod tests {
    use crate::cpu::{builder::BxCpuBuilder, core_i7_skylake::Corei7SkylakeX};
    use crate::memory::{BxMemC, BxMemoryStubC, CpuTlbPin};

    #[test]
    fn test_short_unconditional_jump() {
        // BxICache contains ~19MB fixed arrays; debug-mode struct literal needs large stack
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                let mut cpu = BxCpuBuilder::<Corei7SkylakeX>::new().build().unwrap();
                let mem_stub = BxMemoryStubC::create_and_init(1 << 20, 1 << 20, 4096).unwrap();
                let mut mem = BxMemC::new(mem_stub, false);

                let bytes: [u8; 4] = [0xEB, 0x02, 0x90, 0x90];

                let pins = [CpuTlbPin::new(&cpu)];
                assert_eq!(mem.write_ram(&pins, 0, &bytes).unwrap(), bytes.len());

                cpu.set_rip(0);
                cpu.cpu_loop(&mut mem, &pins, &pins[0]).ok();

                assert_eq!(cpu.rip(), 4);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn test_short_conditional_je() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                let mut cpu = BxCpuBuilder::<Corei7SkylakeX>::new().build().unwrap();
                let mem_stub = BxMemoryStubC::create_and_init(1 << 20, 1 << 20, 4096).unwrap();
                let mut mem = BxMemC::new(mem_stub, false);

                let bytes: [u8; 7] = [0x2B, 0xC0, 0x74, 0x02, 0x90, 0x90, 0x90];

                let pins = [CpuTlbPin::new(&cpu)];
                assert_eq!(mem.write_ram(&pins, 0, &bytes).unwrap(), bytes.len());

                cpu.set_rip(0);
                cpu.cpu_loop(&mut mem, &pins, &pins[0]).ok();

                assert_eq!(cpu.rip(), 6);
            })
            .unwrap()
            .join()
            .unwrap();
    }
}
