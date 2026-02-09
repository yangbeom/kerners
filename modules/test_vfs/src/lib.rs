//! VFS 파일시스템 테스트 모듈
//!
//! 테스트 항목:
//! 1. 디렉토리 생성
//! 2. 파일 생성
//! 3. 파일 쓰기/읽기 정합성
//! 4. 파일 삭제

#![no_std]
#![no_main]

use core::panic::PanicInfo;

unsafe extern "C" {
    fn kernel_print(s: *const u8, len: usize);
    fn kernel_vfs_mkdir(path: *const u8, path_len: usize) -> i32;
    fn kernel_vfs_create_file(path: *const u8, path_len: usize) -> i32;
    fn kernel_vfs_write(path: *const u8, path_len: usize, offset: usize, data: *const u8, data_len: usize) -> i32;
    fn kernel_vfs_read(path: *const u8, path_len: usize, offset: usize, buf: *mut u8, buf_len: usize) -> i32;
    fn kernel_vfs_unlink(path: *const u8, path_len: usize) -> i32;
}

fn print(s: &str) {
    unsafe { kernel_print(s.as_ptr(), s.len()); }
}

#[unsafe(no_mangle)]
pub extern "C" fn module_init() -> i32 {
    print("[test_vfs] === VFS Tests ===\n");

    let dir_path = b"/test_vfs_dir";
    let file_path = b"/test_vfs_dir/hello.txt";
    let test_data = b"Hello from test_vfs module!";

    // 테스트 1: 디렉토리 생성
    print("[test_vfs] test: mkdir ... ");
    let ret = unsafe { kernel_vfs_mkdir(dir_path.as_ptr(), dir_path.len()) };
    if ret != 0 {
        print("FAIL\n");
        return -1;
    }
    print("PASS\n");

    // 테스트 2: 파일 생성
    print("[test_vfs] test: create file ... ");
    let ret = unsafe { kernel_vfs_create_file(file_path.as_ptr(), file_path.len()) };
    if ret != 0 {
        print("FAIL\n");
        return -2;
    }
    print("PASS\n");

    // 테스트 3: 파일 쓰기
    print("[test_vfs] test: write file ... ");
    let written = unsafe {
        kernel_vfs_write(
            file_path.as_ptr(), file_path.len(),
            0,
            test_data.as_ptr(), test_data.len(),
        )
    };
    if written != test_data.len() as i32 {
        print("FAIL\n");
        return -3;
    }
    print("PASS\n");

    // 테스트 4: 파일 읽기 및 비교
    print("[test_vfs] test: read file ... ");
    let mut buf = [0u8; 256];
    let read_len = unsafe {
        kernel_vfs_read(
            file_path.as_ptr(), file_path.len(),
            0,
            buf.as_mut_ptr(), buf.len(),
        )
    };
    if read_len != test_data.len() as i32 {
        print("FAIL (wrong length)\n");
        return -4;
    }
    for i in 0..test_data.len() {
        if buf[i] != test_data[i] {
            print("FAIL (content mismatch)\n");
            return -5;
        }
    }
    print("PASS\n");

    // 테스트 5: 파일 삭제
    print("[test_vfs] test: unlink file ... ");
    let ret = unsafe { kernel_vfs_unlink(file_path.as_ptr(), file_path.len()) };
    if ret != 0 {
        print("FAIL\n");
        return -6;
    }
    // 삭제 후 읽기 시도 → 실패해야 함
    let ret = unsafe {
        kernel_vfs_read(file_path.as_ptr(), file_path.len(), 0, buf.as_mut_ptr(), buf.len())
    };
    if ret != -1 {
        print("FAIL (file still exists after unlink)\n");
        return -7;
    }
    print("PASS\n");

    print("[test_vfs] All tests passed\n");
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn module_exit() {
    print("[test_vfs] Module unloaded\n");
}

#[unsafe(no_mangle)]
pub extern "C" fn module_name() -> *const u8 {
    b"test_vfs\0".as_ptr()
}

#[unsafe(no_mangle)]
pub extern "C" fn module_version() -> *const u8 {
    b"0.1.0\0".as_ptr()
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    print("[test_vfs] PANIC!\n");
    loop {}
}
