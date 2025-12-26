//! RamFS - 메모리 기반 파일시스템
//!
//! 메모리에 파일과 디렉토리를 저장하는 간단한 파일시스템
//! 재부팅 시 데이터가 사라짐

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::sync::RwLock;

use super::{
    DirEntry, FileMode, FileSystem, FsStats, Stat, VfsError, VfsResult, VNode, VNodeType,
};

/// RamFS 파일시스템
pub struct RamFs {
    /// 루트 디렉토리
    root: Arc<RamFsDir>,
    /// 다음 inode 번호
    next_inode: RwLock<u64>,
}

impl RamFs {
    /// 새 RamFS 생성
    pub fn new() -> Arc<Self> {
        let root = Arc::new(RamFsDir::new(String::from("/"), FileMode::default_dir()));

        Arc::new(Self {
            root,
            next_inode: RwLock::new(2), // 1은 루트용
        })
    }

    /// 다음 inode 번호 할당
    fn alloc_inode(&self) -> u64 {
        let mut next = self.next_inode.write();
        let inode = *next;
        *next += 1;
        inode
    }
}

impl FileSystem for RamFs {
    fn name(&self) -> &str {
        "ramfs"
    }

    fn root(&self) -> Arc<dyn VNode> {
        self.root.clone()
    }

    fn sync(&self) -> VfsResult<()> {
        Ok(()) // RAM 기반이므로 동기화 불필요
    }

    fn statfs(&self) -> VfsResult<FsStats> {
        Ok(FsStats {
            fs_type: String::from("ramfs"),
            block_size: 4096,
            total_blocks: 0,     // 무제한
            free_blocks: u64::MAX,
            total_inodes: 0,
            free_inodes: u64::MAX,
        })
    }
}

/// RamFS 디렉토리
pub struct RamFsDir {
    /// 디렉토리 이름
    name: String,
    /// 권한
    mode: RwLock<FileMode>,
    /// 자식 엔트리 (이름, VNode) - Vec으로 변경하여 BTreeMap 문제 회피
    children: RwLock<Vec<(String, Arc<dyn VNode>)>>,
}

impl RamFsDir {
    /// 새 디렉토리 생성
    pub fn new(name: String, mode: FileMode) -> Self {
        Self {
            name,
            mode: RwLock::new(mode),
            children: RwLock::new(Vec::new()),
        }
    }
}

impl VNode for RamFsDir {
    fn node_type(&self) -> VNodeType {
        VNodeType::Directory
    }

    fn lookup(&self, name: &str) -> VfsResult<Arc<dyn VNode>> {
        let children = self.children.read();
        for (child_name, node) in children.iter() {
            if child_name == name {
                return Ok(node.clone());
            }
        }
        Err(VfsError::NotFound)
    }

    fn create(&self, name: &str, node_type: VNodeType, mode: FileMode) -> VfsResult<Arc<dyn VNode>> {
        // 이름 검증
        if name.is_empty() || name.contains('/') {
            return Err(VfsError::InvalidArgument);
        }

        let mut children = self.children.write();

        // 이미 존재하는지 확인
        for (child_name, _) in children.iter() {
            if child_name == name {
                return Err(VfsError::AlreadyExists);
            }
        }

        let node: Arc<dyn VNode> = match node_type {
            VNodeType::File => Arc::new(RamFsFile::new(String::from(name), mode)),
            VNodeType::Directory => Arc::new(RamFsDir::new(String::from(name), mode)),
            _ => return Err(VfsError::NotSupported),
        };

        children.push((String::from(name), node.clone()));

        Ok(node)
    }

    fn unlink(&self, name: &str) -> VfsResult<()> {
        let mut children = self.children.write();

        let index = children.iter().position(|(n, _)| n == name).ok_or(VfsError::NotFound)?;
        let (_, node) = &children[index];

        // 디렉토리인 경우 비어있어야 함
        if node.node_type() == VNodeType::Directory {
            if let Ok(entries) = node.readdir() {
                if !entries.is_empty() {
                    return Err(VfsError::DirectoryNotEmpty);
                }
            }
        }

        children.remove(index);
        Ok(())
    }

    fn readdir(&self) -> VfsResult<Vec<DirEntry>> {
        let children = self.children.read();

        let entries: Vec<DirEntry> = children.iter()
            .map(|(name, node)| DirEntry {
                name: name.clone(),
                node_type: node.node_type(),
            })
            .collect();

        Ok(entries)
    }

