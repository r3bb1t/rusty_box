//! A Windows kernel's driver objects and their devices, decoded from the
//! `_DRIVER_OBJECT` / `_DEVICE_OBJECT` layouts (`wdm.h`), which are identical
//! on every x64 Windows, and the two `_OBJECT_HEADER` name-lookup schemes.
//!
//! These structures carry no build-specific field here: the offsets below hold
//! on Windows XP x64 through Windows 11. Only an object's *name* is reached
//! differently between eras, which [`name_info_delta`] resolves.

/// `_DRIVER_OBJECT` is 0x150 bytes on every x64 Windows.
pub(crate) const DRIVER_OBJECT_SIZE: usize = 0x150;
/// `IO_TYPE_DRIVER`, the `_DRIVER_OBJECT.Type`.
pub(crate) const IO_TYPE_DRIVER: u16 = 4;
/// `IRP_MJ_MAXIMUM_FUNCTION + 1`: the length of `MajorFunction`.
pub(crate) const MAJOR_FUNCTION_COUNT: usize = 28;

const D_TYPE: usize = 0x00;
const D_SIZE: usize = 0x02;
const D_DEVICE_OBJECT: usize = 0x08;
const D_DRIVER_START: usize = 0x18;
const D_DRIVER_SIZE: usize = 0x20;
const D_FAST_IO: usize = 0x50;
const D_INIT: usize = 0x58;
const D_START_IO: usize = 0x60;
const D_UNLOAD: usize = 0x68;
const D_MAJOR: usize = 0x70;

/// A decoded `_DRIVER_OBJECT` body: its raw pointers, not yet classified.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DriverObject {
    pub(crate) start: u64,
    pub(crate) size: u32,
    pub(crate) device_object: u64,
    pub(crate) fast_io: u64,
    pub(crate) init: u64,
    pub(crate) start_io: u64,
    pub(crate) unload: u64,
    pub(crate) major: [u64; MAJOR_FUNCTION_COUNT],
}

fn qword(b: &[u8], at: usize) -> Option<u64> {
    let s: [u8; 8] = b.get(at..at.checked_add(8)?)?.try_into().ok()?;
    Some(u64::from_le_bytes(s))
}

fn dword(b: &[u8], at: usize) -> Option<u32> {
    let s: [u8; 4] = b.get(at..at.checked_add(4)?)?.try_into().ok()?;
    Some(u32::from_le_bytes(s))
}

fn word(b: &[u8], at: usize) -> Option<u16> {
    let s: [u8; 2] = b.get(at..at.checked_add(2)?)?.try_into().ok()?;
    Some(u16::from_le_bytes(s))
}

/// Decodes a `_DRIVER_OBJECT` from a 0x150-byte body, or `None` when the bytes
/// are not a driver object (`Type` `IO_TYPE_DRIVER`, `Size` 0x150).
pub(crate) fn parse_driver(body: &[u8]) -> Option<DriverObject> {
    let b = body.get(..DRIVER_OBJECT_SIZE)?;
    if word(b, D_TYPE)? != IO_TYPE_DRIVER || usize::from(word(b, D_SIZE)?) != DRIVER_OBJECT_SIZE {
        return None;
    }
    let mut major = [0u64; MAJOR_FUNCTION_COUNT];
    for (i, slot) in major.iter_mut().enumerate() {
        *slot = qword(b, D_MAJOR.checked_add(i.checked_mul(8)?)?)?;
    }
    Some(DriverObject {
        start: qword(b, D_DRIVER_START)?,
        size: dword(b, D_DRIVER_SIZE)?,
        device_object: qword(b, D_DEVICE_OBJECT)?,
        fast_io: qword(b, D_FAST_IO)?,
        init: qword(b, D_INIT)?,
        start_io: qword(b, D_START_IO)?,
        unload: qword(b, D_UNLOAD)?,
        major,
    })
}

