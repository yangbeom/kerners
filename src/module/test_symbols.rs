//! 테스트 모듈용 커널 심볼 래퍼
//!
//! 커널 모듈(.ko)은 extern "C" 함수만 호출 가능하므로,
//! 커널 내부 API를 C-compatible 래퍼로 감싸 심볼 테이블에 등록한다.

// ============================================================
// MM (메모리 관리)
// ============================================================

/// 페이지 프레임 할당 (C-ABI 래퍼)
/// 반환: 할당된 주소 (0 = 실패)
#[unsafe(no_mangle)]
pub extern "C" fn alloc_frame() -> usize {
    crate::mm::page::alloc_frame().unwrap_or(0)
}

/// 페이지 프레임 해제
#[unsafe(no_mangle)]
pub extern "C" fn free_frame(addr: usize) {
    unsafe {
        crate::mm::page::free_frame(addr);
    }
}

/// 힙 메모리 할당
/// 반환: 할당된 주소 (0 = 실패)
#[unsafe(no_mangle)]
pub extern "C" fn kernel_heap_alloc(size: usize, align: usize) -> usize {
    use core::alloc::Layout;
    if size == 0 || align == 0 || !align.is_power_of_two() {
        return 0;
    }
    let layout = match Layout::from_size_align(size, align) {
        Ok(l) => l,
        Err(_) => return 0,
    };
    let ptr = unsafe { alloc::alloc::alloc(layout) };
    if ptr.is_null() { 0 } else { ptr as usize }
}

/// 힙 메모리 해제
#[unsafe(no_mangle)]
pub extern "C" fn kernel_heap_dealloc(ptr: usize, size: usize, align: usize) {
    use core::alloc::Layout;
    if ptr == 0 || size == 0 || align == 0 || !align.is_power_of_two() {
        return;
    }
    let layout = match Layout::from_size_align(size, align) {
        Ok(l) => l,
        Err(_) => return,
    };
    unsafe {
        alloc::alloc::dealloc(ptr as *mut u8, layout);
    }
}

// ============================================================
// IPC (메시지 큐)
// ============================================================

/// 안전한 &str 변환 헬퍼
fn str_from_raw(ptr: *const u8, len: usize) -> Option<&'static str> {
    if ptr.is_null() || len == 0 {
        return None;
    }
    let slice = unsafe { core::slice::from_raw_parts(ptr, len) };
    core::str::from_utf8(slice).ok()
}

/// 메시지 큐 열기/생성
/// 반환: 0 = 성공, -1 = 실패
#[unsafe(no_mangle)]
pub extern "C" fn kernel_mq_open(name: *const u8, name_len: usize, create: bool) -> i32 {
    let name = match str_from_raw(name, name_len) {
        Some(s) => s,
        None => return -1,
    };
    match crate::ipc::message_queue::mq_open(name, create) {
        Ok(_) => 0,
        Err(_) => -1,
    }
}

/// 메시지 전송
/// 반환: 0 = 성공, -1 = 실패
#[unsafe(no_mangle)]
pub extern "C" fn kernel_mq_send(
    name: *const u8,
    name_len: usize,
    data: *const u8,
    data_len: usize,
) -> i32 {
    let name = match str_from_raw(name, name_len) {
        Some(s) => s,
        None => return -1,
    };
    if data.is_null() || data_len == 0 {
        return -1;
    }
    let msg = unsafe { core::slice::from_raw_parts(data, data_len) };
    match crate::ipc::message_queue::mq_send(name, msg) {
        Ok(()) => 0,
        Err(_) => -1,
    }
}

/// 메시지 수신
/// 반환: 수신 바이트 수, -1 = 실패
#[unsafe(no_mangle)]
pub extern "C" fn kernel_mq_receive(
    name: *const u8,
    name_len: usize,
    buf: *mut u8,
    buf_len: usize,
) -> i32 {
    let name = match str_from_raw(name, name_len) {
        Some(s) => s,
        None => return -1,
    };
    if buf.is_null() || buf_len == 0 {
        return -1;
    }
    // try_receive 사용 — 빈 큐에서 블로킹하지 않음
    let mq = match crate::ipc::message_queue::mq_open(name, false) {
        Ok(mq) => mq,
        Err(_) => return -1,
    };
    match mq.try_receive() {
        Ok(msg) => {
            let data = &msg.data;
            let copy_len = core::cmp::min(data.len(), buf_len);
            unsafe {
                core::ptr::copy_nonoverlapping(data.as_ptr(), buf, copy_len);
            }
            copy_len as i32
        }
        Err(_) => -1,
    }
}

