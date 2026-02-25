//! execve 준비 경로 테스트 모듈
//!
//! 테스트 항목:
//! 1. 존재하지 않는 파일 → ENOENT(-2)
//! 2. ELF가 아닌 파일 → ENOEXEC(-8)
//! 3. DT_NEEDED 의존성 누락 → ENOENT(-2)
//! 4. DT_NEEDED 의존성 존재 시 동적 ELF 준비 성공

#![no_std]
#![no_main]

use core::panic::PanicInfo;

unsafe extern "C" {
    fn kernel_print(s: *const u8, len: usize);
    fn kernel_vfs_mkdir(path: *const u8, path_len: usize) -> i32;
    fn kernel_vfs_create_file(path: *const u8, path_len: usize) -> i32;
    fn kernel_vfs_write(
        path: *const u8,
        path_len: usize,
        offset: usize,
        data: *const u8,
        data_len: usize,
    ) -> i32;
    fn kernel_vfs_unlink(path: *const u8, path_len: usize) -> i32;
    fn kernel_exec_prepare(path: *const u8, path_len: usize) -> i32;
}

fn print(s: &str) {
    unsafe { kernel_print(s.as_ptr(), s.len()); }
}

const ELF_FILE_SIZE: usize = 0x200;
const ELF_HEADER_SIZE: usize = 0x40;
const PROGRAM_HEADER_SIZE: usize = 0x38;
const PROGRAM_HEADER_TABLE_OFFSET: usize = ELF_HEADER_SIZE;

const SEGMENT_FILE_OFFSET: usize = 0x100;
const SEGMENT_FILE_SIZE: usize = 0x100;
const SEGMENT_VADDR: usize = 0x0020_0000;
const DYNAMIC_OFFSET_IN_SEGMENT: usize = 0x00;
const DYNSTR_OFFSET_IN_SEGMENT: usize = 0x80;

const PT_LOAD: u32 = 1;
const PT_DYNAMIC: u32 = 2;
const PF_W: u32 = 0x2;
const PF_R: u32 = 0x4;
const ET_DYN: u16 = 3;

const DT_NULL: u64 = 0;
const DT_NEEDED: u64 = 1;
const DT_STRTAB: u64 = 5;
const DT_STRSZ: u64 = 10;

const EV_CURRENT: u32 = 1;

const MISSING_DYNAMIC_PATH: &[u8] = b"/execve_dyn_missing.elf";
const MISSING_DEP_NAME: &[u8] = b"libphase15_missing.so";
const OK_DYNAMIC_PATH: &[u8] = b"/execve_dyn_ok.elf";
const LIB_DIR_PATH: &[u8] = b"/lib";
const OK_DEP_PATH: &[u8] = b"/lib/libphase15_dep.so";
const OK_DEP_NAME: &[u8] = b"libphase15_dep.so";

#[cfg(target_arch = "aarch64")]
const ELF_MACHINE: u16 = 183;
#[cfg(target_arch = "riscv64")]
const ELF_MACHINE: u16 = 243;

fn write_u16(buf: &mut [u8], offset: usize, value: u16) {
    let bytes = value.to_le_bytes();
    buf[offset] = bytes[0];
    buf[offset + 1] = bytes[1];
}

fn write_u32(buf: &mut [u8], offset: usize, value: u32) {
    let bytes = value.to_le_bytes();
    buf[offset..offset + 4].copy_from_slice(&bytes);
}

fn write_u64(buf: &mut [u8], offset: usize, value: u64) {
    let bytes = value.to_le_bytes();
    buf[offset..offset + 8].copy_from_slice(&bytes);
}

fn write_ident(buf: &mut [u8]) {
    buf[0] = 0x7f;
    buf[1] = b'E';
    buf[2] = b'L';
    buf[3] = b'F';
    buf[4] = 2; // ELF64
    buf[5] = 1; // little-endian
    buf[6] = 1; // EV_CURRENT
    buf[7] = 0; // System V ABI
}

fn write_program_header(
    buf: &mut [u8],
    offset: usize,
    p_type: u32,
    p_flags: u32,
    p_offset: u64,
    p_vaddr: u64,
    p_filesz: u64,
    p_memsz: u64,
    p_align: u64,
) {
    write_u32(buf, offset, p_type);
    write_u32(buf, offset + 4, p_flags);
    write_u64(buf, offset + 8, p_offset);
    write_u64(buf, offset + 16, p_vaddr);
    write_u64(buf, offset + 24, 0);
    write_u64(buf, offset + 32, p_filesz);
    write_u64(buf, offset + 40, p_memsz);
    write_u64(buf, offset + 48, p_align);
}

fn write_dynamic_entry(buf: &mut [u8], offset: usize, tag: u64, value: u64) {
    write_u64(buf, offset, tag);
    write_u64(buf, offset + 8, value);
}

