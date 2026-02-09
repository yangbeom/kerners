//! 메시지 큐 테스트 모듈
//!
//! 테스트 항목:
//! 1. MQ 생성/열기
//! 2. 메시지 전송/수신 (FIFO)
//! 3. 빈 큐에서 수신 실패 확인

#![no_std]
#![no_main]

use core::panic::PanicInfo;

unsafe extern "C" {
    fn kernel_print(s: *const u8, len: usize);
    fn kernel_mq_open(name: *const u8, name_len: usize, create: bool) -> i32;
    fn kernel_mq_send(name: *const u8, name_len: usize, data: *const u8, data_len: usize) -> i32;
    fn kernel_mq_receive(name: *const u8, name_len: usize, buf: *mut u8, buf_len: usize) -> i32;
}

fn print(s: &str) {
    unsafe { kernel_print(s.as_ptr(), s.len()); }
}

#[unsafe(no_mangle)]
pub extern "C" fn module_init() -> i32 {
    print("[test_ipc] === Message Queue Tests ===\n");

    let qname = b"test_mq";

    // 테스트 1: MQ 생성
    print("[test_ipc] test: mq create ... ");
    let ret = unsafe { kernel_mq_open(qname.as_ptr(), qname.len(), true) };
    if ret != 0 {
        print("FAIL\n");
        return -1;
    }
    print("PASS\n");

    // 테스트 2: 메시지 전송
    print("[test_ipc] test: mq send ... ");
    let msg = b"hello_ipc";
    let ret = unsafe { kernel_mq_send(qname.as_ptr(), qname.len(), msg.as_ptr(), msg.len()) };
    if ret != 0 {
        print("FAIL\n");
        return -2;
    }
    print("PASS\n");

    // 테스트 3: 메시지 수신
    print("[test_ipc] test: mq receive ... ");
    let mut buf = [0u8; 256];
    let received = unsafe { kernel_mq_receive(qname.as_ptr(), qname.len(), buf.as_mut_ptr(), buf.len()) };
    if received != msg.len() as i32 {
        print("FAIL (wrong length)\n");
        return -3;
    }
    // 내용 비교
    let mut match_ok = true;
    for i in 0..msg.len() {
        if buf[i] != msg[i] {
            match_ok = false;
            break;
        }
    }
    if !match_ok {
        print("FAIL (content mismatch)\n");
        return -4;
    }
    print("PASS\n");

    // 테스트 4: 빈 큐에서 수신 → 실패 확인
    print("[test_ipc] test: mq receive empty ... ");
    let ret = unsafe { kernel_mq_receive(qname.as_ptr(), qname.len(), buf.as_mut_ptr(), buf.len()) };
    if ret != -1 {
        print("FAIL (should return -1)\n");
        return -5;
    }
    print("PASS\n");

    print("[test_ipc] All tests passed\n");
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn module_exit() {
    print("[test_ipc] Module unloaded\n");
}

#[unsafe(no_mangle)]
pub extern "C" fn module_name() -> *const u8 {
    b"test_ipc\0".as_ptr()
}

#[unsafe(no_mangle)]
pub extern "C" fn module_version() -> *const u8 {
    b"0.1.0\0".as_ptr()
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    print("[test_ipc] PANIC!\n");
    loop {}
}
