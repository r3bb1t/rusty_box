//! Assembling a machine.
//!
//! A powered-off PC is a [`MachineBuilder`], not an `Emulator` sitting in a
//! half-initialised state. The builder collects firmware, media and CMOS
//! settings; `build()` performs the whole Bochs `bx_init_hardware` sequence
//! (main.cc) and hands back a machine at its reset vector.
//!
//! Doctrine: R2 (assembly is a type, not a `bool` on the finished machine),
//! R3 (the sequence is one choke point nobody outside can interleave with),
//! R4 (`AtaSlot`, `DiskGeometry` and `BootDevice` replace the loose
//! `usize`/`u8` currency the old init chain passed around).

use crate::{
    cpu::{instrumentation::Instrumentation, ResetReason},
    Result,
};

use super::{Emulator, EmulatorConfig};

/// A misconfiguration caught at the construction choke point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum BuildError {
    /// The load address is derived from the image length, and only a length
    /// that fits Bochs's 32-bit `romaddress` has one.
    #[error("a BIOS image must be between 1 byte and 4 GiB; this one is {bytes}")]
    UnusableBiosImage { bytes: usize },

    /// One tracer instance cannot be shared between processors, and
    /// `Instrumentation` is not required to be cloneable. Use
    /// [`MachineBuilder::tracer_factory`] to give each processor its own.
    #[error("a machine built from one tracer instance runs one processor; {requested} were configured")]
    InstrumentedSmp { requested: u32 },
}

/// The ATA channel and drive a piece of media hangs off.
///
/// Bochs names these `ata0-master` … `ata1-slave` (harddrv.cc); the pair of
/// bare `usize`s this used to be is the loose currency R4 forbids.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AtaSlot {
    channel: u8,
    drive: u8,
}

impl AtaSlot {
    /// `ata0-master` — the primary channel's first drive.
    pub const PRIMARY_MASTER: Self = Self {
        channel: 0,
        drive: 0,
    };
    /// `ata0-slave` — the primary channel's second drive.
    pub const PRIMARY_SLAVE: Self = Self {
        channel: 0,
        drive: 1,
    };
    /// `ata1-master` — the secondary channel's first drive.
    pub const SECONDARY_MASTER: Self = Self {
        channel: 1,
        drive: 0,
    };
    /// `ata1-slave` — the secondary channel's second drive.
    pub const SECONDARY_SLAVE: Self = Self {
        channel: 1,
        drive: 1,
    };

    /// The slot on `channel` holding `drive`, or `None` when the controller
    /// has no such socket — the ATA subsystem models two channels of two
    /// drives, exactly as Bochs harddrv.cc does.
    pub const fn new(channel: usize, drive: usize) -> Option<Self> {
        if channel > 1 || drive > 1 {
            return None;
        }
        Some(Self {
            channel: channel as u8,
            drive: drive as u8,
        })
    }

    /// The ATA channel: 0 = primary (0x1F0, IRQ 14), 1 = secondary (0x170, IRQ 15).
    pub const fn channel(self) -> usize {
        self.channel as usize
    }

    /// The drive on that channel: 0 = master, 1 = slave.
    pub const fn drive(self) -> usize {
        self.drive as usize
    }

    const fn index(self) -> usize {
        (self.channel as usize) * 2 + self.drive as usize
    }

    const fn from_index(index: usize) -> Self {
        Self {
            channel: (index / 2) as u8,
            drive: (index % 2) as u8,
        }
    }

    /// The CMOS drive number whose geometry registers describe this slot, if
    /// it has any. Bochs harddrv.cc writes geometry for `BX_DRIVE(0,0)` and
    /// `BX_DRIVE(0,1)` only — the secondary channel has no CMOS registers.
    const fn cmos_drive(self) -> Option<u8> {
        if self.channel == 0 {
            Some(self.drive)
        } else {
            None
        }
    }
}

/// A disk's CHS geometry, as the guest's BIOS and the ATA IDENTIFY response
/// report it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiskGeometry {
    pub cylinders: u32,
    pub heads: u8,
    pub sectors_per_track: u8,
}

