//! Virtual File System (VFS)
//!
//! 다양한 파일시스템을 위한 추상화 레이어
//! - FileSystem trait: 파일시스템 인터페이스
//! - VNode trait: 파일/디렉토리 추상화
//! - Mount 테이블: 마운트 포인트 관리

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;

use crate::sync::RwLock;

pub mod path;
pub mod ramfs;
pub mod devfs;
pub mod fat32;
pub mod fd;

/// VFS 에러
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VfsError {
    /// 파일/디렉토리를 찾을 수 없음
    NotFound,
    /// 권한 없음
    PermissionDenied,
    /// 이미 존재함
    AlreadyExists,
    /// 디렉토리가 아님
    NotADirectory,
    /// 디렉토리임 (파일 작업 시)
    IsADirectory,
    /// 디렉토리가 비어있지 않음
    DirectoryNotEmpty,
    /// 잘못된 경로
    InvalidPath,
    /// I/O 에러
    IoError,
    /// 파일시스템이 가득 참
    NoSpace,
    /// 읽기 전용 파일시스템
    ReadOnly,
    /// 지원하지 않는 작업
    NotSupported,
    /// 잘못된 인자
    InvalidArgument,
    /// 파일이 열려있음
    FileBusy,
    /// 마운트 포인트가 아님
    NotMountPoint,
    /// 너무 많은 심볼릭 링크
    SymlinkLoop,
    /// 잘못된 파일시스템 포맷
    InvalidFormat,
    /// 알 수 없는 에러
    Unknown,
}

impl fmt::Display for VfsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VfsError::NotFound => write!(f, "not found"),
            VfsError::PermissionDenied => write!(f, "permission denied"),
            VfsError::AlreadyExists => write!(f, "already exists"),
            VfsError::NotADirectory => write!(f, "not a directory"),
            VfsError::IsADirectory => write!(f, "is a directory"),
            VfsError::DirectoryNotEmpty => write!(f, "directory not empty"),
            VfsError::InvalidPath => write!(f, "invalid path"),
            VfsError::IoError => write!(f, "I/O error"),
            VfsError::NoSpace => write!(f, "no space left"),
            VfsError::ReadOnly => write!(f, "read-only filesystem"),
            VfsError::NotSupported => write!(f, "not supported"),
            VfsError::InvalidArgument => write!(f, "invalid argument"),
            VfsError::FileBusy => write!(f, "file is busy"),
            VfsError::NotMountPoint => write!(f, "not a mount point"),
            VfsError::SymlinkLoop => write!(f, "too many symbolic links"),
            VfsError::InvalidFormat => write!(f, "invalid filesystem format"),
            VfsError::Unknown => write!(f, "unknown error"),
        }
    }
}

/// VFS 결과 타입
pub type VfsResult<T> = Result<T, VfsError>;

/// VNode 타입
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VNodeType {
    /// 일반 파일
    File,
    /// 디렉토리
    Directory,
    /// 심볼릭 링크
    Symlink,
    /// 블록 디바이스
    BlockDevice,
    /// 캐릭터 디바이스
    CharDevice,
    /// FIFO (파이프)
    Fifo,
    /// 소켓
    Socket,
}

/// 파일 권한
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileMode(pub u32);

impl FileMode {
    pub const fn new(mode: u32) -> Self {
        Self(mode & 0o7777)
    }

    /// 기본 파일 권한 (rw-r--r--)
    pub const fn default_file() -> Self {
        Self(0o644)
    }

    /// 기본 디렉토리 권한 (rwxr-xr-x)
    pub const fn default_dir() -> Self {
        Self(0o755)
    }

    pub fn is_readable(&self) -> bool {
        self.0 & 0o400 != 0
    }

    pub fn is_writable(&self) -> bool {
        self.0 & 0o200 != 0
    }

    pub fn is_executable(&self) -> bool {
        self.0 & 0o100 != 0
    }
}

/// 파일 상태 정보
#[derive(Debug, Clone)]
pub struct Stat {
    /// VNode 타입
    pub node_type: VNodeType,
    /// 파일 권한
    pub mode: FileMode,
    /// 파일 크기 (바이트)
    pub size: u64,
    /// 하드 링크 수
    pub nlink: u32,
    /// 소유자 UID
    pub uid: u32,
    /// 그룹 GID
    pub gid: u32,
    /// 블록 크기
    pub blksize: u32,
    /// 할당된 블록 수
    pub blocks: u64,
    /// 접근 시간 (Unix timestamp)
    pub atime: u64,
    /// 수정 시간 (Unix timestamp)
    pub mtime: u64,
    /// 상태 변경 시간 (Unix timestamp)
    pub ctime: u64,
}