// ============================================================
// Block (블록 디바이스)
// ============================================================

/// RamDisk 생성 및 등록
/// 반환: 0 = 성공, -1 = 실패
#[unsafe(no_mangle)]
pub extern "C" fn kernel_ramdisk_create(name: *const u8, name_len: usize, size: usize) -> i32 {
    let name = match str_from_raw(name, name_len) {
        Some(s) => s,
        None => return -1,
    };
    let _ = crate::block::ramdisk::create_ramdisk(name, size);
    0
}

/// 블록 읽기
/// 반환: 읽은 바이트 수, -1 = 실패
#[unsafe(no_mangle)]
pub extern "C" fn kernel_block_read(
    name: *const u8,
    name_len: usize,
    block_idx: usize,
    buf: *mut u8,
    buf_len: usize,
) -> i32 {
    let name = match str_from_raw(name, name_len) {
        Some(s) => s,
        None => return -1,
    };
    if buf.is_null() || buf_len == 0 {
        return -1;
    }
    let device = match crate::block::get_device(name) {
        Some(d) => d,
        None => return -1,
    };
    let block_size = device.block_size();
    if buf_len < block_size {
        return -1;
    }
    let slice = unsafe { core::slice::from_raw_parts_mut(buf, block_size) };
    match device.read_block(block_idx as u64, slice) {
        Ok(()) => block_size as i32,
        Err(_) => -1,
    }
}

/// 블록 쓰기
/// 반환: 쓴 바이트 수, -1 = 실패
#[unsafe(no_mangle)]
pub extern "C" fn kernel_block_write(
    name: *const u8,
    name_len: usize,
    block_idx: usize,
    data: *const u8,
    data_len: usize,
) -> i32 {
    let name = match str_from_raw(name, name_len) {
        Some(s) => s,
        None => return -1,
    };
    if data.is_null() || data_len == 0 {
        return -1;
    }
    let device = match crate::block::get_device(name) {
        Some(d) => d,
        None => return -1,
    };
    let block_size = device.block_size();
    if data_len < block_size {
        return -1;
    }
    let slice = unsafe { core::slice::from_raw_parts(data, block_size) };
    match device.write_block(block_idx as u64, slice) {
        Ok(()) => block_size as i32,
        Err(_) => -1,
    }
}

// ============================================================
// VFS (파일시스템)
// ============================================================

/// 디렉토리 생성
/// 반환: 0 = 성공, -1 = 실패
#[unsafe(no_mangle)]
pub extern "C" fn kernel_vfs_mkdir(path: *const u8, path_len: usize) -> i32 {
    let path_str = match str_from_raw(path, path_len) {
        Some(s) => s,
        None => return -1,
    };
    let (parent_path, dir_name) = crate::fs::path::split(path_str);
    let parent = match crate::fs::lookup_path(parent_path) {
        Ok(p) => p,
        Err(_) => return -1,
    };
    match parent.create(
        dir_name,
        crate::fs::VNodeType::Directory,
        crate::fs::FileMode::default_dir(),
    ) {
        Ok(_) => 0,
        Err(_) => -1,
    }
}

/// 파일 생성
/// 반환: 0 = 성공, -1 = 실패
#[unsafe(no_mangle)]
pub extern "C" fn kernel_vfs_create_file(path: *const u8, path_len: usize) -> i32 {
    let path_str = match str_from_raw(path, path_len) {
        Some(s) => s,
        None => return -1,
    };
    let (parent_path, file_name) = crate::fs::path::split(path_str);
    let parent = match crate::fs::lookup_path(parent_path) {
        Ok(p) => p,
        Err(_) => return -1,
    };
    match parent.create(
        file_name,
        crate::fs::VNodeType::File,
        crate::fs::FileMode::default_file(),
    ) {
        Ok(_) => 0,
        Err(_) => -1,
    }
}

