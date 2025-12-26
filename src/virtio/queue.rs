//! VirtIO Virtqueue 구현
//!
//! Split Virtqueue (legacy) 구현
//! - Descriptor Table: 버퍼 정보
//! - Available Ring: 드라이버 → 디바이스
//! - Used Ring: 디바이스 → 드라이버

extern crate alloc;

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{fence, Ordering};
use super::{VirtIOError, VirtIOResult};
use super::mmio::VirtIOMMIO;

/// Descriptor 플래그
pub mod desc_flags {
    /// 다음 Descriptor가 있음
    pub const NEXT: u16 = 1;
    /// 쓰기 전용 버퍼 (디바이스가 씀)
    pub const WRITE: u16 = 2;
    /// 간접 Descriptor
    pub const INDIRECT: u16 = 4;
}

/// Virtqueue Descriptor
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct VirtqDesc {
    /// 버퍼 물리 주소
    pub addr: u64,
    /// 버퍼 길이
    pub len: u32,
    /// 플래그 (NEXT, WRITE, INDIRECT)
    pub flags: u16,
    /// 다음 Descriptor 인덱스
    pub next: u16,
}

/// Available Ring
#[repr(C)]
pub struct VirtqAvail {
    /// 플래그 (보통 0)
    pub flags: u16,
    /// 다음 사용할 인덱스
    pub idx: u16,
    /// Descriptor 인덱스 배열 (가변 크기)
    pub ring: [u16; 0],
}

/// Used Ring Element
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct VirtqUsedElem {
    /// Descriptor 체인 시작 인덱스
    pub id: u32,
    /// 쓰여진 바이트 수
    pub len: u32,
}

/// Used Ring
#[repr(C)]
pub struct VirtqUsed {
    /// 플래그
    pub flags: u16,
    /// 다음 인덱스
    pub idx: u16,
    /// Used element 배열 (가변 크기)
    pub ring: [VirtqUsedElem; 0],
}

/// Virtqueue
pub struct Virtqueue {
    /// Queue 인덱스
    queue_idx: u32,
    /// Queue 크기 (Descriptor 수)
    queue_size: u16,
    /// 다음 사용할 Descriptor 인덱스
    free_head: u16,
    /// 사용 가능한 Descriptor 수
    num_free: u16,
    /// 마지막으로 처리한 Used 인덱스
    last_used_idx: u16,
    /// Descriptor Table 메모리
    desc_table: *mut VirtqDesc,
    /// Available Ring 메모리
    avail_ring: *mut VirtqAvail,
    /// Used Ring 메모리
    used_ring: *mut VirtqUsed,
    /// 할당된 메모리 (해제용)
    _memory: Box<[u8]>,
}

// Safety: Virtqueue는 단일 스레드에서만 사용됨
unsafe impl Send for Virtqueue {}
unsafe impl Sync for Virtqueue {}

/// Legacy Queue 정렬 크기 (4KB)
const LEGACY_QUEUE_ALIGN: usize = 4096;

impl Virtqueue {
    /// 새 Virtqueue 생성 (버전 자동 감지)
    pub fn new(mmio: &VirtIOMMIO, queue_idx: u32) -> VirtIOResult<Self> {
        if mmio.is_legacy() {
            Self::new_legacy(mmio, queue_idx)
        } else {
            Self::new_modern(mmio, queue_idx)
        }
    }

