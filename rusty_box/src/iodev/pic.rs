//! 8259 PIC (Programmable Interrupt Controller) Emulation
//!
//! The 8259 PIC is a critical component for handling hardware interrupts.
//! A PC has two PICs: master (ports 0x20-0x21) and slave (ports 0xA0-0xA1).
//!
//! The slave PIC is cascaded to IRQ2 of the master, giving 15 usable IRQs:
//! - IRQ 0-7: Master PIC (IRQ2 is cascade)
//! - IRQ 8-15: Slave PIC

use core::ffi::c_void;

/// PIC I/O port addresses
pub const PIC_MASTER_CMD: u16 = 0x0020;
pub const PIC_MASTER_DATA: u16 = 0x0021;
pub const PIC_SLAVE_CMD: u16 = 0x00A0;
pub const PIC_SLAVE_DATA: u16 = 0x00A1;

/// Edge/Level Control Register ports (ELCR)
pub const PIC_ELCR1: u16 = 0x04D0;
pub const PIC_ELCR2: u16 = 0x04D1;

/// State for a single 8259 PIC chip
#[derive(Debug, Clone)]
pub struct Pic8259State {
    /// Is this the master PIC?
    pub master: bool,
    /// Programmable interrupt vector offset
    pub interrupt_offset: u8,
    /// Specially fully nested mode: 0=no, 1=yes
    pub sfnm: u8,
    /// Buffered mode: 0=no, 1=yes
    pub buffered_mode: u8,
    /// Master/slave: 0=slave PIC, 1=master PIC
    pub master_slave: u8,
    /// Auto EOI: 0=manual, 1=automatic
    pub auto_eoi: u8,
    /// Interrupt Mask Register (1=masked)
    pub imr: u8,
    /// In-Service Register
    pub isr: u8,
    /// Interrupt Request Register
    pub irr: u8,
    /// Read register select: 0=IRR, 1=ISR
    pub read_reg_select: u8,
    /// Current IRQ number
    pub irq: u8,
    /// Current lowest priority IRQ
    pub lowest_priority: u8,
    /// INT request pin of PIC
    pub int_pin: bool,
    /// IRQ pins of PIC (8 lines)
    pub irq_in: [u8; 8],
    /// Initialization state
    pub init: PicInitState,
    /// Special mask mode
    pub special_mask: bool,
    /// Poll command issued
    pub polled: bool,
    /// Rotate on auto-EOI
    pub rotate_on_autoeoi: bool,
    /// Edge/level trigger mode bitmap (0=edge, 1=level)
    pub edge_level: u8,
}

/// PIC initialization sequence state
#[derive(Debug, Clone, Default)]
pub struct PicInitState {
    /// Currently in initialization sequence
    pub in_init: bool,
    /// ICW4 required
    pub requires_4: bool,
    /// Which ICW byte is expected next (1-4)
    pub byte_expected: u8,
}

impl Default for Pic8259State {
    fn default() -> Self {
        Self {
            master: false,
            interrupt_offset: 0,
            sfnm: 0,
            buffered_mode: 0,
            master_slave: 0,
            auto_eoi: 0,
            imr: 0xFF, // All IRQs masked initially
            isr: 0,
            irr: 0,
            read_reg_select: 0,
            irq: 0,
            lowest_priority: 7,
            int_pin: false,
            irq_in: [0; 8],
            init: PicInitState::default(),
            special_mask: false,
            polled: false,
            rotate_on_autoeoi: false,
            edge_level: 0,
        }
    }
}

/// Dual 8259 PIC Controller (Master + Slave)
#[derive(Debug)]
pub struct BxPicC {
    /// Master PIC state
    pub master: Pic8259State,
    /// Slave PIC state  
    pub slave: Pic8259State,
    /// Edge/Level Control Registers
    pub elcr: [u8; 2],
}

impl Default for BxPicC {
    fn default() -> Self {
        Self::new()
    }
}

impl BxPicC {
    /// Create a new PIC controller
    pub fn new() -> Self {
        let mut master = Pic8259State::default();
        master.master = true;
        master.interrupt_offset = 0x08; // IRQ0 = INT 0x08
        master.master_slave = 1;

        let mut slave = Pic8259State::default();
        slave.master = false;
        slave.interrupt_offset = 0x70; // IRQ8 = INT 0x70
        slave.master_slave = 0;

        Self {
            master,
            slave,
            elcr: [0, 0],
        }
    }

    /// Initialize the PIC (called during device init)
    pub fn init(&mut self) {
        tracing::info!("PIC: Initializing 8259 Programmable Interrupt Controller");
        self.reset();
    }

