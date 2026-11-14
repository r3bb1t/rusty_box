//! Alpine packages checked as `apk` checks them: each repository index's
//! RSA signature against Alpine's keys (taken from the ISO's own initramfs),
//! and each package's control section against the checksum the signed index
//! gives it.
//!
//! An index (`APKINDEX.tar.gz`) and a package (`.apk`) are concatenated
//! gzip members. The index's first member is a tar holding the signature,
//! `.SIGN.RSA.<key file>`, an RSA PKCS#1 v1.5 signature over the SHA-1 of
//! every byte after that member; the rest is a tar holding `APKINDEX`. A
//! package's second member is its control section, whose SHA-1 the index
//! names as `C:Q1<base64>`.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;

use base64::Engine;
use rsa::pkcs8::DecodePublicKey;
use rsa::{Pkcs1v15Sign, RsaPublicKey};
use sha1::{Digest, Sha1};

/// The repositories of the branch the packages come from, searched in this
/// order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Repository {
    Main,
    Community,
}

impl Repository {
    pub const ALL: [Self; 2] = [Self::Main, Self::Community];

    /// The repository's directory on a mirror.
    pub fn name(self) -> &'static str {
        match self {
            Self::Main => "main",
            Self::Community => "community",
        }
    }
}

/// The cpio "newc" header (and its CRC variant): magic, then thirteen
/// 8-digit hexadecimal fields, the file size the 7th and the name size
/// (with its NUL) the 12th; name and data each padded to 4 bytes.
const NEWC_MAGIC: &[u8] = b"070701";
const NEWC_CRC_MAGIC: &[u8] = b"070702";
const NEWC_HEADER_LEN: usize = 110;
const NEWC_FILE_SIZE_FIELD: usize = 6;
const NEWC_NAME_SIZE_FIELD: usize = 11;
const NEWC_TRAILER: &str = "TRAILER!!!";
/// Where an initramfs keeps apk's keys.
const KEYS_DIR: &str = "etc/apk/keys/";
const KEY_SUFFIX: &str = ".pub";
/// The signature's name in an index's first member, before the key's.
const SIGNATURE_PREFIX: &str = ".SIGN.RSA.";
/// The index's file in its tar.
const INDEX_FILE: &str = "APKINDEX";
/// The prefix of a SHA-1 checksum in an index's `C:` field.
const CHECKSUM_PREFIX: &str = "Q1";

/// Alpine's apk keys, by file name, as PEM.
pub struct ApkKeys {
    by_name: BTreeMap<String, Vec<u8>>,
}

impl ApkKeys {
    /// The keys' file names, in order.
    pub fn names(&self) -> Vec<&str> {
        self.by_name.keys().map(String::as_str).collect()
    }

    /// The public key in the file `name`.
    fn public_key(&self, name: &str) -> Result<RsaPublicKey, String> {
        let pem = self
            .by_name
            .get(name)
            .ok_or_else(|| format!("no apk key {name} in the initramfs"))?;
        let text = std::str::from_utf8(pem).map_err(|e| format!("apk key {name}: {e}"))?;
        RsaPublicKey::from_public_key_pem(text).map_err(|e| format!("apk key {name}: {e}"))
    }
}