/// 파일 쓰기
/// 반환: 쓴 바이트 수, -1 = 실패
#[unsafe(no_mangle)]
pub extern "C" fn kernel_vfs_write(
    path: *const u8,
    path_len: usize,
    offset: usize,
    data: *const u8,
    data_len: usize,
) -> i32 {
    let path_str = match str_from_raw(path, path_len) {
        Some(s) => s,
        None => return -1,
    };
    if data.is_null() || data_len == 0 {
        return -1;
    }
    let node = match crate::fs::lookup_path(path_str) {
        Ok(n) => n,
        Err(_) => return -1,
    };
    let buf = unsafe { core::slice::from_raw_parts(data, data_len) };
    match node.write(offset, buf) {
        Ok(n) => n as i32,
        Err(_) => -1,
    }
}

/// 파일 읽기
/// 반환: 읽은 바이트 수, -1 = 실패
#[unsafe(no_mangle)]
pub extern "C" fn kernel_vfs_read(
    path: *const u8,
    path_len: usize,
    offset: usize,
    buf: *mut u8,
    buf_len: usize,
) -> i32 {
    let path_str = match str_from_raw(path, path_len) {
        Some(s) => s,
        None => return -1,
    };
    if buf.is_null() || buf_len == 0 {
        return -1;
    }
    let node = match crate::fs::lookup_path(path_str) {
        Ok(n) => n,
        Err(_) => return -1,
    };
    let slice = unsafe { core::slice::from_raw_parts_mut(buf, buf_len) };
    match node.read(offset, slice) {
        Ok(n) => n as i32,
        Err(_) => -1,
    }
}

/// 파일/디렉토리 삭제
/// 반환: 0 = 성공, -1 = 실패
#[unsafe(no_mangle)]
pub extern "C" fn kernel_vfs_unlink(path: *const u8, path_len: usize) -> i32 {
    let path_str = match str_from_raw(path, path_len) {
        Some(s) => s,
        None => return -1,
    };
    let (parent_path, name) = crate::fs::path::split(path_str);
    let parent = match crate::fs::lookup_path(parent_path) {
        Ok(p) => p,
        Err(_) => return -1,
    };
    match parent.unlink(name) {
        Ok(()) => 0,
        Err(_) => -1,
    }
}

// ============================================================
// Exec (유저 ELF 준비)
// ============================================================

/// exec 준비 검증 (실행 전 단계)
/// 반환: 0 = 성공, 음수 = errno 스타일 에러
#[unsafe(no_mangle)]
pub extern "C" fn kernel_exec_prepare(path: *const u8, path_len: usize) -> i32 {
    let path_str = match str_from_raw(path, path_len) {
        Some(s) => s,
        None => return -14, // EFAULT
    };

    let mut argv = alloc::vec::Vec::new();
    argv.push(alloc::string::String::from(path_str));
    let envp: alloc::vec::Vec<alloc::string::String> = alloc::vec::Vec::new();

    match crate::proc::user::prepare_exec_image(path_str, &argv, &envp) {
        Ok(_) => 0,
        Err(crate::proc::user::ExecError::NotFound) => -2, // ENOENT
        Err(crate::proc::user::ExecError::OutOfMemory) => -12, // ENOMEM
        Err(crate::proc::user::ExecError::InvalidArgument) => -22, // EINVAL
        Err(crate::proc::user::ExecError::InvalidElf)
        | Err(crate::proc::user::ExecError::UnsupportedExecutableType)
        | Err(crate::proc::user::ExecError::DynamicElfNotSupported) => -8, // ENOEXEC
        Err(crate::proc::user::ExecError::IoError) => -5,  // EIO
    }
}

// ============================================================
// Process Syscalls (10-1B)
// ============================================================

/// getpid syscall 래퍼
#[unsafe(no_mangle)]
pub extern "C" fn kernel_sys_getpid() -> i64 {
    crate::syscall::syscall_handler(crate::syscall::SYS_GETPID, [0; 6]) as i64
}

/// getppid syscall 래퍼
#[unsafe(no_mangle)]
pub extern "C" fn kernel_sys_getppid() -> i64 {
    crate::syscall::syscall_handler(crate::syscall::SYS_GETPPID, [0; 6]) as i64
}

/// gettid syscall 래퍼
#[unsafe(no_mangle)]
pub extern "C" fn kernel_sys_gettid() -> i64 {
    crate::syscall::syscall_handler(crate::syscall::SYS_GETTID, [0; 6]) as i64
}

/// brk syscall 래퍼
#[unsafe(no_mangle)]
pub extern "C" fn kernel_sys_brk(addr: usize) -> i64 {
    crate::syscall::syscall_handler(crate::syscall::SYS_BRK, [addr, 0, 0, 0, 0, 0]) as i64
}

