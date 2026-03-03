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
    fn kernel_sys_pselect6(
        nfds: i32,
        readfds: *mut u8,
        writefds: *mut u8,
        exceptfds: *mut u8,
        timeout: *const u8,
        sigmask: *const u8,
    ) -> i64;
    fn kernel_sys_epoll_create1(flags: u32) -> i64;
    fn kernel_sys_epoll_ctl(epfd: i32, op: i32, fd: i32, event: *const u8) -> i64;
    fn kernel_sys_epoll_pwait(
        epfd: i32,
        events: *mut u8,
        maxevents: i32,
        timeout: i32,
        sigmask: *const u8,
        sigsetsize: usize,
    ) -> i64;
    fn kernel_sys_close(fd: i32) -> i64;
}

const EFAULT: i64 = -14;
const EBADF: i64 = -9;
const EINVAL: i64 = -22;

const POLLIN: i16 = 0x0001;
const POLLOUT: i16 = 0x0004;
const POLLNVAL: i16 = 0x0020;

const EPOLL_CTL_ADD: i32 = 1;
const EPOLL_CTL_DEL: i32 = 2;
const EPOLL_CTL_MOD: i32 = 3;
const EPOLLIN: u32 = 0x0001;
const EPOLLOUT: u32 = 0x0004;

const FD_SET_BYTES: usize = 128;
const EPOLL_EVENT_SIZE: usize = 12;

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

fn fd_set(fd: usize, set: &mut [u8; FD_SET_BYTES]) {
    let byte = fd / 8;
    let bit = fd % 8;
    set[byte] |= 1u8 << bit;
}

fn fd_isset(fd: usize, set: &[u8; FD_SET_BYTES]) -> bool {
    let byte = fd / 8;
    let bit = fd % 8;
    (set[byte] & (1u8 << bit)) != 0
}

fn encode_epoll_event(events: u32, data: u64) -> [u8; EPOLL_EVENT_SIZE] {
    let mut raw = [0u8; EPOLL_EVENT_SIZE];
    raw[0..4].copy_from_slice(&events.to_ne_bytes());
    raw[4..12].copy_from_slice(&data.to_ne_bytes());
    raw
}

