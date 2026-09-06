//! The machine's interrupt fabric.
//!
//! Bochs routes an interrupt through free functions over globals: a device
//! calls `DEV_pic_raise_irq`, and `bx_pic_c::raise_irq` calls
//! `DEV_ioapic_set_irq_level` in the same breath, so the 8259 pair and the I/O
//! APIC never disagree about a line even for one instruction. A port that owns
//! its parts cannot do that from inside the 8259: the two controllers are
//! sibling fields, and whoever holds one mutably cannot reach the other.
//!
//! [`IrqFabric`] owns both, which is what restores the synchronous edge. A
//! device drives an ISA line through the fabric; the fabric drives the 8259 and
//! then the I/O APIC pin the 8259 says the edge implies, inside the one call.
//!
//! What the fabric deliberately does NOT own is the Local APICs: those live on
//! the CPUs. An I/O APIC message therefore queues here and the machine routes
//! it at the next boundary — see [`BxIoApic::take_pending_deliveries`].

use rusty_box_devices::api::{IrqLine, IrqSink};

use super::ioapic::{BxIoApic, IoApicDeliveryMode, PendingIoApicDelivery};
use crate::pic::BxPicC;

/// How an I/O APIC entry's interrupt is asserted.
///
/// Bochs ioapic.h `bx_io_redirect_entry_t::trigger_mode` — bit 15 of the low
/// word, one bit with two meanings, so it is two states here (R2).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IoApicTrigger {
    /// Delivered once per assertion.
    Edge,
    /// Held until the destination writes EOI; the entry's remote-IRR mirrors it.
    Level,
}

impl IoApicTrigger {
    /// The redirection entry's own encoding: 0 edge, 1 level.
    #[must_use]
    pub const fn from_raw(raw: u8) -> Self {
        if raw & 1 == 0 {
            Self::Edge
        } else {
            Self::Level
        }
    }
}

/// How an I/O APIC entry's destination field names its target.
///
/// Bochs ioapic.h `bx_io_redirect_entry_t::destination_mode` — bit 11 of the
/// low word.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IoApicDestinationMode {
    /// The destination is a Local APIC ID.
    Physical,
    /// The destination is a mask matched against each Local APIC's logical
    /// destination register.
    Logical,
}

impl IoApicDestinationMode {
    /// The redirection entry's own encoding: 0 physical, 1 logical.
    #[must_use]
    pub const fn from_raw(raw: u8) -> Self {
        if raw & 1 == 0 {
            Self::Physical
        } else {
            Self::Logical
        }
    }
}

/// One I/O APIC message, as whoever delivers it needs to see it.
///
/// The queued record the machine drains carries two more fields — the pin it
/// came from, and whether its vector still owes an 8259 acknowledge — and both
/// are the fabric's own bookkeeping, settled before anything can be delivered.
/// What is left is the message itself, which is why this is the shape offered
/// to an engine whose backend owns the Local APICs instead of this machine.
///
/// Every field that has more than one meaning is the type of that meaning (R2):
/// a backend must branch on all three modes, and this record crosses a crate
/// boundary to reach one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct IoApicDelivery {
    /// Interrupt vector — Bochs ioapic.h `bx_io_redirect_entry_t::vector`.
    /// Ignored by the SMI, NMI and INIT delivery modes, and supplied by the
    /// 8259's own acknowledge for ExtINT.
    pub vector: u8,
    /// What kind of message this is: fixed, lowest-priority, SMI, NMI, INIT or
    /// ExtINT.
    pub delivery_mode: IoApicDeliveryMode,
    /// Whether the line is edge- or level-triggered.
    pub trigger_mode: IoApicTrigger,
    /// The entry's destination field, read as [`Self::dest_mode`] says.
    pub dest: u32,
    /// How `dest` names its target.
    pub dest_mode: IoApicDestinationMode,
}

impl IoApicDelivery {
    /// The routable part of a queued message.
    ///
    /// The one conversion (R5), so the two records cannot drift apart field by
    /// field, and the one place the entry's raw encodings become states.
    #[inline]
    pub(crate) fn from_pending(pending: PendingIoApicDelivery) -> Self {
        Self {
            vector: pending.vector,
            delivery_mode: IoApicDeliveryMode::from_raw(pending.delivery_mode),
            trigger_mode: IoApicTrigger::from_raw(pending.trigger_mode),
            dest: pending.dest,
            dest_mode: IoApicDestinationMode::from_raw(pending.dest_mode),
        }
    }
}

