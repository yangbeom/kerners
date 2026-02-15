//! 시스템 콜 인터페이스
//!
//! Linux AArch64/RISC-V 호환 시스템 콜 번호 사용
//! 참조: include/uapi/asm-generic/unistd.h

mod fs;
mod process;

use crate::kprintln;

// ============================================================================
// Linux AArch64/RISC-V 시스템 콜 번호 (asm-generic)
// ============================================================================

/// openat(dirfd, path, flags, mode) -> fd
pub const SYS_OPENAT: usize = 56;

/// dup(oldfd) -> newfd
pub const SYS_DUP: usize = 23;

/// dup3(oldfd, newfd, flags) -> newfd
pub const SYS_DUP3: usize = 24;

/// fcntl(fd, cmd, arg) -> int
pub const SYS_FCNTL: usize = 25;

/// ioctl(fd, request, argp) -> int
pub const SYS_IOCTL: usize = 29;

/// chdir(path) -> int
pub const SYS_CHDIR: usize = 49;

/// faccessat(dirfd, path, mode, flags) -> int
pub const SYS_FACCESSAT: usize = 48;

/// getcwd(buf, size) -> ssize_t
pub const SYS_GETCWD: usize = 17;

/// close(fd) -> int
pub const SYS_CLOSE: usize = 57;

/// lseek(fd, offset, whence) -> off_t
pub const SYS_LSEEK: usize = 62;

/// read(fd, buf, count) -> ssize_t
pub const SYS_READ: usize = 63;

/// write(fd, buf, count) -> ssize_t
pub const SYS_WRITE: usize = 64;

/// fstat(fd, statbuf) -> int
pub const SYS_FSTAT: usize = 80;

/// newfstatat(dirfd, path, statbuf, flags) -> int
pub const SYS_NEWFSTATAT: usize = 79;

/// clock_gettime(clockid, tp) -> int
pub const SYS_CLOCK_GETTIME: usize = 113;

/// gettimeofday(tv, tz) -> int
pub const SYS_GETTIMEOFDAY: usize = 169;

/// exit(status) -> !
pub const SYS_EXIT: usize = 93;

/// exit_group(status) -> !
pub const SYS_EXIT_GROUP: usize = 94;

/// waitid(idtype, id, infop, options, rusage) -> int
pub const SYS_WAITID: usize = 95;

/// sched_yield() -> int
pub const SYS_SCHED_YIELD: usize = 124;

/// getpid() -> pid_t
pub const SYS_GETPID: usize = 172;

/// getppid() -> pid_t
pub const SYS_GETPPID: usize = 173;

/// getuid() -> uid_t
pub const SYS_GETUID: usize = 174;

/// geteuid() -> uid_t
pub const SYS_GETEUID: usize = 175;

/// getgid() -> gid_t
pub const SYS_GETGID: usize = 176;

/// getegid() -> gid_t
pub const SYS_GETEGID: usize = 177;

/// gettid() -> pid_t
pub const SYS_GETTID: usize = 178;

/// execve(path, argv, envp) -> int
pub const SYS_EXECVE: usize = 221;

/// nanosleep(req, rem) -> int
pub const SYS_NANOSLEEP: usize = 101;

/// set_tid_address(tidptr) -> pid_t
pub const SYS_SET_TID_ADDRESS: usize = 96;

/// rt_sigaction(signum, act, oldact, sigsetsize) -> int
pub const SYS_RT_SIGACTION: usize = 134;

/// rt_sigprocmask(how, set, oldset, sigsetsize) -> int
pub const SYS_RT_SIGPROCMASK: usize = 135;

/// rt_sigtimedwait(set, info, timeout, sigsetsize) -> int
pub const SYS_RT_SIGTIMEDWAIT: usize = 137;

/// setgid(gid) -> int
pub const SYS_SETGID: usize = 144;

/// setuid(uid) -> int
pub const SYS_SETUID: usize = 146;

/// setpgid(pid, pgid) -> int
pub const SYS_SETPGID: usize = 154;

/// getpgid(pid) -> pid_t
pub const SYS_GETPGID: usize = 155;

/// getsid(pid) -> pid_t
pub const SYS_GETSID: usize = 156;

/// setsid() -> pid_t
pub const SYS_SETSID: usize = 157;

/// uname(buf) -> int
pub const SYS_UNAME: usize = 160;

/// reboot(magic1, magic2, cmd, arg) -> int
pub const SYS_REBOOT: usize = 142;

/// socket(domain, type, protocol) -> int
pub const SYS_SOCKET: usize = 198;

/// sendto(fd, buf, len, flags, addr, addrlen) -> ssize_t
pub const SYS_SENDTO: usize = 206;

/// clone(flags, child_stack, parent_tid, tls, child_tid) -> pid_t
pub const SYS_CLONE: usize = 220;

/// wait4(pid, status, options, rusage) -> pid_t
pub const SYS_WAIT4: usize = 260;

/// brk(addr) -> void*
pub const SYS_BRK: usize = 214;

/// munmap(addr, len) -> int
pub const SYS_MUNMAP: usize = 215;