    /// Reset the PIC to initial state
    pub fn reset(&mut self) {
        // Reset master PIC
        self.master.imr = 0xFF;
        self.master.isr = 0;
        self.master.irr = 0;
        self.master.int_pin = false;
        self.master.init = PicInitState::default();
        for i in 0..8 {
            self.master.irq_in[i] = 0;
        }

        // Reset slave PIC
        self.slave.imr = 0xFF;
        self.slave.isr = 0;
        self.slave.irr = 0;
        self.slave.int_pin = false;
        self.slave.init = PicInitState::default();
        for i in 0..8 {
            self.slave.irq_in[i] = 0;
        }

        self.elcr = [0, 0];
    }

    /// Read from PIC I/O port
    pub fn read(&self, port: u16, _io_len: u8) -> u32 {
        match port {
            PIC_MASTER_CMD => self.read_cmd(&self.master),
            PIC_MASTER_DATA => self.master.imr as u32,
            PIC_SLAVE_CMD => self.read_cmd(&self.slave),
            PIC_SLAVE_DATA => self.slave.imr as u32,
            PIC_ELCR1 => self.elcr[0] as u32,
            PIC_ELCR2 => self.elcr[1] as u32,
            _ => {
                tracing::warn!("PIC: Unknown read port {:#06x}", port);
                0xFF
            }
        }
    }

    /// Write to PIC I/O port
    pub fn write(&mut self, port: u16, value: u32, _io_len: u8) {
        let value = value as u8;
        match port {
            PIC_MASTER_CMD => self.write_cmd(&mut self.master.clone(), value, true),
            PIC_MASTER_DATA => self.write_data_master(value),
            PIC_SLAVE_CMD => self.write_cmd(&mut self.slave.clone(), value, false),
            PIC_SLAVE_DATA => self.write_data_slave(value),
            PIC_ELCR1 => {
                self.elcr[0] = value & 0xF8; // IRQ0-2 are edge-triggered only
                tracing::debug!("PIC: ELCR1 = {:#04x}", value);
            }
            PIC_ELCR2 => {
                self.elcr[1] = value & 0xDE; // IRQ8,13 are edge-triggered only
                tracing::debug!("PIC: ELCR2 = {:#04x}", value);
            }
            _ => {
                tracing::warn!("PIC: Unknown write port {:#06x} value={:#04x}", port, value);
            }
        }
    }

    fn read_cmd(&self, pic: &Pic8259State) -> u32 {
        if pic.polled {
            // Poll mode - return highest priority interrupt
            self.poll(pic)
        } else if pic.read_reg_select != 0 {
            pic.isr as u32
        } else {
            pic.irr as u32
        }
    }

    fn poll(&self, pic: &Pic8259State) -> u32 {
        // Find highest priority interrupt
        for i in 0..8 {
            let irq = ((pic.lowest_priority + 1 + i) & 7) as usize;
            if (pic.irr & (1 << irq)) != 0 && (pic.imr & (1 << irq)) == 0 {
                return 0x80 | irq as u32;
            }
        }
        0
    }

