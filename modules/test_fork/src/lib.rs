//! fork/wait 계열 syscall 테스트 모듈

#![no_std]
#![no_main]

use core::panic::PanicInfo;

unsafe extern "C" {
    fn kernel_print(s: *const u8, len: usize);
    fn kernel_sys_fork() -> i64;
    fn kernel_sys_vfork() -> i64;
    fn kernel_sys_wait4(pid: isize, status: *mut i32, options: i32) -> i64;
    fn kernel_sys_waitid(idtype: i32, id: usize, infop: *mut u8, options: i32) -> i64;
    fn kernel_sys_uname(buf: *mut u8) -> i64;
    fn kernel_sys_rt_sigprocmask(
        how: i32,
        set: *const u8,
        oldset: *mut u8,
        sigsetsize: usize,
    ) -> i64;
    fn kernel_sys_rt_sigtimedwait(
        set: *const u8,
        info: *mut u8,
        timeout: *const u8,
        sigsetsize: usize,
    ) -> i64;
}

const WNOHANG: i32 = 0x1;
const WEXITED: i32 = 0x4;
const WNOWAIT: i32 = 0x0100_0000;
const P_PID: i32 = 1;
const ECHILD: i64 = -10;
const EAGAIN: i64 = -11;
const CLD_EXITED: i32 = 1;
const SIGCHLD: u32 = 17;
const SIG_SETMASK: i32 = 2;

