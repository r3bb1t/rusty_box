#![allow(dead_code)] // Many variants/helpers defined for completeness but not all used yet.

//! Linux syscall number → name tables for x86-64 and x86 (32-bit).
//!
//! Source: Linux `arch/x86/entry/syscalls/syscall_64.tbl` and `syscall_32.tbl`.
//! Values current as of Linux 6.x. Anything out of range returns `"unknown"`.

use std::fmt;
use std::net::{Ipv4Addr, Ipv6Addr};

// ─────────────────────────── Name lookup tables ───────────────────────────

/// x86-64 syscall numbers.
pub fn name_x86_64(nr: u32) -> &'static str {
    match nr {
        0 => "read",
        1 => "write",
        2 => "open",
        3 => "close",
        4 => "stat",
        5 => "fstat",
        6 => "lstat",
        7 => "poll",
        8 => "lseek",
        9 => "mmap",
        10 => "mprotect",
        11 => "munmap",
        12 => "brk",
        13 => "rt_sigaction",
        14 => "rt_sigprocmask",
        15 => "rt_sigreturn",
        16 => "ioctl",
        17 => "pread64",
        18 => "pwrite64",
        19 => "readv",
        20 => "writev",
        21 => "access",
        22 => "pipe",
        23 => "select",
        24 => "sched_yield",
        25 => "mremap",
        26 => "msync",
        27 => "mincore",
        28 => "madvise",
        29 => "shmget",
        30 => "shmat",
        31 => "shmctl",
        32 => "dup",
        33 => "dup2",
        34 => "pause",
        35 => "nanosleep",
        36 => "getitimer",
        37 => "alarm",
        38 => "setitimer",
        39 => "getpid",
        40 => "sendfile",
        41 => "socket",
        42 => "connect",
        43 => "accept",
        44 => "sendto",
        45 => "recvfrom",
        46 => "sendmsg",
        47 => "recvmsg",
        48 => "shutdown",
        49 => "bind",
        50 => "listen",
        51 => "getsockname",
        52 => "getpeername",
        53 => "socketpair",
        54 => "setsockopt",
        55 => "getsockopt",
        56 => "clone",
        57 => "fork",
        58 => "vfork",
        59 => "execve",
        60 => "exit",
        61 => "wait4",
        62 => "kill",
        63 => "uname",
        72 => "fcntl",
        73 => "flock",
        74 => "fsync",
        75 => "fdatasync",
        76 => "truncate",
        77 => "ftruncate",
        78 => "getdents",
        79 => "getcwd",
        80 => "chdir",
        81 => "fchdir",
        82 => "rename",
        83 => "mkdir",
        84 => "rmdir",
        85 => "creat",
        86 => "link",
        87 => "unlink",
        88 => "symlink",
        89 => "readlink",
        90 => "chmod",
        91 => "fchmod",
        92 => "chown",
        93 => "fchown",
        94 => "lchown",
        95 => "umask",
        96 => "gettimeofday",
        97 => "getrlimit",
        98 => "getrusage",
        99 => "sysinfo",
        100 => "times",
        101 => "ptrace",
        102 => "getuid",
        103 => "syslog",
        104 => "getgid",
        105 => "setuid",
        106 => "setgid",
        107 => "geteuid",
        108 => "getegid",
        109 => "setpgid",
        110 => "getppid",
        111 => "getpgrp",
        112 => "setsid",
        131 => "sigaltstack",
        137 => "statfs",
        138 => "fstatfs",
        157 => "prctl",
        158 => "arch_prctl",
        186 => "gettid",
        201 => "time",
        202 => "futex",
        217 => "getdents64",
        218 => "set_tid_address",
        228 => "clock_gettime",
        230 => "clock_nanosleep",
        231 => "exit_group",
        232 => "epoll_wait",
        233 => "epoll_ctl",
        247 => "waitid",
        257 => "openat",
        258 => "mkdirat",
        259 => "mknodat",
        260 => "fchownat",
        261 => "futimesat",
        262 => "newfstatat",
        263 => "unlinkat",
        264 => "renameat",
        265 => "linkat",
        266 => "symlinkat",
        267 => "readlinkat",
        268 => "fchmodat",
        269 => "faccessat",
        270 => "pselect6",
        271 => "ppoll",
        272 => "unshare",
        273 => "set_robust_list",
        274 => "get_robust_list",
        280 => "utimensat",
        281 => "epoll_pwait",
        283 => "timerfd_create",
        284 => "eventfd",
        285 => "fallocate",
        286 => "timerfd_settime",
        287 => "timerfd_gettime",
        288 => "accept4",
        290 => "eventfd2",
        291 => "epoll_create1",
        292 => "dup3",
        293 => "pipe2",
        294 => "inotify_init1",
        295 => "preadv",
        296 => "pwritev",
        297 => "rt_tgsigqueueinfo",
        298 => "perf_event_open",
        299 => "recvmmsg",
        300 => "fanotify_init",
        301 => "fanotify_mark",
        302 => "prlimit64",
        303 => "name_to_handle_at",
        304 => "open_by_handle_at",
        305 => "clock_adjtime",
        306 => "syncfs",
        307 => "sendmmsg",
        308 => "setns",
        309 => "getcpu",
        310 => "process_vm_readv",
        311 => "process_vm_writev",
        312 => "kcmp",
        313 => "finit_module",
        314 => "sched_setattr",
        315 => "sched_getattr",
        316 => "renameat2",
        317 => "seccomp",
        318 => "getrandom",
        319 => "memfd_create",
        320 => "kexec_file_load",
        321 => "bpf",
        322 => "execveat",
        323 => "userfaultfd",
        324 => "membarrier",
        325 => "mlock2",
        326 => "copy_file_range",
        327 => "preadv2",
        328 => "pwritev2",
        329 => "pkey_mprotect",
        330 => "pkey_alloc",
        331 => "pkey_free",
        332 => "statx",
        333 => "io_pgetevents",
        334 => "rseq",
        424 => "pidfd_send_signal",
        425 => "io_uring_setup",
        426 => "io_uring_enter",
        427 => "io_uring_register",
        428 => "open_tree",
        429 => "move_mount",
        430 => "fsopen",
        431 => "fsconfig",
        432 => "fsmount",
        433 => "fspick",
        434 => "pidfd_open",
        435 => "clone3",
        436 => "close_range",
        437 => "openat2",
        438 => "pidfd_getfd",
        439 => "faccessat2",
        440 => "process_madvise",
        441 => "epoll_pwait2",
        442 => "mount_setattr",
        443 => "quotactl_fd",
        444 => "landlock_create_ruleset",
        445 => "landlock_add_rule",
        446 => "landlock_restrict_self",
        447 => "memfd_secret",
        448 => "process_mrelease",
        449 => "futex_waitv",
        450 => "set_mempolicy_home_node",
        451 => "cachestat",
        452 => "fchmodat2",
        453 => "map_shadow_stack",
        454 => "futex_wake",
        455 => "futex_wait",
        456 => "futex_requeue",
        _ => "unknown",
    }
}

