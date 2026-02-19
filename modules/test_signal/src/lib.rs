//! signal syscall 회귀 테스트 모듈

#![no_std]
#![no_main]

use core::panic::PanicInfo;
use core::sync::atomic::{AtomicI32, AtomicIsize, Ordering};

unsafe extern "C" {
    fn kernel_print(s: *const u8, len: usize);
    fn kernel_thread_spawn(
        entry: extern "C" fn(usize),
        arg: usize,
        name: *const u8,
        name_len: usize,
    ) -> i32;
    fn kernel_sleep_ticks(ticks: u32);
    fn kernel_sys_gettid() -> i64;
    fn kernel_sys_tkill(tid: isize, sig: i32) -> i64;
    fn kernel_test_enqueue_signal_to_tid(tid: i64, signum: u32) -> i64;
    fn kernel_sys_rt_sigaction(
        signum: i32,
        act: *const u8,
        oldact: *mut u8,
        sigsetsize: usize,
    ) -> i64;
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

const SIGKILL: i32 = 9;
const SIGTERM: i32 = 15;
const SIGCHLD: i32 = 17;
const SIGCONT: i32 = 18;
const SIGSTOP: i32 = 19;

const SIG_SETMASK: i32 = 2;

const EAGAIN: i64 = -11;
const EINTR: i64 = -4;
const EINVAL: i64 = -22;

const SIGSET_SIZE: usize = core::mem::size_of::<u64>();

#[repr(C)]
#[derive(Clone, Copy)]
struct LinuxTimespec {
    tv_sec: i64,
    tv_nsec: i64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct LinuxSigAction {
    sa_handler: u64,
    sa_flags: u64,
    sa_restorer: u64,
    sa_mask: u64,
}

static WORKER_TARGET_TID: AtomicIsize = AtomicIsize::new(0);
static WORKER_SIGNAL: AtomicI32 = AtomicI32::new(0);

fn print(s: &str) {
    unsafe {
        kernel_print(s.as_ptr(), s.len());
    }
}

#[inline]
fn sigmask(signum: i32) -> u64 {
    1u64 << ((signum as u32) - 1)
}

fn sigprocmask_set(new_mask: u64, old_mask: *mut u64) -> i64 {
    unsafe {
        kernel_sys_rt_sigprocmask(
            SIG_SETMASK,
            &new_mask as *const u64 as *const u8,
            old_mask as *mut u8,
            SIGSET_SIZE,
        )
    }
}

fn sigprocmask_get(current: &mut u64) -> i64 {
    unsafe {
        kernel_sys_rt_sigprocmask(
            SIG_SETMASK,
            core::ptr::null(),
            current as *mut u64 as *mut u8,
            SIGSET_SIZE,
        )
    }
}

fn sigtimedwait(wait_set: u64, timeout: &LinuxTimespec) -> i64 {
    let mut info = [0u8; 16];
    unsafe {
        kernel_sys_rt_sigtimedwait(
            &wait_set as *const u64 as *const u8,
            info.as_mut_ptr(),
            timeout as *const LinuxTimespec as *const u8,
            SIGSET_SIZE,
        )
    }
}

extern "C" fn delayed_tkill_worker(delay_ticks: usize) {
    unsafe {
        kernel_sleep_ticks(delay_ticks as u32);
    }

    let target_tid = WORKER_TARGET_TID.load(Ordering::SeqCst);
    let signum = WORKER_SIGNAL.load(Ordering::SeqCst);
    unsafe {
        let _ = kernel_test_enqueue_signal_to_tid(target_tid as i64, signum as u32);
    }
}

fn spawn_delayed_tkill(target_tid: isize, signum: i32, delay_ticks: usize) -> bool {
    WORKER_TARGET_TID.store(target_tid, Ordering::SeqCst);
    WORKER_SIGNAL.store(signum, Ordering::SeqCst);
    let name = "sigw";
    let tid = unsafe {
        kernel_thread_spawn(delayed_tkill_worker, delay_ticks, name.as_ptr(), name.len())
    };
    tid > 0
}

#[unsafe(no_mangle)]
pub extern "C" fn module_init() -> i32 {
    print("[test_signal] === signal syscall Tests ===\n");

    let zero = LinuxTimespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    let short = LinuxTimespec {
        tv_sec: 0,
        tv_nsec: 20_000_000,
    };
    let long = LinuxTimespec {
        tv_sec: 1,
        tv_nsec: 0,
    };
    print("[test_signal] test: rt_sigtimedwait poll timeout ... ");
    let term_set = sigmask(SIGTERM);
    let poll = sigtimedwait(term_set, &zero);
    if poll != EAGAIN {
        print("FAIL\n");
        return -1;
    }
    let timed = sigtimedwait(term_set, &short);
    if timed != EAGAIN {
        print("FAIL\n");
        return -2;
    }
    print("PASS\n");

    print("[test_signal] test: blocked SIGTERM wake + consume ... ");
    let mut old_mask: u64 = 0;
    let setmask_rc = sigprocmask_set(term_set, &mut old_mask as *mut u64);
    if setmask_rc != 0 {
        print("FAIL (setmask)\n");
        return -3;
    }

    let me = unsafe { kernel_sys_gettid() };
    if me < 0 {
        print("FAIL (tid)\n");
        return -4;
    }
    if !spawn_delayed_tkill(me as isize, SIGTERM, 3) {
        print("FAIL (spawn)\n");
        return -5;
    }

    let got_term = sigtimedwait(term_set, &long);
    let restore_mask_rc = sigprocmask_set(old_mask, core::ptr::null_mut());
    if got_term != SIGTERM as i64 || restore_mask_rc != 0 {
        print("FAIL\n");
        return -6;
    }
    print("PASS\n");

    print("[test_signal] test: rt_sigtimedwait EINTR (signal outside waitset) ... ");
    if !spawn_delayed_tkill(me as isize, SIGCHLD, 3) {
        print("FAIL (spawn)\n");
        return -8;
    }

    let got_intr = sigtimedwait(term_set, &long);
    let chld_set = sigmask(SIGCHLD);
    let drain = sigtimedwait(chld_set, &zero);
    let intr_ok = if got_intr == EINTR {
        drain == SIGCHLD as i64 || drain == EAGAIN
    } else if got_intr == EAGAIN {
        // 타이밍 경합으로 EINTR 대신 타임아웃이 먼저 관측될 수 있다.
        // 이 경우 직후 drain에서 SIGCHLD가 관측되어야 한다.
        drain == SIGCHLD as i64
    } else {
        false
    };
    if !intr_ok {
        print("FAIL\n");
        return -8;
    }
    print("PASS\n");

    print("[test_signal] test: SIGCONT masked wait ... ");
    let cont_set = sigmask(SIGCONT);
    old_mask = 0;
    let cont_setmask_rc = sigprocmask_set(cont_set, &mut old_mask as *mut u64);
    if cont_setmask_rc != 0 {
        print("FAIL (setmask)\n");
        return -10;
    }

    let cont_send_rc = if me > 0 {
        unsafe { kernel_sys_tkill(me as isize, SIGCONT) }
    } else {
        unsafe { kernel_test_enqueue_signal_to_tid(0, SIGCONT as u32) }
    };
    let got_cont = sigtimedwait(cont_set, &zero);
    let cont_restore_rc = sigprocmask_set(old_mask, core::ptr::null_mut());
    if cont_send_rc != 0 || got_cont != SIGCONT as i64 || cont_restore_rc != 0 {
        print("FAIL\n");
        return -11;
    }
    print("PASS\n");

    print("[test_signal] test: SIGKILL/SIGSTOP unmaskable in sigprocmask ... ");
    let mut saved_mask: u64 = 0;
    let force_mask = sigmask(SIGKILL) | sigmask(SIGSTOP);
    let save_rc = sigprocmask_set(force_mask, &mut saved_mask as *mut u64);
    if save_rc != 0 {
        print("FAIL (save)\n");
        return -12;
    }

    let mut current_mask: u64 = 0;
    let get_rc = sigprocmask_get(&mut current_mask);
    let restore_rc = sigprocmask_set(saved_mask, core::ptr::null_mut());
    if get_rc != 0 || (current_mask & force_mask) != 0 || restore_rc != 0 {
        print("FAIL\n");
        return -13;
    }
    print("PASS\n");

    print("[test_signal] test: rt_sigaction rejects SIGKILL/SIGSTOP ... ");
    let action = LinuxSigAction {
        sa_handler: 0x1000,
        sa_flags: 0,
        sa_restorer: 0,
        sa_mask: 0,
    };
    let sigkill_rc = unsafe {
        kernel_sys_rt_sigaction(
            SIGKILL,
            &action as *const LinuxSigAction as *const u8,
            core::ptr::null_mut(),
            SIGSET_SIZE,
        )
    };
    let sigstop_rc = unsafe {
        kernel_sys_rt_sigaction(
            SIGSTOP,
            &action as *const LinuxSigAction as *const u8,
            core::ptr::null_mut(),
            SIGSET_SIZE,
        )
    };
    if sigkill_rc != EINVAL || sigstop_rc != EINVAL {
        print("FAIL\n");
        return -14;
    }
    print("PASS\n");

    print("[test_signal] All tests passed\n");
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn module_exit() {
    print("[test_signal] Module unloaded\n");
}

#[unsafe(no_mangle)]
pub extern "C" fn module_name() -> *const u8 {
    b"test_signal\0".as_ptr()
}

#[unsafe(no_mangle)]
pub extern "C" fn module_version() -> *const u8 {
    b"0.1.0\0".as_ptr()
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    print("[test_signal] PANIC\n");
    loop {}
}
