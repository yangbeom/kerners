//! FAT32 파일시스템
//!
//! VirtIO 블록 디바이스에서 FAT32 파일시스템 읽기 지원

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::block::BlockDevice;
use crate::sync::RwLock;

use super::{DirEntry, FileMode, FileSystem, FsStats, Stat, VfsError, VfsResult, VNode, VNodeType};

pub mod boot;
pub mod dir;
pub mod fat;

#[inline]
fn is_probably_kernel_ptr(addr: usize) -> bool {
    crate::mm::is_kernel_mapped_addr(addr)
}

static FAT32_READ_LOG_COUNT: AtomicUsize = AtomicUsize::new(0);

/// FAT32 파일시스템
pub struct Fat32FileSystem {
    /// 블록 디바이스
    device: Arc<dyn BlockDevice>,
    /// 부트 섹터 정보
    boot: boot::Fat32BootSector,
    /// 루트 클러스터 번호
    root_cluster: u32,
}

impl Fat32FileSystem {
    /// 새 FAT32 파일시스템 생성
    pub fn new(device: Arc<dyn BlockDevice>, boot: boot::Fat32BootSector) -> Arc<Self> {
        let root_cluster = boot.root_cluster;
        Arc::new(Self {
            device,
            boot,
            root_cluster,
        })
    }

    /// 클러스터 데이터 읽기
    pub fn read_cluster(&self, cluster: u32) -> VfsResult<Vec<u8>> {
        let cluster_size = self.boot.sectors_per_cluster as usize
            * self.boot.bytes_per_sector as usize;
        let mut data = alloc::vec![0u8; cluster_size];

        let start_sector = self.boot.cluster_to_sector(cluster);

        for i in 0..self.boot.sectors_per_cluster {
            let offset = i as usize * self.boot.bytes_per_sector as usize;
            let sector = (start_sector + i as u32) as u64;
            self.device
                .read_block(sector, &mut data[offset..offset + self.boot.bytes_per_sector as usize])
                .map_err(|_| VfsError::IoError)?;
        }

        Ok(data)
    }

    /// FAT 테이블 생성
    pub fn fat_table(&self) -> fat::FatTable {
        fat::FatTable::new(self.device.clone(), &self.boot)
    }

    /// 클러스터 데이터 쓰기
    pub fn write_cluster(&self, cluster: u32, data: &[u8]) -> VfsResult<()> {
        let cluster_size = self.boot.sectors_per_cluster as usize
            * self.boot.bytes_per_sector as usize;

        // 데이터를 클러스터 크기에 맞춤 (패딩 또는 자르기)
        let start_sector = self.boot.cluster_to_sector(cluster);

        for i in 0..self.boot.sectors_per_cluster {
            let offset = i as usize * self.boot.bytes_per_sector as usize;
            let sector = (start_sector + i as u32) as u64;

            // 섹터 버퍼 준비 (데이터가 부족하면 0으로 패딩)
            let sector_size = self.boot.bytes_per_sector as usize;
            let mut sector_buf = alloc::vec![0u8; sector_size];

            if offset < data.len() {
                let copy_len = core::cmp::min(sector_size, data.len() - offset);
                sector_buf[..copy_len].copy_from_slice(&data[offset..offset + copy_len]);
            }

            self.device
                .write_block(sector, &sector_buf)
                .map_err(|_| VfsError::IoError)?;
        }

        Ok(())
    }
}

impl FileSystem for Fat32FileSystem {
    fn name(&self) -> &str {
        "fat32"
    }

    fn root(&self) -> Arc<dyn VNode> {
        Arc::new(Fat32Dir::new_root(self.device.clone(), self.boot, self.root_cluster))
    }

    fn sync(&self) -> VfsResult<()> {
        self.device.sync().map_err(|_| VfsError::IoError)
    }

    fn statfs(&self) -> VfsResult<FsStats> {
        Ok(FsStats {
            fs_type: String::from("fat32"),
            block_size: self.boot.bytes_per_sector as u64,
            total_blocks: self.boot.total_sectors_32 as u64,
            free_blocks: 0, // TODO: FAT 테이블에서 계산
            total_inodes: 0,
            free_inodes: 0,
        })
    }
}

/// FAT32 디렉토리
pub struct Fat32Dir {
    /// 블록 디바이스
    device: Arc<dyn BlockDevice>,
    /// 부트 섹터 정보
    boot: boot::Fat32BootSector,
    /// 시작 클러스터
    cluster: u32,
    /// 디렉토리 이름
    name: String,
}

impl Fat32Dir {
    /// 루트 디렉토리 생성
    pub fn new_root(device: Arc<dyn BlockDevice>, boot: boot::Fat32BootSector, cluster: u32) -> Self {
        Self {
            device,
            boot,
            cluster,
            name: String::from("/"),
        }
    }

    /// 서브디렉토리 생성
    pub fn new(device: Arc<dyn BlockDevice>, boot: boot::Fat32BootSector, cluster: u32, name: String) -> Self {
        Self {
            device,
            boot,
            cluster,
            name,
        }
    }