/// The `IRP_MJ_*` name for a `MajorFunction` index, in dispatch-table order.
pub(crate) fn major_function_name(index: usize) -> &'static str {
    const NAMES: [&str; MAJOR_FUNCTION_COUNT] = [
        "CREATE",
        "CREATE_NAMED_PIPE",
        "CLOSE",
        "READ",
        "WRITE",
        "QUERY_INFORMATION",
        "SET_INFORMATION",
        "QUERY_EA",
        "SET_EA",
        "FLUSH_BUFFERS",
        "QUERY_VOLUME_INFORMATION",
        "SET_VOLUME_INFORMATION",
        "DIRECTORY_CONTROL",
        "FILE_SYSTEM_CONTROL",
        "DEVICE_CONTROL",
        "INTERNAL_DEVICE_CONTROL",
        "SHUTDOWN",
        "LOCK_CONTROL",
        "CLEANUP",
        "CREATE_MAILSLOT",
        "QUERY_SECURITY",
        "SET_SECURITY",
        "POWER",
        "SYSTEM_CONTROL",
        "DEVICE_CHANGE",
        "QUERY_QUOTA",
        "SET_QUOTA",
        "PNP",
    ];
    NAMES.get(index).copied().unwrap_or("?")
}

/// Which `_OBJECT_HEADER` name-lookup scheme a Windows build uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ObjectHeaderEra {
    /// XP / Server 2003: `NameInfoOffset`, a byte at header+0x18, is the
    /// distance back to the `NAME_INFO`.
    Legacy,
    /// Vista and later: `InfoMask`, a byte at header+0x1A, selects which
    /// optional headers precede the object header.
    Modern,
}

/// `_OBJECT_HEADER.NameInfoOffset` (Legacy) / `InfoMask` (Modern) byte offset.
pub(crate) const LEGACY_NAME_INFO_OFFSET: u64 = 0x18;
pub(crate) const MODERN_INFO_MASK_OFFSET: u64 = 0x1A;
/// An object body sits 0x30 past its `_OBJECT_HEADER` on every x64 Windows;
/// the directories hold pointers to bodies, so the header is body-0x30.
pub(crate) const OBJECT_BODY_OFFSET: u64 = 0x30;
/// Each optional header (`CreatorInfo`, `NameInfo`, …) is 0x20 bytes.
const OPTIONAL_HEADER_SIZE: u64 = 0x20;
/// `NameInfo` with `CreatorInfo` before it: two optional headers.
const NAME_AND_CREATOR_SIZE: u64 = 2 * OPTIONAL_HEADER_SIZE;
/// `InfoMask` bits: `CreatorInfo` present (bit 0), `NameInfo` present (bit 1).
const INFO_MASK_CREATOR: u8 = 0x01;
const INFO_MASK_NAME: u8 = 0x02;

/// The distance *back* from an `_OBJECT_HEADER` to its `OBJECT_HEADER_NAME_INFO`,
/// or `None` when the object has no name. `name_byte` is the header's
/// `NameInfoOffset` (Legacy) or `InfoMask` (Modern); the caller reads whichever
/// the era names.
pub(crate) fn name_info_delta(era: ObjectHeaderEra, name_byte: u8) -> Option<u64> {
    match era {
        ObjectHeaderEra::Legacy => (name_byte != 0).then(|| u64::from(name_byte)),
        ObjectHeaderEra::Modern => (name_byte & INFO_MASK_NAME != 0).then(|| {
            // NameInfo is the header just before the object header; CreatorInfo,
            // when present, precedes NameInfo, so it adds its own size.
            if name_byte & INFO_MASK_CREATOR != 0 {
                NAME_AND_CREATOR_SIZE
            } else {
                OPTIONAL_HEADER_SIZE
            }
        }),
    }
}

/// `OBJECT_HEADER_NAME_INFO.Name` (`UNICODE_STRING`) is 8 bytes into the info.
pub(crate) const NAME_INFO_NAME: u64 = 0x08;
/// `OBJECT_DIRECTORY.HashBuckets[37]` at +0.
pub(crate) const OBJECT_DIRECTORY_BUCKETS: usize = 37;
/// `OBJECT_DIRECTORY_ENTRY`: `ChainLink` at +0, `Object` at +8.
pub(crate) const DIRECTORY_ENTRY_CHAIN: u64 = 0x00;
pub(crate) const DIRECTORY_ENTRY_OBJECT: u64 = 0x08;
/// `OBJECT_SYMBOLIC_LINK.LinkTarget` (`UNICODE_STRING`) at +8, both eras.
pub(crate) const SYMBOLIC_LINK_TARGET: u64 = 0x08;

