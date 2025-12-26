//! 파일 시스템 관련 시스템 콜
//!
//! read, write, open, close, lseek, stat 등

use crate::console;
use crate::fs::{self, VfsError, VNodeType, FileMode};
use crate::fs::fd::{self, OpenFlags, SeekFrom};
use super::errno;

/// VFS 에러를 errno로 변환
fn vfs_error_to_errno(e: VfsError) -> isize {
    match e {
        VfsError::NotFound => errno::ENOENT,
        VfsError::PermissionDenied => errno::EACCES,
        VfsError::AlreadyExists => errno::EBUSY,
        VfsError::NotADirectory => errno::ENOTDIR,
        VfsError::IsADirectory => errno::EISDIR,
        VfsError::IoError => errno::EIO,
        VfsError::NoSpace => errno::ENOMEM,
        VfsError::ReadOnly => errno::EACCES,
        VfsError::NotSupported => errno::ENOSYS,
        VfsError::InvalidArgument => errno::EINVAL,
        _ => errno::EIO,
    }
}

/// sys_write - 파일 디스크립터에 쓰기
///
/// # Arguments
/// * `fd` - 파일 디스크립터 (0=stdin, 1=stdout, 2=stderr)
/// * `buf` - 버퍼 포인터
/// * `count` - 쓸 바이트 수
///
/// # Returns
/// * 성공: 쓴 바이트 수
/// * 실패: 음수 에러 코드
pub fn sys_write(fd: usize, buf: *const u8, count: usize) -> isize {
    // 버퍼 유효성 검사 (간단한 null 체크)
    if buf.is_null() {
        return errno::EFAULT;
    }

    // VFS가 초기화되었으면 FD 테이블 사용
    if let Ok(fd_table) = fd::kernel_fd_table() {
        if let Ok(file) = fd_table.get(fd as i32) {
            let slice = unsafe { core::slice::from_raw_parts(buf, count) };
            match file.write(slice) {
                Ok(n) => return n as isize,
                Err(e) => return vfs_error_to_errno(e),
            }
        }
    }

    // 폴백: 기존 콘솔 출력
    match fd {
        1 | 2 => {
            // stdout (1) 또는 stderr (2) - 콘솔 출력
            for i in 0..count {
                let c = unsafe { *buf.add(i) };
                console::putc(c);
            }
            count as isize
        }
        _ => {
            // 지원하지 않는 fd
            errno::ENOENT
        }
    }
}

/// sys_read - 파일 디스크립터에서 읽기
///
/// # Arguments
/// * `fd` - 파일 디스크립터
/// * `buf` - 버퍼 포인터
/// * `count` - 읽을 최대 바이트 수
///
/// # Returns
/// * 성공: 읽은 바이트 수
/// * 실패: 음수 에러 코드
pub fn sys_read(fd: usize, buf: *mut u8, count: usize) -> isize {
    if buf.is_null() {
        return errno::EFAULT;
    }

    // VFS가 초기화되었으면 FD 테이블 사용
    if let Ok(fd_table) = fd::kernel_fd_table() {
        if let Ok(file) = fd_table.get(fd as i32) {
            let slice = unsafe { core::slice::from_raw_parts_mut(buf, count) };
            match file.read(slice) {
                Ok(n) => return n as isize,
                Err(e) => return vfs_error_to_errno(e),
            }
        }
    }

    // 폴백: 기존 콘솔 입력
    match fd {
        0 => {
            // stdin - 콘솔 입력 (한 문자만 읽기)
            if count == 0 {
                return 0;
            }

            // 폴링 방식으로 한 문자 읽기
            loop {
                if let Some(c) = crate::arch::uart::getc() {
                    unsafe {
                        *buf = c;
                    }
                    return 1;
                }
                // CPU 양보
                core::hint::spin_loop();
            }
        }
        _ => errno::ENOENT,
    }
}

/// sys_open - 파일 열기
///
/// # Arguments
/// * `path` - 경로 (null-terminated)
/// * `flags` - 열기 플래그
/// * `mode` - 생성 시 권한
///
/// # Returns
/// * 성공: 파일 디스크립터
/// * 실패: 음수 에러 코드
pub fn sys_open(path: *const u8, flags: u32, mode: u32) -> isize {
    if path.is_null() {
        return errno::EFAULT;
    }

    // 경로 문자열 추출
    let path_str = unsafe {
        let mut len = 0;
        while *path.add(len) != 0 {
            len += 1;
            if len > 4096 {
                return errno::EINVAL;
            }
        }
        core::str::from_utf8_unchecked(core::slice::from_raw_parts(path, len))
    };

    let open_flags = OpenFlags::new(flags);

    // VNode 찾기
    let vnode = match fs::lookup_path(path_str) {
        Ok(v) => {
            // 디렉토리 전용 플래그 체크
            if open_flags.is_directory() && v.node_type() != VNodeType::Directory {
                return errno::ENOTDIR;
            }
            v
        }
        Err(VfsError::NotFound) if open_flags.is_create() => {
            // 파일 생성
            let (parent, name) = match fs::path::resolve_parent(
                &fs::root_fs().unwrap().root(),
                path_str
            ) {
                Ok(p) => p,
                Err(e) => return vfs_error_to_errno(e),
            };

            match parent.create(&name, VNodeType::File, FileMode::new(mode)) {
                Ok(v) => v,
                Err(e) => return vfs_error_to_errno(e),
            }
        }
        Err(e) => return vfs_error_to_errno(e),
    };

    // 파일 열기
    let open_file = fd::OpenFile::new(vnode, open_flags);

    // Truncate 처리
    if open_flags.is_truncate() && open_flags.is_writable() {
        let _ = open_file.vnode.truncate(0);
    }

    // FD 테이블에 추가
    match fd::kernel_fd_table() {
        Ok(table) => {
            match table.insert(alloc::sync::Arc::new(open_file)) {
                Ok(fd) => fd as isize,
                Err(e) => vfs_error_to_errno(e),
            }
        }
        Err(e) => vfs_error_to_errno(e),
    }
}

