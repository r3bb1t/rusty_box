//! Which of the GIC's SPIs each of the board's interrupt sources drives.
//!
//! Provenance (R7): IHI 0048B §2.2.1 numbers the SPIs from INTID 32 to
//! 1019 and leaves which source drives which to the system. BSA DEN0094C
//! I.5 gives the GICv2m frame "an IMPLEMENTATION DEFINED number of
//! contiguous SPIs", which "other interrupt sources must not share", and
//! I.9.1 reports them in `MSI_TYPER`. The board's own choice for a layout
//! ([`InterruptMap::own`], [`InterruptMap::DEFAULT`] for the board's own
//! layout) numbers its devices' lines from SPI 0 in the order of their
//! registers' addresses — the UART, the RTC, the GPIO, the layout's virtio
//! transports, the PCIe host's INTA to INTD — and gives the frame every SPI
//! from the first after them whose interrupt ID is a multiple of 32. A
//! function sending multiple messages sets the low bits
//! of the message data to the vector it sends, up to 32 of them (PCI Local
//! Bus Specification 3.0 §6.8.1.6), and the data is the SPI's interrupt ID
//! (I.9.2), so a frame starting there gives any block of vectors a guest
//! aligns within it SPIs of its own.

use core::fmt;

use rusty_box_core::snap::{SnapRead, SnapResult, SnapWrite};
use rusty_box_core::SnapError;

use crate::devices::gicv2::{MsiFrame, SpiCount};
use crate::layout::{DeviceLayout, TransportCount};

/// The PCIe host bridge's interrupt pins, INTA to INTD.
pub const PCIE_INTX_LINES: u32 = 4;
/// The most vectors a function sending multiple messages has (PCI Local
/// Bus Specification 3.0 §6.8.1.3, Multiple Message Capable).
const MSI_BLOCK: u32 = 32;

/// A shared peripheral interrupt, by its number among the SPIs: SPI 0 is
/// INTID 32, and the last, [`Spi::LAST`], INTID 1019 (IHI 0048B §2.2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Spi(u16);

impl Spi {
    /// The first SPI.
    pub const FIRST: Self = Self(0);
    /// The last SPI a GICv2 can have, INTID 1019.
    pub const LAST: Self = Self(1019 - 32);

    /// SPI `number`, if a GICv2 can have it.
    pub const fn new(number: u32) -> Option<Self> {
        if number <= Self::LAST.0 as u32 {
            Some(Self(number as u16))
        } else {
            None
        }
    }

    /// Its number among the SPIs.
    pub const fn number(self) -> u32 {
        self.0 as u32
    }

    /// Its interrupt ID.
    pub const fn intid(self) -> u32 {
        32 + self.0 as u32
    }

    /// The SPI `offset` past this one's number, which may be past
    /// [`Spi::LAST`].
    const fn plus(self, offset: u32) -> u32 {
        self.0 as u32 + offset
    }

    /// The SPI `offset` past this one, within a block [`InterruptMap::wire`]
    /// found the GIC to hold.
    const fn within(self, offset: u32) -> Self {
        Self(self.0 + offset as u16)
    }
}

impl fmt::Display for Spi {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SPI {} (INTID {})", self.number(), self.intid())
    }
}

/// How many SPIs the GICv2m frame has, which `MSI_TYPER`'s ten-bit count
/// holds: 1 to 1023 (BSA DEN0094C I.9.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MsiCount(u16);

impl MsiCount {
    pub const MIN: Self = Self(1);
    pub const MAX: Self = Self(0x3FF);

    /// `count` SPIs, if `MSI_TYPER` can report them.
    pub const fn new(count: u32) -> Option<Self> {
        if count >= Self::MIN.0 as u32 && count <= Self::MAX.0 as u32 {
            Some(Self(count as u16))
        } else {
            None
        }
    }

    pub const fn get(self) -> u32 {
        self.0 as u32
    }
}

/// Which SPIs the GICv2m frame has: contiguous, from `first`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MsiSpis {
    pub first: Spi,
    pub count: MsiExtent,
}

/// How far the GICv2m frame's SPIs run.
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MsiExtent {
    /// To the GIC's last SPI, however many it has.
    ToTheLast,
    /// This many.
    Count(MsiCount),
}

/// Each of the board's interrupt sources by the SPI it drives, or the
/// first of the block it drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InterruptMap {
    /// The PL011.
    pub uart: Spi,
    /// The PL031.
    pub rtc: Spi,
    /// The PL061.
    pub gpio: Spi,
    /// Virtio-mmio transport `n` drives SPI `virtio + n`, for each of the
    /// layout's transports.
    pub virtio: Spi,
    /// The PCIe host bridge's INTA to INTD drive four SPIs from this one.
    pub pcie_intx: Spi,
    /// The GICv2m frame's.
    pub msi: MsiSpis,
}