    /// 클러스터 데이터 읽기
    fn read_cluster_data(&self) -> VfsResult<Vec<u8>> {
        let cluster_size = self.boot.sectors_per_cluster as usize
            * self.boot.bytes_per_sector as usize;
        let mut data = alloc::vec![0u8; cluster_size];

        let start_sector = self.boot.cluster_to_sector(self.cluster);

        for i in 0..self.boot.sectors_per_cluster {
            let offset = i as usize * self.boot.bytes_per_sector as usize;
            let sector = (start_sector + i as u32) as u64;
            self.device
                .read_block(sector, &mut data[offset..offset + self.boot.bytes_per_sector as usize])
                .map_err(|_| VfsError::IoError)?;
        }

        Ok(data)
    }

    /// 모든 클러스터 데이터 읽기 (FAT 체인 따라가기)
    fn read_all_cluster_data(&self) -> VfsResult<Vec<u8>> {
        let fat = fat::FatTable::new(self.device.clone(), &self.boot);
        let (src_data, src_vtable): (usize, usize) = unsafe { core::mem::transmute_copy(&self.device) };
        let (fat_data, fat_vtable) = fat.debug_device_ptrs();
        if src_data != fat_data
            || src_vtable != fat_vtable
            || !is_probably_kernel_ptr(src_data)
            || !is_probably_kernel_ptr(src_vtable)
            || !is_probably_kernel_ptr(fat_data)
            || !is_probably_kernel_ptr(fat_vtable)
        {
            crate::kprintln!(
                "[fat32] read() device ptr mismatch/invalid: src=({:#x},{:#x}) fat=({:#x},{:#x})",
                src_data,
                src_vtable,
                fat_data,
                fat_vtable
            );
            return Err(VfsError::IoError);
        }
        let chain = fat.read_chain(self.cluster).map_err(|_| VfsError::IoError)?;

        let cluster_size = self.boot.sectors_per_cluster as usize
            * self.boot.bytes_per_sector as usize;
        let mut data = Vec::with_capacity(chain.len() * cluster_size);

        for cluster in chain {
            let start_sector = self.boot.cluster_to_sector(cluster);

            for i in 0..self.boot.sectors_per_cluster {
                let mut sector_buf = alloc::vec![0u8; self.boot.bytes_per_sector as usize];
                let sector = (start_sector + i as u32) as u64;
                self.device
                    .read_block(sector, &mut sector_buf)
                    .map_err(|_| VfsError::IoError)?;
                data.extend_from_slice(&sector_buf);
            }
        }

        Ok(data)
    }

    /// 디렉토리 엔트리 파싱 (LFN 포함)
    fn parse_entries(&self) -> VfsResult<Vec<(String, dir::DirEntry)>> {
        self.parse_entries_with_offsets()
            .map(|entries| {
                entries
                    .into_iter()
                    .map(|(name, entry, _, _)| (name, entry))
                    .collect()
            })
    }

    /// 디렉토리 엔트리 파싱 (오프셋 + LFN 길이 포함)
    fn parse_entries_with_offsets(
        &self,
    ) -> VfsResult<Vec<(String, dir::DirEntry, usize, usize)>> {
        let data = self.read_all_cluster_data()?;
        let mut entries = Vec::new();
        let mut lfn_parts: Vec<dir::LfnEntry> = Vec::new();

        for (idx, chunk) in data.chunks(32).enumerate() {
            if chunk[0] == 0x00 {
                break; // 엔트리 끝
            }
            if chunk[0] == 0xE5 {
                lfn_parts.clear();
                continue; // 삭제된 엔트리
            }

            // LFN 엔트리 확인
            if chunk[11] == dir::attr::LONG_NAME {
                if let Some(lfn) = dir::LfnEntry::from_bytes(chunk) {
                    lfn_parts.push(lfn);
                }
                continue;
            }

            // 일반 엔트리
            if let Some(entry) = dir::DirEntry::from_bytes(chunk) {
                if entry.is_volume_label() {
                    lfn_parts.clear();
                    continue;
                }

                // 이름 결정 (LFN이 있으면 사용, 없으면 8.3)
                let name = if !lfn_parts.is_empty() {
                    let long_name = dir::extract_lfn_name(&lfn_parts);
                    let lfn_count = lfn_parts.len();
                    lfn_parts.clear();
                    (long_name, lfn_count)
                } else {
                    (entry.short_name(), 0usize)
                };

                // . 및 .. 건너뛰기
                if name.0 == "." || name.0 == ".." {
                    lfn_parts.clear();
                    continue;
                }

                let offset = idx * 32;
                entries.push((name.0, entry, offset, name.1));
            } else {
                lfn_parts.clear();
            }
        }

        Ok(entries)
    }

