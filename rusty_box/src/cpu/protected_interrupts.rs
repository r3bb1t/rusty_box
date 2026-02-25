//! Protected mode interrupt handling
//!
//! Based on Bochs cpu/exception.cc protected_mode_int
//! Copyright (C) 2001-2019 The Bochs Project

use super::{
    cpu::{BxCpuC, CpuMode, Exception},
    cpuid::BxCpuIdTrait,
    decoder::BxSegregs,
    descriptor::{
        BxDescriptor, BxSelector, BxSegmentReg, SystemAndGateDescriptorEnum, SEG_VALID_CACHE,
    },
    segment_ctrl_pro::{parse_selector, self},
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
        // interrupt vector must be within IDT table limits, else #GP(vector*8 + 2 + EXT)
        if (vector as u64 * 8 + 7) > self.idtr.limit as u64 {
            tracing::error!(
                "protected_mode_int(): vector must be within IDT table limits, IDT.limit = {:#x}",
                self.idtr.limit
            );
            return Err(super::error::CpuError::BadVector { vector: Exception::Gp });
        }

        let raw_descriptor = self.system_read_qword(self.idtr.base + vector as u64 * 8)?;
        let dword1 = raw_descriptor as u32;
        let dword2 = (raw_descriptor >> 32) as u32;

        let mut gate_descriptor = self.parse_descriptor(dword1, dword2)?;

        if gate_descriptor.valid == 0 || gate_descriptor.segment {
            tracing::error!(
                "protected_mode_int(): gate descriptor is not valid sys seg (vector={:#04x})",
                vector
            );
            return Err(super::error::CpuError::BadVector { vector: Exception::Gp });
        }

        // descriptor AR byte must indicate interrupt gate, trap gate, or task gate, else #GP(vector*8 + 2 + EXT)
        let gate_type = gate_descriptor.r#type;
        let is_valid_gate = matches!(gate_type, 
            0x5 | 0x6 | 0x7 | 0xE | 0xF
        );
        
        if !is_valid_gate {
            tracing::error!(
                "protected_mode_int(): gate.type({:#x}) != {{5,6,7,14,15}}",
                gate_type
            );
            return Err(super::error::CpuError::BadVector { vector: Exception::Gp });
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
                return Err(super::error::CpuError::BadVector { vector: Exception::Gp });
            }
        }

        // if software interrupt, then gate descriptor DPL must be >= CPL, else #GP(vector * 8 + 2 + EXT)
        let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
        if soft_int && gate_descriptor.dpl < cpl {
            tracing::error!("protected_mode_int(): soft_int && (gate.dpl < CPL)");
            return Err(super::error::CpuError::BadVector { vector: Exception::Gp });
        }

        // Gate must be present, else #NP(vector * 8 + 2 + EXT)
        if !gate_descriptor.p {
            tracing::error!("protected_mode_int(): gate not present");
            return Err(super::error::CpuError::BadVector { vector: Exception::Np });
        }

        match gate_type_enum {
            SystemAndGateDescriptorEnum::BxTaskGate => {
                // Task switch via Task Gate (matches original lines 341-381)
                let raw_tss_selector = unsafe { gate_descriptor.u.task_gate.tss_selector };
                let mut tss_selector = BxSelector::default();
                parse_selector(raw_tss_selector, &mut tss_selector);
                
                let (tss_dword1, tss_dword2) = self.fetch_raw_descriptor(&tss_selector)?;
                let tss_descriptor = self.parse_descriptor(tss_dword1, tss_dword2)?;
                
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
            }
            SystemAndGateDescriptorEnum::Bx286InterruptGate
            | SystemAndGateDescriptorEnum::Bx286TrapGate
            | SystemAndGateDescriptorEnum::Bx386InterruptGate
            | SystemAndGateDescriptorEnum::Bx386TrapGate => {
                let gate_dest_offset = unsafe { gate_descriptor.u.gate.dest_offset };
                self.handle_interrupt_trap_gate(
                    &gate_descriptor,
                    push_error,
                    error_code,
                )?;
                // Set EIP after handling the gate (matches original line 714)
                self.set_eip(gate_dest_offset);
            }
            _ => {
                return Err(super::error::CpuError::BadVector { vector: Exception::Gp });
            }
        }

        // if interrupt gate then set IF to 0
        if (gate_descriptor.r#type & 1) == 0 {
            // even is int-gate
            self.eflags &= !(1 << 9); // Clear IF flag
        }
        self.eflags &= !(1 << 8); // Clear TF flag
        self.eflags &= !(1 << 14); // Clear NT flag
        self.eflags &= !(1 << 17); // Clear VM flag
        self.eflags &= !(1 << 16); // Clear RF flag

        Ok(())
    }

    /// Handle interrupt/trap gate (not task gate)
    fn handle_interrupt_trap_gate(
        &mut self,
        gate_descriptor: &BxDescriptor,
        push_error: bool,
        error_code: u16,
    ) -> Result<()> {
        let gate_dest_selector = unsafe { gate_descriptor.u.gate.dest_selector };
        let gate_dest_offset = unsafe { gate_descriptor.u.gate.dest_offset };

        if (gate_dest_selector & 0xfffc) == 0 {
            tracing::error!("handle_interrupt_trap_gate(): selector null");
            return Err(super::error::CpuError::BadVector { vector: Exception::Gp });
        }

        let mut cs_selector = BxSelector::default();
        parse_selector(gate_dest_selector, &mut cs_selector);

        let (cs_dword1, cs_dword2) = self.fetch_raw_descriptor(&cs_selector)?;
        let cs_descriptor = self.parse_descriptor(cs_dword1, cs_dword2)?;

        let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
        if cs_descriptor.valid == 0
            || !cs_descriptor.segment
            || super::descriptor::is_data_segment(cs_descriptor.r#type)
            || cs_descriptor.dpl > cpl
        {
            tracing::error!(
                "handle_interrupt_trap_gate(): not accessible or not code segment cs={:#04x}",
                cs_selector.value
            );
            return Err(super::error::CpuError::BadVector { vector: Exception::Gp });
        }

        if !cs_descriptor.p {
            tracing::error!("handle_interrupt_trap_gate(): segment not present");
            return Err(super::error::CpuError::BadVector { vector: Exception::Np });
        }

        // Save old register values (matches original lines 421-424)
        let old_esp = self.esp();
        let old_ss = self.sregs[BxSegregs::Ss as usize].selector.value;
        let old_eip = self.eip();
        let old_cs = self.sregs[BxSegregs::Cs as usize].selector.value;

        // For now, handle same privilege level case (simplest)
        // TODO: Implement inner privilege level case
        if super::descriptor::is_code_segment_conforming(cs_descriptor.r#type) || cs_descriptor.dpl == cpl {
            // INTERRUPT TO SAME PRIVILEGE LEVEL
            tracing::debug!("handle_interrupt_trap_gate(): INTERRUPT TO SAME PRIVILEGE");

            // v8086 mode check (matches original lines 671-675)
            if self.v8086_mode() && (super::descriptor::is_code_segment_conforming(cs_descriptor.r#type) || cs_descriptor.dpl != 0) {
                tracing::error!("handle_interrupt_trap_gate(): code segment conforming or DPL({}) != 0 in v8086 mode", cs_descriptor.dpl);
                return Err(super::error::CpuError::BadVector { vector: Exception::Gp });
            }

            // EIP must be in CS limit else #GP(0) (matches original line 678)
            if gate_dest_offset > unsafe { cs_descriptor.u.segment.limit_scaled } {
                tracing::error!("handle_interrupt_trap_gate(): IP > CS descriptor limit");
                return Err(super::error::CpuError::BadVector { vector: Exception::Gp });
            }

            // Check if 386 gate (type >= 14) - matches original line 686
            let is_386_gate = gate_descriptor.r#type >= 14;

            if is_386_gate {
                self.push_32(self.eflags)?;
                self.push_32(self.sregs[BxSegregs::Cs as usize].selector.value as u32)?;
                self.push_32(self.eip())?;
                if push_error {
                    self.push_32(error_code as u32)?;
                }
            } else {
                self.push_16((self.eflags & 0xFFFF) as u16)?;
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
            
            self.sregs[BxSegregs::Cs as usize].selector = new_cs_selector;
            self.sregs[BxSegregs::Cs as usize].cache = cs_descriptor;
            self.sregs[BxSegregs::Cs as usize].cache.valid = SEG_VALID_CACHE;
            
            // Invalidate prefetch queue when CS changes
            self.eip_fetch_ptr = None;
            self.eip_page_window_size = 0;
        } else if super::descriptor::is_code_segment_non_conforming(cs_descriptor.r#type) && cs_descriptor.dpl < cpl {
            // INTERRUPT TO INNER PRIVILEGE
            self.handle_interrupt_to_inner_privilege(
                &gate_descriptor,
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
            return Err(super::error::CpuError::BadVector { vector: Exception::Gp });
        }

        // EIP is set in the caller after this function returns
        // (matches original where EIP is set at line 714)

        Ok(())
    }

    /// Handle interrupt to inner privilege level (DPL < CPL)
    /// Based on exception.cc:435-665
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
        if is_v8086_mode && cs_descriptor.dpl != 0 {
            tracing::error!("handle_interrupt_to_inner_privilege(): code segment DPL({}) != 0 in v8086 mode", cs_descriptor.dpl);
            return Err(super::error::CpuError::BadVector { vector: Exception::Gp });
        }

        // Selector must be non-null else #TS(EXT) (matches line 455)
        if (ss_for_cpl_x & 0xfffc) == 0 {
            tracing::error!("handle_interrupt_to_inner_privilege(): SS selector null");
            return Err(super::error::CpuError::BadVector { vector: Exception::Ts });
        }

        // Parse SS selector and fetch descriptor (matches lines 462-465)
        let mut ss_selector = BxSelector::default();
        parse_selector(ss_for_cpl_x, &mut ss_selector);
        let (ss_dword1, ss_dword2) = self.fetch_raw_descriptor(&ss_selector)?;
        let ss_descriptor = self.parse_descriptor(ss_dword1, ss_dword2)?;

        // selector rpl must = dpl of code segment, else #TS(SS selector + ext) (matches line 469)
        if ss_selector.rpl != cs_descriptor.dpl {
            tracing::error!("handle_interrupt_to_inner_privilege(): SS.rpl != CS.dpl");
            return Err(super::error::CpuError::BadVector { vector: Exception::Ts });
        }

        // stack seg DPL must = DPL of code segment, else #TS(SS selector + ext) (matches line 476)
        if ss_descriptor.dpl != cs_descriptor.dpl {
            tracing::error!("handle_interrupt_to_inner_privilege(): SS.dpl != CS.dpl");
            return Err(super::error::CpuError::BadVector { vector: Exception::Ts });
        }

        // descriptor must indicate writable data segment, else #TS(SS selector + EXT) (matches line 483)
        if ss_descriptor.valid == 0
            || !ss_descriptor.segment
            || super::descriptor::is_code_segment(ss_descriptor.r#type)
            || !super::descriptor::is_data_segment_writable(ss_descriptor.r#type)
        {
            tracing::error!("handle_interrupt_to_inner_privilege(): SS is not writable data segment");
            return Err(super::error::CpuError::BadVector { vector: Exception::Ts });
        }

        // seg must be present, else #SS(SS selector + ext) (matches line 492)
        if !ss_descriptor.p {
            tracing::error!("handle_interrupt_to_inner_privilege(): SS not present");
            return Err(super::error::CpuError::BadVector { vector: Exception::Ss });
        }

        // IP must be within CS segment boundaries, else #GP(0) (matches line 498)
        let gate_dest_offset = unsafe { gate_descriptor.u.gate.dest_offset };
        if gate_dest_offset > unsafe { cs_descriptor.u.segment.limit_scaled } {
            tracing::error!("handle_interrupt_to_inner_privilege(): gate EIP > CS.limit");
            return Err(super::error::CpuError::BadVector { vector: Exception::Gp });
        }

        // Prepare new stack segment (matches lines 503-509)
        let mut new_stack = BxSegmentReg {
            selector: ss_selector,
            cache: ss_descriptor.clone(),
        };
        new_stack.selector.rpl = cs_descriptor.dpl;
        new_stack.selector.value = (new_stack.selector.value & 0xfffc) | new_stack.selector.rpl as u16;

        let is_386_gate = gate_descriptor.r#type >= 14;

        // Build stack frame on new stack (matches lines 511-630)
        if unsafe { new_stack.cache.u.segment.d_b } {
            // 32-bit stack
            let mut temp_esp = esp_for_cpl_x;

            // Push segment registers for v8086 mode (matches lines 514-538)
            if is_v8086_mode {
                if is_386_gate {
                    self.write_new_stack_dword(&new_stack, temp_esp.wrapping_sub(4), cs_descriptor.dpl, 
                        self.sregs[BxSegregs::Gs as usize].selector.value as u32)?;
                    self.write_new_stack_dword(&new_stack, temp_esp.wrapping_sub(8), cs_descriptor.dpl,
                        self.sregs[BxSegregs::Fs as usize].selector.value as u32)?;
                    self.write_new_stack_dword(&new_stack, temp_esp.wrapping_sub(12), cs_descriptor.dpl,
                        self.sregs[BxSegregs::Ds as usize].selector.value as u32)?;
                    self.write_new_stack_dword(&new_stack, temp_esp.wrapping_sub(16), cs_descriptor.dpl,
                        self.sregs[BxSegregs::Es as usize].selector.value as u32)?;
                    temp_esp = temp_esp.wrapping_sub(16);
                } else {
                    self.write_new_stack_word(&new_stack, temp_esp.wrapping_sub(2), cs_descriptor.dpl,
                        self.sregs[BxSegregs::Gs as usize].selector.value)?;
                    self.write_new_stack_word(&new_stack, temp_esp.wrapping_sub(4), cs_descriptor.dpl,
                        self.sregs[BxSegregs::Fs as usize].selector.value)?;
                    self.write_new_stack_word(&new_stack, temp_esp.wrapping_sub(6), cs_descriptor.dpl,
                        self.sregs[BxSegregs::Ds as usize].selector.value)?;
                    self.write_new_stack_word(&new_stack, temp_esp.wrapping_sub(8), cs_descriptor.dpl,
                        self.sregs[BxSegregs::Es as usize].selector.value)?;
                    temp_esp = temp_esp.wrapping_sub(8);
                }
            }

            // Push return frame (matches lines 540-567)
            if is_386_gate {
                self.write_new_stack_dword(&new_stack, temp_esp.wrapping_sub(4), cs_descriptor.dpl, old_ss as u32)?;
                self.write_new_stack_dword(&new_stack, temp_esp.wrapping_sub(8), cs_descriptor.dpl, old_esp)?;
                self.write_new_stack_dword(&new_stack, temp_esp.wrapping_sub(12), cs_descriptor.dpl, self.eflags)?;
                self.write_new_stack_dword(&new_stack, temp_esp.wrapping_sub(16), cs_descriptor.dpl, old_cs as u32)?;
                self.write_new_stack_dword(&new_stack, temp_esp.wrapping_sub(20), cs_descriptor.dpl, old_eip)?;
                temp_esp = temp_esp.wrapping_sub(20);

                if push_error {
                    temp_esp = temp_esp.wrapping_sub(4);
                    self.write_new_stack_dword(&new_stack, temp_esp, cs_descriptor.dpl, error_code as u32)?;
                }
            } else {
                // 286 gate
                self.write_new_stack_word(&new_stack, temp_esp.wrapping_sub(2), cs_descriptor.dpl, old_ss)?;
                self.write_new_stack_word(&new_stack, temp_esp.wrapping_sub(4), cs_descriptor.dpl, old_esp as u16)?;
                self.write_new_stack_word(&new_stack, temp_esp.wrapping_sub(6), cs_descriptor.dpl, (self.eflags & 0xFFFF) as u16)?;
                self.write_new_stack_word(&new_stack, temp_esp.wrapping_sub(8), cs_descriptor.dpl, old_cs)?;
                self.write_new_stack_word(&new_stack, temp_esp.wrapping_sub(10), cs_descriptor.dpl, old_eip as u16)?;
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
                    self.write_new_stack_dword(&new_stack, temp_sp.wrapping_sub(4) as u32, cs_descriptor.dpl,
                        self.sregs[BxSegregs::Gs as usize].selector.value as u32)?;
                    self.write_new_stack_dword(&new_stack, temp_sp.wrapping_sub(8) as u32, cs_descriptor.dpl,
                        self.sregs[BxSegregs::Fs as usize].selector.value as u32)?;
                    self.write_new_stack_dword(&new_stack, temp_sp.wrapping_sub(12) as u32, cs_descriptor.dpl,
                        self.sregs[BxSegregs::Ds as usize].selector.value as u32)?;
                    self.write_new_stack_dword(&new_stack, temp_sp.wrapping_sub(16) as u32, cs_descriptor.dpl,
                        self.sregs[BxSegregs::Es as usize].selector.value as u32)?;
                    temp_sp = temp_sp.wrapping_sub(16);
                } else {
                    self.write_new_stack_word(&new_stack, temp_sp.wrapping_sub(2) as u32, cs_descriptor.dpl,
                        self.sregs[BxSegregs::Gs as usize].selector.value)?;
                    self.write_new_stack_word(&new_stack, temp_sp.wrapping_sub(4) as u32, cs_descriptor.dpl,
                        self.sregs[BxSegregs::Fs as usize].selector.value)?;
                    self.write_new_stack_word(&new_stack, temp_sp.wrapping_sub(6) as u32, cs_descriptor.dpl,
                        self.sregs[BxSegregs::Ds as usize].selector.value)?;
                    self.write_new_stack_word(&new_stack, temp_sp.wrapping_sub(8) as u32, cs_descriptor.dpl,
                        self.sregs[BxSegregs::Es as usize].selector.value)?;
                    temp_sp = temp_sp.wrapping_sub(8);
                }
            }

            // Push return frame (matches lines 600-627)
            if is_386_gate {
                self.write_new_stack_dword(&new_stack, temp_sp.wrapping_sub(4) as u32, cs_descriptor.dpl, old_ss as u32)?;
                self.write_new_stack_dword(&new_stack, temp_sp.wrapping_sub(8) as u32, cs_descriptor.dpl, old_esp)?;
                self.write_new_stack_dword(&new_stack, temp_sp.wrapping_sub(12) as u32, cs_descriptor.dpl, self.eflags)?;
                self.write_new_stack_dword(&new_stack, temp_sp.wrapping_sub(16) as u32, cs_descriptor.dpl, old_cs as u32)?;
                self.write_new_stack_dword(&new_stack, temp_sp.wrapping_sub(20) as u32, cs_descriptor.dpl, old_eip)?;
                temp_sp = temp_sp.wrapping_sub(20);

                if push_error {
                    temp_sp = temp_sp.wrapping_sub(4);
                    self.write_new_stack_dword(&new_stack, temp_sp as u32, cs_descriptor.dpl, error_code as u32)?;
                }
            } else {
                // 286 gate
                self.write_new_stack_word(&new_stack, temp_sp.wrapping_sub(2) as u32, cs_descriptor.dpl, old_ss)?;
                self.write_new_stack_word(&new_stack, temp_sp.wrapping_sub(4) as u32, cs_descriptor.dpl, old_esp as u16)?;
                self.write_new_stack_word(&new_stack, temp_sp.wrapping_sub(6) as u32, cs_descriptor.dpl, (self.eflags & 0xFFFF) as u16)?;
                self.write_new_stack_word(&new_stack, temp_sp.wrapping_sub(8) as u32, cs_descriptor.dpl, old_cs)?;
                self.write_new_stack_word(&new_stack, temp_sp.wrapping_sub(10) as u32, cs_descriptor.dpl, old_eip as u16)?;
                temp_sp = temp_sp.wrapping_sub(10);

                if push_error {
                    temp_sp = temp_sp.wrapping_sub(2);
                    self.write_new_stack_word(&new_stack, temp_sp as u32, cs_descriptor.dpl, error_code)?;
                }
            }

            self.set_sp(temp_sp);
        }

        // Load new CS:IP values from gate (matches line 635)
        let mut new_cs_selector = cs_selector.clone();
        new_cs_selector.rpl = cs_descriptor.dpl;
        new_cs_selector.value = (new_cs_selector.value & !0x03) | cs_descriptor.dpl as u16;
        self.sregs[BxSegregs::Cs as usize].selector = new_cs_selector;
        self.sregs[BxSegregs::Cs as usize].cache = cs_descriptor.clone();
        self.sregs[BxSegregs::Cs as usize].cache.valid = SEG_VALID_CACHE;
        self.eip_fetch_ptr = None;
        self.eip_page_window_size = 0;

        // Load new SS:ESP values from TSS (matches line 638)
        let mut new_ss_selector = new_stack.selector.clone();
        let mut ss_descriptor_mut = ss_descriptor.clone();
        self.load_ss(&mut new_ss_selector, &mut ss_descriptor_mut, cs_descriptor.dpl)?;

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

    /// Handle task gate - perform task switch
    /// Based on exception.cc:341-381
    fn handle_task_gate(
        &mut self,
        gate_descriptor: &BxDescriptor,
        _push_error: bool,
        _error_code: u16,
    ) -> Result<()> {
        // Examine selector to TSS, given in task gate descriptor (matches line 343)
        let raw_tss_selector = unsafe { gate_descriptor.u.task_gate.tss_selector };
        let mut tss_selector = BxSelector::default();
        parse_selector(raw_tss_selector, &mut tss_selector);

        // must specify global in the local/global bit, else #GP(TSS selector) (matches line 348)
        if tss_selector.ti != 0 {
            tracing::error!("handle_task_gate(): tss_selector.ti=1 from gate descriptor - #GP(tss_selector)");
            return Err(super::error::CpuError::BadVector { vector: Exception::Gp });
        }

        // index must be within GDT limits, else #TS(TSS selector) (matches line 354)
        let (tss_dword1, tss_dword2) = self.fetch_raw_descriptor(&tss_selector)?;
        let tss_descriptor = self.parse_descriptor(tss_dword1, tss_dword2)?;

        // AR byte must specify available TSS, else #GP(TSS selector) (matches line 360)
        if tss_descriptor.valid == 0 || tss_descriptor.segment {
            tracing::error!("handle_task_gate(): TSS selector points to invalid or bad TSS - #GP(tss_selector)");
            return Err(super::error::CpuError::BadVector { vector: Exception::Gp });
        }

        // Check TSS type (matches line 365)
        let tss_type = tss_descriptor.r#type;
        if tss_type != 0x1 && tss_type != 0x9 {
            // Must be AVAIL_286_TSS (0x1) or AVAIL_386_TSS (0x9)
            tracing::error!("handle_task_gate(): TSS selector points to bad TSS type ({:#x}) - #GP(tss_selector)", tss_type);
            return Err(super::error::CpuError::BadVector { vector: Exception::Gp });
        }

        // TSS must be present, else #NP(TSS selector) (matches line 373)
        if !tss_descriptor.p {
            tracing::error!("handle_task_gate(): TSS descriptor.p == 0");
            return Err(super::error::CpuError::BadVector { vector: Exception::Np });
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
