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

/// Decoded x86-64 Linux syscall with typed arguments.
///
/// Each variant carries exactly the fields the kernel ABI defines for that
/// syscall. `Other` is the fallback for anything we don't decode.
#[derive(Debug, Clone)]
pub enum Syscall {
    // ── File I/O ──
    Read { fd: i32, buf: u64, count: u64 },
    Write { fd: i32, buf: u64, count: u64 },
    Open { path: String, flags: u32, mode: u32 },
    Openat { dirfd: i32, path: String, flags: u32, mode: u32 },
    Close { fd: i32 },
    Lseek { fd: i32, offset: i64, whence: u32 },
    Dup2 { oldfd: i32, newfd: i32 },
    Dup3 { oldfd: i32, newfd: i32, flags: u32 },
    Pipe2 { pipefd: u64, flags: u32 },
    Ioctl { fd: i32, request: u64, arg: u64 },
    Fcntl { fd: i32, cmd: u32, arg: u64 },
    Fstat { fd: i32, buf: u64 },
    Stat { path: String, buf: u64 },
    Lstat { path: String, buf: u64 },
    Newfstatat { dirfd: i32, path: String, buf: u64, flags: u32 },
    Getdents64 { fd: i32, dirp: u64, count: u64 },
    Access { path: String, mode: u32 },
    Faccessat { dirfd: i32, path: String, mode: u32, flags: u32 },
    Readlink { path: String, buf: u64, bufsiz: u64 },
    Unlinkat { dirfd: i32, path: String, flags: u32 },
    Renameat2 { olddirfd: i32, oldpath: String, newdirfd: i32, newpath: String, flags: u32 },
    Getcwd { buf: u64, size: u64 },
    Chdir { path: String },
    Mkdirat { dirfd: i32, path: String, mode: u32 },

    // ── Memory management ──
    Mmap { addr: u64, length: u64, prot: u32, flags: u32, fd: i32, offset: u64 },
    Mprotect { addr: u64, length: u64, prot: u32 },
    Munmap { addr: u64, length: u64 },
    Brk { addr: u64 },
    Mremap { old_addr: u64, old_size: u64, new_size: u64, flags: u32, new_addr: u64 },

    // ── Process ──
    Execve { path: String, argv: u64, envp: u64 },
    Clone { flags: u64, stack: u64, ptid: u64, ctid: u64, tls: u64 },
    Fork,
    Vfork,
    Exit { status: i32 },
    ExitGroup { status: i32 },
    Wait4 { pid: i32, status: u64, options: u32, rusage: u64 },
    Kill { pid: i32, sig: i32 },
    Getpid,
    Gettid,
    Getppid,
    Getuid,
    Getgid,
    Geteuid,
    Getegid,
    SetTidAddress { tidptr: u64 },
    Uname { buf: u64 },

    // ── Signals ──
    RtSigaction { signum: i32, act: u64, oldact: u64, sigsetsize: u64 },
    RtSigprocmask { how: u32, set: u64, oldset: u64, sigsetsize: u64 },

    // ── Network ──
    Socket { domain: AddressFamily, sock_type: SockType, protocol: i32 },
    Connect { sockfd: i32, addr: SockAddr, addrlen: u32 },
    Bind { sockfd: i32, addr: SockAddr, addrlen: u32 },
    Listen { sockfd: i32, backlog: i32 },
    Accept4 { sockfd: i32, addr: u64, addrlen: u64, flags: u32 },
    Sendto { sockfd: i32, buf: u64, len: u64, flags: u32, dest_addr: u64, addrlen: u32 },
    Recvfrom { sockfd: i32, buf: u64, len: u64, flags: u32, src_addr: u64, addrlen: u64 },

    // ── Misc ──
    ArchPrctl { code: ArchPrctlCode, addr: u64 },
    Prctl { option: u32, arg2: u64, arg3: u64, arg4: u64, arg5: u64 },
    Futex { uaddr: u64, op: u32, val: u32, timeout: u64, uaddr2: u64, val3: u32 },
    ClockGettime { clk_id: u32, tp: u64 },
    Getrandom { buf: u64, buflen: u64, flags: u32 },
    Prlimit64 { pid: i32, resource: u32, new_rlim: u64, old_rlim: u64 },
    SetRobustList { head: u64, len: u64 },
    Statx { dirfd: i32, path: String, flags: u32, mask: u32, buf: u64 },

