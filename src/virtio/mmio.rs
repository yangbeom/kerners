//! VirtIO MMIO 레지스터 접근
//!
//! VirtIO MMIO 디바이스의 레지스터 레이아웃 및 접근 함수

use core::ptr::{read_volatile, write_volatile};
use super::{DeviceType, VirtIOError, VirtIOResult};

/// VirtIO MMIO 매직 넘버 ("virt")
pub const VIRTIO_MAGIC: u32 = 0x74726976;

/// VirtIO MMIO 레지스터 오프셋
#[allow(dead_code)]
mod regs {
    /// 매직 넘버 (읽기 전용): "virt" = 0x74726976
    pub const MAGIC_VALUE: usize = 0x000;
    /// 버전 (읽기 전용): 1 = legacy, 2 = modern
    pub const VERSION: usize = 0x004;
    /// 디바이스 ID (읽기 전용): 1=net, 2=blk, 3=console, ...
    pub const DEVICE_ID: usize = 0x008;
    /// 벤더 ID (읽기 전용)
    pub const VENDOR_ID: usize = 0x00c;
    /// 디바이스 Feature 비트 (읽기 전용)
    pub const DEVICE_FEATURES: usize = 0x010;
    /// 디바이스 Feature 선택 (쓰기 전용)
    pub const DEVICE_FEATURES_SEL: usize = 0x014;
    /// 드라이버 Feature 비트 (쓰기 전용)
    pub const DRIVER_FEATURES: usize = 0x020;
    /// 드라이버 Feature 선택 (쓰기 전용)
    pub const DRIVER_FEATURES_SEL: usize = 0x024;
    /// Guest 페이지 크기 (legacy, 쓰기 전용)
    pub const GUEST_PAGE_SIZE: usize = 0x028;
    /// Queue 선택 (쓰기 전용)
    pub const QUEUE_SEL: usize = 0x030;
    /// Queue 최대 크기 (읽기 전용)
    pub const QUEUE_NUM_MAX: usize = 0x034;
    /// Queue 크기 (쓰기 전용)
    pub const QUEUE_NUM: usize = 0x038;
    /// Queue Align (legacy, 쓰기 전용)
    pub const QUEUE_ALIGN: usize = 0x03c;
    /// Queue PFN (legacy, 읽기/쓰기) - 페이지 프레임 번호
    pub const QUEUE_PFN: usize = 0x040;
    /// Queue Ready (modern, 읽기/쓰기)
    pub const QUEUE_READY: usize = 0x044;
    /// Queue Notify (쓰기 전용)
    pub const QUEUE_NOTIFY: usize = 0x050;
    /// 인터럽트 상태 (읽기 전용)
    pub const INTERRUPT_STATUS: usize = 0x060;
    /// 인터럽트 ACK (쓰기 전용)
    pub const INTERRUPT_ACK: usize = 0x064;
    /// 디바이스 상태 (읽기/쓰기)
    pub const STATUS: usize = 0x070;
    /// Queue Descriptor 영역 (modern, 하위 32비트)
    pub const QUEUE_DESC_LOW: usize = 0x080;
    /// Queue Descriptor 영역 (modern, 상위 32비트)
    pub const QUEUE_DESC_HIGH: usize = 0x084;
    /// Queue Available 영역 (modern, 하위 32비트)
    pub const QUEUE_AVAIL_LOW: usize = 0x090;
    /// Queue Available 영역 (modern, 상위 32비트)
    pub const QUEUE_AVAIL_HIGH: usize = 0x094;
    /// Queue Used 영역 (modern, 하위 32비트)
    pub const QUEUE_USED_LOW: usize = 0x0a0;
    /// Queue Used 영역 (modern, 상위 32비트)
    pub const QUEUE_USED_HIGH: usize = 0x0a4;
    /// 디바이스별 설정 (오프셋 0x100부터)
    pub const CONFIG: usize = 0x100;
}