    /// Legacy (v1) Virtqueue 생성
    fn new_legacy(mmio: &VirtIOMMIO, queue_idx: u32) -> VirtIOResult<Self> {
        // Guest 페이지 크기 설정 (legacy 필수)
        mmio.set_guest_page_size(4096);

        // Queue 선택
        mmio.select_queue(queue_idx);

        // Queue 최대 크기 확인
        let max_size = mmio.queue_max_size();
        if max_size == 0 {
            return Err(VirtIOError::QueueSetupFailed);
        }

        // Queue 크기 결정 (최대 256개)
        let queue_size = max_size.min(256) as u16;

        // Legacy 레이아웃 계산 (VirtIO spec 2.6.2):
        // Descriptor Table: 16 bytes × queue_size (aligned to page)
        // Available Ring: 6 + 2 × queue_size (바로 뒤에)
        // Used Ring: 페이지 정렬 후 6 + 8 × queue_size
        let desc_size = 16 * queue_size as usize;
        let avail_size = 6 + 2 * queue_size as usize;
        let used_size = 6 + 8 * queue_size as usize;

        // Legacy에서는 desc와 avail이 연속, used는 페이지 정렬
        let desc_avail_size = desc_size + avail_size;
        let desc_avail_pages = (desc_avail_size + LEGACY_QUEUE_ALIGN - 1) / LEGACY_QUEUE_ALIGN;
        let used_offset = desc_avail_pages * LEGACY_QUEUE_ALIGN;
        let used_pages = (used_size + LEGACY_QUEUE_ALIGN - 1) / LEGACY_QUEUE_ALIGN;
        let total_size = (desc_avail_pages + used_pages) * LEGACY_QUEUE_ALIGN;

        // 메모리 할당 (페이지 정렬)
        let layout = core::alloc::Layout::from_size_align(total_size, LEGACY_QUEUE_ALIGN)
            .map_err(|_| VirtIOError::QueueSetupFailed)?;
        let memory = unsafe {
            let ptr = alloc::alloc::alloc_zeroed(layout);
            if ptr.is_null() {
                return Err(VirtIOError::QueueSetupFailed);
            }
            Box::from_raw(core::slice::from_raw_parts_mut(ptr, total_size))
        };

        let base = memory.as_ptr() as usize;
        let desc_table = base as *mut VirtqDesc;
        let avail_ring = (base + desc_size) as *mut VirtqAvail;
        let used_ring = (base + used_offset) as *mut VirtqUsed;

        // 디버그: Legacy queue setup
        // crate::kprintln!("[VirtIO] Legacy queue setup: base={:#x}, desc={:#x}, avail={:#x}, used={:#x}",
        //     base, base, base + desc_size, base + used_offset);

        // Descriptor 체인 초기화 (free list)
        for i in 0..queue_size {
            unsafe {
                let desc = desc_table.add(i as usize);
                (*desc).next = if i + 1 < queue_size { i + 1 } else { 0 };
            }
        }

        // Legacy MMIO 설정
        mmio.set_queue_size(queue_size as u32);
        mmio.set_queue_align(LEGACY_QUEUE_ALIGN as u32);
        mmio.set_queue_pfn((base / LEGACY_QUEUE_ALIGN) as u32);

        Ok(Self {
            queue_idx,
            queue_size,
            free_head: 0,
            num_free: queue_size,
            last_used_idx: 0,
            desc_table,
            avail_ring,
            used_ring,
            _memory: memory,
        })
    }

    /// Modern (v2) Virtqueue 생성
    fn new_modern(mmio: &VirtIOMMIO, queue_idx: u32) -> VirtIOResult<Self> {
        // Queue 선택
        mmio.select_queue(queue_idx);

        // Queue 최대 크기 확인
        let max_size = mmio.queue_max_size();
        if max_size == 0 {
            return Err(VirtIOError::QueueSetupFailed);
        }

        // Queue 크기 결정 (최대 256개, 2의 거듭제곱)
        let queue_size = max_size.min(256) as u16;

        // 메모리 레이아웃 계산
        // Descriptor Table: 16 bytes × queue_size
        // Available Ring: 6 + 2 × queue_size bytes
        // Used Ring: 6 + 8 × queue_size bytes
        let desc_size = 16 * queue_size as usize;
        let avail_size = 6 + 2 * queue_size as usize;
        let used_size = 6 + 8 * queue_size as usize;

        // 페이지 정렬
        let desc_offset = 0;
        let avail_offset = desc_offset + desc_size;
        let used_offset = ((avail_offset + avail_size) + 4095) & !4095; // 페이지 정렬
        let total_size = used_offset + used_size;

        // 메모리 할당 (페이지 정렬)
        let layout = core::alloc::Layout::from_size_align(total_size, 4096)
            .map_err(|_| VirtIOError::QueueSetupFailed)?;
        let memory = unsafe {
            let ptr = alloc::alloc::alloc_zeroed(layout);
            if ptr.is_null() {
                return Err(VirtIOError::QueueSetupFailed);
            }
            Box::from_raw(core::slice::from_raw_parts_mut(ptr, total_size))
        };

        let base = memory.as_ptr() as usize;
        let desc_table = (base + desc_offset) as *mut VirtqDesc;
        let avail_ring = (base + avail_offset) as *mut VirtqAvail;
        let used_ring = (base + used_offset) as *mut VirtqUsed;

        // 디버그: Modern queue setup
        // crate::kprintln!("[VirtIO] Modern queue setup: base={:#x}, desc={:#x}, avail={:#x}, used={:#x}",
        //     base, base + desc_offset, base + avail_offset, base + used_offset);

        // Descriptor 체인 초기화 (free list)
        for i in 0..queue_size {
            unsafe {
                let desc = desc_table.add(i as usize);
                (*desc).next = if i + 1 < queue_size { i + 1 } else { 0 };
            }
        }

        // Modern MMIO에 주소 설정
        mmio.set_queue_size(queue_size as u32);
        mmio.set_queue_desc((base + desc_offset) as u64);
        mmio.set_queue_avail((base + avail_offset) as u64);
        mmio.set_queue_used((base + used_offset) as u64);
        mmio.set_queue_ready(true);

        Ok(Self {
            queue_idx,
            queue_size,
            free_head: 0,
            num_free: queue_size,
            last_used_idx: 0,
            desc_table,
            avail_ring,
            used_ring,
            _memory: memory,
        })
    }