fn decode_epoll_event(raw: &[u8; EPOLL_EVENT_SIZE]) -> (u32, u64) {
    let events = u32::from_ne_bytes([raw[0], raw[1], raw[2], raw[3]]);
    let data = u64::from_ne_bytes([
        raw[4], raw[5], raw[6], raw[7], raw[8], raw[9], raw[10], raw[11],
    ]);
    (events, data)
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

    print("[test_ppoll] === pselect6(72) Tests ===\n");

    print("[test_ppoll] test: nfds<0 -> EINVAL ... ");
    let rc = unsafe {
        kernel_sys_pselect6(
            -1,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            core::ptr::null(),
            core::ptr::null(),
        )
    };
    if rc != EINVAL {
        print("FAIL\n");
        return -8;
    }
    print("PASS\n");

    print("[test_ppoll] test: stdin/stdout/stderr readiness ... ");
    let mut readfds = [0u8; FD_SET_BYTES];
    let mut writefds = [0u8; FD_SET_BYTES];
    let mut exceptfds = [0u8; FD_SET_BYTES];
    fd_set(0, &mut readfds);
    fd_set(1, &mut writefds);
    fd_set(2, &mut exceptfds);
    let zero_ts = LinuxTimespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    let rc = unsafe {
        kernel_sys_pselect6(
            3,
            readfds.as_mut_ptr(),
            writefds.as_mut_ptr(),
            exceptfds.as_mut_ptr(),
            &zero_ts as *const LinuxTimespec as *const u8,
            core::ptr::null(),
        )
    };
    if rc != 3 || !fd_isset(0, &readfds) || !fd_isset(1, &writefds) || !fd_isset(2, &exceptfds) {
        print("FAIL\n");
        return -9;
    }
    print("PASS\n");

    print("[test_ppoll] test: invalid fd in set -> EBADF ... ");
    let mut bad_set = [0u8; FD_SET_BYTES];
    fd_set(999, &mut bad_set);
    let rc = unsafe {
        kernel_sys_pselect6(
            1000,
            bad_set.as_mut_ptr(),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            &zero_ts as *const LinuxTimespec as *const u8,
            core::ptr::null(),
        )
    };
    if rc != EBADF {
        print("FAIL\n");
        return -10;
    }
    print("PASS\n");

    print("[test_ppoll] === epoll(20/21/22) Tests ===\n");

    print("[test_ppoll] test: epoll_create1 invalid flags -> EINVAL ... ");
    let rc = unsafe { kernel_sys_epoll_create1(1) };
    if rc != EINVAL {
        print("FAIL\n");
        return -11;
    }
    print("PASS\n");

    let epfd = unsafe { kernel_sys_epoll_create1(0) };
    if epfd < 0 {
        print("[test_ppoll] test: epoll_create1(0) ... FAIL\n");
        return -12;
    }
    let epfd = epfd as i32;
    print("[test_ppoll] test: epoll_ctl ADD + epoll_pwait ready ... ");
    let add_evt = encode_epoll_event(EPOLLOUT, 0x1122_3344_5566_7788u64);
    let rc = unsafe { kernel_sys_epoll_ctl(epfd, EPOLL_CTL_ADD, 1, add_evt.as_ptr()) };
    if rc != 0 {
        print("FAIL\n");
        unsafe {
            let _ = kernel_sys_close(epfd);
        }
        return -13;
    }
    let mut events = [[0u8; EPOLL_EVENT_SIZE]; 4];
    let rc = unsafe {
        kernel_sys_epoll_pwait(
            epfd,
            events.as_mut_ptr().cast::<u8>(),
            events.len() as i32,
            0,
            core::ptr::null(),
            0,
        )
    };
    let (ev_mask, ev_data) = decode_epoll_event(&events[0]);
    if rc != 1 || (ev_mask & EPOLLOUT) == 0 || ev_data != 0x1122_3344_5566_7788u64 {
        print("FAIL\n");
        unsafe {
            let _ = kernel_sys_close(epfd);
        }
        return -14;
    }
    print("PASS\n");

    print("[test_ppoll] test: epoll_ctl MOD update data/events ... ");
    let mod_evt = encode_epoll_event(EPOLLIN, 0xAABB_CCDD_EEFF_0011u64);
    let rc = unsafe { kernel_sys_epoll_ctl(epfd, EPOLL_CTL_MOD, 1, mod_evt.as_ptr()) };
    if rc != 0 {
        print("FAIL\n");
        unsafe {
            let _ = kernel_sys_close(epfd);
        }
        return -15;
    }
    events = [[0u8; EPOLL_EVENT_SIZE]; 4];
    let rc = unsafe {
        kernel_sys_epoll_pwait(
            epfd,
            events.as_mut_ptr().cast::<u8>(),
            events.len() as i32,
            0,
            core::ptr::null(),
            0,
        )
    };
    let (ev_mask, ev_data) = decode_epoll_event(&events[0]);
    if rc != 1 || (ev_mask & EPOLLIN) == 0 || ev_data != 0xAABB_CCDD_EEFF_0011u64 {
        print("FAIL\n");
        unsafe {
            let _ = kernel_sys_close(epfd);
        }
        return -16;
    }
    print("PASS\n");

    print("[test_ppoll] test: epoll_ctl DEL + timeout0 -> 0 ... ");
    let rc = unsafe { kernel_sys_epoll_ctl(epfd, EPOLL_CTL_DEL, 1, core::ptr::null()) };
    if rc != 0 {
        print("FAIL\n");
        unsafe {
            let _ = kernel_sys_close(epfd);
        }
        return -17;
    }
    events = [[0u8; EPOLL_EVENT_SIZE]; 4];
    let rc = unsafe {
        kernel_sys_epoll_pwait(
            epfd,
            events.as_mut_ptr().cast::<u8>(),
            events.len() as i32,
            0,
            core::ptr::null(),
            0,
        )
    };
    if rc != 0 {
        print("FAIL\n");
        unsafe {
            let _ = kernel_sys_close(epfd);
        }
        return -18;
    }
    print("PASS\n");

    unsafe {
        let _ = kernel_sys_close(epfd);
    }

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
