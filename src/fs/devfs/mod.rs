//! DevFS - 디바이스 파일시스템
//!
//! /dev 디렉토리에 마운트되어 디바이스 파일 제공
//! - /dev/null: 모든 쓰기를 버림, 읽기 시 EOF
//! - /dev/zero: 읽기 시 0 반환, 쓰기 무시
//! - /dev/console: 콘솔 입출력

use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::block::BlockDevice;
use crate::sync::RwLock;

use super::{
    DirEntry, FileMode, FileSystem, FsStats, Stat, VfsError, VfsResult, VNode, VNodeType,
};

/// DevFS 파일시스템
pub struct DevFs {
    /// 루트 디렉토리
    root: Arc<DevFsRoot>,
}

impl DevFs {
    /// 새 DevFS 생성
    pub fn new() -> Arc<Self> {
        let root = DevFsRoot::new();

        // 기본 디바이스 등록
        root.register("null", Arc::new(NullDevice));
        root.register("zero", Arc::new(ZeroDevice));
        root.register("console", Arc::new(ConsoleDevice));

        Arc::new(Self { root })
    }

    /// 디바이스 등록
    pub fn register_device(&self, name: &str, device: Arc<dyn VNode>) {
        self.root.register(name, device);
    }

    /// 디바이스 해제
    pub fn unregister_device(&self, name: &str) {
        self.root.unregister(name);
    }
}

impl FileSystem for DevFs {
    fn name(&self) -> &str {
        "devfs"
    }

    fn root(&self) -> Arc<dyn VNode> {
        self.root.clone()
    }

    fn statfs(&self) -> VfsResult<FsStats> {
        Ok(FsStats {
            fs_type: String::from("devfs"),
            block_size: 512,
            total_blocks: 0,
            free_blocks: 0,
            total_inodes: 0,
            free_inodes: 0,
        })
    }
}

/// DevFS 루트 디렉토리
pub struct DevFsRoot {
    /// 디바이스 목록 - Vec으로 변경하여 BTreeMap 문제 회피
    devices: RwLock<Vec<(String, Arc<dyn VNode>)>>,
}

impl DevFsRoot {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            devices: RwLock::new(Vec::new()),
        })
    }

    fn register(&self, name: &str, device: Arc<dyn VNode>) {
        let mut devices = self.devices.write();
        // 기존 항목이 있으면 교체
        if let Some(pos) = devices.iter().position(|(n, _)| n == name) {
            devices[pos] = (String::from(name), device);
        } else {
            devices.push((String::from(name), device));
        }
    }

    fn unregister(&self, name: &str) {
        let mut devices = self.devices.write();
        if let Some(pos) = devices.iter().position(|(n, _)| n == name) {
            devices.remove(pos);
        }
    }
}

impl VNode for DevFsRoot {
    fn node_type(&self) -> VNodeType {
        VNodeType::Directory
    }

    fn lookup(&self, name: &str) -> VfsResult<Arc<dyn VNode>> {
        let devices = self.devices.read();
        devices.iter()
            .find(|(n, _)| n == name)
            .map(|(_, device)| device.clone())
            .ok_or(VfsError::NotFound)
    }

    fn readdir(&self) -> VfsResult<Vec<DirEntry>> {
        let devices = self.devices.read();

        let entries: Vec<DirEntry> = devices.iter()
            .map(|(name, device)| DirEntry {
                name: name.clone(),
                node_type: device.node_type(),
            })
            .collect();

        Ok(entries)
    }

    fn stat(&self) -> VfsResult<Stat> {
        let devices = self.devices.read();

        Ok(Stat {
            node_type: VNodeType::Directory,
            mode: FileMode::new(0o755),
            size: devices.len() as u64,
            nlink: 2,
            ..Default::default()
        })
    }
}

/// /dev/null - 모든 쓰기를 버림
pub struct NullDevice;

impl VNode for NullDevice {
    fn node_type(&self) -> VNodeType {
        VNodeType::CharDevice
    }

    fn read(&self, _offset: usize, _buf: &mut [u8]) -> VfsResult<usize> {
        Ok(0) // EOF
    }

    fn write(&self, _offset: usize, buf: &[u8]) -> VfsResult<usize> {
        Ok(buf.len()) // 모든 데이터 버림
    }

    fn stat(&self) -> VfsResult<Stat> {
        Ok(Stat {
            node_type: VNodeType::CharDevice,
            mode: FileMode::new(0o666),
            size: 0,
            nlink: 1,
            ..Default::default()
        })
    }
}

/// /dev/zero - 읽기 시 0 반환
pub struct ZeroDevice;

impl VNode for ZeroDevice {
    fn node_type(&self) -> VNodeType {
        VNodeType::CharDevice
    }