/// x86 (32-bit) syscall numbers.
pub fn name_x86_32(nr: u32) -> &'static str {
    match nr {
        1 => "exit",
        2 => "fork",
        3 => "read",
        4 => "write",
        5 => "open",
        6 => "close",
        7 => "waitpid",
        8 => "creat",
        9 => "link",
        10 => "unlink",
        11 => "execve",
        12 => "chdir",
        13 => "time",
        14 => "mknod",
        15 => "chmod",
        19 => "lseek",
        20 => "getpid",
        24 => "getuid",
        33 => "access",
        37 => "kill",
        39 => "mkdir",
        41 => "dup",
        45 => "brk",
        54 => "ioctl",
        64 => "getppid",
        78 => "gettimeofday",
        91 => "munmap",
        102 => "socketcall",
        114 => "wait4",
        120 => "clone",
        122 => "uname",
        125 => "mprotect",
        140 => "_llseek",
        141 => "getdents",
        142 => "_newselect",
        143 => "flock",
        145 => "readv",
        146 => "writev",
        162 => "nanosleep",
        168 => "poll",
        175 => "rt_sigprocmask",
        183 => "getcwd",
        192 => "mmap2",
        195 => "stat64",
        197 => "fstat64",
        221 => "fcntl64",
        224 => "gettid",
        240 => "futex",
        243 => "set_thread_area",
        252 => "exit_group",
        256 => "epoll_wait",
        257 => "epoll_ctl",
        258 => "epoll_create",
        295 => "openat",
        311 => "set_robust_list",
        318 => "getcpu",
        355 => "getrandom",
        _ => "unknown",
    }
}

// ─────────────────────────── Address family ───────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressFamily {
    Unspec,
    Unix,
    Inet,
    Inet6,
    Netlink,
    Other(u16),
}

impl AddressFamily {
    pub fn from_raw(v: u64) -> Self {
        match v as u16 {
            0 => Self::Unspec,
            1 => Self::Unix,
            2 => Self::Inet,
            10 => Self::Inet6,
            16 => Self::Netlink,
            other => Self::Other(other),
        }
    }
}

