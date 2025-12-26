//! 프로세스 관련 시스템 콜
//!
//! exit, yield, getpid 등

use crate::kprintln;
use crate::proc;

/// sys_exit - 프로세스 종료
///
/// # Arguments
/// * `status` - 종료 상태 코드
///
/// # Returns
/// * 반환하지 않음 (하지만 타입 시그니처상 isize 반환)
pub fn sys_exit(status: i32) -> isize {
    let tid = proc::current_tid().unwrap_or(0);
    kprintln!("[syscall] Process {} exiting with status {}", tid, status);
    proc::exit();
    // exit()는 반환하지 않지만, 컴파일러를 위해
    0
}

/// sys_yield - CPU 양보
///
/// # Returns
/// * 항상 0
pub fn sys_yield() -> isize {
    proc::yield_now();
    0
}

/// sys_getpid - 현재 프로세스 ID 반환
///
/// # Returns
/// * 현재 스레드/프로세스 ID
pub fn sys_getpid() -> isize {
    proc::current_tid().unwrap_or(0) as isize
}
