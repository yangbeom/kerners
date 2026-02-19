//! ppoll(73) syscall 회귀 테스트 모듈

#![no_std]
#![no_main]

use core::panic::PanicInfo;

unsafe extern "C" {
    fn kernel_print(s: *const u8, len: usize);
    fn kernel_sys_ppoll(
        fds: *mut u8,
        nfds: usize,
        timeout: *const u8,
        sigmask: *const u8,
        sigsetsize: usize,
    ) -> i64;
}

const EFAULT: i64 = -14;
const EINVAL: i64 = -22;

const POLLIN: i16 = 0x0001;
const POLLOUT: i16 = 0x0004;
const POLLNVAL: i16 = 0x0020;

#[repr(C)]
#[derive(Clone, Copy)]
struct LinuxPollFd {
    fd: i32,
    events: i16,
    revents: i16,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct LinuxTimespec {
    tv_sec: i64,
    tv_nsec: i64,
}

fn print(s: &str) {
    unsafe {
        kernel_print(s.as_ptr(), s.len());
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn module_init() -> i32 {
    print("[test_ppoll] === ppoll(73) Tests ===\n");

    print("[test_ppoll] test: null fds + nfds>0 -> EFAULT ... ");
    let rc = unsafe { kernel_sys_ppoll(core::ptr::null_mut(), 1, core::ptr::null(), core::ptr::null(), 0) };
    if rc != EFAULT {
        print("FAIL\n");
        return -1;
    }
    print("PASS\n");

    print("[test_ppoll] test: nfds upper bound -> EINVAL ... ");
    let mut dummy = LinuxPollFd {
        fd: 0,
        events: POLLIN,
        revents: 0,
    };
    let rc = unsafe {
        kernel_sys_ppoll(
            (&mut dummy as *mut LinuxPollFd).cast::<u8>(),
            1025,
            core::ptr::null(),
            core::ptr::null(),
            0,
        )
    };
    if rc != EINVAL {
        print("FAIL\n");
        return -2;
    }
    print("PASS\n");

    print("[test_ppoll] test: invalid timespec -> EINVAL ... ");
    let invalid_ts = LinuxTimespec {
        tv_sec: 0,
        tv_nsec: 1_000_000_000,
    };
    let rc = unsafe {
        kernel_sys_ppoll(
            (&mut dummy as *mut LinuxPollFd).cast::<u8>(),
            1,
            &invalid_ts as *const LinuxTimespec as *const u8,
            core::ptr::null(),
            0,
        )
    };
    if rc != EINVAL {
        print("FAIL\n");
        return -3;
    }
    print("PASS\n");

    print("[test_ppoll] test: stdin/stdout/stderr readiness ... ");
    let mut fds = [
        LinuxPollFd {
            fd: 0,
            events: POLLIN,
            revents: 0,
        },
        LinuxPollFd {
            fd: 1,
            events: POLLOUT,
            revents: 0,
        },
        LinuxPollFd {
            fd: 2,
            events: POLLOUT,
            revents: 0,
        },
    ];
    let rc = unsafe {
        kernel_sys_ppoll(
            fds.as_mut_ptr().cast::<u8>(),
            fds.len(),
            core::ptr::null(),
            core::ptr::null(),
            8,
        )
    };
    if rc != 3 {
        print("FAIL\n");
        return -4;
    }
    if (fds[0].revents & POLLIN) == 0 || (fds[1].revents & POLLOUT) == 0 || (fds[2].revents & POLLOUT) == 0 {
        print("FAIL\n");
        return -5;
    }
    print("PASS\n");

    print("[test_ppoll] test: invalid fd -> POLLNVAL ... ");
    let mut invalid_fd = [LinuxPollFd {
        fd: 9999,
        events: POLLIN,
        revents: 0,
    }];
    let rc = unsafe {
        kernel_sys_ppoll(
            invalid_fd.as_mut_ptr().cast::<u8>(),
            1,
            core::ptr::null(),
            core::ptr::null(),
            8,
        )
    };
    if rc != 1 || invalid_fd[0].revents != POLLNVAL {
        print("FAIL\n");
        return -6;
    }
    print("PASS\n");

    print("[test_ppoll] test: zero-timeout with no active fd -> 0 ... ");
    let mut no_fd = [LinuxPollFd {
        fd: -1,
        events: 0,
        revents: 0,
    }];
    let zero_ts = LinuxTimespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    let rc = unsafe {
        kernel_sys_ppoll(
            no_fd.as_mut_ptr().cast::<u8>(),
            1,
            &zero_ts as *const LinuxTimespec as *const u8,
            core::ptr::null(),
            0,
        )
    };
    if rc != 0 {
        print("FAIL\n");
        return -7;
    }
    print("PASS\n");

    print("[test_ppoll] All tests passed\n");
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn module_exit() {
    print("[test_ppoll] Module unloaded\n");
}

#[unsafe(no_mangle)]
pub extern "C" fn module_name() -> *const u8 {
    b"test_ppoll\0".as_ptr()
}

#[unsafe(no_mangle)]
pub extern "C" fn module_version() -> *const u8 {
    b"0.1.0\0".as_ptr()
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    print("[test_ppoll] PANIC!\n");
    loop {}
}
