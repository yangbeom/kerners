//! execve 준비 경로 테스트 모듈
//!
//! 테스트 항목:
//! 1. 존재하지 않는 파일 → ENOENT(-2)
//! 2. ELF가 아닌 파일 → ENOEXEC(-8)
//! 3. DT_NEEDED 의존성 누락 → ENOENT(-2)
//! 4. 미해결 동적 심볼 재배치 → ENOEXEC(-8)
//! 5. 미지원 TLS 재배치 타입 → ENOEXEC(-8)
//! 6. DT_NEEDED 의존성 존재 시 동적 ELF 준비 성공

#![no_std]
#![no_main]

use core::panic::PanicInfo;
use core::ptr::{copy_nonoverlapping, write_bytes};

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

const ELF_FILE_SIZE: usize = 0x300;
const ELF_HEADER_SIZE: usize = 0x40;
const PROGRAM_HEADER_SIZE: usize = 0x38;
const PROGRAM_HEADER_TABLE_OFFSET: usize = ELF_HEADER_SIZE;

const SEGMENT_FILE_OFFSET: usize = 0x100;
const SEGMENT_FILE_SIZE: usize = 0x200;
const SEGMENT_VADDR: usize = 0x0020_0000;
const DYNAMIC_OFFSET_IN_SEGMENT: usize = 0x00;
const DYNSTR_OFFSET_IN_SEGMENT: usize = 0x80;
const UNRESOLVED_DYNSTR_OFFSET_IN_SEGMENT: usize = 0x100;
const HASH_OFFSET_IN_SEGMENT: usize = 0x140;
const SYMTAB_OFFSET_IN_SEGMENT: usize = 0x148;
const RELA_OFFSET_IN_SEGMENT: usize = 0x180;
const RELA_TARGET_OFFSET_IN_SEGMENT: usize = 0x1a0;

const PT_LOAD: u32 = 1;
const PT_DYNAMIC: u32 = 2;
const PF_W: u32 = 0x2;
const PF_R: u32 = 0x4;
const ET_DYN: u16 = 3;

const DT_NULL: u64 = 0;
const DT_NEEDED: u64 = 1;
const DT_HASH: u64 = 4;
const DT_STRTAB: u64 = 5;
const DT_SYMTAB: u64 = 6;
const DT_RELA: u64 = 7;
const DT_RELASZ: u64 = 8;
const DT_RELAENT: u64 = 9;
const DT_STRSZ: u64 = 10;
const DT_SYMENT: u64 = 11;

const EV_CURRENT: u32 = 1;

const MISSING_DYNAMIC_PATH: &[u8] = b"/execve_dyn_missing.elf";
const MISSING_DEP_NAME: &[u8] = b"libphase15_missing.so";
const OK_DYNAMIC_PATH: &[u8] = b"/execve_dyn_ok.elf";
const UNRESOLVED_DYNAMIC_PATH: &[u8] = b"/execve_dyn_unresolved.elf";
const UNSUPPORTED_TLS_DYNAMIC_PATH: &[u8] = b"/execve_dyn_tls_unsupported.elf";
const LIB_DIR_PATH: &[u8] = b"/lib";
const OK_DEP_PATH: &[u8] = b"/lib/libphase15_dep.so";
const OK_DEP_NAME: &[u8] = b"libphase15_dep.so";
const UNRESOLVED_SYMBOL_NAME: &[u8] = b"phase15_unresolved_sym";
const UNSUPPORTED_TLS_SYMBOL_NAME: &[u8] = b"phase15_tls_unsupported_sym";

#[cfg(target_arch = "aarch64")]
const ELF_MACHINE: u16 = 183;
#[cfg(target_arch = "riscv64")]
const ELF_MACHINE: u16 = 243;
#[cfg(target_arch = "aarch64")]
const DYN_UNRESOLVED_RELOC_TYPE: u32 = 1025; // R_AARCH64_GLOB_DAT
#[cfg(target_arch = "riscv64")]
const DYN_UNRESOLVED_RELOC_TYPE: u32 = 6; // R_RISCV_GLOB_DAT
#[cfg(target_arch = "aarch64")]
const DYN_TLS_UNSUPPORTED_RELOC_TYPE: u32 = 1031; // R_AARCH64_TLSDESC
#[cfg(target_arch = "riscv64")]
const DYN_TLS_UNSUPPORTED_RELOC_TYPE: u32 = 12; // R_RISCV_TLSDESC