/// The apk keys (`etc/apk/keys/*.pub`) in a gzip'd newc cpio initramfs.
pub fn initramfs_keys(initramfs: &[u8]) -> Result<ApkKeys, String> {
    let archive = gunzip_all(initramfs).map_err(|e| format!("the initramfs: {e}"))?;
    let mut by_name = BTreeMap::new();
    let mut at = 0;
    while let Some(magic) = archive.get(at..at + NEWC_MAGIC.len()) {
        if magic != NEWC_MAGIC && magic != NEWC_CRC_MAGIC {
            break;
        }
        let header = archive
            .get(at..at + NEWC_HEADER_LEN)
            .ok_or("the initramfs ends in a cpio header")?;
        let file_size = newc_field(header, NEWC_FILE_SIZE_FIELD)?;
        let name_size = newc_field(header, NEWC_NAME_SIZE_FIELD)?;
        let name_at = at + NEWC_HEADER_LEN;
        let name_bytes = archive
            .get(name_at..name_at + name_size.saturating_sub(1))
            .ok_or("a cpio name runs past the initramfs")?;
        let name = std::str::from_utf8(name_bytes).map_err(|e| format!("a cpio name: {e}"))?;
        let data_at = align4(name_at + name_size);
        let data = archive
            .get(data_at..data_at + file_size)
            .ok_or_else(|| format!("{name} runs past the initramfs"))?;
        at = align4(data_at + file_size);
        if name.starts_with(KEYS_DIR) && name.ends_with(KEY_SUFFIX) {
            let file_name = match name.rfind('/') {
                Some(slash) => &name[slash + 1..],
                None => name,
            };
            by_name.insert(file_name.to_owned(), data.to_vec());
        }
        if name == NEWC_TRAILER {
            break;
        }
    }
    if by_name.is_empty() {
        return Err("no apk keys in the initramfs".into());
    }
    Ok(ApkKeys { by_name })
}

fn align4(offset: usize) -> usize {
    (offset + 3) & !3
}

/// The `index`th hexadecimal field of a newc header.
fn newc_field(header: &[u8], index: usize) -> Result<usize, String> {
    let at = NEWC_MAGIC.len() + 8 * index;
    let digits = std::str::from_utf8(&header[at..at + 8]).map_err(|e| e.to_string())?;
    usize::from_str_radix(digits, 16).map_err(|e| format!("cpio header field {digits:?}: {e}"))
}

/// An index whose signature verified.
pub struct SignedIndex {
    /// The `APKINDEX` text.
    pub text: String,
    /// The file name of the key that signed it.
    pub signer: String,
}

/// The `APKINDEX` of an `APKINDEX.tar.gz`, once its signature verifies
/// against one of `keys`.
pub fn verify_index(archive: &[u8], keys: &ApkKeys) -> Result<SignedIndex, String> {
    let signature_end = gzip_member_end(archive, 0)?;
    let signature_tar = gunzip_all(&archive[..signature_end])
        .map_err(|e| format!("the index's signature member: {e}"))?;
    let signature = tar_members(&signature_tar)?
        .into_iter()
        .find(|member| member.name.starts_with(SIGNATURE_PREFIX))
        .ok_or("the index has no RSA signature")?;
    let signer = signature.name[SIGNATURE_PREFIX.len()..].to_owned();
    let signed = &archive[signature_end..];
    if signed.is_empty() {
        return Err("the index has nothing after its signature".into());
    }
    keys.public_key(&signer)?
        .verify(
            Pkcs1v15Sign::new::<Sha1>(),
            &Sha1::digest(signed),
            &signature.contents,
        )
        .map_err(|e| format!("the index's signature by {signer} does not verify: {e}"))?;
    let index_tar = gunzip_all(signed).map_err(|e| format!("the signed index: {e}"))?;
    let index = tar_members(&index_tar)?
        .into_iter()
        .find(|member| member.name == INDEX_FILE)
        .ok_or("the signed index has no APKINDEX")?;
    let text = String::from_utf8(index.contents).map_err(|e| format!("APKINDEX: {e}"))?;
    Ok(SignedIndex { text, signer })
}

/// One package of an index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    pub name: String,
    pub version: String,
    pub repository: Repository,
    /// `D:`: its dependencies, as apk spells them (`so:libz.so.1`,
    /// `musl>=1.2`, `!conflict`).
    depends: Vec<String>,
    /// `C:`: `Q1` and the base64 SHA-1 of its control section.
    checksum: String,
}

impl Package {
    /// Its file name in its repository.
    pub fn file_name(&self) -> String {
        format!("{}-{}.apk", self.name, self.version)
    }
}