    /// 연속된 빈 디렉토리 엔트리 슬롯 찾기
    fn find_free_entry_slots(&self, count: usize) -> VfsResult<usize> {
        if count == 0 {
            return Err(VfsError::InvalidArgument);
        }
        let data = self.read_all_cluster_data()?;
        let mut run_start = 0usize;
        let mut run_len = 0usize;

        for (idx, chunk) in data.chunks(32).enumerate() {
            // 빈 슬롯 (0x00) 또는 삭제된 슬롯 (0xE5)
            if chunk[0] == 0x00 || chunk[0] == 0xE5 {
                if run_len == 0 {
                    run_start = idx;
                }
                run_len += 1;
                if run_len >= count {
                    return Ok(run_start * 32);
                }
            } else {
                run_len = 0;
            }
        }

        // TODO: 디렉토리 클러스터 확장
        Err(VfsError::NoSpace)
    }

    /// 디렉토리 엔트리 쓰기
    fn write_dir_entry(&self, offset: usize, entry: &dir::DirEntry) -> VfsResult<()> {
        self.write_raw_entry(offset, &entry.to_bytes())
    }

    /// 32바이트 raw 엔트리를 디렉토리에 기록
    fn write_raw_entry(&self, offset: usize, raw: &[u8; 32]) -> VfsResult<()> {
        let fat = fat::FatTable::new(self.device.clone(), &self.boot);
        let chain = fat.read_chain(self.cluster).map_err(|_| VfsError::IoError)?;

        let cluster_size = self.boot.sectors_per_cluster as usize
            * self.boot.bytes_per_sector as usize;

        let cluster_idx = offset / cluster_size;
        let offset_in_cluster = offset % cluster_size;

        if cluster_idx >= chain.len() {
            return Err(VfsError::IoError);
        }

        // 클러스터 읽기
        let cluster = chain[cluster_idx];
        let mut data = self.read_cluster_data_for(cluster)?;

        // 엔트리 쓰기
        data[offset_in_cluster..offset_in_cluster + 32].copy_from_slice(raw);

        // 클러스터 쓰기
        self.write_cluster_data(cluster, &data)?;

        Ok(())
    }

    /// 특정 클러스터 데이터 읽기
    fn read_cluster_data_for(&self, cluster: u32) -> VfsResult<Vec<u8>> {
        let cluster_size = self.boot.sectors_per_cluster as usize
            * self.boot.bytes_per_sector as usize;
        let mut data = alloc::vec![0u8; cluster_size];

        let start_sector = self.boot.cluster_to_sector(cluster);

        for i in 0..self.boot.sectors_per_cluster {
            let offset = i as usize * self.boot.bytes_per_sector as usize;
            let sector = (start_sector + i as u32) as u64;
            self.device
                .read_block(sector, &mut data[offset..offset + self.boot.bytes_per_sector as usize])
                .map_err(|_| VfsError::IoError)?;
        }

        Ok(data)
    }

    /// 클러스터 데이터 쓰기
    fn write_cluster_data(&self, cluster: u32, data: &[u8]) -> VfsResult<()> {
        let cluster_size = self.boot.sectors_per_cluster as usize
            * self.boot.bytes_per_sector as usize;

        let start_sector = self.boot.cluster_to_sector(cluster);

        for i in 0..self.boot.sectors_per_cluster {
            let offset = i as usize * self.boot.bytes_per_sector as usize;
            let sector = (start_sector + i as u32) as u64;

            let sector_size = self.boot.bytes_per_sector as usize;
            let mut sector_buf = alloc::vec![0u8; sector_size];

            if offset < data.len() {
                let copy_len = core::cmp::min(sector_size, data.len() - offset);
                sector_buf[..copy_len].copy_from_slice(&data[offset..offset + copy_len]);
            }

            self.device
                .write_block(sector, &sector_buf)
                .map_err(|_| VfsError::IoError)?;
        }

        Ok(())
    }

    /// 엔트리 첫 바이트를 삭제 마커(0xE5)로 설정
    fn mark_entry_deleted(&self, offset: usize) -> VfsResult<()> {
        let fat = fat::FatTable::new(self.device.clone(), &self.boot);
        let chain = fat.read_chain(self.cluster).map_err(|_| VfsError::IoError)?;

        let cluster_size = self.boot.sectors_per_cluster as usize
            * self.boot.bytes_per_sector as usize;
        let cluster_idx = offset / cluster_size;
        let offset_in_cluster = offset % cluster_size;
        if cluster_idx >= chain.len() {
            return Err(VfsError::IoError);
        }

        let cluster = chain[cluster_idx];
        let mut data = self.read_cluster_data_for(cluster)?;
        data[offset_in_cluster] = 0xE5;
        self.write_cluster_data(cluster, &data)?;
        Ok(())
    }

    /// short 엔트리와 연결된 LFN 엔트리들을 삭제 처리
    fn mark_entry_chain_deleted(&self, short_offset: usize, lfn_count: usize) -> VfsResult<()> {
        self.mark_entry_deleted(short_offset)?;
        for idx in 1..=lfn_count {
            let Some(lfn_offset) = short_offset.checked_sub(idx * dir::DirEntry::SIZE) else {
                break;
            };
            self.mark_entry_deleted(lfn_offset)?;
        }
        Ok(())
    }