const DEV_DRIVER_OBJECT: usize = 0x08;
const DEV_NEXT: usize = 0x10;
const DEV_ATTACHED: usize = 0x18;
const DEV_FLAGS: usize = 0x30;
const DEV_CHARACTERISTICS: usize = 0x34;
const DEV_DEVICE_TYPE: usize = 0x48;
/// How much of a `_DEVICE_OBJECT` is read to decode its fields.
pub(crate) const DEVICE_OBJECT_SPAN: usize = 0x50;

/// A decoded `_DEVICE_OBJECT`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DeviceObject {
    pub(crate) driver: u64,
    pub(crate) next: u64,
    pub(crate) attached: u64,
    pub(crate) device_type: u32,
    pub(crate) characteristics: u32,
    pub(crate) flags: u32,
}

/// Decodes a `_DEVICE_OBJECT` from the first `DEVICE_OBJECT_SPAN` bytes.
pub(crate) fn parse_device(body: &[u8]) -> Option<DeviceObject> {
    Some(DeviceObject {
        driver: qword(body, DEV_DRIVER_OBJECT)?,
        next: qword(body, DEV_NEXT)?,
        attached: qword(body, DEV_ATTACHED)?,
        device_type: dword(body, DEV_DEVICE_TYPE)?,
        characteristics: dword(body, DEV_CHARACTERISTICS)?,
        flags: dword(body, DEV_FLAGS)?,
    })
}

/// The `FILE_DEVICE_*` name for a `_DEVICE_OBJECT.DeviceType`, or "0xNN" when
/// it is not one of the common types.
pub(crate) fn device_type_name(device_type: u32) -> String {
    let name = match device_type {
        0x01 => "BEEP",
        0x02 => "CD_ROM",
        0x03 => "CD_ROM_FILE_SYSTEM",
        0x04 => "CONTROLLER",
        0x05 => "DATALINK",
        0x06 => "DFS",
        0x07 => "DISK",
        0x08 => "DISK_FILE_SYSTEM",
        0x09 => "FILE_SYSTEM",
        0x0A => "INPORT_PORT",
        0x0B => "KEYBOARD",
        0x0C => "MAILSLOT",
        0x0D => "MIDI_IN",
        0x0E => "MIDI_OUT",
        0x0F => "MOUSE",
        0x10 => "MULTI_UNC_PROVIDER",
        0x11 => "NAMED_PIPE",
        0x12 => "NETWORK",
        0x13 => "NETWORK_BROWSER",
        0x14 => "NETWORK_FILE_SYSTEM",
        0x15 => "NULL",
        0x16 => "PARALLEL_PORT",
        0x17 => "PHYSICAL_NETCARD",
        0x18 => "PRINTER",
        0x19 => "SCANNER",
        0x1A => "SERIAL_MOUSE_PORT",
        0x1B => "SERIAL_PORT",
        0x1C => "SCREEN",
        0x1D => "SOUND",
        0x1E => "STREAMS",
        0x1F => "TAPE",
        0x20 => "TAPE_FILE_SYSTEM",
        0x21 => "TRANSPORT",
        0x22 => "UNKNOWN",
        0x23 => "VIDEO",
        0x24 => "VIRTUAL_DISK",
        0x25 => "WAVE_IN",
        0x26 => "WAVE_OUT",
        0x27 => "8042_PORT",
        0x28 => "NETWORK_REDIRECTOR",
        0x29 => "BATTERY",
        0x2A => "BUS_EXTENDER",
        0x2B => "MODEM",
        0x2C => "VDM",
        0x2D => "MASS_STORAGE",
        0x2E => "SMB",
        0x2F => "KS",
        0x30 => "CHANGER",
        0x31 => "SMARTCARD",
        0x32 => "ACPI",
        0x33 => "DVD",
        0x34 => "FULLSCREEN_VIDEO",
        0x35 => "DFS_FILE_SYSTEM",
        0x36 => "DFS_VOLUME",
        0x37 => "SERENUM",
        0x38 => "TERMSRV",
        0x39 => "KSEC",
        _ => return format!("0x{device_type:X}"),
    };
    name.to_owned()
}