fn clear_elf_buffer(buf: &mut [u8; ELF_FILE_SIZE]) {
    unsafe {
        // SAFETY: `buf` points to a valid mutable buffer of exactly ELF_FILE_SIZE bytes.
        write_bytes(buf.as_mut_ptr(), 0, ELF_FILE_SIZE);
    }
}

fn write_u8(buf: &mut [u8; ELF_FILE_SIZE], offset: usize, value: u8) {
    unsafe {
        // SAFETY: all offsets in this module are computed from fixed in-bounds constants.
        *buf.as_mut_ptr().add(offset) = value;
    }
}

fn write_bytes_at(buf: &mut [u8; ELF_FILE_SIZE], offset: usize, data: &[u8]) {
    unsafe {
        // SAFETY: all `(offset, len)` pairs are derived from fixed in-bounds layout constants.
        copy_nonoverlapping(data.as_ptr(), buf.as_mut_ptr().add(offset), data.len());
    }
}

fn write_u16(buf: &mut [u8; ELF_FILE_SIZE], offset: usize, value: u16) {
    let bytes = value.to_le_bytes();
    write_bytes_at(buf, offset, &bytes);
}

fn write_u32(buf: &mut [u8; ELF_FILE_SIZE], offset: usize, value: u32) {
    let bytes = value.to_le_bytes();
    write_bytes_at(buf, offset, &bytes);
}

fn write_u64(buf: &mut [u8; ELF_FILE_SIZE], offset: usize, value: u64) {
    let bytes = value.to_le_bytes();
    write_bytes_at(buf, offset, &bytes);
}

fn write_i64(buf: &mut [u8; ELF_FILE_SIZE], offset: usize, value: i64) {
    let bytes = value.to_le_bytes();
    write_bytes_at(buf, offset, &bytes);
}

fn write_ident(buf: &mut [u8; ELF_FILE_SIZE]) {
    const IDENT_PREFIX: [u8; 8] = [0x7f, b'E', b'L', b'F', 2, 1, 1, 0];
    write_bytes_at(buf, 0, &IDENT_PREFIX);
}

