//! Minimal Linux x86-64 syscall name lookup for the shellcode_trace example.
//!
//! Just enough for typical reverse-shell / bindshell / exec-shellcode payloads:
//! socket, connect, bind, listen, accept, dup2, execve, read, write, close, exit.

/// Map an x86-64 syscall number to its kernel name. Returns `"unknown"` for
/// anything not in this abbreviated table.
pub fn name_x86_64(nr: u32) -> &'static str {
    match nr {
        0 => "read",
        1 => "write",
        2 => "open",
        3 => "close",
        9 => "mmap",
        10 => "mprotect",
        11 => "munmap",
        12 => "brk",
        33 => "dup2",
        41 => "socket",
        42 => "connect",
        43 => "accept",
        49 => "bind",
        50 => "listen",
        57 => "fork",
        59 => "execve",
        60 => "exit",
        231 => "exit_group",
        257 => "openat",
        _ => "unknown",
    }
}
