//! FAT 테이블 관리
//!
//! FAT32 파일 할당 테이블 읽기/쓰기

use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

use crate::block::BlockDevice;
use crate::sync::Mutex;

use super::boot::Fat32BootSector;

/// FAT 테이블 특수 값
pub const FAT_FREE: u32 = 0x00000000;
pub const FAT_RESERVED_MIN: u32 = 0x0FFFFFF0;
pub const FAT_BAD_CLUSTER: u32 = 0x0FFFFFF7;
pub const FAT_EOC_MIN: u32 = 0x0FFFFFF8; // End of Chain 최소값
pub const FAT_EOC: u32 = 0x0FFFFFFF; // End of Chain

/// FAT 테이블 관리자
pub struct FatTable {
    /// 블록 디바이스
    device: Arc<dyn BlockDevice>,
    /// FAT 시작 섹터
    fat_start: u32,
    /// FAT 크기 (섹터 수)
    fat_size: u32,
    /// FAT 테이블 수
    num_fats: u8,
    /// 섹터 당 바이트 수
    bytes_per_sector: u16,
    /// 총 클러스터 수
    total_clusters: u32,
    /// 빈 클러스터 수 캐시
    free_count: Mutex<Option<u32>>,
    /// 다음 빈 클러스터 힌트
    next_free_hint: Mutex<u32>,
}

impl FatTable {
    /// 새 FAT 테이블 관리자 생성
    pub fn new(device: Arc<dyn BlockDevice>, boot: &Fat32BootSector) -> Self {
        Self {
            device,
            fat_start: boot.fat_start_sector(),
            fat_size: boot.fat_size_32,
            num_fats: boot.num_fats,
            bytes_per_sector: boot.bytes_per_sector,
            total_clusters: boot.total_clusters(),
            free_count: Mutex::new(None),
            next_free_hint: Mutex::new(2), // 클러스터는 2부터 시작
        }
    }

    /// 클러스터의 다음 클러스터 읽기
    pub fn read_entry(&self, cluster: u32) -> Result<u32, FatError> {
        if cluster < 2 || cluster >= self.total_clusters + 2 {
            return Err(FatError::InvalidCluster);
        }

        // FAT 엔트리 위치 계산
        let fat_offset = cluster * 4; // FAT32는 4바이트 엔트리
        let fat_sector = self.fat_start + (fat_offset / self.bytes_per_sector as u32);
        let entry_offset = (fat_offset % self.bytes_per_sector as u32) as usize;

        // 섹터 읽기
        let mut buf = vec![0u8; self.bytes_per_sector as usize];
        self.device
            .read_block(fat_sector as u64, &mut buf)
            .map_err(|_| FatError::IoError)?;

        // 엔트리 읽기 (리틀 엔디안)
        let entry = u32::from_le_bytes([
            buf[entry_offset],
            buf[entry_offset + 1],
            buf[entry_offset + 2],
            buf[entry_offset + 3],
        ]);

        // FAT32는 상위 4비트 무시
        Ok(entry & 0x0FFFFFFF)
    }

    /// 클러스터의 다음 클러스터 쓰기
    pub fn write_entry(&self, cluster: u32, value: u32) -> Result<(), FatError> {
        if cluster < 2 || cluster >= self.total_clusters + 2 {
            return Err(FatError::InvalidCluster);
        }

        // FAT 엔트리 위치 계산
        let fat_offset = cluster * 4;
        let fat_sector = self.fat_start + (fat_offset / self.bytes_per_sector as u32);
        let entry_offset = (fat_offset % self.bytes_per_sector as u32) as usize;

        // 모든 FAT 테이블에 쓰기
        for fat_num in 0..self.num_fats {
            let sector = fat_sector + (fat_num as u32 * self.fat_size);

            // 섹터 읽기
            let mut buf = vec![0u8; self.bytes_per_sector as usize];
            self.device
                .read_block(sector as u64, &mut buf)
                .map_err(|_| FatError::IoError)?;

            // 상위 4비트 보존
            let old_entry = u32::from_le_bytes([
                buf[entry_offset],
                buf[entry_offset + 1],
                buf[entry_offset + 2],
                buf[entry_offset + 3],
            ]);
            let new_entry = (old_entry & 0xF0000000) | (value & 0x0FFFFFFF);

            // 엔트리 쓰기
            let bytes = new_entry.to_le_bytes();
            buf[entry_offset..entry_offset + 4].copy_from_slice(&bytes);

            // 섹터 쓰기
            self.device
                .write_block(sector as u64, &buf)
                .map_err(|_| FatError::IoError)?;
        }

        Ok(())
    }

    /// 클러스터 체인 읽기
    pub fn read_chain(&self, start_cluster: u32) -> Result<Vec<u32>, FatError> {
        let mut chain = Vec::new();
        let mut current = start_cluster;

        while current >= 2 && current < FAT_RESERVED_MIN {
            chain.push(current);
            current = self.read_entry(current)?;

            // 무한 루프 방지
            if chain.len() > self.total_clusters as usize {
                return Err(FatError::CorruptedChain);
            }
        }

        Ok(chain)
    }

