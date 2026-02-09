//! 블록 디바이스 테스트 모듈
//!
//! 테스트 항목:
//! 1. RamDisk 생성
//! 2. 블록 쓰기/읽기 정합성
//! 3. 다른 블록에 쓰기/읽기

#![no_std]
#![no_main]

use core::panic::PanicInfo;

unsafe extern "C" {
    fn kernel_print(s: *const u8, len: usize);
    fn kernel_ramdisk_create(name: *const u8, name_len: usize, size: usize) -> i32;
    fn kernel_block_read(name: *const u8, name_len: usize, block_idx: usize, buf: *mut u8, buf_len: usize) -> i32;
    fn kernel_block_write(name: *const u8, name_len: usize, block_idx: usize, data: *const u8, data_len: usize) -> i32;
}

fn print(s: &str) {
    unsafe { kernel_print(s.as_ptr(), s.len()); }
}

#[unsafe(no_mangle)]
pub extern "C" fn module_init() -> i32 {
    print("[test_block] === Block Device Tests ===\n");

    let dname = b"test_ramdisk";

    // 테스트 1: RamDisk 생성 (4096바이트 = 8블록 × 512바이트)
    print("[test_block] test: ramdisk create ... ");
    let ret = unsafe { kernel_ramdisk_create(dname.as_ptr(), dname.len(), 4096) };
    if ret != 0 {
        print("FAIL\n");
        return -1;
    }
    print("PASS\n");

    // 테스트 2: 블록 0에 쓰기 → 읽기 → 비교
    print("[test_block] test: write/read block 0 ... ");
    let mut write_buf = [0u8; 512];
    for i in 0..512 {
        write_buf[i] = (i & 0xFF) as u8;
    }
    let ret = unsafe { kernel_block_write(dname.as_ptr(), dname.len(), 0, write_buf.as_ptr(), 512) };
    if ret != 512 {
        print("FAIL (write)\n");
        return -2;
    }
    let mut read_buf = [0u8; 512];
    let ret = unsafe { kernel_block_read(dname.as_ptr(), dname.len(), 0, read_buf.as_mut_ptr(), 512) };
    if ret != 512 {
        print("FAIL (read)\n");
        return -3;
    }
    for i in 0..512 {
        if read_buf[i] != write_buf[i] {
            print("FAIL (data mismatch)\n");
            return -4;
        }
    }
    print("PASS\n");

    // 테스트 3: 다른 블록에 쓰기 → 블록 0이 변경되지 않음 확인
    print("[test_block] test: block isolation ... ");
    let other_buf = [0xFFu8; 512];
    let ret = unsafe { kernel_block_write(dname.as_ptr(), dname.len(), 1, other_buf.as_ptr(), 512) };
    if ret != 512 {
        print("FAIL (write block 1)\n");
        return -5;
    }
    // 블록 0 다시 읽기 — 원래 데이터 유지 확인
    let mut verify_buf = [0u8; 512];
    let ret = unsafe { kernel_block_read(dname.as_ptr(), dname.len(), 0, verify_buf.as_mut_ptr(), 512) };
    if ret != 512 {
        print("FAIL (re-read block 0)\n");
        return -6;
    }
    for i in 0..512 {
        if verify_buf[i] != write_buf[i] {
            print("FAIL (block 0 corrupted)\n");
            return -7;
        }
    }
    print("PASS\n");

    print("[test_block] All tests passed\n");
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn module_exit() {
    print("[test_block] Module unloaded\n");
}

#[unsafe(no_mangle)]
pub extern "C" fn module_name() -> *const u8 {
    b"test_block\0".as_ptr()
}

#[unsafe(no_mangle)]
pub extern "C" fn module_version() -> *const u8 {
    b"0.1.0\0".as_ptr()
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    print("[test_block] PANIC!\n");
    loop {}
}
