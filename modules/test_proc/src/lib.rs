//! 프로세스 syscall(10-1B 1순위/2순위 일부) 테스트 모듈
//!
//! 테스트 항목:
//! 1. getpid/gettid/getppid
//! 2. brk 기본 동작
//! 3. mmap/munmap (anonymous/private)

#![no_std]
#![no_main]

use core::panic::PanicInfo;

unsafe extern "C" {
    fn kernel_print(s: *const u8, len: usize);
    fn kernel_sys_getpid() -> i64;
    fn kernel_sys_getppid() -> i64;
    fn kernel_sys_gettid() -> i64;
    fn kernel_sys_brk(addr: usize) -> i64;
    fn kernel_sys_mmap(
        addr: usize,
        len: usize,
        prot: usize,
        flags: usize,
        fd: i64,
        offset: usize,
    ) -> i64;
    fn kernel_sys_munmap(addr: usize, len: usize) -> i64;
    fn kernel_sys_rt_sigprocmask(
        how: i32,
        set: *const u8,
        oldset: *mut u8,
        sigsetsize: usize,
    ) -> i64;
    fn kernel_sys_rt_sigaction(
        signum: i32,
        act: *const u8,
        oldact: *mut u8,
        sigsetsize: usize,
    ) -> i64;
    fn kernel_sys_rt_sigtimedwait(
        set: *const u8,
        info: *mut u8,
        timeout: *const u8,
        sigsetsize: usize,
    ) -> i64;
    fn kernel_sys_kill(pid: isize, sig: i32) -> i64;
    fn kernel_sys_tkill(tid: isize, sig: i32) -> i64;
    fn kernel_sys_tgkill(tgid: isize, tid: isize, sig: i32) -> i64;
    fn kernel_sys_wait4(pid: isize, status: *mut i32, options: i32) -> i64;
    fn kernel_sys_fork() -> i64;
    fn kernel_sys_vfork() -> i64;
    fn kernel_test_enqueue_signal(signum: u32) -> i64;
}

const PROT_READ: usize = 0x1;
const PROT_WRITE: usize = 0x2;
const MAP_PRIVATE: usize = 0x02;
const MAP_ANONYMOUS: usize = 0x20;
const PAGE_SIZE: usize = 4096;
const EBADF: i64 = -9;
const EAGAIN: i64 = -11;
const ECHILD: i64 = -10;
const ESRCH: i64 = -3;
const EINVAL: i64 = -22;
const SIGCHLD: u32 = 17;
const SIG_SETMASK: i32 = 2;
const WNOHANG: i32 = 0x1;
const SIGSET_SIZE: usize = core::mem::size_of::<u64>();

#[repr(C)]
#[derive(Clone, Copy)]
struct LinuxSigAction {
    sa_handler: u64,
    sa_flags: u64,
    sa_restorer: u64,
    sa_mask: u64,
}

fn print(s: &str) {
    unsafe { kernel_print(s.as_ptr(), s.len()) };
}