impl Default for Stat {
    fn default() -> Self {
        Self {
            node_type: VNodeType::File,
            mode: FileMode::default_file(),
            size: 0,
            nlink: 1,
            uid: 0,
            gid: 0,
            blksize: 512,
            blocks: 0,
            atime: 0,
            mtime: 0,
            ctime: 0,
        }
    }
}

/// 디렉토리 엔트리
#[derive(Debug, Clone)]
pub struct DirEntry {
    /// 파일/디렉토리 이름
    pub name: String,
    /// VNode 타입
    pub node_type: VNodeType,
}

/// VNode trait - 파일/디렉토리 추상화
///
/// 모든 파일시스템 객체는 이 trait을 구현해야 합니다.
pub trait VNode: Send + Sync {
    /// VNode 타입 반환
    fn node_type(&self) -> VNodeType;

    /// 파일 읽기
    ///
    /// `offset`: 읽기 시작 위치
    /// `buf`: 데이터를 저장할 버퍼
    /// 반환: 읽은 바이트 수
    fn read(&self, offset: usize, buf: &mut [u8]) -> VfsResult<usize> {
        Err(VfsError::NotSupported)
    }

    /// 파일 쓰기
    ///
    /// `offset`: 쓰기 시작 위치
    /// `buf`: 쓸 데이터
    /// 반환: 쓴 바이트 수
    fn write(&self, offset: usize, buf: &[u8]) -> VfsResult<usize> {
        Err(VfsError::NotSupported)
    }

    /// 파일 크기 조절
    fn truncate(&self, size: u64) -> VfsResult<()> {
        Err(VfsError::NotSupported)
    }

    /// 디렉토리에서 이름으로 VNode 검색
    fn lookup(&self, name: &str) -> VfsResult<Arc<dyn VNode>> {
        Err(VfsError::NotADirectory)
    }

    /// 디렉토리에 새 파일/디렉토리 생성
    fn create(&self, name: &str, node_type: VNodeType, mode: FileMode) -> VfsResult<Arc<dyn VNode>> {
        Err(VfsError::NotADirectory)
    }

    /// 디렉토리에서 파일 삭제
    fn unlink(&self, name: &str) -> VfsResult<()> {
        Err(VfsError::NotADirectory)
    }

    /// 디렉토리 삭제
    fn rmdir(&self, name: &str) -> VfsResult<()> {
        Err(VfsError::NotADirectory)
    }

    /// 디렉토리 내용 읽기
    fn readdir(&self) -> VfsResult<Vec<DirEntry>> {
        Err(VfsError::NotADirectory)
    }

    /// 파일 상태 정보
    fn stat(&self) -> VfsResult<Stat>;

    /// 권한 변경
    fn chmod(&self, mode: FileMode) -> VfsResult<()> {
        Err(VfsError::NotSupported)
    }

    /// 동기화 (flush)
    fn sync(&self) -> VfsResult<()> {
        Ok(())
    }

    /// 심볼릭 링크 대상 읽기
    fn readlink(&self) -> VfsResult<String> {
        Err(VfsError::InvalidArgument)
    }

    /// 심볼릭 링크 생성
    fn symlink(&self, name: &str, target: &str) -> VfsResult<Arc<dyn VNode>> {
        Err(VfsError::NotSupported)
    }
}

/// FileSystem trait - 파일시스템 추상화
pub trait FileSystem: Send + Sync {
    /// 파일시스템 이름 (예: "ramfs", "fat32")
    fn name(&self) -> &str;

    /// 루트 VNode 반환
    fn root(&self) -> Arc<dyn VNode>;

    /// 파일시스템 동기화
    fn sync(&self) -> VfsResult<()> {
        Ok(())
    }

    /// 파일시스템 정보
    fn statfs(&self) -> VfsResult<FsStats> {
        Err(VfsError::NotSupported)
    }

    /// 언마운트 (정리 작업)
    fn unmount(&self) -> VfsResult<()> {
        Ok(())
    }
}

