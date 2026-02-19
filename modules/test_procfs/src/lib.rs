//! ProcFS + Phase 14 syscall 회귀 테스트 모듈

#![no_std]
#![no_main]

use core::panic::PanicInfo;

unsafe extern "C" {
    fn kernel_print(s: *const u8, len: usize);
    fn kernel_vfs_read(path: *const u8, path_len: usize, offset: usize, buf: *mut u8, buf_len: usize)
        -> i32;

    fn kernel_sys_gettid() -> i64;
    fn kernel_sys_open(path: *const u8, flags: u32, mode: u32) -> i64;
    fn kernel_sys_close(fd: i32) -> i64;
    fn kernel_sys_read(fd: i32, buf: *mut u8, count: usize) -> i64;
    fn kernel_sys_write(fd: i32, buf: *const u8, count: usize) -> i64;
    fn kernel_sys_getdents64(fd: i32, dirp: *mut u8, count: usize) -> i64;
    fn kernel_sys_pipe2(pipefd: *mut i32, flags: u32) -> i64;
    fn kernel_sys_readlinkat(dirfd: i32, path: *const u8, buf: *mut u8, bufsiz: usize) -> i64;
    fn kernel_sys_statfs(path: *const u8, statfs_buf: *mut u8) -> i64;
}

const O_RDONLY: u32 = 0;
const O_CLOEXEC: u32 = 0x80000;
const EINVAL: i64 = -22;
const PROCFS_MAGIC: i64 = 0x9fa0;

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

fn print(s: &str) {
    unsafe {
        kernel_print(s.as_ptr(), s.len());
    }
}

fn eq_bytes(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0usize;
    while i < a.len() {
        let av = unsafe { *a.get_unchecked(i) };
        let bv = unsafe { *b.get_unchecked(i) };
        if av != bv {
            return false;
        }
        i += 1;
    }
    true
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || haystack.len() < needle.len() {
        return false;
    }
    let end = haystack.len() - needle.len();
    let mut i = 0usize;
    while i <= end {
        let mut j = 0usize;
        while j < needle.len() {
            let hv = unsafe { *haystack.get_unchecked(i + j) };
            let nv = unsafe { *needle.get_unchecked(j) };
            if hv != nv {
                break;
            }
            j += 1;
        }
        if j == needle.len() {
            return true;
        }
        i += 1;
    }
    false
}

fn subslice(buf: &[u8], start: usize, end: usize) -> Option<&[u8]> {
    if start > end || end > buf.len() {
        return None;
    }
    Some(unsafe { core::slice::from_raw_parts(buf.as_ptr().add(start), end - start) })
}

fn clamp_len_i32(n: i32, max: usize) -> usize {
    if n <= 0 {
        0
    } else {
        core::cmp::min(n as usize, max)
    }
}

fn clamp_len_i64(n: i64, max: usize) -> usize {
    if n <= 0 {
        0
    } else {
        core::cmp::min(n as usize, max)
    }
}

fn c_strlen(buf: &[u8]) -> usize {
    for (i, b) in buf.iter().enumerate() {
        if *b == 0 {
            return i;
        }
    }
    buf.len()
}

fn getdents_contains_name(buf: &[u8], len: usize, target: &[u8]) -> bool {
    let len = core::cmp::min(len, buf.len());
    let mut pos = 0usize;
    while pos + 19 <= len {
        let reclen = unsafe {
            let b0 = *buf.get_unchecked(pos + 16);
            let b1 = *buf.get_unchecked(pos + 17);
            u16::from_le_bytes([b0, b1]) as usize
        };
        if reclen < 19 || pos + reclen > len {
            break;
        }
        let name_start = pos + 19;
        let name_end = pos + reclen;
        let Some(name_buf) = subslice(buf, name_start, name_end) else {
            break;
        };
        let nlen = c_strlen(name_buf);
        let Some(name_bytes) = subslice(name_buf, 0, nlen) else {
            break;
        };
        if nlen == target.len() && eq_bytes(name_bytes, target) {
            return true;
        }
        pos += reclen;
    }
    false
}