/// Walks stop at these bounds, so a corrupt list cannot spin the run thread.
pub(crate) const MAX_DRIVERS: usize = 4096;
pub(crate) const MAX_DEVICES_PER_DRIVER: usize = 4096;
pub(crate) const MAX_ATTACH_DEPTH: usize = 64;

#[cfg(test)]
mod tests {
    use super::*;

    /// A driver object body with `start`/`size` and one hooked major handler.
    fn driver_body(start: u64, size: u32, read_handler: u64) -> Vec<u8> {
        let mut b = vec![0u8; DRIVER_OBJECT_SIZE];
        b[D_TYPE..D_TYPE + 2].copy_from_slice(&IO_TYPE_DRIVER.to_le_bytes());
        b[D_SIZE..D_SIZE + 2].copy_from_slice(&(DRIVER_OBJECT_SIZE as u16).to_le_bytes());
        b[D_DRIVER_START..D_DRIVER_START + 8].copy_from_slice(&start.to_le_bytes());
        b[D_DRIVER_SIZE..D_DRIVER_SIZE + 4].copy_from_slice(&size.to_le_bytes());
        // IRP_MJ_READ = 3.
        let read = D_MAJOR + 3 * 8;
        b[read..read + 8].copy_from_slice(&read_handler.to_le_bytes());
        b
    }

    #[test]
    fn a_driver_object_decodes_its_start_size_and_handlers() {
        let driver = parse_driver(&driver_body(0xFFFF_F800_0100_0000, 0x4000, 0xDEAD_BEEF))
            .expect("a 0x150 Type-4 body is a driver");
        assert_eq!(driver.start, 0xFFFF_F800_0100_0000);
        assert_eq!(driver.size, 0x4000);
        assert_eq!(driver.major[3], 0xDEAD_BEEF);
    }

    #[test]
    fn a_body_that_is_not_a_driver_object_decodes_to_none() {
        let mut b = driver_body(0, 0, 0);
        b[D_TYPE] = 3; // not IO_TYPE_DRIVER
        assert_eq!(parse_driver(&b), None);
        let mut short = driver_body(0, 0, 0);
        short.truncate(DRIVER_OBJECT_SIZE - 1);
        assert_eq!(parse_driver(&short), None);
    }

    #[test]
    fn the_legacy_era_reads_the_name_offset_byte() {
        assert_eq!(name_info_delta(ObjectHeaderEra::Legacy, 0x10), Some(0x10));
        assert_eq!(name_info_delta(ObjectHeaderEra::Legacy, 0), None, "no name");
    }

    #[test]
    fn the_modern_era_sums_the_optional_headers_before_the_name() {
        // NameInfo present, CreatorInfo absent: one header back.
        assert_eq!(name_info_delta(ObjectHeaderEra::Modern, INFO_MASK_NAME), Some(0x20));
        // NameInfo and CreatorInfo both present: two headers back.
        assert_eq!(
            name_info_delta(ObjectHeaderEra::Modern, INFO_MASK_NAME | INFO_MASK_CREATOR),
            Some(0x40)
        );
        // NameInfo absent: no name even if CreatorInfo is present.
        assert_eq!(name_info_delta(ObjectHeaderEra::Modern, INFO_MASK_CREATOR), None);
    }

    #[test]
    fn major_functions_are_named_in_irp_order() {
        assert_eq!(major_function_name(0), "CREATE");
        assert_eq!(major_function_name(14), "DEVICE_CONTROL");
        assert_eq!(major_function_name(27), "PNP");
    }

    #[test]
    fn a_device_type_is_named_or_shown_in_hex() {
        assert_eq!(device_type_name(0x07), "DISK");
        assert_eq!(device_type_name(0xFEED), "0xFEED");
    }
}