/// Whether an 8259 line transition is one the I/O APIC pin must also see.
///
/// Bochs pic.cc forwards from inside `raise_irq`/`lower_irq`, on the two
/// conditions this value carries: the line actually changed, and it is not the
/// cascade line, which has no I/O APIC pin of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use = "an 8259 edge the I/O APIC never sees leaves the two controllers \
              disagreeing about the line; route it through IrqFabric"]
pub(crate) enum IoApicEdge {
    /// The line changed on a pin the I/O APIC also watches.
    Crossed,
    /// Nothing for the I/O APIC: the line did not move, or it is the cascade.
    NotCrossed,
}

/// Whether an I/O APIC register change leaves interrupts for the servicing scan.
///
/// Bochs ioapic.cc calls `service_ioapic()` at the end of the write; scanning
/// needs an 8259 to acknowledge ExtINT entries with, which only [`IrqFabric`]
/// has, so the I/O APIC states the need instead of acting on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use = "an unserviced I/O APIC leaves a raised pin undelivered"]
pub(crate) enum ServiceRequest {
    Needed,
    NotNeeded,
}

/// Supplies the vector for a redirection entry in ExtINT delivery mode.
///
/// Bochs ioapic.cc reads it with `DEV_pic_iac()` — an interrupt acknowledge,
/// which is irreversible, so it is taken only for the pin that actually needs
/// one. The I/O APIC asks; the fabric is what owns an 8259 to answer.
pub(crate) trait ExtIntVector {
    fn acknowledge(&mut self) -> u8;
}

/// The Local APIC's LINT0 entry as a legacy PC presents it before a guest
/// programs the APIC: unmasked, delivery mode ExtINT — the virtual wire that
/// carries the 8259's INTR to the processor.
///
/// The reset value of the field itself, because that is the state the pin is in
/// for every guest that has not written LVT0: a machine whose fabric started
/// from the architectural masked value would refuse the 8259 its wire and no
/// legacy guest would ever take an interrupt.
const LINT0_VIRTUAL_WIRE: u64 = 0x0000_0700;

/// Bit 16 of an LVT entry, the mask.
const LVT_MASKED: u64 = 1 << 16;
/// Bits 10:8 of an LVT entry, the delivery mode; 7 is ExtINT.
const LVT_DELIVERY_MODE: u64 = 0x7 << 8;
const LVT_DELIVERY_EXT_INT: u64 = 0x7 << 8;

/// The 8259 pair and the I/O APIC, as one machine part.
#[derive(Debug)]
pub struct IrqFabric {
    pic: BxPicC,
    ioapic: BxIoApic,
    /// INTA cycles taken through [`Self::acknowledge`].
    acknowledge_count: u64,
    /// How many of those produced each vector.
    vectors_acknowledged: [u32; 256],
    /// The boot processor's LINT0 entry, as the guest last wrote it.
    ///
    /// This port's own Local APIC models keep their own LVT registers and gate
    /// their own deliveries with them; this copy exists for the guest whose
    /// Local APIC is a BACKEND's rather than this machine's. Measured on the
    /// Windows Hypervisor Platform: an ExtINT placed in a processor's pending
    /// event is delivered whether or not the guest masked LINT0, so nothing but
    /// this record can honour the mask — see
    /// [`Self::lint0_admits_ext_int`]. Bochs has no counterpart, because Bochs
    /// has no backend to offload an APIC to.
    lint0: u64,
}

impl Default for IrqFabric {
    fn default() -> Self {
        Self::new()
    }
}

impl IrqFabric {
    pub fn new() -> Self {
        Self {
            pic: BxPicC::new(),
            ioapic: BxIoApic::new(),
            acknowledge_count: 0,
            vectors_acknowledged: [0; 256],
            lint0: LINT0_VIRTUAL_WIRE,
        }
    }

    // ── The controllers, for the paths that address them as registers ──────
    //
    // Port I/O on 0x20/0xA1, MMIO on 0xFEC00000, snapshot and reset all speak
    // to one controller and imply nothing about the other. Line changes do not
    // come this way — those are `set_isa_level`.

    #[inline]
    pub(crate) fn pic(&self) -> &BxPicC {
        &self.pic
    }

    #[inline]
    pub(crate) fn pic_mut(&mut self) -> &mut BxPicC {
        &mut self.pic
    }