fn write_program_header(
    buf: &mut [u8; ELF_FILE_SIZE],
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

fn write_dynamic_entry(buf: &mut [u8; ELF_FILE_SIZE], offset: usize, tag: u64, value: u64) {
    write_u64(buf, offset, tag);
    write_u64(buf, offset + 8, value);
}

fn build_dynamic_elf(buf: &mut [u8; ELF_FILE_SIZE], needed_name: Option<&[u8]>) -> usize {
    clear_elf_buffer(buf);

    write_ident(buf);
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
    write_u8(buf, dynstr_start, 0);
    let mut dynstr_size = 1usize;

    let needed_offset = if let Some(name) = needed_name {
        let name_start = dynstr_start + 1;
        write_bytes_at(buf, name_start, name);
        write_u8(buf, name_start + name.len(), 0);
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

fn build_symbol_reloc_elf(
    buf: &mut [u8; ELF_FILE_SIZE],
    symbol_name: &[u8],
    reloc_type: u32,
) -> usize {
    clear_elf_buffer(buf);

    write_ident(buf);
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

    let dynamic_size = 9 * 16; // STRTAB, STRSZ, SYMTAB, SYMENT, HASH, RELA, RELASZ, RELAENT, NULL
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

    let strtab_start = SEGMENT_FILE_OFFSET + UNRESOLVED_DYNSTR_OFFSET_IN_SEGMENT;
    write_u8(buf, strtab_start, 0);
    let sym_name_start = strtab_start + 1;
    write_bytes_at(buf, sym_name_start, symbol_name);
    write_u8(buf, sym_name_start + symbol_name.len(), 0);
    let strtab_size = 1 + symbol_name.len() + 1;

    let hash_start = SEGMENT_FILE_OFFSET + HASH_OFFSET_IN_SEGMENT;
    write_u32(buf, hash_start, 1); // nbucket
    write_u32(buf, hash_start + 4, 2); // nchain (undef + unresolved 심볼)

    let symtab_start = SEGMENT_FILE_OFFSET + SYMTAB_OFFSET_IN_SEGMENT;
    let sym1_off = symtab_start + 24;
    write_u32(buf, sym1_off, 1); // st_name (strtab offset)
    write_u8(buf, sym1_off + 4, 0x10); // STB_GLOBAL | STT_NOTYPE
    write_u8(buf, sym1_off + 5, 0);
    write_u16(buf, sym1_off + 6, 0); // SHN_UNDEF
    write_u64(buf, sym1_off + 8, 0);
    write_u64(buf, sym1_off + 16, 0);

    let rela_start = SEGMENT_FILE_OFFSET + RELA_OFFSET_IN_SEGMENT;
    write_u64(
        buf,
        rela_start,
        (SEGMENT_VADDR + RELA_TARGET_OFFSET_IN_SEGMENT) as u64,
    );
    let r_info = ((1u64) << 32) | (reloc_type as u64);
    write_u64(buf, rela_start + 8, r_info);
    write_i64(buf, rela_start + 16, 0);

    let mut dyn_off = SEGMENT_FILE_OFFSET + DYNAMIC_OFFSET_IN_SEGMENT;
    write_dynamic_entry(
        buf,
        dyn_off,
        DT_STRTAB,
        (SEGMENT_VADDR + UNRESOLVED_DYNSTR_OFFSET_IN_SEGMENT) as u64,
    );
    dyn_off += 16;
    write_dynamic_entry(buf, dyn_off, DT_STRSZ, strtab_size as u64);
    dyn_off += 16;
    write_dynamic_entry(
        buf,
        dyn_off,
        DT_SYMTAB,
        (SEGMENT_VADDR + SYMTAB_OFFSET_IN_SEGMENT) as u64,
    );
    dyn_off += 16;
    write_dynamic_entry(buf, dyn_off, DT_SYMENT, 24);
    dyn_off += 16;
    write_dynamic_entry(
        buf,
        dyn_off,
        DT_HASH,
        (SEGMENT_VADDR + HASH_OFFSET_IN_SEGMENT) as u64,
    );
    dyn_off += 16;
    write_dynamic_entry(
        buf,
        dyn_off,
        DT_RELA,
        (SEGMENT_VADDR + RELA_OFFSET_IN_SEGMENT) as u64,
    );
    dyn_off += 16;
    write_dynamic_entry(buf, dyn_off, DT_RELASZ, 24);
    dyn_off += 16;
    write_dynamic_entry(buf, dyn_off, DT_RELAENT, 24);
    dyn_off += 16;
    write_dynamic_entry(buf, dyn_off, DT_NULL, 0);

    ELF_FILE_SIZE
}

fn build_unresolved_reloc_elf(buf: &mut [u8; ELF_FILE_SIZE]) -> usize {
    build_symbol_reloc_elf(buf, UNRESOLVED_SYMBOL_NAME, DYN_UNRESOLVED_RELOC_TYPE)
}

fn build_unsupported_tls_reloc_elf(buf: &mut [u8; ELF_FILE_SIZE]) -> usize {
    build_symbol_reloc_elf(
        buf,
        UNSUPPORTED_TLS_SYMBOL_NAME,
        DYN_TLS_UNSUPPORTED_RELOC_TYPE,
    )
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
    let _ = build_dynamic_elf(&mut dyn_missing, Some(MISSING_DEP_NAME));
    if !create_file_with_contents(MISSING_DYNAMIC_PATH, &dyn_missing) {
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

    // 테스트 4: 미해결 동적 심볼 재배치
    print("[test_execve] test: ENOEXEC on unresolved dynamic symbol relocation ... ");
    let mut dyn_unresolved = [0u8; ELF_FILE_SIZE];
    let _ = build_unresolved_reloc_elf(&mut dyn_unresolved);
    if !create_file_with_contents(UNRESOLVED_DYNAMIC_PATH, &dyn_unresolved) {
        print("FAIL (create/write)\n");
        return -7;
    }
    let unresolved_ret =
        unsafe { kernel_exec_prepare(UNRESOLVED_DYNAMIC_PATH.as_ptr(), UNRESOLVED_DYNAMIC_PATH.len()) };
    if unresolved_ret != -8 {
        print("FAIL\n");
        let _ = unsafe {
            kernel_vfs_unlink(UNRESOLVED_DYNAMIC_PATH.as_ptr(), UNRESOLVED_DYNAMIC_PATH.len())
        };
        return -8;
    }
    print("PASS\n");
    let _ = unsafe {
        kernel_vfs_unlink(UNRESOLVED_DYNAMIC_PATH.as_ptr(), UNRESOLVED_DYNAMIC_PATH.len())
    };

    // 테스트 5: 미지원 TLS 재배치 타입
    print("[test_execve] test: ENOEXEC on unsupported TLS relocation ... ");
    let mut dyn_tls_unsupported = [0u8; ELF_FILE_SIZE];
    let _ = build_unsupported_tls_reloc_elf(&mut dyn_tls_unsupported);
    if !create_file_with_contents(UNSUPPORTED_TLS_DYNAMIC_PATH, &dyn_tls_unsupported) {
        print("FAIL (create/write)\n");
        return -9;
    }
    let tls_unsupported_ret = unsafe {
        kernel_exec_prepare(
            UNSUPPORTED_TLS_DYNAMIC_PATH.as_ptr(),
            UNSUPPORTED_TLS_DYNAMIC_PATH.len(),
        )
    };
    if tls_unsupported_ret != -8 {
        print("FAIL\n");
        let _ = unsafe {
            kernel_vfs_unlink(
                UNSUPPORTED_TLS_DYNAMIC_PATH.as_ptr(),
                UNSUPPORTED_TLS_DYNAMIC_PATH.len(),
            )
        };
        return -10;
    }
    print("PASS\n");
    let _ = unsafe {
        kernel_vfs_unlink(
            UNSUPPORTED_TLS_DYNAMIC_PATH.as_ptr(),
            UNSUPPORTED_TLS_DYNAMIC_PATH.len(),
        )
    };

    // 테스트 6: 동적 ELF + DT_NEEDED 의존성 존재
    print("[test_execve] test: dynamic ELF prepare succeeds with DT_NEEDED present ... ");
    let _ = unsafe { kernel_vfs_mkdir(LIB_DIR_PATH.as_ptr(), LIB_DIR_PATH.len()) };

    let mut dep_elf = [0u8; ELF_FILE_SIZE];
    let _ = build_dynamic_elf(&mut dep_elf, None);
    if !create_file_with_contents(OK_DEP_PATH, &dep_elf) {
        print("FAIL (dep create/write)\n");
        return -11;
    }

    let mut dyn_ok = [0u8; ELF_FILE_SIZE];
    let _ = build_dynamic_elf(&mut dyn_ok, Some(OK_DEP_NAME));
    if !create_file_with_contents(OK_DYNAMIC_PATH, &dyn_ok) {
        print("FAIL (main create/write)\n");
        let _ = unsafe { kernel_vfs_unlink(OK_DEP_PATH.as_ptr(), OK_DEP_PATH.len()) };
        return -12;
    }

    let ok_ret = unsafe { kernel_exec_prepare(OK_DYNAMIC_PATH.as_ptr(), OK_DYNAMIC_PATH.len()) };
    if ok_ret != 0 {
        print("FAIL\n");
        let _ = unsafe { kernel_vfs_unlink(OK_DYNAMIC_PATH.as_ptr(), OK_DYNAMIC_PATH.len()) };
        let _ = unsafe { kernel_vfs_unlink(OK_DEP_PATH.as_ptr(), OK_DEP_PATH.len()) };
        return -13;
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
