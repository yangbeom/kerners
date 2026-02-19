//! sigaltstack(132) syscall 회귀 테스트 모듈

#![no_std]
#![no_main]

use core::panic::PanicInfo;

unsafe extern "C" {
    fn kernel_print(s: *const u8, len: usize);
    fn kernel_sys_sigaltstack(ss: *const u8, old_ss: *mut u8) -> i64;
}

const EFAULT: i64 = -14;
const EINVAL: i64 = -22;
const ENOMEM: i64 = -12;

const SS_DISABLE: i32 = 0x2;

#[repr(C)]
#[derive(Clone, Copy)]
struct LinuxSigAltStack {
    ss_sp: usize,
    ss_flags: i32,
    _pad: i32,
    ss_size: usize,
}

#[repr(align(16))]
struct AltStack([u8; 4096]);

static mut ALT_STACK: AltStack = AltStack([0; 4096]);

fn print(s: &str) {
    unsafe {
        kernel_print(s.as_ptr(), s.len());
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn module_init() -> i32 {
    print("[test_sigaltstack] === sigaltstack(132) Tests ===\n");

    print("[test_sigaltstack] test: default state is disabled ... ");
    let mut old = LinuxSigAltStack {
        ss_sp: 0,
        ss_flags: 0,
        _pad: 0,
        ss_size: 0,
    };
    let rc = unsafe {
        kernel_sys_sigaltstack(core::ptr::null(), &mut old as *mut LinuxSigAltStack as *mut u8)
    };
    if rc != 0 || old.ss_flags != SS_DISABLE || old.ss_sp != 0 || old.ss_size != 0 {
        print("FAIL\n");
        return -1;
    }
    print("PASS\n");

    print("[test_sigaltstack] test: invalid pointer validation ... ");
    let bad_old = unsafe { kernel_sys_sigaltstack(core::ptr::null(), 1usize as *mut u8) };
    let bad_ss = unsafe { kernel_sys_sigaltstack(1usize as *const u8, core::ptr::null_mut()) };
    if bad_old != EFAULT || bad_ss != EFAULT {
        print("FAIL\n");
        return -2;
    }
    print("PASS\n");

    print("[test_sigaltstack] test: invalid flags -> EINVAL ... ");
    let stack_ptr = unsafe { core::ptr::addr_of_mut!(ALT_STACK.0) as usize };
    let invalid_flags = LinuxSigAltStack {
        ss_sp: stack_ptr,
        ss_flags: 0x4,
        _pad: 0,
        ss_size: 4096,
    };
    let rc =
        unsafe { kernel_sys_sigaltstack(&invalid_flags as *const LinuxSigAltStack as *const u8, core::ptr::null_mut()) };
    if rc != EINVAL {
        print("FAIL\n");
        return -3;
    }
    print("PASS\n");

    print("[test_sigaltstack] test: size too small -> ENOMEM ... ");
    let too_small = LinuxSigAltStack {
        ss_sp: stack_ptr,
        ss_flags: 0,
        _pad: 0,
        ss_size: 1024,
    };
    let rc =
        unsafe { kernel_sys_sigaltstack(&too_small as *const LinuxSigAltStack as *const u8, core::ptr::null_mut()) };
    if rc != ENOMEM {
        print("FAIL\n");
        return -4;
    }
    print("PASS\n");

    print("[test_sigaltstack] test: set valid alt stack ... ");
    let valid = LinuxSigAltStack {
        ss_sp: stack_ptr,
        ss_flags: 0,
        _pad: 0,
        ss_size: 4096,
    };
    let rc = unsafe {
        kernel_sys_sigaltstack(
            &valid as *const LinuxSigAltStack as *const u8,
            core::ptr::null_mut(),
        )
    };
    if rc != 0 {
        print("FAIL\n");
        return -5;
    }
    print("PASS\n");

    print("[test_sigaltstack] test: query after set ... ");
    old = LinuxSigAltStack {
        ss_sp: 0,
        ss_flags: SS_DISABLE,
        _pad: 0,
        ss_size: 0,
    };
    let rc = unsafe {
        kernel_sys_sigaltstack(core::ptr::null(), &mut old as *mut LinuxSigAltStack as *mut u8)
    };
    if rc != 0 || old.ss_flags != 0 || old.ss_sp != stack_ptr || old.ss_size != 4096 {
        print("FAIL\n");
        return -6;
    }
    print("PASS\n");

    print("[test_sigaltstack] test: disable alt stack ... ");
    let disable = LinuxSigAltStack {
        ss_sp: 0,
        ss_flags: SS_DISABLE,
        _pad: 0,
        ss_size: 0,
    };
    let rc = unsafe {
        kernel_sys_sigaltstack(
            &disable as *const LinuxSigAltStack as *const u8,
            core::ptr::null_mut(),
        )
    };
    if rc != 0 {
        print("FAIL\n");
        return -7;
    }
    old = LinuxSigAltStack {
        ss_sp: 1,
        ss_flags: 0,
        _pad: 0,
        ss_size: 1,
    };
    let rc = unsafe {
        kernel_sys_sigaltstack(core::ptr::null(), &mut old as *mut LinuxSigAltStack as *mut u8)
    };
    if rc != 0 || old.ss_flags != SS_DISABLE || old.ss_sp != 0 || old.ss_size != 0 {
        print("FAIL\n");
        return -8;
    }
    print("PASS\n");

    print("[test_sigaltstack] All tests passed\n");
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn module_exit() {
    print("[test_sigaltstack] Module unloaded\n");
}

#[unsafe(no_mangle)]
pub extern "C" fn module_name() -> *const u8 {
    b"test_sigaltstack\0".as_ptr()
}

#[unsafe(no_mangle)]
pub extern "C" fn module_version() -> *const u8 {
    b"0.1.0\0".as_ptr()
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    print("[test_sigaltstack] PANIC!\n");
    loop {}
}