impl fmt::Display for AddressFamily {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unspec => f.write_str("AF_UNSPEC"),
            Self::Unix => f.write_str("AF_UNIX"),
            Self::Inet => f.write_str("AF_INET"),
            Self::Inet6 => f.write_str("AF_INET6"),
            Self::Netlink => f.write_str("AF_NETLINK"),
            Self::Other(v) => write!(f, "AF_{v}"),
        }
    }
}

// ─────────────────────────── Socket type ───────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SockType {
    Stream,
    Dgram,
    Raw,
    Other(u32),
}

impl SockType {
    pub fn from_raw(v: u64) -> Self {
        // Mask out SOCK_NONBLOCK (0x800) and SOCK_CLOEXEC (0x80000)
        match (v as u32) & 0xF {
            1 => Self::Stream,
            2 => Self::Dgram,
            3 => Self::Raw,
            other => Self::Other(other),
        }
    }
}

impl fmt::Display for SockType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stream => f.write_str("SOCK_STREAM"),
            Self::Dgram => f.write_str("SOCK_DGRAM"),
            Self::Raw => f.write_str("SOCK_RAW"),
            Self::Other(v) => write!(f, "SOCK_{v}"),
        }
    }
}

// ─────────────────────────── Decoded sockaddr ───────────────────────────

#[derive(Debug, Clone)]
pub enum SockAddr {
    Inet { port: u16, addr: Ipv4Addr },
    Inet6 { port: u16, addr: Ipv6Addr },
    Unix { path: String },
    Unknown { family: u16, len: usize },
}

impl fmt::Display for SockAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Inet { port, addr } => write!(f, "{addr}:{port}"),
            Self::Inet6 { port, addr } => write!(f, "[{addr}]:{port}"),
            Self::Unix { path } => write!(f, "{path:?}"),
            Self::Unknown { family, len } => write!(f, "sa_family={family} [{len}B]"),
        }
    }
}

pub fn decode_sockaddr(data: &[u8]) -> SockAddr {
    if data.len() < 2 {
        return SockAddr::Unknown { family: 0, len: data.len() };
    }
    let family = u16::from_le_bytes([data[0], data[1]]);
    match family {
        2 if data.len() >= 8 => {
            let port = u16::from_be_bytes([data[2], data[3]]);
            let addr = Ipv4Addr::new(data[4], data[5], data[6], data[7]);
            SockAddr::Inet { port, addr }
        }
        10 if data.len() >= 24 => {
            let port = u16::from_be_bytes([data[2], data[3]]);
            let mut octets = [0u8; 16];
            octets.copy_from_slice(&data[8..24]);
            SockAddr::Inet6 { port, addr: Ipv6Addr::from(octets) }
        }
        1 => {
            let path_bytes = &data[2..];
            let end = path_bytes.iter().position(|&b| b == 0).unwrap_or(path_bytes.len());
            SockAddr::Unix { path: String::from_utf8_lossy(&path_bytes[..end]).into_owned() }
        }
        _ => SockAddr::Unknown { family, len: data.len() },
    }
}

// ─────────────────────────── Linux syscall ADT ───────────────────────────

/// Decoded x86-64 Linux syscall.
///
/// Typed variants for syscalls whose output is most useful when structured
/// (sockaddr decoding, AT_FDCWD dirfd, typed enum args). Everything else
/// uses `Generic` with a descriptor table that knows which args are strings.
#[derive(Debug, Clone)]
pub enum Syscall {
    // Typed variants where structured fields add value over raw args.
    Socket { domain: AddressFamily, sock_type: SockType, protocol: i32 },
    Connect { sockfd: i32, addr: SockAddr, addrlen: u32 },
    Bind { sockfd: i32, addr: SockAddr, addrlen: u32 },
    ArchPrctl { code: ArchPrctlCode, addr: u64 },
    Fork,
    Vfork,
    Sync,
    SchedYield,
    Getpid,
    Gettid,
    Getppid,
    Getuid,
    Getgid,
    Geteuid,
    Getegid,
    Getpgrp,
    Setsid,
    Pause,
    Munlockall,
    RtSigreturn,
    InotifyInit,

    /// Generic: covers every other syscall via descriptor table lookup.
    /// Arg strings that require guest memory reads are pre-decoded into
    /// `strings` at the position matching their arg index.
    Generic {
        nr: u64,
        name: &'static str,
        args: [u64; 6],
        /// Decoded strings; `strings[i] = Some(s)` when arg i is a String.
        strings: [Option<String>; 6],
        arg_kinds: [ArgKind; 6],
        nargs: u8,
    },
}

