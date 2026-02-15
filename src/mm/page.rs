//! Page Frame Allocator
//!
//! 물리 페이지 프레임 할당자 구현
//! 비트맵 기반의 간단한 할당자

use crate::kprintln;
use crate::sync::Mutex;
use alloc::vec::Vec;

/// 페이지 크기: 4KB
pub const PAGE_SIZE: usize = 4096;

/// 페이지 프레임 할당자
/// 비트맵 기반으로 구현
pub struct FrameAllocator {
    /// 관리 영역 시작 주소
    base: usize,
    /// 총 페이지 수
    total_pages: usize,
    /// 비트맵 시작 주소 (관리 영역 내)
    bitmap: *mut u64,
    /// 비트맵 크기 (u64 단위)
    bitmap_len: usize,
    /// 다음 검색 시작 위치 (최적화)
    next_search: usize,
    /// 할당된 페이지 수
    allocated_count: usize,
}

#[derive(Debug, Clone, Copy)]
struct FrameRefCount {
    addr: usize,
    count: usize,
}

unsafe impl Send for FrameAllocator {}
unsafe impl Sync for FrameAllocator {}

impl FrameAllocator {
    /// 새 프레임 할당자 생성
    ///
    /// # Safety
    /// `base`와 `size`는 유효한 메모리 영역이어야 함
    pub const fn new() -> Self {
        Self {
            base: 0,
            total_pages: 0,
            bitmap: core::ptr::null_mut(),
            bitmap_len: 0,
            next_search: 0,
            allocated_count: 0,
        }
    }

    /// 할당자 초기화
    ///
    /// # Arguments
    /// * `base` - 관리할 메모리 영역 시작 주소
    /// * `size` - 관리할 메모리 영역 크기
    pub unsafe fn init(&mut self, base: usize, size: usize) {
        // 페이지 정렬 확인
        assert!(base % PAGE_SIZE == 0, "Base address must be page-aligned");

        let total_pages = size / PAGE_SIZE;

        // 비트맵 크기 계산 (1비트 = 1페이지, u64 = 64페이지)
        let bitmap_bits = total_pages;
        let bitmap_u64s = (bitmap_bits + 63) / 64;
        let bitmap_bytes = bitmap_u64s * 8;
        let bitmap_pages = (bitmap_bytes + PAGE_SIZE - 1) / PAGE_SIZE;

        // 비트맵은 관리 영역 시작에 배치
        let bitmap_ptr = base as *mut u64;

        // 비트맵 초기화 (모든 페이지를 사용 가능으로)
        for i in 0..bitmap_u64s {
            unsafe { bitmap_ptr.add(i).write_volatile(0) };
        }

        // 비트맵 자체가 사용하는 페이지들을 할당됨으로 표시
        for page_idx in 0..bitmap_pages {
            let word_idx = page_idx / 64;
            let bit_idx = page_idx % 64;
            unsafe {
                let word = bitmap_ptr.add(word_idx).read_volatile();
                bitmap_ptr
                    .add(word_idx)
                    .write_volatile(word | (1u64 << bit_idx));
            }
        }

        self.base = base;
        self.total_pages = total_pages;
        self.bitmap = bitmap_ptr;
        self.bitmap_len = bitmap_u64s;
        self.next_search = bitmap_pages; // 비트맵 이후부터 검색 시작
        self.allocated_count = bitmap_pages;

        kprintln!(
            "[PageAlloc] Initialized: {} pages ({} MB), bitmap uses {} pages",
            total_pages,
            (total_pages * PAGE_SIZE) / (1024 * 1024),
            bitmap_pages
        );
    }

    /// 단일 페이지 프레임 할당
    ///
    /// # Returns
    /// 할당된 페이지의 물리 주소, 실패 시 None
    pub fn alloc(&mut self) -> Option<usize> {
        self.alloc_pages(1)
    }