    /// 디렉토리 하위 엔트리를 재귀적으로 제거한다.
    fn remove_contents_recursive(&self) -> VfsResult<()> {
        let entries = self.parse_entries_with_offsets()?;
        let fat = fat::FatTable::new(self.device.clone(), &self.boot);

        for (name, entry, offset, lfn_count) in entries {
            if entry.is_dir() {
                let child = Fat32Dir::new(
                    self.device.clone(),
                    self.boot,
                    entry.cluster(),
                    name,
                );
                child.remove_contents_recursive()?;
                if entry.cluster() >= 2 {
                    fat.free_chain(entry.cluster()).map_err(|_| VfsError::IoError)?;
                }
            } else if entry.cluster() >= 2 {
                fat.free_chain(entry.cluster()).map_err(|_| VfsError::IoError)?;
            }

            self.mark_entry_chain_deleted(offset, lfn_count)?;
        }

        Ok(())
    }

    /// 빈 파일 생성
    fn create_file(&self, name: &str) -> VfsResult<Arc<dyn VNode>> {
        // 디렉토리 엔트리 생성 (클러스터 없음, 크기 0)
        let mut entry = dir::DirEntry::new_file(name, 0, 0);
        let lfn_entries = if dir::needs_lfn(name) {
            let short = entry.short_name_raw();
            dir::build_lfn_entries(name, &short)
        } else {
            Vec::new()
        };

        // 빈 슬롯 찾기
        let slot_count = lfn_entries.len() + 1;
        let offset = self.find_free_entry_slots(slot_count)?;

        for (idx, lfn) in lfn_entries.iter().enumerate() {
            self.write_raw_entry(offset + idx * dir::DirEntry::SIZE, &lfn.to_bytes())?;
        }
        let short_offset = offset + lfn_entries.len() * dir::DirEntry::SIZE;
        entry.touch_created_now();
        self.write_dir_entry(short_offset, &entry)?;

        // Fat32File 반환
        Ok(Arc::new(Fat32File::new(
            self.device.clone(),
            self.boot,
            0,
            0,
            String::from(name),
            self.cluster,
            short_offset,
        )))
    }

    /// 새 디렉토리 생성
    fn create_directory(&self, name: &str) -> VfsResult<Arc<dyn VNode>> {
        let fat = fat::FatTable::new(self.device.clone(), &self.boot);

        // 새 디렉토리를 위한 클러스터 할당
        let cluster = fat.alloc_cluster().map_err(|_| VfsError::NoSpace)?;

        // 디렉토리 초기화 (. 및 .. 엔트리)
        let cluster_size = self.boot.sectors_per_cluster as usize
            * self.boot.bytes_per_sector as usize;
        let mut data = alloc::vec![0u8; cluster_size];

        // "." 엔트리
        let dot = dir::DirEntry::dot_entry(cluster);
        data[0..32].copy_from_slice(&dot.to_bytes());

        // ".." 엔트리
        let dotdot = dir::DirEntry::dotdot_entry(self.cluster);
        data[32..64].copy_from_slice(&dotdot.to_bytes());

        // 클러스터에 쓰기
        self.write_cluster_data(cluster, &data)?;

        // 부모 디렉토리에 엔트리 추가
        let mut entry = dir::DirEntry::new_dir(name, cluster);
        let lfn_entries = if dir::needs_lfn(name) {
            let short = entry.short_name_raw();
            dir::build_lfn_entries(name, &short)
        } else {
            Vec::new()
        };
        let slot_count = lfn_entries.len() + 1;
        let offset = self.find_free_entry_slots(slot_count)?;
        for (idx, lfn) in lfn_entries.iter().enumerate() {
            self.write_raw_entry(offset + idx * dir::DirEntry::SIZE, &lfn.to_bytes())?;
        }
        let short_offset = offset + lfn_entries.len() * dir::DirEntry::SIZE;
        entry.touch_created_now();
        self.write_dir_entry(short_offset, &entry)?;

        // Fat32Dir 반환
        Ok(Arc::new(Fat32Dir::new(
            self.device.clone(),
            self.boot,
            cluster,
            String::from(name),
        )))
    }
}

impl VNode for Fat32Dir {
    fn node_type(&self) -> VNodeType {
        VNodeType::Directory
    }

    fn lookup(&self, name: &str) -> VfsResult<Arc<dyn VNode>> {
        let entries = self.parse_entries_with_offsets()?;

        for (entry_name, entry, offset, _) in entries {
            // 대소문자 무시 비교
            if entry_name.eq_ignore_ascii_case(name) {
                if entry.is_dir() {
                    return Ok(Arc::new(Fat32Dir::new(
                        self.device.clone(),
                        self.boot,
                        entry.cluster(),
                        entry_name,
                    )));
                } else {
                    return Ok(Arc::new(Fat32File::new(
                        self.device.clone(),
                        self.boot,
                        entry.cluster(),
                        entry.file_size,
                        entry_name,
                        self.cluster,
                        offset,
                    )));
                }
            }
        }

        Err(VfsError::NotFound)
    }

