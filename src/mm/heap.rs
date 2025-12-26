//! Heap Allocator
//!
//! linked_list_allocator를 사용한 커널 힙 관리

use crate::kprintln;
use linked_list_allocator::LockedHeap;

/// 전역 힙 할당자
#[global_allocator]
static HEAP_ALLOCATOR: LockedHeap = LockedHeap::empty();

/// 힙 초기화 상태
static mut HEAP_INITIALIZED: bool = false;

/// 힙 시작 주소 (디버깅용)
static mut HEAP_START: usize = 0;

/// 힙 크기 (디버깅용)
static mut HEAP_SIZE: usize = 0;

/// 힙 초기화
///
/// # Arguments
/// * `start` - 힙 시작 주소 (페이지 정렬됨)
/// * `size` - 힙 크기
///
/// # Safety
/// 이 함수는 부팅 시 한 번만 호출되어야 함
pub fn init(start: usize, size: usize) -> Result<(), &'static str> {
    if size == 0 {
        return Err("Heap size cannot be zero");
    }

    // 정렬 확인
    if start % 8 != 0 {
        return Err("Heap start must be 8-byte aligned");
    }

    unsafe {
        if HEAP_INITIALIZED {
            return Err("Heap already initialized");
        }

        HEAP_ALLOCATOR.lock().init(start as *mut u8, size);

        HEAP_INITIALIZED = true;
        HEAP_START = start;
        HEAP_SIZE = size;
    }

    kprintln!(
        "[Heap] Initialized: {:#x} - {:#x} ({} MB)",
        start,
        start + size,
        size / (1024 * 1024)
    );

    Ok(())
}

/// 힙이 초기화되었는지 확인
pub fn is_initialized() -> bool {
    unsafe { HEAP_INITIALIZED }
}

/// 힙 사용량 정보
#[derive(Debug, Clone, Copy)]
pub struct HeapStats {
    #[allow(dead_code)]
    pub start: usize,
    pub size: usize,
    pub used: usize,
    pub free: usize,
}

impl HeapStats {
    pub fn dump(&self) {
        kprintln!(
            "[Heap] Stats: total={} KB, used={} KB, free={} KB",
            self.size / 1024,
            self.used / 1024,
            self.free / 1024
        );
    }
}

/// 힙 통계 반환
pub fn stats() -> HeapStats {
    let allocator = HEAP_ALLOCATOR.lock();
    let free = allocator.free();
    let (start, size) = unsafe { (HEAP_START, HEAP_SIZE) };

    HeapStats {
        start,
        size,
        used: size.saturating_sub(free),
        free,
    }
}

/// 힙 통계 출력
pub fn dump_stats() {
    if is_initialized() {
        stats().dump();
    } else {
        kprintln!("[Heap] Not initialized");
    }
}

/// 힙 통계 출력 (별칭)
pub fn print_stats() {
    dump_stats();
}
