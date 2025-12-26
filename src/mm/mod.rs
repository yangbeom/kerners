//! Memory Management
//!
//! 메모리 관리 모듈
//! - 페이지 프레임 할당자
//! - 힙 할당자
//! - MMU 설정 (추후 구현)

pub mod page;
pub mod heap;

use crate::kprintln;

/// 메모리 영역 정보
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct MemoryRegion {
    pub base: usize,
    pub size: usize,
}

/// 커널 메모리 레이아웃
#[derive(Debug)]
pub struct KernelMemoryLayout {
    /// 커널 코드/데이터 시작 (링커 스크립트 기준)
    pub kernel_start: usize,
    /// 커널 끝 (_end 심볼)
    pub kernel_end: usize,
    /// RAM 시작 주소
    pub ram_start: usize,
    /// RAM 크기
    pub ram_size: usize,
    /// 힙 시작 주소
    pub heap_start: usize,
    /// 힙 크기
    pub heap_size: usize,
    /// 페이지 프레임 할당 영역 시작
    pub frame_alloc_start: usize,
    /// 페이지 프레임 할당 영역 크기
    pub frame_alloc_size: usize,
}

impl KernelMemoryLayout {
    pub fn dump(&self) {
        kprintln!("[MM] Kernel Memory Layout:");
        kprintln!("  Kernel:      {:#x} - {:#x} ({} KB)", 
            self.kernel_start, self.kernel_end, 
            (self.kernel_end - self.kernel_start) / 1024);
        kprintln!("  RAM:         {:#x} - {:#x} ({} MB)", 
            self.ram_start, self.ram_start + self.ram_size,
            self.ram_size / (1024 * 1024));
        kprintln!("  Heap:        {:#x} - {:#x} ({} MB)", 
            self.heap_start, self.heap_start + self.heap_size,
            self.heap_size / (1024 * 1024));
        kprintln!("  Frame Pool:  {:#x} - {:#x} ({} MB)", 
            self.frame_alloc_start, self.frame_alloc_start + self.frame_alloc_size,
            self.frame_alloc_size / (1024 * 1024));
    }
}

/// 메모리 관리 시스템 초기화
/// 
/// # Arguments
/// * `ram_start` - RAM 시작 주소 (DTB에서 획득)
/// * `ram_size` - RAM 크기 (DTB에서 획득)
/// 
/// # Returns
/// 초기화된 메모리 레이아웃 정보
pub fn init(ram_start: usize, ram_size: usize) -> Result<KernelMemoryLayout, &'static str> {
    kprintln!("[MM] Initializing memory management...");
    
    // 커널 심볼 주소
    unsafe extern "C" {
        static _end: u8;
    }
    
    // 아키텍처별 커널 시작 주소
    #[cfg(target_arch = "aarch64")]
    const KERNEL_START: usize = 0x4008_0000;
    
    #[cfg(target_arch = "riscv64")]
    const KERNEL_START: usize = 0x8020_0000;
    
    let kernel_end = unsafe { &_end as *const u8 as usize };
    
    // 페이지 정렬 (4KB)
    let kernel_end_aligned = (kernel_end + 0xFFF) & !0xFFF;
    
    // 힙 크기 설정: RAM의 1/4 또는 최대 128MB
    let max_heap_size = 128 * 1024 * 1024; // 128MB
    let heap_size = core::cmp::min(ram_size / 4, max_heap_size);
    let heap_start = kernel_end_aligned;
    let heap_end = heap_start + heap_size;
    
    // 페이지 프레임 할당 영역: 힙 이후 ~ RAM 끝 (DTB 영역 제외)
    // DTB는 RAM 끝에서 2MB 전에 위치하므로 4MB 여유 확보
    let frame_alloc_start = (heap_end + 0xFFF) & !0xFFF;
    let ram_end = ram_start + ram_size;
    let reserved_at_end = 4 * 1024 * 1024; // 4MB 예약 (DTB 등)
    let frame_alloc_end = if ram_end > reserved_at_end {
        ram_end - reserved_at_end
    } else {
        ram_end
    };
    
    let frame_alloc_size = if frame_alloc_end > frame_alloc_start {
        frame_alloc_end - frame_alloc_start
    } else {
        0
    };
    
    let layout = KernelMemoryLayout {
        kernel_start: KERNEL_START,
        kernel_end,
        ram_start,
        ram_size,
        heap_start,
        heap_size,
        frame_alloc_start,
        frame_alloc_size,
    };
    
    layout.dump();
    
    // 힙 초기화
    heap::init(heap_start, heap_size)?;
    
    // 페이지 프레임 할당자 초기화
    if frame_alloc_size > 0 {
        page::init(frame_alloc_start, frame_alloc_size)?;
    } else {
        kprintln!("[MM] Warning: No memory available for page frame allocator");
    }
    
    kprintln!("[MM] Memory management initialized successfully");
    
    Ok(layout)
}