    /// Fallback — raw nr + 6 args.
    Other { nr: u64, name: &'static str, args: [u64; 6] },
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
    /// `read_str` reads a NUL-terminated guest string. If it fails or the
    /// pointer is NULL, return a hex placeholder.
    pub fn decode_x86_64(
        nr: u64,
        a: [u64; 6],
        read_str: &mut dyn FnMut(u64) -> String,
    ) -> Self {
        match nr {
            0 => Self::Read { fd: a[0] as i32, buf: a[1], count: a[2] },
            1 => Self::Write { fd: a[0] as i32, buf: a[1], count: a[2] },
            2 => Self::Open { path: read_str(a[0]), flags: a[1] as u32, mode: a[2] as u32 },
            3 => Self::Close { fd: a[0] as i32 },
            4 => Self::Stat { path: read_str(a[0]), buf: a[1] },
            5 => Self::Fstat { fd: a[0] as i32, buf: a[1] },
            6 => Self::Lstat { path: read_str(a[0]), buf: a[1] },
            8 => Self::Lseek { fd: a[0] as i32, offset: a[1] as i64, whence: a[2] as u32 },
            9 => Self::Mmap { addr: a[0], length: a[1], prot: a[2] as u32, flags: a[3] as u32, fd: a[4] as i32, offset: a[5] },
            10 => Self::Mprotect { addr: a[0], length: a[1], prot: a[2] as u32 },
            11 => Self::Munmap { addr: a[0], length: a[1] },
            12 => Self::Brk { addr: a[0] },
            13 => Self::RtSigaction { signum: a[0] as i32, act: a[1], oldact: a[2], sigsetsize: a[3] },
            14 => Self::RtSigprocmask { how: a[0] as u32, set: a[1], oldset: a[2], sigsetsize: a[3] },
            16 => Self::Ioctl { fd: a[0] as i32, request: a[1], arg: a[2] },
            21 => Self::Access { path: read_str(a[0]), mode: a[1] as u32 },
            25 => Self::Mremap { old_addr: a[0], old_size: a[1], new_size: a[2], flags: a[3] as u32, new_addr: a[4] },
            33 => Self::Dup2 { oldfd: a[0] as i32, newfd: a[1] as i32 },
            39 => Self::Getpid,
            41 => Self::Socket { domain: AddressFamily::from_raw(a[0]), sock_type: SockType::from_raw(a[1]), protocol: a[2] as i32 },
            42 => {
                let sa_data = read_sockaddr_from_guest(a[1], a[2] as usize, read_str);
                Self::Connect { sockfd: a[0] as i32, addr: sa_data, addrlen: a[2] as u32 }
            }
            49 => {
                let sa_data = read_sockaddr_from_guest(a[1], a[2] as usize, read_str);
                Self::Bind { sockfd: a[0] as i32, addr: sa_data, addrlen: a[2] as u32 }
            }
            50 => Self::Listen { sockfd: a[0] as i32, backlog: a[1] as i32 },
            56 => Self::Clone { flags: a[0], stack: a[1], ptid: a[2], ctid: a[3], tls: a[4] },
            57 => Self::Fork,
            58 => Self::Vfork,
            59 => Self::Execve { path: read_str(a[0]), argv: a[1], envp: a[2] },
            60 => Self::Exit { status: a[0] as i32 },
            61 => Self::Wait4 { pid: a[0] as i32, status: a[1], options: a[2] as u32, rusage: a[3] },
            62 => Self::Kill { pid: a[0] as i32, sig: a[1] as i32 },
            63 => Self::Uname { buf: a[0] },
            72 => Self::Fcntl { fd: a[0] as i32, cmd: a[1] as u32, arg: a[2] },
            79 => Self::Getcwd { buf: a[0], size: a[1] },
            80 => Self::Chdir { path: read_str(a[0]) },
            89 => Self::Readlink { path: read_str(a[0]), buf: a[1], bufsiz: a[2] },
            102 => Self::Getuid,
            104 => Self::Getgid,
            107 => Self::Geteuid,
            108 => Self::Getegid,
            110 => Self::Getppid,
            157 => Self::Prctl { option: a[0] as u32, arg2: a[1], arg3: a[2], arg4: a[3], arg5: a[4] },
            158 => Self::ArchPrctl { code: ArchPrctlCode::from_raw(a[0]), addr: a[1] },
            186 => Self::Gettid,
            202 => Self::Futex { uaddr: a[0], op: a[1] as u32, val: a[2] as u32, timeout: a[3], uaddr2: a[4], val3: a[5] as u32 },
            217 => Self::Getdents64 { fd: a[0] as i32, dirp: a[1], count: a[2] },
            218 => Self::SetTidAddress { tidptr: a[0] },
            228 => Self::ClockGettime { clk_id: a[0] as u32, tp: a[1] },
            231 => Self::ExitGroup { status: a[0] as i32 },
            257 => Self::Openat { dirfd: a[0] as i32, path: read_str(a[1]), flags: a[2] as u32, mode: a[3] as u32 },
            258 => Self::Mkdirat { dirfd: a[0] as i32, path: read_str(a[1]), mode: a[2] as u32 },
            262 => Self::Newfstatat { dirfd: a[0] as i32, path: read_str(a[1]), buf: a[2], flags: a[3] as u32 },
            263 => Self::Unlinkat { dirfd: a[0] as i32, path: read_str(a[1]), flags: a[2] as u32 },
            267 => Self::Readlink { path: read_str(a[1]), buf: a[2], bufsiz: a[3] },
            269 => Self::Faccessat { dirfd: a[0] as i32, path: read_str(a[1]), mode: a[2] as u32, flags: a[3] as u32 },
            273 => Self::SetRobustList { head: a[0], len: a[1] },
            288 => Self::Accept4 { sockfd: a[0] as i32, addr: a[1], addrlen: a[2], flags: a[3] as u32 },
            292 => Self::Dup3 { oldfd: a[0] as i32, newfd: a[1] as i32, flags: a[2] as u32 },
            293 => Self::Pipe2 { pipefd: a[0], flags: a[1] as u32 },
            302 => Self::Prlimit64 { pid: a[0] as i32, resource: a[1] as u32, new_rlim: a[2], old_rlim: a[3] },
            316 => Self::Renameat2 { olddirfd: a[0] as i32, oldpath: read_str(a[1]), newdirfd: a[2] as i32, newpath: read_str(a[3]), flags: a[4] as u32 },
            318 => Self::Getrandom { buf: a[0], buflen: a[1], flags: a[2] as u32 },
            332 => Self::Statx { dirfd: a[0] as i32, path: read_str(a[1]), flags: a[2] as u32, mask: a[3] as u32, buf: a[4] },
            435 => Self::Clone { flags: a[0], stack: a[1], ptid: a[2], ctid: a[3], tls: a[4] },
            _ => Self::Other { nr, name: name_x86_64(nr as u32), args: a },
        }
    }
}

/// Read a sockaddr struct from guest memory for socket decode.
fn read_sockaddr_from_guest(
    _addr_ptr: u64,
    len: usize,
    _read_str: &mut dyn FnMut(u64) -> String,
) -> SockAddr {
    // We can't easily read arbitrary guest memory from the decode context.
    // Return a placeholder with the pointer info.
    SockAddr::Unknown { family: 0, len }
    // TODO: when decode is called with full emulator access, read guest
    // memory and call decode_sockaddr(data).
}

// ─────────────────────────── Display ───────────────────────────

/// Format a dirfd value: AT_FDCWD (-100) prints symbolically.
fn fmt_dirfd(fd: i32) -> String {
    if fd == -100 { "AT_FDCWD".into() } else { fd.to_string() }
}

impl fmt::Display for Syscall {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // File I/O
            Self::Read { fd, buf, count } =>
                write!(f, "read({fd}, {buf:#x}, {count})"),
            Self::Write { fd, buf, count } =>
                write!(f, "write({fd}, {buf:#x}, {count})"),
            Self::Open { path, flags, mode } =>
                write!(f, "open({path:?}, {flags:#x}, {mode:#o})"),
            Self::Openat { dirfd, path, flags, mode } =>
                write!(f, "openat({}, {path:?}, {flags:#x}, {mode:#o})", fmt_dirfd(*dirfd)),
            Self::Close { fd } =>
                write!(f, "close({fd})"),
            Self::Lseek { fd, offset, whence } => {
                let w = match whence { 0 => "SEEK_SET", 1 => "SEEK_CUR", 2 => "SEEK_END", _ => "?" };
                write!(f, "lseek({fd}, {offset}, {w})")
            }
            Self::Dup2 { oldfd, newfd } =>
                write!(f, "dup2({oldfd}, {newfd})"),
            Self::Dup3 { oldfd, newfd, flags } =>
                write!(f, "dup3({oldfd}, {newfd}, {flags:#x})"),
            Self::Pipe2 { pipefd, flags } =>
                write!(f, "pipe2({pipefd:#x}, {flags:#x})"),
            Self::Ioctl { fd, request, arg } =>
                write!(f, "ioctl({fd}, {request:#x}, {arg:#x})"),
            Self::Fcntl { fd, cmd, arg } =>
                write!(f, "fcntl({fd}, {cmd}, {arg:#x})"),
            Self::Fstat { fd, buf } =>
                write!(f, "fstat({fd}, {buf:#x})"),
            Self::Stat { path, buf } =>
                write!(f, "stat({path:?}, {buf:#x})"),
            Self::Lstat { path, buf } =>
                write!(f, "lstat({path:?}, {buf:#x})"),
            Self::Newfstatat { dirfd, path, buf, flags } =>
                write!(f, "newfstatat({}, {path:?}, {buf:#x}, {flags:#x})", fmt_dirfd(*dirfd)),
            Self::Getdents64 { fd, dirp, count } =>
                write!(f, "getdents64({fd}, {dirp:#x}, {count})"),
            Self::Access { path, mode } =>
                write!(f, "access({path:?}, {mode:#o})"),
            Self::Faccessat { dirfd, path, mode, flags } =>
                write!(f, "faccessat({}, {path:?}, {mode:#o}, {flags:#x})", fmt_dirfd(*dirfd)),
            Self::Readlink { path, buf, bufsiz } =>
                write!(f, "readlink({path:?}, {buf:#x}, {bufsiz})"),
            Self::Unlinkat { dirfd, path, flags } =>
                write!(f, "unlinkat({}, {path:?}, {flags:#x})", fmt_dirfd(*dirfd)),
            Self::Renameat2 { olddirfd, oldpath, newdirfd, newpath, flags } =>
                write!(f, "renameat2({}, {oldpath:?}, {}, {newpath:?}, {flags:#x})", fmt_dirfd(*olddirfd), fmt_dirfd(*newdirfd)),
            Self::Getcwd { buf, size } =>
                write!(f, "getcwd({buf:#x}, {size})"),
            Self::Chdir { path } =>
                write!(f, "chdir({path:?})"),
            Self::Mkdirat { dirfd, path, mode } =>
                write!(f, "mkdirat({}, {path:?}, {mode:#o})", fmt_dirfd(*dirfd)),