    #[inline]
    pub(crate) fn ioapic(&self) -> &BxIoApic {
        &self.ioapic
    }

    #[inline]
    pub(crate) fn ioapic_mut(&mut self) -> &mut BxIoApic {
        &mut self.ioapic
    }

    // ── Interrupt lines ────────────────────────────────────────────────────

    /// Drive an ISA interrupt line — Bochs pic.cc `set_irq_level`.
    ///
    /// The 8259 edge and the I/O APIC pin move together, in this call. Nothing
    /// is queued for a later boundary, so a device that raises a line inside a
    /// port write leaves both controllers agreeing about it before the write
    /// returns, exactly as upstream does.
    #[inline]
    pub(crate) fn set_isa_level(&mut self, line: IrqLine, level: bool) {
        match self.pic.set_irq_level(line.0, level) {
            IoApicEdge::NotCrossed => {}
            IoApicEdge::Crossed => self.set_ioapic_pin(line.0, level),
        }
    }

    #[inline]
    pub(crate) fn raise(&mut self, line: IrqLine) {
        self.set_isa_level(line, true);
    }

    #[inline]
    pub(crate) fn lower(&mut self, line: IrqLine) {
        self.set_isa_level(line, false);
    }

    /// Restored level of one ISA line at the 8259 input — Bochs pic.h `IRQ_in`.
    #[inline]
    pub(crate) fn line_level(&self, line: IrqLine) -> bool {
        self.pic.irq_line_level(line.0)
    }

    /// Drive an I/O APIC pin that no ISA line backs.
    ///
    /// The HPET is wired straight to the I/O APIC (Bochs hpet.cc
    /// `DEV_ioapic_set_irq_level`), so it has no 8259 edge to imply this one.
    pub(crate) fn set_ioapic_pin(&mut self, pin: u8, level: bool) {
        match self.ioapic.set_pin_level(pin, level) {
            ServiceRequest::NotNeeded => {}
            ServiceRequest::Needed => self.service_ioapic(),
        }
    }

    // ── The I/O APIC's memory window ───────────────────────────────────────
    //
    // Answered here rather than through the device bus: the I/O APIC is a part
    // of this fabric, so it cannot be dispatched with a context built over the
    // fabric that owns it. Bochs has the same asymmetry — ioapic.cc registers
    // its own handlers and never routes through `bx_devices_c`.

    /// Bochs ioapic.cc `ioapic_read`.
    #[inline]
    pub(crate) fn mmio_read(&self, offset: u64, len: u32, data: &mut [u8]) {
        self.ioapic.mem_read(offset, len, data);
    }

    /// Bochs ioapic.cc `ioapic_write`, then its trailing `service_ioapic()`.
    #[inline]
    pub(crate) fn mmio_write(&mut self, offset: u64, len: u32, data: &[u8]) {
        match self.ioapic.mem_write(offset, len, data) {
            ServiceRequest::NotNeeded => {}
            ServiceRequest::Needed => self.service_ioapic(),
        }
    }

    /// Scan the I/O APIC for deliverable interrupts — Bochs ioapic.cc
    /// `service_ioapic`.
    pub(crate) fn service_ioapic(&mut self) {
        let Self {
            pic,
            ioapic,
            acknowledge_count,
            vectors_acknowledged,
            // The Local APIC's own gate, which decides whether a vector may be
            // PLACED; this scan decides only which entries have one to give.
            lint0: _,
        } = self;
        ioapic.service(&mut PicAcknowledge {
            pic,
            acknowledge_count,
            vectors_acknowledged,
        });
    }

    // ── The CPU's INTA cycle ───────────────────────────────────────────────

    /// Is the master 8259's INT pin asserted?
    ///
    /// Deliberately separate from [`Self::acknowledge`], and deliberately
    /// `&self`: asking costs nothing and can be repeated, while acknowledging
    /// consumes the request. A hypervisor backend needs exactly this split —
    /// it must know whether to ask for an interrupt window before it commits
    /// to a vector it might not be able to inject.
    #[inline]
    pub(crate) fn int_pin_asserted(&self) -> bool {
        self.pic.has_interrupt()
    }

