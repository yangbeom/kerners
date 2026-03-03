//! 파일 시스템 관련 시스템 콜
//!
//! read, write, open, close, lseek, stat 등

use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use crate::console;
use crate::fs::{self, VfsError, VNodeType, FileMode};
use crate::fs::fd::{self, OpenFlags, SeekFrom};
use crate::proc;
use crate::sync::Mutex;
use super::{errno, uaccess};

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
static CURRENT_CWD: Mutex<Option<String>> = Mutex::new(None);

fn current_fd_table() -> Result<Arc<fd::FdTable>, VfsError> {
    let files_group = super::process::current_files_group();
    match fd::fd_table_for_group(files_group) {
        Ok(table) => Ok(table),
        Err(VfsError::NotFound) => fd::fd_table_for_group(0),
        Err(e) => Err(e),
    }
}

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
const PIPE_ALLOWED_FLAGS: u32 = 0x800 | 0x80000; // O_NONBLOCK | O_CLOEXEC
const AT_REMOVEDIR: u32 = 0x200;
const PPOLL_MAX_FDS: usize = 1024;
const PSELECT_MAX_FDS: usize = 1024;
const SENDFILE_CHUNK_SIZE: usize = 256 * 1024;
const IOV_MAX: usize = 1024;
const EPOLL_MAX_EVENTS: usize = 1024;

const EPOLL_CLOEXEC: u32 = 0x80000;
const EPOLL_CTL_ADD: i32 = 1;
const EPOLL_CTL_DEL: i32 = 2;
const EPOLL_CTL_MOD: i32 = 3;

const EPOLLIN: u32 = 0x0001;
const EPOLLPRI: u32 = 0x0002;
const EPOLLOUT: u32 = 0x0004;
const EPOLLERR: u32 = 0x0008;
const EPOLLHUP: u32 = 0x0010;

const LINUX_EPOLL_EVENT_SIZE: usize = 12;

const POLLIN: i16 = 0x0001;
const POLLPRI: i16 = 0x0002;
const POLLOUT: i16 = 0x0004;
const POLLERR: i16 = 0x0008;
const POLLHUP: i16 = 0x0010;
const POLLNVAL: i16 = 0x0020;

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

