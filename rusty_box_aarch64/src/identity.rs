//! What the board tells a guest it is.
//!
//! A guest reads who made its machine from a handful of names and numbers:
//! the device tree's `/model` and root `compatible`, the virtio-mmio
//! `VendorID`, the GICv2m frame's `MSI_IIDR`, the virtio input devices'
//! names (`VIRTIO_INPUT_CFG_ID_NAME`) and vendor, products and versions
//! (`VIRTIO_INPUT_CFG_ID_DEVIDS`), and the maker, product code, name and
//! date in the virtio-gpu display's EDID. Software keys on them —
//! systemd's `detect_vm_device_tree` reports QEMU for a node named `fw-cfg`
//! and a generic virtual machine for a root `compatible` of
//! `linux,dummy-virt` — so the board gives none of another product's.
//! [`BoardIdentity::RUSTY_BOX`] names this project; a host names its own
//! machine with [`BoardIdentity::new`].
//!
//! Each name is copied into a fixed buffer and checked when the identity
//! is made, so a machine holds its identity without an allocator.

use crate::devices::virtio::input::InputKind;

/// The longest device tree `/model` a [`BoardIdentity`] holds.
pub const MODEL_CAPACITY: usize = 64;
/// The most bytes a [`BoardIdentity`]'s root `compatible` takes: its
/// strings, each with the NUL that ends it in the device tree.
pub const COMPATIBLE_CAPACITY: usize = 128;
/// The longest virtio input device name: `VIRTIO_INPUT_CFG_ID_NAME`'s
/// string, 128 bytes with no NUL terminator (virtio 1.2, 5.8.4). A guest's
/// driver may keep fewer. Linux's copies `min(size, 64)` bytes into
/// `struct virtio_input`'s `name[64]`, so a name of 64 bytes or more
/// reaches it cut and with no NUL of its own in that buffer, a gap in that
/// driver that the board does not narrow its names to fit.
pub const INPUT_NAME_CAPACITY: usize = 128;
/// The longest EDID display name: the display product name descriptor
/// (tag 0xFC) holds up to 13 characters, a shorter name ended by a line
/// feed (VESA E-EDID 1.4, 3.10.3.4).
pub const DISPLAY_NAME_CAPACITY: usize = 13;

/// Printable ASCII of at most `N` bytes, held in place.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct Text<const N: usize> {
    bytes: [u8; N],
    len: usize,
}

/// Whether `src` is text `rule` lets in: not empty, and of the characters
/// the rule allows.
const fn check(src: &[u8], rule: TextRule) -> Result<(), IdentityProblem> {
    if src.is_empty() {
        return Err(IdentityProblem::Empty);
    }
    if matches!(rule, TextRule::Compatible) && src.len() < 2 {
        return Err(IdentityProblem::NotCompatibleString);
    }
    let mut i = 0;
    while i < src.len() {
        let c = src[i];
        let allowed = match rule {
            TextRule::Printable => matches!(c, 0x20..=0x7E),
            TextRule::Compatible => {
                c.is_ascii_alphanumeric()
                    || (i > 0 && matches!(c, b',' | b'+' | b'-' | b'.' | b'_' | b'/'))
            }
        };
        if !allowed {
            return Err(match rule {
                TextRule::Printable => IdentityProblem::NotPrintableAscii,
                TextRule::Compatible => IdentityProblem::NotCompatibleString,
            });
        }
        i += 1;
    }
    Ok(())
}

impl<const N: usize> Text<N> {
    /// `text`, printable ASCII.
    const fn new(text: &str) -> Result<Self, IdentityProblem> {
        let src = text.as_bytes();
        if let Err(problem) = check(src, TextRule::Printable) {
            return Err(problem);
        }
        if src.len() > N {
            return Err(IdentityProblem::TooLong { max: N });
        }
        let mut bytes = [0u8; N];
        bytes.split_at_mut(src.len()).0.copy_from_slice(src);
        Ok(Self {
            bytes,
            len: src.len(),
        })
    }

    /// The text's bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }

    /// The text.
    pub fn as_str(&self) -> &str {
        match core::str::from_utf8(self.as_bytes()) {
            Ok(text) => text,
            Err(_) => unreachable!("a Text holds only the ASCII its rule let in"),
        }
    }
}

impl<const N: usize> core::fmt::Debug for Text<N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::Debug::fmt(self.as_str(), f)
    }
}

/// Which characters a name may hold.
///
/// Closed (R5).
#[derive(Clone, Copy)]
enum TextRule {
    /// Printable ASCII, 0x20 to 0x7E: a [`Text`].
    Printable,
    /// A `compatible` string as dt-schema's `dt-core.yaml` patterns it,
    /// `^[a-zA-Z0-9][a-zA-Z0-9,+\-._/]+$`: two characters at least, the
    /// first a letter or digit. A [`CompatibleList`]'s strings.
    Compatible,
}

/// A root `compatible`: one string or more, most specific first (a
/// `<stringlist>`, Devicetree Specification v0.4, 2.3.1), each by
/// dt-schema's pattern, held as the device tree holds it, every string
/// ended by a NUL.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct CompatibleList {
    bytes: [u8; COMPATIBLE_CAPACITY],
    len: usize,
}

impl CompatibleList {
    /// `strings`, each checked by dt-schema's pattern.
    const fn new(strings: &[&str]) -> Result<Self, IdentityProblem> {
        if strings.is_empty() {
            return Err(IdentityProblem::Empty);
        }
        let mut bytes = [0u8; COMPATIBLE_CAPACITY];
        let mut len = 0;
        let mut s = 0;
        while s < strings.len() {
            let src = strings[s].as_bytes();
            if let Err(problem) = check(src, TextRule::Compatible) {
                return Err(problem);
            }
            if len + src.len() + 1 > COMPATIBLE_CAPACITY {
                return Err(IdentityProblem::TooLong {
                    max: COMPATIBLE_CAPACITY,
                });
            }
            let (_, free) = bytes.split_at_mut(len);
            free.split_at_mut(src.len()).0.copy_from_slice(src);
            // The byte after the string is the NUL that ends it.
            len += src.len() + 1;
            s += 1;
        }
        Ok(Self { bytes, len })
    }

    /// The property's bytes as the device tree holds them: every string
    /// followed by a NUL.
    pub fn property(&self) -> &[u8] {
        &self.bytes[..self.len]
    }

    /// The strings, most specific first.
    pub fn strings(&self) -> CompatibleStrings<'_> {
        CompatibleStrings {
            rest: self.property(),
        }
    }
}

impl core::fmt::Debug for CompatibleList {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_list().entries(self.strings()).finish()
    }
}

/// The strings of a [`CompatibleList`], most specific first.
#[derive(Debug, Clone)]
pub struct CompatibleStrings<'a> {
    rest: &'a [u8],
}

impl<'a> Iterator for CompatibleStrings<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<&'a str> {
        let end = self.rest.iter().position(|&b| b == 0)?;
        let string = &self.rest[..end];
        self.rest = &self.rest[end + 1..];
        match core::str::from_utf8(string) {
            Ok(text) => Some(text),
            Err(_) => unreachable!("a CompatibleList holds only the ASCII its rule let in"),
        }
    }
}

/// A Plug and Play manufacturer ID, three capital letters: what EDID names
/// a display's maker by (VESA E-EDID 1.4, 3.4.1), from the UEFI Forum's
/// PNP ID registry.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PnpId([u8; 3]);

/// A PNP ID that is not three capital letters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotPnpId;

impl core::fmt::Display for NotPnpId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("a PNP ID is three capital letters")
    }
}

impl core::error::Error for NotPnpId {}

impl PnpId {
    /// `id`, which must be three letters A to Z.
    ///
    /// # Errors
    ///
    /// [`NotPnpId`] for anything else.
    pub const fn new(id: &str) -> Result<Self, NotPnpId> {
        let b = id.as_bytes();
        if b.len() != 3 {
            return Err(NotPnpId);
        }
        let mut i = 0;
        while i < 3 {
            if !b[i].is_ascii_uppercase() {
                return Err(NotPnpId);
            }
            i += 1;
        }
        Ok(Self([b[0], b[1], b[2]]))
    }