impl InterruptMap {
    /// The SPIs the devices drive on a board of `transports` virtio
    /// transports: the UART's, the RTC's and the GPIO's one each, a
    /// transport's each, and the PCIe host's INTA to INTD.
    pub const fn device_lines(transports: TransportCount) -> u32 {
        3 + transports.get() as u32 + PCIE_INTX_LINES
    }

    /// The board's own for `layout`: the devices' lines from SPI 0 in the
    /// order of their registers' addresses, and the GICv2m frame every SPI
    /// from the first after them whose interrupt ID is a multiple of 32.
    /// On the board's own layout, with its four transports, that is SPI 32
    /// (INTID 64); on QEMU's, with its 32, SPI 64 (INTID 96).
    pub const fn own(layout: DeviceLayout) -> Self {
        let transports = layout.windows().virtio.count;
        let lines = Self::device_lines(transports);
        Self {
            uart: Spi(0),
            rtc: Spi(1),
            gpio: Spi(2),
            virtio: Spi(3),
            pcie_intx: Spi(3 + transports.get() as u16),
            msi: MsiSpis {
                first: Spi(((32 + lines).div_ceil(MSI_BLOCK) * MSI_BLOCK - 32) as u16),
                count: MsiExtent::ToTheLast,
            },
        }
    }

    /// The board's own for its own layout ([`DeviceLayout::DEFAULT`]).
    pub const DEFAULT: Self = Self::own(DeviceLayout::DEFAULT);

    /// Whether the map fits a GIC of `spis` on a board of `layout`: every
    /// source's SPIs the GIC's, the frame's at least one, and none shared
    /// ([`InterruptMapError`]).
    pub fn check(&self, spis: SpiCount, layout: DeviceLayout) -> Result<(), InterruptMapError> {
        self.wire(spis, layout).map(|_| ())
    }

    /// The map on a GIC of `spis` on a board of `layout`, refused when it
    /// does not fit ([`InterruptMap::check`]).
    pub(crate) fn wire(
        &self,
        spis: SpiCount,
        layout: DeviceLayout,
    ) -> Result<Wiring, InterruptMapError> {
        let transports = layout.windows().virtio.count;
        let first = self.msi.first;
        let count = match self.msi.count {
            MsiExtent::ToTheLast => MsiCount::new(spis.get().saturating_sub(first.number()))
                .ok_or(InterruptMapError::NoMsiSpis { first, spis })?,
            MsiExtent::Count(count) => count,
        };
        let blocks = [
            Block::of(InterruptSource::Uart, self.uart, 1),
            Block::of(InterruptSource::Rtc, self.rtc, 1),
            Block::of(InterruptSource::Gpio, self.gpio, 1),
            Block::of(InterruptSource::Virtio, self.virtio, u32::from(transports.get())),
            Block::of(InterruptSource::PcieIntx, self.pcie_intx, PCIE_INTX_LINES),
            Block::of(InterruptSource::MsiFrame, first, count.get()),
        ];
        for block in &blocks {
            if block.end > spis.get() {
                return Err(InterruptMapError::OutsideGic {
                    source: block.source,
                    spis,
                });
            }
        }
        for (i, a) in blocks.iter().enumerate() {
            for b in &blocks[i + 1..] {
                let shared = a.first.max(b.first);
                if shared < a.end.min(b.end) {
                    return Err(InterruptMapError::Shared {
                        first: a.source,
                        second: b.source,
                        spi: Spi(shared as u16),
                    });
                }
            }
        }
        Ok(Wiring {
            uart: self.uart,
            rtc: self.rtc,
            gpio: self.gpio,
            virtio: self.virtio,
            transports,
            pcie_intx: self.pcie_intx,
            msi: MsiFrame { first, count },
        })
    }
}

impl Default for InterruptMap {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// A source's SPIs, `first..end` by number.
struct Block {
    source: InterruptSource,
    first: u32,
    end: u32,
}

impl Block {
    const fn of(source: InterruptSource, first: Spi, lines: u32) -> Self {
        Self {
            source,
            first: first.number(),
            end: first.plus(lines),
        }
    }
}

/// One of the board's interrupt sources.
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InterruptSource {
    Uart,
    Rtc,
    Gpio,
    Virtio,
    PcieIntx,
    MsiFrame,
}

impl InterruptSource {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Uart => "the UART",
            Self::Rtc => "the RTC",
            Self::Gpio => "the GPIO",
            Self::Virtio => "the virtio transports",
            Self::PcieIntx => "the PCIe host's INTA to INTD",
            Self::MsiFrame => "the GICv2m frame",
        }
    }
}

