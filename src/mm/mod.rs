//! Memory Management
//!
//! 메모리 관리 모듈
//! - 페이지 프레임 할당자
//! - 힙 할당자
//! - MMU 설정 (추후 구현)

pub mod page;
pub mod heap;

use crate::kprintln;
use crate::sync::RwLock;

/// 메모리 영역 정보
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct MemoryRegion {
    pub base: usize,
    pub size: usize,
}

/// 커널 메모리 레이아웃
#[derive(Debug, Clone, Copy)]
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

static KERNEL_LAYOUT: RwLock<Option<KernelMemoryLayout>> = RwLock::new(None);

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
        static _stack_start: u8;
    }
    
    // 아키텍처별 커널 시작 주소
    #[cfg(target_arch = "aarch64")]
    const KERNEL_START: usize = 0x4008_0000;
    
    #[cfg(target_arch = "riscv64")]
    const KERNEL_START: usize = 0x8020_0000;
    
    let kernel_end = unsafe { &_end as *const u8 as usize };
    let boot_stack_top = unsafe { &_stack_start as *const u8 as usize };
    
    // 페이지 정렬 (4KB)
    let kernel_end_aligned = (kernel_end + 0xFFF) & !0xFFF;
    
    // 부트스트랩 스택은 [_end, _stack_start) 구간을 사용하므로 힙에서 제외한다.
    let heap_start = (boot_stack_top + 0xFFF) & !0xFFF;
    if heap_start <= kernel_end_aligned {
        return Err("Invalid memory layout: heap overlaps bootstrap stack");
    }

    let reserved_boot_stack = heap_start - kernel_end_aligned;
    kprintln!(
        "[MM] Reserved bootstrap stack area: {:#x} - {:#x} ({} KB)",
        kernel_end_aligned,
        heap_start,
        reserved_boot_stack / 1024
    );

    // 힙 크기 설정: RAM의 1/4 또는 최대 128MB (RAM 끝을 넘지 않도록 제한)
    let max_heap_size = 128 * 1024 * 1024; // 128MB
    let ram_end = ram_start + ram_size;
    let available_for_heap = ram_end.saturating_sub(heap_start);
    let heap_size = core::cmp::min(core::cmp::min(ram_size / 4, max_heap_size), available_for_heap);
    let heap_end = heap_start + heap_size;
    
    // 페이지 프레임 할당 영역: 힙 이후 ~ RAM 끝 (DTB 영역 제외)
    // DTB는 RAM 끝에서 2MB 전에 위치하므로 4MB 여유 확보
    let mut frame_alloc_start = (heap_end + 0xFFF) & !0xFFF;
    let mut frame_alloc_end = ram_end;

    // DTB blob의 실제 위치/크기를 반영해 프레임 풀을 조정한다.
    if let Some((dtb_base, dtb_size)) = crate::dtb::blob_range() {
        let dtb_start = dtb_base & !0xFFF;
        let dtb_end = (dtb_base.saturating_add(dtb_size).saturating_add(0xFFF)) & !0xFFF;

        // DTB가 현재 RAM/프레임 풀 후보 구간에 걸쳐 있으면 풀 범위를 잘라낸다.
        if dtb_start < ram_end && dtb_end > ram_start {
            if dtb_start <= frame_alloc_start && dtb_end > frame_alloc_start {
                frame_alloc_start = core::cmp::min(dtb_end, ram_end);
                kprintln!(
                    "[MM] DTB reservation overlap at frame start: [{:#x}, {:#x})",
                    dtb_start,
                    dtb_end
                );
            } else if dtb_start > frame_alloc_start && dtb_start < frame_alloc_end {
                frame_alloc_end = dtb_start;
                kprintln!(
                    "[MM] DTB reservation trims frame pool end: [{:#x}, {:#x})",
                    dtb_start,
                    dtb_end
                );
            } else {
                kprintln!(
                    "[MM] DTB blob in RAM: [{:#x}, {:#x}) (no frame pool overlap)",
                    dtb_start,
                    dtb_end
                );
            }
        }
    } else {
        // DTB 정보를 얻지 못한 경우에는 기존 보수 정책을 폴백으로 유지한다.
        let reserved_at_end = 4 * 1024 * 1024;
        frame_alloc_end = frame_alloc_end.saturating_sub(reserved_at_end);
        kprintln!(
            "[MM] DTB range unavailable, reserving fallback tail region: {} MB",
            reserved_at_end / (1024 * 1024)
        );
    }
    
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

    {
        let mut guard = KERNEL_LAYOUT.write();
        *guard = Some(layout);
    }
    
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

/// 런타임 커널 메모리 레이아웃 조회
pub fn layout() -> Option<KernelMemoryLayout> {
    *KERNEL_LAYOUT.read()
}

/// 런타임 RAM 범위 조회
pub fn ram_range() -> Option<(usize, usize)> {
    layout().map(|l| (l.ram_start, l.ram_start.saturating_add(l.ram_size)))
}

/// 주소가 런타임 RAM/커널 매핑 범위 안인지 확인
pub fn is_kernel_mapped_addr(addr: usize) -> bool {
    let Some(layout) = layout() else {
        return false;
    };

    let ram_end = layout.ram_start.saturating_add(layout.ram_size);
    if addr >= layout.ram_start && addr < ram_end {
        return true;
    }

    #[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
    {
        let kernel_virt_base = crate::arch::mmu::KERNEL_VIRT_BASE;
        let kernel_virt_end = kernel_virt_base.saturating_add(layout.ram_size);
        if addr >= kernel_virt_base && addr < kernel_virt_end {
            return true;
        }
    }

    false
}
