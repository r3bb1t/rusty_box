#[cfg(test)]
mod tests {
    use crate::cpu::{builder::BxCpuBuilder, core_i7_skylake::Corei7SkylakeX, BxCpuC, BxCpuIdTrait};
    use crate::memory::{BxMemC, BxMemoryStubC};
    use crate::cpu::rusty_box::MemoryAccessType;
    use crate::config::BxPhyAddress;

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

                {
                    let cpus: [&BxCpuC<Corei7SkylakeX>; 1] = [&cpu];
                    let host_opt = mem.get_host_mem_addr(0 as BxPhyAddress, MemoryAccessType::RW, &cpus).unwrap();
                    assert!(host_opt.is_some());
                    let host = host_opt.unwrap();
                    host[..bytes.len()].copy_from_slice(&bytes);
                }

                cpu.set_rip(0);
                cpu.cpu_loop(&mut mem, &[]).ok();

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

                {
                    let cpus: [&BxCpuC<Corei7SkylakeX>; 1] = [&cpu];
                    let host_opt = mem.get_host_mem_addr(0 as BxPhyAddress, MemoryAccessType::RW, &cpus).unwrap();
                    assert!(host_opt.is_some());
                    let host = host_opt.unwrap();
                    host[..bytes.len()].copy_from_slice(&bytes);
                }

                cpu.set_rip(0);
                cpu.cpu_loop(&mut mem, &[]).ok();

                assert_eq!(cpu.rip(), 6);
            })
            .unwrap()
            .join()
            .unwrap();
    }
}