impl fmt::Display for InterruptSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Why an [`InterruptMap`] does not fit a GIC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InterruptMapError {
    /// A source's SPIs run past the GIC's last.
    OutsideGic {
        source: InterruptSource,
        spis: SpiCount,
    },
    /// The GICv2m frame runs to the GIC's last SPI from `first`, at or past
    /// it, so it would have none.
    NoMsiSpis { first: Spi, spis: SpiCount },
    /// Two sources drive `spi`: no SPI has two (BSA DEN0094C I.5 for the
    /// GICv2m frame's).
    Shared {
        first: InterruptSource,
        second: InterruptSource,
        spi: Spi,
    },
}

impl fmt::Display for InterruptMapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutsideGic { source, spis } => write!(
                f,
                "{source} would drive SPIs past the last of a GIC of {} SPIs",
                spis.get()
            ),
            Self::NoMsiSpis { first, spis } => write!(
                f,
                "the GICv2m frame from {first} to the last of a GIC of {} SPIs would have none",
                spis.get()
            ),
            Self::Shared { first, second, spi } => {
                write!(f, "{first} and {second} would both drive {spi}")
            }
        }
    }
}

impl core::error::Error for InterruptMapError {}

/// An [`InterruptMap`] that fits the board's GIC: each device's SPIs, and
/// the GICv2m frame's by the count they come to, so a wiring is the same
/// whichever way its map named the frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct Wiring {
    uart: Spi,
    rtc: Spi,
    gpio: Spi,
    virtio: Spi,
    /// How many transports drive SPIs from `virtio`.
    transports: TransportCount,
    pcie_intx: Spi,
    msi: MsiFrame,
}

impl Wiring {
    pub(crate) const fn uart(&self) -> Spi {
        self.uart
    }

    pub(crate) const fn rtc(&self) -> Spi {
        self.rtc
    }

    pub(crate) const fn gpio(&self) -> Spi {
        self.gpio
    }

    /// Virtio-mmio transport `slot`'s, one of the layout's transports.
    pub(crate) const fn virtio(&self, slot: u8) -> Spi {
        debug_assert!(slot < self.transports.get());
        self.virtio.within(slot as u32)
    }

    /// PCIe interrupt pin `pin`'s, 0 for INTA to 3 for INTD.
    pub(crate) const fn pcie_intx(&self, pin: u32) -> Spi {
        debug_assert!(pin < PCIE_INTX_LINES);
        self.pcie_intx.within(pin)
    }

    pub(crate) const fn msi(&self) -> MsiFrame {
        self.msi
    }

    /// Save it, for a snapshot's record of how a machine was built: the
    /// devices' SPIs, then the frame's first and count, then how many
    /// transports there are.
    pub(crate) fn save<W: SnapWrite>(&self, w: &mut W) -> SnapResult {
        for spi in [self.uart, self.rtc, self.gpio, self.virtio, self.pcie_intx, self.msi.first] {
            w.write_u16(spi.0)?;
        }
        w.write_u16(self.msi.count.0)?;
        w.write_u8(self.transports.get())
    }