    /// The three letters.
    pub const fn letters(self) -> [u8; 3] {
        self.0
    }
}

impl core::fmt::Debug for PnpId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let [a, b, c] = self.0;
        write!(f, "{}{}{}", char::from(a), char::from(b), char::from(c))
    }
}

/// What a virtio-mmio transport's `VendorID` register reads (virtio 1.2,
/// 4.2.2): the maker every virtio device on the board reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtioVendorId(u32);

impl VirtioVendorId {
    /// The ID `id`.
    pub const fn new(id: u32) -> Self {
        Self(id)
    }

    /// The ID whose register reads the four characters `name` in its
    /// little-endian bytes, as QEMU's reads "QEMU".
    pub const fn from_ascii(name: [u8; 4]) -> Self {
        Self(u32::from_le_bytes(name))
    }

    /// The register's value.
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// The vendor a virtio input device reports in
/// `VIRTIO_INPUT_CFG_ID_DEVIDS` (virtio 1.2, 5.8.4), Linux's
/// `input_id.vendor`, on bus `BUS_VIRTUAL`, which has no vendor registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputVendorId(u16);

impl InputVendorId {
    /// The ID `id`.
    pub const fn new(id: u16) -> Self {
        Self(id)
    }

    /// The ID as the configuration gives it.
    pub const fn get(self) -> u16 {
        self.0
    }
}

/// A virtio input device's product, under its vendor, in
/// `VIRTIO_INPUT_CFG_ID_DEVIDS` (virtio 1.2, 5.8.4): Linux's
/// `input_id.product`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputProductId(u16);

impl InputProductId {
    /// The product `id`.
    pub const fn new(id: u16) -> Self {
        Self(id)
    }

    /// The product as the configuration gives it.
    pub const fn get(self) -> u16 {
        self.0
    }
}

/// A virtio input device's version in `VIRTIO_INPUT_CFG_ID_DEVIDS`
/// (virtio 1.2, 5.8.4): Linux's `input_id.version`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputVersion(u16);

impl InputVersion {
    /// The version `version`.
    pub const fn new(version: u16) -> Self {
        Self(version)
    }

    /// The version as the configuration gives it.
    pub const fn get(self) -> u16 {
        self.0
    }
}

/// What the GICv2m frame's `MSI_IIDR` reads: the frame's implementer and
/// product, in the fields Arm's Server Base System Architecture gives the
/// register (ARM-DEN-0029, version 2.2, pages 35 to 39).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MsiFrameIidr(u32);

impl MsiFrameIidr {
    /// No implementer and no product: every field 0.
    pub const NONE: Self = Self(0);

    /// The register's value `value`.
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// The register's value.
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// What a virtio input device reports of its model beside the vendor
/// ([`InputVendorId`]): its product and version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputDeviceId {
    pub product: InputProductId,
    pub version: InputVersion,
}

/// An EDID product code: the maker's number for the display model (VESA
/// E-EDID 1.4, 3.4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EdidProductCode(u16);

impl EdidProductCode {
    /// The code `code`.
    pub const fn new(code: u16) -> Self {
        Self(code)
    }

    /// The code as EDID gives it.
    pub const fn get(self) -> u16 {
        self.0
    }
}

/// A year EDID 1.4 can give for a display's manufacture: a byte counted
/// from 1990 whose values 00h to 0Fh are reserved (VESA E-EDID Release A
/// Revision 2, 3.4.4, Table 3.8), so 2006 to 2245.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct EdidYear(u16);

/// A year outside the years EDID 1.4 can give, 2006 to 2245.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotAnEdidYear {
    pub year: u16,
}

impl core::fmt::Display for NotAnEdidYear {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "{} is not a year EDID 1.4 gives, 2006 to 2245",
            self.year
        )
    }
}

impl core::error::Error for NotAnEdidYear {}

