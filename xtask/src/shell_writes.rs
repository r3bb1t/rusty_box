//! Whether a shell command writes files. In this repository files are
//! written with the Edit and Write tools, whose exact-match check is the
//! safety net (CLAUDE.md, "Edit with the Edit/Write tools"); scripted edits
//! have damaged the tree before. A PreToolUse hook runs the
//! `check-shell-command` subcommand (`cargo run --release --quiet --package
//! xtask -- check-shell-command`) on every Bash and PowerShell call and
//! refuses what this module flags. A false refusal costs a retry with the
//! right tool; a false pass is the bug, so the rules lean towards refusing.

use std::io::Read;

/// Why a command is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// `>` or `>>` into something that is not a null device or a stream.
    Redirect,
    /// A command whose job is writing or replacing files.
    WritingCommand(&'static str),
    /// A script interpreter whose script writes files.
    InterpreterWrites(&'static str),
}

impl Refusal {
    pub fn message(self) -> String {
        let what = match self {
            Refusal::Redirect => "redirects output into a file".to_owned(),
            Refusal::WritingCommand(name) => format!("runs `{name}`, which writes files"),
            Refusal::InterpreterWrites(name) => format!("runs a `{name}` script that writes files"),
        };
        format!(
            "Refused: this command {what}. Write files with the Edit or Write tool \
             (CLAUDE.md, \"Edit with the Edit/Write tools\"); keep the shell for builds, \
             tests and reads. A redirect to /dev/null, $null or NUL, or into another \
             stream (2>&1), is allowed."
        )
    }
}

/// Targets a redirect may name without writing a file.
const NULL_DEVICES: &[&str] = &["/dev/null", "$null", "nul", "/dev/stderr", "/dev/stdout"];

/// Commands that write files, as their first word: POSIX tools, PowerShell's
/// aliases of the writing cmdlets (`sc`, `ac`, `cpi`, `mi`, `rni`, `ren`),
/// and cmd's own `copy` and `move`.
const WRITING_COMMANDS: &[&str] = &[
    "tee", "cp", "mv", "install", "touch", "truncate", "ln", "dd", "sc", "ac", "cpi", "mi", "rni", "ren", "copy",
    "move",
];

/// PowerShell cmdlets and aliases that write files, matched case-insensitively
/// anywhere in the command (a nested `pwsh -Command "…"` included).
const WRITING_CMDLETS: &[&str] = &[
    "Set-Content",
    "Add-Content",
    "Out-File",
    "Copy-Item",
    "Move-Item",
    "Rename-Item",
    "Export-Csv",
    "Export-Clixml",
    "Tee-Object",
];

/// Script interpreters, and the calls in their scripts that write files.
const INTERPRETERS: &[&str] = &["python", "python3", "py", "ruby", "node", "perl"];
const SCRIPT_WRITES: &[&str] = &[
    ".write(",
    "write_text(",
    "write_bytes(",
    "writefile",
    "writefilesync",
    "file.write",
    "shutil.copy",
    "shutil.move",
    "os.rename",
    "os.replace",
    ",'w')",
    ", 'w')",
    ",\"w\")",
    ", \"w\")",
    ",'a')",
    ", 'a')",
    ",'wb')",
    ", 'wb')",
];

/// The command with heredoc bodies cut out, and the bodies on their own: a
/// body is data for the command it feeds, so a `>` in it is no redirect.
struct Split {
    commands: String,
    bodies: String,
}

/// Cuts every `<<WORD … WORD` body out of `text`.
fn split_heredocs(text: &str) -> Split {
    let mut commands = String::new();
    let mut bodies = String::new();
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        commands.push_str(line);
        commands.push('\n');
        let Some(delimiter) = heredoc_delimiter(line) else {
            continue;
        };
        for body_line in lines.by_ref() {
            if body_line.trim() == delimiter {
                break;
            }
            bodies.push_str(body_line);
            bodies.push('\n');
        }
    }
    Split { commands, bodies }
}