/// The packages of every index read, and which package provides each name.
#[derive(Default)]
pub struct Index {
    packages: BTreeMap<String, Package>,
    /// The first package whose `p:` names a name; a package of the name
    /// itself is preferred when there is one.
    providers: BTreeMap<String, String>,
}

impl Index {
    /// Read `text`, an `APKINDEX` of `repository`: blank-line separated
    /// blocks of `K:value` lines. A package an earlier index also has is
    /// replaced; a name an earlier package provides keeps that provider.
    pub fn add(&mut self, text: &str, repository: Repository) -> Result<(), String> {
        for block in text.trim().split("\n\n") {
            let mut fields: BTreeMap<&str, &str> = BTreeMap::new();
            for line in block.lines() {
                match line.find(':') {
                    Some(colon) => fields.insert(&line[..colon], &line[colon + 1..]),
                    None => fields.insert(line, ""),
                };
            }
            let field = |key: &str| {
                fields
                    .get(key)
                    .copied()
                    .ok_or_else(|| format!("an APKINDEX block has no {key}: field"))
            };
            let name = field("P")?.to_owned();
            for token in fields
                .get("p")
                .copied()
                .unwrap_or_default()
                .split_whitespace()
            {
                self.providers
                    .entry(base_name(token).to_owned())
                    .or_insert_with(|| name.clone());
            }
            let package = Package {
                name: name.clone(),
                version: field("V")?.to_owned(),
                repository,
                depends: fields
                    .get("D")
                    .copied()
                    .unwrap_or_default()
                    .split_whitespace()
                    .map(str::to_owned)
                    .collect(),
                checksum: field("C")?.to_owned(),
            };
            self.packages.insert(name, package);
        }
        Ok(())
    }

    /// `roots` and everything they depend on, by name; conflicts (`!`)
    /// are not dependencies.
    pub fn resolve(&self, roots: &[&str]) -> Result<Vec<&Package>, String> {
        let mut wanted: BTreeSet<&str> = BTreeSet::new();
        let mut todo: Vec<&str> = roots.to_vec();
        while let Some(token) = todo.pop() {
            if token.starts_with('!') {
                continue;
            }
            let name = base_name(token);
            let package = match self.packages.get(name) {
                Some(package) => package,
                None => self
                    .providers
                    .get(name)
                    .and_then(|provider| self.packages.get(provider))
                    .ok_or_else(|| format!("nothing provides {token}"))?,
            };
            if wanted.insert(&package.name) {
                todo.extend(package.depends.iter().map(String::as_str));
            }
        }
        Ok(wanted
            .into_iter()
            .filter_map(|name| self.packages.get(name))
            .collect())
    }
}

/// A dependency or provision's name, its version constraint dropped.
fn base_name(token: &str) -> &str {
    match token.find(['<', '>', '=', '~']) {
        Some(at) => &token[..at],
        None => token,
    }
}

/// Whether `apk`'s control section is the one the index names.
pub fn check_package(package: &Package, apk: &[u8]) -> Result<(), String> {
    let control_start = gzip_member_end(apk, 0)?;
    let control_end = gzip_member_end(apk, control_start)?;
    let checksum = format!(
        "{CHECKSUM_PREFIX}{}",
        base64::engine::general_purpose::STANDARD
            .encode(Sha1::digest(&apk[control_start..control_end]))
    );
    if checksum == package.checksum {
        Ok(())
    } else {
        Err(format!(
            "{}: checksum {checksum} is not the index's {}",
            package.file_name(),
            package.checksum
        ))
    }
}