    /// What [`Wiring::save`] saved, refused when a number is no SPI or no
    /// count. Whether it fits a GIC is not asked: a snapshot's record is
    /// only compared with a machine's own.
    pub(crate) fn restore<R: SnapRead>(r: &mut R) -> SnapResult<Self> {
        let mut spi = || {
            r.read_u16().and_then(|n| {
                Spi::new(u32::from(n)).ok_or(SnapError::Invalid("wiring SPI out of range"))
            })
        };
        let (uart, rtc, gpio, virtio, pcie_intx) = (spi()?, spi()?, spi()?, spi()?, spi()?);
        let first = spi()?;
        let count = MsiCount::new(u32::from(r.read_u16()?))
            .ok_or(SnapError::Invalid("wiring MSI count out of range"))?;
        let transports = TransportCount::new(r.read_u8()?)
            .ok_or(SnapError::Invalid("wiring transport count out of range"))?;
        Ok(Self {
            uart,
            rtc,
            gpio,
            virtio,
            transports,
            pcie_intx,
            msi: MsiFrame { first, count },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spi(n: u32) -> Spi {
        Spi::new(n).expect("an SPI")
    }

    fn frame(first: u32, count: u32) -> MsiFrame {
        MsiFrame {
            first: spi(first),
            count: MsiCount::new(count).expect("a count"),
        }
    }

    /// The board's own layout.
    const BOARD: DeviceLayout = DeviceLayout::Board;

    /// The default numbers the devices from SPI 0 by address, the board's
    /// four transports among them, and gives the frame the rest of the GIC
    /// from SPI 32, INTID 64, a multiple of the 32 vectors a function may
    /// send: on the GIC-400's 480, INTIDs 64 to 511.
    #[test]
    fn the_default_numbers_the_devices_by_address_and_the_frame_takes_the_rest() {
        let wiring = InterruptMap::DEFAULT
            .wire(SpiCount::DEFAULT, BOARD)
            .expect("the default fits the default GIC");
        assert_eq!(wiring.uart().intid(), 32);
        assert_eq!(wiring.rtc().intid(), 33);
        assert_eq!(wiring.gpio().intid(), 34);
        assert_eq!(wiring.virtio(0).intid(), 35);
        assert_eq!(wiring.virtio(3).intid(), 38);
        assert_eq!(wiring.pcie_intx(0).intid(), 39);
        assert_eq!(wiring.pcie_intx(3).intid(), 42);
        assert_eq!(wiring.msi(), frame(32, 480 - 32));
        assert_eq!(wiring.msi().first.intid() % MSI_BLOCK, 0);
        assert_eq!(InterruptMap::device_lines(BOARD.windows().virtio.count), 11);
    }

    /// The board's own map for QEMU's layout numbers the devices the same
    /// way around its 32 transports, so its frame starts at SPI 64, INTID
    /// 96; the map for one layout is checked against that layout's
    /// transports, so the board's own layout's crosses the PCIe host's
    /// lines on QEMU's.
    #[test]
    fn a_map_is_checked_against_its_layouts_transports() {
        let qemu = DeviceLayout::QemuVirt;
        let wiring = InterruptMap::own(qemu)
            .wire(SpiCount::DEFAULT, qemu)
            .expect("QEMU's layout's own map fits");
        assert_eq!(wiring.virtio(31).intid(), 66);
        assert_eq!(wiring.pcie_intx(0).intid(), 67);
        assert_eq!(wiring.pcie_intx(3).intid(), 70);
        assert_eq!(wiring.msi(), frame(64, 480 - 64));
        assert_eq!(InterruptMap::device_lines(qemu.windows().virtio.count), 39);
        assert_eq!(
            InterruptMap::DEFAULT.check(SpiCount::DEFAULT, qemu),
            Err(InterruptMapError::Shared {
                first: InterruptSource::Virtio,
                second: InterruptSource::PcieIntx,
                spi: spi(7)
            })
        );
        assert_eq!(InterruptMap::own(BOARD), InterruptMap::DEFAULT);
    }

    /// The frame's SPIs follow the GIC's size when they run to its last;
    /// one that would have none, and a source past the GIC's last SPI, are
    /// refused.
    #[test]
    fn the_frame_runs_to_the_gics_last_spi() {
        let small = SpiCount::new(64).expect("a count");
        let wiring = InterruptMap::DEFAULT.wire(small, BOARD).expect("fits");
        assert_eq!(wiring.msi(), frame(32, 32));
        assert_eq!(
            InterruptMap::DEFAULT.wire(SpiCount::MIN, BOARD),
            Err(InterruptMapError::NoMsiSpis {
                first: spi(32),
                spis: SpiCount::MIN
            }),
            "the board's own frame starts past the fewest SPIs a GIC may have"
        );
        let packed = InterruptMap {
            msi: MsiSpis {
                first: spi(InterruptMap::device_lines(BOARD.windows().virtio.count)),
                count: MsiExtent::ToTheLast,
            },
            ..InterruptMap::DEFAULT
        };
        assert_eq!(
            packed.wire(SpiCount::MIN, BOARD).map(|w| w.msi()),
            Ok(frame(11, 21)),
            "a map packed after the devices fits the fewest"
        );
        let counted = InterruptMap {
            msi: MsiSpis {
                first: spi(100),
                count: MsiExtent::Count(MsiCount::new(28).expect("a count")),
            },
            ..InterruptMap::DEFAULT
        };
        let spis = SpiCount::new(128).expect("a count");
        assert_eq!(counted.wire(spis, BOARD).map(|w| w.msi()), Ok(frame(100, 28)));
        let past = InterruptMap {
            msi: MsiSpis {
                first: spi(100),
                count: MsiExtent::Count(MsiCount::new(29).expect("a count")),
            },
            ..InterruptMap::DEFAULT
        };
        assert_eq!(
            past.wire(spis, BOARD),
            Err(InterruptMapError::OutsideGic {
                source: InterruptSource::MsiFrame,
                spis
            })
        );
        for first in [128, 200] {
            let none_left = InterruptMap {
                msi: MsiSpis {
                    first: spi(first),
                    count: MsiExtent::ToTheLast,
                },
                ..InterruptMap::DEFAULT
            };
            assert_eq!(
                none_left.wire(spis, BOARD),
                Err(InterruptMapError::NoMsiSpis {
                    first: spi(first),
                    spis
                })
            );
        }
        let uart_past = InterruptMap {
            uart: spi(128),
            ..InterruptMap::DEFAULT
        };
        assert_eq!(
            uart_past.wire(spis, BOARD),
            Err(InterruptMapError::OutsideGic {
                source: InterruptSource::Uart,
                spis
            })
        );
    }

    /// No two sources share an SPI: a single line inside a block, two
    /// blocks overlapping by one, and the frame over a device are refused,
    /// naming the first SPI they share; the SPIs between the devices' and
    /// the frame's are free.
    #[test]
    fn no_two_sources_share_an_spi() {
        let spis = SpiCount::DEFAULT;
        let rtc_on_virtio = InterruptMap {
            rtc: spi(5),
            ..InterruptMap::DEFAULT
        };
        assert_eq!(
            rtc_on_virtio.wire(spis, BOARD),
            Err(InterruptMapError::Shared {
                first: InterruptSource::Rtc,
                second: InterruptSource::Virtio,
                spi: spi(5)
            })
        );
        let overlapping = InterruptMap {
            pcie_intx: spi(6),
            ..InterruptMap::DEFAULT
        };
        assert_eq!(
            overlapping.wire(spis, BOARD),
            Err(InterruptMapError::Shared {
                first: InterruptSource::Virtio,
                second: InterruptSource::PcieIntx,
                spi: spi(6)
            })
        );
        let frame_over_uart = InterruptMap {
            uart: spi(200),
            ..InterruptMap::DEFAULT
        };
        assert_eq!(
            frame_over_uart.wire(spis, BOARD),
            Err(InterruptMapError::Shared {
                first: InterruptSource::Uart,
                second: InterruptSource::MsiFrame,
                spi: spi(200)
            })
        );
        let in_the_gap = InterruptMap {
            uart: spi(31),
            ..InterruptMap::DEFAULT
        };
        assert!(in_the_gap.wire(spis, BOARD).is_ok(), "blocks that touch share none");
    }

    /// A wiring saved is restored as it was; a number no SPI, count or
    /// transport count has is refused.
    #[test]
    fn a_wiring_round_trips_and_refuses_what_is_no_spi() {
        let wiring = InterruptMap::DEFAULT.wire(SpiCount::DEFAULT, BOARD).expect("fits");
        let counted = InterruptMap {
            msi: MsiSpis {
                first: spi(32),
                count: MsiExtent::Count(MsiCount::new(448).expect("a count")),
            },
            ..InterruptMap::DEFAULT
        };
        assert_eq!(
            counted.wire(SpiCount::DEFAULT, BOARD),
            Ok(wiring),
            "a frame to the last and one of the count it comes to wire alike"
        );
        let mut image = std::vec::Vec::new();
        wiring.save(&mut image).expect("saves");
        assert_eq!(Wiring::restore(&mut image.as_slice()), Ok(wiring));
        let mut bad = image.clone();
        bad[0..2].copy_from_slice(&988u16.to_le_bytes());
        assert!(Wiring::restore(&mut bad.as_slice()).is_err());
        // The frame's count, then the transports'.
        let count_at = image.len() - 3;
        let mut zero = image.clone();
        zero[count_at..count_at + 2].copy_from_slice(&0u16.to_le_bytes());
        assert!(Wiring::restore(&mut zero.as_slice()).is_err());
        for transports in [3, 33] {
            let mut odd = image.clone();
            odd[count_at + 2] = transports;
            assert!(Wiring::restore(&mut odd.as_slice()).is_err(), "{transports}");
        }
    }

    #[test]
    fn spis_and_counts_take_what_a_gicv2_and_msi_typer_hold() {
        assert_eq!(Spi::new(987).map(Spi::intid), Some(1019));
        assert_eq!(Spi::new(988), None);
        assert_eq!(MsiCount::new(0), None);
        assert_eq!(MsiCount::new(1023).map(MsiCount::get), Some(1023));
        assert_eq!(MsiCount::new(1024), None);
    }
}
