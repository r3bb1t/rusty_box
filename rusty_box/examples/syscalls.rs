//! Linux syscall number → name tables for x86-64 and x86 (32-bit).
//!
//! Source: Linux `arch/x86/entry/syscalls/syscall_64.tbl` and `syscall_32.tbl`.
//! Values current as of Linux 6.x. Anything out of range returns `"unknown"`.

/// x86-64 syscall numbers. Indexed array up to the ones we're likely to
/// see during Alpine init; larger numbers fall through `match` to "unknown".
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

/// x86 (32-bit) syscall numbers, used by INT 0x80 / SYSENTER gate from
/// 32-bit user space (early kernel init uses some of these before switching
/// to 64-bit entry points).
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


use std::fmt;
use std::net::{Ipv4Addr, Ipv6Addr};

// ─────────────────────────── Address family ───────────────────────────

/// Address family for socket syscalls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressFamily {
    Unspec,
    Unix,
    Inet,
    Inet6,
    Netlink,
    #[non_exhaustive]
    Other(u64),
}

impl AddressFamily {
    pub fn from_raw(v: u64) -> Self {
        match v {
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
            Self::Unspec => write!(f, "AF_UNSPEC"),
            Self::Unix => write!(f, "AF_UNIX"),
            Self::Inet => write!(f, "AF_INET"),
            Self::Inet6 => write!(f, "AF_INET6"),
            Self::Netlink => write!(f, "AF_NETLINK"),
            Self::Other(v) => write!(f, "AF_UNKNOWN({v})"),
        }
    }
}

// ─────────────────────────── Socket type ───────────────────────────

/// Socket type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SockType {
    Stream,
    Dgram,
    Raw,
    #[non_exhaustive]
    Other(u64),
}

impl SockType {
    pub fn from_raw(v: u64) -> Self {
        match v {
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
            Self::Stream => write!(f, "SOCK_STREAM"),
            Self::Dgram => write!(f, "SOCK_DGRAM"),
            Self::Raw => write!(f, "SOCK_RAW"),
            Self::Other(v) => write!(f, "SOCK_UNKNOWN({v})"),
        }
    }
}

// ─────────────────────────── Decoded sockaddr ───────────────────────────

/// Decoded sockaddr.
#[derive(Debug, Clone)]
pub enum SockAddr {
    Inet { port: u16, addr: Ipv4Addr },
    Inet6 { port: u16, addr: Ipv6Addr },
    Unix { path: String },
    Unknown { family: u16, data: Vec<u8> },
}

impl fmt::Display for SockAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Inet { port, addr } => write!(f, "AF_INET {addr}:{port}"),
            Self::Inet6 { port, addr } => write!(f, "AF_INET6 [{addr}]:{port}"),
            Self::Unix { path } => write!(f, "AF_UNIX {path:?}"),
            Self::Unknown { family, data } => {
                write!(f, "AF_UNKNOWN({family}) [{} bytes]", data.len())
            }
        }
    }
}

/// Decode a `sockaddr` from a raw byte slice.
///
/// The first two bytes are the address family (little-endian on x86).
/// Returns `SockAddr::Unknown` for families we don't handle or if the
/// buffer is too short.
pub fn decode_sockaddr(data: &[u8]) -> SockAddr {
    if data.len() < 2 {
        return SockAddr::Unknown {
            family: 0,
            data: data.to_vec(),
        };
    }
    let family = u16::from_le_bytes([data[0], data[1]]);
    match family {
        // AF_INET — sockaddr_in: family(2) + port(2, big-endian) + addr(4).
        2 => {
            if data.len() < 8 {
                return SockAddr::Unknown {
                    family,
                    data: data.to_vec(),
                };
            }
            let port = u16::from_be_bytes([data[2], data[3]]);
            let addr = Ipv4Addr::new(data[4], data[5], data[6], data[7]);
            SockAddr::Inet { port, addr }
        }
        // AF_INET6 — sockaddr_in6: family(2) + port(2, big-endian) + flowinfo(4) + addr(16).
        10 => {
            if data.len() < 24 {
                return SockAddr::Unknown {
                    family,
                    data: data.to_vec(),
                };
            }
            let port = u16::from_be_bytes([data[2], data[3]]);
            let mut octets = [0u8; 16];
            octets.copy_from_slice(&data[8..24]);
            let addr = Ipv6Addr::from(octets);
            SockAddr::Inet6 { port, addr }
        }
        // AF_UNIX — sockaddr_un: family(2) + path (up to 108 bytes, NUL-terminated).
        1 => {
            let path_bytes = &data[2..];
            let end = path_bytes.iter().position(|&b| b == 0).unwrap_or(path_bytes.len());
            let path = String::from_utf8_lossy(&path_bytes[..end]).into_owned();
            SockAddr::Unix { path }
        }
        _ => SockAddr::Unknown {
            family,
            data: data.to_vec(),
        },
    }
}