/// Where the gzip member starting at `start` of `data` ends: the decoder
/// takes from its `BufRead` exactly the member's bytes, trailer included.
pub fn gzip_member_end(data: &[u8], start: usize) -> Result<usize, String> {
    let member = data
        .get(start..)
        .filter(|rest| !rest.is_empty())
        .ok_or_else(|| format!("no gzip member at byte {start}"))?;
    let mut decoder = flate2::bufread::GzDecoder::new(member);
    let mut sink = vec![0u8; 64 * 1024];
    loop {
        match decoder.read(&mut sink) {
            Ok(0) => break,
            Ok(_) => {}
            Err(e) => return Err(format!("the gzip member at byte {start}: {e}")),
        }
    }
    Ok(data.len() - decoder.into_inner().len())
}

/// Every gzip member of `data`, decompressed and joined.
pub fn gunzip_all(data: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    flate2::bufread::MultiGzDecoder::new(data)
        .read_to_end(&mut out)
        .map_err(|e| e.to_string())?;
    Ok(out)
}

/// One file of a tar.
struct TarMember {
    name: String,
    contents: Vec<u8>,
}

/// Every member of a tar, which may end without its two zero blocks (an
/// index's signature tar does).
fn tar_members(tar: &[u8]) -> Result<Vec<TarMember>, String> {
    let mut archive = tar::Archive::new(tar);
    let mut members = Vec::new();
    for entry in archive.entries().map_err(|e| format!("tar: {e}"))? {
        let mut entry = entry.map_err(|e| format!("tar: {e}"))?;
        let name = entry
            .path()
            .map_err(|e| format!("tar: {e}"))?
            .to_string_lossy()
            .into_owned();
        let mut contents = Vec::new();
        entry
            .read_to_end(&mut contents)
            .map_err(|e| format!("tar {name}: {e}"))?;
        members.push(TarMember { name, contents });
    }
    Ok(members)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn gzip(bytes: &[u8]) -> Vec<u8> {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(bytes).expect("compress");
        encoder.finish().expect("finish")
    }

    /// Each member's end is found exactly, so the bytes between two ends are
    /// one member, as the checksum and the signature need.
    #[test]
    fn concatenated_gzip_members_are_split_exactly() {
        let members = [gzip(b"signature"), gzip(&[7u8; 100_000]), gzip(b"data")];
        let joined: Vec<u8> = members.concat();
        let first = gzip_member_end(&joined, 0).expect("first");
        let second = gzip_member_end(&joined, first).expect("second");
        let third = gzip_member_end(&joined, second).expect("third");
        assert_eq!(first, members[0].len());
        assert_eq!(second, first + members[1].len());
        assert_eq!(third, joined.len());
        assert!(gzip_member_end(&joined, third).is_err());
        assert_eq!(
            gunzip_all(&joined[first..]).expect("gunzip"),
            [&[7u8; 100_000][..], b"data"].concat()
        );
    }

    fn newc(name: &str, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(NEWC_MAGIC);
        let fields = [
            0,
            0o100644,
            0,
            0,
            1,
            0,
            data.len(),
            0,
            0,
            0,
            0,
            name.len() + 1,
            0,
        ];
        for field in fields {
            out.extend_from_slice(format!("{field:08X}").as_bytes());
        }
        out.extend_from_slice(name.as_bytes());
        out.push(0);
        while out.len() % 4 != 0 {
            out.push(0);
        }
        out.extend_from_slice(data);
        while out.len() % 4 != 0 {
            out.push(0);
        }
        out
    }

    #[test]
    fn keys_come_from_the_initramfs_keys_directory() {
        let cpio = [
            newc("etc", b""),
            newc("etc/apk/keys/alpine-a.rsa.pub", b"A-PEM"),
            newc("etc/apk/keys/README", b"not a key"),
            newc("etc/apk/keys/alpine-b.rsa.pub", b"B-PEM!"),
            newc(NEWC_TRAILER, b""),
            newc("etc/apk/keys/after-the-trailer.rsa.pub", b"C"),
        ]
        .concat();
        let keys = initramfs_keys(&gzip(&cpio)).expect("keys");
        assert_eq!(keys.names(), ["alpine-a.rsa.pub", "alpine-b.rsa.pub"]);
        assert_eq!(keys.by_name["alpine-b.rsa.pub"], b"B-PEM!");
        assert!(initramfs_keys(&gzip(&newc(NEWC_TRAILER, b""))).is_err());
    }

    const MAIN: &str = "C:Q1aaaa=\nP:qemu-system-aarch64\nV:10.1.5-r0\n\
D:qemu so:libglib-2.0.so.0 !qemu-old musl>=1.2\n\n\
C:Q1bbbb=\nP:qemu\nV:10.1.5-r0\n\n\
C:Q1cccc=\nP:glib\nV:2.86-r0\np:so:libglib-2.0.so.0=0.8600.0 so:libgio-2.0.so.0=0.8600.0\n\n\
C:Q1dddd=\nP:musl\nV:1.2.5-r10\np:so:libc.musl-aarch64.so.1=1\n\n\
C:Q1eeee=\nP:unrelated\nV:1\n";

    #[test]
    fn dependencies_resolve_through_providers() {
        let mut index = Index::default();
        index.add(MAIN, Repository::Main).expect("add");
        let names: Vec<String> = index
            .resolve(&["qemu-system-aarch64"])
            .expect("resolve")
            .iter()
            .map(|p| p.file_name())
            .collect();
        assert_eq!(
            names,
            [
                "glib-2.86-r0.apk",
                "musl-1.2.5-r10.apk",
                "qemu-10.1.5-r0.apk",
                "qemu-system-aarch64-10.1.5-r0.apk"
            ]
        );
        assert!(index.resolve(&["so:libmissing.so.1"]).is_err());
    }

    #[test]
    fn a_later_repository_replaces_a_package_but_not_a_provider() {
        let mut index = Index::default();
        index.add(MAIN, Repository::Main).expect("main");
        index
            .add(
                "C:Q1ffff=\nP:qemu\nV:10.2-r0\n\nC:Q1gggg=\nP:glib2\nV:9\np:so:libglib-2.0.so.0=9\n",
                Repository::Community,
            )
            .expect("community");
        let resolved = index.resolve(&["qemu-system-aarch64"]).expect("resolve");
        let qemu = resolved.iter().find(|p| p.name == "qemu").expect("qemu");
        assert_eq!(qemu.version, "10.2-r0");
        assert_eq!(qemu.repository, Repository::Community);
        assert!(resolved.iter().any(|p| p.name == "glib"));
        assert!(!resolved.iter().any(|p| p.name == "glib2"));
    }

    #[test]
    fn a_block_without_a_checksum_is_refused() {
        let mut index = Index::default();
        assert!(index.add("P:x\nV:1\n", Repository::Main).is_err());
    }

    #[test]
    fn a_package_checksum_covers_its_second_member() {
        let control = gzip(b"control");
        let apk = [gzip(b"signature"), control.clone(), gzip(b"data")].concat();
        let package = Package {
            name: "p".into(),
            version: "1".into(),
            repository: Repository::Main,
            depends: Vec::new(),
            checksum: format!(
                "Q1{}",
                base64::engine::general_purpose::STANDARD.encode(Sha1::digest(&control))
            ),
        };
        check_package(&package, &apk).expect("matches");
        let mut other = package.clone();
        other.checksum = "Q1AAAAAAAAAAAAAAAAAAAAAAAAAAA=".into();
        assert!(check_package(&other, &apk).is_err());
    }

    #[test]
    fn version_constraints_are_not_part_of_a_name() {
        assert_eq!(base_name("musl>=1.2"), "musl");
        assert_eq!(base_name("so:libz.so.1=1.3"), "so:libz.so.1");
        assert_eq!(base_name("busybox~1.37"), "busybox");
        assert_eq!(base_name("cmd:qemu-img"), "cmd:qemu-img");
    }
}