/// mmap syscall 래퍼
#[unsafe(no_mangle)]
pub extern "C" fn kernel_sys_mmap(
    addr: usize,
    len: usize,
    prot: usize,
    flags: usize,
    fd: i64,
    offset: usize,
) -> i64 {
    crate::syscall::syscall_handler(
        crate::syscall::SYS_MMAP,
        [addr, len, prot, flags, fd as usize, offset],
    ) as i64
}

/// munmap syscall 래퍼
#[unsafe(no_mangle)]
pub extern "C" fn kernel_sys_munmap(addr: usize, len: usize) -> i64 {
    crate::syscall::syscall_handler(crate::syscall::SYS_MUNMAP, [addr, len, 0, 0, 0, 0]) as i64
}

/// mprotect syscall 래퍼
#[unsafe(no_mangle)]
pub extern "C" fn kernel_sys_mprotect(addr: usize, len: usize, prot: usize) -> i64 {
    crate::syscall::syscall_handler(crate::syscall::SYS_MPROTECT, [addr, len, prot, 0, 0, 0]) as i64
}

/// open syscall 래퍼
#[unsafe(no_mangle)]
pub extern "C" fn kernel_sys_open(path: *const u8, flags: u32, mode: u32) -> i64 {
    crate::syscall::syscall_handler(
        crate::syscall::SYS_OPENAT,
        [0, path as usize, flags as usize, mode as usize, 0, 0],
    ) as i64
}

/// close syscall 래퍼
#[unsafe(no_mangle)]
pub extern "C" fn kernel_sys_close(fd: i32) -> i64 {
    crate::syscall::syscall_handler(crate::syscall::SYS_CLOSE, [fd as usize, 0, 0, 0, 0, 0]) as i64
}

/// lseek syscall 래퍼
#[unsafe(no_mangle)]
pub extern "C" fn kernel_sys_lseek(fd: i32, offset: i64, whence: i32) -> i64 {
    crate::syscall::syscall_handler(
        crate::syscall::SYS_LSEEK,
        [fd as usize, offset as usize, whence as usize, 0, 0, 0],
    ) as i64
}

/// read syscall 래퍼
#[unsafe(no_mangle)]
pub extern "C" fn kernel_sys_read(fd: i32, buf: *mut u8, count: usize) -> i64 {
    crate::syscall::syscall_handler(
        crate::syscall::SYS_READ,
        [fd as usize, buf as usize, count, 0, 0, 0],
    ) as i64
}

/// write syscall 래퍼
#[unsafe(no_mangle)]
pub extern "C" fn kernel_sys_write(fd: i32, buf: *const u8, count: usize) -> i64 {
    crate::syscall::syscall_handler(
        crate::syscall::SYS_WRITE,
        [fd as usize, buf as usize, count, 0, 0, 0],
    ) as i64
}

/// getdents64 syscall 래퍼
#[unsafe(no_mangle)]
pub extern "C" fn kernel_sys_getdents64(fd: i32, dirp: *mut u8, count: usize) -> i64 {
    crate::syscall::syscall_handler(
        crate::syscall::SYS_GETDENTS64,
        [fd as usize, dirp as usize, count, 0, 0, 0],
    ) as i64
}

/// pipe2 syscall 래퍼
#[unsafe(no_mangle)]
pub extern "C" fn kernel_sys_pipe2(pipefd: *mut i32, flags: u32) -> i64 {
    crate::syscall::syscall_handler(
        crate::syscall::SYS_PIPE2,
        [pipefd as usize, flags as usize, 0, 0, 0, 0],
    ) as i64
}

/// readlinkat syscall 래퍼
#[unsafe(no_mangle)]
pub extern "C" fn kernel_sys_readlinkat(
    dirfd: i32,
    path: *const u8,
    buf: *mut u8,
    bufsiz: usize,
) -> i64 {
    crate::syscall::syscall_handler(
        crate::syscall::SYS_READLINKAT,
        [dirfd as usize, path as usize, buf as usize, bufsiz, 0, 0],
    ) as i64
}

/// statfs syscall 래퍼
#[unsafe(no_mangle)]
pub extern "C" fn kernel_sys_statfs(path: *const u8, statfs_buf: *mut u8) -> i64 {
    crate::syscall::syscall_handler(
        crate::syscall::SYS_STATFS,
        [path as usize, statfs_buf as usize, 0, 0, 0, 0],
    ) as i64
}