    /// Queue 인덱스
    pub fn index(&self) -> u32 {
        self.queue_idx
    }

    /// Queue 크기
    pub fn size(&self) -> u16 {
        self.queue_size
    }

    /// 사용 가능한 Descriptor 수
    pub fn available_descs(&self) -> u16 {
        self.num_free
    }

    /// 버퍼 추가 (단일 버퍼, 읽기 또는 쓰기)
    pub fn add_buffer(&mut self, buf: &[u8], write: bool) -> VirtIOResult<u16> {
        if self.num_free == 0 {
            return Err(VirtIOError::BufferTooSmall);
        }

        // Descriptor 할당
        let desc_idx = self.free_head;
        unsafe {
            let desc = &mut *self.desc_table.add(desc_idx as usize);
            self.free_head = desc.next;
            self.num_free -= 1;

            desc.addr = buf.as_ptr() as u64;
            desc.len = buf.len() as u32;
            desc.flags = if write { desc_flags::WRITE } else { 0 };
            desc.next = 0;
        }

        // Available Ring에 추가
        unsafe {
            let avail = &mut *self.avail_ring;
            let avail_idx = read_volatile(&avail.idx);
            let ring_ptr = (avail as *mut VirtqAvail).add(1) as *mut u16;
            write_volatile(
                ring_ptr.add((avail_idx % self.queue_size) as usize),
                desc_idx,
            );
            fence(Ordering::SeqCst);
            write_volatile(&mut avail.idx, avail_idx.wrapping_add(1));
        }

        Ok(desc_idx)
    }

    /// 버퍼 체인 추가 (읽기 버퍼들 + 쓰기 버퍼들)
    pub fn add_buffer_chain(
        &mut self,
        read_bufs: &[&[u8]],
        write_bufs: &[&mut [u8]],
    ) -> VirtIOResult<u16> {
        let total = read_bufs.len() + write_bufs.len();
        if total == 0 {
            return Err(VirtIOError::BufferTooSmall);
        }
        if self.num_free < total as u16 {
            return Err(VirtIOError::BufferTooSmall);
        }

        let head = self.free_head;
        let mut prev_idx: Option<u16> = None;

        // 읽기 버퍼들 (디바이스가 읽음)
        for buf in read_bufs {
            let desc_idx = self.free_head;
            unsafe {
                let desc = &mut *self.desc_table.add(desc_idx as usize);
                self.free_head = desc.next;
                self.num_free -= 1;

                desc.addr = buf.as_ptr() as u64;
                desc.len = buf.len() as u32;
                desc.flags = desc_flags::NEXT;
                desc.next = self.free_head;

                if let Some(prev) = prev_idx {
                    (*self.desc_table.add(prev as usize)).next = desc_idx;
                }
                prev_idx = Some(desc_idx);
            }
        }

        // 쓰기 버퍼들 (디바이스가 씀)
        for (i, buf) in write_bufs.iter().enumerate() {
            let desc_idx = self.free_head;
            unsafe {
                let desc = &mut *self.desc_table.add(desc_idx as usize);
                self.free_head = desc.next;
                self.num_free -= 1;

                desc.addr = buf.as_ptr() as u64;
                desc.len = buf.len() as u32;
                desc.flags = desc_flags::WRITE;

                // 마지막이 아니면 NEXT 플래그
                if i + 1 < write_bufs.len() {
                    desc.flags |= desc_flags::NEXT;
                    desc.next = self.free_head;
                }

                if let Some(prev) = prev_idx {
                    (*self.desc_table.add(prev as usize)).next = desc_idx;
                }
                prev_idx = Some(desc_idx);
            }
        }

        // Available Ring에 추가
        unsafe {
            let avail = &mut *self.avail_ring;
            let avail_idx = read_volatile(&avail.idx);
            let ring_ptr = (avail as *mut VirtqAvail).add(1) as *mut u16;
            write_volatile(
                ring_ptr.add((avail_idx % self.queue_size) as usize),
                head,
            );
            fence(Ordering::SeqCst);
            write_volatile(&mut avail.idx, avail_idx.wrapping_add(1));
        }

        Ok(head)
    }