    fn read(&self, _offset: usize, buf: &mut [u8]) -> VfsResult<usize> {
        for b in buf.iter_mut() {
            *b = 0;
        }
        Ok(buf.len())
    }

    fn write(&self, _offset: usize, buf: &[u8]) -> VfsResult<usize> {
        Ok(buf.len()) // 무시
    }

    fn stat(&self) -> VfsResult<Stat> {
        Ok(Stat {
            node_type: VNodeType::CharDevice,
            mode: FileMode::new(0o666),
            size: 0,
            nlink: 1,
            ..Default::default()
        })
    }
}

/// /dev/console - 콘솔 입출력
pub struct ConsoleDevice;

impl VNode for ConsoleDevice {
    fn node_type(&self) -> VNodeType {
        VNodeType::CharDevice
    }

    fn read(&self, _offset: usize, buf: &mut [u8]) -> VfsResult<usize> {
        // TODO: UART에서 읽기 구현
        // 현재는 non-blocking으로 0 반환
        Ok(0)
    }

    fn write(&self, _offset: usize, buf: &[u8]) -> VfsResult<usize> {
        // UART로 출력
        for &b in buf {
            crate::console::putc(b);
        }
        Ok(buf.len())
    }

    fn stat(&self) -> VfsResult<Stat> {
        Ok(Stat {
            node_type: VNodeType::CharDevice,
            mode: FileMode::new(0o666),
            size: 0,
            nlink: 1,
            ..Default::default()
        })
    }
}

/// /dev/random - 난수 생성 (간단한 PRNG)
pub struct RandomDevice {
    seed: RwLock<u64>,
}

impl RandomDevice {
    pub fn new() -> Self {
        Self {
            seed: RwLock::new(12345), // TODO: 타이머 등으로 초기화
        }
    }

    fn next(&self) -> u64 {
        let mut seed = self.seed.write();
        // xorshift64
        let mut x = *seed;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *seed = x;
        x
    }
}

impl VNode for RandomDevice {
    fn node_type(&self) -> VNodeType {
        VNodeType::CharDevice
    }

    fn read(&self, _offset: usize, buf: &mut [u8]) -> VfsResult<usize> {
        let mut pos = 0;
        while pos < buf.len() {
            let rand = self.next();
            let bytes = rand.to_le_bytes();
            let to_copy = core::cmp::min(8, buf.len() - pos);
            buf[pos..pos + to_copy].copy_from_slice(&bytes[..to_copy]);
            pos += to_copy;
        }
        Ok(buf.len())
    }

    fn write(&self, _offset: usize, buf: &[u8]) -> VfsResult<usize> {
        // 쓰기는 엔트로피 풀에 추가 (현재는 무시)
        Ok(buf.len())
    }

    fn stat(&self) -> VfsResult<Stat> {
        Ok(Stat {
            node_type: VNodeType::CharDevice,
            mode: FileMode::new(0o666),
            size: 0,
            nlink: 1,
            ..Default::default()
        })
    }
}

/// /dev/vda, /dev/vdb, ... - 블록 디바이스 노드
pub struct BlockDeviceNode {
    /// 블록 디바이스 참조
    device: Arc<dyn BlockDevice>,
    /// 디바이스 이름
    name: String,
}

impl BlockDeviceNode {
    /// 새 블록 디바이스 노드 생성
    pub fn new(device: Arc<dyn BlockDevice>, name: &str) -> Self {
        Self {
            device,
            name: String::from(name),
        }
    }
}

impl VNode for BlockDeviceNode {
    fn node_type(&self) -> VNodeType {
        VNodeType::BlockDevice
    }

    fn read(&self, offset: usize, buf: &mut [u8]) -> VfsResult<usize> {
        let block_size = self.device.block_size();
        let device_size = self.device.capacity() as usize;

        // 범위 확인
        if offset >= device_size {
            return Ok(0); // EOF
        }

        // 읽을 바이트 수 계산
        let bytes_to_read = core::cmp::min(buf.len(), device_size - offset);
        if bytes_to_read == 0 {
            return Ok(0);
        }

        // 시작 블록과 오프셋
        let start_block = offset / block_size;
        let start_offset = offset % block_size;

        // 끝 블록
        let end_block = (offset + bytes_to_read - 1) / block_size;

        let mut block_buf = alloc::vec![0u8; block_size];
        let mut bytes_read = 0;

        for block_num in start_block..=end_block {
            // 블록 읽기
            self.device
                .read_block(block_num as u64, &mut block_buf)
                .map_err(|_| VfsError::IoError)?;

            // 이 블록에서 복사할 범위 계산
            let block_start = if block_num == start_block {
                start_offset
            } else {
                0
            };
            let block_end = if block_num == end_block {
                (offset + bytes_to_read - 1) % block_size + 1
            } else {
                block_size
            };

            let copy_len = block_end - block_start;
            buf[bytes_read..bytes_read + copy_len]
                .copy_from_slice(&block_buf[block_start..block_end]);
            bytes_read += copy_len;
        }

        Ok(bytes_read)
    }

