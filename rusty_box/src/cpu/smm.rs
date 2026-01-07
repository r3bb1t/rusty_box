use alloc::borrow::ToOwned;

use super::{cpuid::BxCpuIdTrait, BxCpuC, CpuError, Result};

const SMM_SAVE_STATE_MAP_SIZE: u32 = 128;

#[allow(non_camel_case_types)]
#[derive(Debug)]
pub(super) enum SMMRAM_Fields {
    SMRAM_FIELD_SMBASE_OFFSET = 0,
    SMRAM_FIELD_SMM_REVISION_ID,
    SMRAM_FIELD_RAX_HI32,
    SMRAM_FIELD_EAX,
    SMRAM_FIELD_RCX_HI32,
    SMRAM_FIELD_ECX,
    SMRAM_FIELD_RDX_HI32,
    SMRAM_FIELD_EDX,
    SMRAM_FIELD_RBX_HI32,
    SMRAM_FIELD_EBX,
    SMRAM_FIELD_RSP_HI32,
    SMRAM_FIELD_ESP,
    SMRAM_FIELD_RBP_HI32,
    SMRAM_FIELD_EBP,
    SMRAM_FIELD_RSI_HI32,
    SMRAM_FIELD_ESI,
    SMRAM_FIELD_RDI_HI32,
    SMRAM_FIELD_EDI,
    SMRAM_FIELD_R8_HI32,
    SMRAM_FIELD_R8,
    SMRAM_FIELD_R9_HI32,
    SMRAM_FIELD_R9,
    SMRAM_FIELD_R10_HI32,
    SMRAM_FIELD_R10,
    SMRAM_FIELD_R11_HI32,
    SMRAM_FIELD_R11,
    SMRAM_FIELD_R12_HI32,
    SMRAM_FIELD_R12,
    SMRAM_FIELD_R13_HI32,
    SMRAM_FIELD_R13,
    SMRAM_FIELD_R14_HI32,
    SMRAM_FIELD_R14,
    SMRAM_FIELD_R15_HI32,
    SMRAM_FIELD_R15,
    SMRAM_FIELD_RIP_HI32,
    SMRAM_FIELD_EIP,
    SMRAM_FIELD_RFLAGS_HI32, // always zero
    SMRAM_FIELD_EFLAGS,
    SMRAM_FIELD_DR6_HI32, // always zero
    SMRAM_FIELD_DR6,
    SMRAM_FIELD_DR7_HI32, // always zero
    SMRAM_FIELD_DR7,
    SMRAM_FIELD_CR0_HI32, // always zero
    SMRAM_FIELD_CR0,
    SMRAM_FIELD_CR3_HI32, // zero when physical address size 32-bit
    SMRAM_FIELD_CR3,
    SMRAM_FIELD_CR4_HI32, // always zero
    SMRAM_FIELD_CR4,
    SMRAM_FIELD_EFER_HI32, // always zero
    SMRAM_FIELD_EFER,
    SMRAM_FIELD_IO_INSTRUCTION_RESTART,
    SMRAM_FIELD_AUTOHALT_RESTART,
    SMRAM_FIELD_NMI_MASK,
    SMRAM_FIELD_SSP_HI32,
    SMRAM_FIELD_SSP,
    SMRAM_FIELD_TR_BASE_HI32,
    SMRAM_FIELD_TR_BASE,
    SMRAM_FIELD_TR_LIMIT,
    SMRAM_FIELD_TR_SELECTOR_AR,
    SMRAM_FIELD_LDTR_BASE_HI32,
    SMRAM_FIELD_LDTR_BASE,
    SMRAM_FIELD_LDTR_LIMIT,
    SMRAM_FIELD_LDTR_SELECTOR_AR,
    SMRAM_FIELD_IDTR_BASE_HI32,
    SMRAM_FIELD_IDTR_BASE,
    SMRAM_FIELD_IDTR_LIMIT,
    SMRAM_FIELD_GDTR_BASE_HI32,
    SMRAM_FIELD_GDTR_BASE,
    SMRAM_FIELD_GDTR_LIMIT,
    SMRAM_FIELD_ES_BASE_HI32,
    SMRAM_FIELD_ES_BASE,
    SMRAM_FIELD_ES_LIMIT,
    SMRAM_FIELD_ES_SELECTOR_AR,
    SMRAM_FIELD_CS_BASE_HI32,
    SMRAM_FIELD_CS_BASE,
    SMRAM_FIELD_CS_LIMIT,
    SMRAM_FIELD_CS_SELECTOR_AR,
    SMRAM_FIELD_SS_BASE_HI32,
    SMRAM_FIELD_SS_BASE,
    SMRAM_FIELD_SS_LIMIT,
    SMRAM_FIELD_SS_SELECTOR_AR,
    SMRAM_FIELD_DS_BASE_HI32,
    SMRAM_FIELD_DS_BASE,
    SMRAM_FIELD_DS_LIMIT,
    SMRAM_FIELD_DS_SELECTOR_AR,
    SMRAM_FIELD_FS_BASE_HI32,
    SMRAM_FIELD_FS_BASE,
    SMRAM_FIELD_FS_LIMIT,
    SMRAM_FIELD_FS_SELECTOR_AR,
    SMRAM_FIELD_GS_BASE_HI32,
    SMRAM_FIELD_GS_BASE,
    SMRAM_FIELD_GS_LIMIT,
    SMRAM_FIELD_GS_SELECTOR_AR,
    SMRAM_FIELD_LAST,
}

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    pub(super) fn init_smram() -> Result<[u32; SMMRAM_Fields::SMRAM_FIELD_LAST as _]> {
        let mut smram_map = [0; SMMRAM_Fields::SMRAM_FIELD_LAST as _];
        smram_map[SMMRAM_Fields::SMRAM_FIELD_SMBASE_OFFSET as usize] = smram_translate(0x7f00);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_SMM_REVISION_ID as usize] = smram_translate(0x7efc);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_RAX_HI32 as usize] = smram_translate(0x7ffc);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_EAX as usize] = smram_translate(0x7ff8);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_RCX_HI32 as usize] = smram_translate(0x7ff4);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_ECX as usize] = smram_translate(0x7ff0);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_RDX_HI32 as usize] = smram_translate(0x7fec);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_EDX as usize] = smram_translate(0x7fe8);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_RBX_HI32 as usize] = smram_translate(0x7fe4);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_EBX as usize] = smram_translate(0x7fe0);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_RSP_HI32 as usize] = smram_translate(0x7fdc);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_ESP as usize] = smram_translate(0x7fd8);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_RBP_HI32 as usize] = smram_translate(0x7fd4);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_EBP as usize] = smram_translate(0x7fd0);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_RSI_HI32 as usize] = smram_translate(0x7fcc);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_ESI as usize] = smram_translate(0x7fc8);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_RDI_HI32 as usize] = smram_translate(0x7fc4);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_EDI as usize] = smram_translate(0x7fc0);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_R8_HI32 as usize] = smram_translate(0x7fbc);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_R8 as usize] = smram_translate(0x7fb8);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_R9_HI32 as usize] = smram_translate(0x7fb4);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_R9 as usize] = smram_translate(0x7fb0);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_R10_HI32 as usize] = smram_translate(0x7fac);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_R10 as usize] = smram_translate(0x7fa8);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_R11_HI32 as usize] = smram_translate(0x7fa4);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_R11 as usize] = smram_translate(0x7fa0);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_R12_HI32 as usize] = smram_translate(0x7f9c);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_R12 as usize] = smram_translate(0x7f98);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_R13_HI32 as usize] = smram_translate(0x7f94);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_R13 as usize] = smram_translate(0x7f90);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_R14_HI32 as usize] = smram_translate(0x7f8c);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_R14 as usize] = smram_translate(0x7f88);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_R15_HI32 as usize] = smram_translate(0x7f84);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_R15 as usize] = smram_translate(0x7f80);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_RIP_HI32 as usize] = smram_translate(0x7f7c);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_EIP as usize] = smram_translate(0x7f78);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_RFLAGS_HI32 as usize] = smram_translate(0x7f74); // always zero
        smram_map[SMMRAM_Fields::SMRAM_FIELD_EFLAGS as usize] = smram_translate(0x7f70);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_DR6_HI32 as usize] = smram_translate(0x7f6c); // always zero
        smram_map[SMMRAM_Fields::SMRAM_FIELD_DR6 as usize] = smram_translate(0x7f68);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_DR7_HI32 as usize] = smram_translate(0x7f64); // always zero
        smram_map[SMMRAM_Fields::SMRAM_FIELD_DR7 as usize] = smram_translate(0x7f60);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_CR0_HI32 as usize] = smram_translate(0x7f5c); // always zero
        smram_map[SMMRAM_Fields::SMRAM_FIELD_CR0 as usize] = smram_translate(0x7f58);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_CR3_HI32 as usize] = smram_translate(0x7f54); // zero when physical address size 32-bit
        smram_map[SMMRAM_Fields::SMRAM_FIELD_CR3 as usize] = smram_translate(0x7f50);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_CR4_HI32 as usize] = smram_translate(0x7f4c); // always zero
        smram_map[SMMRAM_Fields::SMRAM_FIELD_CR4 as usize] = smram_translate(0x7f48);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_SSP_HI32 as usize] = smram_translate(0x7f44);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_SSP as usize] = smram_translate(0x7f40);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_EFER_HI32 as usize] = smram_translate(0x7ed4); // always zero
        smram_map[SMMRAM_Fields::SMRAM_FIELD_EFER as usize] = smram_translate(0x7ed0);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_IO_INSTRUCTION_RESTART as usize] =
            smram_translate(0x7ec8);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_AUTOHALT_RESTART as usize] = smram_translate(0x7ec8);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_NMI_MASK as usize] = smram_translate(0x7ec8);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_TR_BASE_HI32 as usize] = smram_translate(0x7e9c);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_TR_BASE as usize] = smram_translate(0x7e98);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_TR_LIMIT as usize] = smram_translate(0x7e94);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_TR_SELECTOR_AR as usize] = smram_translate(0x7e90);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_IDTR_BASE_HI32 as usize] = smram_translate(0x7e8c);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_IDTR_BASE as usize] = smram_translate(0x7e88);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_IDTR_LIMIT as usize] = smram_translate(0x7e84);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_LDTR_BASE_HI32 as usize] = smram_translate(0x7e7c);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_LDTR_BASE as usize] = smram_translate(0x7e78);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_LDTR_LIMIT as usize] = smram_translate(0x7e74);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_LDTR_SELECTOR_AR as usize] = smram_translate(0x7e70);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_GDTR_BASE_HI32 as usize] = smram_translate(0x7e6c);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_GDTR_BASE as usize] = smram_translate(0x7e68);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_GDTR_LIMIT as usize] = smram_translate(0x7e64);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_ES_BASE_HI32 as usize] = smram_translate(0x7e0c);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_ES_BASE as usize] = smram_translate(0x7e08);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_ES_LIMIT as usize] = smram_translate(0x7e04);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_ES_SELECTOR_AR as usize] = smram_translate(0x7e00);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_CS_BASE_HI32 as usize] = smram_translate(0x7e1c);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_CS_BASE as usize] = smram_translate(0x7e18);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_CS_LIMIT as usize] = smram_translate(0x7e14);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_CS_SELECTOR_AR as usize] = smram_translate(0x7e10);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_SS_BASE_HI32 as usize] = smram_translate(0x7e2c);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_SS_BASE as usize] = smram_translate(0x7e28);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_SS_LIMIT as usize] = smram_translate(0x7e24);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_SS_SELECTOR_AR as usize] = smram_translate(0x7e20);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_DS_BASE_HI32 as usize] = smram_translate(0x7e3c);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_DS_BASE as usize] = smram_translate(0x7e38);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_DS_LIMIT as usize] = smram_translate(0x7e34);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_DS_SELECTOR_AR as usize] = smram_translate(0x7e30);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_FS_BASE_HI32 as usize] = smram_translate(0x7e4c);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_FS_BASE as usize] = smram_translate(0x7e48);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_FS_LIMIT as usize] = smram_translate(0x7e44);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_FS_SELECTOR_AR as usize] = smram_translate(0x7e40);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_GS_BASE_HI32 as usize] = smram_translate(0x7e5c);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_GS_BASE as usize] = smram_translate(0x7e58);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_GS_LIMIT as usize] = smram_translate(0x7e54);
        smram_map[SMMRAM_Fields::SMRAM_FIELD_GS_SELECTOR_AR as usize] = smram_translate(0x7e50);

        for (index, value) in smram_map.iter().enumerate() {
            let value = value.to_owned();
            if value >= SMM_SAVE_STATE_MAP_SIZE {
                return Err(CpuError::SmramMap { index, value });
            }
        }

        Ok(smram_map)
    }
}

const fn smram_translate(addr: u32) -> u32 {
    ((0x8000 - (addr)) >> 2) - 1
}