/// The year EDID's year byte counts from (VESA E-EDID 1.4, 3.4.4).
const EDID_YEAR_ZERO: u16 = 1990;

impl EdidYear {
    /// The first year EDID 1.4 can give, byte 10h.
    pub const FIRST: Self = Self(EDID_YEAR_ZERO + 0x10);
    /// The last year EDID 1.4 can give, byte FFh.
    pub const LAST: Self = Self(EDID_YEAR_ZERO + 0xFF);

    /// The year `year`.
    ///
    /// # Errors
    ///
    /// [`NotAnEdidYear`] for a year before [`EdidYear::FIRST`] or after
    /// [`EdidYear::LAST`].
    pub const fn new(year: u16) -> Result<Self, NotAnEdidYear> {
        if year < Self::FIRST.0 || year > Self::LAST.0 {
            Err(NotAnEdidYear { year })
        } else {
            Ok(Self(year))
        }
    }

    /// The year.
    pub const fn get(self) -> u16 {
        self.0
    }

    /// The byte EDID gives it as: years since 1990.
    pub const fn byte(self) -> u8 {
        (self.0 - EDID_YEAR_ZERO) as u8
    }
}

/// The week EDID gives for a display's manufacture (VESA E-EDID 1.4,
/// 3.4.4): none, or a week of the year.
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdidWeek {
    /// No week given: the byte is 0.
    Unstated,
    /// A week of the year.
    Of(EdidWeekNumber),
}

/// A week of the year EDID can give, 1 to 54.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct EdidWeekNumber(u8);

/// A week outside the weeks EDID can give, 1 to 54.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotAnEdidWeek {
    pub week: u8,
}

impl core::fmt::Display for NotAnEdidWeek {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{} is not a week EDID gives, 1 to 54", self.week)
    }
}

impl core::error::Error for NotAnEdidWeek {}

impl EdidWeekNumber {
    /// Week `week` of the year.
    ///
    /// # Errors
    ///
    /// [`NotAnEdidWeek`] for a week outside 1 to 54.
    pub const fn new(week: u8) -> Result<Self, NotAnEdidWeek> {
        if week >= 1 && week <= 54 {
            Ok(Self(week))
        } else {
            Err(NotAnEdidWeek { week })
        }
    }

    /// The week.
    pub const fn get(self) -> u8 {
        self.0
    }
}

impl EdidWeek {
    /// The byte EDID gives it as.
    pub const fn byte(self) -> u8 {
        match self {
            Self::Unstated => 0,
            Self::Of(week) => week.get(),
        }
    }
}

/// When EDID says a display is from (VESA E-EDID 1.4, 3.4.4, Table 3.8):
/// the year it was made, with the week where stated, or its model year.
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdidDate {
    /// Made in `year`, in `week` of it where stated.
    Manufactured { week: EdidWeek, year: EdidYear },
    /// The display's model year: byte 10h reads FFh, the Model Year Flag.
    ModelYear(EdidYear),
}

impl EdidDate {
    /// Byte 10h: the week of manufacture, or the Model Year Flag.
    pub const fn week_byte(self) -> u8 {
        match self {
            Self::Manufactured { week, .. } => week.byte(),
            Self::ModelYear(_) => 0xFF,
        }
    }

    /// Byte 11h: the year of manufacture or the model year.
    pub const fn year_byte(self) -> u8 {
        match self {
            Self::Manufactured { year, .. } | Self::ModelYear(year) => year.byte(),
        }
    }
}