    /// 완료된 버퍼 확인
    pub fn poll_used(&mut self) -> Option<(u16, u32)> {
        fence(Ordering::SeqCst);

        unsafe {
            let used = &*self.used_ring;
            let used_idx = read_volatile(&used.idx);

            if self.last_used_idx == used_idx {
                return None;
            }

            let ring_ptr = (used as *const VirtqUsed).add(1) as *const VirtqUsedElem;
            let elem = read_volatile(
                ring_ptr.add((self.last_used_idx % self.queue_size) as usize),
            );

            self.last_used_idx = self.last_used_idx.wrapping_add(1);

            // Descriptor 해제
            self.free_descriptor_chain(elem.id as u16);

            Some((elem.id as u16, elem.len))
        }
    }

    /// Descriptor 체인 해제
    fn free_descriptor_chain(&mut self, mut head: u16) {
        loop {
            unsafe {
                let desc = &mut *self.desc_table.add(head as usize);
                let has_next = desc.flags & desc_flags::NEXT != 0;
                let next = desc.next;

                desc.next = self.free_head;
                self.free_head = head;
                self.num_free += 1;

                if has_next {
                    head = next;
                } else {
                    break;
                }
            }
        }
    }

    /// 처리 대기 중인 Used 항목 수
    pub fn pending_count(&self) -> u16 {
        unsafe {
            let used = &*self.used_ring;
            let used_idx = read_volatile(&used.idx);
            used_idx.wrapping_sub(self.last_used_idx)
        }
    }

    /// 대기 중인 항목이 있는지
    pub fn has_pending(&self) -> bool {
        self.pending_count() > 0
    }

    /// Available 링 인덱스 (디버그용)
    pub fn avail_idx(&self) -> u16 {
        unsafe {
            let avail = &*self.avail_ring;
            core::ptr::read_volatile(&avail.idx)
        }
    }

    /// Last used 인덱스 (디버그용)
    pub fn last_used_idx(&self) -> u16 {
        self.last_used_idx
    }

    /// Used 링 인덱스 (디버그용)
    pub fn used_idx(&self) -> u16 {
        unsafe {
            let used = &*self.used_ring;
            core::ptr::read_volatile(&used.idx)
        }
    }

    /// Descriptor 체인 디버그 출력
    pub fn debug_descriptor_chain(&self, head: u16) {
        let mut idx = head;
        let mut count = 0;

        crate::kprintln!("[VirtIO] Descriptor chain (head={}):", head);
        loop {
            if count > 10 {
                crate::kprintln!("  (chain too long, stopping)");
                break;
            }

            unsafe {
                let desc = &*self.desc_table.add(idx as usize);
                crate::kprintln!("  [{}] addr={:#x} len={} flags={:#x} next={}",
                    idx, desc.addr, desc.len, desc.flags, desc.next);

                if desc.flags & desc_flags::NEXT == 0 {
                    break;
                }
                idx = desc.next;
            }
            count += 1;
        }
    }
}
