//! VirtIO 블록 드라이버
//!
//! VirtIO MMIO 기반 블록 디바이스 드라이버

extern crate alloc;

use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use core::sync::atomic::{AtomicBool, Ordering};
use crate::block::{BlockDevice, BlockError, BlockResult};
use crate::sync::Mutex;
use crate::virtio::mmio::{self, VirtIOMMIO};
use crate::virtio::queue::Virtqueue;
use crate::virtio::{DeviceType, VirtIODeviceInfo, VirtIOError, VirtIOResult};

/// VirtIO 블록 디바이스 Feature 비트
#[allow(dead_code)]
mod features {
    /// 최대 세그먼트 크기
    pub const SIZE_MAX: u64 = 1 << 1;
    /// 최대 세그먼트 수
    pub const SEG_MAX: u64 = 1 << 2;
    /// 디스크 지오메트리
    pub const GEOMETRY: u64 = 1 << 4;
    /// 읽기 전용
    pub const RO: u64 = 1 << 5;
    /// 블록 크기
    pub const BLK_SIZE: u64 = 1 << 6;
    /// 플러시 지원
    pub const FLUSH: u64 = 1 << 9;
    /// 토폴로지 정보
    pub const TOPOLOGY: u64 = 1 << 10;
    /// 쓰기 제로 지원
    pub const WRITE_ZEROES: u64 = 1 << 14;

    // VirtIO 공통 Feature 비트 (selector 1)
    /// VirtIO 1.0+ 현대적 디바이스
    pub const VIRTIO_F_VERSION_1: u32 = 1 << 0; // 비트 32 -> selector 1의 비트 0
}

/// VirtIO 블록 요청 타입
#[repr(u32)]
#[derive(Debug, Clone, Copy)]
pub enum RequestType {
    In = 0,      // 읽기
    Out = 1,     // 쓰기
    Flush = 4,   // 플러시
    GetId = 8,   // 디바이스 ID 조회
}

/// VirtIO 블록 요청 헤더
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct VirtIOBlkReqHeader {
    /// 요청 타입
    pub req_type: u32,
    /// 예약
    pub reserved: u32,
    /// 섹터 번호
    pub sector: u64,
}

/// 요청 컨텍스트 (DMA가 접근하는 헤더/상태를 한 묶음으로 유지)
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct VirtIOBlkReqCtx {
    header: VirtIOBlkReqHeader,
    status: u8,
}

/// VirtIO 블록 응답 상태
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VirtIOBlkStatus {
    Ok = 0,
    IoErr = 1,
    Unsupported = 2,
}

/// VirtIO 블록 디바이스 설정
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct VirtIOBlkConfig {
    /// 용량 (512바이트 섹터 수)
    pub capacity: u64,
    /// 최대 세그먼트 크기
    pub size_max: u32,
    /// 최대 세그먼트 수
    pub seg_max: u32,
    /// 지오메트리
    pub geometry: VirtIOBlkGeometry,
    /// 블록 크기
    pub blk_size: u32,
}

/// 디스크 지오메트리
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct VirtIOBlkGeometry {
    pub cylinders: u16,
    pub heads: u8,
    pub sectors: u8,
}

/// VirtIO 블록 디바이스
pub struct VirtIOBlock {
    /// 디바이스 이름
    name: String,
    /// MMIO 핸들
    mmio: VirtIOMMIO,
    /// Virtqueue
    queue: Mutex<Virtqueue>,
    /// 용량 (섹터 수)
    capacity: u64,
    /// 블록 크기 (바이트)
    block_size: usize,
    /// 읽기 전용 여부
    read_only: bool,
    /// IRQ 번호
    irq: u32,
    /// 인터럽트 플래그
    interrupt_flag: AtomicBool,
}

// Safety: VirtIOBlock은 Mutex로 보호됨
unsafe impl Send for VirtIOBlock {}
unsafe impl Sync for VirtIOBlock {}