/// ppoll syscall 래퍼
#[unsafe(no_mangle)]
pub extern "C" fn kernel_sys_ppoll(
    fds: *mut u8,
    nfds: usize,
    timeout: *const u8,
    sigmask: *const u8,
    sigsetsize: usize,
) -> i64 {
    crate::syscall::syscall_handler(
        crate::syscall::SYS_PPOLL,
        [
            fds as usize,
            nfds,
            timeout as usize,
            sigmask as usize,
            sigsetsize,
            0,
        ],
    ) as i64
}

/// pselect6 syscall 래퍼
#[unsafe(no_mangle)]
pub extern "C" fn kernel_sys_pselect6(
    nfds: i32,
    readfds: *mut u8,
    writefds: *mut u8,
    exceptfds: *mut u8,
    timeout: *const u8,
    sigmask: *const u8,
) -> i64 {
    crate::syscall::syscall_handler(
        crate::syscall::SYS_PSELECT6,
        [
            nfds as usize,
            readfds as usize,
            writefds as usize,
            exceptfds as usize,
            timeout as usize,
            sigmask as usize,
        ],
    ) as i64
}

/// epoll_create1 syscall 래퍼
#[unsafe(no_mangle)]
pub extern "C" fn kernel_sys_epoll_create1(flags: u32) -> i64 {
    crate::syscall::syscall_handler(
        crate::syscall::SYS_EPOLL_CREATE1,
        [flags as usize, 0, 0, 0, 0, 0],
    ) as i64
}

/// epoll_ctl syscall 래퍼
#[unsafe(no_mangle)]
pub extern "C" fn kernel_sys_epoll_ctl(epfd: i32, op: i32, fd: i32, event: *const u8) -> i64 {
    crate::syscall::syscall_handler(
        crate::syscall::SYS_EPOLL_CTL,
        [epfd as usize, op as usize, fd as usize, event as usize, 0, 0],
    ) as i64
}

/// epoll_pwait syscall 래퍼
#[unsafe(no_mangle)]
pub extern "C" fn kernel_sys_epoll_pwait(
    epfd: i32,
    events: *mut u8,
    maxevents: i32,
    timeout: i32,
    sigmask: *const u8,
    sigsetsize: usize,
) -> i64 {
    crate::syscall::syscall_handler(
        crate::syscall::SYS_EPOLL_PWAIT,
        [
            epfd as usize,
            events as usize,
            maxevents as usize,
            timeout as usize,
            sigmask as usize,
            sigsetsize,
        ],
    ) as i64
}

/// clock_gettime syscall 래퍼
#[unsafe(no_mangle)]
pub extern "C" fn kernel_sys_clock_gettime(clock_id: i32, tp: *mut u8) -> i64 {
    crate::syscall::syscall_handler(
        crate::syscall::SYS_CLOCK_GETTIME,
        [clock_id as usize, tp as usize, 0, 0, 0, 0],
    ) as i64
}

/// clock_getres syscall 래퍼
#[unsafe(no_mangle)]
pub extern "C" fn kernel_sys_clock_getres(clock_id: i32, tp: *mut u8) -> i64 {
    crate::syscall::syscall_handler(
        crate::syscall::SYS_CLOCK_GETRES,
        [clock_id as usize, tp as usize, 0, 0, 0, 0],
    ) as i64
}

/// gettimeofday syscall 래퍼
#[unsafe(no_mangle)]
pub extern "C" fn kernel_sys_gettimeofday(tv: *mut u8, tz: *mut u8) -> i64 {
    crate::syscall::syscall_handler(
        crate::syscall::SYS_GETTIMEOFDAY,
        [tv as usize, tz as usize, 0, 0, 0, 0],
    ) as i64
}

/// nanosleep syscall 래퍼
#[unsafe(no_mangle)]
pub extern "C" fn kernel_sys_nanosleep(req: *const u8, rem: *mut u8) -> i64 {
    crate::syscall::syscall_handler(
        crate::syscall::SYS_NANOSLEEP,
        [req as usize, rem as usize, 0, 0, 0, 0],
    ) as i64
}

/// sigaltstack syscall 래퍼
#[unsafe(no_mangle)]
pub extern "C" fn kernel_sys_sigaltstack(ss: *const u8, old_ss: *mut u8) -> i64 {
    crate::syscall::syscall_handler(
        crate::syscall::SYS_SIGALTSTACK,
        [ss as usize, old_ss as usize, 0, 0, 0, 0],
    ) as i64
}

