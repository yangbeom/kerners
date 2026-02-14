//! execve 준비 경로 테스트 모듈
//!
//! 테스트 항목:
//! 1. 존재하지 않는 파일 → ENOENT(-2)
//! 2. ELF가 아닌 파일 → ENOEXEC(-8)

#![no_std]
#![no_main]

use core::panic::PanicInfo;

unsafe extern "C" {
    fn kernel_print(s: *const u8, len: usize);
    fn kernel_vfs_create_file(path: *const u8, path_len: usize) -> i32;
    fn kernel_vfs_write(
        path: *const u8,
        path_len: usize,
        offset: usize,
        data: *const u8,
        data_len: usize,
    ) -> i32;
    fn kernel_vfs_unlink(path: *const u8, path_len: usize) -> i32;
    fn kernel_exec_prepare(path: *const u8, path_len: usize) -> i32;
}

fn print(s: &str) {
    unsafe { kernel_print(s.as_ptr(), s.len()); }
}

#[unsafe(no_mangle)]
pub extern "C" fn module_init() -> i32 {
    print("[test_execve] === execve Prepare Tests ===\n");

    // 테스트 1: 존재하지 않는 경로
    print("[test_execve] test: ENOENT on missing path ... ");
    let missing = b"/no/such/binary";
    let ret = unsafe { kernel_exec_prepare(missing.as_ptr(), missing.len()) };
    if ret != -2 {
        print("FAIL\n");
        return -1;
    }
    print("PASS\n");

    // 테스트 2: ELF가 아닌 파일
    print("[test_execve] test: ENOEXEC on non-ELF ... ");
    let invalid_path = b"/execve_invalid.bin";
    let payload = b"this is not an elf binary";

    // 이전 테스트 잔여 파일이 있으면 삭제
    let _ = unsafe { kernel_vfs_unlink(invalid_path.as_ptr(), invalid_path.len()) };

    let create_ret = unsafe { kernel_vfs_create_file(invalid_path.as_ptr(), invalid_path.len()) };
    if create_ret != 0 {
        print("FAIL (create)\n");
        return -2;
    }

    let write_ret = unsafe {
        kernel_vfs_write(
            invalid_path.as_ptr(),
            invalid_path.len(),
            0,
            payload.as_ptr(),
            payload.len(),
        )
    };
    if write_ret != payload.len() as i32 {
        print("FAIL (write)\n");
        return -3;
    }

    let exec_ret = unsafe { kernel_exec_prepare(invalid_path.as_ptr(), invalid_path.len()) };
    if exec_ret != -8 {
        print("FAIL\n");
        return -4;
    }
    print("PASS\n");

    let _ = unsafe { kernel_vfs_unlink(invalid_path.as_ptr(), invalid_path.len()) };

    print("[test_execve] All tests passed\n");
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn module_exit() {
    print("[test_execve] Module unloaded\n");
}

#[unsafe(no_mangle)]
pub extern "C" fn module_name() -> *const u8 {
    b"test_execve\0".as_ptr()
}

#[unsafe(no_mangle)]
pub extern "C" fn module_version() -> *const u8 {
    b"0.1.0\0".as_ptr()
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    print("[test_execve] PANIC!\n");
    loop {}
}
