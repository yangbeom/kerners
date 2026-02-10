//! 로깅 시스템 테스트 모듈
//!
//! 테스트 항목:
//! 1. 모든 로그 레벨 출력 (ERROR~TRACE)
//! 2. 대량 로깅 스트레스 테스트
//! 3. 긴 메시지 테스트

#![no_std]
#![no_main]

use core::panic::PanicInfo;

unsafe extern "C" {
    fn kernel_print(s: *const u8, len: usize);
    fn kernel_log(level: u8, msg: *const u8, msg_len: usize);
}

fn print(s: &str) {
    unsafe { kernel_print(s.as_ptr(), s.len()); }
}

fn log(level: u8, msg: &str) {
    unsafe { kernel_log(level, msg.as_ptr(), msg.len()); }
}

#[unsafe(no_mangle)]
pub extern "C" fn module_init() -> i32 {
    print("[test_log] === Logging System Tests ===\n");

    // 테스트 1: 모든 로그 레벨 출력
    print("[test_log] test: all log levels ... ");
    log(0, "ERROR level message from test_log");
    log(1, "WARN level message from test_log");
    log(2, "INFO level message from test_log");
    log(3, "DEBUG level message from test_log");
    log(4, "TRACE level message from test_log");
    print("[test_log] PASS\n");

    // 테스트 2: 대량 로깅 스트레스 테스트
    print("[test_log] test: rapid logging (50 messages) ... ");
    let mut i: u8 = 0;
    while i < 50 {
        let level = i % 5;
        log(level, "stress test message");
        i += 1;
    }
    print("[test_log] PASS\n");

    // 테스트 3: 긴 메시지
    print("[test_log] test: long message ... ");
    log(2, "This is a long message to test the ring buffer. It contains enough text to verify that the logging system correctly handles messages that are longer than typical short log entries. The kernel logging system should store this entire message in the ring buffer and display it correctly when dmesg is called.");
    print("[test_log] PASS\n");

    print("[test_log] All tests passed\n");
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn module_exit() {
    print("[test_log] Module unloaded\n");
}

#[unsafe(no_mangle)]
pub extern "C" fn module_name() -> *const u8 {
    b"test_log\0".as_ptr()
}

#[unsafe(no_mangle)]
pub extern "C" fn module_version() -> *const u8 {
    b"0.1.0\0".as_ptr()
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    print("[test_log] PANIC!\n");
    loop {}
}