#[repr(C)]
#[derive(Clone, Copy)]
struct LinuxStatFs {
    f_type: i64,
    f_bsize: i64,
    f_blocks: u64,
    f_bfree: u64,
    f_bavail: u64,
    f_files: u64,
    f_ffree: u64,
    f_fsid: [i32; 2],
    f_namelen: i64,
    f_frsize: i64,
    f_flags: i64,
    f_spare: [i64; 4],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct LinuxPollFd {
    fd: i32,
    events: i16,
    revents: i16,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct LinuxTimespec {
    tv_sec: i64,
    tv_nsec: i64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct LinuxIovec {
    iov_base: usize,
    iov_len: usize,
}

#[derive(Clone, Copy)]
struct EpollRegistration {
    fd: i32,
    events: u32,
    data: u64,
}

struct EpollInstance {
    watches: Vec<EpollRegistration>,
}

static EPOLL_INSTANCES: Mutex<Vec<(u64, i32, EpollInstance)>> = Mutex::new(Vec::new());

const DT_UNKNOWN: u8 = 0;
const DT_FIFO: u8 = 1;
const DT_CHR: u8 = 2;
const DT_DIR: u8 = 4;
const DT_BLK: u8 = 6;
const DT_REG: u8 = 8;
const DT_LNK: u8 = 10;
const DT_SOCK: u8 = 12;

fn read_c_path(path: *const u8) -> Result<String, isize> {
    match uaccess::read_c_string(path, MAX_PATH_LEN) {
        Ok(s) => Ok(s),
        Err(e) if e == errno::E2BIG => Err(errno::EINVAL),
        Err(e) => Err(e),
    }
}

fn normalize_user_path(path: &str) -> Result<String, isize> {
    if path.is_empty() {
        return Err(errno::EINVAL);
    }

    let abs = if path.starts_with('/') {
        String::from(path)
    } else {
        let cwd = current_cwd();
        if cwd == "/" {
            format!("/{}", path)
        } else {
            format!("{}/{}", cwd, path)
        }
    };

    fs::path::normalize(&abs).map_err(vfs_error_to_errno)
}

fn current_cwd() -> String {
    let cwd = CURRENT_CWD.lock();
    match cwd.as_ref() {
        Some(path) if !path.is_empty() => path.clone(),
        _ => String::from("/"),
    }
}

struct EpollVNode;

impl fs::VNode for EpollVNode {
    fn node_type(&self) -> VNodeType {
        VNodeType::File
    }

    fn stat(&self) -> fs::VfsResult<fs::Stat> {
        Ok(fs::Stat {
            node_type: VNodeType::File,
            mode: FileMode::new(0o600),
            ..fs::Stat::default()
        })
    }
}

fn alloc_zeroed_user_buffer(len: usize) -> Result<Vec<u8>, isize> {
    let mut out = Vec::new();
    if out.try_reserve_exact(len).is_err() {
        return Err(errno::ENOMEM);
    }
    out.resize(len, 0);
    Ok(out)
}

fn fdset_byte_len(nfds: usize) -> Result<usize, isize> {
    nfds.checked_add(7)
        .map(|v| v / 8)
        .ok_or(errno::EINVAL)
}

fn fdset_is_set(set: &[u8], fd: usize) -> bool {
    let byte = fd / 8;
    let bit = fd % 8;
    if byte >= set.len() {
        return false;
    }
    set[byte] & (1u8 << bit) != 0
}

fn fdset_set(set: &mut [u8], fd: usize) {
    let byte = fd / 8;
    let bit = fd % 8;
    if byte < set.len() {
        set[byte] |= 1u8 << bit;
    }
}

fn parse_timeout_timespec_deadline(timeout: *const u8) -> Result<Option<u64>, isize> {
    if timeout.is_null() {
        return Ok(None);
    }

    let ts = uaccess::read_unaligned(timeout as *const LinuxTimespec)?;
    if ts.tv_sec < 0 || ts.tv_nsec < 0 || ts.tv_nsec >= 1_000_000_000 {
        return Err(errno::EINVAL);
    }
    let timeout_ns = (ts.tv_sec as u64)
        .saturating_mul(1_000_000_000)
        .saturating_add(ts.tv_nsec as u64);
    Ok(Some(
        crate::time::monotonic_now_ns().saturating_add(timeout_ns),
    ))
}

fn read_linux_epoll_event(event: *const u8) -> Result<(u32, u64), isize> {
    if event.is_null() {
        return Err(errno::EFAULT);
    }
    let mut raw = [0u8; LINUX_EPOLL_EVENT_SIZE];
    uaccess::copy_from_user(&mut raw, event)?;
    let events = u32::from_ne_bytes([raw[0], raw[1], raw[2], raw[3]]);
    let data = u64::from_ne_bytes([
        raw[4], raw[5], raw[6], raw[7], raw[8], raw[9], raw[10], raw[11],
    ]);
    Ok((events, data))
}

fn write_linux_epoll_event(dst: *mut u8, events: u32, data: u64) -> Result<(), isize> {
    let mut raw = [0u8; LINUX_EPOLL_EVENT_SIZE];
    raw[0..4].copy_from_slice(&events.to_ne_bytes());
    raw[4..12].copy_from_slice(&data.to_ne_bytes());
    uaccess::copy_to_user(dst, &raw)
}

fn epoll_instance_index(instances: &[(u64, i32, EpollInstance)], files_group: u64, epfd: i32) -> Option<usize> {
    instances
        .iter()
        .position(|(group, owned_epfd, _)| *group == files_group && *owned_epfd == epfd)
}

fn epoll_cleanup_for_closed_fd(files_group: u64, fd: i32) {
    let mut instances = EPOLL_INSTANCES.lock();
    instances.retain(|(group, epfd, _)| !(*group == files_group && *epfd == fd));
    for (group, _epfd, instance) in instances.iter_mut() {
        if *group == files_group {
            instance.watches.retain(|watch| watch.fd != fd);
        }
    }
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

fn linux_d_type(node_type: VNodeType) -> u8 {
    match node_type {
        VNodeType::File => DT_REG,
        VNodeType::Directory => DT_DIR,
        VNodeType::Symlink => DT_LNK,
        VNodeType::BlockDevice => DT_BLK,
        VNodeType::CharDevice => DT_CHR,
        VNodeType::Fifo => DT_FIFO,
        VNodeType::Socket => DT_SOCK,
    }
}

#[inline]
fn align_up(value: usize, align: usize) -> usize {
    (value + align - 1) & !(align - 1)
}

fn fs_magic(fs_type: &str) -> i64 {
    match fs_type {
        "ramfs" => 0x8584_58f6,
        "fat32" => 0x4d44,
        "devfs" => 0x1373,
        "procfs" => 0x9fa0,
        _ => {
            let mut hash = 0u32;
            for b in fs_type.as_bytes() {
                hash = hash.wrapping_mul(33).wrapping_add(*b as u32);
            }
            hash as i64
        }
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

    match uaccess::write_unaligned(stat_buf as *mut LinuxStat, linux_stat) {
        Ok(()) => 0,
        Err(e) => e,
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
    if buf.is_null() {
        return errno::EFAULT;
    }
    if count == 0 {
        return 0;
    }

    if let Ok(fd_table) = current_fd_table() {
        if let Ok(file) = fd_table.get(fd as i32) {
            let chunk_size = core::cmp::min(count, SENDFILE_CHUNK_SIZE);
            let mut chunk = Vec::new();
            if chunk.try_reserve_exact(chunk_size).is_err() {
                return errno::ENOMEM;
            }
            chunk.resize(chunk_size, 0);

            let mut total_written = 0usize;
            while total_written < count {
                let to_copy = core::cmp::min(chunk_size, count - total_written);
                let src_addr = match (buf as usize).checked_add(total_written) {
                    Some(v) => v as *const u8,
                    None => {
                        return if total_written > 0 {
                            total_written as isize
                        } else {
                            errno::EFAULT
                        };
                    }
                };
                if let Err(e) = uaccess::copy_from_user(&mut chunk[..to_copy], src_addr) {
                    return if total_written > 0 {
                        total_written as isize
                    } else {
                        e
                    };
                }

                let written = match file.write(&chunk[..to_copy]) {
                    Ok(n) => n,
                    Err(e) => {
                        return if total_written > 0 {
                            total_written as isize
                        } else {
                            vfs_error_to_errno(e)
                        };
                    }
                };
                total_written += written;
                if written != to_copy {
                    break;
                }
            }
            return total_written as isize;
        }
    }

    match fd {
        1 | 2 => {
            let mut written = 0usize;
            for i in 0..count {
                let c_addr = match (buf as usize).checked_add(i) {
                    Some(v) => v as *const u8,
                    None => {
                        return if written > 0 {
                            written as isize
                        } else {
                            errno::EFAULT
                        };
                    }
                };
                let c = match uaccess::read_byte(c_addr) {
                    Ok(c) => c,
                    Err(e) => {
                        return if written > 0 {
                            written as isize
                        } else {
                            e
                        };
                    }
                };
                console::putc(c);
                written += 1;
            }
            written as isize
        }
        _ => errno::ENOENT,
    }
}

/// sys_writev - scatter/gather 쓰기
///
/// iovec 배열을 순회하며 각 버퍼를 기존 sys_write 경로로 전달한다.
/// 일부 버퍼 기록 이후 오류가 발생하면 Linux와 동일하게 누적 기록량을 우선 반환한다.
pub fn sys_writev(fd: i32, iov: *const u8, iovcnt: i32) -> isize {
    if fd < 0 {
        return errno::EBADF;
    }
    if iovcnt < 0 || iovcnt as usize > IOV_MAX {
        return errno::EINVAL;
    }
    if iovcnt == 0 {
        return 0;
    }
    if iov.is_null() {
        return errno::EFAULT;
    }

    let mut total_written = 0isize;
    let iov_base = iov as usize;
    let iov_ent_size = core::mem::size_of::<LinuxIovec>();
    for idx in 0..iovcnt as usize {
        let entry_addr = match iov_base.checked_add(idx * iov_ent_size) {
            Some(v) => v as *const LinuxIovec,
            None => {
                return if total_written > 0 {
                    total_written
                } else {
                    errno::EFAULT
                };
            }
        };
        let entry = match uaccess::read_unaligned(entry_addr) {
            Ok(entry) => entry,
            Err(e) => {
                return if total_written > 0 {
                    total_written
                } else {
                    e
                };
            }
        };
        if entry.iov_len == 0 {
            continue;
        }
        if entry.iov_base == 0 {
            return if total_written > 0 {
                total_written
            } else {
                errno::EFAULT
            };
        }

        let written = sys_write(fd as usize, entry.iov_base as *const u8, entry.iov_len);
        if written < 0 {
            return if total_written > 0 {
                total_written
            } else {
                written
            };
        }
        total_written = match total_written.checked_add(written) {
            Some(v) => v,
            None => return errno::EINVAL,
        };

        if written as usize != entry.iov_len {
            break;
        }
    }

    total_written
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
    if count == 0 {
        return 0;
    }

    if let Ok(fd_table) = current_fd_table() {
        if let Ok(file) = fd_table.get(fd as i32) {
            let chunk_len = core::cmp::min(count, SENDFILE_CHUNK_SIZE);
            let mut tmp = Vec::new();
            if tmp.try_reserve_exact(chunk_len).is_err() {
                return errno::ENOMEM;
            }
            tmp.resize(chunk_len, 0);

            match file.read(&mut tmp[..]) {
                Ok(n) => {
                    if n == 0 {
                        return 0;
                    }
                    match uaccess::copy_to_user(buf, &tmp[..n]) {
                        Ok(()) => return n as isize,
                        Err(e) => return e,
                    }
                }
                Err(e) => return vfs_error_to_errno(e),
            }
        }
    }

    match fd {
        0 => {
            loop {
                if let Some(c) = crate::arch::uart::getc() {
                    if let Err(e) = uaccess::write_byte(buf, c) {
                        return e;
                    }
                    return 1;
                }
                core::hint::spin_loop();
            }
        }
        _ => errno::ENOENT,
    }
}

/// sys_sendfile - 파일 디스크립터 간 데이터 전송
///
/// baseline:
/// - in_fd에서 읽어 out_fd로 순차 기록한다.
/// - `offset`이 non-null이면 입력 FD의 현재 오프셋은 보존하고, 해당 위치를 갱신한다.
/// - 부분 전송 후 오류가 발생하면 이미 전송된 바이트 수를 우선 반환한다.
pub fn sys_sendfile(out_fd: i32, in_fd: i32, offset: *mut i64, count: usize) -> isize {
    if count == 0 {
        return 0;
    }

    let table = match current_fd_table() {
        Ok(t) => t,
        Err(e) => return vfs_error_to_errno(e),
    };

    let out_file = match table.get(out_fd) {
        Ok(f) => f,
        Err(_) => return errno::EBADF,
    };
    let in_file = match table.get(in_fd) {
        Ok(f) => f,
        Err(_) => return errno::EBADF,
    };

    if !out_file.flags.is_writable() || !in_file.flags.is_readable() {
        return errno::EBADF;
    }

    let use_explicit_offset = !offset.is_null();
    let mut explicit_in_pos = if use_explicit_offset {
        let initial = match uaccess::read_unaligned(offset) {
            Ok(v) => v,
            Err(e) => return e,
        };
        if initial < 0 {
            return errno::EINVAL;
        }
        initial as usize
    } else {
        0
    };

    let mut remaining = count;
    let mut moved = 0usize;
    let mut buf = Vec::new();
    if buf.try_reserve_exact(SENDFILE_CHUNK_SIZE).is_err() {
        return errno::ENOMEM;
    }
    buf.resize(SENDFILE_CHUNK_SIZE, 0);

    while remaining > 0 {
        let to_read = core::cmp::min(remaining, buf.len());
        let read_res = if use_explicit_offset {
            in_file.vnode.read(explicit_in_pos, &mut buf[..to_read])
        } else {
            in_file.read(&mut buf[..to_read])
        };

        let nread = match read_res {
            Ok(0) => break, // EOF
            Ok(n) => n,
            Err(e) => {
                if moved > 0 {
                    break;
                }
                return vfs_error_to_errno(e);
            }
        };

        if use_explicit_offset {
            explicit_in_pos = explicit_in_pos.saturating_add(nread);
        }

        let mut written_from_chunk = 0usize;
        while written_from_chunk < nread {
            let write_res = out_file.write(&buf[written_from_chunk..nread]);
            let nwritten = match write_res {
                Ok(0) => {
                    if moved > 0 {
                        break;
                    }
                    return errno::EIO;
                }
                Ok(n) => n,
                Err(e) => {
                    if moved > 0 {
                        break;
                    }
                    return vfs_error_to_errno(e);
                }
            };

            written_from_chunk += nwritten;
            moved += nwritten;
            remaining = remaining.saturating_sub(nwritten);
        }

        if written_from_chunk < nread {
            break;
        }
    }

    if use_explicit_offset {
        if let Err(e) = uaccess::write_unaligned(offset, explicit_in_pos as i64) {
            return e;
        }
    }

    moved as isize
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
            let (parent, name) = match fs::resolve_parent_path(&path_norm) {
                Ok(p) => p,
                Err(e) => return vfs_error_to_errno(e),
            };
            let create_mode = mode & !super::process::current_umask();

            match parent.create(&name, VNodeType::File, FileMode::new(create_mode)) {
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
    match current_fd_table() {
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
    match current_fd_table() {
        Ok(table) => {
            match table.close(fd) {
                Ok(()) => {
                    let files_group = super::process::current_files_group();
                    epoll_cleanup_for_closed_fd(files_group, fd);
                    0
                }
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

    match current_fd_table() {
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
/// 현재 구현은 per-process cwd 대신 커널 전역 cwd baseline을 갱신한다.
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
        Ok(v) if v.node_type() == VNodeType::Directory => {
            *CURRENT_CWD.lock() = Some(path_norm);
            0
        }
        Ok(_) => errno::ENOTDIR,
        Err(e) => vfs_error_to_errno(e),
    }
}

/// sys_getcwd - 현재 작업 디렉토리 조회
///
/// 현재 구현은 프로세스별 cwd 대신 커널 전역 cwd baseline을 반환한다.
/// 성공 시 NUL 종료 문자열 길이(terminator 포함)를 반환한다.
pub fn sys_getcwd(buf: *mut u8, size: usize) -> isize {
    if buf.is_null() {
        return errno::EFAULT;
    }
    if size == 0 {
        return errno::EINVAL;
    }

    let cwd = current_cwd();
    let bytes = cwd.as_bytes();
    let required = bytes.len() + 1; // trailing NUL
    if required > size {
        return errno::ERANGE;
    }

    if let Err(e) = uaccess::copy_to_user(buf, bytes) {
        return e;
    }
    let nul_ptr = match (buf as usize).checked_add(bytes.len()) {
        Some(v) => v as *mut u8,
        None => return errno::EFAULT,
    };
    if let Err(e) = uaccess::write_byte(nul_ptr, 0) {
        return e;
    }
    required as isize
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
    match current_fd_table() {
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

    match current_fd_table() {
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
    let table = match current_fd_table() {
        Ok(t) => t,
        Err(e) => return vfs_error_to_errno(e),
    };

    match cmd {
        F_DUPFD | F_DUPFD_CLOEXEC => {
            if table.get(fd_num).is_err() {
                return errno::EBADF;
            }
            if arg > i32::MAX as usize {
                return errno::EINVAL;
            }

            match table.dup_from_min(fd_num, arg as i32) {
                Ok(fd) => fd as isize,
                Err(VfsError::InvalidArgument) => errno::EINVAL,
                Err(VfsError::NoSpace) => errno::EMFILE,
                Err(e) => vfs_error_to_errno(e),
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
    let table = match current_fd_table() {
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
            if let Err(e) = uaccess::write_unaligned(argp as *mut LinuxTermios, termios) {
                return e;
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
            if let Err(e) = uaccess::write_unaligned(argp as *mut LinuxWinSize, ws) {
                return e;
            }
            0
        }
        _ => errno::ENOTTY,
    }
}

/// sys_fstat - 파일 상태 조회
pub fn sys_fstat(fd: i32, stat_buf: *mut u8) -> isize {
    match current_fd_table() {
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

/// sys_getdents64 - 디렉토리 엔트리 읽기
pub fn sys_getdents64(fd_num: i32, dirp: *mut u8, count: usize) -> isize {
    if dirp.is_null() {
        return errno::EFAULT;
    }
    if count < 24 {
        return errno::EINVAL;
    }

    let table = match current_fd_table() {
        Ok(t) => t,
        Err(e) => return vfs_error_to_errno(e),
    };
    let file = match table.get(fd_num) {
        Ok(f) => f,
        Err(_) => return errno::EBADF,
    };
    if file.vnode.node_type() != VNodeType::Directory {
        return errno::ENOTDIR;
    }

    let entries = match file.vnode.readdir() {
        Ok(v) => v,
        Err(e) => return vfs_error_to_errno(e),
    };

    let mut cursor = file.offset.write();
    if *cursor > entries.len() {
        *cursor = entries.len();
    }

    let mut written = 0usize;
    let mut idx = *cursor;
    let mut record = Vec::new();

    while idx < entries.len() {
        let entry = &entries[idx];
        let name = entry.name.as_bytes();
        let reclen = align_up(19 + name.len() + 1, 8);
        if reclen > u16::MAX as usize {
            return errno::EINVAL;
        }
        if written + reclen > count {
            break;
        }

        let ino = match file.vnode.lookup(&entry.name) {
            Ok(node) => node.stable_id(),
            Err(_) => 0,
        };
        let d_off = (idx + 1) as i64;
        let d_type = linux_d_type(entry.node_type);

        if record.try_reserve_exact(reclen).is_err() {
            return if written > 0 {
                written as isize
            } else {
                errno::ENOMEM
            };
        }
        record.clear();
        record.resize(reclen, 0);
        record[0..8].copy_from_slice(&ino.to_le_bytes());
        record[8..16].copy_from_slice(&d_off.to_le_bytes());
        record[16..18].copy_from_slice(&(reclen as u16).to_le_bytes());
        record[18] = d_type;
        record[19..(19 + name.len())].copy_from_slice(name);

        let rec = match (dirp as usize).checked_add(written) {
            Some(v) => v as *mut u8,
            None => {
                return if written > 0 {
                    written as isize
                } else {
                    errno::EFAULT
                };
            }
        };
        if let Err(e) = uaccess::copy_to_user(rec, &record[..]) {
            return if written > 0 { written as isize } else { e };
        }

        written += reclen;
        idx += 1;
    }

    *cursor = idx;
    written as isize
}

/// sys_pipe2 - 익명 파이프 생성
pub fn sys_pipe2(pipefd: *mut i32, flags: u32) -> isize {
    if pipefd.is_null() {
        return errno::EFAULT;
    }
    if flags & !PIPE_ALLOWED_FLAGS != 0 {
        return errno::EINVAL;
    }

    let table = match current_fd_table() {
        Ok(t) => t,
        Err(e) => return vfs_error_to_errno(e),
    };

    let (read_vnode, write_vnode) = fs::pipe::create_pipe_pair();
    let read_open = Arc::new(fd::OpenFile::new(
        read_vnode,
        OpenFlags::new((flags & PIPE_ALLOWED_FLAGS) | OpenFlags::O_RDONLY),
    ));
    let write_open = Arc::new(fd::OpenFile::new(
        write_vnode,
        OpenFlags::new((flags & PIPE_ALLOWED_FLAGS) | OpenFlags::O_WRONLY),
    ));

    let read_fd = match table.insert(read_open) {
        Ok(fd_num) => fd_num,
        Err(e) => return vfs_error_to_errno(e),
    };
    let write_fd = match table.insert(write_open) {
        Ok(fd_num) => fd_num,
        Err(e) => {
            let _ = table.close(read_fd);
            return vfs_error_to_errno(e);
        }
    };

    if let Err(e) = uaccess::write_unaligned(pipefd, read_fd) {
        let _ = table.close(read_fd);
        let _ = table.close(write_fd);
        return e;
    }
    let write_ptr = match (pipefd as usize).checked_add(core::mem::size_of::<i32>()) {
        Some(v) => v as *mut i32,
        None => {
            let _ = table.close(read_fd);
            let _ = table.close(write_fd);
            return errno::EFAULT;
        }
    };
    if let Err(e) = uaccess::write_unaligned(write_ptr, write_fd) {
        let _ = table.close(read_fd);
        let _ = table.close(write_fd);
        return e;
    }

    0
}

/// sys_readlinkat - 심볼릭 링크 대상 읽기
///
/// baseline: dirfd는 무시하고 경로 기준으로 처리한다.
pub fn sys_readlinkat(_dirfd: i32, path: *const u8, buf: *mut u8, bufsiz: usize) -> isize {
    if buf.is_null() {
        return errno::EFAULT;
    }
    if bufsiz == 0 {
        return errno::EINVAL;
    }

    let path_owned = match read_c_path(path) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let path_norm = match normalize_user_path(&path_owned) {
        Ok(p) => p,
        Err(e) => return e,
    };

    let vnode = match fs::lookup_path(&path_norm) {
        Ok(v) => v,
        Err(e) => return vfs_error_to_errno(e),
    };
    let target = match vnode.readlink() {
        Ok(s) => s,
        Err(e) => return vfs_error_to_errno(e),
    };

    let bytes = target.as_bytes();
    let to_copy = core::cmp::min(bytes.len(), bufsiz);
    if let Err(e) = uaccess::copy_to_user(buf, &bytes[..to_copy]) {
        return e;
    }
    to_copy as isize
}

/// sys_statfs - 파일시스템 통계 조회
pub fn sys_statfs(path: *const u8, buf: *mut u8) -> isize {
    if buf.is_null() {
        return errno::EFAULT;
    }

    let path_owned = match read_c_path(path) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let path_norm = match normalize_user_path(&path_owned) {
        Ok(p) => p,
        Err(e) => return e,
    };

    let (mount_fs, _) = match fs::find_mount(&path_norm) {
        Some(found) => found,
        None => return errno::ENOENT,
    };

    let stats = match mount_fs.statfs() {
        Ok(s) => s,
        Err(e) => return vfs_error_to_errno(e),
    };

    let linux = LinuxStatFs {
        f_type: fs_magic(&stats.fs_type),
        f_bsize: stats.block_size as i64,
        f_blocks: stats.total_blocks,
        f_bfree: stats.free_blocks,
        f_bavail: stats.free_blocks,
        f_files: stats.total_inodes,
        f_ffree: stats.free_inodes,
        f_fsid: [0, 0],
        f_namelen: 255,
        f_frsize: stats.block_size as i64,
        f_flags: 0,
        f_spare: [0; 4],
    };

    match uaccess::write_unaligned(buf as *mut LinuxStatFs, linux) {
        Ok(()) => 0,
        Err(e) => e,
    }
}

/// sys_ppoll - 파일 디스크립터 이벤트 대기
///
/// baseline:
/// - POLLIN/POLLPRI/POLLOUT만 readiness를 보고한다.
/// - timeout이 지정된 경우에만 sleep 대기한다.
/// - sigmask/sigsetsize는 현재 미사용이다.
pub fn sys_ppoll(
    fds: *mut u8,
    nfds: usize,
    timeout: *const u8,
    _sigmask: *const u8,
    _sigsetsize: usize,
) -> isize {
    if nfds > PPOLL_MAX_FDS {
        return errno::EINVAL;
    }
    if nfds > 0 && fds.is_null() {
        return errno::EFAULT;
    }

    let timeout_deadline_ns = match parse_timeout_timespec_deadline(timeout) {
        Ok(deadline) => deadline,
        Err(e) => return e,
    };

    let pollfds = fds as usize;
    let pollfd_size = core::mem::size_of::<LinuxPollFd>();

    loop {
        let table = match current_fd_table() {
            Ok(t) => t,
            Err(_) => return errno::EIO,
        };

        let mut ready = 0isize;
        for idx in 0..nfds {
            let entry_addr = match pollfds.checked_add(idx * pollfd_size) {
                Some(v) => v as *mut LinuxPollFd,
                None => return errno::EFAULT,
            };
            let mut entry = match uaccess::read_unaligned(entry_addr as *const LinuxPollFd) {
                Ok(entry) => entry,
                Err(e) => return e,
            };
            entry.revents = 0;

            if entry.fd >= 0 {
                if table.get(entry.fd).is_err() {
                    entry.revents = POLLNVAL;
                } else {
                    if entry.events & POLLIN != 0 {
                        entry.revents |= POLLIN;
                    }
                    if entry.events & POLLPRI != 0 {
                        entry.revents |= POLLPRI;
                    }
                    if entry.events & POLLOUT != 0 {
                        entry.revents |= POLLOUT;
                    }
                }
            }

            if entry.revents & (POLLIN | POLLPRI | POLLOUT | POLLERR | POLLHUP | POLLNVAL) != 0 {
                ready += 1;
            }

            if let Err(e) = uaccess::write_unaligned(entry_addr, entry) {
                return e;
            }
        }

        if ready > 0 {
            return ready;
        }

        match timeout_deadline_ns {
            Some(deadline_ns) => {
                let now_ns = crate::time::monotonic_now_ns();
                if now_ns >= deadline_ns {
                    return 0;
                }
                let wake_reason = proc::sleep_current_until(deadline_ns);
                if wake_reason == proc::SleepWakeReason::Signal {
                    return errno::EINTR;
                }
            }
            None => return 0,
        }
    }
}

/// sys_pselect6 - fd_set 기반 이벤트 대기
///
/// baseline:
/// - readfds/writefds/exceptfds 모두 "요청된 valid FD는 즉시 ready" 모델을 사용한다.
/// - invalid FD가 포함되면 `EBADF`를 반환한다.
/// - sigmask(6번째 인자)는 현재 미사용이다.
pub fn sys_pselect6(
    nfds: i32,
    readfds: *mut u8,
    writefds: *mut u8,
    exceptfds: *mut u8,
    timeout: *const u8,
    _sigmask_with_len: *const u8,
) -> isize {
    if nfds < 0 {
        return errno::EINVAL;
    }
    let nfds = nfds as usize;
    if nfds > PSELECT_MAX_FDS {
        return errno::EINVAL;
    }

    let set_len = match fdset_byte_len(nfds) {
        Ok(len) => len,
        Err(e) => return e,
    };

    let read_in = if readfds.is_null() {
        None
    } else {
        let mut set = match alloc_zeroed_user_buffer(set_len) {
            Ok(v) => v,
            Err(e) => return e,
        };
        if let Err(e) = uaccess::copy_from_user(&mut set, readfds as *const u8) {
            return e;
        }
        Some(set)
    };
    let write_in = if writefds.is_null() {
        None
    } else {
        let mut set = match alloc_zeroed_user_buffer(set_len) {
            Ok(v) => v,
            Err(e) => return e,
        };
        if let Err(e) = uaccess::copy_from_user(&mut set, writefds as *const u8) {
            return e;
        }
        Some(set)
    };
    let except_in = if exceptfds.is_null() {
        None
    } else {
        let mut set = match alloc_zeroed_user_buffer(set_len) {
            Ok(v) => v,
            Err(e) => return e,
        };
        if let Err(e) = uaccess::copy_from_user(&mut set, exceptfds as *const u8) {
            return e;
        }
        Some(set)
    };

    let mut read_out = if read_in.is_some() {
        match alloc_zeroed_user_buffer(set_len) {
            Ok(v) => Some(v),
            Err(e) => return e,
        }
    } else {
        None
    };
    let mut write_out = if write_in.is_some() {
        match alloc_zeroed_user_buffer(set_len) {
            Ok(v) => Some(v),
            Err(e) => return e,
        }
    } else {
        None
    };
    let mut except_out = if except_in.is_some() {
        match alloc_zeroed_user_buffer(set_len) {
            Ok(v) => Some(v),
            Err(e) => return e,
        }
    } else {
        None
    };

    let timeout_deadline_ns = match parse_timeout_timespec_deadline(timeout) {
        Ok(deadline) => deadline,
        Err(e) => return e,
    };

    loop {
        if let Some(set) = read_out.as_mut() {
            set.fill(0);
        }
        if let Some(set) = write_out.as_mut() {
            set.fill(0);
        }
        if let Some(set) = except_out.as_mut() {
            set.fill(0);
        }

        let table = match current_fd_table() {
            Ok(t) => t,
            Err(_) => return errno::EIO,
        };

        let mut ready = 0isize;
        for fd in 0..nfds {
            let watch_read = read_in
                .as_ref()
                .map(|set| fdset_is_set(set, fd))
                .unwrap_or(false);
            let watch_write = write_in
                .as_ref()
                .map(|set| fdset_is_set(set, fd))
                .unwrap_or(false);
            let watch_except = except_in
                .as_ref()
                .map(|set| fdset_is_set(set, fd))
                .unwrap_or(false);

            if !watch_read && !watch_write && !watch_except {
                continue;
            }

            if table.get(fd as i32).is_err() {
                return errno::EBADF;
            }

            let mut fd_ready = false;
            if watch_read {
                if let Some(set) = read_out.as_mut() {
                    fdset_set(set, fd);
                }
                fd_ready = true;
            }
            if watch_write {
                if let Some(set) = write_out.as_mut() {
                    fdset_set(set, fd);
                }
                fd_ready = true;
            }
            if watch_except {
                if let Some(set) = except_out.as_mut() {
                    fdset_set(set, fd);
                }
                fd_ready = true;
            }

            if fd_ready {
                ready += 1;
            }
        }

        if ready > 0 {
            if let Some(set) = read_out.as_ref() {
                if let Err(e) = uaccess::copy_to_user(readfds, set) {
                    return e;
                }
            }
            if let Some(set) = write_out.as_ref() {
                if let Err(e) = uaccess::copy_to_user(writefds, set) {
                    return e;
                }
            }
            if let Some(set) = except_out.as_ref() {
                if let Err(e) = uaccess::copy_to_user(exceptfds, set) {
                    return e;
                }
            }
            return ready;
        }

        match timeout_deadline_ns {
            Some(deadline_ns) => {
                let now_ns = crate::time::monotonic_now_ns();
                if now_ns >= deadline_ns {
                    if let Some(set) = read_out.as_ref() {
                        if let Err(e) = uaccess::copy_to_user(readfds, set) {
                            return e;
                        }
                    }
                    if let Some(set) = write_out.as_ref() {
                        if let Err(e) = uaccess::copy_to_user(writefds, set) {
                            return e;
                        }
                    }
                    if let Some(set) = except_out.as_ref() {
                        if let Err(e) = uaccess::copy_to_user(exceptfds, set) {
                            return e;
                        }
                    }
                    return 0;
                }
                let wake_reason = proc::sleep_current_until(deadline_ns);
                if wake_reason == proc::SleepWakeReason::Signal {
                    return errno::EINTR;
                }
            }
            None => {
                if let Some(set) = read_out.as_ref() {
                    if let Err(e) = uaccess::copy_to_user(readfds, set) {
                        return e;
                    }
                }
                if let Some(set) = write_out.as_ref() {
                    if let Err(e) = uaccess::copy_to_user(writefds, set) {
                        return e;
                    }
                }
                if let Some(set) = except_out.as_ref() {
                    if let Err(e) = uaccess::copy_to_user(exceptfds, set) {
                        return e;
                    }
                }
                return 0;
            }
        }
    }
}

/// sys_epoll_create1 - epoll 인스턴스 생성
pub fn sys_epoll_create1(flags: u32) -> isize {
    if flags & !EPOLL_CLOEXEC != 0 {
        return errno::EINVAL;
    }

    let table = match current_fd_table() {
        Ok(t) => t,
        Err(e) => return vfs_error_to_errno(e),
    };

    let epoll_vnode: Arc<dyn fs::VNode> = Arc::new(EpollVNode);
    let epoll_file = Arc::new(fd::OpenFile::new(
        epoll_vnode,
        OpenFlags::new(OpenFlags::O_RDWR),
    ));
    let epfd = match table.insert(epoll_file) {
        Ok(fd) => fd,
        Err(e) => return vfs_error_to_errno(e),
    };

    let files_group = super::process::current_files_group();
    let mut instances = EPOLL_INSTANCES.lock();
    instances.push((
        files_group,
        epfd,
        EpollInstance {
            watches: Vec::new(),
        },
    ));

    epfd as isize
}

/// sys_epoll_ctl - epoll 관심 FD 등록/수정/삭제
pub fn sys_epoll_ctl(epfd: i32, op: i32, fd: i32, event: *const u8) -> isize {
    if epfd < 0 || fd < 0 {
        return errno::EBADF;
    }
    if epfd == fd {
        return errno::EINVAL;
    }

    let table = match current_fd_table() {
        Ok(t) => t,
        Err(_) => return errno::EIO,
    };
    if table.get(epfd).is_err() || table.get(fd).is_err() {
        return errno::EBADF;
    }

    let files_group = super::process::current_files_group();
    let mut instances = EPOLL_INSTANCES.lock();
    let idx = match epoll_instance_index(&instances, files_group, epfd) {
        Some(idx) => idx,
        None => return errno::EINVAL,
    };
    let instance = &mut instances[idx].2;

    match op {
        EPOLL_CTL_ADD => {
            let (events, data) = match read_linux_epoll_event(event) {
                Ok(v) => v,
                Err(e) => return e,
            };
            if instance.watches.iter().any(|watch| watch.fd == fd) {
                return errno::EBUSY;
            }
            instance.watches.push(EpollRegistration { fd, events, data });
            0
        }
        EPOLL_CTL_MOD => {
            let (events, data) = match read_linux_epoll_event(event) {
                Ok(v) => v,
                Err(e) => return e,
            };
            if let Some(watch) = instance.watches.iter_mut().find(|watch| watch.fd == fd) {
                watch.events = events;
                watch.data = data;
                0
            } else {
                errno::ENOENT
            }
        }
        EPOLL_CTL_DEL => {
            let prev_len = instance.watches.len();
            instance.watches.retain(|watch| watch.fd != fd);
            if instance.watches.len() == prev_len {
                errno::ENOENT
            } else {
                0
            }
        }
        _ => errno::EINVAL,
    }
}

/// sys_epoll_pwait - epoll 이벤트 대기
///
/// baseline:
/// - `EPOLLIN/EPOLLPRI/EPOLLOUT` 요청 시 valid FD를 즉시 ready로 간주한다.
/// - `sigmask/sigsetsize`는 현재 미사용이다.
pub fn sys_epoll_pwait(
    epfd: i32,
    events: *mut u8,
    maxevents: i32,
    timeout: i32,
    _sigmask: *const u8,
    _sigsetsize: usize,
) -> isize {
    if epfd < 0 {
        return errno::EBADF;
    }
    if events.is_null() {
        return errno::EFAULT;
    }
    if maxevents <= 0 || maxevents as usize > EPOLL_MAX_EVENTS {
        return errno::EINVAL;
    }
    if timeout < -1 {
        return errno::EINVAL;
    }

    let maxevents = maxevents as usize;
    let timeout_deadline_ns = if timeout == -1 {
        None
    } else {
        let timeout_ns = (timeout as u64).saturating_mul(1_000_000);
        Some(crate::time::monotonic_now_ns().saturating_add(timeout_ns))
    };

    loop {
        let table = match current_fd_table() {
            Ok(t) => t,
            Err(_) => return errno::EIO,
        };

        let files_group = super::process::current_files_group();
        let watches = {
            let instances = EPOLL_INSTANCES.lock();
            let idx = match epoll_instance_index(&instances, files_group, epfd) {
                Some(idx) => idx,
                None => return errno::EINVAL,
            };
            instances[idx].2.watches.clone()
        };

        let mut ready = 0usize;
        for watch in watches.iter() {
            if ready >= maxevents {
                break;
            }

            let mut revents = 0u32;
            if table.get(watch.fd).is_err() {
                revents |= EPOLLERR;
            } else {
                if watch.events & EPOLLIN != 0 {
                    revents |= EPOLLIN;
                }
                if watch.events & EPOLLPRI != 0 {
                    revents |= EPOLLPRI;
                }
                if watch.events & EPOLLOUT != 0 {
                    revents |= EPOLLOUT;
                }
                if watch.events & EPOLLHUP != 0 {
                    revents |= EPOLLHUP;
                }
            }

            if revents == 0 {
                continue;
            }

            let event_addr = match (events as usize).checked_add(ready * LINUX_EPOLL_EVENT_SIZE) {
                Some(addr) => addr as *mut u8,
                None => return errno::EFAULT,
            };
            if let Err(e) = write_linux_epoll_event(event_addr, revents, watch.data) {
                return e;
            }
            ready += 1;
        }

        if ready > 0 {
            return ready as isize;
        }

        match timeout_deadline_ns {
            Some(deadline_ns) => {
                let now_ns = crate::time::monotonic_now_ns();
                if now_ns >= deadline_ns {
                    return 0;
                }
                let wake_reason = proc::sleep_current_until(deadline_ns);
                if wake_reason == proc::SleepWakeReason::Signal {
                    return errno::EINTR;
                }
            }
            None => return 0,
        }
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

    match fs::resolve_parent_path(&path_norm) {
        Ok((parent, name)) => {
            let create_mode = mode & !super::process::current_umask();
            match parent.create(&name, VNodeType::Directory, FileMode::new(create_mode)) {
                Ok(_) => 0,
                Err(e) => vfs_error_to_errno(e),
            }
        }
        Err(e) => vfs_error_to_errno(e),
    }
}

/// sys_unlink - 파일 삭제
pub fn sys_unlink(path: *const u8) -> isize {
    sys_unlinkat(path, 0)
}

/// sys_unlinkat - 파일/디렉토리 삭제
///
/// baseline:
/// - dirfd는 무시한다.
/// - flags는 `AT_REMOVEDIR`만 지원한다.
pub fn sys_unlinkat(path: *const u8, flags: u32) -> isize {
    if flags & !AT_REMOVEDIR != 0 {
        return errno::EINVAL;
    }

    let path_owned = match read_c_path(path) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let path_norm = match normalize_user_path(&path_owned) {
        Ok(p) => p,
        Err(e) => return e,
    };

    match fs::resolve_parent_path(&path_norm) {
        Ok((parent, name)) => {
            if flags & AT_REMOVEDIR != 0 {
                match parent.rmdir(&name) {
                    Ok(()) => 0,
                    Err(e) => vfs_error_to_errno(e),
                }
            } else {
                match parent.unlink(&name) {
                    Ok(()) => 0,
                    Err(e) => vfs_error_to_errno(e),
                }
            }
        }
        Err(e) => vfs_error_to_errno(e),
    }
}