/// The delimiter of a heredoc `line` opens (`<<EOF`, `<<'EOF'`, `<<-EOF`),
/// outside quotes; `<<<` is a here-string, not a heredoc.
fn heredoc_delimiter(line: &str) -> Option<String> {
    let at = line.find("<<")?;
    let rest = line.get(at.checked_add(2)?..)?;
    if rest.starts_with('<') {
        return None;
    }
    let word: String = rest
        .trim_start_matches('-')
        .trim_start()
        .chars()
        .take_while(|c| !c.is_whitespace() && *c != ';' && *c != '|' && *c != '&' && *c != ')')
        .filter(|c| *c != '\'' && *c != '"')
        .collect();
    (!word.is_empty()).then_some(word)
}

/// Whether `text`, outside single and double quotes, redirects into a file.
fn redirects_into_a_file(text: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    let mut quote: Option<char> = None;
    let mut at = 0usize;
    while let Some(&c) = chars.get(at) {
        at += 1;
        match quote {
            Some(open) => {
                if c == open {
                    quote = None;
                }
                continue;
            }
            None if c == '\'' || c == '"' => {
                quote = Some(c);
                continue;
            }
            None => {}
        }
        if c != '>' {
            continue;
        }
        // `>>` appends and `>|` overrides noclobber, both into a file; `>&N`
        // and `2>&1` name a stream.
        if chars.get(at) == Some(&'>') || chars.get(at) == Some(&'|') {
            at += 1;
        }
        if chars.get(at) == Some(&'&') {
            continue;
        }
        while chars.get(at).is_some_and(|c| *c == ' ' || *c == '\t') {
            at += 1;
        }
        let target: String = chars
            .iter()
            .skip(at)
            .take_while(|c| !c.is_whitespace() && **c != ';' && **c != '|' && **c != ')')
            .filter(|c| **c != '\'' && **c != '"')
            .collect();
        if target.is_empty() || target.starts_with('&') {
            continue;
        }
        if !NULL_DEVICES.contains(&target.to_ascii_lowercase().as_str()) {
            return true;
        }
    }
    false
}

/// Each simple command's words, split on `;`, `|`, `&` and newlines (quotes
/// kept as part of their word, which is enough to find command names).
fn simple_commands(text: &str) -> Vec<Vec<String>> {
    text.split(['\n', ';', '|', '&'])
        .map(|part| part.split_whitespace().map(str::to_owned).collect::<Vec<String>>())
        .filter(|words| !words.is_empty())
        .collect()
}

fn writing_command(words: &[String]) -> Option<&'static str> {
    // Leading `VAR=value` assignments do not name the command.
    let mut rest = words.iter().skip_while(|word| {
        word.contains('=') && !word.starts_with('-') && !word.starts_with('$')
    });
    let mut name = rest.next()?;
    // `cmd /c …` and `cmd /k …` run the command that follows.
    if name.eq_ignore_ascii_case("cmd") {
        let mut after = rest.clone();
        if after.next().is_some_and(|flag| flag.eq_ignore_ascii_case("/c") || flag.eq_ignore_ascii_case("/k")) {
            if let Some(inner) = after.next() {
                name = inner;
                rest = after;
            }
        }
    }
    let args: Vec<&String> = rest.collect();
    let lower = name.to_ascii_lowercase();
    if let Some(&found) = WRITING_COMMANDS.iter().find(|candidate| **candidate == lower) {
        return Some(found);
    }
    let has = |flag: &str| args.iter().any(|arg| arg.as_str() == flag);
    match lower.as_str() {
        "sed" if args.iter().any(|arg| arg.starts_with("-i") || arg.starts_with("--in-place")) => {
            Some("sed -i")
        }
        "perl"
            if args
                .iter()
                .any(|arg| arg.starts_with('-') && !arg.starts_with("--") && arg.contains('i')) =>
        {
            Some("perl -i")
        }
        "git" if has("apply") => Some("git apply"),
        "patch" => Some("patch"),
        "cargo" if has("fmt") => Some("cargo fmt"),
        "new-item"
            if args.windows(2).any(|pair| {
                pair.first().is_some_and(|flag| flag.eq_ignore_ascii_case("-itemtype"))
                    && pair.get(1).is_some_and(|kind| kind.eq_ignore_ascii_case("file"))
            }) || args.iter().any(|arg| arg.eq_ignore_ascii_case("-value")) =>
        {
            Some("New-Item -ItemType File")
        }
        _ => None,
    }
}

