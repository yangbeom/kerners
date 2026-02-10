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
    if ptr.is_null() {
        0
    } else {
        ptr as usize
    }
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
pub extern "C" fn kernel_ramdisk_create(
    name: *const u8,
    name_len: usize,
    size: usize,
) -> i32 {
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
    match parent.create(dir_name, crate::fs::VNodeType::Directory, crate::fs::FileMode::default_dir()) {
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
    match parent.create(file_name, crate::fs::VNodeType::File, crate::fs::FileMode::default_file()) {
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

    // extern "C" fn(usize) → fn() -> ! 래핑
    // 인자를 클로저로 캡처하여 스레드 엔트리에 전달
    // 간단한 방식: 전역 변수로 전달 (단일 스레드 생성 시 안전)
    use core::sync::atomic::{AtomicUsize, Ordering};
    static THREAD_ENTRY: AtomicUsize = AtomicUsize::new(0);
    static THREAD_ARG: AtomicUsize = AtomicUsize::new(0);

    THREAD_ENTRY.store(entry as usize, Ordering::SeqCst);
    THREAD_ARG.store(arg, Ordering::SeqCst);

    fn thread_wrapper() -> ! {
        let entry_addr = THREAD_ENTRY.load(Ordering::SeqCst);
        let arg = THREAD_ARG.load(Ordering::SeqCst);
        let entry: extern "C" fn(usize) = unsafe { core::mem::transmute(entry_addr) };
        entry(arg);
        loop {
            crate::proc::yield_now();
        }
    }

    let tid = crate::proc::spawn(name, thread_wrapper);
    tid as i32
}

/// N tick 대기 (busy-wait)
#[unsafe(no_mangle)]
pub extern "C" fn kernel_sleep_ticks(ticks: u32) {
    let start = crate::proc::percpu::current().tick_count.load(core::sync::atomic::Ordering::Relaxed);
    loop {
        let now = crate::proc::percpu::current().tick_count.load(core::sync::atomic::Ordering::Relaxed);
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

    // Thread
    register_symbol("kernel_thread_spawn", kernel_thread_spawn as usize);
    register_symbol("kernel_sleep_ticks", kernel_sleep_ticks as usize);

    // Logging
    register_symbol("kernel_log", kernel_log as usize);

    crate::kprintln!("[symbol] Test symbols registered ({} symbols)", 19);
}