    fn readdir(&self) -> VfsResult<Vec<DirEntry>> {
        let entries = self.parse_entries()?;

        Ok(entries
            .into_iter()
            .map(|(name, entry)| DirEntry {
                name,
                node_type: if entry.is_dir() {
                    VNodeType::Directory
                } else {
                    VNodeType::File
                },
            })
            .collect())
    }

    fn stat(&self) -> VfsResult<Stat> {
        Ok(Stat {
            node_type: VNodeType::Directory,
            mode: FileMode::default_dir(),
            size: 0,
            nlink: 2,
            ..Default::default()
        })
    }

    fn create(&self, name: &str, node_type: VNodeType, _mode: FileMode) -> VfsResult<Arc<dyn VNode>> {
        // 이름 검증
        if name.is_empty() || name.contains('/') || name.len() > 255 {
            return Err(VfsError::InvalidArgument);
        }

        // 이미 존재하는지 확인
        if self.lookup(name).is_ok() {
            return Err(VfsError::AlreadyExists);
        }

        match node_type {
            VNodeType::File => self.create_file(name),
            VNodeType::Directory => self.create_directory(name),
            _ => Err(VfsError::NotSupported),
        }
    }

    fn unlink(&self, name: &str) -> VfsResult<()> {
        // 엔트리 찾기
        let entries = self.parse_entries_with_offsets()?;

        let (offset, lfn_count, entry) = entries
            .iter()
            .find(|(entry_name, _, _, _)| entry_name.eq_ignore_ascii_case(name))
            .map(|(_, entry, offset, lfn_count)| (*offset, *lfn_count, *entry))
            .ok_or(VfsError::NotFound)?;

        // 디렉토리면 에러
        if entry.is_dir() {
            return Err(VfsError::IsADirectory);
        }

        let fat = fat::FatTable::new(self.device.clone(), &self.boot);

        // 클러스터 해제
        if entry.cluster() >= 2 {
            fat.free_chain(entry.cluster()).map_err(|_| VfsError::IoError)?;
        }

        // 디렉토리 엔트리 + 연결된 LFN 엔트리 삭제 마킹
        self.mark_entry_chain_deleted(offset, lfn_count)?;

        Ok(())
    }

    fn rmdir(&self, name: &str) -> VfsResult<()> {
        // 엔트리 찾기
        let entries = self.parse_entries_with_offsets()?;

        let (offset, lfn_count, entry) = entries
            .iter()
            .find(|(entry_name, _, _, _)| entry_name.eq_ignore_ascii_case(name))
            .map(|(_, entry, offset, lfn_count)| (*offset, *lfn_count, *entry))
            .ok_or(VfsError::NotFound)?;

        // 파일이면 에러
        if !entry.is_dir() {
            return Err(VfsError::NotADirectory);
        }

        // 디렉토리 하위 엔트리를 재귀적으로 제거
        let subdir = Fat32Dir::new(
            self.device.clone(),
            self.boot,
            entry.cluster(),
            String::from(name),
        );
        subdir.remove_contents_recursive()?;

        let fat = fat::FatTable::new(self.device.clone(), &self.boot);

        // 클러스터 해제
        if entry.cluster() >= 2 {
            fat.free_chain(entry.cluster()).map_err(|_| VfsError::IoError)?;
        }

        // 디렉토리 엔트리 + 연결된 LFN 엔트리 삭제 마킹
        self.mark_entry_chain_deleted(offset, lfn_count)?;

        Ok(())
    }

    fn sync(&self) -> VfsResult<()> {
        self.device.sync().map_err(|_| VfsError::IoError)
    }
}

/// FAT32 파일
pub struct Fat32File {
    /// 블록 디바이스
    device: Arc<dyn BlockDevice>,
    /// 부트 섹터 정보
    boot: boot::Fat32BootSector,
    /// 시작 클러스터 (가변)
    start_cluster: RwLock<u32>,
    /// 파일 크기 (가변)
    size: RwLock<u32>,
    /// 파일 이름
    name: String,
    /// 부모 디렉토리 클러스터
    parent_cluster: u32,
    /// 부모 디렉토리 내 엔트리 오프셋
    entry_offset: usize,
}

impl Fat32File {
    /// 새 파일 생성
    pub fn new(
        device: Arc<dyn BlockDevice>,
        boot: boot::Fat32BootSector,
        start_cluster: u32,
        size: u32,
        name: String,
        parent_cluster: u32,
        entry_offset: usize,
    ) -> Self {
        Self {
            device,
            boot,
            start_cluster: RwLock::new(start_cluster),
            size: RwLock::new(size),
            name,
            parent_cluster,
            entry_offset,
        }
    }

    /// 클러스터 데이터 읽기
    fn read_cluster(&self, cluster: u32) -> VfsResult<Vec<u8>> {
        let cluster_size = self.boot.sectors_per_cluster as usize
            * self.boot.bytes_per_sector as usize;
        let mut data = alloc::vec![0u8; cluster_size];

        let start_sector = self.boot.cluster_to_sector(cluster);

        for i in 0..self.boot.sectors_per_cluster {
            let offset = i as usize * self.boot.bytes_per_sector as usize;
            let sector = (start_sector + i as u32) as u64;
            self.device
                .read_block(sector, &mut data[offset..offset + self.boot.bytes_per_sector as usize])
                .map_err(|_| VfsError::IoError)?;
        }

        Ok(data)
    }