/// 파일시스템 통계
#[derive(Debug, Clone)]
pub struct FsStats {
    /// 파일시스템 타입
    pub fs_type: String,
    /// 블록 크기
    pub block_size: u64,
    /// 총 블록 수
    pub total_blocks: u64,
    /// 사용 가능한 블록 수
    pub free_blocks: u64,
    /// 총 inode 수
    pub total_inodes: u64,
    /// 사용 가능한 inode 수
    pub free_inodes: u64,
}

/// 마운트 포인트
struct MountPoint {
    /// 마운트 경로
    path: String,
    /// 파일시스템
    fs: Arc<dyn FileSystem>,
}

/// 마운트 테이블
static MOUNT_TABLE: RwLock<Vec<MountPoint>> = RwLock::new(Vec::new());

/// 루트 파일시스템
static ROOT_FS: RwLock<Option<Arc<dyn FileSystem>>> = RwLock::new(None);

/// 루트 파일시스템 설정
pub fn set_root_fs(fs: Arc<dyn FileSystem>) {
    let mut root = ROOT_FS.write();
    *root = Some(fs.clone());

    // 마운트 테이블에도 추가
    let mut mounts = MOUNT_TABLE.write();
    mounts.push(MountPoint {
        path: String::from("/"),
        fs,
    });

    crate::kprintln!("[vfs] Root filesystem mounted");
}

/// 루트 파일시스템 가져오기
pub fn root_fs() -> Option<Arc<dyn FileSystem>> {
    ROOT_FS.read().clone()
}

/// 파일시스템 마운트
pub fn mount(path: &str, fs: Arc<dyn FileSystem>) -> VfsResult<()> {
    if path.is_empty() || !path.starts_with('/') {
        return Err(VfsError::InvalidPath);
    }

    let mut mounts = MOUNT_TABLE.write();

    // 이미 마운트되어 있는지 확인
    if mounts.iter().any(|m| m.path == path) {
        return Err(VfsError::AlreadyExists);
    }

    mounts.push(MountPoint {
        path: String::from(path),
        fs,
    });

    // 경로 길이로 정렬 (긴 경로가 먼저 매칭되도록)
    mounts.sort_by(|a, b| b.path.len().cmp(&a.path.len()));

    crate::kprintln!("[vfs] Mounted filesystem at {}", path);

    Ok(())
}

/// 파일시스템 언마운트
pub fn unmount(path: &str) -> VfsResult<()> {
    let mut mounts = MOUNT_TABLE.write();

    let idx = mounts.iter()
        .position(|m| m.path == path)
        .ok_or(VfsError::NotMountPoint)?;

    // 루트는 언마운트 불가
    if path == "/" {
        return Err(VfsError::FileBusy);
    }

    let mount = &mounts[idx];
    mount.fs.unmount()?;

    mounts.remove(idx);

    crate::kprintln!("[vfs] Unmounted filesystem at {}", path);

    Ok(())
}

/// 경로에 해당하는 파일시스템 찾기
pub fn find_mount(path: &str) -> Option<(Arc<dyn FileSystem>, String)> {
    let mounts = MOUNT_TABLE.read();

    for mount in mounts.iter() {
        if path == mount.path || path.starts_with(&format!("{}/", mount.path)) || mount.path == "/" {
            // 상대 경로 계산
            let relative = if mount.path == "/" {
                String::from(path)
            } else if path == mount.path {
                String::from("/")
            } else {
                String::from(&path[mount.path.len()..])
            };

            return Some((mount.fs.clone(), relative));
        }
    }

    None
}

/// 경로로 VNode 검색
pub fn lookup_path(path: &str) -> VfsResult<Arc<dyn VNode>> {
    let path = path::normalize(path)?;

    let (fs, relative_path) = find_mount(&path)
        .ok_or(VfsError::NotFound)?;

    path::resolve(&fs.root(), &relative_path)
}

/// 마운트 목록 반환
pub fn list_mounts() -> Vec<(String, String)> {
    let mounts = MOUNT_TABLE.read();
    mounts.iter()
        .map(|m| (m.path.clone(), String::from(m.fs.name())))
        .collect()
}

/// VFS 초기화
pub fn init() {
    crate::kprintln!("[vfs] Virtual File System initialized");
}
