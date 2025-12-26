//! 파일 디스크립터 관리
//!
//! 프로세스별 파일 디스크립터 테이블

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::sync::RwLock;

use super::{VfsError, VfsResult, VNode};

/// 표준 파일 디스크립터
pub const STDIN_FD: i32 = 0;
pub const STDOUT_FD: i32 = 1;
pub const STDERR_FD: i32 = 2;

/// 파일 열기 플래그
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenFlags(pub u32);

impl OpenFlags {
    /// 읽기 전용
    pub const O_RDONLY: u32 = 0;
    /// 쓰기 전용
    pub const O_WRONLY: u32 = 1;
    /// 읽기/쓰기
    pub const O_RDWR: u32 = 2;
    /// 없으면 생성
    pub const O_CREAT: u32 = 0o100;
    /// 이미 있으면 에러
    pub const O_EXCL: u32 = 0o200;
    /// 파일 크기 0으로
    pub const O_TRUNC: u32 = 0o1000;
    /// 추가 모드
    pub const O_APPEND: u32 = 0o2000;
    /// 디렉토리만
    pub const O_DIRECTORY: u32 = 0o200000;

    pub fn new(flags: u32) -> Self {
        Self(flags)
    }

    pub fn is_readable(&self) -> bool {
        let access = self.0 & 3;
        access == Self::O_RDONLY || access == Self::O_RDWR
    }

    pub fn is_writable(&self) -> bool {
        let access = self.0 & 3;
        access == Self::O_WRONLY || access == Self::O_RDWR
    }

    pub fn is_create(&self) -> bool {
        self.0 & Self::O_CREAT != 0
    }

    pub fn is_exclusive(&self) -> bool {
        self.0 & Self::O_EXCL != 0
    }

    pub fn is_truncate(&self) -> bool {
        self.0 & Self::O_TRUNC != 0
    }

    pub fn is_append(&self) -> bool {
        self.0 & Self::O_APPEND != 0
    }

    pub fn is_directory(&self) -> bool {
        self.0 & Self::O_DIRECTORY != 0
    }
}

/// Seek 위치
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeekFrom {
    /// 파일 시작부터
    Start(u64),
    /// 현재 위치부터
    Current(i64),
    /// 파일 끝부터
    End(i64),
}

/// 열린 파일
pub struct OpenFile {
    /// VNode 참조
    pub vnode: Arc<dyn VNode>,
    /// 열기 플래그
    pub flags: OpenFlags,
    /// 현재 오프셋
    pub offset: RwLock<usize>,
}

impl OpenFile {
    /// 새 OpenFile 생성
    pub fn new(vnode: Arc<dyn VNode>, flags: OpenFlags) -> Self {
        Self {
            vnode,
            flags,
            offset: RwLock::new(0),
        }
    }

    /// 읽기
    pub fn read(&self, buf: &mut [u8]) -> VfsResult<usize> {
        if !self.flags.is_readable() {
            return Err(VfsError::PermissionDenied);
        }

        let mut offset = self.offset.write();
        let n = self.vnode.read(*offset, buf)?;
        *offset += n;
        Ok(n)
    }

    /// 쓰기
    pub fn write(&self, buf: &[u8]) -> VfsResult<usize> {
        if !self.flags.is_writable() {
            return Err(VfsError::PermissionDenied);
        }

        let mut offset = self.offset.write();

        // 추가 모드면 파일 끝으로 이동
        if self.flags.is_append() {
            if let Ok(stat) = self.vnode.stat() {
                *offset = stat.size as usize;
            }
        }

        let n = self.vnode.write(*offset, buf)?;
        *offset += n;
        Ok(n)
    }

    /// Seek
    pub fn seek(&self, pos: SeekFrom) -> VfsResult<u64> {
        let mut offset = self.offset.write();

        let new_offset = match pos {
            SeekFrom::Start(n) => n as i64,
            SeekFrom::Current(n) => *offset as i64 + n,
            SeekFrom::End(n) => {
                let stat = self.vnode.stat()?;
                stat.size as i64 + n
            }
        };

        if new_offset < 0 {
            return Err(VfsError::InvalidArgument);
        }

        *offset = new_offset as usize;
        Ok(new_offset as u64)
    }

    /// 현재 오프셋
    pub fn tell(&self) -> u64 {
        *self.offset.read() as u64
    }
}