            // Memory
            Self::Mmap { addr, length, prot, flags, fd, offset } =>
                write!(f, "mmap({addr:#x}, {length:#x}, {prot:#x}, {flags:#x}, {fd}, {offset:#x})"),
            Self::Mprotect { addr, length, prot } =>
                write!(f, "mprotect({addr:#x}, {length:#x}, {prot:#x})"),
            Self::Munmap { addr, length } =>
                write!(f, "munmap({addr:#x}, {length:#x})"),
            Self::Brk { addr } =>
                write!(f, "brk({addr:#x})"),
            Self::Mremap { old_addr, old_size, new_size, flags, new_addr } =>
                write!(f, "mremap({old_addr:#x}, {old_size:#x}, {new_size:#x}, {flags:#x}, {new_addr:#x})"),

            // Process
            Self::Execve { path, argv, envp } =>
                write!(f, "execve({path:?}, {argv:#x}, {envp:#x})"),
            Self::Clone { flags, stack, ptid, ctid, tls } =>
                write!(f, "clone({flags:#x}, {stack:#x}, {ptid:#x}, {ctid:#x}, {tls:#x})"),
            Self::Fork => f.write_str("fork()"),
            Self::Vfork => f.write_str("vfork()"),
            Self::Exit { status } => write!(f, "exit({status})"),
            Self::ExitGroup { status } => write!(f, "exit_group({status})"),
            Self::Wait4 { pid, status, options, rusage } =>
                write!(f, "wait4({pid}, {status:#x}, {options:#x}, {rusage:#x})"),
            Self::Kill { pid, sig } => write!(f, "kill({pid}, {sig})"),
            Self::Getpid => f.write_str("getpid()"),
            Self::Gettid => f.write_str("gettid()"),
            Self::Getppid => f.write_str("getppid()"),
            Self::Getuid => f.write_str("getuid()"),
            Self::Getgid => f.write_str("getgid()"),
            Self::Geteuid => f.write_str("geteuid()"),
            Self::Getegid => f.write_str("getegid()"),
            Self::SetTidAddress { tidptr } => write!(f, "set_tid_address({tidptr:#x})"),
            Self::Uname { buf } => write!(f, "uname({buf:#x})"),