/// The names and numbers a host gives [`BoardIdentity::new`], borrowed
/// until they are checked and copied. [`IdentityNames::RUSTY_BOX`] is a
/// whole set to start from.
#[derive(Debug, Clone, Copy)]
pub struct IdentityNames<'a> {
    /// The device tree's `/model`, printable ASCII.
    pub model: &'a str,
    /// The device tree's root `compatible`: one string or more, most
    /// specific first, each by dt-schema's pattern; the recommended form of
    /// each is `manufacturer,model` (Devicetree Specification v0.4, 2.3.1).
    pub compatible: &'a [&'a str],
    /// The `VendorID` of every virtio-mmio transport.
    pub virtio_vendor: VirtioVendorId,
    /// What the GICv2m frame's `MSI_IIDR` reads.
    pub msi_frame_iidr: MsiFrameIidr,
    /// The virtio input devices' vendor.
    pub input_vendor: InputVendorId,
    /// The virtio keyboard's name, printable ASCII.
    pub keyboard: &'a str,
    /// The virtio keyboard's product and version.
    pub keyboard_id: InputDeviceId,
    /// The virtio tablet's name, printable ASCII.
    pub tablet: &'a str,
    /// The virtio tablet's product and version.
    pub tablet_id: InputDeviceId,
    /// The virtio-gpu display's EDID manufacturer.
    pub display_vendor: PnpId,
    /// The display's EDID product code.
    pub display_product: EdidProductCode,
    /// The display's EDID product name, printable ASCII.
    pub display_name: &'a str,
    /// The display's EDID date: when it was made, or its model year.
    pub display_date: EdidDate,
}

impl IdentityNames<'static> {
    /// This project's names. `RBX` is a PNP ID no maker holds in the UEFI
    /// Forum's registry (its export of 2026-10-31); the virtio vendor reads
    /// "RBOX" in the register's little-endian bytes, and the input vendor
    /// is "RB" read big-endian, 0x5242, whose products are numbered here:
    /// the keyboard 1 and the tablet 2, each at version 1. The GICv2m
    /// frame names no implementer and no product, as the project has no
    /// implementer code.
    pub const RUSTY_BOX: Self = Self {
        model: "Rusty Box virt",
        compatible: &["rusty-box,virt"],
        virtio_vendor: VirtioVendorId::from_ascii(*b"RBOX"),
        msi_frame_iidr: MsiFrameIidr::NONE,
        input_vendor: InputVendorId::new(u16::from_be_bytes(*b"RB")),
        keyboard: "Rusty Box Virtio Keyboard",
        keyboard_id: InputDeviceId {
            product: InputProductId::new(1),
            version: InputVersion::new(1),
        },
        tablet: "Rusty Box Virtio Tablet",
        tablet_id: InputDeviceId {
            product: InputProductId::new(2),
            version: InputVersion::new(1),
        },
        display_vendor: match PnpId::new("RBX") {
            Ok(id) => id,
            Err(_) => panic!("RBX is three capital letters"),
        },
        display_product: EdidProductCode::new(0x0001),
        display_name: "Rusty Box",
        display_date: EdidDate::Manufactured {
            week: EdidWeek::Unstated,
            year: match EdidYear::new(2026) {
                Ok(year) => year,
                Err(_) => panic!("2026 is a year EDID gives"),
            },
        },
    };
}

/// What the board tells a guest it is: see the [module docs](self).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoardIdentity {
    model: Text<MODEL_CAPACITY>,
    compatible: CompatibleList,
    virtio_vendor: VirtioVendorId,
    msi_frame_iidr: MsiFrameIidr,
    input_vendor: InputVendorId,
    keyboard: Text<INPUT_NAME_CAPACITY>,
    keyboard_id: InputDeviceId,
    tablet: Text<INPUT_NAME_CAPACITY>,
    tablet_id: InputDeviceId,
    display: DisplayIdentity,
}

/// One of a [`BoardIdentity`]'s names, the texts [`BoardIdentity::new`]
/// checks; its numbers and the PNP ID are their own types, checked when
/// they are made.
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityField {
    Model,
    Compatible,
    Keyboard,
    Tablet,
    DisplayName,
}

/// What is wrong with a name.
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityProblem {
    Empty,
    TooLong {
        max: usize,
    },
    /// A byte outside printable ASCII, 0x20 to 0x7E.
    NotPrintableAscii,
    /// A `compatible` string not of dt-schema's pattern.
    NotCompatibleString,
}

/// Why [`BoardIdentity::new`] refused its names: the first name, in
/// [`IdentityNames`]' order, that is not one the board can give.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdentityError {
    pub field: IdentityField,
    pub problem: IdentityProblem,
}

