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

/// exit(status) -> !
pub const SYS_EXIT: usize = 93;

/// exit_group(status) -> !
pub const SYS_EXIT_GROUP: usize = 94;

/// sched_yield() -> int
pub const SYS_SCHED_YIELD: usize = 124;

/// getpid() -> pid_t
pub const SYS_GETPID: usize = 172;

/// nanosleep(req, rem) -> int
pub const SYS_NANOSLEEP: usize = 101;

/// brk(addr) -> void*
pub const SYS_BRK: usize = 214;

/// mkdirat(dirfd, path, mode) -> int
pub const SYS_MKDIRAT: usize = 34;

/// unlinkat(dirfd, path, flags) -> int
pub const SYS_UNLINKAT: usize = 35;

/// mmap(addr, len, prot, flags, fd, offset) -> void*
pub const SYS_MMAP: usize = 222;

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
        SYS_OPENAT => {
            // openat(dirfd, path, flags, mode) - dirfd 무시하고 path만 사용
            fs::sys_open(args[1] as *const u8, args[2] as u32, args[3] as u32)
        }
        SYS_CLOSE => fs::sys_close(args[0] as i32),
        SYS_LSEEK => fs::sys_lseek(args[0] as i32, args[1] as i64, args[2] as i32),
        SYS_READ => fs::sys_read(args[0], args[1] as *mut u8, args[2]),
        SYS_WRITE => fs::sys_write(args[0], args[1] as *const u8, args[2]),
        SYS_FSTAT => fs::sys_fstat(args[0] as i32, args[1] as *mut u8),
        SYS_EXIT => process::sys_exit(args[0] as i32),
        SYS_EXIT_GROUP => process::sys_exit(args[0] as i32),
        SYS_SCHED_YIELD => process::sys_yield(),
        SYS_GETPID => process::sys_getpid(),
        SYS_MKDIRAT => {
            // mkdirat(dirfd, path, mode) - dirfd 무시
            fs::sys_mkdir(args[1] as *const u8, args[2] as u32)
        }
        SYS_UNLINKAT => {
            // unlinkat(dirfd, path, flags) - dirfd, flags 무시
            fs::sys_unlink(args[1] as *const u8)
        }
        _ => {
            kprintln!("[syscall] Unknown syscall: {} (args: {:?})", syscall_num, args);
            -1 // EPERM
        }
    }
}

/// 에러 코드 (Linux 호환)
#[allow(dead_code)]
pub mod errno {
    pub const EPERM: isize = -1;
    pub const ENOENT: isize = -2;
    pub const ESRCH: isize = -3;
    pub const EINTR: isize = -4;
    pub const EIO: isize = -5;
    pub const ENOMEM: isize = -12;
    pub const EACCES: isize = -13;
    pub const EFAULT: isize = -14;
    pub const EBUSY: isize = -16;
    pub const ENOTDIR: isize = -20;
    pub const EISDIR: isize = -21;
    pub const EINVAL: isize = -22;
    pub const ENOSYS: isize = -38;
}