    /// 연속 페이지 프레임 할당
    ///
    /// # Arguments
    /// * `count` - 할당할 페이지 수
    ///
    /// # Returns
    /// 할당된 첫 페이지의 물리 주소, 실패 시 None
    pub fn alloc_pages(&mut self, count: usize) -> Option<usize> {
        if count == 0 {
            return None;
        }

        // First-fit 검색
        let mut start_page = self.next_search;
        let mut consecutive = 0;
        let mut first_page = 0;

        for _ in 0..self.total_pages {
            if start_page >= self.total_pages {
                start_page = 0;
                consecutive = 0;
            }

            if self.is_page_free(start_page) {
                if consecutive == 0 {
                    first_page = start_page;
                }
                consecutive += 1;

                if consecutive == count {
                    // 연속 페이지 찾음, 할당
                    for i in 0..count {
                        self.mark_allocated(first_page + i);
                    }
                    self.allocated_count += count;
                    self.next_search = first_page + count;

                    return Some(self.base + first_page * PAGE_SIZE);
                }
            } else {
                consecutive = 0;
            }

            start_page += 1;
        }

        None
    }

    /// 페이지 프레임 해제
    ///
    /// # Arguments
    /// * `addr` - 해제할 페이지의 물리 주소
    ///
    /// # Safety
    /// `addr`은 이전에 alloc으로 할당된 주소여야 함
    pub unsafe fn free(&mut self, addr: usize) {
        unsafe {
            self.free_pages(addr, 1);
        }
    }

    /// 연속 페이지 프레임 해제
    ///
    /// # Arguments
    /// * `addr` - 해제할 첫 페이지의 물리 주소
    /// * `count` - 해제할 페이지 수
    pub unsafe fn free_pages(&mut self, addr: usize, count: usize) {
        if addr < self.base {
            return;
        }

        let page_idx = (addr - self.base) / PAGE_SIZE;

        for i in 0..count {
            if page_idx + i < self.total_pages {
                self.mark_free(page_idx + i);
                self.allocated_count = self.allocated_count.saturating_sub(1);
            }
        }

        // 다음 검색을 해제된 위치부터 시작
        if page_idx < self.next_search {
            self.next_search = page_idx;
        }
    }

    /// 페이지가 사용 가능한지 확인
    fn is_page_free(&self, page_idx: usize) -> bool {
        if page_idx >= self.total_pages {
            return false;
        }

        let word_idx = page_idx / 64;
        let bit_idx = page_idx % 64;

        unsafe {
            let word = self.bitmap.add(word_idx).read_volatile();
            (word & (1u64 << bit_idx)) == 0
        }
    }

    /// 페이지를 할당됨으로 표시
    fn mark_allocated(&mut self, page_idx: usize) {
        let word_idx = page_idx / 64;
        let bit_idx = page_idx % 64;

        unsafe {
            let word = self.bitmap.add(word_idx).read_volatile();
            self.bitmap
                .add(word_idx)
                .write_volatile(word | (1u64 << bit_idx));
        }
    }

    /// 페이지를 사용 가능으로 표시
    fn mark_free(&mut self, page_idx: usize) {
        let word_idx = page_idx / 64;
        let bit_idx = page_idx % 64;

        unsafe {
            let word = self.bitmap.add(word_idx).read_volatile();
            self.bitmap
                .add(word_idx)
                .write_volatile(word & !(1u64 << bit_idx));
        }
    }

    /// 통계 정보 반환
    pub fn stats(&self) -> FrameAllocatorStats {
        FrameAllocatorStats {
            total_pages: self.total_pages,
            allocated_pages: self.allocated_count,
            free_pages: self.total_pages.saturating_sub(self.allocated_count),
        }
    }
}

/// 프레임 할당자 통계
#[derive(Debug, Clone, Copy)]
pub struct FrameAllocatorStats {
    pub total_pages: usize,
    pub allocated_pages: usize,
    pub free_pages: usize,
}

impl FrameAllocatorStats {
    pub fn dump(&self) {
        kprintln!(
            "[PageAlloc] Stats: total={}, allocated={}, free={} ({} MB free)",
            self.total_pages,
            self.allocated_pages,
            self.free_pages,
            (self.free_pages * PAGE_SIZE) / (1024 * 1024)
        );
    }
}

