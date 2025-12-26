//! RAM 디스크 블록 디바이스
//!
//! 메모리 기반 블록 디바이스로, 테스트 및 임시 저장소로 사용

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

use crate::sync::RwLock;

use super::{BlockDevice, BlockError, BlockResult};

/// 기본 블록 크기 (512 바이트)
pub const DEFAULT_BLOCK_SIZE: usize = 512;

/// RAM 디스크
pub struct RamDisk {
    /// 디바이스 이름
    name: String,
    /// 블록 크기
    block_size: usize,
    /// 데이터 저장소
    data: RwLock<Vec<u8>>,
    /// 읽기 전용 여부
    read_only: bool,
}

impl RamDisk {
    /// 새 RAM 디스크 생성
    ///
    /// `name`: 디바이스 이름
    /// `size`: 총 크기 (바이트)
    /// `block_size`: 블록 크기 (기본 512)
    pub fn new(name: &str, size: usize, block_size: usize) -> Self {
        // 블록 크기로 정렬
        let aligned_size = (size + block_size - 1) / block_size * block_size;

        Self {
            name: String::from(name),
            block_size,
            data: RwLock::new(vec![0u8; aligned_size]),
            read_only: false,
        }
    }

    /// 기본 블록 크기(512)로 RAM 디스크 생성
    pub fn new_default(name: &str, size: usize) -> Self {
        Self::new(name, size, DEFAULT_BLOCK_SIZE)
    }

    /// 기존 데이터로 RAM 디스크 생성 (읽기 전용)
    pub fn from_data(name: &str, data: Vec<u8>, block_size: usize) -> Self {
        // 블록 크기로 정렬
        let aligned_size = (data.len() + block_size - 1) / block_size * block_size;
        let mut aligned_data = data;
        aligned_data.resize(aligned_size, 0);

        Self {
            name: String::from(name),
            block_size,
            data: RwLock::new(aligned_data),
            read_only: true,
        }
    }

    /// 읽기 전용 설정
    pub fn set_read_only(&mut self, read_only: bool) {
        self.read_only = read_only;
    }

    /// Arc로 감싸서 반환
    pub fn into_arc(self) -> Arc<Self> {
        Arc::new(self)
    }

    /// 전체 데이터 읽기 (테스트용)
    pub fn read_all(&self) -> Vec<u8> {
        self.data.read().clone()
    }

    /// 전체 데이터 쓰기 (테스트용)
    pub fn write_all(&self, data: &[u8]) -> BlockResult<()> {
        if self.read_only {
            return Err(BlockError::ReadOnly);
        }

        let mut storage = self.data.write();
        let len = core::cmp::min(data.len(), storage.len());
        storage[..len].copy_from_slice(&data[..len]);
        Ok(())
    }
}

impl BlockDevice for RamDisk {
    fn name(&self) -> &str {
        &self.name
    }

    fn block_size(&self) -> usize {
        self.block_size
    }

    fn block_count(&self) -> u64 {
        let data = self.data.read();
        (data.len() / self.block_size) as u64
    }

    fn read_block(&self, block_num: u64, buf: &mut [u8]) -> BlockResult<()> {
        if buf.len() != self.block_size {
            return Err(BlockError::BufferSizeMismatch);
        }

        let data = self.data.read();
        let offset = block_num as usize * self.block_size;
        let end = offset + self.block_size;

        if end > data.len() {
            return Err(BlockError::InvalidBlock);
        }

        buf.copy_from_slice(&data[offset..end]);
        Ok(())
    }

    fn write_block(&self, block_num: u64, buf: &[u8]) -> BlockResult<()> {
        if self.read_only {
            return Err(BlockError::ReadOnly);
        }

        if buf.len() != self.block_size {
            return Err(BlockError::BufferSizeMismatch);
        }

        let mut data = self.data.write();
        let offset = block_num as usize * self.block_size;
        let end = offset + self.block_size;

        if end > data.len() {
            return Err(BlockError::InvalidBlock);
        }

        data[offset..end].copy_from_slice(buf);
        Ok(())
    }

    fn is_read_only(&self) -> bool {
        self.read_only
    }
}

/// RAM 디스크 생성 및 등록 헬퍼
pub fn create_ramdisk(name: &str, size: usize) -> Arc<RamDisk> {
    let disk = RamDisk::new_default(name, size);
    let arc = Arc::new(disk);
    super::register_device(name, arc.clone());
    arc
}

/// 크기를 MB 단위로 지정하여 RAM 디스크 생성
pub fn create_ramdisk_mb(name: &str, size_mb: usize) -> Arc<RamDisk> {
    create_ramdisk(name, size_mb * 1024 * 1024)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_ramdisk_basic() {
        let disk = RamDisk::new_default("test", 4096);

        assert_eq!(disk.block_size(), 512);
        assert_eq!(disk.block_count(), 8);
        assert_eq!(disk.capacity(), 4096);

        // 쓰기
        let write_buf = [0xABu8; 512];
        disk.write_block(0, &write_buf).unwrap();

        // 읽기
        let mut read_buf = [0u8; 512];
        disk.read_block(0, &mut read_buf).unwrap();

        assert_eq!(read_buf, write_buf);
    }

    fn test_ramdisk_readonly() {
        let mut disk = RamDisk::new_default("test_ro", 4096);
        disk.set_read_only(true);

        let buf = [0u8; 512];
        let result = disk.write_block(0, &buf);
        assert_eq!(result, Err(BlockError::ReadOnly));
    }
}