/// rt_sigaction syscall 래퍼
#[unsafe(no_mangle)]
pub extern "C" fn kernel_sys_rt_sigaction(
    signum: i32,
    act: *const u8,
    oldact: *mut u8,
    sigsetsize: usize,
) -> i64 {
    crate::syscall::syscall_handler(
        crate::syscall::SYS_RT_SIGACTION,
        [
            signum as usize,
            act as usize,
            oldact as usize,
            sigsetsize,
            0,
            0,
        ],
    ) as i64
}

/// rt_sigprocmask syscall 래퍼
#[unsafe(no_mangle)]
pub extern "C" fn kernel_sys_rt_sigprocmask(
    how: i32,
    set: *const u8,
    oldset: *mut u8,
    sigsetsize: usize,
) -> i64 {
    crate::syscall::syscall_handler(
        crate::syscall::SYS_RT_SIGPROCMASK,
        [
            how as usize,
            set as usize,
            oldset as usize,
            sigsetsize,
            0,
            0,
        ],
    ) as i64
}

/// kill syscall 래퍼
#[unsafe(no_mangle)]
pub extern "C" fn kernel_sys_kill(pid: isize, sig: i32) -> i64 {
    crate::syscall::syscall_handler(
        crate::syscall::SYS_KILL,
        [pid as usize, sig as usize, 0, 0, 0, 0],
    ) as i64
}

/// tkill syscall 래퍼
#[unsafe(no_mangle)]
pub extern "C" fn kernel_sys_tkill(tid: isize, sig: i32) -> i64 {
    crate::syscall::syscall_handler(
        crate::syscall::SYS_TKILL,
        [tid as usize, sig as usize, 0, 0, 0, 0],
    ) as i64
}

/// tgkill syscall 래퍼
#[unsafe(no_mangle)]
pub extern "C" fn kernel_sys_tgkill(tgid: isize, tid: isize, sig: i32) -> i64 {
    crate::syscall::syscall_handler(
        crate::syscall::SYS_TGKILL,
        [tgid as usize, tid as usize, sig as usize, 0, 0, 0],
    ) as i64
}

/// rt_sigtimedwait syscall 래퍼
#[unsafe(no_mangle)]
pub extern "C" fn kernel_sys_rt_sigtimedwait(
    set: *const u8,
    info: *mut u8,
    timeout: *const u8,
    sigsetsize: usize,
) -> i64 {
    crate::syscall::syscall_handler(
        crate::syscall::SYS_RT_SIGTIMEDWAIT,
        [
            set as usize,
            info as usize,
            timeout as usize,
            sigsetsize,
            0,
            0,
        ],
    ) as i64
}

/// wait4 syscall 래퍼
#[unsafe(no_mangle)]
pub extern "C" fn kernel_sys_wait4(pid: isize, status: *mut i32, options: i32) -> i64 {
    crate::syscall::syscall_handler(
        crate::syscall::SYS_WAIT4,
        [pid as usize, status as usize, options as usize, 0, 0, 0],
    ) as i64
}

/// waitid syscall 래퍼
#[unsafe(no_mangle)]
pub extern "C" fn kernel_sys_waitid(idtype: i32, id: usize, infop: *mut u8, options: i32) -> i64 {
    crate::syscall::syscall_handler(
        crate::syscall::SYS_WAITID,
        [idtype as usize, id, infop as usize, options as usize, 0, 0],
    ) as i64
}

/// uname syscall 래퍼
#[unsafe(no_mangle)]
pub extern "C" fn kernel_sys_uname(buf: *mut u8) -> i64 {
    crate::syscall::syscall_handler(crate::syscall::SYS_UNAME, [buf as usize, 0, 0, 0, 0, 0]) as i64
}

/// fork 래퍼 (테스트 경로)
#[unsafe(no_mangle)]
pub extern "C" fn kernel_sys_fork() -> i64 {
    crate::syscall::fork_for_test() as i64
}

/// vfork 래퍼 (테스트 경로)
#[unsafe(no_mangle)]
pub extern "C" fn kernel_sys_vfork() -> i64 {
    crate::syscall::vfork_for_test() as i64
}

/// 테스트용 pending signal 삽입
#[unsafe(no_mangle)]
pub extern "C" fn kernel_test_enqueue_signal(signum: u32) -> i64 {
    crate::syscall::enqueue_signal_for_test(signum) as i64
}