    fn stat(&self) -> VfsResult<Stat> {
        let mode = self.mode.read();
        let children = self.children.read();

        Ok(Stat {
            node_type: VNodeType::Directory,
            mode: *mode,
            size: children.len() as u64,
            nlink: 2, // . 및 ..
            ..Default::default()
        })
    }

    fn chmod(&self, mode: FileMode) -> VfsResult<()> {
        let mut m = self.mode.write();
        *m = mode;
        Ok(())
    }
}

/// RamFS 파일
pub struct RamFsFile {
    /// 파일 이름
    name: String,
    /// 권한
    mode: RwLock<FileMode>,
    /// 파일 내용
    data: RwLock<Vec<u8>>,
}

impl RamFsFile {
    /// 새 파일 생성
    pub fn new(name: String, mode: FileMode) -> Self {
        Self {
            name,
            mode: RwLock::new(mode),
            data: RwLock::new(Vec::new()),
        }
    }
}

impl VNode for RamFsFile {
    fn node_type(&self) -> VNodeType {
        VNodeType::File
    }

    fn read(&self, offset: usize, buf: &mut [u8]) -> VfsResult<usize> {
        let data = self.data.read();

        if offset >= data.len() {
            return Ok(0);
        }

        let available = data.len() - offset;
        let to_read = core::cmp::min(buf.len(), available);

        buf[..to_read].copy_from_slice(&data[offset..offset + to_read]);

        Ok(to_read)
    }

    fn write(&self, offset: usize, buf: &[u8]) -> VfsResult<usize> {
        let mut data = self.data.write();

        // 필요하면 크기 확장
        let required_size = offset + buf.len();
        if required_size > data.len() {
            data.resize(required_size, 0);
        }

        data[offset..offset + buf.len()].copy_from_slice(buf);

        Ok(buf.len())
    }

    fn truncate(&self, size: u64) -> VfsResult<()> {
        let mut data = self.data.write();
        data.resize(size as usize, 0);
        Ok(())
    }

    fn stat(&self) -> VfsResult<Stat> {
        let mode = self.mode.read();
        let data = self.data.read();

        Ok(Stat {
            node_type: VNodeType::File,
            mode: *mode,
            size: data.len() as u64,
            nlink: 1,
            blocks: ((data.len() + 511) / 512) as u64,
            ..Default::default()
        })
    }

    fn chmod(&self, mode: FileMode) -> VfsResult<()> {
        let mut m = self.mode.write();
        *m = mode;
        Ok(())
    }

    fn sync(&self) -> VfsResult<()> {
        Ok(()) // RAM 기반이므로 동기화 불필요
    }
}

/// RamFS 심볼릭 링크
pub struct RamFsSymlink {
    /// 링크 이름
    name: String,
    /// 대상 경로
    target: String,
    /// 권한
    mode: FileMode,
}

impl RamFsSymlink {
    /// 새 심볼릭 링크 생성
    pub fn new(name: String, target: String) -> Self {
        Self {
            name,
            target,
            mode: FileMode::new(0o777),
        }
    }
}

impl VNode for RamFsSymlink {
    fn node_type(&self) -> VNodeType {
        VNodeType::Symlink
    }

    fn readlink(&self) -> VfsResult<String> {
        Ok(self.target.clone())
    }

    fn stat(&self) -> VfsResult<Stat> {
        Ok(Stat {
            node_type: VNodeType::Symlink,
            mode: self.mode,
            size: self.target.len() as u64,
            nlink: 1,
            ..Default::default()
        })
    }
}

/// RamFS 생성 헬퍼
pub fn create_ramfs() -> Arc<RamFs> {
    RamFs::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_ramfs_basic() {
        let fs = create_ramfs();
        let root = fs.root();

        // 파일 생성
        let file = root.create("test.txt", VNodeType::File, FileMode::default_file()).unwrap();

        // 쓰기
        file.write(0, b"Hello, World!").unwrap();

        // 읽기
        let mut buf = [0u8; 13];
        let n = file.read(0, &mut buf).unwrap();
        assert_eq!(n, 13);
        assert_eq!(&buf, b"Hello, World!");

        // 디렉토리 읽기
        let entries = root.readdir().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "test.txt");
    }

    fn test_ramfs_directory() {
        let fs = create_ramfs();
        let root = fs.root();

        // 디렉토리 생성
        let dir = root.create("subdir", VNodeType::Directory, FileMode::default_dir()).unwrap();

        // 디렉토리 내에 파일 생성
        dir.create("file.txt", VNodeType::File, FileMode::default_file()).unwrap();

        // lookup 테스트
        let found = root.lookup("subdir").unwrap();
        assert_eq!(found.node_type(), VNodeType::Directory);
    }
}
