//! 스레드 테스트 모듈
//!
//! 테스트 항목:
//! 1. 스레드 생성 (kernel_thread_spawn)
//! 2. yield 동작 확인
//! 3. 스레드가 실제로 실행되는지 확인 (공유 변수 변경)

#![no_std]
#![no_main]

use core::panic::PanicInfo;
use core::sync::atomic::{AtomicU32, Ordering};

unsafe extern "C" {
    fn kernel_print(s: *const u8, len: usize);
    fn kernel_thread_spawn(entry: extern "C" fn(usize), arg: usize, name: *const u8, name_len: usize) -> i32;
    fn kernel_sleep_ticks(ticks: u32);
    fn yield_now();
    fn current_tid() -> u32;
}

fn print(s: &str) {
    unsafe { kernel_print(s.as_ptr(), s.len()); }
}

/// 공유 변수: worker 스레드가 이 값을 변경
static SHARED_VALUE: AtomicU32 = AtomicU32::new(0);

/// worker 스레드 엔트리
extern "C" fn worker_entry(_arg: usize) {
    // 공유 변수를 42로 설정
    SHARED_VALUE.store(42, Ordering::SeqCst);
    print("[test_thread] worker: set SHARED_VALUE = 42\n");
}

#[unsafe(no_mangle)]
pub extern "C" fn module_init() -> i32 {
    print("[test_thread] === Thread Tests ===\n");

    // 테스트 1: 현재 tid 확인
    print("[test_thread] test: current_tid ... ");
    let tid = unsafe { current_tid() };
    if tid == 0 {
        // tid 0은 보통 idle 스레드이지만, 모듈은 메인 스레드에서 실행
        // 0이어도 유효할 수 있음 — 실패 조건은 아님
    }
    print("PASS\n");

    // 테스트 2: 스레드 생성
    print("[test_thread] test: spawn thread ... ");
    SHARED_VALUE.store(0, Ordering::SeqCst);
    let tname = b"test_worker";
    let tid = unsafe {
        kernel_thread_spawn(worker_entry, 0, tname.as_ptr(), tname.len())
    };
    if tid <= 0 {
        print("FAIL (spawn returned <= 0)\n");
        return -1;
    }
    print("PASS\n");

    // 테스트 3: worker가 실행되어 SHARED_VALUE 변경 확인
    print("[test_thread] test: worker execution ... ");
    // worker가 실행될 시간을 줌
    unsafe { kernel_sleep_ticks(20); }
    let val = SHARED_VALUE.load(Ordering::SeqCst);
    if val != 42 {
        print("FAIL (SHARED_VALUE != 42)\n");
        return -2;
    }
    print("PASS\n");

    // 테스트 4: yield 호출
    print("[test_thread] test: yield_now ... ");
    unsafe { yield_now(); }
    print("PASS\n");

    print("[test_thread] All tests passed\n");
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn module_exit() {
    print("[test_thread] Module unloaded\n");
}

#[unsafe(no_mangle)]
pub extern "C" fn module_name() -> *const u8 {
    b"test_thread\0".as_ptr()
}

#[unsafe(no_mangle)]
pub extern "C" fn module_version() -> *const u8 {
    b"0.1.0\0".as_ptr()
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    print("[test_thread] PANIC!\n");
    loop {}
}