    fn write_cmd(&mut self, pic: &mut Pic8259State, value: u8, is_master: bool) {
        if (value & 0x10) != 0 {
            // ICW1 - Initialize
            tracing::debug!("PIC: ICW1 = {:#04x} ({})", value, if is_master { "master" } else { "slave" });
            pic.init.in_init = true;
            pic.init.requires_4 = (value & 0x01) != 0;
            pic.init.byte_expected = 2;
            pic.imr = 0;
            pic.isr = 0;
            pic.irr = 0;
            pic.int_pin = false;
            pic.special_mask = false;
            pic.read_reg_select = 0;
            
            if is_master {
                self.master = pic.clone();
            } else {
                self.slave = pic.clone();
            }
        } else if (value & 0x08) != 0 {
            // OCW3
            if (value & 0x02) != 0 {
                pic.read_reg_select = value & 0x01;
            }
            if (value & 0x04) != 0 {
                pic.polled = true;
            }
            if (value & 0x40) != 0 {
                pic.special_mask = (value & 0x20) != 0;
            }
            
            if is_master {
                self.master = pic.clone();
            } else {
                self.slave = pic.clone();
            }
        } else {
            // OCW2 - EOI commands
            let eoi_type = (value >> 5) & 0x07;
            match eoi_type {
                0b001 => {
                    // Non-specific EOI
                    self.clear_highest_interrupt(pic);
                    if is_master {
                        self.master = pic.clone();
                    } else {
                        self.slave = pic.clone();
                    }
                }
                0b011 => {
                    // Specific EOI
                    let irq = value & 0x07;
                    pic.isr &= !(1 << irq);
                    if is_master {
                        self.master = pic.clone();
                    } else {
                        self.slave = pic.clone();
                    }
                }
                0b101 => {
                    // Rotate on non-specific EOI
                    self.clear_highest_interrupt(pic);
                    pic.lowest_priority = pic.irq;
                    if is_master {
                        self.master = pic.clone();
                    } else {
                        self.slave = pic.clone();
                    }
                }
                0b111 => {
                    // Rotate on specific EOI
                    let irq = value & 0x07;
                    pic.isr &= !(1 << irq);
                    pic.lowest_priority = irq;
                    if is_master {
                        self.master = pic.clone();
                    } else {
                        self.slave = pic.clone();
                    }
                }
                0b110 => {
                    // Set priority
                    pic.lowest_priority = value & 0x07;
                    if is_master {
                        self.master = pic.clone();
                    } else {
                        self.slave = pic.clone();
                    }
                }
                0b100 => {
                    // Rotate in auto-EOI mode (set)
                    pic.rotate_on_autoeoi = true;
                    if is_master {
                        self.master = pic.clone();
                    } else {
                        self.slave = pic.clone();
                    }
                }
                0b000 => {
                    // Rotate in auto-EOI mode (clear)
                    pic.rotate_on_autoeoi = false;
                    if is_master {
                        self.master = pic.clone();
                    } else {
                        self.slave = pic.clone();
                    }
                }
                _ => {}
            }
        }
    }

    fn write_data_master(&mut self, value: u8) {
        if self.master.init.in_init {
            self.write_icw(&mut self.master.clone(), value, true);
        } else {
            // OCW1 - Set IMR
            self.master.imr = value;
            tracing::trace!("PIC: Master IMR = {:#04x}", value);
        }
    }

    fn write_data_slave(&mut self, value: u8) {
        if self.slave.init.in_init {
            self.write_icw(&mut self.slave.clone(), value, false);
        } else {
            // OCW1 - Set IMR
            self.slave.imr = value;
            tracing::trace!("PIC: Slave IMR = {:#04x}", value);
        }
    }

    fn write_icw(&mut self, pic: &mut Pic8259State, value: u8, is_master: bool) {
        match pic.init.byte_expected {
            2 => {
                // ICW2 - Interrupt vector offset
                pic.interrupt_offset = value & 0xF8;
                tracing::debug!("PIC: ICW2 = {:#04x} (offset = {:#04x})", value, pic.interrupt_offset);
                pic.init.byte_expected = 3;
            }
            3 => {
                // ICW3 - Cascade configuration
                tracing::debug!("PIC: ICW3 = {:#04x}", value);
                if pic.init.requires_4 {
                    pic.init.byte_expected = 4;
                } else {
                    pic.init.in_init = false;
                }
            }
            4 => {
                // ICW4 - Mode configuration
                pic.auto_eoi = (value >> 1) & 0x01;
                pic.buffered_mode = (value >> 2) & 0x01;
                pic.master_slave = (value >> 3) & 0x01;
                pic.sfnm = (value >> 4) & 0x01;
                tracing::debug!("PIC: ICW4 = {:#04x} (auto_eoi={})", value, pic.auto_eoi);
                pic.init.in_init = false;
            }
            _ => {}
        }
        
        if is_master {
            self.master = pic.clone();
        } else {
            self.slave = pic.clone();
        }
    }

    fn clear_highest_interrupt(&mut self, pic: &mut Pic8259State) {
        // Find highest priority interrupt in service
        for i in 0..8 {
            let irq = ((pic.lowest_priority + 1 + i) & 7) as u8;
            if (pic.isr & (1 << irq)) != 0 {
                pic.isr &= !(1 << irq);
                pic.irq = irq;
                return;
            }
        }
    }

    /// Raise an IRQ line
    pub fn raise_irq(&mut self, irq_no: u8) {
        if irq_no < 8 {
            // Master PIC
            if self.master.irq_in[irq_no as usize] == 0 {
                self.master.irq_in[irq_no as usize] = 1;
                self.master.irr |= 1 << irq_no;
                self.service_pic(&mut self.master.clone(), true);
            }
        } else if irq_no < 16 {
            // Slave PIC
            let slave_irq = irq_no - 8;
            if self.slave.irq_in[slave_irq as usize] == 0 {
                self.slave.irq_in[slave_irq as usize] = 1;
                self.slave.irr |= 1 << slave_irq;
                self.service_pic(&mut self.slave.clone(), false);
            }
        }
    }