/// 전역 프레임 할당자
static FRAME_ALLOCATOR: Mutex<FrameAllocator> = Mutex::new(FrameAllocator::new());
/// 프레임별 참조 카운트 (프레임 공유/COW용)
static FRAME_REFS: Mutex<Vec<FrameRefCount>> = Mutex::new(Vec::new());

/// 프레임 할당자 초기화
pub fn init(base: usize, size: usize) -> Result<(), &'static str> {
    if size < PAGE_SIZE * 2 {
        return Err("Not enough memory for frame allocator");
    }

    let mut allocator = FRAME_ALLOCATOR.lock();
    unsafe { allocator.init(base, size) };

    Ok(())
}

/// 단일 페이지 할당
pub fn alloc_frame() -> Option<usize> {
    let addr = FRAME_ALLOCATOR.lock().alloc()?;
    let mut refs = FRAME_REFS.lock();
    refs.push(FrameRefCount { addr, count: 1 });
    Some(addr)
}

/// 연속 페이지 할당
pub fn alloc_frames(count: usize) -> Option<usize> {
    let addr = FRAME_ALLOCATOR.lock().alloc_pages(count)?;
    let mut refs = FRAME_REFS.lock();
    for i in 0..count {
        refs.push(FrameRefCount {
            addr: addr + i * PAGE_SIZE,
            count: 1,
        });
    }
    Some(addr)
}

/// 프레임 참조 카운트 증가
///
/// 이미 알려진 프레임이면 +1, 없으면 새 엔트리(count=1)를 생성한다.
pub fn retain_frame(addr: usize) -> bool {
    if addr % PAGE_SIZE != 0 {
        return false;
    }
    let mut refs = FRAME_REFS.lock();
    if let Some(item) = refs.iter_mut().find(|item| item.addr == addr) {
        item.count = item.count.saturating_add(1);
        return true;
    }
    refs.push(FrameRefCount { addr, count: 1 });
    true
}

/// 프레임 참조 카운트 조회
pub fn frame_refcount(addr: usize) -> usize {
    if addr % PAGE_SIZE != 0 {
        return 0;
    }
    let refs = FRAME_REFS.lock();
    refs.iter()
        .find(|item| item.addr == addr)
        .map(|item| item.count)
        .unwrap_or(0)
}

/// 프레임 참조 카운트 감소
///
/// 참조 카운트가 0이 되면 실제 프레임을 allocator에 반환한다.
pub fn release_frame(addr: usize) -> bool {
    if addr % PAGE_SIZE != 0 {
        return false;
    }

    let mut should_free = false;
    {
        let mut refs = FRAME_REFS.lock();
        if let Some(pos) = refs.iter().position(|item| item.addr == addr) {
            if refs[pos].count <= 1 {
                refs.swap_remove(pos);
                should_free = true;
            } else {
                refs[pos].count -= 1;
            }
        } else {
            return false;
        }
    }

    if should_free {
        unsafe {
            // SAFETY: 참조 카운트가 0이 된 프레임만 실제 allocator로 반환한다.
            FRAME_ALLOCATOR.lock().free(addr);
        }
        true
    } else {
        false
    }
}

/// 단일 페이지 해제
///
/// # Safety
/// 유효한 주소여야 함
pub unsafe fn free_frame(addr: usize) {
    let _ = release_frame(addr);
}

/// 연속 페이지 해제
///
/// # Safety
/// 유효한 주소여야 함
pub unsafe fn free_frames(addr: usize, count: usize) {
    for i in 0..count {
        let _ = release_frame(addr + i * PAGE_SIZE);
    }
}

/// 할당자 통계 반환
pub fn stats() -> FrameAllocatorStats {
    FRAME_ALLOCATOR.lock().stats()
}

/// 할당자 통계 출력
pub fn dump_stats() {
    stats().dump();
}

/// 할당자 통계 출력 (별칭)
pub fn print_stats() {
    dump_stats();
}
