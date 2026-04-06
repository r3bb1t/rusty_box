//! Protected mode interrupt handling
//!
//! Based on Bochs cpu/exception.cc protected_mode_int
//! Copyright (C) 2001-2019 The Bochs Project

use alloc::vec::Vec;

use super::{
    cpu::{BxCpuC, Exception},
    cpuid::BxCpuIdTrait,
    decoder::BxSegregs,
    descriptor::{BxDescriptor, BxSegmentReg, BxSelector, SystemAndGateDescriptorEnum},
    eflags::EFlags,
    segment_ctrl_pro::parse_selector,
    Result,
};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    /// Handle interrupt in protected mode via IDT
    /// Based on BX_CPU_C::protected_mode_int in exception.cc:284
    pub(super) fn protected_mode_int(
        &mut self,
        vector: u8,
        soft_int: bool,
        push_error: bool,
        error_code: u16,
    ) -> Result<()> {
        // Only log for exceptions (vectors 0-31), not hardware IRQs (32+)
        if vector < 32 {
            tracing::trace!("PM_INT: vec={:#04x} IDTR.base={:#010x} IDTR.limit={:#06x} CPL={} RIP={:#010x} icount={}",
                vector, self.idtr.base, self.idtr.limit,
                self.sregs[BxSegregs::Cs as usize].selector.rpl,
                self.rip(), self.icount);
        }
        // Bochs: error code for IDT descriptor errors = vector*8 + 2 + EXT
        let idt_error_code = (vector as u16) * 8 + 2;

        // interrupt vector must be within IDT table limits, else #GP(vector*8 + 2 + EXT)
        if (vector as u64 * 8 + 7) > self.idtr.limit as u64 {
            tracing::error!(
                "protected_mode_int(): vector must be within IDT table limits, IDT.limit = {:#x}",
                self.idtr.limit
            );
            return Err(super::error::CpuError::BadVector {
                vector: Exception::Gp,
                error_code: idt_error_code,
            });
        }

        let gate_addr = self.idtr.base + vector as u64 * 8;
        let raw_descriptor = self.system_read_qword(gate_addr)?;
        let dword1 = raw_descriptor as u32;
        let dword2 = (raw_descriptor >> 32) as u32;
        if vector < 32 {
            tracing::debug!(
                "PM_INT: IDT[{:#04x}] @ {:#010x}: dword1={:#010x} dword2={:#010x}",
                vector,
                gate_addr,
                dword1,
                dword2
            );
        }

        let gate_descriptor = self.parse_descriptor(dword1, dword2)?;

        if gate_descriptor.valid == 0 || gate_descriptor.segment {
            tracing::error!(
                "protected_mode_int(): gate descriptor is not valid sys seg (vector={:#04x})",
                vector
            );
            return Err(super::error::CpuError::BadVector {
                vector: Exception::Gp,
                error_code: idt_error_code,
            });
        }

        // descriptor AR byte must indicate interrupt gate, trap gate, or task gate, else #GP(vector*8 + 2 + EXT)
        let gate_type = gate_descriptor.r#type;
        let is_valid_gate = matches!(gate_type, 0x5 | 0x6 | 0x7 | 0xE | 0xF);

        if !is_valid_gate {
            tracing::error!(
                "protected_mode_int(): gate.type({:#x}) != {{5,6,7,14,15}}",
                gate_type
            );
            return Err(super::error::CpuError::BadVector {
                vector: Exception::Gp,
                error_code: idt_error_code,
            });
        }

        // Convert gate type to enum for matching
        let gate_type_enum = match gate_type {
            0x5 => SystemAndGateDescriptorEnum::BxTaskGate,
            0x6 => SystemAndGateDescriptorEnum::Bx286InterruptGate,
            0x7 => SystemAndGateDescriptorEnum::Bx286TrapGate,
            0xE => SystemAndGateDescriptorEnum::Bx386InterruptGate,
            0xF => SystemAndGateDescriptorEnum::Bx386TrapGate,
            _ => unreachable!(),
        };

        match gate_type_enum {
            SystemAndGateDescriptorEnum::BxTaskGate
            | SystemAndGateDescriptorEnum::Bx286InterruptGate
            | SystemAndGateDescriptorEnum::Bx286TrapGate
            | SystemAndGateDescriptorEnum::Bx386InterruptGate
            | SystemAndGateDescriptorEnum::Bx386TrapGate => {}
            _ => {
                tracing::error!(
                    "protected_mode_int(): gate.type({:#x}) != {{5,6,7,14,15}}",
                    gate_descriptor.r#type
                );
                return Err(super::error::CpuError::BadVector {
                    vector: Exception::Gp,
                    error_code: idt_error_code,
                });
            }
        }

        // if software interrupt, then gate descriptor DPL must be >= CPL, else #GP(vector * 8 + 2 + EXT)
        let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
        if soft_int && gate_descriptor.dpl < cpl {
            tracing::error!("protected_mode_int(): soft_int && (gate.dpl < CPL)");
            return Err(super::error::CpuError::BadVector {
                vector: Exception::Gp,
                error_code: idt_error_code,
            });
        }

        // Gate must be present, else #NP(vector * 8 + 2 + EXT)
        if !gate_descriptor.p {
            tracing::error!("protected_mode_int(): gate not present");
            return Err(super::error::CpuError::BadVector {
                vector: Exception::Np,
                error_code: idt_error_code,
            });
        }

        match gate_type_enum {
            SystemAndGateDescriptorEnum::BxTaskGate => {
                // Task switch via Task Gate (matches original lines 341-381)
                // Bochs returns immediately after task_switch — no flag clearing
                // SAFETY: descriptor type verified as task gate before union access
                let raw_tss_selector = gate_descriptor.u.task_gate_tss_selector();
                let mut tss_selector = BxSelector::default();
                parse_selector(raw_tss_selector, &mut tss_selector);

                let (tss_dword1, tss_dword2) = match self.fetch_raw_descriptor(&tss_selector) {
                    Ok(v) => v,
                    Err(_) => {
                        return Err(super::error::CpuError::BadVector {
                            vector: Exception::Ts,
                            error_code: raw_tss_selector & 0xfffc,
                        })
                    }
                };
                let tss_descriptor =
                    self.parse_descriptor(tss_dword1, tss_dword2).map_err(|_| {
                        super::error::CpuError::BadVector {
                            vector: Exception::Ts,
                            error_code: raw_tss_selector & 0xfffc,
                        }
                    })?;

                // Call task_switch with BX_TASK_FROM_INT (0x2)
                self.task_switch(
                    &tss_selector,
                    &tss_descriptor,
                    0x2, // BX_TASK_FROM_INT
                    tss_dword1,
                    tss_dword2,
                    push_error,
                    error_code as u32,
                )?;
                // Bochs returns here — task_switch handles all flags
                return Ok(());
            }
            SystemAndGateDescriptorEnum::Bx286InterruptGate
            | SystemAndGateDescriptorEnum::Bx286TrapGate
            | SystemAndGateDescriptorEnum::Bx386InterruptGate
            | SystemAndGateDescriptorEnum::Bx386TrapGate => {
                // SAFETY: descriptor type verified as gate before union access
                let gate_dest_offset = gate_descriptor.u.gate_dest_offset();
                self.handle_interrupt_trap_gate(&gate_descriptor, push_error, error_code)?;
                // Set EIP after handling the gate (matches original line 714)
                self.set_eip(gate_dest_offset);
            }
            _ => {
                return Err(super::error::CpuError::BadVector {
                    vector: Exception::Gp,
                    error_code: idt_error_code,
                });
            }
        }

        // if interrupt gate then set IF to 0 (Bochs exception.cc:716-722)
        // Only reached for interrupt/trap gates — task gate returns early above
        if (gate_descriptor.r#type & 1) == 0 {
            // even is int-gate
            self.eflags.remove(EFlags::IF_);
            self.handle_interrupt_mask_change();
        }
        self.eflags.remove(EFlags::TF);
        self.eflags.remove(EFlags::NT);
        self.eflags.remove(EFlags::VM);
        self.eflags.remove(EFlags::RF);

        Ok(())
    }

    /// Handle interrupt/trap gate (not task gate)
    fn handle_interrupt_trap_gate(
        &mut self,
        gate_descriptor: &BxDescriptor,
        push_error: bool,
        error_code: u16,
    ) -> Result<()> {
        // SAFETY: descriptor type verified as gate before union access
        let gate_dest_selector = gate_descriptor.u.gate_dest_selector();
        let gate_dest_offset = gate_descriptor.u.gate_dest_offset();

        if (gate_dest_selector & 0xfffc) == 0 {
            tracing::error!("handle_interrupt_trap_gate(): selector null");
            // Bochs exception.cc:395: #GP(0) for null selector
            return Err(super::error::CpuError::BadVector {
                vector: Exception::Gp,
                error_code: 0,
            });
        }

        let mut cs_selector = BxSelector::default();
        parse_selector(gate_dest_selector, &mut cs_selector);

        let cs_err_code = cs_selector.value & 0xfffc;
        let (cs_dword1, cs_dword2) = match self.fetch_raw_descriptor(&cs_selector) {
            Ok(v) => v,
            Err(_) => {
                return Err(super::error::CpuError::BadVector {
                    vector: Exception::Gp,
                    error_code: cs_err_code,
                })
            }
        };
        let cs_descriptor = self.parse_descriptor(cs_dword1, cs_dword2).map_err(|_| {
            super::error::CpuError::BadVector {
                vector: Exception::Gp,
                error_code: cs_err_code,
            }
        })?;

        let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
        // Bochs exception.cc:407-413: #GP(selector+EXT)
        if cs_descriptor.valid == 0
            || !cs_descriptor.segment
            || super::descriptor::is_data_segment(cs_descriptor.r#type)
            || cs_descriptor.dpl > cpl
        {
            tracing::debug!(
                "handle_interrupt_trap_gate(): not accessible or not code segment cs={:#04x} \
                 valid={} segment={} type={:#x} dpl={} cpl={} icount={}",
                cs_selector.value,
                cs_descriptor.valid,
                cs_descriptor.segment,
                cs_descriptor.r#type,
                cs_descriptor.dpl,
                cpl,
                self.icount
            );
            return Err(super::error::CpuError::BadVector {
                vector: Exception::Gp,
                error_code: cs_err_code,
            });
        }

        // Bochs exception.cc:416-418: #NP(selector+EXT)
        if !cs_descriptor.p {
            tracing::error!("handle_interrupt_trap_gate(): segment not present");
            return Err(super::error::CpuError::BadVector {
                vector: Exception::Np,
                error_code: cs_err_code,
            });
        }

        // Save old register values (matches original lines 421-424)
        let old_esp = self.esp();
        let old_ss = self.sregs[BxSegregs::Ss as usize].selector.value;
        let old_eip = self.eip();
        let old_cs = self.sregs[BxSegregs::Cs as usize].selector.value;

        // Bochs exception.cc:667-711: conforming or DPL == CPL → same privilege
        // Bochs exception.cc:435-665: non-conforming and DPL < CPL → inner privilege
        if super::descriptor::is_code_segment_conforming(cs_descriptor.r#type)
            || cs_descriptor.dpl == cpl
        {
            // INTERRUPT TO SAME PRIVILEGE LEVEL
            tracing::debug!("handle_interrupt_trap_gate(): INTERRUPT TO SAME PRIVILEGE");

            // v8086 mode check (Bochs exception.cc:671-674): #GP(cs_selector+EXT)
            if self.v8086_mode()
                && (super::descriptor::is_code_segment_conforming(cs_descriptor.r#type)
                    || cs_descriptor.dpl != 0)
            {
                tracing::error!("handle_interrupt_trap_gate(): code segment conforming or DPL({}) != 0 in v8086 mode", cs_descriptor.dpl);
                return Err(super::error::CpuError::BadVector {
                    vector: Exception::Gp,
                    error_code: cs_err_code,
                });
            }

            // EIP must be in CS limit else #GP(0) (matches original line 678)
            // SAFETY: segment cache populated during segment load; union read matches descriptor type
            if gate_dest_offset > cs_descriptor.u.segment_limit_scaled() {
                let gdt_entry_laddr = self.gdtr.base + (cs_selector.index as u64 * 8);
                let gdt_phys = gdt_entry_laddr.wrapping_sub(0xC0000000);
                let raw_lo = self.mem_read_dword(gdt_phys);
                let raw_hi = self.mem_read_dword(gdt_phys + 4);
                tracing::error!(
                    "handle_interrupt_trap_gate(): IP > CS descriptor limit: \
                     gate_dest_offset={:#x} limit_scaled={:#x} cs_sel={:#06x} \
                     GDT[{}] raw={:#010x}_{:#010x} at phys={:#010x} \
                     GDTR.base={:#010x} GDTR.limit={:#06x} RIP={:#010x} icount={}",
                    gate_dest_offset,
                    cs_descriptor.u.segment_limit_scaled(),
                    cs_selector.value,
                    cs_selector.index,
                    raw_hi,
                    raw_lo,
                    gdt_phys,
                    self.gdtr.base,
                    self.gdtr.limit,
                    self.rip(),
                    self.icount
                );
                return Err(super::error::CpuError::BadVector {
                    vector: Exception::Gp,
                    error_code: 0,
                });
            }

            // Check if 386 gate (type >= 14) - matches original line 686
            let is_386_gate = gate_descriptor.r#type >= 14;

            if is_386_gate {
                self.push_32(self.eflags.bits())?;
                self.push_32(self.sregs[BxSegregs::Cs as usize].selector.value as u32)?;
                self.push_32(self.eip())?;
                if push_error {
                    self.push_32(error_code as u32)?;
                }
            } else {
                self.push_16((self.eflags.bits() & 0xFFFF) as u16)?;
                self.push_16(self.sregs[BxSegregs::Cs as usize].selector.value)?;
                self.push_16(self.eip() as u16)?;
                if push_error {
                    self.push_16(error_code)?;
                }
            }

            // Load CS segment register
            // Set the RPL field of CS to CPL (matches original line 711: load_cs(&cs_selector, &cs_descriptor, CPL))
            let mut new_cs_selector = cs_selector;
            new_cs_selector.rpl = cpl;
            new_cs_selector.value = (new_cs_selector.value & !0x03) | cpl as u16;

            if tracing::enabled!(tracing::Level::DEBUG) {
                // SAFETY: segment cache populated during segment load; union read matches descriptor type
                let base = cs_descriptor.u.segment_base();
                let linear = base + gate_dest_offset as u64;
                let bytes: Vec<u8> = (0..48u64)
                    .map(|i| {
                        if let Ok(pa) = self.translate_linear_system_read(linear + i) {
                            self.mem_read_byte(pa)
                        } else {
                            0xFF
                        }
                    })
                    .collect();
                tracing::trace!("PM_INT: loading CS sel={:#06x} base={:#010x} limit={:#010x} d_b={} -> EIP={:#010x} (linear={:#010x})",
                    new_cs_selector.value, base,
                    cs_descriptor.u.segment_limit_scaled(),
                    cs_descriptor.u.segment_d_b(),
                    gate_dest_offset, linear);
                tracing::trace!("PM_INT: handler bytes @ {:#010x}: {:02x?}", linear, bytes);
            }

            // Use load_cs() to properly update user_pl, touch accessed bit, and
            // invalidate prefetch queue (matches Bochs exception.cc:711)
            let mut cs_desc_mut = cs_descriptor;
            self.load_cs(&mut new_cs_selector, &mut cs_desc_mut, cpl)?;
        } else if super::descriptor::is_code_segment_non_conforming(cs_descriptor.r#type)
            && cs_descriptor.dpl < cpl
        {
            // INTERRUPT TO INNER PRIVILEGE
            self.handle_interrupt_to_inner_privilege(
                gate_descriptor,
                &cs_selector,
                &cs_descriptor,
                old_esp,
                old_ss,
                old_eip,
                old_cs,
                push_error,
                error_code,
            )?;
        } else {
            tracing::error!(
                "handle_interrupt_trap_gate(): bad descriptor type {:#x} (CS.DPL={:#x} CPL={:#x})",
                cs_descriptor.r#type,
                cs_descriptor.dpl,
                cpl
            );
            return Err(super::error::CpuError::BadVector {
                vector: Exception::Gp,
                error_code: 0,
            });
        }

        // EIP is set in the caller after this function returns
        // (matches original where EIP is set at line 714)

        Ok(())
    }

    /// Handle interrupt to inner privilege level (DPL < CPL)
    /// Based on exception.cc:435-665
    #[allow(clippy::too_many_arguments)]
    fn handle_interrupt_to_inner_privilege(
        &mut self,
        gate_descriptor: &BxDescriptor,
        cs_selector: &BxSelector,
        cs_descriptor: &BxDescriptor,
        old_esp: u32,
        old_ss: u16,
        old_eip: u32,
        old_cs: u16,
        push_error: bool,
        error_code: u16,
    ) -> Result<()> {
        tracing::debug!("handle_interrupt_to_inner_privilege(): INTERRUPT TO INNER PRIVILEGE");

        // Get SS and ESP from TSS for the new privilege level (matches line 446)
        let (ss_for_cpl_x, esp_for_cpl_x) = self.get_ss_esp_from_tss(cs_descriptor.dpl)?;

        let is_v8086_mode = self.v8086_mode();
        // Bochs exception.cc:448-451: #GP(new code segment selector)
        if is_v8086_mode && cs_descriptor.dpl != 0 {
            tracing::error!(
                "handle_interrupt_to_inner_privilege(): code segment DPL({}) != 0 in v8086 mode",
                cs_descriptor.dpl
            );
            return Err(super::error::CpuError::BadVector {
                vector: Exception::Gp,
                error_code: cs_selector.value & 0xfffc,
            });
        }

        // Bochs exception.cc:455-457: Selector must be non-null else #TS(0)
        if (ss_for_cpl_x & 0xfffc) == 0 {
            tracing::error!("handle_interrupt_to_inner_privilege(): SS selector null");
            return Err(super::error::CpuError::BadVector {
                vector: Exception::Ts,
                error_code: 0,
            });
        }

        // Parse SS selector and fetch descriptor (matches lines 462-465)
        let mut ss_selector = BxSelector::default();
        parse_selector(ss_for_cpl_x, &mut ss_selector);
        let ss_err_code = ss_for_cpl_x & 0xfffc;
        let (ss_dword1, ss_dword2) = match self.fetch_raw_descriptor(&ss_selector) {
            Ok(v) => v,
            Err(_) => {
                return Err(super::error::CpuError::BadVector {
                    vector: Exception::Ts,
                    error_code: ss_err_code,
                })
            }
        };
        let ss_descriptor = self.parse_descriptor(ss_dword1, ss_dword2).map_err(|_| {
            super::error::CpuError::BadVector {
                vector: Exception::Ts,
                error_code: ss_err_code,
            }
        })?;

        // Bochs exception.cc:469-471: #TS(SS selector + ext)
        if ss_selector.rpl != cs_descriptor.dpl {
            tracing::error!("handle_interrupt_to_inner_privilege(): SS.rpl != CS.dpl");
            return Err(super::error::CpuError::BadVector {
                vector: Exception::Ts,
                error_code: ss_err_code,
            });
        }

        // Bochs exception.cc:476-478: #TS(SS selector + ext)
        if ss_descriptor.dpl != cs_descriptor.dpl {
            tracing::error!("handle_interrupt_to_inner_privilege(): SS.dpl != CS.dpl");
            return Err(super::error::CpuError::BadVector {
                vector: Exception::Ts,
                error_code: ss_err_code,
            });
        }

        // Bochs exception.cc:483-488: #TS(SS selector + EXT)
        if ss_descriptor.valid == 0
            || !ss_descriptor.segment
            || super::descriptor::is_code_segment(ss_descriptor.r#type)
            || !super::descriptor::is_data_segment_writable(ss_descriptor.r#type)
        {
            tracing::error!(
                "handle_interrupt_to_inner_privilege(): SS is not writable data segment"
            );
            return Err(super::error::CpuError::BadVector {
                vector: Exception::Ts,
                error_code: ss_err_code,
            });
        }

        // Bochs exception.cc:492-494: #SS(SS selector + ext)
        if !ss_descriptor.p {
            tracing::error!("handle_interrupt_to_inner_privilege(): SS not present");
            return Err(super::error::CpuError::BadVector {
                vector: Exception::Ss,
                error_code: ss_err_code,
            });
        }

        // Bochs exception.cc:497-500: IP must be within CS segment boundaries, else #GP(0)
        // SAFETY: descriptor type verified as gate before union access
        let gate_dest_offset = gate_descriptor.u.gate_dest_offset();
        // SAFETY: segment cache populated during segment load; union read matches descriptor type
        if gate_dest_offset > cs_descriptor.u.segment_limit_scaled() {
            tracing::error!("handle_interrupt_to_inner_privilege(): gate EIP > CS.limit");
            return Err(super::error::CpuError::BadVector {
                vector: Exception::Gp,
                error_code: 0,
            });
        }

        // Prepare new stack segment (matches lines 503-509)
        let mut new_stack = BxSegmentReg {
            selector: ss_selector,
            cache: ss_descriptor.clone(),
        };
        new_stack.selector.rpl = cs_descriptor.dpl;
        new_stack.selector.value =
            (new_stack.selector.value & 0xfffc) | new_stack.selector.rpl as u16;

        let is_386_gate = gate_descriptor.r#type >= 14;

        // Build stack frame on new stack (matches lines 511-630)
        // SAFETY: segment cache populated during segment load; union read matches descriptor type
        if new_stack.cache.u.segment_d_b() {
            // 32-bit stack
            let mut temp_esp = esp_for_cpl_x;

            // Push segment registers for v8086 mode (matches lines 514-538)
            if is_v8086_mode {
                if is_386_gate {
                    self.write_new_stack_dword(
                        &new_stack,
                        temp_esp.wrapping_sub(4),
                        cs_descriptor.dpl,
                        self.sregs[BxSegregs::Gs as usize].selector.value as u32,
                    )?;
                    self.write_new_stack_dword(
                        &new_stack,
                        temp_esp.wrapping_sub(8),
                        cs_descriptor.dpl,
                        self.sregs[BxSegregs::Fs as usize].selector.value as u32,
                    )?;
                    self.write_new_stack_dword(
                        &new_stack,
                        temp_esp.wrapping_sub(12),
                        cs_descriptor.dpl,
                        self.sregs[BxSegregs::Ds as usize].selector.value as u32,
                    )?;
                    self.write_new_stack_dword(
                        &new_stack,
                        temp_esp.wrapping_sub(16),
                        cs_descriptor.dpl,
                        self.sregs[BxSegregs::Es as usize].selector.value as u32,
                    )?;
                    temp_esp = temp_esp.wrapping_sub(16);
                } else {
                    self.write_new_stack_word(
                        &new_stack,
                        temp_esp.wrapping_sub(2),
                        cs_descriptor.dpl,
                        self.sregs[BxSegregs::Gs as usize].selector.value,
                    )?;
                    self.write_new_stack_word(
                        &new_stack,
                        temp_esp.wrapping_sub(4),
                        cs_descriptor.dpl,
                        self.sregs[BxSegregs::Fs as usize].selector.value,
                    )?;
                    self.write_new_stack_word(
                        &new_stack,
                        temp_esp.wrapping_sub(6),
                        cs_descriptor.dpl,
                        self.sregs[BxSegregs::Ds as usize].selector.value,
                    )?;
                    self.write_new_stack_word(
                        &new_stack,
                        temp_esp.wrapping_sub(8),
                        cs_descriptor.dpl,
                        self.sregs[BxSegregs::Es as usize].selector.value,
                    )?;
                    temp_esp = temp_esp.wrapping_sub(8);
                }
            }

            // Push return frame (matches lines 540-567)
            if is_386_gate {
                self.write_new_stack_dword(
                    &new_stack,
                    temp_esp.wrapping_sub(4),
                    cs_descriptor.dpl,
                    old_ss as u32,
                )?;
                self.write_new_stack_dword(
                    &new_stack,
                    temp_esp.wrapping_sub(8),
                    cs_descriptor.dpl,
                    old_esp,
                )?;
                self.write_new_stack_dword(
                    &new_stack,
                    temp_esp.wrapping_sub(12),
                    cs_descriptor.dpl,
                    self.eflags.bits(),
                )?;
                self.write_new_stack_dword(
                    &new_stack,
                    temp_esp.wrapping_sub(16),
                    cs_descriptor.dpl,
                    old_cs as u32,
                )?;
                self.write_new_stack_dword(
                    &new_stack,
                    temp_esp.wrapping_sub(20),
                    cs_descriptor.dpl,
                    old_eip,
                )?;
                temp_esp = temp_esp.wrapping_sub(20);

                if push_error {
                    temp_esp = temp_esp.wrapping_sub(4);
                    self.write_new_stack_dword(
                        &new_stack,
                        temp_esp,
                        cs_descriptor.dpl,
                        error_code as u32,
                    )?;
                }
            } else {
                // 286 gate
                self.write_new_stack_word(
                    &new_stack,
                    temp_esp.wrapping_sub(2),
                    cs_descriptor.dpl,
                    old_ss,
                )?;
                self.write_new_stack_word(
                    &new_stack,
                    temp_esp.wrapping_sub(4),
                    cs_descriptor.dpl,
                    old_esp as u16,
                )?;
                self.write_new_stack_word(
                    &new_stack,
                    temp_esp.wrapping_sub(6),
                    cs_descriptor.dpl,
                    (self.eflags.bits() & 0xFFFF) as u16,
                )?;
                self.write_new_stack_word(
                    &new_stack,
                    temp_esp.wrapping_sub(8),
                    cs_descriptor.dpl,
                    old_cs,
                )?;
                self.write_new_stack_word(
                    &new_stack,
                    temp_esp.wrapping_sub(10),
                    cs_descriptor.dpl,
                    old_eip as u16,
                )?;
                temp_esp = temp_esp.wrapping_sub(10);

                if push_error {
                    temp_esp = temp_esp.wrapping_sub(2);
                    self.write_new_stack_word(&new_stack, temp_esp, cs_descriptor.dpl, error_code)?;
                }
            }

            self.set_esp(temp_esp);
        } else {
            // 16-bit stack
            let mut temp_sp = esp_for_cpl_x as u16;

            // Push segment registers for v8086 mode (matches lines 574-598)
            if is_v8086_mode {
                if is_386_gate {
                    self.write_new_stack_dword(
                        &new_stack,
                        temp_sp.wrapping_sub(4) as u32,
                        cs_descriptor.dpl,
                        self.sregs[BxSegregs::Gs as usize].selector.value as u32,
                    )?;
                    self.write_new_stack_dword(
                        &new_stack,
                        temp_sp.wrapping_sub(8) as u32,
                        cs_descriptor.dpl,
                        self.sregs[BxSegregs::Fs as usize].selector.value as u32,
                    )?;
                    self.write_new_stack_dword(
                        &new_stack,
                        temp_sp.wrapping_sub(12) as u32,
                        cs_descriptor.dpl,
                        self.sregs[BxSegregs::Ds as usize].selector.value as u32,
                    )?;
                    self.write_new_stack_dword(
                        &new_stack,
                        temp_sp.wrapping_sub(16) as u32,
                        cs_descriptor.dpl,
                        self.sregs[BxSegregs::Es as usize].selector.value as u32,
                    )?;
                    temp_sp = temp_sp.wrapping_sub(16);
                } else {
                    self.write_new_stack_word(
                        &new_stack,
                        temp_sp.wrapping_sub(2) as u32,
                        cs_descriptor.dpl,
                        self.sregs[BxSegregs::Gs as usize].selector.value,
                    )?;
                    self.write_new_stack_word(
                        &new_stack,
                        temp_sp.wrapping_sub(4) as u32,
                        cs_descriptor.dpl,
                        self.sregs[BxSegregs::Fs as usize].selector.value,
                    )?;
                    self.write_new_stack_word(
                        &new_stack,
                        temp_sp.wrapping_sub(6) as u32,
                        cs_descriptor.dpl,
                        self.sregs[BxSegregs::Ds as usize].selector.value,
                    )?;
                    self.write_new_stack_word(
                        &new_stack,
                        temp_sp.wrapping_sub(8) as u32,
                        cs_descriptor.dpl,
                        self.sregs[BxSegregs::Es as usize].selector.value,
                    )?;
                    temp_sp = temp_sp.wrapping_sub(8);
                }
            }

            // Push return frame (matches lines 600-627)
            if is_386_gate {
                self.write_new_stack_dword(
                    &new_stack,
                    temp_sp.wrapping_sub(4) as u32,
                    cs_descriptor.dpl,
                    old_ss as u32,
                )?;
                self.write_new_stack_dword(
                    &new_stack,
                    temp_sp.wrapping_sub(8) as u32,
                    cs_descriptor.dpl,
                    old_esp,
                )?;
                self.write_new_stack_dword(
                    &new_stack,
                    temp_sp.wrapping_sub(12) as u32,
                    cs_descriptor.dpl,
                    self.eflags.bits(),
                )?;
                self.write_new_stack_dword(
                    &new_stack,
                    temp_sp.wrapping_sub(16) as u32,
                    cs_descriptor.dpl,
                    old_cs as u32,
                )?;
                self.write_new_stack_dword(
                    &new_stack,
                    temp_sp.wrapping_sub(20) as u32,
                    cs_descriptor.dpl,
                    old_eip,
                )?;
                temp_sp = temp_sp.wrapping_sub(20);

                if push_error {
                    temp_sp = temp_sp.wrapping_sub(4);
                    self.write_new_stack_dword(
                        &new_stack,
                        temp_sp as u32,
                        cs_descriptor.dpl,
                        error_code as u32,
                    )?;
                }
            } else {
                // 286 gate
                self.write_new_stack_word(
                    &new_stack,
                    temp_sp.wrapping_sub(2) as u32,
                    cs_descriptor.dpl,
                    old_ss,
                )?;
                self.write_new_stack_word(
                    &new_stack,
                    temp_sp.wrapping_sub(4) as u32,
                    cs_descriptor.dpl,
                    old_esp as u16,
                )?;
                self.write_new_stack_word(
                    &new_stack,
                    temp_sp.wrapping_sub(6) as u32,
                    cs_descriptor.dpl,
                    (self.eflags.bits() & 0xFFFF) as u16,
                )?;
                self.write_new_stack_word(
                    &new_stack,
                    temp_sp.wrapping_sub(8) as u32,
                    cs_descriptor.dpl,
                    old_cs,
                )?;
                self.write_new_stack_word(
                    &new_stack,
                    temp_sp.wrapping_sub(10) as u32,
                    cs_descriptor.dpl,
                    old_eip as u16,
                )?;
                temp_sp = temp_sp.wrapping_sub(10);

                if push_error {
                    temp_sp = temp_sp.wrapping_sub(2);
                    self.write_new_stack_word(
                        &new_stack,
                        temp_sp as u32,
                        cs_descriptor.dpl,
                        error_code,
                    )?;
                }
            }

            self.set_sp(temp_sp);
        }

        // Load new CS:IP values from gate (matches Bochs exception.cc:635)
        // Must use load_cs() to properly update user_pl, touch accessed bit, and
        // invalidate prefetch queue. Manual CS cache assignment would leave user_pl
        // stale (e.g., still true after ring 3 → ring 0 transition), causing all
        // paging permission checks during the interrupt handler to use user-mode access.
        let mut new_cs_selector = cs_selector.clone();
        let mut new_cs_descriptor = cs_descriptor.clone();
        self.load_cs(
            &mut new_cs_selector,
            &mut new_cs_descriptor,
            cs_descriptor.dpl,
        )?;

        // Load new SS:ESP values from TSS (matches line 638)
        let mut new_ss_selector = new_stack.selector.clone();
        let mut ss_descriptor_mut = ss_descriptor.clone();
        self.load_ss(
            &mut new_ss_selector,
            &mut ss_descriptor_mut,
            cs_descriptor.dpl,
        )?;

        // Clear segment registers in v8086 mode (matches lines 655-665)
        if is_v8086_mode {
            self.sregs[BxSegregs::Gs as usize].cache.valid = 0;
            self.sregs[BxSegregs::Gs as usize].selector.value = 0;
            self.sregs[BxSegregs::Fs as usize].cache.valid = 0;
            self.sregs[BxSegregs::Fs as usize].selector.value = 0;
            self.sregs[BxSegregs::Ds as usize].cache.valid = 0;
            self.sregs[BxSegregs::Ds as usize].selector.value = 0;
            self.sregs[BxSegregs::Es as usize].cache.valid = 0;
            self.sregs[BxSegregs::Es as usize].selector.value = 0;
        }

        Ok(())
    }

    /// Handle interrupt in long mode via 16-byte IDT entries.
    /// Based on BX_CPU_C::long_mode_int in exception.cc:44-281
    pub(super) fn long_mode_int(
        &mut self,
        vector: u8,
        soft_int: bool,
        push_error: bool,
        error_code: u16,
    ) -> Result<()> {
        let idt_error_code = (vector as u16) * 8 + 2;

        // interrupt vector must be within IDT table limits (16-byte entries)
        // else #GP(vector*8 + 2 + EXT)
        if (vector as u64 * 16 + 15) > self.idtr.limit as u64 {
            tracing::error!(
                "long_mode_int(): vector must be within IDT table limits, IDT.limit = {:#x}",
                self.idtr.limit
            );
            return Err(super::error::CpuError::BadVector {
                vector: Exception::Gp,
                error_code: idt_error_code,
            });
        }

        // Read 16-byte IDT entry (two qwords)
        let idt_entry_addr = self.idtr.base + vector as u64 * 16;
        let desctmp1 = self.system_read_qword(idt_entry_addr)?;
        let desctmp2 = self.system_read_qword(idt_entry_addr + 8)?;

        // Bochs exception.cc:59-62 — extended attributes DWORD4 TYPE must be 0
        if desctmp2 & 0x00001F00_00000000u64 != 0 {
            tracing::error!("long_mode_int(): IDT entry extended attributes DWORD4 TYPE != 0");
            return Err(super::error::CpuError::BadVector {
                vector: Exception::Gp,
                error_code: idt_error_code,
            });
        }

        let dword1 = desctmp1 as u32;
        let dword2 = (desctmp1 >> 32) as u32;
        let dword3 = desctmp2 as u32;

        let gate_descriptor = self.parse_descriptor(dword1, dword2)?;

        if gate_descriptor.valid == 0 || gate_descriptor.segment {
            tracing::debug!(
                "long_mode_int(): gate descriptor is not valid sys seg: vector={} type={:#x} dword1={:#010x} dword2={:#010x} dword3={:#010x} idt_addr={:#x} icount={}",
                { vector }, gate_descriptor.r#type, dword1, dword2, dword3, idt_entry_addr, self.icount
            );
            return Err(super::error::CpuError::BadVector {
                vector: Exception::Gp,
                error_code: idt_error_code,
            });
        }

        // Must be 386 interrupt gate (0xE) or trap gate (0xF)
        // No task gates or 286 gates in long mode
        if gate_descriptor.r#type != 0xE && gate_descriptor.r#type != 0xF {
            tracing::error!(
                "long_mode_int(): unsupported gate type {:#x}",
                gate_descriptor.r#type
            );
            return Err(super::error::CpuError::BadVector {
                vector: Exception::Gp,
                error_code: idt_error_code,
            });
        }

        // if software interrupt, then gate descriptor DPL must be >= CPL
        let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
        if soft_int && gate_descriptor.dpl < cpl {
            tracing::error!("long_mode_int(): soft_int && gate.dpl < CPL");
            return Err(super::error::CpuError::BadVector {
                vector: Exception::Gp,
                error_code: idt_error_code,
            });
        }

        // Gate must be present
        if !gate_descriptor.p {
            tracing::error!("long_mode_int(): gate.p == 0");
            return Err(super::error::CpuError::BadVector {
                vector: Exception::Np,
                error_code: idt_error_code,
            });
        }

        // SAFETY: descriptor type verified as gate before union access
        let gate_dest_selector = gate_descriptor.u.gate_dest_selector();
        // 64-bit offset: low 16 bits from gate dword1, high 16 from gate dword2, upper 32 from dword3
        let gate_dest_offset = ((dword3 as u64) << 32)
            // SAFETY: descriptor type verified as gate before union access
            | (gate_descriptor.u.gate_dest_offset() as u64);

        // IST (Interrupt Stack Table) index from gate param_count bits 0-2
        // SAFETY: descriptor type verified as gate before union access
        let ist = gate_descriptor.u.gate_param_count() & 0x7;

        // CS selector must be non-null
        if (gate_dest_selector & 0xfffc) == 0 {
            tracing::error!("long_mode_int(): selector null");
            return Err(super::error::CpuError::BadVector {
                vector: Exception::Gp,
                error_code: 0,
            });
        }

        let mut cs_selector = BxSelector::default();
        parse_selector(gate_dest_selector, &mut cs_selector);

        let cs_err_code = cs_selector.value & 0xfffc;
        let (cs_dword1, cs_dword2) = match self.fetch_raw_descriptor(&cs_selector) {
            Ok(v) => v,
            Err(_) => {
                return Err(super::error::CpuError::BadVector {
                    vector: Exception::Gp,
                    error_code: cs_err_code,
                })
            }
        };
        let cs_descriptor = self.parse_descriptor(cs_dword1, cs_dword2).map_err(|_| {
            super::error::CpuError::BadVector {
                vector: Exception::Gp,
                error_code: cs_err_code,
            }
        })?;

        // Must be a valid code segment with DPL <= CPL
        if cs_descriptor.valid == 0
            || !cs_descriptor.segment
            || super::descriptor::is_data_segment(cs_descriptor.r#type)
            || cs_descriptor.dpl > cpl
        {
            tracing::error!("long_mode_int(): not accessible or not code segment");
            return Err(super::error::CpuError::BadVector {
                vector: Exception::Gp,
                error_code: cs_err_code,
            });
        }

        // Must be a 64-bit segment (L=1, D_B=0)
        if !cs_descriptor.u.segment_l() || cs_descriptor.u.segment_d_b() {
            tracing::error!("long_mode_int(): must be 64 bit segment");
            return Err(super::error::CpuError::BadVector {
                vector: Exception::Gp,
                error_code: cs_err_code,
            });
        }

        // Segment must be present
        if !cs_descriptor.p {
            tracing::error!("long_mode_int(): segment not present");
            return Err(super::error::CpuError::BadVector {
                vector: Exception::Np,
                error_code: cs_err_code,
            });
        }

        let old_cs = self.sregs[BxSegregs::Cs as usize].selector.value as u64;
        let old_rip = self.rip();
        let old_ss = self.sregs[BxSegregs::Ss as usize].selector.value as u64;
        let old_rsp = self.rsp();

        let rsp_for_cpl_x;

        if super::descriptor::is_code_segment_non_conforming(cs_descriptor.r#type)
            && cs_descriptor.dpl < cpl
        {
            // INTERRUPT TO INNER PRIVILEGE
            tracing::debug!("long_mode_int(): INTERRUPT TO INNER PRIVILEGE");

            if ist > 0 {
                tracing::debug!("long_mode_int(): trap to IST, vector = {}", ist);
                rsp_for_cpl_x = self.get_rsp_from_tss(ist + 3)?;
            } else {
                rsp_for_cpl_x = self.get_rsp_from_tss(cs_descriptor.dpl)?;
            }

            // Align stack to 16 bytes
            let mut rsp = rsp_for_cpl_x & !0xF;

            // Push old stack, flags, return address onto new stack
            self.write_new_stack_qword_64(rsp - 8, cs_descriptor.dpl, old_ss)?;
            self.write_new_stack_qword_64(rsp - 16, cs_descriptor.dpl, old_rsp)?;
            self.write_new_stack_qword_64(rsp - 24, cs_descriptor.dpl, self.eflags.bits() as u64)?;
            self.write_new_stack_qword_64(rsp - 32, cs_descriptor.dpl, old_cs)?;
            self.write_new_stack_qword_64(rsp - 40, cs_descriptor.dpl, old_rip)?;
            rsp -= 40;

            if push_error {
                rsp -= 8;
                self.write_new_stack_qword_64(rsp, cs_descriptor.dpl, error_code as u64)?;
            }

            // Load CS:RIP (guaranteed 64-bit mode)
            let mut cs_sel = cs_selector.clone();
            let mut cs_desc = cs_descriptor.clone();
            self.branch_far(&mut cs_sel, &mut cs_desc, gate_dest_offset, cs_descriptor.dpl)?;

            // Set up null SS descriptor
            self.load_null_selector(BxSegregs::Ss, cs_descriptor.dpl as u16);

            self.set_rsp(rsp);
        } else if super::descriptor::is_code_segment_conforming(cs_descriptor.r#type)
            || cs_descriptor.dpl == cpl
        {
            // INTERRUPT TO SAME PRIVILEGE LEVEL
            tracing::debug!("long_mode_int(): INTERRUPT TO SAME PRIVILEGE");

            if ist > 0 {
                tracing::debug!("long_mode_int(): trap to IST, vector = {}", ist);
                rsp_for_cpl_x = self.get_rsp_from_tss(ist + 3)?;
            } else {
                rsp_for_cpl_x = old_rsp;
            }

            // Align stack to 16 bytes
            let mut rsp = rsp_for_cpl_x & !0xF;

            // Push SS, RSP, RFLAGS, CS, RIP
            self.write_new_stack_qword_64(rsp - 8, cs_descriptor.dpl, old_ss)?;
            self.write_new_stack_qword_64(rsp - 16, cs_descriptor.dpl, old_rsp)?;
            self.write_new_stack_qword_64(rsp - 24, cs_descriptor.dpl, self.eflags.bits() as u64)?;
            self.write_new_stack_qword_64(rsp - 32, cs_descriptor.dpl, old_cs)?;
            self.write_new_stack_qword_64(rsp - 40, cs_descriptor.dpl, old_rip)?;
            rsp -= 40;

            if push_error {
                rsp -= 8;
                self.write_new_stack_qword_64(rsp, cs_descriptor.dpl, error_code as u64)?;
            }

            // set the RPL field of CS to CPL
            let mut cs_sel = cs_selector.clone();
            let mut cs_desc = cs_descriptor.clone();
            self.branch_far(&mut cs_sel, &mut cs_desc, gate_dest_offset, cpl)?;

            self.set_rsp(rsp);
        } else {
            tracing::error!(
                "long_mode_int(): bad descriptor type {:#x} (CS.DPL={} CPL={})",
                cs_descriptor.r#type,
                cs_descriptor.dpl,
                cpl
            );
            return Err(super::error::CpuError::BadVector {
                vector: Exception::Gp,
                error_code: cs_err_code,
            });
        }

        // if interrupt gate then set IF to 0
        if (gate_descriptor.r#type & 1) == 0 {
            self.eflags.remove(EFlags::IF_);
            self.handle_interrupt_mask_change();
        }
        self.eflags.remove(EFlags::TF);
        // VM is clear in long mode (already 0)
        self.eflags.remove(EFlags::RF);
        self.eflags.remove(EFlags::NT);

        Ok(())
    }

    /// Handle task gate - perform task switch
    /// Based on exception.cc:341-381
    fn handle_task_gate(
        &mut self,
        gate_descriptor: &BxDescriptor,
        _push_error: bool,
        _error_code: u16,
    ) -> Result<()> {
        // Examine selector to TSS, given in task gate descriptor (matches line 343)
        // SAFETY: descriptor type verified as task gate before union access
        let raw_tss_selector = gate_descriptor.u.task_gate_tss_selector();
        let mut tss_selector = BxSelector::default();
        parse_selector(raw_tss_selector, &mut tss_selector);

        // must specify global in the local/global bit, else #GP(TSS selector) (matches line 348)
        if tss_selector.ti != 0 {
            tracing::error!(
                "handle_task_gate(): tss_selector.ti=1 from gate descriptor - #GP(tss_selector)"
            );
            return Err(super::error::CpuError::BadVector {
                vector: Exception::Gp,
                error_code: 0,
            });
        }

        // index must be within GDT limits, else #TS(TSS selector) (matches line 354)
        let (tss_dword1, tss_dword2) = match self.fetch_raw_descriptor(&tss_selector) {
            Ok(v) => v,
            Err(_) => {
                return Err(super::error::CpuError::BadVector {
                    vector: Exception::Ts,
                    error_code: raw_tss_selector & 0xfffc,
                })
            }
        };
        let tss_descriptor = self.parse_descriptor(tss_dword1, tss_dword2).map_err(|_| {
            super::error::CpuError::BadVector {
                vector: Exception::Ts,
                error_code: raw_tss_selector & 0xfffc,
            }
        })?;

        // AR byte must specify available TSS, else #GP(TSS selector) (matches line 360)
        if tss_descriptor.valid == 0 || tss_descriptor.segment {
            tracing::error!(
                "handle_task_gate(): TSS selector points to invalid or bad TSS - #GP(tss_selector)"
            );
            return Err(super::error::CpuError::BadVector {
                vector: Exception::Gp,
                error_code: 0,
            });
        }

        // Check TSS type (matches line 365)
        let tss_type = tss_descriptor.r#type;
        if tss_type != 0x1 && tss_type != 0x9 {
            // Must be AVAIL_286_TSS (0x1) or AVAIL_386_TSS (0x9)
            tracing::error!("handle_task_gate(): TSS selector points to bad TSS type ({:#x}) - #GP(tss_selector)", tss_type);
            return Err(super::error::CpuError::BadVector {
                vector: Exception::Gp,
                error_code: 0,
            });
        }

        // TSS must be present, else #NP(TSS selector) (matches line 373)
        if !tss_descriptor.p {
            tracing::error!("handle_task_gate(): TSS descriptor.p == 0");
            return Err(super::error::CpuError::BadVector {
                vector: Exception::Np,
                error_code: 0,
            });
        }

        // Task switching is a complex operation that requires:
        // 1. Saving current task state to old TSS
        // 2. Loading new task state from new TSS
        // 3. Updating TR register
        // 4. Handling busy bit in TSS descriptors
        // 5. Handling task linking and nesting
        //
        // The full task_switch() function in Bochs is ~900 lines and handles all of this.
        // For now, return an error indicating task switching is not yet fully implemented.
        tracing::warn!(
            "handle_task_gate(): Task switching via Task Gate detected but not yet fully implemented. \
             TSS selector={:#04x}, type={:#x}. \
             Full implementation requires complete task_switch() function (~900 lines).",
            raw_tss_selector,
            tss_type
        );

        // Return a more specific error to indicate this is an unimplemented feature
        Err(super::error::CpuError::UnimplementedInstruction)
    }
}