    /// Lower an IRQ line
    pub fn lower_irq(&mut self, irq_no: u8) {
        if irq_no < 8 {
            self.master.irq_in[irq_no as usize] = 0;
            // For edge-triggered, clear IRR when line goes low
            if (self.master.edge_level & (1 << irq_no)) == 0 {
                self.master.irr &= !(1 << irq_no);
            }
        } else if irq_no < 16 {
            let slave_irq = irq_no - 8;
            self.slave.irq_in[slave_irq as usize] = 0;
            if (self.slave.edge_level & (1 << slave_irq)) == 0 {
                self.slave.irr &= !(1 << slave_irq);
            }
        }
    }

    fn service_pic(&mut self, pic: &mut Pic8259State, is_master: bool) {
        // Find highest priority unmasked interrupt
        for i in 0..8 {
            let irq = ((pic.lowest_priority + 1 + i) & 7) as u8;
            if (pic.irr & (1 << irq)) != 0 && (pic.imr & (1 << irq)) == 0 {
                // Check if higher priority interrupt is in service
                let mut in_service = false;
                for j in 0..i {
                    let higher_irq = ((pic.lowest_priority + 1 + j) & 7) as u8;
                    if (pic.isr & (1 << higher_irq)) != 0 {
                        in_service = true;
                        break;
                    }
                }
                
                if !in_service {
                    pic.int_pin = true;
                    pic.irq = irq;
                    
                    if is_master {
                        self.master = pic.clone();
                    } else {
                        // Slave needs to signal master via IRQ2
                        self.slave = pic.clone();
                        self.raise_irq(2);
                    }
                    return;
                }
            }
        }
        
        pic.int_pin = false;
        if is_master {
            self.master = pic.clone();
        } else {
            self.slave = pic.clone();
        }
    }

    /// Check if an interrupt is pending
    pub fn has_interrupt(&self) -> bool {
        self.master.int_pin
    }

    /// Acknowledge interrupt and get vector (called during INTA cycle)
    pub fn iac(&mut self) -> u8 {
        // First check slave PIC (if cascade)
        if self.master.irq == 2 && self.slave.int_pin {
            let vector = self.slave.interrupt_offset + self.slave.irq;
            self.slave.irr &= !(1 << self.slave.irq);
            self.slave.isr |= 1 << self.slave.irq;
            self.slave.int_pin = false;
            
            if self.slave.auto_eoi != 0 {
                self.slave.isr &= !(1 << self.slave.irq);
            }
            
            tracing::trace!("PIC: IAC slave vector = {:#04x}", vector);
            return vector;
        }

        // Master PIC
        let vector = self.master.interrupt_offset + self.master.irq;
        self.master.irr &= !(1 << self.master.irq);
        self.master.isr |= 1 << self.master.irq;
        self.master.int_pin = false;
        
        if self.master.auto_eoi != 0 {
            self.master.isr &= !(1 << self.master.irq);
        }
        
        tracing::trace!("PIC: IAC master vector = {:#04x}", vector);
        vector
    }
}

/// PIC read handler for I/O port infrastructure
pub fn pic_read_handler(this_ptr: *mut c_void, port: u16, io_len: u8) -> u32 {
    let pic = unsafe { &*(this_ptr as *const BxPicC) };
    pic.read(port, io_len)
}

/// PIC write handler for I/O port infrastructure
pub fn pic_write_handler(this_ptr: *mut c_void, port: u16, value: u32, io_len: u8) {
    let pic = unsafe { &mut *(this_ptr as *mut BxPicC) };
    pic.write(port, value, io_len);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pic_creation() {
        let pic = BxPicC::new();
        assert!(pic.master.master);
        assert!(!pic.slave.master);
        assert_eq!(pic.master.interrupt_offset, 0x08);
        assert_eq!(pic.slave.interrupt_offset, 0x70);
    }

    #[test]
    fn test_pic_imr() {
        let mut pic = BxPicC::new();
        
        // Write to IMR
        pic.write(PIC_MASTER_DATA, 0x00, 1);
        assert_eq!(pic.master.imr, 0x00);
        
        pic.write(PIC_MASTER_DATA, 0xFF, 1);
        assert_eq!(pic.master.imr, 0xFF);
    }

    #[test]
    fn test_pic_irq() {
        let mut pic = BxPicC::new();
        pic.master.imr = 0x00; // Unmask all
        
        pic.raise_irq(0);
        assert!(pic.has_interrupt());
        
        let vector = pic.iac();
        assert_eq!(vector, 0x08); // IRQ0 -> INT 0x08
    }
}