/// 테스트용 pending signal 삽입 (지정 tid)
#[unsafe(no_mangle)]
pub extern "C" fn kernel_test_enqueue_signal_to_tid(tid: i64, signum: u32) -> i64 {
    crate::syscall::enqueue_signal_to_tid_for_test(tid as isize, signum) as i64
}

#[derive(Clone, Copy)]
struct ThreadSpawnRequest {
    tid: crate::proc::Tid,
    entry: extern "C" fn(usize),
    arg: usize,
}

static THREAD_SPAWN_REQUESTS: crate::sync::IrqSpinlock<alloc::vec::Vec<ThreadSpawnRequest>> =
    crate::sync::IrqSpinlock::new(alloc::vec::Vec::new());

fn take_thread_spawn_request(tid: crate::proc::Tid) -> Option<ThreadSpawnRequest> {
    let mut pending = THREAD_SPAWN_REQUESTS.lock();
    let idx = pending.iter().position(|req| req.tid == tid)?;
    Some(pending.swap_remove(idx))
}

fn thread_spawn_trampoline() -> ! {
    let tid = loop {
        if let Some(tid) = crate::proc::current_tid() {
            break tid;
        }
        crate::proc::yield_now();
    };

    let request = loop {
        if let Some(request) = take_thread_spawn_request(tid) {
            break request;
        }
        crate::proc::yield_now();
    };

    (request.entry)(request.arg);
    loop {
        crate::proc::yield_now();
    }
}

// ============================================================
// Thread (스레드)
// ============================================================

/// 스레드 생성
/// entry: 스레드 엔트리 함수 (usize 인자 1개, 반환 안 함)
/// 반환: tid (> 0), -1 = 실패
#[unsafe(no_mangle)]
pub extern "C" fn kernel_thread_spawn(
    entry: extern "C" fn(usize),
    arg: usize,
    name: *const u8,
    name_len: usize,
) -> i32 {
    let name = match str_from_raw(name, name_len) {
        Some(s) => s,
        None => "test_thread",
    };

    let tid = crate::proc::spawn(name, thread_spawn_trampoline);
    {
        let mut pending = THREAD_SPAWN_REQUESTS.lock();
        pending.push(ThreadSpawnRequest { tid, entry, arg });
    }
    tid as i32
}

/// N tick 대기 (busy-wait)
#[unsafe(no_mangle)]
pub extern "C" fn kernel_sleep_ticks(ticks: u32) {
    let start = crate::proc::percpu::current()
        .tick_count
        .load(core::sync::atomic::Ordering::Relaxed);
    loop {
        let now = crate::proc::percpu::current()
            .tick_count
            .load(core::sync::atomic::Ordering::Relaxed);
        if now.wrapping_sub(start) >= ticks as u64 {
            break;
        }
        crate::proc::yield_now();
    }
}

// ============================================================
// Logging (로깅)
// ============================================================

/// 로그 메시지 출력
/// level: 0=ERROR, 1=WARN, 2=INFO, 3=DEBUG, 4=TRACE
#[unsafe(no_mangle)]
pub extern "C" fn kernel_log(level: u8, msg: *const u8, msg_len: usize) {
    let level_enum = crate::log::LogLevel::from_u8(level);
    if let Some(s) = str_from_raw(msg, msg_len) {
        crate::log::log(level_enum, core::format_args!("{}", s));
    }
}

// ============================================================
// 심볼 등록
// ============================================================