fn build_dynamic_elf(buf: &mut [u8; ELF_FILE_SIZE], needed_name: Option<&[u8]>) -> usize {
    for byte in buf.iter_mut() {
        *byte = 0;
    }

    write_ident(&mut buf[0..16]);
    write_u16(buf, 16, ET_DYN);
    write_u16(buf, 18, ELF_MACHINE);
    write_u32(buf, 20, EV_CURRENT);
    write_u64(buf, 24, (SEGMENT_VADDR + 0x40) as u64);
    write_u64(buf, 32, PROGRAM_HEADER_TABLE_OFFSET as u64);
    write_u64(buf, 40, 0);
    write_u32(buf, 48, 0);
    write_u16(buf, 52, ELF_HEADER_SIZE as u16);
    write_u16(buf, 54, PROGRAM_HEADER_SIZE as u16);
    write_u16(buf, 56, 2);
    write_u16(buf, 58, 0);
    write_u16(buf, 60, 0);
    write_u16(buf, 62, 0);

    write_program_header(
        buf,
        PROGRAM_HEADER_TABLE_OFFSET,
        PT_LOAD,
        PF_R | PF_W,
        SEGMENT_FILE_OFFSET as u64,
        SEGMENT_VADDR as u64,
        SEGMENT_FILE_SIZE as u64,
        SEGMENT_FILE_SIZE as u64,
        0x1000,
    );

    let mut dyn_entry_count = 3usize; // DT_STRTAB, DT_STRSZ, DT_NULL
    if needed_name.is_some() {
        dyn_entry_count += 1; // DT_NEEDED
    }
    let dynamic_size = dyn_entry_count * 16;

    write_program_header(
        buf,
        PROGRAM_HEADER_TABLE_OFFSET + PROGRAM_HEADER_SIZE,
        PT_DYNAMIC,
        PF_R,
        (SEGMENT_FILE_OFFSET + DYNAMIC_OFFSET_IN_SEGMENT) as u64,
        (SEGMENT_VADDR + DYNAMIC_OFFSET_IN_SEGMENT) as u64,
        dynamic_size as u64,
        dynamic_size as u64,
        8,
    );

    let dynstr_start = SEGMENT_FILE_OFFSET + DYNSTR_OFFSET_IN_SEGMENT;
    buf[dynstr_start] = 0;
    let mut dynstr_size = 1usize;

    let needed_offset = if let Some(name) = needed_name {
        let name_start = dynstr_start + 1;
        buf[name_start..name_start + name.len()].copy_from_slice(name);
        buf[name_start + name.len()] = 0;
        dynstr_size = 1 + name.len() + 1;
        1usize
    } else {
        0usize
    };

    let mut dyn_off = SEGMENT_FILE_OFFSET + DYNAMIC_OFFSET_IN_SEGMENT;
    write_dynamic_entry(
        buf,
        dyn_off,
        DT_STRTAB,
        (SEGMENT_VADDR + DYNSTR_OFFSET_IN_SEGMENT) as u64,
    );
    dyn_off += 16;

    write_dynamic_entry(buf, dyn_off, DT_STRSZ, dynstr_size as u64);
    dyn_off += 16;

    if needed_name.is_some() {
        write_dynamic_entry(buf, dyn_off, DT_NEEDED, needed_offset as u64);
        dyn_off += 16;
    }

    write_dynamic_entry(buf, dyn_off, DT_NULL, 0);
    ELF_FILE_SIZE
}

fn create_file_with_contents(path: &[u8], payload: &[u8]) -> bool {
    let _ = unsafe { kernel_vfs_unlink(path.as_ptr(), path.len()) };

    if unsafe { kernel_vfs_create_file(path.as_ptr(), path.len()) } != 0 {
        return false;
    }

    let written = unsafe {
        kernel_vfs_write(
            path.as_ptr(),
            path.len(),
            0,
            payload.as_ptr(),
            payload.len(),
        )
    };
    written == payload.len() as i32
}

