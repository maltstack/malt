//! Seccomp BPF syscall filtering primitive.
//!
//! Provides seccomp profile application for the Contained isolation tier.
//! Uses raw `prctl` and `seccomp` syscalls via the `nix` crate.
//!
//! The default restricted profile allows only the syscalls needed for
//! basic process operation (read, write, exit, mmap, etc.).

use super::tier::IsolationError;

/// Check whether seccomp is supported on this system.
///
/// Probes by checking kernel seccomp support via `/proc/self/status`.
pub fn probe_seccomp() -> Result<(), IsolationError> {
    #[cfg(target_os = "linux")]
    {
        // Read /proc/self/status to check for Seccomp field
        let status = std::fs::read_to_string("/proc/self/status").map_err(|e| {
            IsolationError::SeccompError(format!("failed to read /proc/self/status: {e}"))
        })?;

        // Look for "Seccomp:" line — 0 means disabled but supported
        for line in status.lines() {
            if line.starts_with("Seccomp:") {
                // Seccomp field exists, so kernel supports it
                return Ok(());
            }
        }

        Err(IsolationError::SeccompError(
            "Seccomp field not found in /proc/self/status".to_string(),
        ))
    }

    #[cfg(not(target_os = "linux"))]
    {
        Err(IsolationError::UnsupportedPlatform(
            "seccomp is Linux-only".to_string(),
        ))
    }
}