impl core::fmt::Display for IdentityError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let field = match self.field {
            IdentityField::Model => "the model",
            IdentityField::Compatible => "the compatible list",
            IdentityField::Keyboard => "the keyboard's name",
            IdentityField::Tablet => "the tablet's name",
            IdentityField::DisplayName => "the display's name",
        };
        match self.problem {
            IdentityProblem::Empty => write!(f, "{field} is empty"),
            IdentityProblem::TooLong { max } => match self.field {
                IdentityField::Compatible => write!(
                    f,
                    "{field} takes more than {max} bytes, each string with the NUL that ends it"
                ),
                IdentityField::Model
                | IdentityField::Keyboard
                | IdentityField::Tablet
                | IdentityField::DisplayName => write!(f, "{field} is longer than {max} bytes"),
            },
            IdentityProblem::NotPrintableAscii => {
                write!(f, "{field} holds a byte outside printable ASCII")
            }
            IdentityProblem::NotCompatibleString => write!(
                f,
                "{field} holds a string not of the pattern ^[a-zA-Z0-9][a-zA-Z0-9,+\\-._/]+$"
            ),
        }
    }
}

impl core::error::Error for IdentityError {}

/// The text `text` of `field`, printable ASCII.
const fn text<const N: usize>(field: IdentityField, text: &str) -> Result<Text<N>, IdentityError> {
    match Text::new(text) {
        Ok(t) => Ok(t),
        Err(problem) => Err(IdentityError { field, problem }),
    }
}

impl BoardIdentity {
    /// This project's identity, [`IdentityNames::RUSTY_BOX`].
    pub const RUSTY_BOX: Self = match Self::new(IdentityNames::RUSTY_BOX) {
        Ok(identity) => identity,
        Err(_) => panic!("IdentityNames::RUSTY_BOX is a valid identity"),
    };

    /// The identity `names` give, each name checked and copied.
    ///
    /// # Errors
    ///
    /// [`IdentityError`] names the first name that is empty, longer than
    /// its capacity, or of characters it cannot hold.
    pub const fn new(names: IdentityNames<'_>) -> Result<Self, IdentityError> {
        let model = match text(IdentityField::Model, names.model) {
            Ok(t) => t,
            Err(e) => return Err(e),
        };
        let compatible = match CompatibleList::new(names.compatible) {
            Ok(list) => list,
            Err(problem) => {
                return Err(IdentityError {
                    field: IdentityField::Compatible,
                    problem,
                })
            }
        };
        let keyboard = match text(IdentityField::Keyboard, names.keyboard) {
            Ok(t) => t,
            Err(e) => return Err(e),
        };
        let tablet = match text(IdentityField::Tablet, names.tablet) {
            Ok(t) => t,
            Err(e) => return Err(e),
        };
        let display_name = match text(IdentityField::DisplayName, names.display_name) {
            Ok(t) => t,
            Err(e) => return Err(e),
        };
        Ok(Self {
            model,
            compatible,
            virtio_vendor: names.virtio_vendor,
            msi_frame_iidr: names.msi_frame_iidr,
            input_vendor: names.input_vendor,
            keyboard,
            keyboard_id: names.keyboard_id,
            tablet,
            tablet_id: names.tablet_id,
            display: DisplayIdentity {
                vendor: names.display_vendor,
                product: names.display_product,
                name: display_name,
                date: names.display_date,
            },
        })
    }

    /// The device tree's `/model`.
    pub fn model(&self) -> &str {
        self.model.as_str()
    }

    /// The device tree's root `compatible`.
    pub const fn compatible(&self) -> &CompatibleList {
        &self.compatible
    }

    /// The virtio-mmio `VendorID`.
    pub const fn virtio_vendor(&self) -> VirtioVendorId {
        self.virtio_vendor
    }

    /// What the GICv2m frame's `MSI_IIDR` reads.
    pub const fn msi_frame_iidr(&self) -> MsiFrameIidr {
        self.msi_frame_iidr
    }

    /// The virtio input devices' vendor.
    pub const fn input_vendor(&self) -> InputVendorId {
        self.input_vendor
    }

