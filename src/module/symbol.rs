//! 커널 심볼 테이블
//!
//! 모듈이 커널 함수를 호출할 수 있도록 심볼 테이블 관리
//! - 정적 커널 심볼 (런타임 초기화)
//! - 동적 심볼 등록/해제

use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::sync::RwLock;

/// 커널 심볼 정보
#[derive(Debug, Clone)]
pub struct KernelSymbol {
    /// 심볼 이름
    pub name: String,
    /// 심볼 주소
    pub address: usize,
}

impl KernelSymbol {
    pub fn new(name: &str, address: usize) -> Self {
        Self {
            name: String::from(name),
            address,
        }
    }
}

/// 심볼 테이블 (동적 + 정적 통합) - Vec으로 변경하여 BTreeMap 문제 회피
static SYMBOLS: RwLock<Option<Vec<(String, usize)>>> = RwLock::new(None);
static INITIALIZED: AtomicBool = AtomicBool::new(false);

/// 모듈에서 사용할 수 있는 커널 출력 함수
/// 모듈은 이 심볼을 extern "C"로 참조하여 커널과 통신
#[unsafe(no_mangle)]
pub extern "C" fn kernel_print(s: *const u8, len: usize) {
    if s.is_null() || len == 0 {
        return;
    }
    let slice = unsafe { core::slice::from_raw_parts(s, len) };
    if let Ok(msg) = core::str::from_utf8(slice) {
        crate::console::puts(msg);
    }
}

/// 심볼 테이블 초기화
pub fn init() {
    if INITIALIZED.swap(true, Ordering::SeqCst) {
        return; // 이미 초기화됨
    }

    let mut symbols = SYMBOLS.write();
    let mut list = Vec::new();

    // 커널 심볼 등록
    list.push((String::from("console_puts"), crate::console::puts as usize));
    list.push((String::from("console_putc"), crate::console::putc as usize));
    list.push((String::from("alloc_frame"), crate::mm::page::alloc_frame as usize));
    list.push((String::from("yield_now"), crate::proc::yield_now as usize));
    list.push((String::from("current_tid"), crate::proc::current_tid as usize));
    // 모듈용 출력 함수
    list.push((String::from("kernel_print"), kernel_print as usize));

    *symbols = Some(list);

    crate::kprintln!("[symbol] Kernel symbol table initialized");

    // 테스트 모듈용 심볼 등록
    drop(symbols); // 쓰기 락 해제 후 등록 (register_symbol이 다시 잠금)
    super::test_symbols::register_test_symbols();
}

/// 심볼 조회
pub fn lookup_symbol(name: &str) -> Option<usize> {
    // 초기화 확인
    if !INITIALIZED.load(Ordering::SeqCst) {
        init();
    }

    let symbols = SYMBOLS.read();
    if let Some(ref list) = *symbols {
        return list.iter().find(|(n, _)| n == name).map(|(_, addr)| *addr);
    }
    None
}

/// 동적 심볼 등록
pub fn register_symbol(name: &str, address: usize) {
    // 초기화 확인
    if !INITIALIZED.load(Ordering::SeqCst) {
        init();
    }

    let mut symbols = SYMBOLS.write();
    if let Some(ref mut list) = *symbols {
        // 기존 항목이 있으면 교체
        if let Some(pos) = list.iter().position(|(n, _)| n == name) {
            list[pos] = (String::from(name), address);
        } else {
            list.push((String::from(name), address));
        }
    }
}

/// 동적 심볼 해제
pub fn unregister_symbol(name: &str) -> bool {
    let mut symbols = SYMBOLS.write();
    if let Some(ref mut list) = *symbols {
        if let Some(pos) = list.iter().position(|(n, _)| n == name) {
            list.remove(pos);
            return true;
        }
    }
    false
}

/// 등록된 심볼 목록 반환
pub fn list_symbols() -> Vec<(String, usize)> {
    // 초기화 확인
    if !INITIALIZED.load(Ordering::SeqCst) {
        init();
    }

    let symbols = SYMBOLS.read();
    if let Some(ref list) = *symbols {
        list.clone()
    } else {
        Vec::new()
    }
}

/// 심볼 개수
pub fn symbol_count() -> usize {
    // 초기화 확인
    if !INITIALIZED.load(Ordering::SeqCst) {
        init();
    }

    let symbols = SYMBOLS.read();
    symbols.as_ref().map(|m| m.len()).unwrap_or(0)
}