            // Signals
            Self::RtSigaction { signum, act, oldact, sigsetsize } =>
                write!(f, "rt_sigaction({signum}, {act:#x}, {oldact:#x}, {sigsetsize})"),
            Self::RtSigprocmask { how, set, oldset, sigsetsize } =>
                write!(f, "rt_sigprocmask({how}, {set:#x}, {oldset:#x}, {sigsetsize})"),

            // Network
            Self::Socket { domain, sock_type, protocol } =>
                write!(f, "socket({domain}, {sock_type}, {protocol})"),
            Self::Connect { sockfd, addr, addrlen } =>
                write!(f, "connect({sockfd}, {{{addr}}}, {addrlen})"),
            Self::Bind { sockfd, addr, addrlen } =>
                write!(f, "bind({sockfd}, {{{addr}}}, {addrlen})"),
            Self::Listen { sockfd, backlog } =>
                write!(f, "listen({sockfd}, {backlog})"),
            Self::Accept4 { sockfd, addr, addrlen, flags } =>
                write!(f, "accept4({sockfd}, {addr:#x}, {addrlen:#x}, {flags:#x})"),
            Self::Sendto { sockfd, buf, len, flags, dest_addr, addrlen } =>
                write!(f, "sendto({sockfd}, {buf:#x}, {len}, {flags:#x}, {dest_addr:#x}, {addrlen})"),
            Self::Recvfrom { sockfd, buf, len, flags, src_addr, addrlen } =>
                write!(f, "recvfrom({sockfd}, {buf:#x}, {len}, {flags:#x}, {src_addr:#x}, {addrlen:#x})"),