impl DiskGeometry {
    pub const fn new(cylinders: u32, heads: u8, sectors_per_track: u8) -> Self {
        Self {
            cylinders,
            heads,
            sectors_per_track,
        }
    }
}

/// One entry of the CMOS boot sequence.
///
/// The discriminants are the ElTorito codes Bochs stores in CMOS (cmos.cc,
/// floppy.cc).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BootDevice {
    /// Nothing in this position; the BIOS skips it.
    #[default]
    None = 0,
    Floppy = 1,
    Disk = 2,
    Cdrom = 3,
}

impl BootDevice {
    const fn code(self) -> u8 {
        self as u8
    }
}

/// The three CMOS boot-sequence positions the BIOS tries in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BootOrder {
    pub first: BootDevice,
    pub second: BootDevice,
    pub third: BootDevice,
}

impl BootOrder {
    /// Try one device and nothing else.
    pub const fn just(first: BootDevice) -> Self {
        Self {
            first,
            second: BootDevice::None,
            third: BootDevice::None,
        }
    }

    pub const fn new(first: BootDevice, second: BootDevice, third: BootDevice) -> Self {
        Self {
            first,
            second,
            third,
        }
    }
}

/// Where a drive's bytes come from.
///
/// Private, because which sources exist depends on the allocator features.
/// R0 keeps the *public* surface shape-stable by exposing one named builder
/// method per source instead of a feature-dependent enum.
enum Media {
    #[cfg(feature = "std")]
    DiskFile {
        path: alloc::string::String,
        geometry: DiskGeometry,
    },
    #[cfg(feature = "alloc")]
    DiskBytes {
        data: alloc::vec::Vec<u8>,
        geometry: DiskGeometry,
    },
    DiskStatic {
        data: &'static [u8],
        geometry: DiskGeometry,
    },
    #[cfg(feature = "std")]
    CdromFile { path: alloc::string::String },
    #[cfg(feature = "alloc")]
    CdromBytes { data: alloc::vec::Vec<u8> },
    CdromStatic { data: &'static [u8] },
}

impl Media {
    /// The geometry to write into CMOS for this slot, or `None` for media the
    /// BIOS discovers for itself — Bochs writes geometry for hard disks only.
    const fn cmos_geometry(&self) -> Option<DiskGeometry> {
        match self {
            #[cfg(feature = "std")]
            Media::DiskFile { geometry, .. } => Some(*geometry),
            #[cfg(feature = "alloc")]
            Media::DiskBytes { geometry, .. } => Some(*geometry),
            Media::DiskStatic { geometry, .. } => Some(*geometry),
            #[cfg(feature = "std")]
            Media::CdromFile { .. } => None,
            #[cfg(feature = "alloc")]
            Media::CdromBytes { .. } => None,
            Media::CdromStatic { .. } => None,
        }
    }
}

/// How the machine's processors get their tracers.
///
/// A no-alloc host builds its own processors — `BxCpuBuilder::init_cpu_at`
/// takes the tracer there — so a no-alloc builder has only the type to
/// remember, and `build_at` reads it off the processors it is handed.
#[cfg(feature = "alloc")]
enum Tracers<T: Instrumentation> {
    /// One instance, for the sole processor of a uniprocessor machine.
    Single(T),
    /// A fresh tracer per processor — the only shape an SMP machine can take,
    /// because `Instrumentation` is not required to be cloneable.
    PerProcessor(fn() -> T),
}

#[cfg(not(feature = "alloc"))]
type Tracers<T> = core::marker::PhantomData<fn() -> T>;

/// The tracer arrangement of a machine with no instrumentation.
fn untraced() -> Tracers<()> {
    #[cfg(feature = "alloc")]
    {
        Tracers::PerProcessor(|| ())
    }
    #[cfg(not(feature = "alloc"))]
    {
        core::marker::PhantomData
    }
}

/// The VGA BIOS sits at the bottom of the option-ROM window (Bochs
/// misc_mem.cc `BX_VGA_BIOS_ADDR`).
const VGA_BIOS_ADDRESS: u64 = 0xC0000;

/// Everything the assembly sequence applies to a freshly constructed machine.
#[derive(Default)]
struct Settings<'r> {
    bios: Option<&'r [u8]>,
    vga_bios: Option<&'r [u8]>,
    boot_order: Option<BootOrder>,
    media: [Option<Media>; 4],
    #[cfg(feature = "alloc")]
    gui: Option<alloc::boxed::Box<dyn crate::gui::BxGui>>,
}

