//! Checks against real files and Microsoft's real symbol server (spec §13
//! items 3 and 5, and the Windows 11 `*2` type records). Every test is
//! ignored: ci has neither the files nor the network. The owner runs them:
//!
//!     cargo test --release -p rusty_box_symbols --test live -- --ignored --nocapture
//!
//! with the environment variable each test names set to a real file.

use std::path::PathBuf;

use rusty_box_symbols::cab::extract_single;
use rusty_box_symbols::http::{Flow, HttpClient};
use rusty_box_symbols::identity::{PdbId, TablesIdentity};
use rusty_box_symbols::service::DEFAULT_MAX_DOWNLOAD;
use rusty_box_symbols::tables::{supplement_of, SymbolTables};
use rusty_box_symbols::units::Rva;
use rusty_box_symbols::ureq_http::UreqHttp;

/// The Windows 11 kernel PDB whose RSDS record `identity`'s tests decode.
const KERNEL_URL: &str =
    "https://msdl.microsoft.com/download/symbols/ntkrnlmp.pdb/B661C6FE72991D160FE37C48B8BC54701/ntkrnlmp.pdb";

fn file_from(variable: &str, what: &str) -> Vec<u8> {
    let Some(path) = std::env::var_os(variable) else {
        panic!("set {variable} to {what}");
    };
    let path = PathBuf::from(path);
    std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

/// What the debugger needs from a kernel's PDB: the system-call entry by
/// name and back, and the process structure's layout.
fn assert_kernel_tables(tables: &SymbolTables) {
    println!("identity: {}", tables.identity());
    let entry = tables.address_of("KiSystemCall64").expect("KiSystemCall64 is named");
    assert_eq!(tables.symbolize(entry).map(|hit| hit.name), Some("KiSystemCall64".to_owned()));
    let process = tables.layout("_EPROCESS").expect("_EPROCESS reads").expect("_EPROCESS is defined");
    assert!(process.size > 0x100, "_EPROCESS is {} bytes", process.size);
    let links = tables.member_location("_EPROCESS", &["ActiveProcessLinks", "Flink"]).expect("the path reads");
    println!("_EPROCESS: {} bytes; ActiveProcessLinks.Flink: {links:?}", process.size);
    assert!(links.is_some());
}

/// A Windows 10 or 11 kernel PDB (`ntkrnlmp.pdb`). On Windows 11 its
/// structures are `LF_STRUCTURE2` records, which this reads through.
#[test]
#[ignore = "needs RUSTY_BOX_LIVE_PDB"]
fn a_real_kernel_pdb_names_its_system_call_entry_and_lays_out_its_processes() {
    let bytes = file_from("RUSTY_BOX_LIVE_PDB", "a kernel PDB (ntkrnlmp.pdb)");
    let tables = SymbolTables::parse(&bytes).expect("the PDB parses");
    assert_kernel_tables(&tables);
}

/// A compressed `.pd_` as a symbol server holds it, LZX at a 2 MiB window.
#[test]
#[ignore = "needs RUSTY_BOX_LIVE_PD_"]
fn a_real_cabinet_opens_to_its_pdb() {
    let bytes = file_from("RUSTY_BOX_LIVE_PD_", "a compressed PDB (*.pd_)");
    let extracted = extract_single(&bytes, DEFAULT_MAX_DOWNLOAD).expect("the cabinet opens");
    println!("{}: {:?}, {} bytes", extracted.name, extracted.compression, extracted.bytes.len());
    let tables = SymbolTables::parse(&extracted.bytes).expect("its PDB parses");
    println!("identity: {}", tables.identity());
}

/// Microsoft's server answers with a redirect to blob storage; the adapter
/// follows it, and the PDB that arrives is the build the URL names.
/// `RUSTY_BOX_LIVE_URL` replaces the default kernel URL.
#[test]
#[ignore = "needs the network"]
fn the_symbol_server_serves_a_kernel_pdb_through_its_redirect() {
    let url = std::env::var("RUSTY_BOX_LIVE_URL").unwrap_or_else(|_| KERNEL_URL.to_owned());
    let mut http = UreqHttp::new();
    let response = http
        .get(&url, DEFAULT_MAX_DOWNLOAD, |progress| {
            if progress.done % (16 << 20) < 64 << 10 {
                println!("{} of {:?} bytes", progress.done, progress.total);
            }
            Flow::Continue
        })
        .expect("the server answers");
    assert_eq!(response.status, 200, "{url}");
    let tables = SymbolTables::parse(&response.body).expect("the PDB parses");
    let TablesIdentity::Pdb { guid, age } = tables.identity() else {
        panic!("{url} gave something other than a PDB");
    };
    let key = PdbId { name: "ntkrnlmp.pdb".to_owned(), guid, age }.store_key();
    assert!(url.contains(&format!("/{key}/")), "{url} names another build than {key}");
    assert_kernel_tables(&tables);
}

/// A distribution's `.debug` file, and its dwz supplement when it names one
/// (Fedora's are dwz-processed: `/usr/lib/debug/.dwz/…`).
#[test]
#[ignore = "needs RUSTY_BOX_LIVE_ELF"]
fn a_real_debug_file_reads_with_its_supplement() {
    let bytes = file_from("RUSTY_BOX_LIVE_ELF", "an ELF debug file");
    let link = supplement_of(&bytes).expect("its sections read");
    println!("supplement: {link:?}");
    let supplement = match (&link, std::env::var_os("RUSTY_BOX_LIVE_ELF_SUPPLEMENT")) {
        (Some(_), Some(path)) => Some(std::fs::read(&path).expect("the supplement reads")),
        (Some(link), None) => panic!("the file names {}; set RUSTY_BOX_LIVE_ELF_SUPPLEMENT to it", link.file_name),
        (None, Some(_) | None) => None,
    };
    let tables = SymbolTables::parse_with_supplement(&bytes, supplement.as_deref()).expect("the file parses");
    println!("identity: {}", tables.identity());
    let named = (0..0x10_0000u64).step_by(0x100).filter(|rva| tables.symbolize(Rva(*rva)).is_some()).count();
    println!("{named} of the first 4096 probes at 256-byte steps are named");
    assert!(named > 0);
}