/// mkdirat(dirfd, path, mode) -> int
pub const SYS_MKDIRAT: usize = 34;

/// unlinkat(dirfd, path, flags) -> int
pub const SYS_UNLINKAT: usize = 35;

/// mmap(addr, len, prot, flags, fd, offset) -> void*
pub const SYS_MMAP: usize = 222;

/// mprotect(addr, len, prot) -> int
pub const SYS_MPROTECT: usize = 226;

// ============================================================================
// 시스템 콜 디스패처
// ============================================================================

/// 시스템 콜 핸들러
///
/// # Arguments
/// * `syscall_num` - 시스템 콜 번호 (x8/a7)
/// * `args` - 인자 배열 [a0, a1, a2, a3, a4, a5]
///
/// # Returns
/// * 성공 시 양수 또는 0
/// * 실패 시 음수 에러 코드
pub fn syscall_handler(syscall_num: usize, args: [usize; 6]) -> isize {
    match syscall_num {
        SYS_DUP => fs::sys_dup(args[0] as i32),
        SYS_DUP3 => fs::sys_dup3(args[0] as i32, args[1] as i32, args[2] as u32),
        SYS_FCNTL => fs::sys_fcntl(args[0] as i32, args[1] as i32, args[2]),
        SYS_IOCTL => fs::sys_ioctl(args[0] as i32, args[1], args[2]),
        SYS_FACCESSAT => fs::sys_faccessat(
            args[0] as i32,
            args[1] as *const u8,
            args[2] as u32,
            args[3] as u32,
        ),
        SYS_GETCWD => fs::sys_getcwd(args[0] as *mut u8, args[1]),
        SYS_CHDIR => fs::sys_chdir(args[0] as *const u8),
        SYS_OPENAT => {
            // openat(dirfd, path, flags, mode) - dirfd 무시하고 path만 사용
            fs::sys_open(args[1] as *const u8, args[2] as u32, args[3] as u32)
        }
        SYS_CLOSE => fs::sys_close(args[0] as i32),
        SYS_LSEEK => fs::sys_lseek(args[0] as i32, args[1] as i64, args[2] as i32),
        SYS_READ => fs::sys_read(args[0], args[1] as *mut u8, args[2]),
        SYS_WRITE => fs::sys_write(args[0], args[1] as *const u8, args[2]),
        SYS_FSTAT => fs::sys_fstat(args[0] as i32, args[1] as *mut u8),
        SYS_NEWFSTATAT => fs::sys_newfstatat(
            args[0] as i32,
            args[1] as *const u8,
            args[2] as *mut u8,
            args[3] as usize,
        ),
        SYS_CLOCK_GETTIME => process::sys_clock_gettime(args[0] as i32, args[1] as *mut u8),
        SYS_GETTIMEOFDAY => process::sys_gettimeofday(args[0] as *mut u8, args[1] as *mut u8),
        SYS_EXIT => process::sys_exit(args[0] as i32),
        SYS_EXIT_GROUP => process::sys_exit(args[0] as i32),
        SYS_WAITID => process::sys_waitid(
            args[0] as i32,
            args[1],
            args[2] as *mut u8,
            args[3] as i32,
            args[4] as *mut u8,
        ),
        SYS_SCHED_YIELD => process::sys_yield(),
        SYS_GETPID => process::sys_getpid(),
        SYS_GETPPID => process::sys_getppid(),
        SYS_GETUID => process::sys_getuid(),
        SYS_GETEUID => process::sys_geteuid(),
        SYS_GETGID => process::sys_getgid(),
        SYS_GETEGID => process::sys_getegid(),
        SYS_GETTID => process::sys_gettid(),
        SYS_SET_TID_ADDRESS => process::sys_set_tid_address(args[0] as *mut i32),
        SYS_NANOSLEEP => process::sys_nanosleep(args[0] as *const u8, args[1] as *mut u8),
        SYS_RT_SIGACTION => process::sys_rt_sigaction(
            args[0] as i32,
            args[1] as *const u8,
            args[2] as *mut u8,
            args[3],
        ),
        SYS_RT_SIGPROCMASK => process::sys_rt_sigprocmask(
            args[0] as i32,
            args[1] as *const u8,
            args[2] as *mut u8,
            args[3],
        ),
        SYS_RT_SIGTIMEDWAIT => process::sys_rt_sigtimedwait(
            args[0] as *const u8,
            args[1] as *mut u8,
            args[2] as *const u8,
            args[3],
        ),
        SYS_SETGID => process::sys_setgid(args[0] as u32),
        SYS_SETUID => process::sys_setuid(args[0] as u32),
        SYS_SETPGID => process::sys_setpgid(args[0] as isize, args[1] as isize),
        SYS_GETPGID => process::sys_getpgid(args[0] as isize),
        SYS_GETSID => process::sys_getsid(args[0] as isize),
        SYS_SETSID => process::sys_setsid(),
        SYS_UNAME => process::sys_uname(args[0] as *mut u8),
        SYS_REBOOT => process::sys_reboot(args[0], args[1], args[2], args[3]),
        SYS_SOCKET => process::sys_socket(args[0] as i32, args[1] as i32, args[2] as i32),
        SYS_SENDTO => process::sys_sendto(
            args[0] as i32,
            args[1] as *const u8,
            args[2],
            args[3] as i32,
            args[4] as *const u8,
            args[5] as u32,
        ),
        SYS_CLONE => process::sys_clone(
            args[0],
            args[1],
            args[2] as *mut u8,
            args[3],
            args[4] as *mut u8,
        ),
        SYS_WAIT4 => process::sys_wait4(
            args[0] as isize,
            args[1] as *mut i32,
            args[2] as i32,
            args[3] as *mut u8,
        ),
        SYS_BRK => process::sys_brk(args[0]),
        SYS_EXECVE => process::sys_execve(
            args[0] as *const u8,
            args[1] as *const *const u8,
            args[2] as *const *const u8,
        ),
        SYS_MMAP => process::sys_mmap(
            args[0],
            args[1],
            args[2],
            args[3],
            args[4] as isize,
            args[5],
        ),
        SYS_MUNMAP => process::sys_munmap(args[0], args[1]),
        SYS_MPROTECT => process::sys_mprotect(args[0], args[1], args[2]),
        SYS_MKDIRAT => {
            // mkdirat(dirfd, path, mode) - dirfd 무시
            fs::sys_mkdir(args[1] as *const u8, args[2] as u32)
        }
        SYS_UNLINKAT => {
            // unlinkat(dirfd, path, flags) - dirfd, flags 무시
            fs::sys_unlink(args[1] as *const u8)
        }
        _ => {
            kprintln!(
                "[syscall] Unknown syscall: {} (args: {:?})",
                syscall_num,
                args
            );
            errno::ENOSYS
        }
    }
}

