//! mmap/munmap/mprotect + file-backed COW 테스트 모듈

#![no_std]
#![no_main]

use core::panic::PanicInfo;

unsafe extern "C" {
    fn kernel_print(s: *const u8, len: usize);
    fn kernel_sys_mmap(
        addr: usize,
        len: usize,
        prot: usize,
        flags: usize,
        fd: i64,
        offset: usize,
    ) -> i64;
    fn kernel_sys_munmap(addr: usize, len: usize) -> i64;
    fn kernel_sys_mprotect(addr: usize, len: usize, prot: usize) -> i64;
    fn kernel_sys_open(path: *const u8, flags: u32, mode: u32) -> i64;
    fn kernel_sys_close(fd: i32) -> i64;
    fn kernel_sys_lseek(fd: i32, offset: i64, whence: i32) -> i64;
    fn kernel_vfs_create_file(path: *const u8, path_len: usize) -> i32;
    fn kernel_vfs_write(
        path: *const u8,
        path_len: usize,
        offset: usize,
        data: *const u8,
        data_len: usize,
    ) -> i32;
    fn kernel_vfs_read(
        path: *const u8,
        path_len: usize,
        offset: usize,
        buf: *mut u8,
        buf_len: usize,
    ) -> i32;
}

const PROT_READ: usize = 0x1;
const PROT_WRITE: usize = 0x2;
const MAP_SHARED: usize = 0x01;
const MAP_PRIVATE: usize = 0x02;
const MAP_FIXED: usize = 0x10;
const MAP_ANONYMOUS: usize = 0x20;
const O_RDWR: u32 = 2;
const SEEK_SET: i32 = 0;
const PAGE_SIZE: usize = 4096;
const EBADF: i64 = -9;
const EINVAL: i64 = -22;

const FILE_PATH: &[u8] = b"/mmap_test.bin";
const FILE_PATH_C: &[u8] = b"/mmap_test.bin\0";

