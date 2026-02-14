//! 파일 시스템 관련 시스템 콜
//!
//! read, write, open, close, lseek, stat 등

use alloc::format;
use alloc::string::String;
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

const MAX_PATH_LEN: usize = 4096;

const F_DUPFD: i32 = 0;
const F_GETFD: i32 = 1;
const F_SETFD: i32 = 2;
const F_GETFL: i32 = 3;
const F_SETFL: i32 = 4;
const F_DUPFD_CLOEXEC: i32 = 1030;

const TIOCSCTTY: usize = 0x540E;
const TIOCGWINSZ: usize = 0x5413;
const TCGETS: usize = 0x5401;
const TCSETS: usize = 0x5402;
const TCSETSW: usize = 0x5403;
const TCSETSF: usize = 0x5404;

#[repr(C)]
#[derive(Clone, Copy)]
struct LinuxStat {
    st_dev: u64,
    st_ino: u64,
    st_mode: u32,
    st_nlink: u32,
    st_uid: u32,
    st_gid: u32,
    st_rdev: u64,
    st_size: i64,
    st_blksize: i32,
    __pad1: i32,
    st_blocks: i64,
    st_atime: i64,
    st_atime_nsec: u64,
    st_mtime: i64,
    st_mtime_nsec: u64,
    st_ctime: i64,
    st_ctime_nsec: u64,
    __unused4: u32,
    __unused5: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct LinuxTermios {
    c_iflag: u32,
    c_oflag: u32,
    c_cflag: u32,
    c_lflag: u32,
    c_line: u8,
    c_cc: [u8; 19],
    c_ispeed: u32,
    c_ospeed: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct LinuxWinSize {
    ws_row: u16,
    ws_col: u16,
    ws_xpixel: u16,
    ws_ypixel: u16,
}

fn read_c_path(path: *const u8) -> Result<String, isize> {
    if path.is_null() {
        return Err(errno::EFAULT);
    }

    let mut len = 0usize;
    unsafe {
        // SAFETY: syscall 인자로 전달된 NUL 종단 문자열 포인터를 최대 길이 내에서 순회한다.
        while *path.add(len) != 0 {
            len += 1;
            if len > MAX_PATH_LEN {
                return Err(errno::EINVAL);
            }
        }
    }

    let bytes = unsafe {
        // SAFETY: 위에서 길이를 검증했으며, 동일 범위를 read-only slice로 변환한다.
        core::slice::from_raw_parts(path, len)
    };
    let s = core::str::from_utf8(bytes).map_err(|_| errno::EINVAL)?;
    Ok(String::from(s))
}

fn normalize_user_path(path: &str) -> Result<String, isize> {
    if path.is_empty() {
        return Err(errno::EINVAL);
    }

    let abs = if path.starts_with('/') {
        String::from(path)
    } else {
        format!("/{}", path)
    };

    fs::path::normalize(&abs).map_err(vfs_error_to_errno)
}

fn linux_mode_bits(node_type: VNodeType) -> u32 {
    match node_type {
        VNodeType::File => 0o100000,
        VNodeType::Directory => 0o040000,
        VNodeType::CharDevice => 0o020000,
        VNodeType::BlockDevice => 0o060000,
        VNodeType::Symlink => 0o120000,
        VNodeType::Fifo => 0o010000,
        VNodeType::Socket => 0o140000,
    }
}

fn write_linux_stat(stat_buf: *mut u8, stat: &fs::Stat) -> isize {
    if stat_buf.is_null() {
        return errno::EFAULT;
    }

    let linux_stat = LinuxStat {
        st_dev: 0,
        st_ino: 0,
        st_mode: linux_mode_bits(stat.node_type) | (stat.mode.0 & 0o7777),
        st_nlink: stat.nlink,
        st_uid: stat.uid,
        st_gid: stat.gid,
        st_rdev: 0,
        st_size: stat.size as i64,
        st_blksize: stat.blksize as i32,
        __pad1: 0,
        st_blocks: stat.blocks as i64,
        st_atime: stat.atime as i64,
        st_atime_nsec: 0,
        st_mtime: stat.mtime as i64,
        st_mtime_nsec: 0,
        st_ctime: stat.ctime as i64,
        st_ctime_nsec: 0,
        __unused4: 0,
        __unused5: 0,
    };

    unsafe {
        // SAFETY: 사용자 버퍼에 LinuxStat 호환 구조체를 그대로 복사한다.
        core::ptr::write_unaligned(stat_buf as *mut LinuxStat, linux_stat);
    }
    0
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
    let path_owned = match read_c_path(path) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let path_norm = match normalize_user_path(&path_owned) {
        Ok(p) => p,
        Err(e) => return e,
    };

    let open_flags = OpenFlags::new(flags);

    // VNode 찾기
    let vnode = match fs::lookup_path(&path_norm) {
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
                &path_norm
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

/// sys_chdir - 현재 작업 디렉토리 변경
///
/// 현재 커널은 per-process cwd를 보관하지 않으므로,
/// 경로 유효성(디렉토리 존재)만 검사한 뒤 성공 처리한다.
pub fn sys_chdir(path: *const u8) -> isize {
    let path_owned = match read_c_path(path) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let path_norm = match normalize_user_path(&path_owned) {
        Ok(p) => p,
        Err(e) => return e,
    };

    match fs::lookup_path(&path_norm) {
        Ok(v) if v.node_type() == VNodeType::Directory => 0,
        Ok(_) => errno::ENOTDIR,
        Err(e) => vfs_error_to_errno(e),
    }
}

/// sys_faccessat - 파일 접근 권한 확인
///
/// baseline: dirfd/flags는 무시하고 경로 존재/기본 타입 유효성만 확인한다.
pub fn sys_faccessat(_dirfd: i32, path: *const u8, _mode: u32, _flags: u32) -> isize {
    let path_owned = match read_c_path(path) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let path_norm = match normalize_user_path(&path_owned) {
        Ok(p) => p,
        Err(e) => return e,
    };

    match fs::lookup_path(&path_norm) {
        Ok(_) => 0,
        Err(e) => vfs_error_to_errno(e),
    }
}

/// sys_newfstatat - 경로 기반 stat 조회
///
/// dirfd/flags는 10-1C baseline에서 제한적으로 처리한다.
pub fn sys_newfstatat(_dirfd: i32, path: *const u8, stat_buf: *mut u8, _flags: usize) -> isize {
    let path_owned = match read_c_path(path) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let path_norm = match normalize_user_path(&path_owned) {
        Ok(p) => p,
        Err(e) => return e,
    };

    match fs::lookup_path(&path_norm) {
        Ok(vnode) => match vnode.stat() {
            Ok(stat) => write_linux_stat(stat_buf, &stat),
            Err(e) => vfs_error_to_errno(e),
        },
        Err(e) => vfs_error_to_errno(e),
    }
}

/// sys_dup - 파일 디스크립터 복제
pub fn sys_dup(old_fd: i32) -> isize {
    match fd::kernel_fd_table() {
        Ok(table) => match table.dup(old_fd) {
            Ok(new_fd) => new_fd as isize,
            Err(VfsError::InvalidArgument) => errno::EBADF,
            Err(e) => vfs_error_to_errno(e),
        },
        Err(e) => vfs_error_to_errno(e),
    }
}

/// sys_dup3 - 특정 번호로 파일 디스크립터 복제
pub fn sys_dup3(old_fd: i32, new_fd: i32, flags: u32) -> isize {
    if old_fd == new_fd {
        return errno::EINVAL;
    }
    // O_CLOEXEC(0x80000) 이외 플래그는 현재 미지원
    if flags != 0 && flags != 0x80000 {
        return errno::EINVAL;
    }

    match fd::kernel_fd_table() {
        Ok(table) => match table.dup2(old_fd, new_fd) {
            Ok(fd) => fd as isize,
            Err(VfsError::InvalidArgument) => errno::EBADF,
            Err(e) => vfs_error_to_errno(e),
        },
        Err(e) => vfs_error_to_errno(e),
    }
}

/// sys_fcntl - 파일 디스크립터 제어
pub fn sys_fcntl(fd_num: i32, cmd: i32, arg: usize) -> isize {
    let table = match fd::kernel_fd_table() {
        Ok(t) => t,
        Err(e) => return vfs_error_to_errno(e),
    };

    match cmd {
        F_DUPFD | F_DUPFD_CLOEXEC => {
            if arg == 0 {
                sys_dup(fd_num)
            } else {
                // baseline: 최소 fd 요구는 정확히 만족하지 못하며, arg 위치로 복제한다.
                match table.dup2(fd_num, arg as i32) {
                    Ok(fd) => fd as isize,
                    Err(VfsError::InvalidArgument) => errno::EBADF,
                    Err(e) => vfs_error_to_errno(e),
                }
            }
        }
        F_GETFD => {
            if table.get(fd_num).is_err() {
                errno::EBADF
            } else {
                0
            }
        }
        F_SETFD => {
            if table.get(fd_num).is_err() {
                errno::EBADF
            } else {
                0
            }
        }
        F_GETFL => match table.get(fd_num) {
            Ok(file) => file.flags.0 as isize,
            Err(_) => errno::EBADF,
        },
        F_SETFL => {
            if table.get(fd_num).is_err() {
                errno::EBADF
            } else {
                0
            }
        }
        _ => errno::ENOSYS,
    }
}

/// sys_ioctl - 디바이스 제어
///
/// 10-1C baseline: BusyBox init에 필요한 TTY 요청만 최소 지원한다.
pub fn sys_ioctl(fd_num: i32, request: usize, argp: usize) -> isize {
    let table = match fd::kernel_fd_table() {
        Ok(t) => t,
        Err(e) => return vfs_error_to_errno(e),
    };
    if table.get(fd_num).is_err() {
        return errno::EBADF;
    }

    match request {
        TCGETS => {
            if argp == 0 {
                return errno::EFAULT;
            }
            let termios = LinuxTermios {
                c_iflag: 0,
                c_oflag: 0,
                c_cflag: 0,
                c_lflag: 0,
                c_line: 0,
                c_cc: [0; 19],
                c_ispeed: 0,
                c_ospeed: 0,
            };
            unsafe {
                // SAFETY: 사용자 공간이 제공한 버퍼에 termios 구조체를 기록한다.
                core::ptr::write_unaligned(argp as *mut LinuxTermios, termios);
            }
            0
        }
        TCSETS | TCSETSW | TCSETSF | TIOCSCTTY => 0,
        TIOCGWINSZ => {
            if argp == 0 {
                return errno::EFAULT;
            }
            let ws = LinuxWinSize {
                ws_row: 24,
                ws_col: 80,
                ws_xpixel: 0,
                ws_ypixel: 0,
            };
            unsafe {
                // SAFETY: 사용자 공간이 제공한 버퍼에 winsize 구조체를 기록한다.
                core::ptr::write_unaligned(argp as *mut LinuxWinSize, ws);
            }
            0
        }
        _ => errno::ENOTTY,
    }
}

/// sys_fstat - 파일 상태 조회
pub fn sys_fstat(fd: i32, stat_buf: *mut u8) -> isize {
    match fd::kernel_fd_table() {
        Ok(table) => {
            match table.get(fd) {
                Ok(file) => {
                    match file.vnode.stat() {
                        Ok(stat) => write_linux_stat(stat_buf, &stat),
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
    let path_owned = match read_c_path(path) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let path_norm = match normalize_user_path(&path_owned) {
        Ok(p) => p,
        Err(e) => return e,
    };

    let root = match fs::root_fs() {
        Some(fs) => fs.root(),
        None => return errno::EIO,
    };

    match fs::path::resolve_parent(&root, &path_norm) {
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
    let path_owned = match read_c_path(path) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let path_norm = match normalize_user_path(&path_owned) {
        Ok(p) => p,
        Err(e) => return e,
    };

    let root = match fs::root_fs() {
        Some(fs) => fs.root(),
        None => return errno::EIO,
    };

    match fs::path::resolve_parent(&root, &path_norm) {
        Ok((parent, name)) => {
            match parent.unlink(&name) {
                Ok(()) => 0,
                Err(e) => vfs_error_to_errno(e),
            }
        }
        Err(e) => vfs_error_to_errno(e),
    }
}
