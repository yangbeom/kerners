//! time/timer syscall 테스트 모듈

#![no_std]
#![no_main]

use core::panic::PanicInfo;

unsafe extern "C" {
    fn kernel_print(s: *const u8, len: usize);
    fn kernel_sys_gettid() -> i64;
    fn kernel_sys_clock_gettime(clock_id: i32, tp: *mut u8) -> i64;
    fn kernel_sys_clock_getres(clock_id: i32, tp: *mut u8) -> i64;
    fn kernel_sys_gettimeofday(tv: *mut u8, tz: *mut u8) -> i64;
    fn kernel_sys_nanosleep(req: *const u8, rem: *mut u8) -> i64;
    fn kernel_sys_tkill(tid: isize, sig: i32) -> i64;
    fn kernel_sys_rt_sigtimedwait(
        set: *const u8,
        info: *mut u8,
        timeout: *const u8,
        sigsetsize: usize,
    ) -> i64;
}

const CLOCK_REALTIME: i32 = 0;
const CLOCK_MONOTONIC: i32 = 1;
const SIGCHLD: i32 = 17;
const EINVAL: i64 = -22;
const EINTR: i64 = -4;
const EAGAIN: i64 = -11;

#[repr(C)]
#[derive(Clone, Copy)]
struct LinuxTimespec {
    tv_sec: i64,
    tv_nsec: i64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct LinuxTimeval {
    tv_sec: i64,
    tv_usec: i64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct LinuxTimezone {
    tz_minuteswest: i32,
    tz_dsttime: i32,
}

fn print(s: &str) {
    unsafe {
        kernel_print(s.as_ptr(), s.len());
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn module_init() -> i32 {
    print("[test_timer] === timer syscall Tests ===\n");

    print("[test_timer] test: clock_gettime monotonic increase ... ");
    let mut t0 = LinuxTimespec { tv_sec: 0, tv_nsec: 0 };
    let mut t1 = LinuxTimespec { tv_sec: 0, tv_nsec: 0 };
    let rc0 = unsafe { kernel_sys_clock_gettime(CLOCK_MONOTONIC, &mut t0 as *mut _ as *mut u8) };
    let rc1 = unsafe { kernel_sys_clock_gettime(CLOCK_MONOTONIC, &mut t1 as *mut _ as *mut u8) };
    let n0 = t0.tv_sec.saturating_mul(1_000_000_000).saturating_add(t0.tv_nsec);
    let n1 = t1.tv_sec.saturating_mul(1_000_000_000).saturating_add(t1.tv_nsec);
    if rc0 != 0 || rc1 != 0 || n1 < n0 {
        print("FAIL\n");
        return -1;
    }
    print("PASS\n");

    print("[test_timer] test: clock_gettime realtime valid ... ");
    let mut tr = LinuxTimespec { tv_sec: 0, tv_nsec: 0 };
    let rcr = unsafe { kernel_sys_clock_gettime(CLOCK_REALTIME, &mut tr as *mut _ as *mut u8) };
    if rcr != 0 || tr.tv_sec < 0 || tr.tv_nsec < 0 || tr.tv_nsec >= 1_000_000_000 {
        print("FAIL\n");
        return -2;
    }
    print("PASS\n");

    print("[test_timer] test: clock_getres normal/invalid/null ... ");
    let mut res = LinuxTimespec { tv_sec: 0, tv_nsec: 0 };
    let r0 = unsafe { kernel_sys_clock_getres(CLOCK_MONOTONIC, &mut res as *mut _ as *mut u8) };
    let r1 = unsafe { kernel_sys_clock_getres(99, &mut res as *mut _ as *mut u8) };
    let r2 = unsafe { kernel_sys_clock_getres(CLOCK_REALTIME, core::ptr::null_mut()) };
    if r0 != 0 || r1 != EINVAL || r2 != 0 || (res.tv_sec == 0 && res.tv_nsec <= 0) {
        print("FAIL\n");
        return -3;
    }
    print("PASS\n");

    print("[test_timer] test: gettimeofday + timezone ... ");
    let mut tv = LinuxTimeval { tv_sec: 0, tv_usec: 0 };
    let mut tz = LinuxTimezone {
        tz_minuteswest: -1,
        tz_dsttime: -1,
    };
    let gtr = unsafe { kernel_sys_gettimeofday(&mut tv as *mut _ as *mut u8, &mut tz as *mut _ as *mut u8) };
    if gtr != 0 || tv.tv_sec < 0 || tv.tv_usec < 0 || tv.tv_usec >= 1_000_000 || tz.tz_minuteswest != 0 || tz.tz_dsttime != 0 {
        print("FAIL\n");
        return -4;
    }
    print("PASS\n");

    // 이전 테스트에서 남아 있을 수 있는 SIGCHLD pending을 비운다.
    let wait_set: u64 = 1u64 << ((SIGCHLD as u32) - 1);
    let poll_timeout = LinuxTimespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    loop {
        let mut siginfo = [0u8; 16];
        let got = unsafe {
            kernel_sys_rt_sigtimedwait(
                &wait_set as *const u64 as *const u8,
                siginfo.as_mut_ptr(),
                &poll_timeout as *const LinuxTimespec as *const u8,
                core::mem::size_of::<u64>(),
            )
        };
        if got == EAGAIN {
            break;
        }
        if got != SIGCHLD as i64 {
            print("FAIL (drain-pre)\n");
            return -5;
        }
    }

    print("[test_timer] test: nanosleep timeout ... ");
    let req = LinuxTimespec {
        tv_sec: 0,
        tv_nsec: 20_000_000,
    };
    let mut rem = LinuxTimespec {
        tv_sec: -1,
        tv_nsec: -1,
    };
    let ns0 = unsafe { kernel_sys_nanosleep(&req as *const _ as *const u8, &mut rem as *mut _ as *mut u8) };
    if ns0 != 0 || rem.tv_sec != 0 || rem.tv_nsec != 0 {
        print("FAIL\n");
        return -5;
    }
    print("PASS\n");

    print("[test_timer] test: nanosleep EINTR + rem ... ");
    let me = unsafe { kernel_sys_gettid() };
    if me < 0 {
        print("FAIL (tid)\n");
        return -6;
    }
    let krc = unsafe { kernel_sys_tkill(me as isize, SIGCHLD) };
    if krc != 0 {
        print("FAIL (tkill)\n");
        return -7;
    }

    let long_req = LinuxTimespec {
        tv_sec: 1,
        tv_nsec: 0,
    };
    let mut long_rem = LinuxTimespec { tv_sec: 0, tv_nsec: 0 };
    let ns1 = unsafe {
        kernel_sys_nanosleep(
            &long_req as *const _ as *const u8,
            &mut long_rem as *mut _ as *mut u8,
        )
    };
    if ns1 != EINTR {
        print("FAIL (eintr)\n");
        return -8;
    }
    if long_rem.tv_sec < 0 || long_rem.tv_nsec < 0 || long_rem.tv_nsec >= 1_000_000_000 {
        print("FAIL (rem)\n");
        return -9;
    }

    let mut siginfo = [0u8; 16];
    let got = unsafe {
        kernel_sys_rt_sigtimedwait(
            &wait_set as *const u64 as *const u8,
            siginfo.as_mut_ptr(),
            &poll_timeout as *const LinuxTimespec as *const u8,
            core::mem::size_of::<u64>(),
        )
    };
    if got != SIGCHLD as i64 && got != EAGAIN {
        print("FAIL (drain)\n");
        return -10;
    }

    print("PASS\n");
    print("[test_timer] All tests passed\n");
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn module_exit() {
    print("[test_timer] Module unloaded\n");
}

#[unsafe(no_mangle)]
pub extern "C" fn module_name() -> *const u8 {
    b"test_timer\0".as_ptr()
}

#[unsafe(no_mangle)]
pub extern "C" fn module_version() -> *const u8 {
    b"0.1.0\0".as_ptr()
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    print("[test_timer] PANIC\n");
    loop {}
}