#[unsafe(no_mangle)]
pub extern "C" fn module_init() -> i32 {
    print("[test_execve] === execve Prepare Tests ===\n");

    // 테스트 1: 존재하지 않는 경로
    print("[test_execve] test: ENOENT on missing path ... ");
    let missing = b"/no/such/binary";
    let ret = unsafe { kernel_exec_prepare(missing.as_ptr(), missing.len()) };
    if ret != -2 {
        print("FAIL\n");
        return -1;
    }
    print("PASS\n");

    // 테스트 2: ELF가 아닌 파일
    print("[test_execve] test: ENOEXEC on non-ELF ... ");
    let invalid_path = b"/execve_invalid.bin";
    let payload = b"this is not an elf binary";

    // 이전 테스트 잔여 파일이 있으면 삭제
    let _ = unsafe { kernel_vfs_unlink(invalid_path.as_ptr(), invalid_path.len()) };

    let create_ret = unsafe { kernel_vfs_create_file(invalid_path.as_ptr(), invalid_path.len()) };
    if create_ret != 0 {
        print("FAIL (create)\n");
        return -2;
    }

    let write_ret = unsafe {
        kernel_vfs_write(
            invalid_path.as_ptr(),
            invalid_path.len(),
            0,
            payload.as_ptr(),
            payload.len(),
        )
    };
    if write_ret != payload.len() as i32 {
        print("FAIL (write)\n");
        return -3;
    }

    let exec_ret = unsafe { kernel_exec_prepare(invalid_path.as_ptr(), invalid_path.len()) };
    if exec_ret != -8 {
        print("FAIL\n");
        return -4;
    }
    print("PASS\n");

    let _ = unsafe { kernel_vfs_unlink(invalid_path.as_ptr(), invalid_path.len()) };

    // 테스트 3: 동적 ELF + DT_NEEDED 의존성 누락
    print("[test_execve] test: ENOENT on missing DT_NEEDED dependency ... ");
    let mut dyn_missing = [0u8; ELF_FILE_SIZE];
    let dyn_missing_len = build_dynamic_elf(&mut dyn_missing, Some(MISSING_DEP_NAME));
    if !create_file_with_contents(MISSING_DYNAMIC_PATH, &dyn_missing[..dyn_missing_len]) {
        print("FAIL (create/write)\n");
        return -5;
    }
    let missing_dep_ret = unsafe { kernel_exec_prepare(MISSING_DYNAMIC_PATH.as_ptr(), MISSING_DYNAMIC_PATH.len()) };
    if missing_dep_ret != -2 {
        print("FAIL\n");
        let _ = unsafe { kernel_vfs_unlink(MISSING_DYNAMIC_PATH.as_ptr(), MISSING_DYNAMIC_PATH.len()) };
        return -6;
    }
    print("PASS\n");
    let _ = unsafe { kernel_vfs_unlink(MISSING_DYNAMIC_PATH.as_ptr(), MISSING_DYNAMIC_PATH.len()) };

    // 테스트 4: 동적 ELF + DT_NEEDED 의존성 존재
    print("[test_execve] test: dynamic ELF prepare succeeds with DT_NEEDED present ... ");
    let _ = unsafe { kernel_vfs_mkdir(LIB_DIR_PATH.as_ptr(), LIB_DIR_PATH.len()) };

    let mut dep_elf = [0u8; ELF_FILE_SIZE];
    let dep_elf_len = build_dynamic_elf(&mut dep_elf, None);
    if !create_file_with_contents(OK_DEP_PATH, &dep_elf[..dep_elf_len]) {
        print("FAIL (dep create/write)\n");
        return -7;
    }

    let mut dyn_ok = [0u8; ELF_FILE_SIZE];
    let dyn_ok_len = build_dynamic_elf(&mut dyn_ok, Some(OK_DEP_NAME));
    if !create_file_with_contents(OK_DYNAMIC_PATH, &dyn_ok[..dyn_ok_len]) {
        print("FAIL (main create/write)\n");
        let _ = unsafe { kernel_vfs_unlink(OK_DEP_PATH.as_ptr(), OK_DEP_PATH.len()) };
        return -8;
    }

    let ok_ret = unsafe { kernel_exec_prepare(OK_DYNAMIC_PATH.as_ptr(), OK_DYNAMIC_PATH.len()) };
    if ok_ret != 0 {
        print("FAIL\n");
        let _ = unsafe { kernel_vfs_unlink(OK_DYNAMIC_PATH.as_ptr(), OK_DYNAMIC_PATH.len()) };
        let _ = unsafe { kernel_vfs_unlink(OK_DEP_PATH.as_ptr(), OK_DEP_PATH.len()) };
        return -9;
    }
    print("PASS\n");

    let _ = unsafe { kernel_vfs_unlink(OK_DYNAMIC_PATH.as_ptr(), OK_DYNAMIC_PATH.len()) };
    let _ = unsafe { kernel_vfs_unlink(OK_DEP_PATH.as_ptr(), OK_DEP_PATH.len()) };

    print("[test_execve] All tests passed\n");
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn module_exit() {
    print("[test_execve] Module unloaded\n");
}

#[unsafe(no_mangle)]
pub extern "C" fn module_name() -> *const u8 {
    b"test_execve\0".as_ptr()
}

#[unsafe(no_mangle)]
pub extern "C" fn module_version() -> *const u8 {
    b"0.1.0\0".as_ptr()
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    print("[test_execve] PANIC!\n");
    loop {}
}