#[cfg(target_arch = "aarch64")]
pub fn syscall_handler_aarch64_with_user_context(
    syscall_num: usize,
    args: [usize; 6],
    gpr: [u64; 31],
    elr: u64,
    spsr: u64,
    sp_el0: usize,
) -> isize {
    match syscall_num {
        SYS_CLONE => process::sys_clone_with_user_context(
            args[0],
            args[1],
            args[2] as *mut u8,
            args[3],
            args[4] as *mut u8,
            gpr,
            elr,
            spsr,
            sp_el0,
        ),
        _ => syscall_handler(syscall_num, args),
    }
}

#[cfg(target_arch = "riscv64")]
pub fn syscall_handler_riscv64_with_user_context(
    syscall_num: usize,
    args: [usize; 6],
    gpr: [u64; 32],
    mstatus: u64,
    mepc: u64,
) -> isize {
    match syscall_num {
        SYS_CLONE => process::sys_clone_with_user_context_riscv(
            args[0],
            args[1],
            args[2] as *mut u8,
            args[3],
            args[4] as *mut u8,
            gpr,
            mstatus,
            mepc,
        ),
        _ => syscall_handler(syscall_num, args),
    }
}

/// 현재 스레드의 pending exec 전이 정보를 가져온다.
pub fn take_exec_transition_for_current() -> Option<process::ExecTransition> {
    process::take_exec_transition_for_current()
}

/// 테스트 모듈용: fork 래퍼
pub fn fork_for_test() -> isize {
    process::sys_fork()
}

/// 테스트 모듈용: vfork 래퍼
pub fn vfork_for_test() -> isize {
    process::sys_vfork()
}

/// 테스트 모듈용: 현재 태스크 pending signal 큐에 시그널 삽입
pub fn enqueue_signal_for_test(signum: u32) -> isize {
    process::test_enqueue_signal_for_current(signum)
}

#[cfg(target_arch = "aarch64")]
pub fn handle_user_page_fault_aarch64(far: usize, esr: u64) -> bool {
    process::handle_user_page_fault_aarch64(far, esr)
}

#[cfg(target_arch = "riscv64")]
pub fn handle_user_page_fault_riscv64(far: usize, cause: u64) -> bool {
    process::handle_user_page_fault_riscv64(far, cause)
}

/// 에러 코드 (Linux 호환)
#[allow(dead_code)]
pub mod errno {
    pub const E2BIG: isize = -7;
    pub const EBADF: isize = -9;
    pub const ECHILD: isize = -10;
    pub const EAGAIN: isize = -11;
    pub const EPERM: isize = -1;
    pub const ENOENT: isize = -2;
    pub const ESRCH: isize = -3;
    pub const EINTR: isize = -4;
    pub const EIO: isize = -5;
    pub const ENOEXEC: isize = -8;
    pub const ENOMEM: isize = -12;
    pub const EACCES: isize = -13;
    pub const EFAULT: isize = -14;
    pub const EBUSY: isize = -16;
    pub const ENOTDIR: isize = -20;
    pub const EISDIR: isize = -21;
    pub const EINVAL: isize = -22;
    pub const ENOTTY: isize = -25;
    pub const ERANGE: isize = -34;
    pub const EAFNOSUPPORT: isize = -97;
    pub const ENOSYS: isize = -38;
}