/// A powered-off PC, and the only way to reach a running one.
///
/// ```no_run
/// use rusty_box::config::EmulatorConfig;
/// use rusty_box::emulator::{AtaSlot, BootDevice, BootOrder, DiskGeometry, MachineBuilder};
///
/// # fn main() -> rusty_box::Result<()> {
/// # let bios: &[u8] = &[]; let vga_bios: &[u8] = &[];
/// let mut machine = MachineBuilder::new(EmulatorConfig::default())
///     .bios(bios)
///     .vga_bios(vga_bios)
///     .boot_order(BootOrder::just(BootDevice::Disk))
///     .disk_file(
///         AtaSlot::PRIMARY_MASTER,
///         "disk.img",
///         DiskGeometry::new(306, 4, 17),
///     )
///     .build()?;
/// let outcome = machine.step_batch(100_000)?;
/// # let _ = outcome;
/// # Ok(())
/// # }
/// ```
pub struct MachineBuilder<'r, T: Instrumentation = ()> {
    config: EmulatorConfig,
    tracers: Tracers<T>,
    settings: Settings<'r>,
}

impl<'r> MachineBuilder<'r, ()> {
    /// A machine with no instrumentation. Every processor gets the unit
    /// tracer, so this shape is SMP-capable.
    pub fn new(config: EmulatorConfig) -> Self {
        Self {
            config,
            tracers: untraced(),
            settings: Settings::default(),
        }
    }
}

#[cfg(feature = "alloc")]
impl<'r, T: Instrumentation> MachineBuilder<'r, T> {
    /// Give the sole processor `tracer`. The resulting machine is
    /// uniprocessor: one instance cannot be shared between processors, and
    /// `Instrumentation` is not required to be cloneable. For an instrumented
    /// SMP machine use [`Self::tracer_factory`].
    pub fn tracer<U: Instrumentation>(self, tracer: U) -> MachineBuilder<'r, U> {
        self.retype(Tracers::Single(tracer))
    }

    /// Give every processor its own tracer, minted by `make`.
    pub fn tracer_factory<U: Instrumentation>(self, make: fn() -> U) -> MachineBuilder<'r, U> {
        self.retype(Tracers::PerProcessor(make))
    }

    fn retype<U: Instrumentation>(self, tracers: Tracers<U>) -> MachineBuilder<'r, U> {
        MachineBuilder {
            config: self.config,
            tracers,
            settings: self.settings,
        }
    }
}

impl<'r, T: Instrumentation> MachineBuilder<'r, T> {
    /// The system BIOS image. Its load address is derived from its length the
    /// way Bochs misc_mem.cc does (`romaddress = ~(size - 1)`), so the reset
    /// vector at `0xFFFFFFF0` always falls inside it.
    pub fn bios(mut self, rom: &'r [u8]) -> Self {
        self.settings.bios = Some(rom);
        self
    }

    /// The VGA BIOS image, loaded at `0xC0000`.
    pub fn vga_bios(mut self, rom: &'r [u8]) -> Self {
        self.settings.vga_bios = Some(rom);
        self
    }

    /// The CMOS boot sequence. Left alone when never set, so a machine that
    /// does not care keeps the CMOS power-up default.
    pub fn boot_order(mut self, order: BootOrder) -> Self {
        self.settings.boot_order = Some(order);
        self
    }

    /// Attach a hard disk backed by a file on the host.
    #[cfg(feature = "std")]
    pub fn disk_file(mut self, slot: AtaSlot, path: &str, geometry: DiskGeometry) -> Self {
        self.settings.media[slot.index()] = Some(Media::DiskFile {
            path: alloc::string::ToString::to_string(path),
            geometry,
        });
        self
    }