    /// Interrupt acknowledge — Bochs pic.cc `IAC`.
    ///
    /// The one INTA site in the machine, which is what makes the counters below
    /// a complete record: an ExtINT redirection entry acknowledges through the
    /// very same call (see [`PicAcknowledge`]).
    pub(crate) fn acknowledge(&mut self) -> u8 {
        acknowledge_8259(
            &mut self.pic,
            &mut self.acknowledge_count,
            &mut self.vectors_acknowledged,
        )
    }

    /// Are there I/O APIC messages waiting for the machine to route them?
    ///
    /// They are queued by mid-slice accesses without raising any request flag,
    /// so the boundary has to ask directly.
    #[inline]
    pub(crate) fn has_pending_deliveries(&self) -> bool {
        self.ioapic().num_pending_deliveries != 0
    }

    /// End of interrupt for `vector` — Bochs ioapic.cc `receive_eoi`.
    #[inline]
    pub(crate) fn receive_eoi(&mut self, vector: u8) {
        self.ioapic.receive_eoi(vector);
    }

    // ── The Local APIC a backend owns ──────────────────────────────────────
    //
    // Only reached when the guest's Local APIC is not this machine's. The
    // model LAPICs on the processors gate their own LINT0 and end their own
    // level interrupts; these two verbs are what a machine has instead when
    // the register file lives in a hypervisor.

    /// Record what the guest wrote to the boot processor's LINT0 entry.
    ///
    /// Called from the trap a backend raises on the write, so this copy stays
    /// current with the register the guest actually programmed.
    #[inline]
    pub fn set_lint0(&mut self, value: u64) {
        self.lint0 = value;
    }

    /// Whether the guest's LINT0 still admits the 8259's INTR.
    ///
    /// Unmasked AND in ExtINT delivery mode: a guest that masked the entry
    /// wants no legacy interrupt at all, and one that gave LINT0 a fixed vector
    /// wants that vector rather than the 8259's — in neither case may a vector
    /// be acknowledged and placed. The platform delivers a placed ExtINT
    /// without consulting its own LINT0 (measured), so this is the whole gate.
    #[inline]
    #[must_use]
    pub fn lint0_admits_ext_int(&self) -> bool {
        self.lint0 & LVT_MASKED == 0 && self.lint0 & LVT_DELIVERY_MODE == LVT_DELIVERY_EXT_INT
    }

    /// The guest ended `vector`; re-service the I/O APIC if the line that
    /// raised it is still asserted.
    ///
    /// What a Local APIC's EOI does to an I/O APIC on hardware, and what this
    /// machine has no other way to do once the Local APIC is a backend's: a
    /// level entry keeps its IRR bit until its line drops, so a line still high
    /// at the EOI must produce the interrupt again. Answers whether anything
    /// was queued, so the caller routes only when there is something to route.
    ///
    /// Bounded by the guest, not by hope: exactly one servicing scan per EOI,
    /// no loop, and a scan queues at most one message per pin. A device that
    /// never drops its line therefore costs one interrupt per handler the guest
    /// actually runs to completion — which is what an interrupt storm is on
    /// hardware too, and what makes it the device's defect rather than this
    /// machine's live-lock.
    pub fn resample_on_eoi(&mut self, vector: u8) -> bool {
        if self.ioapic.has_asserted_level_entry(vector) {
            self.service_ioapic();
            true
        } else {
            false
        }
    }

    // ── Diagnostics ────────────────────────────────────────────────────────

    #[inline]
    pub fn acknowledge_count(&self) -> u64 {
        self.acknowledge_count
    }

    #[inline]
    pub fn vectors_acknowledged(&self, vector: u8) -> u32 {
        self.vectors_acknowledged[usize::from(vector)]
    }
}

/// One INTA cycle, counted.
///
/// Split out of [`IrqFabric::acknowledge`] so the ExtINT path can take the same
/// cycle while the I/O APIC holds the fabric's other field: one body, so the
/// two can never drift.
fn acknowledge_8259(pic: &mut BxPicC, count: &mut u64, histogram: &mut [u32; 256]) -> u8 {
    let vector = pic.iac();
    *count += 1;
    histogram[usize::from(vector)] += 1;
    vector
}

/// The fabric's 8259, lent to the I/O APIC's servicing scan.
///
/// Carries the acknowledge counters too, so an ExtINT vector read is recorded
/// like any other INTA cycle rather than slipping past the tally.
struct PicAcknowledge<'a> {
    pic: &'a mut BxPicC,
    acknowledge_count: &'a mut u64,
    vectors_acknowledged: &'a mut [u32; 256],
}

