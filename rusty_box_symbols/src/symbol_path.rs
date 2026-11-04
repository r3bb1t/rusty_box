//! The symbol search paths other tools read from the environment, used as
//! defaults: `_NT_SYMBOL_PATH` (WinDbg's; Microsoft Learn, "Symbol path for
//! Windows debuggers" and "Advanced SymSrv Use") and `DEBUGINFOD_URLS`
//! (elfutils').

use std::path::PathBuf;

/// What a symbol path gives the engine: the downstream store its first
/// `srv*` element names, and every symbol server it names, in order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParsedSymbolPath {
    pub store: Option<PathBuf>,
    pub servers: Vec<String>,
}

fn is_server(token: &str) -> bool {
    let lower = token.to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

/// Reads `_NT_SYMBOL_PATH`, with `_NT_ALT_SYMBOL_PATH` (`alternate`) put
/// first as symsrv puts it. Elements are `;`-separated. `srv*A*B*server`
/// names downstream stores, leftmost first, then the server; `srv**server`
/// leaves the store to the default; `symsrv*symsrv.dll*…` names its DLL
/// first. A `cache*dir` element names a store when no `srv*` element does.
/// Plain folders, which symsrv searches by file name, are not used.
#[must_use]
pub fn parse_nt_symbol_path(alternate: Option<&str>, path: Option<&str>) -> ParsedSymbolPath {
    let mut parsed = ParsedSymbolPath::default();
    let mut cache: Option<PathBuf> = None;
    let joined = [alternate, path].into_iter().flatten().collect::<Vec<&str>>().join(";");
    for element in joined.split(';').map(str::trim).filter(|element| !element.is_empty()) {
        let tokens: Vec<&str> = element.split('*').collect();
        let head = tokens.first().map(|head| head.to_ascii_lowercase()).unwrap_or_default();
        let rest: &[&str] = match head.as_str() {
            "srv" => tokens.get(1..).unwrap_or(&[]),
            // `symsrv*symsrv.dll*store*server`: the DLL is not a store.
            "symsrv" => tokens.get(2..).unwrap_or(&[]),
            "cache" => {
                if cache.is_none() {
                    cache = tokens.get(1).filter(|dir| !dir.is_empty()).map(PathBuf::from);
                }
                continue;
            }
            _ => continue,
        };
        for token in rest {
            if is_server(token) {
                if !parsed.servers.iter().any(|known| known == token) {
                    parsed.servers.push((*token).to_owned());
                }
            } else if !token.is_empty() && parsed.store.is_none() {
                parsed.store = Some(PathBuf::from(token));
            }
        }
    }
    if parsed.store.is_none() {
        parsed.store = cache;
    }
    parsed
}

/// `DEBUGINFOD_URLS`: servers separated by spaces.
#[must_use]
pub fn parse_debuginfod_urls(urls: &str) -> Vec<String> {
    urls.split_whitespace().map(str::to_owned).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MSDL: &str = "https://msdl.microsoft.com/download/symbols";

    #[test]
    fn a_srv_element_names_its_store_and_its_server() {
        let parsed = parse_nt_symbol_path(None, Some(&format!("srv*C:\\Symbols*{MSDL}")));
        assert_eq!(parsed.store, Some(PathBuf::from(r"C:\Symbols")));
        assert_eq!(parsed.servers, [MSDL]);
    }

    #[test]
    fn the_leftmost_store_of_a_cascade_is_the_one_used() {
        let parsed = parse_nt_symbol_path(None, Some(&format!("srv*D:\\near*\\\\share\\far*{MSDL}")));
        assert_eq!(parsed.store, Some(PathBuf::from(r"D:\near")));
    }

    #[test]
    fn an_empty_store_leaves_the_default() {
        let parsed = parse_nt_symbol_path(None, Some(&format!("srv**{MSDL}")));
        assert_eq!(parsed.store, None);
        assert_eq!(parsed.servers, [MSDL]);
    }

    #[test]
    fn the_alternate_path_comes_first_and_folders_are_passed_by() {
        let parsed = parse_nt_symbol_path(
            Some("srv*E:\\alt*https://symbols.example/"),
            Some(&format!("C:\\plain;symsrv*symsrv.dll*C:\\Symbols*{MSDL};cache*C:\\cache")),
        );
        assert_eq!(parsed.store, Some(PathBuf::from(r"E:\alt")));
        assert_eq!(parsed.servers, ["https://symbols.example/", MSDL]);
    }

    #[test]
    fn a_cache_element_is_the_store_when_no_srv_names_one() {
        let parsed = parse_nt_symbol_path(None, Some(&format!("cache*C:\\cache;srv*{MSDL}")));
        assert_eq!(parsed.store, Some(PathBuf::from(r"C:\cache")));
    }

    #[test]
    fn debuginfod_urls_are_space_separated() {
        assert_eq!(
            parse_debuginfod_urls("https://debuginfod.fedoraproject.org/  https://debuginfod.elfutils.org/"),
            ["https://debuginfod.fedoraproject.org/", "https://debuginfod.elfutils.org/"]
        );
        assert!(parse_debuginfod_urls("   ").is_empty());
    }
}