    /// Attach a hard disk backed by an owned image in host memory.
    #[cfg(feature = "alloc")]
    pub fn disk_bytes(
        mut self,
        slot: AtaSlot,
        data: alloc::vec::Vec<u8>,
        geometry: DiskGeometry,
    ) -> Self {
        self.settings.media[slot.index()] = Some(Media::DiskBytes { data, geometry });
        self
    }

    /// Attach a hard disk backed by a borrowed image — the no-alloc path,
    /// where the image is usually linked into the binary.
    pub fn disk_static(
        mut self,
        slot: AtaSlot,
        data: &'static [u8],
        geometry: DiskGeometry,
    ) -> Self {
        self.settings.media[slot.index()] = Some(Media::DiskStatic { data, geometry });
        self
    }

    /// Attach a CD-ROM backed by an ISO file on the host.
    #[cfg(feature = "std")]
    pub fn cdrom_file(mut self, slot: AtaSlot, path: &str) -> Self {
        self.settings.media[slot.index()] = Some(Media::CdromFile {
            path: alloc::string::ToString::to_string(path),
        });
        self
    }

    /// Attach a CD-ROM backed by an owned ISO image in host memory.
    #[cfg(feature = "alloc")]
    pub fn cdrom_bytes(mut self, slot: AtaSlot, data: alloc::vec::Vec<u8>) -> Self {
        self.settings.media[slot.index()] = Some(Media::CdromBytes { data });
        self
    }

    /// Attach a CD-ROM backed by a borrowed ISO image — the no-alloc path.
    pub fn cdrom_static(mut self, slot: AtaSlot, data: &'static [u8]) -> Self {
        self.settings.media[slot.index()] = Some(Media::CdromStatic { data });
        self
    }

    /// The display front end. Bochs loads it before `bx_init_hardware`
    /// (main.cc `load_and_init_display_lib`), and so does `build()`.
    #[cfg(feature = "alloc")]
    pub fn gui<G: crate::gui::BxGui + 'static>(mut self, gui: G) -> Self {
        self.settings.gui = Some(alloc::boxed::Box::new(gui));
        self
    }
}

#[cfg(feature = "alloc")]
impl<'r, T: Instrumentation> MachineBuilder<'r, T> {
    /// Build the machine, run it through hardware initialisation, and reset it.
    ///
    /// The result sits at its reset vector with timers armed: the next
    /// `step_batch` executes the first firmware instruction.
    pub fn build(self) -> Result<alloc::boxed::Box<Emulator<T>>> {
        let Self {
            config,
            tracers,
            settings,
        } = self;
        let mut machine = construct(config, tracers)?;
        settings.furnish(&mut machine)?;
        Ok(machine)
    }
}

#[cfg(not(feature = "alloc"))]
impl<'r> MachineBuilder<'r, ()> {
    /// Build the machine into caller-provided storage, then run it through
    /// hardware initialisation and reset.
    ///
    /// The no-alloc host owns the three big allocations, so it supplies the
    /// machine's storage, its processors and its already-initialised memory
    /// stub; everything after that is the same sequence `build()` performs.
    /// The processors carry their own tracers, so `T` is read off them here
    /// rather than configured on the builder.
    pub fn build_at<'a, T: Instrumentation>(
        self,
        storage: &'a mut core::mem::MaybeUninit<Emulator<T>>,
        cpus: &'static mut [&'static mut crate::cpu::BxCpuC<T>],
        mem_stub: crate::memory::BxMemoryStubC,
    ) -> Result<&'a mut Emulator<T>> {
        let Self {
            config, settings, ..
        } = self;
        let machine = Emulator::init_at(storage, cpus, mem_stub, config)?;
        settings.furnish(machine)?;
        Ok(machine)
    }
}

#[cfg(feature = "alloc")]
fn construct<T: Instrumentation>(
    config: EmulatorConfig,
    tracers: Tracers<T>,
) -> Result<alloc::boxed::Box<Emulator<T>>> {
    match tracers {
        Tracers::Single(tracer) => Emulator::with_tracer(config, tracer),
        Tracers::PerProcessor(make) => Emulator::with_tracer_factory(config, make),
    }
}