impl ExtIntVector for PicAcknowledge<'_> {
    #[inline]
    fn acknowledge(&mut self) -> u8 {
        acknowledge_8259(
            self.pic,
            self.acknowledge_count,
            self.vectors_acknowledged,
        )
    }
}

/// A device drives its lines through the fabric, so both controllers see the
/// edge at once. Bochs `DEV_pic_raise_irq` / `DEV_pic_lower_irq`.
impl IrqSink for IrqFabric {
    #[inline]
    fn set_level(&mut self, line: IrqLine, level: bool) {
        self.set_isa_level(line, level);
    }

    #[inline]
    fn level(&self, line: IrqLine) -> bool {
        self.line_level(line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEYBOARD: IrqLine = IrqLine(1);

    /// Program one redirection entry's low word the way a guest does: select
    /// the register through IOREGSEL, then write the data window — through the
    /// same memory path a guest store takes.
    fn route_pin(fabric: &mut IrqFabric, pin: u8, low: u32) {
        let select = 0x10 + u32::from(pin) * 2;
        fabric.mmio_write(0x00, 4, &select.to_ne_bytes());
        fabric.mmio_write(0x10, 4, &low.to_ne_bytes());
    }

    /// Both words of one redirection entry, the way a guest programs a routed
    /// line: vector, trigger mode, and a physical destination of APIC 0.
    fn program_ioapic_pin(fabric: &mut IrqFabric, pin: u8, vector: u8, level: bool) {
        route_pin(fabric, pin, u32::from(vector) | (u32::from(level) << 15));
        let select_high = 0x11 + u32::from(pin) * 2;
        fabric.mmio_write(0x00, 4, &select_high.to_ne_bytes());
        fabric.mmio_write(0x10, 4, &0u32.to_ne_bytes());
    }

    /// The hardware behaviour a backend-owned Local APIC expects of us: a level
    /// entry whose line is still high is re-serviced the moment the guest EOIs
    /// its vector. The model LAPICs on this machine's processors do this for
    /// themselves; a guest whose LAPIC is a hypervisor's has nothing else that
    /// can.
    #[test]
    fn an_eoi_re_services_a_level_entry_whose_line_is_still_asserted() {
        let mut fabric = IrqFabric::new();
        program_ioapic_pin(&mut fabric, 5, 0x45, true);
        fabric.set_ioapic_pin(5, true);
        let (first, n) = fabric.ioapic_mut().take_pending_deliveries();
        assert_eq!(n, 1);
        assert_eq!(first[0].vector, 0x45);

        assert!(fabric.resample_on_eoi(0x45), "line still high → re-serviced");
        let (again, n) = fabric.ioapic_mut().take_pending_deliveries();
        assert_eq!(n, 1);
        assert_eq!(again[0].vector, 0x45);

        fabric.set_ioapic_pin(5, false);
        assert!(!fabric.resample_on_eoi(0x45), "line low → nothing");
        assert_eq!(fabric.ioapic_mut().take_pending_deliveries().1, 0);

        program_ioapic_pin(&mut fabric, 6, 0x46, false);
        fabric.set_ioapic_pin(6, true);
        fabric.ioapic_mut().take_pending_deliveries();
        assert!(!fabric.resample_on_eoi(0x46), "edge entries are never resampled");
        assert_eq!(fabric.ioapic_mut().take_pending_deliveries().1, 0);
    }

    /// The gate that stands between a masked legacy line and a delivered
    /// interrupt.
    ///
    /// Measured on the Windows Hypervisor Platform: an ExtINT written into a
    /// processor's pending-event slot reaches the guest with LINT0 masked
    /// exactly as it does with LINT0 unmasked. So this predicate is the only
    /// thing that honours the mask, and a guest that programmed LINT0 for a
    /// fixed vector — asking for that vector rather than the 8259's — is
    /// refused by the same rule.
    #[test]
    fn lvt0_gates_the_legacy_path_the_way_the_lapic_would() {
        let mut fabric = IrqFabric::new();
        assert!(
            fabric.lint0_admits_ext_int(),
            "a machine whose guest has not touched the APIC is a virtual wire: \
             the 8259's INTR reaches the processor"
        );
        fabric.set_lint0(0x0001_0700); // masked, ExtINT
        assert!(!fabric.lint0_admits_ext_int());
        fabric.set_lint0(0x0000_0700); // unmasked, ExtINT
        assert!(fabric.lint0_admits_ext_int());
        // Unmasked, fixed vector 0x30 — not ExtINT: the guest wants a fixed
        // vector on LINT0, not the 8259's.
        fabric.set_lint0(0x0000_0030);
        assert!(!fabric.lint0_admits_ext_int());
    }

    /// The property the fabric exists for: an ISA line raised by a device
    /// reaches the I/O APIC pin inside the same call, with no scheduler
    /// boundary in between. Bochs pic.cc raise_irq forwards synchronously; a
    /// port that queues the edge instead lets the two controllers disagree for
    /// as long as the queue sits there, which is guest-visible whenever the
    /// guest reads the I/O APIC before that boundary arrives.
    #[test]
    fn a_raised_line_reaches_the_io_apic_pin_in_the_same_call() {
        let mut fabric = IrqFabric::new();
        route_pin(&mut fabric, 1, 0x31); // fixed delivery, vector 0x31

        assert!(!fabric.ioapic().pin_state(1).0, "pin starts low");
        fabric.raise(KEYBOARD);

        assert!(fabric.line_level(KEYBOARD), "the 8259 sees the line");
        assert!(
            fabric.ioapic().pin_state(1).0,
            "the I/O APIC pin must follow within the raising call"
        );

        fabric.lower(KEYBOARD);
        assert!(!fabric.line_level(KEYBOARD));
        assert!(
            !fabric.ioapic().pin_state(1).0,
            "the falling edge must arrive just as promptly"
        );
    }

    /// The cascade line is the 8259 pair's own wiring, not an input the I/O
    /// APIC watches — Bochs pic.cc skips IRQ2 when forwarding.
    #[test]
    fn the_cascade_line_never_moves_an_io_apic_pin() {
        let mut fabric = IrqFabric::new();
        route_pin(&mut fabric, 2, 0x32);

        fabric.raise(IrqLine(2));
        assert!(fabric.line_level(IrqLine(2)), "the 8259 still latches it");
        assert!(
            !fabric.ioapic().pin_state(2).0,
            "IRQ2 is the cascade, not an I/O APIC input"
        );
    }

    /// Messages reach the Local APICs in the order the lines moved.
    ///
    /// The fabric queues a message the moment its edge crosses, and the machine
    /// drains that queue in order at the next boundary — so the order the guest
    /// sees vectors arrive is the order its devices raised them. Queuing the
    /// edges instead and replaying them later is what could reorder this, which
    /// is a guest-visible difference and not merely an internal one.
    #[test]
    fn queued_messages_keep_the_order_their_lines_moved_in() {
        let mut fabric = IrqFabric::new();
        route_pin(&mut fabric, 3, 0x33);
        route_pin(&mut fabric, 6, 0x36);
        route_pin(&mut fabric, 1, 0x31);

        fabric.raise(IrqLine(6));
        fabric.raise(IrqLine(1));
        fabric.raise(IrqLine(3));

        let (queued, count) = fabric.ioapic_mut().take_pending_deliveries();
        assert_eq!(count, 3, "each raised line must queue exactly one message");
        let vectors = [queued[0].vector, queued[1].vector, queued[2].vector];
        assert_eq!(
            vectors,
            [0x36, 0x31, 0x33],
            "deliveries must follow the order the lines were raised in"
        );
    }

    /// Every INTA cycle is counted, whichever path took it. The ExtINT vector
    /// read inside the servicing scan is an acknowledge like any other, and
    /// used to slip past the tally because it reached the 8259 directly.
    #[test]
    fn an_ext_int_vector_read_is_counted_as_the_acknowledge_it_is() {
        let mut fabric = IrqFabric::new();
        // Pin 1 in ExtINT delivery mode: the vector comes from the 8259, not
        // from the redirection entry (Bochs ioapic.cc).
        route_pin(&mut fabric, 1, 0x0000_0700);
        fabric.pic_mut().master.imr = 0x00;
        assert_eq!(fabric.acknowledge_count(), 0);

        fabric.raise(KEYBOARD);

        assert_eq!(
            fabric.acknowledge_count(),
            1,
            "the ExtINT vector read is an INTA cycle and must be tallied"
        );
        assert_eq!(
            fabric.vectors_acknowledged(0x09),
            1,
            "IRQ1 acknowledges to the master 8259's offset + 1"
        );
    }
}