/// 파일 디스크립터 테이블
pub struct FdTable {
    /// 파일 디스크립터 배열 (None = 미사용)
    files: RwLock<Vec<Option<Arc<OpenFile>>>>,
    /// 최대 파일 디스크립터 수
    max_fds: usize,
}

impl FdTable {
    /// 새 FD 테이블 생성
    pub fn new(max_fds: usize) -> Self {
        Self {
            files: RwLock::new(Vec::new()),
            max_fds,
        }
    }

    /// 기본 FD 테이블 (stdin, stdout, stderr)
    pub fn with_stdio(console: Arc<dyn VNode>) -> Self {
        let table = Self::new(256);

        // stdin (읽기)
        let stdin = OpenFile::new(console.clone(), OpenFlags::new(OpenFlags::O_RDONLY));
        table.insert(Arc::new(stdin));

        // stdout (쓰기)
        let stdout = OpenFile::new(console.clone(), OpenFlags::new(OpenFlags::O_WRONLY));
        table.insert(Arc::new(stdout));

        // stderr (쓰기)
        let stderr = OpenFile::new(console, OpenFlags::new(OpenFlags::O_WRONLY));
        table.insert(Arc::new(stderr));

        table
    }

    /// 새 FD 할당
    pub fn insert(&self, file: Arc<OpenFile>) -> VfsResult<i32> {
        let mut files = self.files.write();

        // 빈 슬롯 찾기
        for (i, slot) in files.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some(file);
                return Ok(i as i32);
            }
        }

        // 새 슬롯 추가
        if files.len() >= self.max_fds {
            return Err(VfsError::NoSpace);
        }

        let fd = files.len() as i32;
        files.push(Some(file));
        Ok(fd)
    }

    /// FD로 파일 가져오기
    pub fn get(&self, fd: i32) -> VfsResult<Arc<OpenFile>> {
        if fd < 0 {
            return Err(VfsError::InvalidArgument);
        }

        let files = self.files.read();
        files.get(fd as usize)
            .and_then(|slot| slot.clone())
            .ok_or(VfsError::InvalidArgument)
    }

    /// FD 닫기
    pub fn close(&self, fd: i32) -> VfsResult<()> {
        if fd < 0 {
            return Err(VfsError::InvalidArgument);
        }

        let mut files = self.files.write();
        if let Some(slot) = files.get_mut(fd as usize) {
            if slot.is_some() {
                *slot = None;
                return Ok(());
            }
        }

        Err(VfsError::InvalidArgument)
    }

    /// FD 복제
    pub fn dup(&self, old_fd: i32) -> VfsResult<i32> {
        let file = self.get(old_fd)?;
        self.insert(file)
    }

    /// FD를 특정 번호로 복제 (dup2)
    pub fn dup2(&self, old_fd: i32, new_fd: i32) -> VfsResult<i32> {
        if new_fd < 0 || new_fd as usize >= self.max_fds {
            return Err(VfsError::InvalidArgument);
        }

        let file = self.get(old_fd)?;

        let mut files = self.files.write();

        // 필요하면 확장
        while files.len() <= new_fd as usize {
            files.push(None);
        }

        // 기존 파일 닫기
        files[new_fd as usize] = Some(file);

        Ok(new_fd)
    }

    /// 열린 FD 수
    pub fn count(&self) -> usize {
        let files = self.files.read();
        files.iter().filter(|f| f.is_some()).count()
    }

    /// 모든 FD 닫기
    pub fn close_all(&self) {
        let mut files = self.files.write();
        files.clear();
    }
}

impl Default for FdTable {
    fn default() -> Self {
        Self::new(256)
    }
}

/// 커널 전역 FD 테이블 (단일 프로세스 환경용)
static KERNEL_FD_TABLE: RwLock<Option<FdTable>> = RwLock::new(None);

/// 커널 FD 테이블 초기화
pub fn init_kernel_fd_table(console: Arc<dyn VNode>) {
    let mut table = KERNEL_FD_TABLE.write();
    *table = Some(FdTable::with_stdio(console));
}

/// 커널 FD 테이블 가져오기
pub fn kernel_fd_table() -> VfsResult<&'static FdTable> {
    // Safety: 초기화 후에는 변경되지 않음
    let table = KERNEL_FD_TABLE.read();
    if table.is_some() {
        // 참조 유지를 위해 unsafe 사용
        Ok(unsafe { &*(table.as_ref().unwrap() as *const FdTable) })
    } else {
        Err(VfsError::NotFound)
    }
}