    fn write(&self, offset: usize, buf: &[u8]) -> VfsResult<usize> {
        if self.device.is_read_only() {
            return Err(VfsError::ReadOnly);
        }

        let block_size = self.device.block_size();
        let device_size = self.device.capacity() as usize;

        // 범위 확인
        if offset >= device_size {
            return Err(VfsError::NoSpace);
        }

        // 쓸 바이트 수 계산
        let bytes_to_write = core::cmp::min(buf.len(), device_size - offset);
        if bytes_to_write == 0 {
            return Ok(0);
        }

        // 시작 블록과 오프셋
        let start_block = offset / block_size;
        let start_offset = offset % block_size;

        // 끝 블록
        let end_block = (offset + bytes_to_write - 1) / block_size;

        let mut block_buf = alloc::vec![0u8; block_size];
        let mut bytes_written = 0;

        for block_num in start_block..=end_block {
            // 이 블록에서 쓸 범위 계산
            let block_start = if block_num == start_block {
                start_offset
            } else {
                0
            };
            let block_end = if block_num == end_block {
                (offset + bytes_to_write - 1) % block_size + 1
            } else {
                block_size
            };

            // 부분 쓰기인 경우 먼저 읽기
            if block_start != 0 || block_end != block_size {
                self.device
                    .read_block(block_num as u64, &mut block_buf)
                    .map_err(|_| VfsError::IoError)?;
            }

            // 데이터 복사
            let copy_len = block_end - block_start;
            block_buf[block_start..block_end]
                .copy_from_slice(&buf[bytes_written..bytes_written + copy_len]);

            // 블록 쓰기
            self.device
                .write_block(block_num as u64, &block_buf)
                .map_err(|_| VfsError::IoError)?;

            bytes_written += copy_len;
        }

        Ok(bytes_written)
    }

    fn stat(&self) -> VfsResult<Stat> {
        Ok(Stat {
            node_type: VNodeType::BlockDevice,
            mode: FileMode::new(0o660),
            size: self.device.capacity(),
            nlink: 1,
            blksize: self.device.block_size() as u32,
            blocks: self.device.block_count(),
            ..Default::default()
        })
    }
}

/// DevFS 생성 헬퍼
pub fn create_devfs() -> Arc<DevFs> {
    let devfs = DevFs::new();

    // 추가 디바이스 등록
    devfs.register_device("random", Arc::new(RandomDevice::new()));
    devfs.register_device("urandom", Arc::new(RandomDevice::new()));

    // 전역 참조 설정 (나중에 블록 디바이스 등록 시 사용)
    set_devfs(devfs.clone());

    devfs
}

/// 등록된 블록 디바이스들을 DevFS에 추가
/// 블록 서브시스템 초기화 후 호출해야 함
pub fn register_block_devices_to_devfs() {
    // DevFS가 /dev에 마운트되어 있다고 가정
    if let Ok(dev_node) = crate::fs::lookup_path("/dev") {
        // 블록 서브시스템에서 등록된 디바이스 목록 가져오기
        let devices = crate::block::list_devices();

        for name in devices {
            if let Some(device) = crate::block::get_device(&name) {
                let node = Arc::new(BlockDeviceNode::new(device, &name));
                // DevFS의 root에 직접 등록하기 위해 DevFsRoot를 통해 등록
                // lookup으로 /dev를 찾고, DevFS root에 등록
                if let Ok(_existing) = dev_node.lookup(&name) {
                    // 이미 존재함 - 스킵
                    continue;
                }
                // DevFS root의 devices에 추가하려면 downcast가 필요하지만
                // 현재 구조에서는 어려우므로 전역 DEVFS 참조 사용
                if let Some(devfs) = get_devfs() {
                    devfs.register_device(&name, node);
                    crate::kprintln!("[devfs] Registered block device: /dev/{}", name);
                }
            }
        }
    }
}

/// 전역 DevFS 참조
static DEVFS: RwLock<Option<Arc<DevFs>>> = RwLock::new(None);

/// DevFS 참조 설정
pub fn set_devfs(devfs: Arc<DevFs>) {
    let mut global = DEVFS.write();
    *global = Some(devfs);
}

/// DevFS 참조 가져오기
pub fn get_devfs() -> Option<Arc<DevFs>> {
    let global = DEVFS.read();
    global.clone()
}