/// How to format each syscall argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArgKind {
    /// No argument at this position.
    None,
    /// Integer / pointer — print as hex.
    Hex,
    /// Decimal integer (sizes, counts, small numbers).
    Dec,
    /// File descriptor — print as decimal i32.
    Fd,
    /// Directory fd — AT_FDCWD (-100) prints symbolically.
    Dirfd,
    /// NUL-terminated string, read from guest memory.
    Str,
    /// Octal (modes, permissions).
    Oct,
    /// Signed decimal (status, offsets).
    SDec,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchPrctlCode {
    SetGs,   // 0x1001
    SetFs,   // 0x1002
    GetFs,   // 0x1003
    GetGs,   // 0x1004
    Other(u64),
}

impl ArchPrctlCode {
    fn from_raw(v: u64) -> Self {
        match v {
            0x1001 => Self::SetGs,
            0x1002 => Self::SetFs,
            0x1003 => Self::GetFs,
            0x1004 => Self::GetGs,
            _ => Self::Other(v),
        }
    }
}

impl fmt::Display for ArchPrctlCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SetGs => f.write_str("ARCH_SET_GS"),
            Self::SetFs => f.write_str("ARCH_SET_FS"),
            Self::GetFs => f.write_str("ARCH_GET_FS"),
            Self::GetGs => f.write_str("ARCH_GET_GS"),
            Self::Other(v) => write!(f, "{v:#x}"),
        }
    }
}

// ─────────────────────────── Decode from raw registers ───────────────────────────

impl Syscall {
    /// Decode a syscall from its number and raw argument registers.
    ///
    /// `read_str` reads a NUL-terminated guest string at the given address.
    pub fn decode_x86_64(
        nr: u64,
        a: [u64; 6],
        // Pre-captured strings from syscall-entry snapshot (preferred when valid).
        pre_strings: &[Option<String>; 6],
        // Fallback reader when pre_strings[i] is None (rarely used).
        read_str: &mut dyn FnMut(u64) -> String,
    ) -> Self {
        // Typed variants first for syscalls where structured args add value.
        match nr {
            41 => return Self::Socket {
                domain: AddressFamily::from_raw(a[0]),
                sock_type: SockType::from_raw(a[1]),
                protocol: a[2] as i32,
            },
            42 => return Self::Connect {
                sockfd: a[0] as i32,
                addr: SockAddr::Unknown { family: 0, len: a[2] as usize },
                addrlen: a[2] as u32,
            },
            49 => return Self::Bind {
                sockfd: a[0] as i32,
                addr: SockAddr::Unknown { family: 0, len: a[2] as usize },
                addrlen: a[2] as u32,
            },
            158 => return Self::ArchPrctl {
                code: ArchPrctlCode::from_raw(a[0]),
                addr: a[1],
            },
            24 => return Self::SchedYield,
            34 => return Self::Pause,
            15 => return Self::RtSigreturn,
            39 => return Self::Getpid,
            57 => return Self::Fork,
            58 => return Self::Vfork,
            102 => return Self::Getuid,
            104 => return Self::Getgid,
            107 => return Self::Geteuid,
            108 => return Self::Getegid,
            110 => return Self::Getppid,
            111 => return Self::Getpgrp,
            112 => return Self::Setsid,
            152 => return Self::Munlockall,
            162 => return Self::Sync,
            186 => return Self::Gettid,
            253 => return Self::InotifyInit,
            _ => {}
        }

        // Everything else: descriptor-based decode.
        let (name, kinds, nargs) = syscall_arg_kinds_x86_64(nr);
        let mut strings: [Option<String>; 6] = [None, None, None, None, None, None];
        for (i, k) in kinds.iter().take(nargs).enumerate() {
            if *k == ArgKind::Str {
                strings[i] = pre_strings[i].clone().or_else(|| Some(read_str(a[i])));
            }
        }
        Self::Generic { nr, name, args: a, strings, arg_kinds: kinds, nargs: nargs as u8 }
    }
}

// ────────────────────────── Arg descriptor table ─────────────────────────

