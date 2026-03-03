//! ppoll/pselect6/epoll syscall 회귀 테스트 모듈

#![no_std]
#![no_main]

use core::panic::PanicInfo;

unsafe extern "C" {
    fn kernel_print(s: *const u8, len: usize);
    fn kernel_thread_spawn(
        entry: extern "C" fn(usize),
        arg: usize,
        name: *const u8,
        name_len: usize,
    ) -> i32;
    fn kernel_sleep_ticks(ticks: u32);
    fn kernel_sys_pipe2(pipefd: *mut i32, flags: u32) -> i64;
    fn kernel_sys_read(fd: i32, buf: *mut u8, len: usize) -> i64;
    fn kernel_sys_write(fd: i32, buf: *const u8, len: usize) -> i64;
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
const EPOLLONESHOT: u32 = 1u32 << 30;
const EPOLLET: u32 = 1u32 << 31;

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

#[repr(C)]
#[derive(Clone, Copy)]
struct LinuxPselectSigmaskArg {
    sigmask: usize,
    sigsetsize: usize,
}

fn print(s: &str) {
    unsafe {
        kernel_print(s.as_ptr(), s.len());
    }
}

fn close_fd(fd: i32) {
    unsafe {
        let _ = kernel_sys_close(fd);
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

extern "C" fn delayed_pipe_writer(arg: usize) {
    let fd = arg as i32;
    let payload = [0x5Au8];
    unsafe {
        kernel_sleep_ticks(5);
        let _ = kernel_sys_write(fd, payload.as_ptr(), payload.len());
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
    if (fds[0].revents & POLLIN) == 0
        || (fds[1].revents & POLLOUT) == 0
        || (fds[2].revents & POLLOUT) == 0
    {
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

    print("[test_ppoll] test: sigmask size validation -> EINVAL ... ");
    let mut sigmask_dummy: u64 = 0;
    let rc = unsafe {
        kernel_sys_ppoll(
            core::ptr::null_mut(),
            0,
            &zero_ts as *const LinuxTimespec as *const u8,
            &mut sigmask_dummy as *mut u64 as *const u8,
            4,
        )
    };
    if rc != EINVAL {
        print("FAIL\n");
        return -8;
    }
    print("PASS\n");

    print("[test_ppoll] test: blocking ppoll wakeup by pipe writer ... ");
    let mut ppoll_pipe = [0i32; 2];
    let rc = unsafe { kernel_sys_pipe2(ppoll_pipe.as_mut_ptr(), 0) };
    if rc != 0 {
        print("FAIL\n");
        return -9;
    }
    let name = "ppw";
    let tid = unsafe {
        kernel_thread_spawn(
            delayed_pipe_writer,
            ppoll_pipe[1] as usize,
            name.as_ptr(),
            name.len(),
        )
    };
    if tid <= 0 {
        close_fd(ppoll_pipe[0]);
        close_fd(ppoll_pipe[1]);
        print("FAIL\n");
        return -10;
    }
    let one_sec = LinuxTimespec {
        tv_sec: 1,
        tv_nsec: 0,
    };
    let mut wait_fd = [LinuxPollFd {
        fd: ppoll_pipe[0],
        events: POLLIN,
        revents: 0,
    }];
    let rc = unsafe {
        kernel_sys_ppoll(
            wait_fd.as_mut_ptr().cast::<u8>(),
            1,
            &one_sec as *const LinuxTimespec as *const u8,
            core::ptr::null(),
            0,
        )
    };
    let mut read_buf = [0u8; 1];
    let read_rc = unsafe { kernel_sys_read(ppoll_pipe[0], read_buf.as_mut_ptr(), read_buf.len()) };
    close_fd(ppoll_pipe[0]);
    close_fd(ppoll_pipe[1]);
    if rc != 1 || (wait_fd[0].revents & POLLIN) == 0 || read_rc != 1 {
        print("FAIL\n");
        return -11;
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
        return -12;
    }
    print("PASS\n");

    print("[test_ppoll] test: stdin/stdout/stderr readiness ... ");
    let mut readfds = [0u8; FD_SET_BYTES];
    let mut writefds = [0u8; FD_SET_BYTES];
    let mut exceptfds = [0u8; FD_SET_BYTES];
    fd_set(0, &mut readfds);
    fd_set(1, &mut writefds);
    fd_set(2, &mut exceptfds);
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
        return -13;
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
        return -14;
    }
    print("PASS\n");

    print("[test_ppoll] test: pselect6 sigmask size validation -> EINVAL ... ");
    let mut pselect_sigmask: u64 = 0;
    let sigarg = LinuxPselectSigmaskArg {
        sigmask: (&mut pselect_sigmask as *mut u64) as usize,
        sigsetsize: 4,
    };
    let rc = unsafe {
        kernel_sys_pselect6(
            0,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            &zero_ts as *const LinuxTimespec as *const u8,
            &sigarg as *const LinuxPselectSigmaskArg as *const u8,
        )
    };
    if rc != EINVAL {
        print("FAIL\n");
        return -15;
    }
    print("PASS\n");

    print("[test_ppoll] === epoll(20/21/22) Tests ===\n");

    print("[test_ppoll] test: epoll_create1 invalid flags -> EINVAL ... ");
    let rc = unsafe { kernel_sys_epoll_create1(1) };
    if rc != EINVAL {
        print("FAIL\n");
        return -16;
    }
    print("PASS\n");

    let epfd = unsafe { kernel_sys_epoll_create1(0) };
    if epfd < 0 {
        print("[test_ppoll] test: epoll_create1(0) ... FAIL\n");
        return -17;
    }
    let epfd = epfd as i32;

    print("[test_ppoll] test: epoll_ctl ADD + epoll_pwait ready ... ");
    let add_evt = encode_epoll_event(EPOLLOUT, 0x1122_3344_5566_7788u64);
    let rc = unsafe { kernel_sys_epoll_ctl(epfd, EPOLL_CTL_ADD, 1, add_evt.as_ptr()) };
    if rc != 0 {
        close_fd(epfd);
        print("FAIL\n");
        return -18;
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
        close_fd(epfd);
        print("FAIL\n");
        return -19;
    }
    print("PASS\n");

    print("[test_ppoll] test: epoll_ctl MOD update data/events ... ");
    let mod_evt = encode_epoll_event(EPOLLIN, 0xAABB_CCDD_EEFF_0011u64);
    let rc = unsafe { kernel_sys_epoll_ctl(epfd, EPOLL_CTL_MOD, 1, mod_evt.as_ptr()) };
    if rc != 0 {
        close_fd(epfd);
        print("FAIL\n");
        return -20;
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
        close_fd(epfd);
        print("FAIL\n");
        return -21;
    }
    print("PASS\n");

    print("[test_ppoll] test: epoll_ctl DEL + timeout0 -> 0 ... ");
    let rc = unsafe { kernel_sys_epoll_ctl(epfd, EPOLL_CTL_DEL, 1, core::ptr::null()) };
    if rc != 0 {
        close_fd(epfd);
        print("FAIL\n");
        return -22;
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
        close_fd(epfd);
        print("FAIL\n");
        return -23;
    }
    print("PASS\n");

    print("[test_ppoll] test: epoll_pwait sigmask size validation -> EINVAL ... ");
    let rc = unsafe {
        kernel_sys_epoll_pwait(
            epfd,
            events.as_mut_ptr().cast::<u8>(),
            events.len() as i32,
            0,
            &mut sigmask_dummy as *mut u64 as *const u8,
            4,
        )
    };
    if rc != EINVAL {
        close_fd(epfd);
        print("FAIL\n");
        return -24;
    }
    print("PASS\n");

    print("[test_ppoll] test: epollet edge semantics on pipe ... ");
    let mut edge_pipe = [0i32; 2];
    let rc = unsafe { kernel_sys_pipe2(edge_pipe.as_mut_ptr(), 0) };
    if rc != 0 {
        close_fd(epfd);
        print("FAIL\n");
        return -25;
    }
    let edge_evt = encode_epoll_event(EPOLLIN | EPOLLET, 0xE1E2_E3E4_E5E6_E7E8u64);
    let rc = unsafe { kernel_sys_epoll_ctl(epfd, EPOLL_CTL_ADD, edge_pipe[0], edge_evt.as_ptr()) };
    if rc != 0 {
        close_fd(edge_pipe[0]);
        close_fd(edge_pipe[1]);
        close_fd(epfd);
        print("FAIL\n");
        return -26;
    }

    events = [[0u8; EPOLL_EVENT_SIZE]; 4];
    let rc0 = unsafe {
        kernel_sys_epoll_pwait(
            epfd,
            events.as_mut_ptr().cast::<u8>(),
            events.len() as i32,
            0,
            core::ptr::null(),
            0,
        )
    };
    let payload = [0x11u8];
    let wr1 = unsafe { kernel_sys_write(edge_pipe[1], payload.as_ptr(), payload.len()) };
    let rc1 = unsafe {
        kernel_sys_epoll_pwait(
            epfd,
            events.as_mut_ptr().cast::<u8>(),
            events.len() as i32,
            0,
            core::ptr::null(),
            0,
        )
    };
    let (edge_mask1, edge_data1) = decode_epoll_event(&events[0]);
    let rc2 = unsafe {
        kernel_sys_epoll_pwait(
            epfd,
            events.as_mut_ptr().cast::<u8>(),
            events.len() as i32,
            0,
            core::ptr::null(),
            0,
        )
    };
    let mut edge_buf = [0u8; 1];
    let rd1 = unsafe { kernel_sys_read(edge_pipe[0], edge_buf.as_mut_ptr(), edge_buf.len()) };
    let wr2 = unsafe { kernel_sys_write(edge_pipe[1], payload.as_ptr(), payload.len()) };
    let rc3 = unsafe {
        kernel_sys_epoll_pwait(
            epfd,
            events.as_mut_ptr().cast::<u8>(),
            events.len() as i32,
            0,
            core::ptr::null(),
            0,
        )
    };
    let del_edge = unsafe { kernel_sys_epoll_ctl(epfd, EPOLL_CTL_DEL, edge_pipe[0], core::ptr::null()) };
    close_fd(edge_pipe[0]);
    close_fd(edge_pipe[1]);
    if rc0 != 0
        || wr1 != 1
        || rc1 != 1
        || (edge_mask1 & EPOLLIN) == 0
        || edge_data1 != 0xE1E2_E3E4_E5E6_E7E8u64
        || rc2 != 0
        || rd1 != 1
        || wr2 != 1
        || rc3 != 1
        || del_edge != 0
    {
        close_fd(epfd);
        print("FAIL\n");
        return -27;
    }
    print("PASS\n");

    print("[test_ppoll] test: epoll oneshot rearm semantics ... ");
    let mut oneshot_pipe = [0i32; 2];
    let rc = unsafe { kernel_sys_pipe2(oneshot_pipe.as_mut_ptr(), 0) };
    if rc != 0 {
        close_fd(epfd);
        print("FAIL\n");
        return -28;
    }
    let add_oneshot = encode_epoll_event(EPOLLIN | EPOLLONESHOT, 0x0102_0304_0506_0708u64);
    let rc = unsafe { kernel_sys_epoll_ctl(epfd, EPOLL_CTL_ADD, oneshot_pipe[0], add_oneshot.as_ptr()) };
    if rc != 0 {
        close_fd(oneshot_pipe[0]);
        close_fd(oneshot_pipe[1]);
        close_fd(epfd);
        print("FAIL\n");
        return -29;
    }

    let wr = unsafe { kernel_sys_write(oneshot_pipe[1], payload.as_ptr(), payload.len()) };
    events = [[0u8; EPOLL_EVENT_SIZE]; 4];
    let rc_first = unsafe {
        kernel_sys_epoll_pwait(
            epfd,
            events.as_mut_ptr().cast::<u8>(),
            events.len() as i32,
            0,
            core::ptr::null(),
            0,
        )
    };
    let (oneshot_mask1, oneshot_data1) = decode_epoll_event(&events[0]);
    let rc_second = unsafe {
        kernel_sys_epoll_pwait(
            epfd,
            events.as_mut_ptr().cast::<u8>(),
            events.len() as i32,
            0,
            core::ptr::null(),
            0,
        )
    };
    let mod_oneshot = encode_epoll_event(EPOLLIN | EPOLLONESHOT, 0x1020_3040_5060_7080u64);
    let mod_rc = unsafe { kernel_sys_epoll_ctl(epfd, EPOLL_CTL_MOD, oneshot_pipe[0], mod_oneshot.as_ptr()) };
    let rc_third = unsafe {
        kernel_sys_epoll_pwait(
            epfd,
            events.as_mut_ptr().cast::<u8>(),
            events.len() as i32,
            0,
            core::ptr::null(),
            0,
        )
    };
    let (oneshot_mask2, oneshot_data2) = decode_epoll_event(&events[0]);
    let del_oneshot = unsafe { kernel_sys_epoll_ctl(epfd, EPOLL_CTL_DEL, oneshot_pipe[0], core::ptr::null()) };
    close_fd(oneshot_pipe[0]);
    close_fd(oneshot_pipe[1]);
    if wr != 1
        || rc_first != 1
        || (oneshot_mask1 & EPOLLIN) == 0
        || oneshot_data1 != 0x0102_0304_0506_0708u64
        || rc_second != 0
        || mod_rc != 0
        || rc_third != 1
        || (oneshot_mask2 & EPOLLIN) == 0
        || oneshot_data2 != 0x1020_3040_5060_7080u64
        || del_oneshot != 0
    {
        close_fd(epfd);
        print("FAIL\n");
        return -30;
    }
    print("PASS\n");

    close_fd(epfd);

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
    b"0.2.0\0".as_ptr()
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    print("[test_ppoll] PANIC!\n");
    loop {}
}