/// 디바이스 상태 비트
#[allow(dead_code)]
pub mod status {
    /// 드라이버가 디바이스 인식
    pub const ACKNOWLEDGE: u32 = 1;
    /// 드라이버가 디바이스 사용 가능
    pub const DRIVER: u32 = 2;
    /// 드라이버 준비 완료
    pub const DRIVER_OK: u32 = 4;
    /// Feature 협상 완료
    pub const FEATURES_OK: u32 = 8;
    /// 디바이스 에러
    pub const DEVICE_NEEDS_RESET: u32 = 64;
    /// 드라이버 실패
    pub const FAILED: u32 = 128;
}

/// 인터럽트 상태 비트
#[allow(dead_code)]
pub mod interrupt {
    /// Queue에서 사용된 버퍼 있음
    pub const USED_BUFFER: u32 = 1;
    /// 설정 변경됨
    pub const CONFIG_CHANGE: u32 = 2;
}

/// VirtIO MMIO 디바이스 핸들
pub struct VirtIOMMIO {
    base: usize,
}

impl VirtIOMMIO {
    /// 새 VirtIO MMIO 핸들 생성
    pub const fn new(base: usize) -> Self {
        Self { base }
    }

    /// MMIO 베이스 주소 반환
    pub fn base(&self) -> usize {
        self.base
    }

    /// 레지스터 읽기
    #[inline]
    unsafe fn read(&self, offset: usize) -> u32 {
        read_volatile((self.base + offset) as *const u32)
    }

    /// 레지스터 쓰기
    #[inline]
    unsafe fn write(&self, offset: usize, value: u32) {
        write_volatile((self.base + offset) as *mut u32, value);
    }

    /// 유효한 VirtIO 디바이스인지 확인
    pub fn is_valid(&self) -> bool {
        unsafe {
            let magic = self.read(regs::MAGIC_VALUE);
            let version = self.read(regs::VERSION);
            let device_id = self.read(regs::DEVICE_ID);

            magic == VIRTIO_MAGIC && version >= 1 && device_id != 0
        }
    }

    /// 매직 넘버 읽기
    pub fn magic(&self) -> u32 {
        unsafe { self.read(regs::MAGIC_VALUE) }
    }

    /// 버전 읽기
    pub fn version(&self) -> u32 {
        unsafe { self.read(regs::VERSION) }
    }

    /// 디바이스 ID 읽기
    pub fn device_id(&self) -> u32 {
        unsafe { self.read(regs::DEVICE_ID) }
    }

    /// 디바이스 타입 읽기
    pub fn device_type(&self) -> DeviceType {
        DeviceType::from(self.device_id())
    }

    /// 벤더 ID 읽기
    pub fn vendor_id(&self) -> u32 {
        unsafe { self.read(regs::VENDOR_ID) }
    }

    /// 디바이스 상태 읽기
    pub fn status(&self) -> u32 {
        unsafe { self.read(regs::STATUS) }
    }

    /// 디바이스 상태 쓰기
    pub fn set_status(&self, status: u32) {
        unsafe { self.write(regs::STATUS, status) }
    }

    /// 디바이스 리셋
    pub fn reset(&self) {
        self.set_status(0);
    }

    /// 디바이스 Feature 읽기 (32비트씩)
    pub fn device_features(&self, sel: u32) -> u32 {
        unsafe {
            self.write(regs::DEVICE_FEATURES_SEL, sel);
            self.read(regs::DEVICE_FEATURES)
        }
    }

    /// 드라이버 Feature 쓰기 (32비트씩)
    pub fn set_driver_features(&self, sel: u32, features: u32) {
        unsafe {
            self.write(regs::DRIVER_FEATURES_SEL, sel);
            self.write(regs::DRIVER_FEATURES, features);
        }
    }

    /// Queue 선택
    pub fn select_queue(&self, queue: u32) {
        unsafe { self.write(regs::QUEUE_SEL, queue) }
    }

    /// Queue 최대 크기 읽기
    pub fn queue_max_size(&self) -> u32 {
        unsafe { self.read(regs::QUEUE_NUM_MAX) }
    }

    /// Queue 크기 설정
    pub fn set_queue_size(&self, size: u32) {
        unsafe { self.write(regs::QUEUE_NUM, size) }
    }

    /// Queue Ready 읽기
    pub fn queue_ready(&self) -> bool {
        unsafe { self.read(regs::QUEUE_READY) != 0 }
    }