    /// 빈 클러스터 할당
    pub fn alloc_cluster(&self) -> Result<u32, FatError> {
        let mut hint = self.next_free_hint.lock();
        let start = *hint;

        // 힌트부터 검색
        for cluster in start..self.total_clusters + 2 {
            if self.read_entry(cluster)? == FAT_FREE {
                // EOC로 마킹
                self.write_entry(cluster, FAT_EOC)?;
                *hint = cluster + 1;

                // 빈 클러스터 수 업데이트
                if let Some(ref mut count) = *self.free_count.lock() {
                    if *count > 0 {
                        *count -= 1;
                    }
                }

                return Ok(cluster);
            }
        }

        // 처음부터 힌트까지 검색
        for cluster in 2..start {
            if self.read_entry(cluster)? == FAT_FREE {
                self.write_entry(cluster, FAT_EOC)?;
                *hint = cluster + 1;

                if let Some(ref mut count) = *self.free_count.lock() {
                    if *count > 0 {
                        *count -= 1;
                    }
                }

                return Ok(cluster);
            }
        }

        Err(FatError::NoSpace)
    }

    /// 여러 클러스터 연속 할당
    pub fn alloc_clusters(&self, count: usize) -> Result<Vec<u32>, FatError> {
        if count == 0 {
            return Ok(Vec::new());
        }

        let mut clusters = Vec::with_capacity(count);

        // 첫 번째 클러스터 할당
        let first = self.alloc_cluster()?;
        clusters.push(first);

        // 나머지 클러스터 할당 및 체인 연결
        for _ in 1..count {
            match self.alloc_cluster() {
                Ok(cluster) => {
                    // 이전 클러스터를 새 클러스터에 연결
                    let prev = *clusters.last().unwrap();
                    self.write_entry(prev, cluster)?;
                    clusters.push(cluster);
                }
                Err(e) => {
                    // 할당 실패 시 이미 할당된 클러스터 해제
                    for &c in &clusters {
                        let _ = self.free_cluster(c);
                    }
                    return Err(e);
                }
            }
        }

        Ok(clusters)
    }

    /// 클러스터 해제
    pub fn free_cluster(&self, cluster: u32) -> Result<(), FatError> {
        self.write_entry(cluster, FAT_FREE)?;

        // 빈 클러스터 수 업데이트
        if let Some(ref mut count) = *self.free_count.lock() {
            *count += 1;
        }

        // 힌트 업데이트 (더 낮은 번호로)
        let mut hint = self.next_free_hint.lock();
        if cluster < *hint {
            *hint = cluster;
        }

        Ok(())
    }

    /// 클러스터 체인 해제
    pub fn free_chain(&self, start_cluster: u32) -> Result<(), FatError> {
        let chain = self.read_chain(start_cluster)?;
        for cluster in chain {
            self.free_cluster(cluster)?;
        }
        Ok(())
    }

    /// 체인 확장 (기존 체인에 클러스터 추가)
    pub fn extend_chain(&self, last_cluster: u32, count: usize) -> Result<Vec<u32>, FatError> {
        let new_clusters = self.alloc_clusters(count)?;

        if !new_clusters.is_empty() {
            // 기존 체인의 마지막을 새 체인의 시작에 연결
            self.write_entry(last_cluster, new_clusters[0])?;
        }

        Ok(new_clusters)
    }

    /// 체인 축소 (마지막 N개 클러스터 해제)
    pub fn truncate_chain(&self, start_cluster: u32, keep_count: usize) -> Result<(), FatError> {
        let chain = self.read_chain(start_cluster)?;

        if keep_count >= chain.len() {
            return Ok(()); // 축소할 필요 없음
        }

        if keep_count == 0 {
            // 전체 해제
            return self.free_chain(start_cluster);
        }

        // 새 마지막 클러스터를 EOC로 마킹
        self.write_entry(chain[keep_count - 1], FAT_EOC)?;

        // 나머지 해제
        for &cluster in &chain[keep_count..] {
            self.free_cluster(cluster)?;
        }

        Ok(())
    }

    /// End of Chain 확인
    pub fn is_eoc(value: u32) -> bool {
        value >= FAT_EOC_MIN
    }

    /// 빈 클러스터 수 계산 (느림)
    pub fn count_free_clusters(&self) -> Result<u32, FatError> {
        let mut count = 0;
        for cluster in 2..self.total_clusters + 2 {
            if self.read_entry(cluster)? == FAT_FREE {
                count += 1;
            }
        }
        *self.free_count.lock() = Some(count);
        Ok(count)
    }
}

/// FAT 테이블 에러
#[derive(Debug, Clone, Copy)]
pub enum FatError {
    /// 잘못된 클러스터 번호
    InvalidCluster,
    /// I/O 에러
    IoError,
    /// 공간 부족
    NoSpace,
    /// 손상된 체인
    CorruptedChain,
}
