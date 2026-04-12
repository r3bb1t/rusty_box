#[cfg(test)]
mod tests {
    use crate::cpu::{builder::BxCpuBuilder, core_i7_skylake::Corei7SkylakeX, BxCpuC, BxCpuIdTrait};
    use crate::memory::{BxMemC, BxMemoryStubC};
    use crate::cpu::rusty_box::MemoryAccessType;
    use crate::config::BxPhyAddress;

    #[test]
    fn test_short_unconditional_jump() {
        // Build CPU and memory
        let mut cpu = BxCpuBuilder::<Corei7SkylakeX>::new().build().unwrap();
        let mem_stub = BxMemoryStubC::create_and_init(1 << 20, 1 << 20, 4096).unwrap();
        let mut mem = BxMemC::new(mem_stub, false);

        // Program: 0: EB 02    jmp +2 (to offset 4)
        //          2: 90       nop
        //          3: 90       nop
        //          4: 90       nop ; target
        let bytes: [u8; 4] = [0xEB, 0x02, 0x90, 0x90];

        // write bytes to guest physical memory at 0
        {
            // pass cpu as slice to satisfy API
            let cpus: [&BxCpuC<Corei7SkylakeX>; 1] = [&cpu];
            let host_opt = mem.get_host_mem_addr(0 as BxPhyAddress, MemoryAccessType::RW, &cpus).unwrap();
            assert!(host_opt.is_some());
            let host = host_opt.unwrap();
            host[..bytes.len()].copy_from_slice(&bytes);
        }

        cpu.set_rip(0);
        // run cpu loop (limited iterations)
        cpu.cpu_loop(&mut mem, &[]).ok();

        // After jump, RIP should point to target at 4
        assert_eq!(cpu.rip(), 4);
    }

    #[test]
    fn test_short_conditional_je() {
        // Build CPU and memory
        let mut cpu = BxCpuBuilder::<Corei7SkylakeX>::new().build().unwrap();
        let mem_stub = BxMemoryStubC::create_and_init(1 << 20, 1 << 20, 4096).unwrap();
        let mut mem = BxMemC::new(mem_stub, false);

        // Program: 0: 2B C0    sub eax, eax  ; sets ZF
        //          2: 74 02    je +2 (to offset 6)
        //          4: 90       nop
        //          5: 90       nop
        //          6: 90       nop ; target
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

        // After JE taken, RIP should be 6
        assert_eq!(cpu.rip(), 6);
    }
}