    /// 클러스터의 일부를 직접 목적 버퍼로 읽기
    ///
    /// 반환값은 실제로 복사한 바이트 수다.
    fn read_cluster_into(
        &self,
        cluster: u32,
        cluster_offset: usize,
        out: &mut [u8],
    ) -> VfsResult<usize> {
        if out.is_empty() {
            return Ok(0);
        }

        let bytes_per_sector = self.boot.bytes_per_sector as usize;
        let cluster_size = self.boot.sectors_per_cluster as usize * bytes_per_sector;
        if cluster_offset >= cluster_size {
            return Ok(0);
        }

        let start_sector = self.boot.cluster_to_sector(cluster) as u64;
        let mut copied = 0usize;
        let mut cur = cluster_offset;

        while cur < cluster_size && copied < out.len() {
            let sector_idx = cur / bytes_per_sector;
            let offset_in_sector = cur % bytes_per_sector;
            let sector = start_sector + sector_idx as u64;

            // 디버깅 안정화: 섹터 버퍼를 항상 힙에 두어 스택 버퍼 경로를 배제한다.
            let mut sector_buf = alloc::vec![0u8; bytes_per_sector];
            self.device
                .read_block(sector, &mut sector_buf)
                .map_err(|_| VfsError::IoError)?;

            let copied_this_sector = {
                let n = core::cmp::min(bytes_per_sector - offset_in_sector, out.len() - copied);
                out[copied..copied + n]
                    .copy_from_slice(&sector_buf[offset_in_sector..offset_in_sector + n]);
                n
            };

            copied += copied_this_sector;
            cur += copied_this_sector;
        }

        Ok(copied)
    }

    /// 클러스터 데이터 쓰기
    fn write_cluster(&self, cluster: u32, data: &[u8]) -> VfsResult<()> {
        let cluster_size = self.boot.sectors_per_cluster as usize
            * self.boot.bytes_per_sector as usize;

        let start_sector = self.boot.cluster_to_sector(cluster);

        for i in 0..self.boot.sectors_per_cluster {
            let offset = i as usize * self.boot.bytes_per_sector as usize;
            let sector = (start_sector + i as u32) as u64;

            let sector_size = self.boot.bytes_per_sector as usize;
            let mut sector_buf = alloc::vec![0u8; sector_size];

            if offset < data.len() {
                let copy_len = core::cmp::min(sector_size, data.len() - offset);
                sector_buf[..copy_len].copy_from_slice(&data[offset..offset + copy_len]);
            }

            self.device
                .write_block(sector, &sector_buf)
                .map_err(|_| VfsError::IoError)?;
        }

        Ok(())
    }

    /// 부모 디렉토리의 엔트리 업데이트
    fn update_dir_entry(&self, new_cluster: u32, new_size: u32) -> VfsResult<()> {
        let fat = fat::FatTable::new(self.device.clone(), &self.boot);
        let chain = fat.read_chain(self.parent_cluster).map_err(|_| VfsError::IoError)?;

        let cluster_size = self.boot.sectors_per_cluster as usize
            * self.boot.bytes_per_sector as usize;

        // 엔트리가 있는 클러스터 찾기
        let cluster_idx = self.entry_offset / cluster_size;
        let offset_in_cluster = self.entry_offset % cluster_size;

        if cluster_idx >= chain.len() {
            return Err(VfsError::IoError);
        }

        // 클러스터 읽기
        let cluster = chain[cluster_idx];
        let mut data = self.read_cluster(cluster)?;

        // 엔트리 수정
        if let Some(mut entry) = dir::DirEntry::from_bytes(&data[offset_in_cluster..]) {
            entry.set_cluster(new_cluster);
            entry.file_size = new_size;
            entry.touch_write_now();
            let entry_bytes = entry.to_bytes();
            data[offset_in_cluster..offset_in_cluster + 32].copy_from_slice(&entry_bytes);

            // 클러스터 쓰기
            self.write_cluster(cluster, &data)?;
        }

        Ok(())
    }
}

impl VNode for Fat32File {
    fn stable_id(&self) -> u64 {
        let (dev_data, dev_vtable): (usize, usize) = unsafe {
            // SAFETY: Arc<dyn BlockDevice> fat pointer를 포인터 크기 튜플로 비트복사한다.
            core::mem::transmute_copy(&self.device)
        };
        let mut id = dev_data as u64;
        id ^= (dev_vtable as u64).rotate_left(13);
        id ^= (self.parent_cluster as u64).rotate_left(29);
        id ^= (self.entry_offset as u64).rotate_left(7);
        id
    }

    fn node_type(&self) -> VNodeType {
        VNodeType::File
    }