            // Misc
            Self::ArchPrctl { code, addr } =>
                write!(f, "arch_prctl({code}, {addr:#x})"),
            Self::Prctl { option, arg2, arg3, arg4, arg5 } =>
                write!(f, "prctl({option}, {arg2:#x}, {arg3:#x}, {arg4:#x}, {arg5:#x})"),
            Self::Futex { uaddr, op, val, timeout, uaddr2, val3 } =>
                write!(f, "futex({uaddr:#x}, {op}, {val}, {timeout:#x}, {uaddr2:#x}, {val3})"),
            Self::ClockGettime { clk_id, tp } =>
                write!(f, "clock_gettime({clk_id}, {tp:#x})"),
            Self::Getrandom { buf, buflen, flags } =>
                write!(f, "getrandom({buf:#x}, {buflen}, {flags:#x})"),
            Self::Prlimit64 { pid, resource, new_rlim, old_rlim } =>
                write!(f, "prlimit64({pid}, {resource}, {new_rlim:#x}, {old_rlim:#x})"),
            Self::SetRobustList { head, len } =>
                write!(f, "set_robust_list({head:#x}, {len})"),
            Self::Statx { dirfd, path, flags, mask, buf } =>
                write!(f, "statx({}, {path:?}, {flags:#x}, {mask:#x}, {buf:#x})", fmt_dirfd(*dirfd)),

            // Fallback
            Self::Other { nr: _, name, args } =>
                write!(f, "{name}({:#x}, {:#x}, {:#x}, {:#x}, {:#x}, {:#x})",
                    args[0], args[1], args[2], args[3], args[4], args[5]),
        }
    }
}