/// Whether `word` stands in `text` as a word of its own: no letter, digit or
/// `-` touches it on either side (`Remove-Item` does not hold `Move-Item`).
fn contains_word(text: &str, word: &str) -> bool {
    let joins = |c: char| c.is_ascii_alphanumeric() || c == '-';
    text.match_indices(word).any(|(at, found)| {
        let before = text.get(..at).and_then(|head| head.chars().next_back());
        let after = text.get(at + found.len()..).and_then(|tail| tail.chars().next());
        !before.is_some_and(joins) && !after.is_some_and(joins)
    })
}

/// Why `command` must not run through the shell, or `None` when it may.
pub fn refusal(command: &str) -> Option<Refusal> {
    let split = split_heredocs(command);
    let lower = command.to_ascii_lowercase();
    if let Some(&cmdlet) = WRITING_CMDLETS
        .iter()
        .find(|cmdlet| contains_word(&lower, &cmdlet.to_ascii_lowercase()))
    {
        return Some(Refusal::WritingCommand(cmdlet));
    }
    if lower.contains("[io.file]::write") || lower.contains("[system.io.file]::write") {
        return Some(Refusal::WritingCommand("[IO.File]::Write"));
    }
    if redirects_into_a_file(&split.commands) {
        return Some(Refusal::Redirect);
    }
    let commands = simple_commands(&split.commands);
    for words in &commands {
        if let Some(name) = writing_command(words) {
            return Some(Refusal::WritingCommand(name));
        }
    }
    let script = format!("{}\n{}", split.commands, split.bodies).to_ascii_lowercase();
    let writes = SCRIPT_WRITES.iter().any(|call| script.contains(call));
    if writes {
        for words in &commands {
            let Some(first) = words.first() else {
                continue;
            };
            let first = first.to_ascii_lowercase();
            if let Some(&interpreter) = INTERPRETERS.iter().find(|name| **name == first) {
                return Some(Refusal::InterpreterWrites(interpreter));
            }
        }
    }
    None
}

/// What the hook answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Allow,
    Refuse(Refusal),
}

/// The verdict on one Claude Code PreToolUse event: Bash and PowerShell
/// commands are classified; every other tool passes.
pub fn verdict_of(event: &str) -> Result<Verdict, String> {
    let value: serde_json::Value =
        serde_json::from_str(event).map_err(|error| format!("the hook event is not JSON: {error}"))?;
    let tool = value.get("tool_name").and_then(serde_json::Value::as_str).unwrap_or("");
    if tool != "Bash" && tool != "PowerShell" {
        return Ok(Verdict::Allow);
    }
    let command = value
        .get("tool_input")
        .and_then(|input| input.get("command"))
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| format!("the {tool} event has no command"))?;
    Ok(match refusal(command) {
        Some(reason) => Verdict::Refuse(reason),
        None => Verdict::Allow,
    })
}