impl VirtIOBlock {
    #[inline]
    fn validate_read_chain(
        queue: &Virtqueue,
        head: u16,
        header_ptr: u64,
        data_ptr: u64,
        data_len: u32,
        status_ptr: u64,
    ) -> bool {
        if head >= queue.size() {
            return false;
        }

        let d0 = queue.descriptor(head);
        if d0.addr != header_ptr || d0.len != core::mem::size_of::<VirtIOBlkReqHeader>() as u32 {
            return false;
        }
        if d0.flags & crate::virtio::queue::desc_flags::NEXT == 0
            || d0.flags & crate::virtio::queue::desc_flags::WRITE != 0
        {
            return false;
        }

        if d0.next >= queue.size() {
            return false;
        }
        let d1 = queue.descriptor(d0.next);
        if d1.addr != data_ptr || d1.len != data_len {
            return false;
        }
        if d1.flags & crate::virtio::queue::desc_flags::NEXT == 0
            || d1.flags & crate::virtio::queue::desc_flags::WRITE == 0
        {
            return false;
        }

        if d1.next >= queue.size() {
            return false;
        }
        let d2 = queue.descriptor(d1.next);
        if d2.addr != status_ptr || d2.len != 1 {
            return false;
        }
        if d2.flags & crate::virtio::queue::desc_flags::NEXT != 0
            || d2.flags & crate::virtio::queue::desc_flags::WRITE == 0
        {
            return false;
        }

        true
    }

    #[inline]
    fn validate_write_chain(
        queue: &Virtqueue,
        head: u16,
        header_ptr: u64,
        data_ptr: u64,
        data_len: u32,
        status_ptr: u64,
    ) -> bool {
        if head >= queue.size() {
            return false;
        }

        let d0 = queue.descriptor(head);
        if d0.addr != header_ptr || d0.len != core::mem::size_of::<VirtIOBlkReqHeader>() as u32 {
            return false;
        }
        if d0.flags & crate::virtio::queue::desc_flags::NEXT == 0
            || d0.flags & crate::virtio::queue::desc_flags::WRITE != 0
        {
            return false;
        }

        if d0.next >= queue.size() {
            return false;
        }
        let d1 = queue.descriptor(d0.next);
        if d1.addr != data_ptr || d1.len != data_len {
            return false;
        }
        if d1.flags & crate::virtio::queue::desc_flags::NEXT == 0
            || d1.flags & crate::virtio::queue::desc_flags::WRITE != 0
        {
            return false;
        }

        if d1.next >= queue.size() {
            return false;
        }
        let d2 = queue.descriptor(d1.next);
        if d2.addr != status_ptr || d2.len != 1 {
            return false;
        }
        if d2.flags & crate::virtio::queue::desc_flags::NEXT != 0
            || d2.flags & crate::virtio::queue::desc_flags::WRITE == 0
        {
            return false;
        }

        true
    }

    /// 새 VirtIO 블록 디바이스 생성
    pub fn new(info: &VirtIODeviceInfo, name: &str) -> VirtIOResult<Self> {
        if info.device_type != DeviceType::Block {
            return Err(VirtIOError::NoDevice);
        }

        let mmio = VirtIOMMIO::new(info.mmio_base);

        // VirtIO 버전 확인
        let version = mmio.version();
        crate::kprintln!("[VirtIO-blk] MMIO version: {}", version);

        // 디바이스 초기화
        mmio.init_device()?;

        // Feature 읽기 (selector 0: 디바이스별 기능, selector 1: VirtIO 공통 기능)
        let device_features_lo = mmio.device_features(0);
        let device_features_hi = mmio.device_features(1);
        let device_features = device_features_lo as u64 | ((device_features_hi as u64) << 32);
        crate::kprintln!(
            "[VirtIO-blk] Device features: {:#x} (lo={:#x}, hi={:#x})",
            device_features, device_features_lo, device_features_hi
        );

        // Feature 협상
        // VirtIO 현대적 디바이스 (v2)는 VIRTIO_F_VERSION_1 필수
        let driver_features_lo = 0u32; // 기본 기능만 사용
        let driver_features_hi = if version >= 2 && (device_features_hi & features::VIRTIO_F_VERSION_1) != 0 {
            crate::kprintln!("[VirtIO-blk] Negotiating VIRTIO_F_VERSION_1");
            features::VIRTIO_F_VERSION_1
        } else {
            0u32
        };

        mmio.set_driver_features(0, driver_features_lo);
        mmio.set_driver_features(1, driver_features_hi);

        // Feature 협상 완료
        mmio.finish_features()?;

        // 용량 읽기
        let capacity = mmio.read_config64(0);
        let block_size = if device_features & features::BLK_SIZE != 0 {
            mmio.read_config32(20) as usize
        } else {
            512 // 기본 섹터 크기
        };
        let read_only = device_features & features::RO != 0;

        crate::kprintln!(
            "[VirtIO-blk] Capacity: {} sectors ({} MB), block_size: {}, read_only: {}",
            capacity,
            capacity * 512 / (1024 * 1024),
            block_size,
            read_only
        );

        // Virtqueue 설정
        let queue = Virtqueue::new(&mmio, 0)?;
        crate::kprintln!(
            "[VirtIO-blk] Queue setup: size={}",
            queue.size()
        );

        // 드라이버 준비 완료
        mmio.driver_ok();

        Ok(Self {
            name: String::from(name),
            mmio,
            queue: Mutex::new(queue),
            capacity,
            block_size,
            read_only,
            irq: info.irq,
            interrupt_flag: AtomicBool::new(false),
        })
    }

