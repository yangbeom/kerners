//! Hello World 테스트 모듈
//!
//! 커널 모듈 시스템 테스트를 위한 간단한 모듈입니다.
//! PLT를 통해 커널의 extern 함수를 호출합니다.

#![no_std]
#![no_main]

use core::panic::PanicInfo;

// 커널에서 제공하는 extern 함수 선언
unsafe extern "C" {
    /// 커널 출력 함수 (PLT를 통해 호출됨)
    fn kernel_print(s: *const u8, len: usize);
}

/// 안전한 문자열 출력 헬퍼
fn print(s: &str) {
    unsafe {
        kernel_print(s.as_ptr(), s.len());
    }
}

/// 모듈 초기화 함수
#[unsafe(no_mangle)]
pub extern "C" fn module_init() -> i32 {
    print("[hello_module] Module initialized!\n");
    print("[hello_module] Hello from the loadable module!\n");
    print("[hello_module] Using PLT for kernel function calls!\n");
    0 // 성공
}

/// 모듈 정리 함수
#[unsafe(no_mangle)]
pub extern "C" fn module_exit() {
    print("[hello_module] Module unloading, goodbye!\n");
}

/// 모듈 이름 반환
#[unsafe(no_mangle)]
pub extern "C" fn module_name() -> *const u8 {
    b"hello_module\0".as_ptr()
}

/// 모듈 버전 반환
#[unsafe(no_mangle)]
pub extern "C" fn module_version() -> *const u8 {
    b"0.1.0\0".as_ptr()
}

/// 테스트용 함수 - 모듈에서 export하는 심볼
#[unsafe(no_mangle)]
pub extern "C" fn hello_add(a: i32, b: i32) -> i32 {
    a + b
}

/// Panic 핸들러
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