fn print(s: &str) {
    unsafe {
        kernel_print(s.as_ptr(), s.len());
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn module_init() -> i32 {
    print("[test_mmap] === mmap syscall Tests ===\n");

    print("[test_mmap] test: anonymous map + mprotect + partial munmap ... ");
    let mapped = unsafe {
        kernel_sys_mmap(
            0,
            PAGE_SIZE * 2,
            PROT_READ | PROT_WRITE,
            MAP_PRIVATE | MAP_ANONYMOUS,
            -1,
            0,
        )
    };
    if mapped <= 0 || (mapped as usize) % PAGE_SIZE != 0 {
        print("FAIL (mmap)\n");
        return -1;
    }

    unsafe {
        let p = mapped as *mut u8;
        p.write_volatile(0x5A);
        p.add(PAGE_SIZE).write_volatile(0xA5);
        if p.read_volatile() != 0x5A || p.add(PAGE_SIZE).read_volatile() != 0xA5 {
            print("FAIL (rw)\n");
            return -2;
        }
    }

    let ro = unsafe { kernel_sys_mprotect(mapped as usize, PAGE_SIZE * 2, PROT_READ) };
    if ro != 0 {
        print("FAIL (mprotect-ro)\n");
        return -3;
    }
    let rw = unsafe { kernel_sys_mprotect(mapped as usize, PAGE_SIZE * 2, PROT_READ | PROT_WRITE) };
    if rw != 0 {
        print("FAIL (mprotect-rw)\n");
        return -4;
    }

    if unsafe { kernel_sys_munmap(mapped as usize, PAGE_SIZE) } != 0 {
        print("FAIL (munmap-head)\n");
        return -5;
    }
    unsafe {
        let tail = (mapped as usize + PAGE_SIZE) as *mut u8;
        tail.write_volatile(0xCC);
        if tail.read_volatile() != 0xCC {
            print("FAIL (tail-rw)\n");
            return -6;
        }
    }
    if unsafe { kernel_sys_munmap(mapped as usize + PAGE_SIZE, PAGE_SIZE) } != 0 {
        print("FAIL (munmap-tail)\n");
        return -7;
    }
    print("PASS\n");

    print("[test_mmap] test: MAP_FIXED replace ... ");
    let base = unsafe {
        kernel_sys_mmap(
            0,
            PAGE_SIZE,
            PROT_READ | PROT_WRITE,
            MAP_PRIVATE | MAP_ANONYMOUS,
            -1,
            0,
        )
    };
    if base <= 0 {
        print("FAIL (base-map)\n");
        return -8;
    }
    let fixed = unsafe {
        kernel_sys_mmap(
            base as usize,
            PAGE_SIZE,
            PROT_READ | PROT_WRITE,
            MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED,
            -1,
            0,
        )
    };
    if fixed != base {
        print("FAIL (fixed)\n");
        return -9;
    }
    unsafe {
        let p = fixed as *mut u8;
        p.write_volatile(0x7E);
        if p.read_volatile() != 0x7E {
            print("FAIL (fixed-rw)\n");
            return -10;
        }
    }
    if unsafe { kernel_sys_munmap(fixed as usize, PAGE_SIZE) } != 0 {
        print("FAIL (fixed-unmap)\n");
        return -11;
    }
    print("PASS\n");

    #[cfg(target_arch = "riscv64")]
    {
        print("[test_mmap] test: file-backed mmap arg check (riscv64) ... ");
        let unsupported =
            unsafe { kernel_sys_mmap(0, PAGE_SIZE, PROT_READ | PROT_WRITE, MAP_PRIVATE, -1, 0) };
        if unsupported != EBADF {
            print("FAIL\n");
            return -12;
        }
        print("PASS\n");
    }

    #[cfg(target_arch = "aarch64")]
    {
        print("[test_mmap] test: file-backed MAP_SHARED/MAP_PRIVATE + COW ... ");

        let _ = unsafe { kernel_vfs_create_file(FILE_PATH.as_ptr(), FILE_PATH.len()) };
        let mut init_page = [0u8; PAGE_SIZE];
        init_page[0] = 0x11;
        init_page[1] = 0x22;
        let wrote = unsafe {
            kernel_vfs_write(
                FILE_PATH.as_ptr(),
                FILE_PATH.len(),
                0,
                init_page.as_ptr(),
                init_page.len(),
            )
        };
        if wrote != PAGE_SIZE as i32 {
            print("FAIL (seed-write)\n");
            return -12;
        }

        let fd1 = unsafe { kernel_sys_open(FILE_PATH_C.as_ptr(), O_RDWR, 0) };
        let fd2 = unsafe { kernel_sys_open(FILE_PATH_C.as_ptr(), O_RDWR, 0) };
        if fd1 < 0 || fd2 < 0 {
            print("FAIL (open)\n");
            return -13;
        }
        if unsafe { kernel_sys_lseek(fd1 as i32, 0, SEEK_SET) } < 0 {
            print("FAIL (lseek)\n");
            return -14;
        }

        let shared1 = unsafe {
            kernel_sys_mmap(
                0,
                PAGE_SIZE,
                PROT_READ | PROT_WRITE,
                MAP_SHARED,
                fd1,
                0,
            )
        };
        let shared2 = unsafe {
            kernel_sys_mmap(
                0,
                PAGE_SIZE,
                PROT_READ | PROT_WRITE,
                MAP_SHARED,
                fd2,
                0,
            )
        };
        if shared1 <= 0 || shared2 <= 0 {
            print("FAIL (shared-map)\n");
            return -15;
        }

        unsafe {
            let p1 = shared1 as *mut u8;
            let p2 = shared2 as *mut u8;
            if p1.read_volatile() != 0x11 || p2.read_volatile() != 0x11 {
                print("FAIL (shared-init)\n");
                return -16;
            }
            p1.write_volatile(0x55);
            if p2.read_volatile() != 0x55 {
                print("FAIL (shared-cross-fd)\n");
                return -17;
            }
        }

        let private_map = unsafe {
            kernel_sys_mmap(
                0,
                PAGE_SIZE,
                PROT_READ | PROT_WRITE,
                MAP_PRIVATE,
                fd1,
                0,
            )
        };
        if private_map <= 0 {
            print("FAIL (private-map)\n");
            return -18;
        }
        unsafe {
            let p = private_map as *mut u8;
            p.write_volatile(0xAA); // first write should fault-COW
            p.write_volatile(0xAB); // second write should not copy again
            let s = shared1 as *mut u8;
            if s.read_volatile() != 0x55 {
                print("FAIL (private-isolation)\n");
                return -19;
            }
        }

        if unsafe { kernel_sys_munmap(private_map as usize, PAGE_SIZE) } != 0 {
            print("FAIL (private-unmap)\n");
            return -20;
        }
        if unsafe { kernel_sys_munmap(shared1 as usize, PAGE_SIZE) } != 0 {
            print("FAIL (shared1-unmap)\n");
            return -21;
        }
        if unsafe { kernel_sys_munmap(shared2 as usize, PAGE_SIZE) } != 0 {
            print("FAIL (shared2-unmap)\n");
            return -22;
        }

        let mut verify = [0u8; 4];
        let read_back = unsafe {
            kernel_vfs_read(
                FILE_PATH.as_ptr(),
                FILE_PATH.len(),
                0,
                verify.as_mut_ptr(),
                verify.len(),
            )
        };
        if read_back < 1 || verify[0] != 0x55 {
            print("FAIL (writeback)\n");
            return -23;
        }

        let shared3 = unsafe {
            kernel_sys_mmap(
                0,
                PAGE_SIZE,
                PROT_READ | PROT_WRITE,
                MAP_SHARED,
                fd1,
                0,
            )
        };
        if shared3 <= 0 {
            print("FAIL (remap)\n");
            return -24;
        }
        unsafe {
            let p = shared3 as *mut u8;
            if p.read_volatile() != 0x55 {
                print("FAIL (remap-data)\n");
                return -25;
            }
        }
        if unsafe { kernel_sys_munmap(shared3 as usize, PAGE_SIZE) } != 0 {
            print("FAIL (remap-unmap)\n");
            return -26;
        }

        let bad_fd =
            unsafe { kernel_sys_mmap(0, PAGE_SIZE, PROT_READ | PROT_WRITE, MAP_SHARED, -1, 0) };
        if bad_fd != EBADF {
            print("FAIL (bad-fd)\n");
            return -27;
        }
        let bad_offset =
            unsafe { kernel_sys_mmap(0, PAGE_SIZE, PROT_READ | PROT_WRITE, MAP_SHARED, fd1, 1) };
        if bad_offset != EINVAL {
            print("FAIL (bad-offset)\n");
            return -28;
        }
        let eof_over = unsafe {
            kernel_sys_mmap(
                0,
                PAGE_SIZE * 2,
                PROT_READ | PROT_WRITE,
                MAP_SHARED,
                fd1,
                0,
            )
        };
        if eof_over != EINVAL {
            print("FAIL (bad-eof)\n");
            return -29;
        }

        if unsafe { kernel_sys_close(fd1 as i32) } != 0 || unsafe { kernel_sys_close(fd2 as i32) } != 0 {
            print("FAIL (close)\n");
            return -30;
        }

        print("PASS\n");
    }

    print("[test_mmap] All tests passed\n");
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn module_exit() {
    print("[test_mmap] Module unloaded\n");
}

#[unsafe(no_mangle)]
pub extern "C" fn module_name() -> *const u8 {
    b"test_mmap\0".as_ptr()
}

#[unsafe(no_mangle)]
pub extern "C" fn module_version() -> *const u8 {
    b"0.1.0\0".as_ptr()
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    print("[test_mmap] PANIC!\n");
    loop {}
}