    /// 블록 읽기 (내부)
    fn read_block_internal(&self, block_num: u64, buf: &mut [u8]) -> VirtIOResult<()> {
        if buf.len() < self.block_size {
            return Err(VirtIOError::BufferTooSmall);
        }
        if block_num >= self.capacity {
            return Err(VirtIOError::IoError);
        }

        // DMA가 참조하는 헤더/상태를 힙에 둬서 스택 생명주기와 분리한다.
        let mut req = Box::new(VirtIOBlkReqCtx {
            header: VirtIOBlkReqHeader {
                req_type: RequestType::In as u32,
                reserved: 0,
                sector: block_num,
            },
            status: 0xFF,
        });

        // Virtqueue에 요청 추가
        let header_buf = unsafe {
            core::slice::from_raw_parts(
                &req.header as *const _ as *const u8,
                core::mem::size_of::<VirtIOBlkReqHeader>(),
            )
        };
        let status_buf = unsafe {
            core::slice::from_raw_parts_mut(&mut req.status as *mut u8, 1)
        };

        let expected_head = {
            let mut queue = self.queue.lock();
            if queue.available_descs() != queue.size() {
                crate::kprintln!(
                    "[VirtIO-blk] queue not empty before read: free={}/{}",
                    queue.available_descs(),
                    queue.size()
                );
            }

            let head = queue.add_buffer_chain(
                &[header_buf],
                &[buf, status_buf],
            )?;
            if !Self::validate_read_chain(
                &queue,
                head,
                header_buf.as_ptr() as u64,
                buf.as_ptr() as u64,
                buf.len() as u32,
                status_buf.as_ptr() as u64,
            ) {
                crate::kprintln!("[VirtIO-blk] invalid read descriptor chain before notify");
                queue.debug_descriptor_chain(head);
                return Err(VirtIOError::IoError);
            }

            // 메모리 배리어 - 디바이스가 descriptor를 볼 수 있도록
            core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
            #[cfg(target_arch = "aarch64")]
            unsafe {
                core::arch::asm!("dmb oshst", options(nostack, preserves_flags));
            }
            #[cfg(target_arch = "riscv64")]
            unsafe {
                core::arch::asm!("fence ow,ow", options(nostack, preserves_flags));
            }

            // 디바이스에 알림
            self.mmio.notify_queue(0);
            head
        };

        // 완료 대기 (폴링)
        self.wait_for_completion(expected_head)?;

        // 상태 확인
        if req.status != VirtIOBlkStatus::Ok as u8 {
            crate::kprintln!("[VirtIO-blk] Read error: status={}", req.status);
            return Err(VirtIOError::IoError);
        }

        {
            let queue = self.queue.lock();
            if queue.available_descs() != queue.size() {
                crate::kprintln!(
                    "[VirtIO-blk] queue leak/corruption after read: free={}/{}",
                    queue.available_descs(),
                    queue.size()
                );
            }
        }

        Ok(())
    }

