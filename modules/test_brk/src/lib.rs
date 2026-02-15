//! brk syscall 고도화 테스트 모듈

#![no_std]
#![no_main]

use core::panic::PanicInfo;

unsafe extern "C" {
    fn kernel_print(s: *const u8, len: usize);
    fn kernel_sys_brk(addr: usize) -> i64;
}

const PAGE_SIZE: usize = 4096;

fn print(s: &str) {
    unsafe {
        kernel_print(s.as_ptr(), s.len());
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn module_init() -> i32 {
    print("[test_brk] === brk syscall Tests ===\n");

    print("[test_brk] test: grow pages ... ");
    let brk0 = unsafe { kernel_sys_brk(0) };
    if brk0 <= 0 {
        print("FAIL (initial)\n");
        return -1;
    }

    let grow_target = brk0 as usize + PAGE_SIZE * 3;
    let brk1 = unsafe { kernel_sys_brk(grow_target) };
    if brk1 < grow_target as i64 {
        print("FAIL (grow)\n");
        return -2;
    }

    unsafe {
        let p0 = brk0 as *mut u8;
        let p1 = (brk0 as usize + PAGE_SIZE) as *mut u8;
        let p2 = (brk0 as usize + PAGE_SIZE * 2) as *mut u8;

        p0.write_volatile(0x11);
        p1.write_volatile(0x22);
        p2.write_volatile(0x33);

        if p0.read_volatile() != 0x11 || p1.read_volatile() != 0x22 || p2.read_volatile() != 0x33 {
            print("FAIL (rw)\n");
            return -3;
        }
    }
    print("PASS\n");

    print("[test_brk] test: shrink + keep current ... ");
    let shrink_target = brk0 as usize + PAGE_SIZE;
    let brk2 = unsafe { kernel_sys_brk(shrink_target) };
    if brk2 != shrink_target as i64 {
        print("FAIL (shrink)\n");
        return -4;
    }

    unsafe {
        let p0 = brk0 as *mut u8;
        p0.write_volatile(0x44);
        if p0.read_volatile() != 0x44 {
            print("FAIL (rw-after-shrink)\n");
            return -5;
        }
    }

    let invalid = unsafe { kernel_sys_brk(usize::MAX) };
    if invalid != brk2 {
        print("FAIL (invalid-range)\n");
        return -6;
    }
    print("PASS\n");

    print("[test_brk] test: shrink to baseline ... ");
    let brk3 = unsafe { kernel_sys_brk(brk0 as usize) };
    if brk3 != brk0 {
        print("FAIL\n");
        return -7;
    }
    print("PASS\n");

    print("[test_brk] All tests passed\n");
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn module_exit() {
    print("[test_brk] Module unloaded\n");
}

#[unsafe(no_mangle)]
pub extern "C" fn module_name() -> *const u8 {
    b"test_brk\0".as_ptr()
}

#[unsafe(no_mangle)]
pub extern "C" fn module_version() -> *const u8 {
    b"0.1.0\0".as_ptr()
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    print("[test_brk] PANIC!\n");
    loop {}
}