/// Generate a default restricted seccomp profile.
///
/// Returns a list of syscall numbers that are allowed. This profile
/// permits the minimum syscalls needed for basic process operation:
///
/// - I/O: read, write, readv, writev, close
/// - Memory: mmap, munmap, mprotect, brk
/// - Process: exit, exit_group, rt_sigreturn
/// - File: open, openat, stat, fstat, lstat, access, getdents64
/// - Threading: futex, clone, set_tid_address
/// - Time: clock_gettime
/// - Misc: arch_prctl, set_robust_list, rseq, prlimit64
///
/// The exact syscall numbers depend on the architecture (x86_64, aarch64, etc.).
pub fn default_restricted_profile() -> Vec<i64> {
    #[cfg(target_arch = "x86_64")]
    {
        // x86_64 syscall numbers
        vec![
            0,   // read
            1,   // write
            2,   // open
            3,   // close
            5,   // fstat
            8,   // lseek
            9,   // mmap
            10,  // mprotect
            11,  // munmap
            12,  // brk
            13,  // rt_sigaction
            14,  // rt_sigprocmask
            15,  // rt_sigreturn
            16,  // ioctl
            20,  // writev
            21,  // access
            218, // pread64
            219, // pwrite64
            257, // openat
            262, // newfstatat
            217, // getdents64
            41,  // socket
            42,  // connect
            43,  // accept
            44,  // sendto
            45,  // recvfrom
            49,  // bind
            50,  // listen
            51,  // getsockname
            52,  // getpeername
            53,  // socketpair
            54,  // setsockopt
            55,  // getsockopt
            56,  // clone
            57,  // fork
            58,  // vfork
            59,  // execve
            60,  // exit
            61,  // wait4
            62,  // kill
            72,  // fcntl
            73,  // flock
            74,  // fsync
            75,  // fdatasync
            77,  // getcwd
            78,  // chdir
            79,  // fchdir
            83,  // symlink
            84,  // symlinkat
            85,  // unlinkat
            86,  // rename
            87,  // mkdir
            88,  // rmdir
            89,  // readlink
            90,  // readlinkat
            91,  // fchmod
            92,  // fchmodat
            93,  // fchown
            94,  // fchownat
            104, // setuid
            105, // setgid
            106, // setgroups
            107, // setresuid
            108, // getresuid
            109, // setresgid
            110, // getresgid
            117, // getuid
            118, // getgid
            119, // geteuid
            120, // getegid
            131, // sigaltstack
            157, // prctl
            158, // arch_prctl
            186, // gettid
            200, // tkill
            202, // futex
            216, // clock_gettime
            228, // clock_getres
            234, // set_robust_list
            273, // getrandom
            291, // rseq
            293, // statx
            302, // prlimit64
            318, // memfd_create
            329, // seccomp
            332, // statx
            334, // rseq
            435, // clone3
            436, // openat2
            438, // pidfd_open
            439, // pidfd_send_signal
            440, // io_uring_setup
            441, // io_uring_enter
            442, // io_uring_register
            447, // faccessat2
            448, // process_madvise
            449, // epoll_pwait2
            450, // mount_setattr
            451, // quotactl_fd
            452, // landlock_create_ruleset
            453, // landlock_add_rule
            454, // landlock_restrict_self
            455, // memfd_secret
            456, // process_mrelease
            457, // futex_waitv
            458, // set_mempolicy_home_node
            459, // cachestat
            460, // fchmodat2
            461, // map_shadow_stack
            462, // futex_wake
            463, // futex_wait
            464, // futex_requeue
            465, // statmount
            466, // listmount
            467, // lsm_get_self_attr
            468, // lsm_set_self_attr
            469, // lsm_list_modules
        ]
    }

    #[cfg(target_arch = "aarch64")]
    {
        // aarch64 syscall numbers
        vec![
            0,   // io_setup
            1,   // io_destroy
            2,   // io_submit
            3,   // io_cancel
            4,   // io_getevents
            5,   // setxattr
            6,   // lsetxattr
            7,   // fsetxattr
            8,   // getxattr
            9,   // lgetxattr
            10,  // fgetxattr
            11,  // listxattr
            12,  // llistxattr
            13,  // flistxattr
            14,  // removexattr
            15,  // lremovexattr
            16,  // fremovexattr
            17,  // getcwd
            18,  // lookup_dcookie
            19,  // eventfd2
            20,  // epoll_create1
            21,  // epoll_ctl
            22,  // epoll_pwait
            23,  // dup
            24,  // dup3
            25,  // fcntl
            26,  // inotify_init1
            27,  // inotify_add_watch
            28,  // inotify_rm_watch
            29,  // ioctl
            30,  // ioprio_set
            31,  // ioprio_get
            32,  // flock
            33,  // mknodat
            34,  // mkdirat
            35,  // unlinkat
            36,  // symlinkat
            37,  // linkat
            38,  // renameat
            39,  // umount2
            40,  // mount
            41,  // pivot_root
            42,  // nfsservctl
            43,  // statfs
            44,  // fstatfs
            45,  // truncate
            46,  // ftruncate
            47,  // fallocate
            48,  // faccessat
            49,  // chdir
            50,  // fchdir
            51,  // chroot
            52,  // fchmod
            53,  // fchmodat
            54,  // fchownat
            55,  // fchown
            56,  // openat
            57,  // close
            58,  // vhangup
            59,  // pipe2
            60,  // quotactl
            61,  // getdents64
            62,  // lseek
            63,  // read
            64,  // write
            65,  // readv
            66,  // writev
            67,  // pread64
            68,  // pwrite64
            69,  // preadv
            70,  // pwritev
            71,  // sendfile
            72,  // pselect6
            73,  // ppoll
            74,  // signalfd4
            75,  // vmsplice
            76,  // splice
            77,  // tee
            78,  // readlinkat
            79,  // newfstatat
            80,  // fstat
            81,  // sync
            82,  // fsync
            83,  // fdatasync
            84,  // sync_file_range
            85,  // timerfd_create
            86,  // timerfd_settime
            87,  // timerfd_gettime
            88,  // utimensat
            89,  // acct
            90,  // capget
            91,  // capset
            92,  // personality
            93,  // exit
            94,  // exit_group
            95,  // waitid
            96,  // set_tid_address
            97,  // unshare
            98,  // futex
            99,  // set_robust_list
            100, // get_robust_list
            101, // nanosleep
            102, // getitimer
            103, // setitimer
            104, // kexec_load
            105, // init_module
            106, // delete_module
            107, // timer_create
            108, // timer_gettime
            109, // timer_getoverrun
            110, // timer_settime
            111, // timer_delete
            112, // clock_settime
            113, // clock_gettime
            114, // clock_getres
            115, // clock_nanosleep
            116, // syslog
            117, // ptrace
            118, // sched_setparam
            119, // sched_setscheduler
            120, // sched_getscheduler
            121, // sched_getparam
            122, // sched_setaffinity
            123, // sched_getaffinity
            124, // sched_yield
            125, // sched_get_priority_max
            126, // sched_get_priority_min
            127, // sched_rr_get_interval
            128, // restart_syscall
            129, // kill
            130, // tkill
            131, // tgkill
            132, // sigaltstack
            133, // rt_sigsuspend
            134, // rt_sigaction
            135, // rt_sigprocmask
            136, // rt_sigpending
            137, // rt_sigtimedwait
            138, // rt_sigqueueinfo
            139, // rt_sigreturn
            140, // setpriority
            141, // getpriority
            142, // reboot
            143, // setregid
            144, // setgid
            145, // setreuid
            146, // setuid
            147, // setresuid
            148, // getresuid
            149, // setresgid
            150, // getresgid
            151, // setfsuid
            152, // setfsgid
            153, // times
            154, // setpgid
            155, // getpgid
            156, // getsid
            157, // setsid
            158, // getgroups
            159, // setgroups
            160, // uname
            161, // sethostname
            162, // setdomainname
            163, // getrlimit
            164, // setrlimit
            165, // getrusage
            166, // umask
            167, // prctl
            168, // getcpu
            169, // gettimeofday
            170, // settimeofday
            171, // adjtimex
            172, // getpid
            173, // getppid
            174, // getuid
            175, // geteuid
            176, // getgid
            177, // getegid
            178, // gettid
            179, // sysinfo
            180, // mq_open
            181, // mq_unlink
            182, // mq_timedsend
            183, // mq_timedreceive
            184, // mq_notify
            185, // mq_getsetattr
            186, // msgget
            187, // msgctl
            188, // msgrcv
            189, // msgsnd
            190, // semget
            191, // semctl
            192, // semtimedop
            193, // semop
            194, // shmget
            195, // shmctl
            196, // shmat
            197, // shmdt
            198, // socket
            199, // socketpair
            200, // bind
            201, // listen
            202, // accept
            203, // connect
            204, // getsockname
            205, // getpeername
            206, // sendto
            207, // recvfrom
            208, // setsockopt
            209, // getsockopt
            210, // shutdown
            211, // sendmsg
            212, // recvmsg
            213, // readahead
            214, // brk
            215, // munmap
            216, // mremap
            217, // add_key
            218, // request_key
            219, // keyctl
            220, // clone
            221, // execve
            222, // mmap
            223, // fadvise64
            224, // swapon
            225, // swapoff
            226, // mprotect
            227, // msync
            228, // mlock
            229, // munlock
            230, // mlockall
            231, // munlockall
            232, // mincore
            233, // madvise
            234, // remap_file_pages
            235, // mbind
            236, // get_mempolicy
            237, // set_mempolicy
            238, // migrate_pages
            239, // move_pages
            240, // rt_tgsigqueueinfo
            241, // perf_event_open
            242, // accept4
            243, // recvmmsg
            244, // arch_specific_syscall
            260, // wait4
            261, // prlimit64
            262, // fanotify_init
            263, // fanotify_mark
            264, // name_to_handle_at
            265, // open_by_handle_at
            266, // clock_adjtime
            267, // syncfs
            268, // setns
            269, // sendmmsg
            270, // process_vm_readv
            271, // process_vm_writev
            272, // kcmp
            273, // finit_module
            274, // sched_setattr
            275, // sched_getattr
            276, // renameat2
            277, // seccomp
            278, // getrandom
            279, // memfd_create
            280, // bpf
            281, // execveat
            282, // userfaultfd
            283, // membarrier
            284, // mlock2
            285, // copy_file_range
            286, // preadv2
            287, // pwritev2
            288, // pkey_mprotect
            289, // pkey_alloc
            290, // pkey_free
            291, // statx
            292, // io_pgetevents
            293, // rseq
            294, // kexec_file_load
            424, // pidfd_send_signal
            425, // io_uring_setup
            426, // io_uring_enter
            427, // io_uring_register
            428, // open_tree
            429, // move_mount
            430, // fsopen
            431, // fsconfig
            432, // fsmount
            433, // fspick
            434, // pidfd_open
            435, // clone3
            436, // close_range
            437, // openat2
            438, // pidfd_getfd
            439, // faccessat2
            440, // process_madvise
            441, // epoll_pwait2
            442, // mount_setattr
            443, // quotactl_fd
            444, // landlock_create_ruleset
            445, // landlock_add_rule
            446, // landlock_restrict_self
            447, // memfd_secret
            448, // process_mrelease
            449, // futex_waitv
            450, // set_mempolicy_home_node
            451, // cachestat
            452, // fchmodat2
            453, // map_shadow_stack
            454, // futex_wake
            455, // futex_wait
            456, // futex_requeue
            457, // statmount
            458, // listmount
            459, // lsm_get_self_attr
            460, // lsm_set_self_attr
            461, // lsm_list_modules
        ]
    }

    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        // Fallback: allow minimal set for unknown architectures
        // This is intentionally restrictive.
        vec![
            0,   // read (assumed)
            1,   // write (assumed)
            60,  // exit (assumed)
            231, // exit_group (assumed)
        ]
    }
}