/// Returns (name, arg_kinds[6], nargs) for each syscall number.
/// Linux x86-64 syscall table, derived from
/// https://blog.rchapman.org/posts/Linux_System_Call_Table_for_x86_64/
fn syscall_arg_kinds_x86_64(nr: u64) -> (&'static str, [ArgKind; 6], usize) {
    use ArgKind::*;
    let none6 = [None, None, None, None, None, None];
    let (name, kinds, nargs): (&'static str, [ArgKind; 6], usize) = match nr {
        // File I/O
        0 => ("read", [Fd, Hex, Dec, None, None, None], 3),
        1 => ("write", [Fd, Hex, Dec, None, None, None], 3),
        2 => ("open", [Str, Hex, Oct, None, None, None], 3),
        3 => ("close", [Fd, None, None, None, None, None], 1),
        4 => ("stat", [Str, Hex, None, None, None, None], 2),
        5 => ("fstat", [Fd, Hex, None, None, None, None], 2),
        6 => ("lstat", [Str, Hex, None, None, None, None], 2),
        7 => ("poll", [Hex, Dec, SDec, None, None, None], 3),
        8 => ("lseek", [Fd, SDec, Dec, None, None, None], 3),
        9 => ("mmap", [Hex, Hex, Hex, Hex, Fd, Hex], 6),
        10 => ("mprotect", [Hex, Hex, Hex, None, None, None], 3),
        11 => ("munmap", [Hex, Hex, None, None, None, None], 2),
        12 => ("brk", [Hex, None, None, None, None, None], 1),
        13 => ("rt_sigaction", [SDec, Hex, Hex, Dec, None, None], 4),
        14 => ("rt_sigprocmask", [Dec, Hex, Hex, Dec, None, None], 4),
        16 => ("ioctl", [Fd, Hex, Hex, None, None, None], 3),
        17 => ("pread64", [Fd, Hex, Dec, SDec, None, None], 4),
        18 => ("pwrite64", [Fd, Hex, Dec, SDec, None, None], 4),
        19 => ("readv", [Fd, Hex, Dec, None, None, None], 3),
        20 => ("writev", [Fd, Hex, Dec, None, None, None], 3),
        21 => ("access", [Str, Oct, None, None, None, None], 2),
        22 => ("pipe", [Hex, None, None, None, None, None], 1),
        23 => ("select", [Dec, Hex, Hex, Hex, Hex, None], 5),
        25 => ("mremap", [Hex, Hex, Hex, Hex, Hex, None], 5),
        26 => ("msync", [Hex, Hex, Hex, None, None, None], 3),
        27 => ("mincore", [Hex, Hex, Hex, None, None, None], 3),
        28 => ("madvise", [Hex, Hex, SDec, None, None, None], 3),
        32 => ("dup", [Fd, None, None, None, None, None], 1),
        33 => ("dup2", [Fd, Fd, None, None, None, None], 2),
        35 => ("nanosleep", [Hex, Hex, None, None, None, None], 2),
        36 => ("getitimer", [Dec, Hex, None, None, None, None], 2),
        37 => ("alarm", [Dec, None, None, None, None, None], 1),
        38 => ("setitimer", [Dec, Hex, Hex, None, None, None], 3),
        40 => ("sendfile", [Fd, Fd, Hex, Dec, None, None], 4),
        43 => ("accept", [Fd, Hex, Hex, None, None, None], 3),
        44 => ("sendto", [Fd, Hex, Dec, Hex, Hex, Dec], 6),
        45 => ("recvfrom", [Fd, Hex, Dec, Hex, Hex, Hex], 6),
        46 => ("sendmsg", [Fd, Hex, Hex, None, None, None], 3),
        47 => ("recvmsg", [Fd, Hex, Hex, None, None, None], 3),
        48 => ("shutdown", [Fd, SDec, None, None, None, None], 2),
        50 => ("listen", [Fd, SDec, None, None, None, None], 2),
        51 => ("getsockname", [Fd, Hex, Hex, None, None, None], 3),
        52 => ("getpeername", [Fd, Hex, Hex, None, None, None], 3),
        53 => ("socketpair", [Dec, Dec, Dec, Hex, None, None], 4),
        54 => ("setsockopt", [Fd, SDec, SDec, Hex, Dec, None], 5),
        55 => ("getsockopt", [Fd, SDec, SDec, Hex, Hex, None], 5),
        56 => ("clone", [Hex, Hex, Hex, Hex, Hex, None], 5),
        59 => ("execve", [Str, Hex, Hex, None, None, None], 3),
        60 => ("exit", [SDec, None, None, None, None, None], 1),
        61 => ("wait4", [SDec, Hex, Hex, Hex, None, None], 4),
        62 => ("kill", [SDec, SDec, None, None, None, None], 2),
        63 => ("uname", [Hex, None, None, None, None, None], 1),
        72 => ("fcntl", [Fd, Dec, Hex, None, None, None], 3),
        73 => ("flock", [Fd, SDec, None, None, None, None], 2),
        74 => ("fsync", [Fd, None, None, None, None, None], 1),
        75 => ("fdatasync", [Fd, None, None, None, None, None], 1),
        76 => ("truncate", [Str, SDec, None, None, None, None], 2),
        77 => ("ftruncate", [Fd, SDec, None, None, None, None], 2),
        78 => ("getdents", [Fd, Hex, Dec, None, None, None], 3),
        79 => ("getcwd", [Hex, Dec, None, None, None, None], 2),
        80 => ("chdir", [Str, None, None, None, None, None], 1),
        81 => ("fchdir", [Fd, None, None, None, None, None], 1),
        82 => ("rename", [Str, Str, None, None, None, None], 2),
        83 => ("mkdir", [Str, Oct, None, None, None, None], 2),
        84 => ("rmdir", [Str, None, None, None, None, None], 1),
        85 => ("creat", [Str, Oct, None, None, None, None], 2),
        86 => ("link", [Str, Str, None, None, None, None], 2),
        87 => ("unlink", [Str, None, None, None, None, None], 1),
        88 => ("symlink", [Str, Str, None, None, None, None], 2),
        89 => ("readlink", [Str, Hex, Dec, None, None, None], 3),
        90 => ("chmod", [Str, Oct, None, None, None, None], 2),
        91 => ("fchmod", [Fd, Oct, None, None, None, None], 2),
        92 => ("chown", [Str, Dec, Dec, None, None, None], 3),
        93 => ("fchown", [Fd, Dec, Dec, None, None, None], 3),
        94 => ("lchown", [Str, Dec, Dec, None, None, None], 3),
        95 => ("umask", [Oct, None, None, None, None, None], 1),
        96 => ("gettimeofday", [Hex, Hex, None, None, None, None], 2),
        97 => ("getrlimit", [Dec, Hex, None, None, None, None], 2),
        98 => ("getrusage", [SDec, Hex, None, None, None, None], 2),
        99 => ("sysinfo", [Hex, None, None, None, None, None], 1),
        100 => ("times", [Hex, None, None, None, None, None], 1),
        101 => ("ptrace", [SDec, SDec, Hex, Hex, None, None], 4),
        103 => ("syslog", [SDec, Hex, SDec, None, None, None], 3),
        105 => ("setuid", [Dec, None, None, None, None, None], 1),
        106 => ("setgid", [Dec, None, None, None, None, None], 1),
        109 => ("setpgid", [SDec, SDec, None, None, None, None], 2),
        113 => ("setreuid", [Dec, Dec, None, None, None, None], 2),
        114 => ("setregid", [Dec, Dec, None, None, None, None], 2),
        115 => ("getgroups", [SDec, Hex, None, None, None, None], 2),
        116 => ("setgroups", [SDec, Hex, None, None, None, None], 2),
        121 => ("getpgid", [SDec, None, None, None, None, None], 1),
        124 => ("getsid", [SDec, None, None, None, None, None], 1),
        125 => ("capget", [Hex, Hex, None, None, None, None], 2),
        126 => ("capset", [Hex, Hex, None, None, None, None], 2),
        127 => ("rt_sigpending", [Hex, Dec, None, None, None, None], 2),
        130 => ("rt_sigsuspend", [Hex, Dec, None, None, None, None], 2),
        131 => ("sigaltstack", [Hex, Hex, None, None, None, None], 2),
        132 => ("utime", [Str, Hex, None, None, None, None], 2),
        133 => ("mknod", [Str, Oct, Hex, None, None, None], 3),
        137 => ("statfs", [Str, Hex, None, None, None, None], 2),
        138 => ("fstatfs", [Fd, Hex, None, None, None, None], 2),
        155 => ("pivot_root", [Str, Str, None, None, None, None], 2),
        157 => ("prctl", [Dec, Hex, Hex, Hex, Hex, None], 5),
        161 => ("chroot", [Str, None, None, None, None, None], 1),
        163 => ("acct", [Str, None, None, None, None, None], 1),
        165 => ("mount", [Str, Str, Str, Hex, Hex, None], 5),
        166 => ("umount2", [Str, Hex, None, None, None, None], 2),
        167 => ("swapon", [Str, Hex, None, None, None, None], 2),
        168 => ("swapoff", [Str, None, None, None, None, None], 1),
        170 => ("sethostname", [Str, Dec, None, None, None, None], 2),
        171 => ("setdomainname", [Str, Dec, None, None, None, None], 2),
        175 => ("init_module", [Hex, Dec, Str, None, None, None], 3),
        176 => ("delete_module", [Str, Hex, None, None, None, None], 2),
        200 => ("tkill", [SDec, SDec, None, None, None, None], 2),
        201 => ("time", [Hex, None, None, None, None, None], 1),
        202 => ("futex", [Hex, Dec, Dec, Hex, Hex, Dec], 6),
        203 => ("sched_setaffinity", [SDec, Dec, Hex, None, None, None], 3),
        204 => ("sched_getaffinity", [SDec, Dec, Hex, None, None, None], 3),
        217 => ("getdents64", [Fd, Hex, Dec, None, None, None], 3),
        218 => ("set_tid_address", [Hex, None, None, None, None, None], 1),
        221 => ("fadvise64", [Fd, SDec, Dec, SDec, None, None], 4),
        227 => ("clock_settime", [Dec, Hex, None, None, None, None], 2),
        228 => ("clock_gettime", [Dec, Hex, None, None, None, None], 2),
        229 => ("clock_getres", [Dec, Hex, None, None, None, None], 2),
        230 => ("clock_nanosleep", [Dec, Hex, Hex, Hex, None, None], 4),
        231 => ("exit_group", [SDec, None, None, None, None, None], 1),
        232 => ("epoll_wait", [Fd, Hex, SDec, SDec, None, None], 4),
        233 => ("epoll_ctl", [Fd, SDec, Fd, Hex, None, None], 4),
        234 => ("tgkill", [SDec, SDec, SDec, None, None, None], 3),
        235 => ("utimes", [Str, Hex, None, None, None, None], 2),
        247 => ("waitid", [Dec, SDec, Hex, Hex, Hex, None], 5),
        254 => ("inotify_add_watch", [Fd, Str, Hex, None, None, None], 3),
        255 => ("inotify_rm_watch", [Fd, SDec, None, None, None, None], 2),
        257 => ("openat", [Dirfd, Str, Hex, Oct, None, None], 4),
        258 => ("mkdirat", [Dirfd, Str, Oct, None, None, None], 3),
        259 => ("mknodat", [Dirfd, Str, Oct, Hex, None, None], 4),
        260 => ("fchownat", [Dirfd, Str, Dec, Dec, Hex, None], 5),
        261 => ("futimesat", [Dirfd, Str, Hex, None, None, None], 3),
        262 => ("newfstatat", [Dirfd, Str, Hex, Hex, None, None], 4),
        263 => ("unlinkat", [Dirfd, Str, Hex, None, None, None], 3),
        264 => ("renameat", [Dirfd, Str, Dirfd, Str, None, None], 4),
        265 => ("linkat", [Dirfd, Str, Dirfd, Str, Hex, None], 5),
        266 => ("symlinkat", [Str, Dirfd, Str, None, None, None], 3),
        267 => ("readlinkat", [Dirfd, Str, Hex, Dec, None, None], 4),
        268 => ("fchmodat", [Dirfd, Str, Oct, None, None, None], 3),
        269 => ("faccessat", [Dirfd, Str, Oct, None, None, None], 3),
        270 => ("pselect6", [Dec, Hex, Hex, Hex, Hex, Hex], 6),
        271 => ("ppoll", [Hex, Dec, Hex, Hex, Dec, None], 5),
        272 => ("unshare", [Hex, None, None, None, None, None], 1),
        273 => ("set_robust_list", [Hex, Dec, None, None, None, None], 2),
        274 => ("get_robust_list", [SDec, Hex, Hex, None, None, None], 3),
        280 => ("utimensat", [Dirfd, Str, Hex, Hex, None, None], 4),
        281 => ("epoll_pwait", [Fd, Hex, SDec, SDec, Hex, None], 5),
        283 => ("timerfd_create", [Dec, Hex, None, None, None, None], 2),
        284 => ("eventfd", [Dec, None, None, None, None, None], 1),
        285 => ("fallocate", [Fd, Hex, SDec, SDec, None, None], 4),
        286 => ("timerfd_settime", [Fd, Hex, Hex, Hex, None, None], 4),
        287 => ("timerfd_gettime", [Fd, Hex, None, None, None, None], 2),
        288 => ("accept4", [Fd, Hex, Hex, Hex, None, None], 4),
        290 => ("eventfd2", [Dec, Hex, None, None, None, None], 2),
        291 => ("epoll_create1", [Hex, None, None, None, None, None], 1),
        292 => ("dup3", [Fd, Fd, Hex, None, None, None], 3),
        293 => ("pipe2", [Hex, Hex, None, None, None, None], 2),
        294 => ("inotify_init1", [Hex, None, None, None, None, None], 1),
        302 => ("prlimit64", [SDec, Dec, Hex, Hex, None, None], 4),
        303 => ("name_to_handle_at", [Dirfd, Str, Hex, Hex, Hex, None], 5),
        304 => ("open_by_handle_at", [Dirfd, Hex, Hex, None, None, None], 3),
        306 => ("syncfs", [Fd, None, None, None, None, None], 1),
        308 => ("setns", [Fd, SDec, None, None, None, None], 2),
        309 => ("getcpu", [Hex, Hex, Hex, None, None, None], 3),
        316 => ("renameat2", [Dirfd, Str, Dirfd, Str, Hex, None], 5),
        317 => ("seccomp", [Dec, Hex, Hex, None, None, None], 3),
        318 => ("getrandom", [Hex, Dec, Hex, None, None, None], 3),
        319 => ("memfd_create", [Str, Hex, None, None, None, None], 2),
        321 => ("bpf", [SDec, Hex, Dec, None, None, None], 3),
        322 => ("execveat", [Dirfd, Str, Hex, Hex, Hex, None], 5),
        323 => ("userfaultfd", [Hex, None, None, None, None, None], 1),
        324 => ("membarrier", [SDec, SDec, None, None, None, None], 2),
        325 => ("mlock2", [Hex, Hex, Hex, None, None, None], 3),
        326 => ("copy_file_range", [Fd, Hex, Fd, Hex, Dec, Hex], 6),
        329 => ("pkey_mprotect", [Hex, Hex, Hex, SDec, None, None], 4),
        332 => ("statx", [Dirfd, Str, Hex, Hex, Hex, None], 5),
        334 => ("rseq", [Hex, Dec, Hex, Dec, None, None], 4),
        435 => ("clone3", [Hex, Dec, None, None, None, None], 2),
        439 => ("faccessat2", [Dirfd, Str, Oct, Hex, None, None], 4),
        _ => (name_x86_64(nr as u32), none6, 0),
    };
    (name, kinds, nargs)
}

