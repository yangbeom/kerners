//! VirtIO 드라이버 프레임워크
//!
//! VirtIO MMIO 기반 디바이스 지원
//! - virtio-blk: 블록 디바이스
//! - virtio-net: 네트워크 (향후)
//! - virtio-console: 콘솔 (향후)

extern crate alloc;

pub mod mmio;
pub mod queue;
pub mod irq;

use alloc::vec::Vec;
use crate::dtb::DeviceInfo;

/// VirtIO 디바이스 타입
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum DeviceType {
    Invalid = 0,
    Network = 1,
    Block = 2,
    Console = 3,
    Entropy = 4,
    Balloon = 5,
    IoMemory = 6,
    Rpmsg = 7,
    Scsi = 8,
    Transport9P = 9,
    Mac80211 = 10,
    RprocSerial = 11,
    Caif = 12,
    MemoryBalloon = 13,
    Gpu = 16,
    Timer = 17,
    Input = 18,
    Socket = 19,
    Crypto = 20,
    SignalDist = 21,
    Pstore = 22,
    Iommu = 23,
    Memory = 24,
}

impl From<u32> for DeviceType {
    fn from(value: u32) -> Self {
        match value {
            0 => DeviceType::Invalid,
            1 => DeviceType::Network,
            2 => DeviceType::Block,
            3 => DeviceType::Console,
            4 => DeviceType::Entropy,
            5 => DeviceType::Balloon,
            16 => DeviceType::Gpu,
            18 => DeviceType::Input,
            _ => DeviceType::Invalid,
        }
    }
}

/// VirtIO 에러
#[derive(Debug, Clone, Copy)]
pub enum VirtIOError {
    /// 잘못된 매직 넘버
    InvalidMagic,
    /// 지원하지 않는 버전
    UnsupportedVersion,
    /// 디바이스 없음
    NoDevice,
    /// Feature 협상 실패
    FeatureNegotiationFailed,
    /// Queue 설정 실패
    QueueSetupFailed,
    /// I/O 에러
    IoError,
    /// 버퍼 부족
    BufferTooSmall,
    /// 타임아웃
    Timeout,
}

/// VirtIO 결과 타입
pub type VirtIOResult<T> = Result<T, VirtIOError>;

/// VirtIO MMIO 디바이스 정보
#[derive(Debug, Clone)]
pub struct VirtIODeviceInfo {
    /// MMIO 기본 주소
    pub mmio_base: usize,
    /// MMIO 크기
    pub mmio_size: usize,
    /// IRQ 번호
    pub irq: u32,
    /// 디바이스 타입
    pub device_type: DeviceType,
}

/// DTB에서 VirtIO 디바이스 목록 가져오기
pub fn find_virtio_devices() -> Vec<VirtIODeviceInfo> {
    let mut devices = Vec::new();

    if let Some(dt) = crate::dtb::get() {
        let virtio_nodes = dt.find_compatible("virtio,mmio");

        for info in virtio_nodes {
            if info.reg_base == 0 {
                continue;
            }

            // MMIO에서 디바이스 타입 확인
            let device_type = unsafe {
                let mmio = mmio::VirtIOMMIO::new(info.reg_base as usize);
                if mmio.is_valid() {
                    mmio.device_type()
                } else {
                    DeviceType::Invalid
                }
            };

            // Invalid가 아닌 실제 디바이스만 추가
            if device_type != DeviceType::Invalid {
                // IRQ 번호 추출 (아키텍처별 DTB 인터럽트 셀 형식이 다름)
                let irq = extract_irq_number(&info);

                devices.push(VirtIODeviceInfo {
                    mmio_base: info.reg_base as usize,
                    mmio_size: info.reg_size as usize,
                    irq,
                    device_type,
                });
            }
        }
    }

    devices
}

/// VirtIO 서브시스템 초기화
pub fn init() {
    crate::kprintln!("\n[VirtIO] Scanning for devices...");

    let devices = find_virtio_devices();

    if devices.is_empty() {
        crate::kprintln!("[VirtIO] No devices found");
        return;
    }

    for dev in &devices {
        crate::kprintln!(
            "[VirtIO] Found {:?} @ {:#x} (IRQ {})",
            dev.device_type,
            dev.mmio_base,
            dev.irq
        );
    }

    crate::kprintln!("[VirtIO] Found {} device(s)", devices.len());
}

/// DTB 인터럽트 속성에서 IRQ 번호 추출 (아키텍처별)
fn extract_irq_number(info: &DeviceInfo) -> u32 {
    #[cfg(target_arch = "aarch64")]
    {
        // GIC: interrupts = <type IRQ_num flags> (3-cell 형식)
        // type: 0=SPI, 1=PPI
        // SPI는 +32 오프셋 필요
        if info.interrupts.len() >= 2 {
            info.interrupts[1] + 32 // SPI 오프셋
        } else if !info.interrupts.is_empty() {
            info.interrupts[0]
        } else {
            0
        }
    }

    #[cfg(target_arch = "riscv64")]
    {
        // PLIC: interrupts = <IRQ_num> (1-cell 형식)
        if !info.interrupts.is_empty() {
            info.interrupts[0]
        } else {
            0
        }
    }
}