#[unsafe(no_mangle)]
pub extern "C" fn module_init() -> i32 {
    print("[test_proc] === Process Syscall Tests ===\n");

    // 테스트 1: getpid/gettid/getppid
    print("[test_proc] test: getpid/gettid/getppid ... ");
    let pid = unsafe { kernel_sys_getpid() };
    let tid = unsafe { kernel_sys_gettid() };
    let ppid = unsafe { kernel_sys_getppid() };
    if pid < 0 || tid < 0 || pid != tid || ppid != 0 {
        print("FAIL\n");
        return -1;
    }
    print("PASS\n");

    // 테스트 2: brk 증가/감소
    print("[test_proc] test: brk grow/shrink ... ");
    let brk0 = unsafe { kernel_sys_brk(0) };
    if brk0 <= 0 {
        print("FAIL (initial)\n");
        return -2;
    }
    let brk1 = unsafe { kernel_sys_brk(brk0 as usize + PAGE_SIZE) };
    if brk1 < brk0 + PAGE_SIZE as i64 {
        print("FAIL (grow)\n");
        return -3;
    }
    let brk2 = unsafe { kernel_sys_brk(brk0 as usize) };
    if brk2 != brk0 {
        print("FAIL (shrink)\n");
        return -4;
    }
    print("PASS\n");

    // 테스트 3: mmap/munmap 정상 경로
    print("[test_proc] test: mmap/munmap anonymous ... ");
    let mapped = unsafe {
        kernel_sys_mmap(
            0,
            PAGE_SIZE * 2,
            PROT_READ | PROT_WRITE,
            MAP_PRIVATE | MAP_ANONYMOUS,
            -1,
            0,
        )
    };
    if mapped <= 0 || (mapped as usize) % PAGE_SIZE != 0 {
        print("FAIL (mmap)\n");
        return -5;
    }

    unsafe {
        let p = mapped as *mut u8;
        p.write_volatile(0x5A);
        p.add(PAGE_SIZE).write_volatile(0xA5);
        if p.read_volatile() != 0x5A || p.add(PAGE_SIZE).read_volatile() != 0xA5 {
            print("FAIL (rw)\n");
            return -6;
        }
    }

    let unmap_ok = unsafe { kernel_sys_munmap(mapped as usize, PAGE_SIZE * 2) };
    if unmap_ok != 0 {
        print("FAIL (munmap)\n");
        return -7;
    }
    print("PASS\n");

    // 테스트 4: file-backed mmap 동작 확인
    print("[test_proc] test: mmap file-backed mode ... ");
    let file_backed = unsafe {
        kernel_sys_mmap(0, PAGE_SIZE, PROT_READ | PROT_WRITE, MAP_PRIVATE, -1, 0)
    };
    if file_backed != EBADF {
        print("FAIL\n");
        return -8;
    }
    print("PASS\n");

    // 테스트 5: munmap 부분 해제
    print("[test_proc] test: munmap partial ... ");
    let mapped2 = unsafe {
        kernel_sys_mmap(
            0,
            PAGE_SIZE * 2,
            PROT_READ | PROT_WRITE,
            MAP_PRIVATE | MAP_ANONYMOUS,
            -1,
            0,
        )
    };
    if mapped2 <= 0 {
        print("FAIL (setup)\n");
        return -9;
    }

    let unmap_head = unsafe { kernel_sys_munmap(mapped2 as usize, PAGE_SIZE) };
    if unmap_head != 0 {
        print("FAIL (head)\n");
        return -10;
    }

    unsafe {
        let tail = (mapped2 as usize + PAGE_SIZE) as *mut u8;
        tail.write_volatile(0xCC);
        if tail.read_volatile() != 0xCC {
            print("FAIL (tail-rw)\n");
            return -11;
        }
    }

    let final_unmap = unsafe { kernel_sys_munmap(mapped2 as usize + PAGE_SIZE, PAGE_SIZE) };
    if final_unmap != 0 {
        print("FAIL (cleanup)\n");
        return -12;
    }
    print("PASS\n");

    // 테스트 6: signal queue + rt_sigtimedwait
    print("[test_proc] test: signal queue/sigtimedwait ... ");
    let mut old_mask: u64 = 0;
    let wait_set: u64 = 1u64 << (SIGCHLD - 1);
    let mut siginfo: [u8; 16] = [0; 16];
    let setmask_ok = unsafe {
        kernel_sys_rt_sigprocmask(
            SIG_SETMASK,
            &wait_set as *const u64 as *const u8,
            &mut old_mask as *mut u64 as *mut u8,
            SIGSET_SIZE,
        )
    };
    if setmask_ok != 0 {
        print("FAIL (setmask)\n");
        return -12;
    }

    let enqueue = unsafe { kernel_test_enqueue_signal(SIGCHLD) };
    if enqueue != 0 {
        print("FAIL (enqueue)\n");
        return -13;
    }

    let got = unsafe {
        kernel_sys_rt_sigtimedwait(
            &wait_set as *const u64 as *const u8,
            siginfo.as_mut_ptr(),
            core::ptr::null(),
            SIGSET_SIZE,
        )
    };
    if got != SIGCHLD as i64 {
        print("FAIL (wait1)\n");
        return -14;
    }

    let empty = unsafe {
        kernel_sys_rt_sigtimedwait(
            &wait_set as *const u64 as *const u8,
            siginfo.as_mut_ptr(),
            core::ptr::null(),
            SIGSET_SIZE,
        )
    };
    if empty != EAGAIN {
        print("FAIL (wait2)\n");
        return -15;
    }

    // 원래 마스크 복원
    let restore = unsafe {
        kernel_sys_rt_sigprocmask(
            SIG_SETMASK,
            &old_mask as *const u64 as *const u8,
            core::ptr::null_mut(),
            SIGSET_SIZE,
        )
    };
    if restore != 0 {
        print("FAIL (restore)\n");
        return -16;
    }
    print("PASS\n");

    // 테스트 7: rt_sigaction set/get roundtrip
    print("[test_proc] test: rt_sigaction set/get ... ");
    let new_action = LinuxSigAction {
        sa_handler: 0x1000,
        sa_flags: 0x1234_0000,
        sa_restorer: 0x2000,
        sa_mask: wait_set,
    };
    let mut old_action = LinuxSigAction {
        sa_handler: 0,
        sa_flags: 0,
        sa_restorer: 0,
        sa_mask: 0,
    };
    let set_action = unsafe {
        kernel_sys_rt_sigaction(
            SIGCHLD as i32,
            &new_action as *const _ as *const u8,
            &mut old_action as *mut _ as *mut u8,
            SIGSET_SIZE,
        )
    };
    if set_action != 0 {
        print("FAIL (set)\n");
        return -17;
    }

    let mut got_action = LinuxSigAction {
        sa_handler: 0,
        sa_flags: 0,
        sa_restorer: 0,
        sa_mask: 0,
    };
    let get_action = unsafe {
        kernel_sys_rt_sigaction(
            SIGCHLD as i32,
            core::ptr::null(),
            &mut got_action as *mut _ as *mut u8,
            SIGSET_SIZE,
        )
    };
    if get_action != 0 || got_action.sa_handler != new_action.sa_handler || got_action.sa_restorer != new_action.sa_restorer || got_action.sa_mask != new_action.sa_mask {
        print("FAIL (get)\n");
        return -18;
    }
    print("PASS\n");

    // 테스트 8: kill/tkill/tgkill 기본/에러 경로
    print("[test_proc] test: kill/tkill/tgkill ... ");
    let me = unsafe { kernel_sys_gettid() };
    if me < 0 {
        print("FAIL (tid)\n");
        return -19;
    }
    if unsafe { kernel_sys_kill(me as isize, 0) } != 0
        || unsafe { kernel_sys_tkill(me as isize, 0) } != 0
        || unsafe { kernel_sys_tgkill(me as isize, me as isize, 0) } != 0
    {
        print("FAIL (probe)\n");
        return -20;
    }
    if unsafe { kernel_sys_kill(999_999, 0) } != ESRCH
        || unsafe { kernel_sys_tkill(999_999, 0) } != ESRCH
        || unsafe { kernel_sys_tgkill(me as isize, (me + 1) as isize, 0) } != ESRCH
        || unsafe { kernel_sys_kill(me as isize, 99) } != EINVAL
    {
        print("FAIL (errno)\n");
        return -21;
    }

    let mut siginfo2: [u8; 16] = [0; 16];
    let sent = unsafe { kernel_sys_tkill(me as isize, SIGCHLD as i32) };
    let recv = unsafe {
        kernel_sys_rt_sigtimedwait(
            &wait_set as *const u64 as *const u8,
            siginfo2.as_mut_ptr(),
            core::ptr::null(),
            SIGSET_SIZE,
        )
    };
    if sent != 0 || (recv != SIGCHLD as i64 && recv != EAGAIN) {
        print("FAIL (delivery)\n");
        return -22;
    }
    print("PASS\n");

    // 테스트 9: fork + wait4
    print("[test_proc] test: fork/wait4 ... ");
    let child = unsafe { kernel_sys_fork() };
    if child <= 0 {
        print("FAIL (fork)\n");
        return -23;
    }
    let mut wait_status: i32 = -1;
    let waited = unsafe { kernel_sys_wait4(child as isize, &mut wait_status as *mut i32, 0) };
    if waited != child {
        print("FAIL (wait)\n");
        return -24;
    }
    if wait_status != 0 {
        print("FAIL (status)\n");
        return -25;
    }
    let no_child = unsafe { kernel_sys_wait4(child as isize, &mut wait_status as *mut i32, WNOHANG) };
    if no_child != ECHILD {
        print("FAIL (wnohang)\n");
        return -26;
    }
    print("PASS\n");

    // 테스트 10: vfork + wait4
    print("[test_proc] test: vfork/wait4 ... ");
    let vchild = unsafe { kernel_sys_vfork() };
    if vchild <= 0 {
        print("FAIL (vfork)\n");
        return -27;
    }
    wait_status = -1;
    let vwaited = unsafe { kernel_sys_wait4(vchild as isize, &mut wait_status as *mut i32, 0) };
    if vwaited != vchild || wait_status != 0 {
        print("FAIL\n");
        return -28;
    }
    print("PASS\n");

    print("[test_proc] All tests passed\n");
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn module_exit() {
    print("[test_proc] Module unloaded\n");
}

#[unsafe(no_mangle)]
pub extern "C" fn module_name() -> *const u8 {
    b"test_proc\0".as_ptr()
}

#[unsafe(no_mangle)]
pub extern "C" fn module_version() -> *const u8 {
    b"0.1.0\0".as_ptr()
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    print("[test_proc] PANIC!\n");
    loop {}
}
