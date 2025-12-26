//! 블록 디바이스 추상화
//!
//! 다양한 블록 디바이스(RAM 디스크, VirtIO 등)를 위한 공통 인터페이스

use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;

use crate::sync::RwLock;

pub mod ramdisk;
pub mod virtio_blk;

/// 블록 디바이스 에러
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockError {
    /// 잘못된 블록 번호
    InvalidBlock,
    /// I/O 에러
    IoError,
    /// 디바이스를 찾을 수 없음
    DeviceNotFound,
    /// 버퍼 크기 불일치
    BufferSizeMismatch,
    /// 읽기 전용 디바이스
    ReadOnly,
    /// 디바이스가 준비되지 않음
    NotReady,
}

impl fmt::Display for BlockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BlockError::InvalidBlock => write!(f, "invalid block number"),
            BlockError::IoError => write!(f, "I/O error"),
            BlockError::DeviceNotFound => write!(f, "device not found"),
            BlockError::BufferSizeMismatch => write!(f, "buffer size mismatch"),
            BlockError::ReadOnly => write!(f, "read-only device"),
            BlockError::NotReady => write!(f, "device not ready"),
        }
    }
}

/// 블록 디바이스 결과 타입
pub type BlockResult<T> = Result<T, BlockError>;

/// 블록 디바이스 trait
///
/// 모든 블록 디바이스는 이 trait을 구현해야 합니다.
pub trait BlockDevice: Send + Sync {
    /// 디바이스 이름
    fn name(&self) -> &str;

    /// 블록 크기 (바이트)
    fn block_size(&self) -> usize;

    /// 총 블록 수
    fn block_count(&self) -> u64;

    /// 블록 읽기
    ///
    /// `block_num`: 읽을 블록 번호
    /// `buf`: 데이터를 저장할 버퍼 (block_size 크기여야 함)
    fn read_block(&self, block_num: u64, buf: &mut [u8]) -> BlockResult<()>;

    /// 블록 쓰기
    ///
    /// `block_num`: 쓸 블록 번호
    /// `buf`: 쓸 데이터 (block_size 크기여야 함)
    fn write_block(&self, block_num: u64, buf: &[u8]) -> BlockResult<()>;

    /// 여러 블록 읽기 (기본 구현)
    fn read_blocks(&self, start_block: u64, buf: &mut [u8]) -> BlockResult<()> {
        let block_size = self.block_size();
        if buf.len() % block_size != 0 {
            return Err(BlockError::BufferSizeMismatch);
        }

        let block_count = buf.len() / block_size;
        for i in 0..block_count {
            let offset = i * block_size;
            self.read_block(start_block + i as u64, &mut buf[offset..offset + block_size])?;
        }
        Ok(())
    }

    /// 여러 블록 쓰기 (기본 구현)
    fn write_blocks(&self, start_block: u64, buf: &[u8]) -> BlockResult<()> {
        let block_size = self.block_size();
        if buf.len() % block_size != 0 {
            return Err(BlockError::BufferSizeMismatch);
        }

        let block_count = buf.len() / block_size;
        for i in 0..block_count {
            let offset = i * block_size;
            self.write_block(start_block + i as u64, &buf[offset..offset + block_size])?;
        }
        Ok(())
    }

    /// 캐시된 데이터를 디스크에 동기화
    fn sync(&self) -> BlockResult<()> {
        // 기본 구현: 아무것도 하지 않음 (RAM 기반은 필요 없음)
        Ok(())
    }

    /// 읽기 전용 여부
    fn is_read_only(&self) -> bool {
        false
    }

    /// 총 용량 (바이트)
    fn capacity(&self) -> u64 {
        self.block_count() * self.block_size() as u64
    }
}

/// 등록된 블록 디바이스
struct RegisteredDevice {
    name: String,
    device: Arc<dyn BlockDevice>,
}

/// 블록 디바이스 레지스트리
static BLOCK_DEVICES: RwLock<Vec<RegisteredDevice>> = RwLock::new(Vec::new());

/// 블록 디바이스 등록
pub fn register_device(name: &str, device: Arc<dyn BlockDevice>) {
    let capacity = device.capacity();
    let block_count = device.block_count();
    let mut devices = BLOCK_DEVICES.write();
    devices.push(RegisteredDevice {
        name: String::from(name),
        device,
    });
    crate::kprintln!("[block] Registered device: {} ({} bytes, {} blocks)",
        name,
        capacity,
        block_count
    );
}

/// 블록 디바이스 등록 해제
pub fn unregister_device(name: &str) -> bool {
    let mut devices = BLOCK_DEVICES.write();
    if let Some(pos) = devices.iter().position(|d| d.name == name) {
        devices.remove(pos);
        crate::kprintln!("[block] Unregistered device: {}", name);
        true
    } else {
        false
    }
}

/// 이름으로 블록 디바이스 검색
pub fn get_device(name: &str) -> Option<Arc<dyn BlockDevice>> {
    let devices = BLOCK_DEVICES.read();
    devices.iter()
        .find(|d| d.name == name)
        .map(|d| d.device.clone())
}

/// 등록된 모든 블록 디바이스 목록
pub fn list_devices() -> Vec<String> {
    let devices = BLOCK_DEVICES.read();
    devices.iter().map(|d| d.name.clone()).collect()
}

/// 블록 디바이스 정보
pub struct BlockDeviceInfo {
    pub name: String,
    pub block_size: usize,
    pub block_count: u64,
    pub capacity: u64,
    pub read_only: bool,
}

/// 블록 디바이스 정보 조회
pub fn device_info(name: &str) -> Option<BlockDeviceInfo> {
    let devices = BLOCK_DEVICES.read();
    devices.iter()
        .find(|d| d.name == name)
        .map(|d| BlockDeviceInfo {
            name: d.name.clone(),
            block_size: d.device.block_size(),
            block_count: d.device.block_count(),
            capacity: d.device.capacity(),
            read_only: d.device.is_read_only(),
        })
}

/// 블록 서브시스템 초기화
pub fn init() {
    crate::kprintln!("\n[block] Initializing block subsystem...");

    // VirtIO 블록 디바이스 초기화
    if let Some(vda) = virtio_blk::init() {
        register_device("vda", vda);
    }

    let devices = list_devices();
    if devices.is_empty() {
        crate::kprintln!("[block] No block devices found");
    } else {
        crate::kprintln!("[block] {} device(s) available: {:?}", devices.len(), devices);
    }
}