    fn read(&self, offset: usize, buf: &mut [u8]) -> VfsResult<usize> {
        let buf_ptr = buf.as_mut_ptr() as usize;
        let log_idx = FAT32_READ_LOG_COUNT.fetch_add(1, Ordering::Relaxed);
        if log_idx < 32 {
            crate::kprintln!(
                "[fat32] read#{} file='{}' offset={} len={} buf={:#x}",
                log_idx,
                self.name,
                offset,
                buf.len(),
                buf_ptr
            );
        }
        if !is_probably_kernel_ptr(buf_ptr) {
            crate::kprintln!(
                "[fat32] invalid read buffer pointer: file='{}' offset={} len={} buf={:#x}",
                self.name,
                offset,
                buf.len(),
                buf_ptr
            );
            return Err(VfsError::IoError);
        }

        if buf.is_empty() {
            return Ok(0);
        }

        let size = *self.size.read();
        let start_cluster = *self.start_cluster.read();

        if offset >= size as usize {
            return Ok(0);
        }

        // 빈 파일 처리
        if start_cluster < 2 {
            return Ok(0);
        }

        let fat = fat::FatTable::new(self.device.clone(), &self.boot);

        let cluster_size = self.boot.sectors_per_cluster as usize
            * self.boot.bytes_per_sector as usize;

        // 읽을 바이트 수 계산
        let bytes_to_read = core::cmp::min(buf.len(), size as usize - offset);
        let mut bytes_read = 0usize;

        // 시작 클러스터 계산 (offset이 포함된 클러스터까지 FAT 체인 순회)
        let mut current_cluster = start_cluster;
        let mut skip_clusters = offset / cluster_size;
        while skip_clusters > 0 {
            let (src_data_now, src_vtable_now): (usize, usize) =
                unsafe { core::mem::transmute_copy(&self.device) };
            let (fat_data_now, fat_vtable_now) = fat.debug_device_ptrs();
            if src_data_now != fat_data_now
                || src_vtable_now != fat_vtable_now
                || !is_probably_kernel_ptr(src_data_now)
                || !is_probably_kernel_ptr(src_vtable_now)
                || !is_probably_kernel_ptr(fat_data_now)
                || !is_probably_kernel_ptr(fat_vtable_now)
            {
                crate::kprintln!(
                    "[fat32] read(skip) device ptr mismatch/invalid: src=({:#x},{:#x}) fat=({:#x},{:#x}) skip_clusters={} current_cluster={}",
                    src_data_now,
                    src_vtable_now,
                    fat_data_now,
                    fat_vtable_now,
                    skip_clusters,
                    current_cluster
                );
                return Err(VfsError::IoError);
            }
            let next = fat.read_entry(current_cluster).map_err(|_| VfsError::IoError)?;
            if next < 2 || next >= fat::FAT_RESERVED_MIN {
                return Ok(bytes_read);
            }
            current_cluster = next;
            skip_clusters -= 1;
        }

        // 현재 클러스터 내 시작 오프셋
        let mut cluster_offset = offset % cluster_size;

        loop {
            if bytes_read >= bytes_to_read {
                break;
            }

            // 클러스터의 필요한 구간만 직접 복사
            let copied = self.read_cluster_into(
                current_cluster,
                cluster_offset,
                &mut buf[bytes_read..bytes_to_read],
            )?;
            if copied == 0 {
                break;
            }
            bytes_read += copied;

            if bytes_read >= bytes_to_read {
                break;
            }

            // 다음 클러스터로 이동
            let (src_data_now, src_vtable_now): (usize, usize) =
                unsafe { core::mem::transmute_copy(&self.device) };
            let (fat_data_now, fat_vtable_now) = fat.debug_device_ptrs();
            if src_data_now != fat_data_now
                || src_vtable_now != fat_vtable_now
                || !is_probably_kernel_ptr(src_data_now)
                || !is_probably_kernel_ptr(src_vtable_now)
                || !is_probably_kernel_ptr(fat_data_now)
                || !is_probably_kernel_ptr(fat_vtable_now)
            {
                crate::kprintln!(
                    "[fat32] read(loop) device ptr mismatch/invalid: src=({:#x},{:#x}) fat=({:#x},{:#x}) bytes_read={} bytes_to_read={} cluster={}",
                    src_data_now,
                    src_vtable_now,
                    fat_data_now,
                    fat_vtable_now,
                    bytes_read,
                    bytes_to_read,
                    current_cluster
                );
                break;
            }
            let next = fat.read_entry(current_cluster).map_err(|_| VfsError::IoError)?;
            if next < 2 || next >= fat::FAT_RESERVED_MIN {
                break;
            }
            current_cluster = next;
            cluster_offset = 0;
        }

        Ok(bytes_read)
    }