    /// The virtio keyboard's name.
    pub fn keyboard(&self) -> &str {
        self.keyboard.as_str()
    }

    /// The virtio keyboard's product and version.
    pub const fn keyboard_id(&self) -> InputDeviceId {
        self.keyboard_id
    }

    /// The virtio tablet's name.
    pub fn tablet(&self) -> &str {
        self.tablet.as_str()
    }

    /// The virtio tablet's product and version.
    pub const fn tablet_id(&self) -> InputDeviceId {
        self.tablet_id
    }

    /// The display's EDID manufacturer.
    pub const fn display_vendor(&self) -> PnpId {
        self.display.vendor
    }

    /// The display's EDID product code.
    pub const fn display_product(&self) -> EdidProductCode {
        self.display.product
    }

    /// The display's EDID product name.
    pub fn display_name(&self) -> &str {
        self.display.name.as_str()
    }

    /// The display's EDID date: when it was made, or its model year.
    pub const fn display_date(&self) -> EdidDate {
        self.display.date
    }

    /// The name of the virtio input device of `kind`, as the device holds
    /// it.
    pub(crate) const fn input_name(&self, kind: InputKind) -> Text<INPUT_NAME_CAPACITY> {
        match kind {
            InputKind::Keyboard => self.keyboard,
            InputKind::Tablet => self.tablet,
        }
    }

    /// The product and version of the virtio input device of `kind`.
    pub(crate) const fn input_id(&self, kind: InputKind) -> InputDeviceId {
        match kind {
            InputKind::Keyboard => self.keyboard_id,
            InputKind::Tablet => self.tablet_id,
        }
    }

    /// What the virtio-gpu display's EDID says of it.
    #[cfg(feature = "alloc")]
    pub(crate) const fn display(&self) -> DisplayIdentity {
        self.display
    }

    /// Every name and number, each passed to `part` as the bytes a
    /// fingerprint of the identity takes: a name followed by a NUL, so no
    /// two identities' names run together alike (the `compatible` list as
    /// its property's bytes, whose strings end in NULs, and no string is
    /// empty, so its extra NUL marks where it ends), a number little-endian,
    /// the PNP ID as its letters and the date as its two EDID bytes. Every
    /// field counts, whether or not the machine's devices show it to the
    /// guest: the identity is part of the machine's shape, as its RAM is.
    /// The destructuring names every field, so a field added to the
    /// identity cannot be left out.
    pub(crate) fn each_part<F: FnMut(&[u8])>(&self, mut part: F) {
        let Self {
            model,
            compatible,
            virtio_vendor,
            msi_frame_iidr,
            input_vendor,
            keyboard,
            keyboard_id,
            tablet,
            tablet_id,
            display,
        } = self;
        let DisplayIdentity {
            vendor,
            product,
            name,
            date,
        } = display;
        for text in [
            model.as_bytes(),
            compatible.property(),
            keyboard.as_bytes(),
            tablet.as_bytes(),
            name.as_bytes(),
        ] {
            part(text);
            part(&[0]);
        }
        part(&virtio_vendor.get().to_le_bytes());
        part(&msi_frame_iidr.get().to_le_bytes());
        part(&input_vendor.get().to_le_bytes());
        for InputDeviceId { product, version } in [keyboard_id, tablet_id] {
            part(&product.get().to_le_bytes());
            part(&version.get().to_le_bytes());
        }
        part(&vendor.letters());
        part(&product.get().to_le_bytes());
        part(&[date.week_byte(), date.year_byte()]);
    }
}

/// What a display's EDID says of it (VESA E-EDID 1.4, 3.4, 3.10.3.4): its
/// maker, product, name and date.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DisplayIdentity {
    pub(crate) vendor: PnpId,
    pub(crate) product: EdidProductCode,
    pub(crate) name: Text<DISPLAY_NAME_CAPACITY>,
    pub(crate) date: EdidDate,
}

impl Default for BoardIdentity {
    fn default() -> Self {
        Self::RUSTY_BOX
    }
}