    /// 블록 쓰기 (내부)
    fn write_block_internal(&self, block_num: u64, buf: &[u8]) -> VirtIOResult<()> {
        if self.read_only {
            return Err(VirtIOError::IoError);
        }
        if buf.len() < self.block_size {
            return Err(VirtIOError::BufferTooSmall);
        }
        if block_num >= self.capacity {
            return Err(VirtIOError::IoError);
        }

        // DMA가 참조하는 헤더/상태를 힙에 둬서 스택 생명주기와 분리한다.
        let mut req = Box::new(VirtIOBlkReqCtx {
            header: VirtIOBlkReqHeader {
                req_type: RequestType::Out as u32,
                reserved: 0,
                sector: block_num,
            },
            status: 0xFF,
        });

        // Virtqueue에 요청 추가
        let header_buf = unsafe {
            core::slice::from_raw_parts(
                &req.header as *const _ as *const u8,
                core::mem::size_of::<VirtIOBlkReqHeader>(),
            )
        };
        let status_buf = unsafe {
            core::slice::from_raw_parts_mut(&mut req.status as *mut u8, 1)
        };

        let expected_head = {
            let mut queue = self.queue.lock();
            if queue.available_descs() != queue.size() {
                crate::kprintln!(
                    "[VirtIO-blk] queue not empty before write: free={}/{}",
                    queue.available_descs(),
                    queue.size()
                );
            }
            let head = queue.add_buffer_chain(
                &[header_buf, buf],
                &[status_buf],
            )?;
            if !Self::validate_write_chain(
                &queue,
                head,
                header_buf.as_ptr() as u64,
                buf.as_ptr() as u64,
                buf.len() as u32,
                status_buf.as_ptr() as u64,
            ) {
                crate::kprintln!("[VirtIO-blk] invalid write descriptor chain before notify");
                queue.debug_descriptor_chain(head);
                return Err(VirtIOError::IoError);
            }

            core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
            #[cfg(target_arch = "aarch64")]
            unsafe {
                core::arch::asm!("dmb oshst", options(nostack, preserves_flags));
            }
            #[cfg(target_arch = "riscv64")]
            unsafe {
                core::arch::asm!("fence ow,ow", options(nostack, preserves_flags));
            }

            // 디바이스에 알림
            self.mmio.notify_queue(0);
            head
        };

        // 완료 대기 (폴링)
        self.wait_for_completion(expected_head)?;

        // 상태 확인
        if req.status != VirtIOBlkStatus::Ok as u8 {
            crate::kprintln!("[VirtIO-blk] Write error: status={}", req.status);
            return Err(VirtIOError::IoError);
        }

        {
            let queue = self.queue.lock();
            if queue.available_descs() != queue.size() {
                crate::kprintln!(
                    "[VirtIO-blk] queue leak/corruption after write: free={}/{}",
                    queue.available_descs(),
                    queue.size()
                );
            }
        }

        Ok(())
    }

    /// 완료 대기 (인터럽트 + WFI 기반, 폴링 fallback)
    fn wait_for_completion(&self, expected_head: u16) -> VirtIOResult<()> {
        // 디버깅용: IRQ/WFI 경로를 우회하고 순수 폴링으로만 완료를 대기한다.
        // 원인 분리 후 인터럽트 경로를 복구할 계획이다.
        let mut spins = 0u32;
        let mut unexpected_used = 0u16;
        loop {
            {
                #[cfg(target_arch = "aarch64")]
                unsafe {
                    core::arch::asm!("dmb oshld", options(nostack, preserves_flags));
                }
                #[cfg(target_arch = "riscv64")]
                unsafe {
                    core::arch::asm!("fence ir,ir", options(nostack, preserves_flags));
                }
                let mut queue = self.queue.lock();
                if let Some((id, _len)) = queue.poll_used() {
                    if id >= queue.size() {
                        crate::kprintln!(
                            "[VirtIO-blk] invalid used descriptor id: {} (queue size={})",
                            id,
                            queue.size()
                        );
                        return Err(VirtIOError::IoError);
                    }
                    let status = self.mmio.interrupt_status();
                    if status != 0 {
                        self.mmio.ack_interrupt(status);
                    }
                    self.interrupt_flag.store(false, Ordering::SeqCst);
                    if id == expected_head {
                        return Ok(());
                    }

                    unexpected_used = unexpected_used.wrapping_add(1);
                    crate::kprintln!(
                        "[VirtIO-blk] unexpected used descriptor id: got={}, expected={}",
                        id,
                        expected_head
                    );
                    if unexpected_used >= queue.size() {
                        crate::kprintln!(
                            "[VirtIO-blk] too many unexpected completions (count={})",
                            unexpected_used
                        );
                        return Err(VirtIOError::IoError);
                    }
                }
            }

            spins = spins.wrapping_add(1);
            if spins == 100_000 {
                let queue = self.queue.lock();
                crate::kprintln!("[VirtIO-blk] slow completion, still waiting...");
                let avail_idx = queue.avail_idx();
                let used_idx = queue.used_idx();
                let last_used = queue.last_used_idx();
                crate::kprintln!(
                    "  avail_idx={}, used_idx={}, last_used={}",
                    avail_idx,
                    used_idx,
                    last_used
                );
                let pending_head = queue.avail_head_at(last_used);
                let pending_desc = queue.descriptor(pending_head);
                crate::kprintln!(
                    "  pending head={} desc(addr={:#x}, len={}, flags={:#x}, next={})",
                    pending_head,
                    pending_desc.addr,
                    pending_desc.len,
                    pending_desc.flags,
                    pending_desc.next
                );
                queue.debug_descriptor_chain(pending_head);
                crate::kprintln!("  ISR status: {:#x}", self.mmio.interrupt_status());
                spins = 0;
            }

            core::hint::spin_loop();
        }
    }