    /// Queue Ready 설정 (modern only)
    pub fn set_queue_ready(&self, ready: bool) {
        unsafe { self.write(regs::QUEUE_READY, if ready { 1 } else { 0 }) }
    }

    /// Guest 페이지 크기 설정 (legacy only)
    pub fn set_guest_page_size(&self, size: u32) {
        unsafe { self.write(regs::GUEST_PAGE_SIZE, size) }
    }

    /// Queue 정렬 설정 (legacy only)
    pub fn set_queue_align(&self, align: u32) {
        unsafe { self.write(regs::QUEUE_ALIGN, align) }
    }

    /// Queue PFN 설정 (legacy only) - 페이지 프레임 번호
    pub fn set_queue_pfn(&self, pfn: u32) {
        unsafe { self.write(regs::QUEUE_PFN, pfn) }
    }

    /// Queue PFN 읽기 (legacy only)
    pub fn queue_pfn(&self) -> u32 {
        unsafe { self.read(regs::QUEUE_PFN) }
    }

    /// Legacy 모드인지 확인
    pub fn is_legacy(&self) -> bool {
        self.version() == 1
    }

    /// Queue Notify
    pub fn notify_queue(&self, queue: u32) {
        unsafe { self.write(regs::QUEUE_NOTIFY, queue) }
    }

    /// 인터럽트 상태 읽기
    pub fn interrupt_status(&self) -> u32 {
        unsafe { self.read(regs::INTERRUPT_STATUS) }
    }

    /// 디바이스 상태 읽기
    pub fn device_status(&self) -> u32 {
        unsafe { self.read(regs::STATUS) }
    }

    /// 인터럽트 ACK
    pub fn ack_interrupt(&self, status: u32) {
        unsafe { self.write(regs::INTERRUPT_ACK, status) }
    }

    /// Queue Descriptor 주소 설정
    pub fn set_queue_desc(&self, addr: u64) {
        unsafe {
            self.write(regs::QUEUE_DESC_LOW, addr as u32);
            self.write(regs::QUEUE_DESC_HIGH, (addr >> 32) as u32);
        }
    }

    /// Queue Available 주소 설정
    pub fn set_queue_avail(&self, addr: u64) {
        unsafe {
            self.write(regs::QUEUE_AVAIL_LOW, addr as u32);
            self.write(regs::QUEUE_AVAIL_HIGH, (addr >> 32) as u32);
        }
    }

    /// Queue Used 주소 설정
    pub fn set_queue_used(&self, addr: u64) {
        unsafe {
            self.write(regs::QUEUE_USED_LOW, addr as u32);
            self.write(regs::QUEUE_USED_HIGH, (addr >> 32) as u32);
        }
    }

    /// 디바이스 설정 읽기 (8비트)
    pub fn read_config8(&self, offset: usize) -> u8 {
        unsafe { read_volatile((self.base + regs::CONFIG + offset) as *const u8) }
    }

    /// 디바이스 설정 읽기 (32비트)
    pub fn read_config32(&self, offset: usize) -> u32 {
        unsafe { read_volatile((self.base + regs::CONFIG + offset) as *const u32) }
    }

    /// 디바이스 설정 읽기 (64비트)
    pub fn read_config64(&self, offset: usize) -> u64 {
        unsafe { read_volatile((self.base + regs::CONFIG + offset) as *const u64) }
    }

    /// 디바이스 초기화
    pub fn init_device(&self) -> VirtIOResult<()> {
        // 1. 리셋
        self.reset();

        // 2. ACKNOWLEDGE 상태 설정
        self.set_status(status::ACKNOWLEDGE);

        // 3. DRIVER 상태 설정
        self.set_status(self.status() | status::DRIVER);

        Ok(())
    }

    /// Feature 협상 완료
    pub fn finish_features(&self) -> VirtIOResult<()> {
        self.set_status(self.status() | status::FEATURES_OK);

        // 디바이스가 Feature를 수락했는지 확인
        if self.status() & status::FEATURES_OK == 0 {
            self.set_status(status::FAILED);
            return Err(VirtIOError::FeatureNegotiationFailed);
        }

        Ok(())
    }

    /// 드라이버 준비 완료
    pub fn driver_ok(&self) {
        self.set_status(self.status() | status::DRIVER_OK);
    }
}