#[unsafe(no_mangle)]
pub extern "C" fn module_init() -> i32 {
    print("[test_procfs] === procfs/syscall Tests ===\n");

    print("[test_procfs] test: /proc/meminfo read ... ");
    let mut meminfo = [0u8; 256];
    let n = unsafe {
        kernel_vfs_read(
            b"/proc/meminfo".as_ptr(),
            b"/proc/meminfo".len(),
            0,
            meminfo.as_mut_ptr(),
            meminfo.len(),
        )
    };
    let meminfo_n = clamp_len_i32(n, meminfo.len());
    let Some(meminfo_slice) = subslice(&meminfo, 0, meminfo_n) else {
        print("FAIL\n");
        return -1;
    };
    if n <= 0 || !contains(meminfo_slice, b"MemTotal:") {
        print("FAIL\n");
        return -1;
    }
    print("PASS\n");

    print("[test_procfs] test: getdents64 on /proc ... ");
    let proc_fd = unsafe { kernel_sys_open(b"/proc\0".as_ptr(), O_RDONLY, 0) };
    if proc_fd < 0 {
        print("FAIL (open)\n");
        return -2;
    }
    let mut dents = [0u8; 1024];
    let dents_n = unsafe { kernel_sys_getdents64(proc_fd as i32, dents.as_mut_ptr(), dents.len()) };
    unsafe {
        let _ = kernel_sys_close(proc_fd as i32);
    }
    if dents_n <= 0
        || !getdents_contains_name(&dents, dents_n as usize, b"self")
        || !getdents_contains_name(&dents, dents_n as usize, b"meminfo")
        || !getdents_contains_name(&dents, dents_n as usize, b"cpuinfo")
        || !getdents_contains_name(&dents, dents_n as usize, b"uptime")
    {
        print("FAIL\n");
        return -3;
    }
    print("PASS\n");

    print("[test_procfs] test: /proc/self/status via read syscall ... ");
    let status_fd = unsafe { kernel_sys_open(b"/proc/self/status\0".as_ptr(), O_RDONLY, 0) };
    if status_fd < 0 {
        print("FAIL (open)\n");
        return -4;
    }
    let mut status = [0u8; 512];
    let status_n = unsafe { kernel_sys_read(status_fd as i32, status.as_mut_ptr(), status.len()) };
    unsafe {
        let _ = kernel_sys_close(status_fd as i32);
    }
    let tid = unsafe { kernel_sys_gettid() };
    let status_len = clamp_len_i64(status_n, status.len());
    let Some(status_slice) = subslice(&status, 0, status_len) else {
        print("FAIL\n");
        return -5;
    };
    if status_n <= 0
        || !contains(status_slice, b"Pid:\t")
        || !contains(status_slice, b"Name:\t")
        || tid < 0
    {
        print("FAIL\n");
        return -5;
    }
    print("PASS\n");

    print("[test_procfs] test: /proc/self/maps read ... ");
    let maps_fd = unsafe { kernel_sys_open(b"/proc/self/maps\0".as_ptr(), O_RDONLY, 0) };
    if maps_fd < 0 {
        print("FAIL (open)\n");
        return -6;
    }
    let mut maps = [0u8; 512];
    let maps_n = unsafe { kernel_sys_read(maps_fd as i32, maps.as_mut_ptr(), maps.len()) };
    unsafe {
        let _ = kernel_sys_close(maps_fd as i32);
    }
    if maps_n < 0 {
        print("FAIL\n");
        return -7;
    }
    print("PASS\n");

    print("[test_procfs] test: statfs(/proc) ... ");
    let mut st = LinuxStatFs {
        f_type: 0,
        f_bsize: 0,
        f_blocks: 0,
        f_bfree: 0,
        f_bavail: 0,
        f_files: 0,
        f_ffree: 0,
        f_fsid: [0, 0],
        f_namelen: 0,
        f_frsize: 0,
        f_flags: 0,
        f_spare: [0; 4],
    };
    let st_rc = unsafe { kernel_sys_statfs(b"/proc\0".as_ptr(), &mut st as *mut LinuxStatFs as *mut u8) };
    if st_rc != 0 || st.f_type != PROCFS_MAGIC {
        print("FAIL\n");
        return -8;
    }
    print("PASS\n");

    print("[test_procfs] test: pipe2/read/write ... ");
    let mut pipefds = [-1i32; 2];
    let pipe_rc = unsafe { kernel_sys_pipe2(pipefds.as_mut_ptr(), O_CLOEXEC) };
    if pipe_rc != 0 || pipefds[0] < 0 || pipefds[1] < 0 {
        print("FAIL (pipe2)\n");
        return -9;
    }
    let msg = b"pipe-ok";
    let write_n = unsafe { kernel_sys_write(pipefds[1], msg.as_ptr(), msg.len()) };
    let mut pipe_buf = [0u8; 16];
    let read_n = unsafe { kernel_sys_read(pipefds[0], pipe_buf.as_mut_ptr(), pipe_buf.len()) };
    unsafe {
        let _ = kernel_sys_close(pipefds[0]);
        let _ = kernel_sys_close(pipefds[1]);
    }
    let read_len = clamp_len_i64(read_n, pipe_buf.len());
    let Some(read_slice) = subslice(&pipe_buf, 0, read_len) else {
        print("FAIL\n");
        return -10;
    };
    if write_n != msg.len() as i64 || read_n != msg.len() as i64 || !eq_bytes(read_slice, msg) {
        print("FAIL\n");
        return -10;
    }
    print("PASS\n");

    print("[test_procfs] test: readlinkat(non-symlink) ... ");
    let mut link_buf = [0u8; 64];
    let rl_rc = unsafe {
        kernel_sys_readlinkat(
            -100,
            b"/proc/self\0".as_ptr(),
            link_buf.as_mut_ptr(),
            link_buf.len(),
        )
    };
    if rl_rc != EINVAL {
        print("FAIL\n");
        return -11;
    }
    print("PASS\n");

    print("[test_procfs] All tests passed\n");
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn module_exit() {
    print("[test_procfs] Module unloaded\n");
}

#[unsafe(no_mangle)]
pub extern "C" fn module_name() -> *const u8 {
    b"test_procfs\0".as_ptr()
}

#[unsafe(no_mangle)]
pub extern "C" fn module_version() -> *const u8 {
    b"0.1.0\0".as_ptr()
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    print("[test_procfs] PANIC\n");
    loop {}
}