    /// 인터럽트 핸들러
    pub fn handle_irq(&self) {
        let status = self.mmio.interrupt_status();
        if status != 0 {
            self.mmio.ack_interrupt(status);
            self.interrupt_flag.store(true, Ordering::SeqCst);
        }
    }

    /// 인터럽트 컨트롤러에 IRQ 등록 및 디스패치 테이블에 등록
    pub fn register_interrupt(&self) {
        if self.irq == 0 {
            return;
        }

        // 인터럽트 컨트롤러에 IRQ 활성화
        #[cfg(target_arch = "aarch64")]
        unsafe {
            crate::arch::gic::set_priority(self.irq, 0x90);
            crate::arch::gic::set_target(self.irq, 1); // CPU 0
            crate::arch::gic::enable_irq(self.irq);
        }

        #[cfg(target_arch = "riscv64")]
        unsafe {
            crate::arch::plic::set_priority(self.irq, 1);
            crate::arch::plic::enable_irq(self.irq);
        }

        // VirtIO IRQ 디스패치 테이블에 등록
        crate::virtio::irq::register_irq(
            self.irq,
            self.mmio.base(),
            &self.interrupt_flag,
        );
    }
}

impl BlockDevice for VirtIOBlock {
    fn name(&self) -> &str {
        &self.name
    }

    fn block_size(&self) -> usize {
        self.block_size
    }

    fn block_count(&self) -> u64 {
        self.capacity
    }

    fn read_block(&self, block_num: u64, buf: &mut [u8]) -> BlockResult<()> {
        self.read_block_internal(block_num, buf)
            .map_err(|_| BlockError::IoError)
    }

    fn write_block(&self, block_num: u64, buf: &[u8]) -> BlockResult<()> {
        self.write_block_internal(block_num, buf)
            .map_err(|_| BlockError::IoError)
    }

    fn is_read_only(&self) -> bool {
        self.read_only
    }

    fn sync(&self) -> BlockResult<()> {
        // TODO: FLUSH 요청 구현
        Ok(())
    }
}

/// VirtIO 블록 디바이스 초기화
pub fn init() -> Option<Arc<VirtIOBlock>> {
    let devices = crate::virtio::find_virtio_devices();

    for info in devices {
        if info.device_type == DeviceType::Block {
            match VirtIOBlock::new(&info, "vda") {
                Ok(dev) => {
                    crate::kprintln!(
                        "[VirtIO-blk] Initialized {} @ {:#x}",
                        dev.name(),
                        info.mmio_base
                    );
                    let dev = Arc::new(dev);
                    // 디버깅: 원인 분리용으로 IRQ 등록을 잠시 비활성화하고 폴링만 사용한다.
                    // 인터럽트 경로가 원인이 아닌지 확인 후 복구한다.
                    // dev.register_interrupt();
                    return Some(dev);
                }
                Err(e) => {
                    crate::kprintln!("[VirtIO-blk] Init failed: {:?}", e);
                }
            }
        }
    }

    None
}