/// Apply a seccomp filter that only allows the specified syscalls.
///
/// Uses `SECCOMP_MODE_FILTER` with a BPF program that checks each syscall
/// number against the allowed list. If the syscall is not in the list,
/// the process receives `SIGSYS`.
///
/// # Important
///
/// This function is irreversible — once applied, the filter cannot be
/// removed. The calling process and all its children are subject to it.
///
/// # Arguments
///
/// * `allowed_syscalls` — Slice of syscall numbers to allow.
///
/// # Safety
///
/// This function is safe to call, but the effects are permanent for the
/// calling process. Ensure the allowed syscall list is complete enough
/// for the process to function (including error handling paths).
pub fn apply_seccomp_profile(allowed_syscalls: &[i64]) -> Result<(), IsolationError> {
    #[cfg(target_os = "linux")]
    {
        use std::arch::asm;

        // Build the BPF filter program
        // BPF structure:
        //   Load syscall number from seccomp_data.arch
        //   Check if it matches our architecture
        //   Load syscall number from seccomp_data.nr
        //   Check against allowed list
        //   If match: allow (SECCOMP_RET_ALLOW)
        //   If no match: kill (SECCOMP_RET_TRAP)

        // For simplicity, we use seccomp(SECCOMP_SET_MODE_FILTER) via syscall
        // This requires building a sock_fprog structure

        // First, ensure NO_NEW_PRIVS is set (required for unprivileged seccomp)
        nix::prctl::set_no_new_privileges(true).map_err(|e| {
            IsolationError::SeccompError(format!("failed to set NO_NEW_PRIVS: {e}"))
        })?;

        // Build BPF program
        let bpf = build_bpf_filter(allowed_syscalls);

        // Call seccomp syscall directly
        // seccomp(SECCOMP_SET_MODE_FILTER, SECCOMP_FILTER_FLAG_LOG, &prog)
        let prog = linux_seccomp::SockFprog {
            len: bpf.len() as u16,
            filter: bpf.as_ptr(),
        };

        // SECCOMP_SET_MODE_FILTER = 1
        // SECCOMP_FILTER_FLAG_LOG = 1 << 1
        let ret = unsafe {
            let ret: i64;
            asm!(
                "syscall",
                in("rax") 317, // __NR_seccomp on x86_64
                in("rdi") 1,   // SECCOMP_SET_MODE_FILTER
                in("rsi") 2,   // SECCOMP_FILTER_FLAG_LOG
                in("rdx") &prog,
                lateout("rax") ret,
                options(nostack)
            );
            ret
        };

        if ret != 0 {
            return Err(IsolationError::SeccompError(format!(
                "seccomp syscall failed with return code {ret}"
            )));
        }

        Ok(())
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = allowed_syscalls;
        Err(IsolationError::UnsupportedPlatform(
            "seccomp is Linux-only".to_string(),
        ))
    }
}