// ─────────────────────────── Linux syscall ADT ───────────────────────────

/// A decoded Linux x86-64 syscall with typed arguments.
#[derive(Debug, Clone)]
pub enum LinuxSyscall {
    Socket {
        domain: AddressFamily,
        sock_type: SockType,
        protocol: i32,
    },
    Connect {
        sockfd: i32,
        addr: SockAddr,
        addrlen: u32,
    },
    Dup2 {
        oldfd: i32,
        newfd: i32,
    },
    Execve {
        filename: String,
        argv: Vec<String>,
        envp_addr: u64,
    },
    Read {
        fd: i32,
        buf: u64,
        count: u64,
    },
    Write {
        fd: i32,
        buf: u64,
        count: u64,
    },
    Open {
        filename: String,
        flags: i32,
        mode: u32,
    },
    Close {
        fd: i32,
    },
    Mmap {
        addr: u64,
        length: u64,
        prot: i32,
        flags: i32,
        fd: i32,
        offset: u64,
    },
    Mprotect {
        addr: u64,
        length: u64,
        prot: i32,
    },
    Brk {
        addr: u64,
    },
    Exit {
        status: i32,
    },
    ExitGroup {
        status: i32,
    },
    /// Fallback for unrecognized syscalls.
    #[non_exhaustive]
    Other {
        nr: u64,
        args: [u64; 6],
    },
}

impl fmt::Display for LinuxSyscall {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Socket {
                domain,
                sock_type,
                protocol,
            } => write!(f, "socket({domain}, {sock_type}, {protocol})"),
            Self::Connect {
                sockfd,
                addr,
                addrlen,
            } => write!(f, "connect({sockfd}, {addr}, {addrlen})"),
            Self::Dup2 { oldfd, newfd } => write!(f, "dup2({oldfd}, {newfd})"),
            Self::Execve {
                filename,
                argv,
                envp_addr,
            } => write!(f, "execve({filename:?}, {argv:?}, {envp_addr:#x})"),
            Self::Read { fd, buf, count } => write!(f, "read({fd}, {buf:#x}, {count})"),
            Self::Write { fd, buf, count } => write!(f, "write({fd}, {buf:#x}, {count})"),
            Self::Open {
                filename,
                flags,
                mode,
            } => write!(f, "open({filename:?}, {flags:#x}, {mode:#o})"),
            Self::Close { fd } => write!(f, "close({fd})"),
            Self::Mmap {
                addr,
                length,
                prot,
                flags,
                fd,
                offset,
            } => write!(
                f,
                "mmap({addr:#x}, {length}, {prot:#x}, {flags:#x}, {fd}, {offset})",
            ),
            Self::Mprotect { addr, length, prot } => {
                write!(f, "mprotect({addr:#x}, {length}, {prot:#x})")
            }
            Self::Brk { addr } => write!(f, "brk({addr:#x})"),
            Self::Exit { status } => write!(f, "exit({status})"),
            Self::ExitGroup { status } => write!(f, "exit_group({status})"),
            Self::Other { nr, args } => {
                write!(
                    f,
                    "syscall_{nr}({:#x}, {:#x}, {:#x}, {:#x}, {:#x}, {:#x})",
                    args[0], args[1], args[2], args[3], args[4], args[5],
                )
            }
        }
    }
}