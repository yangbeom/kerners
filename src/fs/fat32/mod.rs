//! FAT32 파일시스템
//!
//! VirtIO 블록 디바이스에서 FAT32 파일시스템 읽기 지원

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::block::BlockDevice;
use crate::sync::RwLock;

use super::{DirEntry, FileMode, FileSystem, FsStats, Stat, VfsError, VfsResult, VNode, VNodeType};

pub mod boot;
pub mod dir;
pub mod fat;

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
            .map(|entries| entries.into_iter().map(|(name, entry, _)| (name, entry)).collect())
    }

    /// 디렉토리 엔트리 파싱 (오프셋 포함)
    fn parse_entries_with_offsets(&self) -> VfsResult<Vec<(String, dir::DirEntry, usize)>> {
        let data = self.read_all_cluster_data()?;
        let mut entries = Vec::new();
        let mut lfn_parts: Vec<dir::LfnEntry> = Vec::new();

        for (idx, chunk) in data.chunks(32).enumerate() {
            if chunk[0] == 0x00 {
                break; // 엔트리 끝
            }
            if chunk[0] == 0xE5 {
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
                    lfn_parts.clear();
                    long_name
                } else {
                    entry.short_name()
                };

                // . 및 .. 건너뛰기
                if name == "." || name == ".." {
                    continue;
                }

                let offset = idx * 32;
                entries.push((name, entry, offset));
            } else {
                lfn_parts.clear();
            }
        }

        Ok(entries)
    }

    /// 빈 디렉토리 엔트리 슬롯 찾기
    fn find_free_entry_slot(&self) -> VfsResult<usize> {
        let data = self.read_all_cluster_data()?;

        for (idx, chunk) in data.chunks(32).enumerate() {
            // 빈 슬롯 (0x00) 또는 삭제된 슬롯 (0xE5)
            if chunk[0] == 0x00 || chunk[0] == 0xE5 {
                return Ok(idx * 32);
            }
        }

        // TODO: 디렉토리 클러스터 확장
        Err(VfsError::NoSpace)
    }

    /// 디렉토리 엔트리 쓰기
    fn write_dir_entry(&self, offset: usize, entry: &dir::DirEntry) -> VfsResult<()> {
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
        let entry_bytes = entry.to_bytes();
        data[offset_in_cluster..offset_in_cluster + 32].copy_from_slice(&entry_bytes);

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

    /// 빈 파일 생성
    fn create_file(&self, name: &str) -> VfsResult<Arc<dyn VNode>> {
        // 디렉토리 엔트리 생성 (클러스터 없음, 크기 0)
        let entry = dir::DirEntry::new_file(name, 0, 0);

        // 빈 슬롯 찾기
        let offset = self.find_free_entry_slot()?;

        // 엔트리 쓰기
        self.write_dir_entry(offset, &entry)?;

        // Fat32File 반환
        Ok(Arc::new(Fat32File::new(
            self.device.clone(),
            self.boot,
            0,
            0,
            String::from(name),
            self.cluster,
            offset,
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
        let entry = dir::DirEntry::new_dir(name, cluster);
        let offset = self.find_free_entry_slot()?;
        self.write_dir_entry(offset, &entry)?;

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

        for (entry_name, entry, offset) in entries {
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

        let (offset, entry) = entries
            .iter()
            .find(|(entry_name, _, _)| entry_name.eq_ignore_ascii_case(name))
            .map(|(_, entry, offset)| (*offset, *entry))
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

        // 디렉토리 엔트리 삭제 마킹
        let mut deleted_entry = entry;
        deleted_entry.mark_deleted();
        self.write_dir_entry(offset, &deleted_entry)?;

        Ok(())
    }

    fn rmdir(&self, name: &str) -> VfsResult<()> {
        // 엔트리 찾기
        let entries = self.parse_entries_with_offsets()?;

        let (offset, entry) = entries
            .iter()
            .find(|(entry_name, _, _)| entry_name.eq_ignore_ascii_case(name))
            .map(|(_, entry, offset)| (*offset, *entry))
            .ok_or(VfsError::NotFound)?;

        // 파일이면 에러
        if !entry.is_dir() {
            return Err(VfsError::NotADirectory);
        }

        // 디렉토리가 비어있는지 확인
        let subdir = Fat32Dir::new(
            self.device.clone(),
            self.boot,
            entry.cluster(),
            String::from(name),
        );

        let subdir_entries = subdir.readdir()?;
        if !subdir_entries.is_empty() {
            return Err(VfsError::DirectoryNotEmpty);
        }

        let fat = fat::FatTable::new(self.device.clone(), &self.boot);

        // 클러스터 해제
        fat.free_chain(entry.cluster()).map_err(|_| VfsError::IoError)?;

        // 디렉토리 엔트리 삭제 마킹
        let mut deleted_entry = entry;
        deleted_entry.mark_deleted();
        self.write_dir_entry(offset, &deleted_entry)?;

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
            let entry_bytes = entry.to_bytes();
            data[offset_in_cluster..offset_in_cluster + 32].copy_from_slice(&entry_bytes);

            // 클러스터 쓰기
            self.write_cluster(cluster, &data)?;
        }

        Ok(())
    }
}

impl VNode for Fat32File {
    fn node_type(&self) -> VNodeType {
        VNodeType::File
    }

    fn read(&self, offset: usize, buf: &mut [u8]) -> VfsResult<usize> {
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
        let chain = fat.read_chain(start_cluster).map_err(|_| VfsError::IoError)?;

        let cluster_size = self.boot.sectors_per_cluster as usize
            * self.boot.bytes_per_sector as usize;

        // 읽을 바이트 수 계산
        let bytes_to_read = core::cmp::min(buf.len(), size as usize - offset);
        let mut bytes_read = 0;

        // 시작 클러스터 및 오프셋 계산
        let start_cluster_idx = offset / cluster_size;
        let mut cluster_offset = offset % cluster_size;

        for (_i, &cluster) in chain.iter().enumerate().skip(start_cluster_idx) {
            if bytes_read >= bytes_to_read {
                break;
            }

            // 클러스터 데이터 읽기
            let cluster_data = self.read_cluster(cluster)?;

            // 데이터 복사
            let copy_start = cluster_offset;
            let copy_len = core::cmp::min(cluster_size - copy_start, bytes_to_read - bytes_read);

            buf[bytes_read..bytes_read + copy_len]
                .copy_from_slice(&cluster_data[copy_start..copy_start + copy_len]);

            bytes_read += copy_len;
            cluster_offset = 0; // 첫 클러스터 이후는 0부터 시작
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