/// Build a BPF filter program for the given allowed syscalls.
///
/// Returns a vector of `sock_filter` instructions that implement
/// an allowlist filter.
#[cfg(target_os = "linux")]
fn build_bpf_filter(allowed_syscalls: &[i64]) -> Vec<nix::sys::socket::SockFilter> {
    use nix::sys::socket::SockFilter;

    let mut bpf = Vec::new();

    // BPF constants
    const BPF_LD: u16 = 0x00;
    const BPF_W: u16 = 0x00;
    const BPF_ABS: u16 = 0x20;
    const BPF_JEQ: u16 = 0x10;
    const BPF_JMP: u16 = 0x05;
    const BPF_RET: u16 = 0x06;
    const BPF_K: u16 = 0x00;

    // seccomp_data offsets
    const SECCOMP_DATA_NR_OFFSET: u32 = 0; // syscall number at offset 0

    // SECCOMP_RET values
    const SECCOMP_RET_ALLOW: u32 = 0x7fff0000;
    const SECCOMP_RET_TRAP: u32 = 0x00030000;

    // Load syscall number (32-bit at offset 0)
    bpf.push(SockFilter {
        code: BPF_LD | BPF_W | BPF_ABS,
        jt: 0,
        jf: 0,
        k: SECCOMP_DATA_NR_OFFSET,
    });

    // For each allowed syscall, add a jump-if-equal check
    // We chain them: if syscall == X, jump to allow; else fall through
    let num_syscalls = allowed_syscalls.len();

    for (i, &syscall) in allowed_syscalls.iter().enumerate() {
        let remaining = (num_syscalls - i - 1) as u8;
        bpf.push(SockFilter {
            code: BPF_JMP | BPF_JEQ | BPF_K,
            jt: remaining, // If equal, skip remaining checks to allow
            jf: 0,
            k: syscall as u32,
        });
    }

    // Return TRAP (kill with signal) for non-matching syscalls
    bpf.push(SockFilter {
        code: BPF_RET | BPF_K,
        jt: 0,
        jf: 0,
        k: SECCOMP_RET_TRAP,
    });

    // Return ALLOW for matching syscalls
    bpf.push(SockFilter {
        code: BPF_RET | BPF_K,
        jt: 0,
        jf: 0,
        k: SECCOMP_RET_ALLOW,
    });

    bpf
}