/// `cargo xtask check-shell-command`: reads the event on stdin; a refusal
/// prints its reason on stderr and exits with 2, which Claude Code takes as
/// "block this call and show the reason".
pub fn execute_check_shell_command() -> Result<(), String> {
    let mut event = String::new();
    std::io::stdin()
        .read_to_string(&mut event)
        .map_err(|error| format!("the hook event did not read: {error}"))?;
    match verdict_of(&event)? {
        Verdict::Allow => Ok(()),
        Verdict::Refuse(reason) => {
            eprintln!("{}", reason.message());
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Commands the hook must let through: reads, searches, builds, and
    /// redirects into a null device or another stream.
    #[test]
    fn reads_and_builds_pass() {
        for command in [
            "cargo test --release -p rusty_box_symbols",
            "git diff --stat",
            "grep -n \"a > b\" src/lib.rs",
            "ls ~/.cargo/registry/src/*/ 2>/dev/null | head",
            "cargo build --release 2>&1 | tail -5",
            "Get-ChildItem C:\\ 2>$null",
            "rustup component list 2>&1 >$null",
            "awk '$1>3800' file.txt",
            "sed -n 10,20p src/lib.rs",
            "echo done > NUL",
            "cargo test 1>&2",
            "git log --format='%h %s' -3",
            "New-Item -ItemType Directory -Force out",
            // `Remove-Item` holds `Move-Item`; a cmdlet is matched as a word.
            "Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue",
            "cat <<'EOF' | python -I -c \"import sys; print(sys.stdin.read().count('>'))\"\na > b\nEOF",
        ] {
            assert_eq!(refusal(command), None, "{command}");
        }
    }

    /// A command and the refusal it must get.
    struct Case {
        command: &'static str,
        refusal: Refusal,
    }

    /// Commands that write files, whichever shell spells them.
    #[test]
    fn writes_are_refused() {
        let cases = [
            Case { command: "echo hi > notes.md", refusal: Refusal::Redirect },
            Case { command: "cargo test >> log.txt", refusal: Refusal::Redirect },
            Case { command: "cat > src/a.rs <<'EOF'\nfn main() {}\nEOF", refusal: Refusal::Redirect },
            Case { command: "echo x | tee out.txt", refusal: Refusal::WritingCommand("tee") },
            Case { command: "sed -i 's/a/b/' src/lib.rs", refusal: Refusal::WritingCommand("sed -i") },
            Case { command: "perl -pi -e 's/a/b/' x.rs", refusal: Refusal::WritingCommand("perl -i") },
            Case { command: "cp a.rs b.rs", refusal: Refusal::WritingCommand("cp") },
            Case { command: "mv a.rs b.rs", refusal: Refusal::WritingCommand("mv") },
            Case { command: "touch src/lib.rs", refusal: Refusal::WritingCommand("touch") },
            Case { command: "git apply fix.patch", refusal: Refusal::WritingCommand("git apply") },
            Case { command: "cargo fmt", refusal: Refusal::WritingCommand("cargo fmt") },
            Case {
                command: "Set-Content -Path a.txt -Value hi",
                refusal: Refusal::WritingCommand("Set-Content"),
            },
            Case { command: "'x' | Out-File b.txt", refusal: Refusal::WritingCommand("Out-File") },
            Case { command: "Copy-Item a b", refusal: Refusal::WritingCommand("Copy-Item") },
            Case {
                command: "New-Item -ItemType File x.rs",
                refusal: Refusal::WritingCommand("New-Item -ItemType File"),
            },
            Case {
                command: "[IO.File]::WriteAllText('a', 'b')",
                refusal: Refusal::WritingCommand("[IO.File]::Write"),
            },
            Case {
                command: "python -c \"open('x.rs','w').write('fn main(){}')\"",
                refusal: Refusal::InterpreterWrites("python"),
            },
            Case {
                command: "python3 - <<'EOF'\nfrom pathlib import Path\nPath('a').write_text('b')\nEOF",
                refusal: Refusal::InterpreterWrites("python3"),
            },
            // `>|` writes the file even with noclobber set.
            Case { command: "echo x >| notes.md", refusal: Refusal::Redirect },
            Case { command: "Get-Date | Tee-Object out.txt", refusal: Refusal::WritingCommand("Tee-Object") },
            // PowerShell's aliases, and cmd's own copy and move.
            Case { command: "sc a.txt hi", refusal: Refusal::WritingCommand("sc") },
            Case { command: "'x' | ac a.txt", refusal: Refusal::WritingCommand("ac") },
            Case { command: "cpi a.rs b.rs", refusal: Refusal::WritingCommand("cpi") },
            Case { command: "mi a.rs b.rs", refusal: Refusal::WritingCommand("mi") },
            Case { command: "rni a.rs b.rs", refusal: Refusal::WritingCommand("rni") },
            Case { command: "ren a.rs b.rs", refusal: Refusal::WritingCommand("ren") },
            Case { command: "copy a.rs b.rs", refusal: Refusal::WritingCommand("copy") },
            Case { command: "cmd /c move a.rs b.rs", refusal: Refusal::WritingCommand("move") },
        ];
        for case in cases {
            assert_eq!(refusal(case.command), Some(case.refusal), "{}", case.command);
        }
    }

    #[test]
    fn a_hook_event_is_read_from_its_json() {
        let event = r#"{"tool_name":"Bash","tool_input":{"command":"echo a > b"}}"#;
        assert_eq!(verdict_of(event), Ok(Verdict::Refuse(Refusal::Redirect)));
        let other = r#"{"tool_name":"Read","tool_input":{"file_path":"a"}}"#;
        assert_eq!(verdict_of(other), Ok(Verdict::Allow));
        let pwsh = r#"{"tool_name":"PowerShell","tool_input":{"command":"Get-Date"}}"#;
        assert_eq!(verdict_of(pwsh), Ok(Verdict::Allow));
        assert!(verdict_of("not json").is_err());
    }
}