#[repr(C)]
#[derive(Clone, Copy)]
struct LinuxWaitidSigInfo {
    si_signo: i32,
    si_errno: i32,
    si_code: i32,
    si_pid: i32,
    si_uid: u32,
    si_status: i32,
    si_utime: i64,
    si_stime: i64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct LinuxUtsName {
    sysname: [u8; 65],
    nodename: [u8; 65],
    release: [u8; 65],
    version: [u8; 65],
    machine: [u8; 65],
    domainname: [u8; 65],
}

fn print(s: &str) {
    unsafe {
        kernel_print(s.as_ptr(), s.len());
    }
}

fn wifexited(status: i32) -> bool {
    (status & 0x7f) == 0
}

fn wexitstatus(status: i32) -> i32 {
    (status >> 8) & 0xff
}

fn c_field_starts_with(field: &[u8; 65], expected: &str) -> bool {
    let expected_bytes = expected.as_bytes();
    if expected_bytes.len() > field.len() {
        return false;
    }
    let mut i = 0usize;
    while i < expected_bytes.len() {
        if field[i] != expected_bytes[i] {
            return false;
        }
        i += 1;
    }
    true
}

fn c_field_non_empty(field: &[u8; 65]) -> bool {
    field[0] != 0
}

#[unsafe(no_mangle)]
pub extern "C" fn module_init() -> i32 {
    print("[test_fork] === fork/wait syscall Tests ===\n");

    print("[test_fork] test: fork/wait4 status macros ... ");
    let child = unsafe { kernel_sys_fork() };
    if child <= 0 {
        print("FAIL (fork)\n");
        return -1;
    }

    let mut wait_status: i32 = -1;
    let waited = unsafe { kernel_sys_wait4(child as isize, &mut wait_status as *mut i32, 0) };
    if waited != child || !wifexited(wait_status) || wexitstatus(wait_status) != 0 {
        print("FAIL\n");
        return -2;
    }
    print("PASS\n");

    print("[test_fork] test: waitid(WNOWAIT) + wait4 consume ... ");
    let child2 = unsafe { kernel_sys_fork() };
    if child2 <= 0 {
        print("FAIL (fork2)\n");
        return -3;
    }

    let mut info = LinuxWaitidSigInfo {
        si_signo: 0,
        si_errno: 0,
        si_code: 0,
        si_pid: 0,
        si_uid: 0,
        si_status: 0,
        si_utime: 0,
        si_stime: 0,
    };

    let waitid_rc = unsafe {
        kernel_sys_waitid(
            P_PID,
            child2 as usize,
            &mut info as *mut LinuxWaitidSigInfo as *mut u8,
            WEXITED | WNOWAIT,
        )
    };
    if waitid_rc != 0 || info.si_pid != child2 as i32 || info.si_code != CLD_EXITED || info.si_status != 0 {
        print("FAIL (waitid)\n");
        return -4;
    }

    wait_status = -1;
    let waited2 = unsafe { kernel_sys_wait4(child2 as isize, &mut wait_status as *mut i32, 0) };
    if waited2 != child2 || !wifexited(wait_status) || wexitstatus(wait_status) != 0 {
        print("FAIL (wait4)\n");
        return -5;
    }
    print("PASS\n");

    print("[test_fork] test: vfork/waitid consume ... ");
    let child3 = unsafe { kernel_sys_vfork() };
    if child3 <= 0 {
        print("FAIL (vfork)\n");
        return -6;
    }

    info = LinuxWaitidSigInfo {
        si_signo: 0,
        si_errno: 0,
        si_code: 0,
        si_pid: 0,
        si_uid: 0,
        si_status: 0,
        si_utime: 0,
        si_stime: 0,
    };

    let vwaitid_rc = unsafe {
        kernel_sys_waitid(
            P_PID,
            child3 as usize,
            &mut info as *mut LinuxWaitidSigInfo as *mut u8,
            WEXITED,
        )
    };
    if vwaitid_rc != 0 || info.si_pid != child3 as i32 || info.si_code != CLD_EXITED {
        print("FAIL (waitid)\n");
        return -7;
    }

    wait_status = -1;
    let no_child = unsafe { kernel_sys_wait4(child3 as isize, &mut wait_status as *mut i32, WNOHANG) };
    if no_child != ECHILD {
        print("FAIL (reap)\n");
        return -8;
    }
    print("PASS\n");

    print("[test_fork] test: uname basics ... ");
    let mut uts = LinuxUtsName {
        sysname: [0; 65],
        nodename: [0; 65],
        release: [0; 65],
        version: [0; 65],
        machine: [0; 65],
        domainname: [0; 65],
    };

    let uname_rc = unsafe { kernel_sys_uname(&mut uts as *mut LinuxUtsName as *mut u8) };
    if uname_rc != 0 || !c_field_starts_with(&uts.sysname, "Kerners") || !c_field_non_empty(&uts.machine)
    {
        print("FAIL\n");
        return -9;
    }
    print("PASS\n");

    print("[test_fork] test: drain pending SIGCHLD ... ");
    let sigset_size = core::mem::size_of::<u64>();
    let sigchld_mask: u64 = 1u64 << (SIGCHLD - 1);
    let mut old_mask: u64 = 0;
    let setmask_rc = unsafe {
        kernel_sys_rt_sigprocmask(
            SIG_SETMASK,
            &sigchld_mask as *const u64 as *const u8,
            &mut old_mask as *mut u64 as *mut u8,
            sigset_size,
        )
    };
    if setmask_rc != 0 {
        print("FAIL (mask)\n");
        return -10;
    }

    let mut siginfo: [u8; 16] = [0; 16];
    loop {
        let rc = unsafe {
            kernel_sys_rt_sigtimedwait(
                &sigchld_mask as *const u64 as *const u8,
                siginfo.as_mut_ptr(),
                core::ptr::null(),
                sigset_size,
            )
        };
        if rc == EAGAIN {
            break;
        }
        if rc != SIGCHLD as i64 {
            print("FAIL (drain)\n");
            return -11;
        }
    }

    let restore_rc = unsafe {
        kernel_sys_rt_sigprocmask(
            SIG_SETMASK,
            &old_mask as *const u64 as *const u8,
            core::ptr::null_mut(),
            sigset_size,
        )
    };
    if restore_rc != 0 {
        print("FAIL (restore)\n");
        return -12;
    }
    print("PASS\n");

    print("[test_fork] All tests passed\n");
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn module_exit() {
    print("[test_fork] Module unloaded\n");
}

#[unsafe(no_mangle)]
pub extern "C" fn module_name() -> *const u8 {
    b"test_fork\0".as_ptr()
}

#[unsafe(no_mangle)]
pub extern "C" fn module_version() -> *const u8 {
    b"0.1.0\0".as_ptr()
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    print("[test_fork] PANIC!\n");
    loop {}
}