/// 테스트 심볼들을 커널 심볼 테이블에 등록
pub fn register_test_symbols() {
    use crate::module::symbol::register_symbol;

    // MM
    register_symbol("alloc_frame", alloc_frame as usize);
    register_symbol("free_frame", free_frame as usize);
    register_symbol("kernel_heap_alloc", kernel_heap_alloc as usize);
    register_symbol("kernel_heap_dealloc", kernel_heap_dealloc as usize);

    // IPC
    register_symbol("kernel_mq_open", kernel_mq_open as usize);
    register_symbol("kernel_mq_send", kernel_mq_send as usize);
    register_symbol("kernel_mq_receive", kernel_mq_receive as usize);

    // Block
    register_symbol("kernel_ramdisk_create", kernel_ramdisk_create as usize);
    register_symbol("kernel_block_read", kernel_block_read as usize);
    register_symbol("kernel_block_write", kernel_block_write as usize);

    // VFS
    register_symbol("kernel_vfs_mkdir", kernel_vfs_mkdir as usize);
    register_symbol("kernel_vfs_create_file", kernel_vfs_create_file as usize);
    register_symbol("kernel_vfs_write", kernel_vfs_write as usize);
    register_symbol("kernel_vfs_read", kernel_vfs_read as usize);
    register_symbol("kernel_vfs_unlink", kernel_vfs_unlink as usize);
    register_symbol("kernel_exec_prepare", kernel_exec_prepare as usize);

    // Process syscalls
    register_symbol("kernel_sys_getpid", kernel_sys_getpid as usize);
    register_symbol("kernel_sys_getppid", kernel_sys_getppid as usize);
    register_symbol("kernel_sys_gettid", kernel_sys_gettid as usize);
    register_symbol("kernel_sys_brk", kernel_sys_brk as usize);
    register_symbol("kernel_sys_mmap", kernel_sys_mmap as usize);
    register_symbol("kernel_sys_munmap", kernel_sys_munmap as usize);
    register_symbol("kernel_sys_mprotect", kernel_sys_mprotect as usize);
    register_symbol("kernel_sys_open", kernel_sys_open as usize);
    register_symbol("kernel_sys_close", kernel_sys_close as usize);
    register_symbol("kernel_sys_lseek", kernel_sys_lseek as usize);
    register_symbol("kernel_sys_read", kernel_sys_read as usize);
    register_symbol("kernel_sys_write", kernel_sys_write as usize);
    register_symbol("kernel_sys_getdents64", kernel_sys_getdents64 as usize);
    register_symbol("kernel_sys_pipe2", kernel_sys_pipe2 as usize);
    register_symbol("kernel_sys_readlinkat", kernel_sys_readlinkat as usize);
    register_symbol("kernel_sys_statfs", kernel_sys_statfs as usize);
    register_symbol("kernel_sys_ppoll", kernel_sys_ppoll as usize);
    register_symbol("kernel_sys_pselect6", kernel_sys_pselect6 as usize);
    register_symbol("kernel_sys_epoll_create1", kernel_sys_epoll_create1 as usize);
    register_symbol("kernel_sys_epoll_ctl", kernel_sys_epoll_ctl as usize);
    register_symbol("kernel_sys_epoll_pwait", kernel_sys_epoll_pwait as usize);
    register_symbol("kernel_sys_clock_gettime", kernel_sys_clock_gettime as usize);
    register_symbol("kernel_sys_clock_getres", kernel_sys_clock_getres as usize);
    register_symbol("kernel_sys_gettimeofday", kernel_sys_gettimeofday as usize);
    register_symbol("kernel_sys_nanosleep", kernel_sys_nanosleep as usize);
    register_symbol("kernel_sys_sigaltstack", kernel_sys_sigaltstack as usize);
    register_symbol("kernel_sys_rt_sigaction", kernel_sys_rt_sigaction as usize);
    register_symbol(
        "kernel_sys_rt_sigprocmask",
        kernel_sys_rt_sigprocmask as usize,
    );
    register_symbol(
        "kernel_sys_rt_sigtimedwait",
        kernel_sys_rt_sigtimedwait as usize,
    );
    register_symbol("kernel_sys_kill", kernel_sys_kill as usize);
    register_symbol("kernel_sys_tkill", kernel_sys_tkill as usize);
    register_symbol("kernel_sys_tgkill", kernel_sys_tgkill as usize);
    register_symbol("kernel_sys_wait4", kernel_sys_wait4 as usize);
    register_symbol("kernel_sys_waitid", kernel_sys_waitid as usize);
    register_symbol("kernel_sys_uname", kernel_sys_uname as usize);
    register_symbol("kernel_sys_fork", kernel_sys_fork as usize);
    register_symbol("kernel_sys_vfork", kernel_sys_vfork as usize);
    register_symbol(
        "kernel_test_enqueue_signal",
        kernel_test_enqueue_signal as usize,
    );
    register_symbol(
        "kernel_test_enqueue_signal_to_tid",
        kernel_test_enqueue_signal_to_tid as usize,
    );

    // Thread
    register_symbol("kernel_thread_spawn", kernel_thread_spawn as usize);
    register_symbol("kernel_sleep_ticks", kernel_sleep_ticks as usize);

    // Logging
    register_symbol("kernel_log", kernel_log as usize);

    crate::kprintln!("[symbol] Test symbols registered ({} symbols)", 62);
}