// ────────────────────────── Display ─────────────────────────

/// Format a dirfd value: AT_FDCWD (-100) prints symbolically.
fn fmt_dirfd(fd: i32) -> String {
    if fd == -100 { "AT_FDCWD".into() } else { fd.to_string() }
}

/// Format a single arg based on its descriptor.
fn fmt_arg(f: &mut fmt::Formatter<'_>, kind: ArgKind, raw: u64, decoded_str: Option<&str>) -> fmt::Result {
    match kind {
        ArgKind::None => Ok(()),
        ArgKind::Hex => write!(f, "{raw:#x}"),
        ArgKind::Dec => write!(f, "{raw}"),
        ArgKind::SDec => write!(f, "{}", raw as i64),
        ArgKind::Fd => write!(f, "{}", raw as i32),
        ArgKind::Dirfd => f.write_str(&fmt_dirfd(raw as i32)),
        ArgKind::Oct => write!(f, "0o{:o}", raw as u32),
        ArgKind::Str => {
            if let Some(s) = decoded_str {
                write!(f, "{s:?}")
            } else {
                write!(f, "{raw:#x}")
            }
        }
    }
}

impl fmt::Display for Syscall {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Socket { domain, sock_type, protocol } =>
                write!(f, "socket({domain}, {sock_type}, {protocol})"),
            Self::Connect { sockfd, addr, addrlen } =>
                write!(f, "connect({sockfd}, {{{addr}}}, {addrlen})"),
            Self::Bind { sockfd, addr, addrlen } =>
                write!(f, "bind({sockfd}, {{{addr}}}, {addrlen})"),
            Self::ArchPrctl { code, addr } =>
                write!(f, "arch_prctl({code}, {addr:#x})"),
            Self::Fork => f.write_str("fork()"),
            Self::Vfork => f.write_str("vfork()"),
            Self::Sync => f.write_str("sync()"),
            Self::SchedYield => f.write_str("sched_yield()"),
            Self::Getpid => f.write_str("getpid()"),
            Self::Gettid => f.write_str("gettid()"),
            Self::Getppid => f.write_str("getppid()"),
            Self::Getuid => f.write_str("getuid()"),
            Self::Getgid => f.write_str("getgid()"),
            Self::Geteuid => f.write_str("geteuid()"),
            Self::Getegid => f.write_str("getegid()"),
            Self::Getpgrp => f.write_str("getpgrp()"),
            Self::Setsid => f.write_str("setsid()"),
            Self::Pause => f.write_str("pause()"),
            Self::Munlockall => f.write_str("munlockall()"),
            Self::RtSigreturn => f.write_str("rt_sigreturn()"),
            Self::InotifyInit => f.write_str("inotify_init()"),

            Self::Generic { name, args, strings, arg_kinds, nargs, .. } => {
                write!(f, "{name}(")?;
                for i in 0..(*nargs as usize) {
                    if i > 0 { f.write_str(", ")?; }
                    fmt_arg(f, arg_kinds[i], args[i], strings[i].as_deref())?;
                }
                f.write_str(")")
            }
        }
    }
}