    fn write(&self, offset: usize, buf: &[u8]) -> VfsResult<usize> {
        if buf.is_empty() {
            return Ok(0);
        }

        let fat = fat::FatTable::new(self.device.clone(), &self.boot);
        let cluster_size = self.boot.sectors_per_cluster as usize
            * self.boot.bytes_per_sector as usize;

        let mut start_cluster = self.start_cluster.write();
        let mut size = self.size.write();

        let end_offset = offset + buf.len();

        // 빈 파일이면 첫 클러스터 할당
        if *start_cluster < 2 {
            let new_cluster = fat.alloc_cluster().map_err(|_| VfsError::NoSpace)?;
            *start_cluster = new_cluster;
        }

        // 기존 체인 읽기
        let mut chain = fat.read_chain(*start_cluster).map_err(|_| VfsError::IoError)?;

        // 필요한 클러스터 수 계산
        let required_clusters = (end_offset + cluster_size - 1) / cluster_size;

        // 체인 확장 필요 시
        if required_clusters > chain.len() {
            let additional = required_clusters - chain.len();
            let last = *chain.last().unwrap();
            let new_clusters = fat.extend_chain(last, additional).map_err(|_| VfsError::NoSpace)?;
            chain.extend(new_clusters);
        }

        // 데이터 쓰기
        let mut bytes_written = 0;
        let start_cluster_idx = offset / cluster_size;
        let mut cluster_offset = offset % cluster_size;

        for &cluster in chain.iter().skip(start_cluster_idx) {
            if bytes_written >= buf.len() {
                break;
            }

            // 클러스터 데이터 읽기 (Read-Modify-Write)
            let mut cluster_data = self.read_cluster(cluster)?;

            // 데이터 수정
            let copy_len = core::cmp::min(cluster_size - cluster_offset, buf.len() - bytes_written);
            cluster_data[cluster_offset..cluster_offset + copy_len]
                .copy_from_slice(&buf[bytes_written..bytes_written + copy_len]);

            // 클러스터 쓰기
            self.write_cluster(cluster, &cluster_data)?;

            bytes_written += copy_len;
            cluster_offset = 0;
        }

        // 파일 크기 업데이트
        if end_offset > *size as usize {
            *size = end_offset as u32;
        }

        // 디렉토리 엔트리 업데이트
        self.update_dir_entry(*start_cluster, *size)?;

        Ok(bytes_written)
    }

    fn truncate(&self, new_size: u64) -> VfsResult<()> {
        let fat = fat::FatTable::new(self.device.clone(), &self.boot);
        let cluster_size = self.boot.sectors_per_cluster as usize
            * self.boot.bytes_per_sector as usize;

        let mut start_cluster = self.start_cluster.write();
        let mut size = self.size.write();

        let new_size = new_size as u32;

        if new_size == 0 {
            // 모든 클러스터 해제
            if *start_cluster >= 2 {
                fat.free_chain(*start_cluster).map_err(|_| VfsError::IoError)?;
                *start_cluster = 0;
            }
            *size = 0;
        } else if new_size < *size {
            // 축소
            let keep_clusters = (new_size as usize + cluster_size - 1) / cluster_size;
            if *start_cluster >= 2 {
                fat.truncate_chain(*start_cluster, keep_clusters).map_err(|_| VfsError::IoError)?;
            }
            *size = new_size;
        } else if new_size > *size {
            // 확장 (0으로 채워진 클러스터 추가)
            if *start_cluster < 2 {
                let new_cluster = fat.alloc_cluster().map_err(|_| VfsError::NoSpace)?;
                *start_cluster = new_cluster;
            }

            let required_clusters = (new_size as usize + cluster_size - 1) / cluster_size;
            let chain = fat.read_chain(*start_cluster).map_err(|_| VfsError::IoError)?;

            if required_clusters > chain.len() {
                let additional = required_clusters - chain.len();
                let last = *chain.last().unwrap();
                fat.extend_chain(last, additional).map_err(|_| VfsError::NoSpace)?;
            }
            *size = new_size;
        }

        // 디렉토리 엔트리 업데이트
        self.update_dir_entry(*start_cluster, *size)?;

        Ok(())
    }

    fn stat(&self) -> VfsResult<Stat> {
        let size = *self.size.read();
        Ok(Stat {
            node_type: VNodeType::File,
            mode: FileMode::default_file(),
            size: size as u64,
            nlink: 1,
            blksize: self.boot.bytes_per_sector as u32,
            blocks: ((size as u64 + 511) / 512),
            ..Default::default()
        })
    }

    fn sync(&self) -> VfsResult<()> {
        self.device.sync().map_err(|_| VfsError::IoError)
    }
}

/// FAT32 마운트
pub fn mount_fat32(device: Arc<dyn BlockDevice>) -> VfsResult<Arc<dyn FileSystem>> {
    // 부트 섹터 읽기
    let block_size = device.block_size();
    let mut buf = alloc::vec![0u8; block_size];

    device
        .read_block(0, &mut buf)
        .map_err(|_| VfsError::IoError)?;

    // 부트 섹터 파싱
    let boot = boot::Fat32BootSector::from_bytes(&buf)
        .ok_or(VfsError::InvalidFormat)?;

    if !boot.is_valid() {
        return Err(VfsError::InvalidFormat);
    }

    crate::kprintln!("[FAT32] Volume: {}, {} MB",
        boot.volume_label_str(),
        (boot.total_clusters() as u64 * boot.sectors_per_cluster as u64 * boot.bytes_per_sector as u64) / (1024 * 1024)
    );

    let fs = Fat32FileSystem::new(device, boot);
    Ok(fs)
}