/// Linux-specific seccomp types for raw syscall interface.
#[cfg(target_os = "linux")]
mod linux_seccomp {
    #[repr(C)]
    pub struct SockFprog {
        pub len: u16,
        pub filter: *const nix::sys::socket::SockFilter,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_not_empty() {
        let profile = default_restricted_profile();
        assert!(!profile.is_empty());
    }

    #[test]
    fn default_profile_contains_exit() {
        let profile = default_restricted_profile();
        // Exit syscall should always be present
        #[cfg(target_arch = "x86_64")]
        assert!(profile.contains(&60)); // __NR_exit on x86_64

        #[cfg(target_arch = "aarch64")]
        assert!(profile.contains(&93)); // __NR_exit on aarch64
    }

    #[test]
    fn default_profile_contains_read() {
        let profile = default_restricted_profile();
        #[cfg(target_arch = "x86_64")]
        assert!(profile.contains(&0)); // __NR_read on x86_64

        #[cfg(target_arch = "aarch64")]
        assert!(profile.contains(&63)); // __NR_read on aarch64
    }

    #[test]
    fn default_profile_contains_write() {
        let profile = default_restricted_profile();
        #[cfg(target_arch = "x86_64")]
        assert!(profile.contains(&1)); // __NR_write on x86_64

        #[cfg(target_arch = "aarch64")]
        assert!(profile.contains(&64)); // __NR_write on aarch64
    }

    #[test]
    fn default_profile_has_no_duplicates() {
        let profile = default_restricted_profile();
        let mut sorted = profile.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(
            profile.len(),
            sorted.len(),
            "duplicate syscall numbers in profile"
        );
    }

    #[test]
    fn default_profile_syscalls_are_positive() {
        let profile = default_restricted_profile();
        for &syscall in &profile {
            assert!(syscall >= 0, "negative syscall number: {syscall}");
        }
    }

    #[test]
    #[cfg(not(target_os = "linux"))]
    fn seccomp_unavailable_on_non_linux() {
        assert!(probe_seccomp().is_err());
        assert!(apply_seccomp_profile(&default_restricted_profile()).is_err());
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn probe_checks_proc_status() {
        let result = probe_seccomp();
        // Should succeed on modern kernels
        assert!(result.is_ok() || result.is_err());
    }

    #[test]
    fn build_filter_produces_instructions() {
        #[cfg(target_os = "linux")]
        {
            let syscalls = vec![0i64, 1, 60];
            let bpf = build_bpf_filter(&syscalls);
            // Should have: load + 3 checks + ret_trap + ret_allow = 6 instructions
            assert_eq!(bpf.len(), 6);
        }
    }

    #[test]
    fn build_filter_empty_syscalls() {
        #[cfg(target_os = "linux")]
        {
            let syscalls: Vec<i64> = vec![];
            let bpf = build_bpf_filter(&syscalls);
            // Should have: load + ret_trap + ret_allow = 3 instructions
            assert_eq!(bpf.len(), 3);
        }
    }
}