impl Settings<'_> {
    /// Run the Bochs `bx_init_hardware` sequence (main.cc) against a freshly
    /// constructed machine, leaving it at its reset vector with timers armed.
    ///
    /// This is the one choke point for the whole sequence: no caller can
    /// reorder or skip a step (R5).
    fn furnish<T: Instrumentation>(self, machine: &mut Emulator<T>) -> Result<()> {
        #[cfg(feature = "alloc")]
        if let Some(gui) = self.gui {
            machine.set_boxed_gui(gui);
        }

        // Memory and PC system. A no-alloc machine received its memory as a
        // caller-built stub, so only the PC system is left to initialise.
        #[cfg(feature = "alloc")]
        machine.init_memory_and_pc_system()?;
        #[cfg(not(feature = "alloc"))]
        machine.init_pc_system();

        if let Some(rom) = self.bios {
            machine.load_bios(rom, bios_load_address(rom.len())?)?;
        }
        if let Some(rom) = self.vga_bios {
            machine.load_optional_rom(rom, VGA_BIOS_ADDRESS)?;
        }

        machine.init_cpu_and_devices()?;

        // Media first, then the CMOS registers describing it: Bochs
        // harddrv.cc opens its images and only afterwards writes the geometry
        // registers, so the geometry can only ever describe what is attached.
        for (index, media) in self.media.into_iter().enumerate() {
            let Some(media) = media else {
                continue;
            };
            let slot = AtaSlot::from_index(index);
            let geometry = media.cmos_geometry();
            attach(machine, slot, media)?;
            if let (Some(geometry), Some(cmos_drive)) = (geometry, slot.cmos_drive()) {
                // Bochs stores the cylinder count as two bytes taken from an
                // `unsigned`, so a geometry wider than 16 bits truncates there
                // exactly as it does here.
                machine.configure_disk_geometry_in_cmos(
                    cmos_drive,
                    geometry.cylinders as u16,
                    geometry.heads,
                    geometry.sectors_per_track,
                );
            }
        }

        machine.configure_memory_in_cmos_from_config();
        if let Some(order) = self.boot_order {
            machine.configure_boot_sequence(
                order.first.code(),
                order.second.code(),
                order.third.code(),
            );
        }

        #[cfg(feature = "alloc")]
        machine.init_gui(0, &[])?;

        machine.reset(ResetReason::Hardware)?;

        #[cfg(feature = "alloc")]
        machine.init_gui_signal_handlers();

        machine.start_timers();
        Ok(())
    }
}

/// The address a BIOS image of `len` bytes loads at, following Bochs
/// misc_mem.cc `load_ROM`: `romaddress = ~(size - 1)` over its 32-bit
/// `bx_phy_address`, so the image ends at the 4 GiB boundary and the reset
/// vector at `0xFFFFFFF0` falls inside it. Deriving it here is also what makes
/// upstream's "System BIOS must end at 0xfffff" rejection unreachable: no
/// caller supplies an address to get wrong.
fn bios_load_address(len: usize) -> Result<u64> {
    let size = u32::try_from(len)
        .ok()
        .filter(|&size| size != 0)
        .ok_or(BuildError::UnusableBiosImage { bytes: len })?;
    Ok(u64::from(!(size - 1)))
}