/// sys_close - 파일 닫기
pub fn sys_close(fd: i32) -> isize {
    match fd::kernel_fd_table() {
        Ok(table) => {
            match table.close(fd) {
                Ok(()) => 0,
                Err(e) => vfs_error_to_errno(e),
            }
        }
        Err(e) => vfs_error_to_errno(e),
    }
}

/// sys_lseek - 파일 오프셋 이동
///
/// # Arguments
/// * `fd` - 파일 디스크립터
/// * `offset` - 오프셋
/// * `whence` - 기준 (0=SEEK_SET, 1=SEEK_CUR, 2=SEEK_END)
pub fn sys_lseek(fd: i32, offset: i64, whence: i32) -> isize {
    let seek_from = match whence {
        0 => SeekFrom::Start(offset as u64), // SEEK_SET
        1 => SeekFrom::Current(offset),       // SEEK_CUR
        2 => SeekFrom::End(offset),           // SEEK_END
        _ => return errno::EINVAL,
    };

    match fd::kernel_fd_table() {
        Ok(table) => {
            match table.get(fd) {
                Ok(file) => {
                    match file.seek(seek_from) {
                        Ok(pos) => pos as isize,
                        Err(e) => vfs_error_to_errno(e),
                    }
                }
                Err(e) => vfs_error_to_errno(e),
            }
        }
        Err(e) => vfs_error_to_errno(e),
    }
}

/// sys_fstat - 파일 상태 조회
pub fn sys_fstat(fd: i32, stat_buf: *mut u8) -> isize {
    if stat_buf.is_null() {
        return errno::EFAULT;
    }

    match fd::kernel_fd_table() {
        Ok(table) => {
            match table.get(fd) {
                Ok(file) => {
                    match file.vnode.stat() {
                        Ok(stat) => {
                            // 간단한 stat 구조체 (64바이트)
                            // TODO: Linux 호환 stat 구조체 구현
                            let out = unsafe { core::slice::from_raw_parts_mut(stat_buf, 64) };
                            // size at offset 0
                            out[0..8].copy_from_slice(&stat.size.to_le_bytes());
                            // mode at offset 8
                            out[8..12].copy_from_slice(&stat.mode.0.to_le_bytes());
                            0
                        }
                        Err(e) => vfs_error_to_errno(e),
                    }
                }
                Err(e) => vfs_error_to_errno(e),
            }
        }
        Err(e) => vfs_error_to_errno(e),
    }
}

/// sys_mkdir - 디렉토리 생성
pub fn sys_mkdir(path: *const u8, mode: u32) -> isize {
    if path.is_null() {
        return errno::EFAULT;
    }

    let path_str = unsafe {
        let mut len = 0;
        while *path.add(len) != 0 {
            len += 1;
        }
        core::str::from_utf8_unchecked(core::slice::from_raw_parts(path, len))
    };

    let root = match fs::root_fs() {
        Some(fs) => fs.root(),
        None => return errno::EIO,
    };

    match fs::path::resolve_parent(&root, path_str) {
        Ok((parent, name)) => {
            match parent.create(&name, VNodeType::Directory, FileMode::new(mode)) {
                Ok(_) => 0,
                Err(e) => vfs_error_to_errno(e),
            }
        }
        Err(e) => vfs_error_to_errno(e),
    }
}

/// sys_unlink - 파일 삭제
pub fn sys_unlink(path: *const u8) -> isize {
    if path.is_null() {
        return errno::EFAULT;
    }

    let path_str = unsafe {
        let mut len = 0;
        while *path.add(len) != 0 {
            len += 1;
        }
        core::str::from_utf8_unchecked(core::slice::from_raw_parts(path, len))
    };

    let root = match fs::root_fs() {
        Some(fs) => fs.root(),
        None => return errno::EIO,
    };

    match fs::path::resolve_parent(&root, path_str) {
        Ok((parent, name)) => {
            match parent.unlink(&name) {
                Ok(()) => 0,
                Err(e) => vfs_error_to_errno(e),
            }
        }
        Err(e) => vfs_error_to_errno(e),
    }
}
