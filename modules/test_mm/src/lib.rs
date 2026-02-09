//! 메모리 관리 테스트 모듈
//!
//! 테스트 항목:
//! 1. 페이지 프레임 할당/해제
//! 2. 힙 메모리 할당/해제
//! 3. 연속 프레임 할당 시 주소 겹침 없음

#![no_std]
#![no_main]

use core::panic::PanicInfo;

unsafe extern "C" {
    fn kernel_print(s: *const u8, len: usize);
    fn alloc_frame() -> usize;
    fn free_frame(addr: usize);
    fn kernel_heap_alloc(size: usize, align: usize) -> usize;
    fn kernel_heap_dealloc(ptr: usize, size: usize, align: usize);
}

fn print(s: &str) {
    unsafe { kernel_print(s.as_ptr(), s.len()); }
}

/// 모듈 초기화 — 테스트 실행
#[unsafe(no_mangle)]
pub extern "C" fn module_init() -> i32 {
    print("[test_mm] === Memory Management Tests ===\n");

    // 테스트 1: 페이지 프레임 할당/해제
    print("[test_mm] test: page alloc/free ... ");
    let frame = unsafe { alloc_frame() };
    if frame == 0 {
        print("FAIL (alloc_frame returned 0)\n");
        return -1;
    }
    unsafe { free_frame(frame); }
    print("PASS\n");

    // 테스트 2: 힙 메모리 할당/해제
    print("[test_mm] test: heap alloc/free ... ");
    let ptr = unsafe { kernel_heap_alloc(1024, 8) };
    if ptr == 0 {
        print("FAIL (kernel_heap_alloc returned 0)\n");
        return -2;
    }
    // 할당된 메모리에 쓰기/읽기 테스트
    unsafe {
        let p = ptr as *mut u8;
        for i in 0..1024 {
            p.add(i).write(0xAB);
        }
        if p.read() != 0xAB {
            print("FAIL (memory read mismatch)\n");
            return -3;
        }
    }
    unsafe { kernel_heap_dealloc(ptr, 1024, 8); }
    print("PASS\n");

    // 테스트 3: 연속 프레임 할당 — 주소 겹침 없음
    print("[test_mm] test: multiple frames no overlap ... ");
    let f1 = unsafe { alloc_frame() };
    let f2 = unsafe { alloc_frame() };
    let f3 = unsafe { alloc_frame() };
    if f1 == 0 || f2 == 0 || f3 == 0 {
        print("FAIL (alloc returned 0)\n");
        return -4;
    }
    if f1 == f2 || f2 == f3 || f1 == f3 {
        print("FAIL (addresses overlap)\n");
        return -5;
    }
    unsafe {
        free_frame(f3);
        free_frame(f2);
        free_frame(f1);
    }
    print("PASS\n");

    print("[test_mm] All tests passed\n");
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn module_exit() {
    print("[test_mm] Module unloaded\n");
}

#[unsafe(no_mangle)]
pub extern "C" fn module_name() -> *const u8 {
    b"test_mm\0".as_ptr()
}

#[unsafe(no_mangle)]
pub extern "C" fn module_version() -> *const u8 {
    b"0.1.0\0".as_ptr()
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    print("[test_mm] PANIC!\n");
    loop {}
}