fn attach<T: Instrumentation>(
    machine: &mut Emulator<T>,
    slot: AtaSlot,
    media: Media,
) -> Result<()> {
    match media {
        #[cfg(feature = "std")]
        Media::DiskFile { path, geometry } => machine.attach_disk(
            slot.channel(),
            slot.drive(),
            &path,
            geometry.cylinders,
            geometry.heads,
            geometry.sectors_per_track,
        )?,
        #[cfg(feature = "alloc")]
        Media::DiskBytes { data, geometry } => machine.attach_disk_data(
            slot.channel(),
            slot.drive(),
            data,
            geometry.cylinders,
            geometry.heads,
            geometry.sectors_per_track,
        ),
        Media::DiskStatic { data, geometry } => machine.attach_disk_data_ref(
            slot.channel(),
            slot.drive(),
            data,
            geometry.cylinders,
            geometry.heads,
            geometry.sectors_per_track,
        ),
        #[cfg(feature = "std")]
        Media::CdromFile { path } => machine.attach_cdrom(slot.channel(), slot.drive(), &path)?,
        #[cfg(feature = "alloc")]
        Media::CdromBytes { data } => machine.attach_cdrom_data(slot.channel(), slot.drive(), data),
        Media::CdromStatic { data } => {
            machine.attach_cdrom_data_ref(slot.channel(), slot.drive(), data)
        }
    }
    Ok(())
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use crate::emulator::MemorySize;
    use crate::emulator::StopReason;

    const TEST_STACK_SIZE: usize = 64 * 1024 * 1024;

    /// A machine is ~1.4 MiB of embedded device state; libtest's own 2 MiB
    /// thread is not enough to build one.
    fn on_a_big_stack(body: impl FnOnce() + Send + 'static) {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(body)
            .unwrap()
            .join()
            .unwrap();
    }

    fn small_machine() -> EmulatorConfig {
        EmulatorConfig {
            memory: MemorySize::bytes(4 * 1024 * 1024),
            ..EmulatorConfig::default()
        }
    }

    /// The CMOS register a guest reads after selecting `register` through the
    /// index port — exactly the pair of `OUT 0x70` / `IN 0x71` the BIOS issues.
    fn cmos_register<T: Instrumentation>(machine: &mut Emulator<T>, register: u8) -> u8 {
        let cmos = &mut machine.device_manager.cmos;
        cmos.write(0x70, u32::from(register), 1);
        cmos.read(0x71, 1) as u8
    }

    #[test]
    fn a_bios_image_lands_where_the_reset_vector_points() {
        on_a_big_stack(|| {
            // A 64 KiB image of HLT: whatever the reset vector resolves to
            // inside it, the machine parks on the first instruction.
            let bios = alloc::vec![0xF4u8; 0x10000];
            let mut machine = MachineBuilder::new(small_machine())
                .bios(&bios)
                .build()
                .expect("build");

            let outcome = machine.step_batch(4).expect("step");
            assert_eq!(
                outcome.stop,
                StopReason::Halted,
                "the reset vector must fetch from the loaded BIOS"
            );
        });
    }

    #[test]
    fn an_empty_bios_image_has_no_load_address() {
        assert!(matches!(
            bios_load_address(0),
            Err(crate::Error::Build(BuildError::UnusableBiosImage { bytes: 0 }))
        ));
        // Bochs misc_mem.cc: a 64 KiB image ends at the 4 GiB boundary, a
        // 128 KiB image one bank below it.
        assert_eq!(bios_load_address(0x10000).unwrap(), 0xFFFF_0000);
        assert_eq!(bios_load_address(0x20000).unwrap(), 0xFFFE_0000);
    }

    #[test]
    fn a_disks_geometry_reaches_the_cmos_registers_for_its_slot() {
        on_a_big_stack(|| {
            static IMAGE: &[u8] = &[0u8; 512];
            let mut machine = MachineBuilder::new(small_machine())
                .disk_static(
                    AtaSlot::PRIMARY_SLAVE,
                    IMAGE,
                    DiskGeometry::new(0x1234, 9, 17),
                )
                .build()
                .expect("build");

            // Bochs harddrv.cc drive-1 registers: 0x24/0x25 cylinders,
            // 0x26 heads, 0x2C sectors per track, 0x12 low nibble = type 0xF.
            assert_eq!(cmos_register(&mut machine, 0x24), 0x34);
            assert_eq!(cmos_register(&mut machine, 0x25), 0x12);
            assert_eq!(cmos_register(&mut machine, 0x26), 9);
            assert_eq!(cmos_register(&mut machine, 0x2C), 17);
            assert_eq!(cmos_register(&mut machine, 0x12) & 0x0F, 0x0F);
            // Nothing hangs off drive 0, so its nibble stays clear.
            assert_eq!(cmos_register(&mut machine, 0x12) & 0xF0, 0x00);
        });
    }

    #[test]
    fn a_secondary_channel_disk_gets_no_cmos_geometry() {
        on_a_big_stack(|| {
            static IMAGE: &[u8] = &[0u8; 512];
            let mut machine = MachineBuilder::new(small_machine())
                .disk_static(
                    AtaSlot::SECONDARY_MASTER,
                    IMAGE,
                    DiskGeometry::new(306, 4, 17),
                )
                .build()
                .expect("build");

            // Bochs writes geometry for BX_DRIVE(0,0) and BX_DRIVE(0,1) only.
            assert_eq!(cmos_register(&mut machine, 0x12), 0x00);
            assert_eq!(cmos_register(&mut machine, 0x1B), 0x00);
        });
    }

    #[test]
    fn a_cdrom_carries_no_geometry_but_still_sets_the_boot_order() {
        on_a_big_stack(|| {
            static ISO: &[u8] = &[0u8; 2048];
            let mut machine = MachineBuilder::new(small_machine())
                .cdrom_static(AtaSlot::PRIMARY_MASTER, ISO)
                .boot_order(BootOrder::just(BootDevice::Cdrom))
                .build()
                .expect("build");

            assert_eq!(cmos_register(&mut machine, 0x12), 0x00);
            // Bochs cmos.cc register 0x3D holds the first two boot devices,
            // one nibble each.
            assert_eq!(cmos_register(&mut machine, 0x3D) & 0x0F, 3);
        });
    }

    #[test]
    fn an_unset_boot_order_leaves_the_cmos_default_alone() {
        on_a_big_stack(|| {
            let mut built = MachineBuilder::new(small_machine()).build().expect("build");
            let mut untouched = Emulator::new(small_machine()).expect("machine");
            untouched.initialize().expect("initialize");

            assert_eq!(
                cmos_register(&mut built, 0x3D),
                cmos_register(&mut untouched, 0x3D)
            );
        });
    }

    #[test]
    fn the_ata_controller_has_two_channels_of_two_drives() {
        assert_eq!(AtaSlot::new(0, 0), Some(AtaSlot::PRIMARY_MASTER));
        assert_eq!(AtaSlot::new(0, 1), Some(AtaSlot::PRIMARY_SLAVE));
        assert_eq!(AtaSlot::new(1, 0), Some(AtaSlot::SECONDARY_MASTER));
        assert_eq!(AtaSlot::new(1, 1), Some(AtaSlot::SECONDARY_SLAVE));
        assert_eq!(AtaSlot::new(2, 0), None);
        assert_eq!(AtaSlot::new(0, 2), None);
    }

    #[test]
    fn every_slot_survives_the_round_trip_through_its_index() {
        for channel in 0..2 {
            for drive in 0..2 {
                let slot = AtaSlot::new(channel, drive).expect("valid slot");
                assert_eq!(AtaSlot::from_index(slot.index()), slot);
            }
        }
    }

    #[test]
    fn one_tracer_instance_cannot_be_shared_by_several_processors() {
        on_a_big_stack(|| {
            #[derive(Default)]
            struct CountingTracer;
            impl Instrumentation for CountingTracer {}

            let config = EmulatorConfig {
                cpu_params: crate::params::BxParams::default()
                    .with_topology(2, 1, 1)
                    .expect("valid topology"),
                ..small_machine()
            };
            let verdict = MachineBuilder::new(config).tracer(CountingTracer).build();
            assert!(matches!(
                verdict.err(),
                Some(crate::Error::Build(BuildError::InstrumentedSmp {
                    requested: 2
                }))
            ));
        });
    }

    #[test]
    fn a_tracer_factory_gives_every_processor_its_own() {
        on_a_big_stack(|| {
            #[derive(Default)]
            struct PerCpuTracer;
            impl Instrumentation for PerCpuTracer {}

            let config = EmulatorConfig {
                cpu_params: crate::params::BxParams::default()
                    .with_topology(2, 1, 1)
                    .expect("valid topology"),
                ..small_machine()
            };
            let machine = MachineBuilder::new(config)
                .tracer_factory(|| PerCpuTracer)
                .build()
                .expect("build");
            assert_eq!(machine.cpu_count(), 2);
        });
    }
}
