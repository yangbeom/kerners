//! 유저 모드 지원
//!
//! EL0 (AArch64) / U-mode (RISC-V) 전환 및 유저 프로세스 관리

use alloc::string::String;
use alloc::vec::Vec;
use crate::fs::{self, VfsError};
use crate::module::elf::{program_type, Elf64ProgramHeader, ElfType};
use crate::module::{ModuleError, ModuleLoader};
use crate::kprintln;
use crate::sync::Mutex;

/// 유저 스택 크기 (64KB)
pub const USER_STACK_SIZE: usize = 64 * 1024;
const MAX_EXECUTABLE_SIZE: usize = 16 * 1024 * 1024; // 16MB
const AUXV_AT_NULL: usize = 0;
const AUXV_AT_PHDR: usize = 3;
const AUXV_AT_PHNUM: usize = 5;
const AUXV_AT_PAGESZ: usize = 6;
const AUXV_AT_ENTRY: usize = 9;

/// 유저 스택 베이스 주소 (가상 주소, 높은 주소에서 시작)
/// 실제로는 물리 메모리를 매핑해야 하지만, 현재는 identity mapping 사용
#[cfg(target_arch = "aarch64")]
pub const USER_STACK_BASE: usize = 0x0000_0000_8000_0000;  // 2GB

#[cfg(target_arch = "riscv64")]
pub const USER_STACK_BASE: usize = 0x0000_0000_C000_0000;  // 3GB

/// 유저 프로세스 구조체
pub struct UserProcess {
    /// 유저 스택 (커널에서 할당)
    pub user_stack: Vec<u8>,
    /// 스택 탑 주소
    pub stack_top: usize,
    /// 엔트리 포인트
    pub entry: usize,
}

impl UserProcess {
    /// 새 유저 프로세스 생성
    pub fn new(entry: usize) -> Self {
        let mut user_stack = Vec::with_capacity(USER_STACK_SIZE);
        user_stack.resize(USER_STACK_SIZE, 0);
        
        // 스택 탑 계산 (16바이트 정렬)
        let stack_top = user_stack.as_ptr() as usize + USER_STACK_SIZE;
        let stack_top = stack_top & !0xF;
        
        kprintln!("[user] Created user process: entry={:#x}, stack_top={:#x}", 
                  entry, stack_top);
        
        UserProcess {
            user_stack,
            stack_top,
            entry,
        }
    }
    
    /// 유저 모드로 전환하여 실행
    /// 
    /// # Safety
    /// 유저 코드가 유효한 주소를 가리켜야 함
    #[cfg(target_arch = "aarch64")]
    pub unsafe fn run(&self) -> ! {
        kprintln!("[user] Switching to EL0...");
        
        // EL0로 전환
        // SPSR_EL1: 0 = EL0t (EL0, SP_EL0 사용)
        // ELR_EL1: 유저 엔트리 포인트
        // SP_EL0: 유저 스택
        unsafe {
            core::arch::asm!(
                // SPSR_EL1 = 0 (EL0t, 모든 인터럽트 활성화)
                "msr spsr_el1, xzr",
                
                // ELR_EL1 = 유저 엔트리 포인트
                "msr elr_el1, {entry}",
                
                // SP_EL0 = 유저 스택
                "msr sp_el0, {sp}",
                
                // EL0로 전환
                "eret",
                entry = in(reg) self.entry,
                sp = in(reg) self.stack_top,
                options(noreturn)
            );
        }
    }
    
    /// 유저 모드로 전환하여 실행 (RISC-V)
    #[cfg(target_arch = "riscv64")]
    pub unsafe fn run(&self) -> ! {
        kprintln!("[user] Switching to U-mode...");
        
        // M-mode에서 U-mode로 전환
        // mstatus.MPP = 0 (U-mode)
        // mepc = 유저 엔트리 포인트
        unsafe {
            core::arch::asm!(
                // mstatus.MPP 클리어 (U-mode로 설정)
                "li t0, 0x1800",        // MPP 비트 마스크 (bits 11-12)
                "csrc mstatus, t0",     // MPP = 0 (U-mode)
                
                // mstatus.MPIE 설정 (mret 후 인터럽트 활성화)
                "li t0, 0x80",          // MPIE 비트
                "csrs mstatus, t0",
                
                // mepc = 유저 엔트리
                "csrw mepc, {entry}",
                
                // 스택 설정
                "mv sp, {sp}",
                
                // U-mode로 전환
                "mret",
                entry = in(reg) self.entry,
                sp = in(reg) self.stack_top,
                options(noreturn)
            );
        }
    }
}

/// execve 준비 과정 에러
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecError {
    NotFound,
    IoError,
    InvalidElf,
    UnsupportedExecutableType,
    DynamicElfNotSupported,
    OutOfMemory,
    InvalidArgument,
}

/// execve 전이 정보 (트랩 복귀 시 컨텍스트 교체용)
pub struct PreparedExecImage {
    /// 엔트리 포인트 주소
    pub entry: usize,
    /// 유저 스택 메모리 (수명 보장을 위해 스레드에 바인딩)
    pub user_stack: Vec<u8>,
    /// 유저 스택 포인터 (argc 위치)
    pub stack_top: usize,
    /// argc 값
    pub argc: usize,
    /// argv 포인터
    pub argv: usize,
    /// envp 포인터
    pub envp: usize,
}

struct ExecAuxv {
    entry: usize,
    phdr: usize,
    phnum: usize,
    pagesz: usize,
}

/// 부팅 시 PID 1 이미지를 전달하기 위한 단일 슬롯
///
/// 현재는 부트 경로에서 한 번만 사용한다.
static BOOT_INIT_IMAGE: Mutex<Option<PreparedExecImage>> = Mutex::new(None);

/// 부팅용 init(PID 1) 프로세스를 준비하고 스레드를 생성한다.
///
/// 성공 시 init 스레드 tid를 반환한다.
pub fn spawn_init_process(
    path: &str,
    argv: &[String],
    envp: &[String],
) -> Result<crate::proc::Tid, ExecError> {
    let image = prepare_exec_image(path, argv, envp)?;
    {
        let mut slot = BOOT_INIT_IMAGE.lock();
        *slot = Some(image);
    }

    Ok(super::spawn("init", init_process_entry))
}

/// init 스레드 엔트리
///
/// BOOT_INIT_IMAGE 슬롯에서 준비된 이미지를 꺼내 EL0/U-mode로 진입한다.
fn init_process_entry() -> ! {
    let image = {
        let mut slot = BOOT_INIT_IMAGE.lock();
        slot.take()
    };

    let Some(image) = image else {
        kprintln!("[init] no prepared image for init thread");
        super::exit();
    };

    let PreparedExecImage {
        entry,
        user_stack,
        stack_top,
        argc,
        argv,
        envp,
    } = image;

    if !super::set_current_user_stack(user_stack) {
        kprintln!("[init] failed to bind user stack to current thread");
        super::exit();
    }

    kprintln!(
        "[init] entering user mode: entry={:#x}, sp={:#x}, argc={}",
        entry,
        stack_top,
        argc
    );

    unsafe {
        // SAFETY: init_process_entry는 준비된 유저 이미지/스택을 현재 스레드에 바인딩한 뒤
        // 아키텍처별 예외 복귀 경로(eret/mret)로 유저 모드에 진입한다.
        enter_user_image(entry, stack_top, argc, argv, envp);
    }
}

/// 준비된 유저 이미지로 직접 진입 (AArch64)
///
/// # Safety
/// entry/stack_top/argv/envp가 유효한 유저 주소를 가리켜야 한다.
#[cfg(target_arch = "aarch64")]
unsafe fn enter_user_image(entry: usize, stack_top: usize, argc: usize, argv: usize, envp: usize) -> ! {
    unsafe {
        core::arch::asm!(
            "msr spsr_el1, xzr", // EL0t
            "msr elr_el1, {entry}",
            "msr sp_el0, {sp}",
            "mov x0, {argc}",
            "mov x1, {argv}",
            "mov x2, {envp}",
            "eret",
            entry = in(reg) entry,
            sp = in(reg) stack_top,
            argc = in(reg) argc,
            argv = in(reg) argv,
            envp = in(reg) envp,
            options(noreturn)
        );
    }
}

/// 준비된 유저 이미지로 직접 진입 (RISC-V)
///
/// # Safety
/// entry/stack_top/argv/envp가 유효한 유저 주소를 가리켜야 한다.
#[cfg(target_arch = "riscv64")]
unsafe fn enter_user_image(entry: usize, stack_top: usize, argc: usize, argv: usize, envp: usize) -> ! {
    unsafe {
        core::arch::asm!(
            "li t0, 0x1800",   // mstatus.MPP 마스크
            "csrc mstatus, t0",// MPP=U-mode
            "li t0, 0x80",     // mstatus.MPIE
            "csrs mstatus, t0",
            "csrw mepc, {entry}",
            "mv sp, {sp}",
            "mv a0, {argc}",
            "mv a1, {argv}",
            "mv a2, {envp}",
            "mret",
            entry = in(reg) entry,
            sp = in(reg) stack_top,
            argc = in(reg) argc,
            argv = in(reg) argv,
            envp = in(reg) envp,
            options(noreturn)
        );
    }
}

/// static ELF(PT_INTERP 없음) 기준으로 실행 이미지를 준비한다.
pub fn prepare_exec_image(
    path: &str,
    argv: &[String],
    envp: &[String],
) -> Result<PreparedExecImage, ExecError> {
    let elf_data = read_executable(path)?;
    let elf = crate::module::Elf64::parse(&elf_data).map_err(|_| ExecError::InvalidElf)?;

    // Phase 10-1 1차 범위: ET_EXEC만 허용 (ET_DYN은 추후)
    if elf.file_type() != ElfType::Exec {
        return Err(ExecError::UnsupportedExecutableType);
    }

    // 동적 ELF는 추후 로드맵(PT_INTERP)에서 지원
    if elf
        .program_headers()
        .into_iter()
        .flatten()
        .any(|ph| ph.p_type == program_type::PT_INTERP)
    {
        return Err(ExecError::DynamicElfNotSupported);
    }

    if elf.load_segments().next().is_none() {
        return Err(ExecError::InvalidElf);
    }

    let mut auxv = build_exec_auxv(&elf);
    let entry = ModuleLoader::load_executable(&elf_data).map_err(map_module_error)?;
    auxv.entry = entry;

    let (user_stack, stack_top, argc, argv_ptr, envp_ptr) =
        build_user_stack(path, argv, envp, &auxv)?;

    Ok(PreparedExecImage {
        entry,
        user_stack,
        stack_top,
        argc,
        argv: argv_ptr,
        envp: envp_ptr,
    })
}

fn map_module_error(err: ModuleError) -> ExecError {
    match err {
        ModuleError::OutOfMemory => ExecError::OutOfMemory,
        ModuleError::NotFound => ExecError::NotFound,
        ModuleError::InvalidFormat | ModuleError::ElfError(_) => ExecError::InvalidElf,
        _ => ExecError::IoError,
    }
}

fn map_vfs_error(err: VfsError) -> ExecError {
    match err {
        VfsError::NotFound => ExecError::NotFound,
        VfsError::NoSpace => ExecError::OutOfMemory,
        VfsError::InvalidArgument => ExecError::InvalidArgument,
        _ => ExecError::IoError,
    }
}

#[inline(never)]
fn read_executable(path: &str) -> Result<Vec<u8>, ExecError> {
    let node = match fs::lookup_path(path) {
        Ok(node) => node,
        Err(err) => return Err(map_vfs_error(err)),
    };

    let stat = match node.stat() {
        Ok(stat) => stat,
        Err(err) => return Err(map_vfs_error(err)),
    };

    let file_size = stat.size as usize;
    if file_size == 0 {
        return Err(ExecError::InvalidElf);
    }
    if file_size > MAX_EXECUTABLE_SIZE {
        return Err(ExecError::InvalidArgument);
    }

    let mut buf = Vec::new();
    buf.resize(file_size, 0);
    kprintln!(
        "[exec] reading '{}' size={} bytes into {:#x}",
        path,
        file_size,
        buf.as_ptr() as usize
    );

    let n = match node.read(0, &mut buf) {
        Ok(n) => n,
        Err(err) => return Err(map_vfs_error(err)),
    };
    kprintln!("[exec] read complete '{}' -> {} bytes", path, n);
    if n != file_size {
        return Err(ExecError::IoError);
    }

    Ok(buf)
}

fn build_user_stack(
    path: &str,
    argv: &[String],
    envp: &[String],
    auxv: &ExecAuxv,
) -> Result<(Vec<u8>, usize, usize, usize, usize), ExecError> {
    let mut user_stack = Vec::new();
    user_stack.resize(USER_STACK_SIZE, 0);

    #[cfg(target_arch = "aarch64")]
    let stack_base = USER_STACK_BASE - USER_STACK_SIZE;
    #[cfg(target_arch = "aarch64")]
    let mut sp = USER_STACK_BASE;

    #[cfg(not(target_arch = "aarch64"))]
    let stack_base = user_stack.as_ptr() as usize;
    #[cfg(not(target_arch = "aarch64"))]
    let mut sp = stack_base + USER_STACK_SIZE;

    let mut argv_owned = if argv.is_empty() {
        let mut v = Vec::new();
        v.push(String::from(path));
        v
    } else {
        argv.to_vec()
    };

    // argv[0]은 빈 문자열보다 실행 경로를 유지하는 쪽이 디버깅에 유리함
    if argv_owned[0].is_empty() {
        argv_owned[0] = String::from(path);
    }

    let mut env_ptrs = Vec::new();
    for s in envp.iter().rev() {
        let ptr = push_c_string(&mut user_stack, stack_base, &mut sp, s)?;
        env_ptrs.push(ptr);
    }
    env_ptrs.reverse();

    let mut argv_ptrs = Vec::new();
    for s in argv_owned.iter().rev() {
        let ptr = push_c_string(&mut user_stack, stack_base, &mut sp, s)?;
        argv_ptrs.push(ptr);
    }
    argv_ptrs.reverse();

    let argc = argv_ptrs.len();
    let usize_bytes = core::mem::size_of::<usize>();

    // Linux 프로세스 시작 스택: argc, argv[], NULL, envp[], NULL, auxv...
    let mut words = Vec::new();
    words.push(argc);
    words.extend(argv_ptrs.iter().copied());
    words.push(0);
    words.extend(env_ptrs.iter().copied());
    words.push(0);
    words.push(AUXV_AT_ENTRY);
    words.push(auxv.entry);
    words.push(AUXV_AT_PHDR);
    words.push(auxv.phdr);
    words.push(AUXV_AT_PHNUM);
    words.push(auxv.phnum);
    words.push(AUXV_AT_PAGESZ);
    words.push(auxv.pagesz);
    words.push(AUXV_AT_NULL);
    words.push(0);

    let table_bytes = words.len() * usize_bytes;
    if sp < stack_base + table_bytes {
        return Err(ExecError::OutOfMemory);
    }

    let table_start = (sp - table_bytes) & !0xF;
    if table_start < stack_base {
        return Err(ExecError::OutOfMemory);
    }

    for (i, value) in words.iter().enumerate() {
        let addr = table_start + i * usize_bytes;
        let offset = addr - stack_base;
        let bytes = value.to_ne_bytes();
        user_stack[offset..offset + usize_bytes].copy_from_slice(&bytes);
    }

    let argv_ptr = table_start + usize_bytes;
    let envp_ptr = argv_ptr + (argc + 1) * usize_bytes;

    #[cfg(target_arch = "aarch64")]
    map_user_stack_pages(stack_base, &user_stack)?;

    Ok((user_stack, table_start, argc, argv_ptr, envp_ptr))
}

fn push_c_string(
    user_stack: &mut [u8],
    stack_base: usize,
    sp: &mut usize,
    s: &str,
) -> Result<usize, ExecError> {
    let bytes = s.as_bytes();
    let required = bytes.len() + 1;

    if *sp < stack_base + required {
        return Err(ExecError::OutOfMemory);
    }

    *sp -= required;
    let offset = *sp - stack_base;
    user_stack[offset..offset + bytes.len()].copy_from_slice(bytes);
    user_stack[offset + bytes.len()] = 0;

    Ok(*sp)
}

fn build_exec_auxv(elf: &crate::module::Elf64<'_>) -> ExecAuxv {
    let phnum = elf.header.e_phnum as usize;
    let phdr = find_phdr_vaddr(elf).unwrap_or(0);

    ExecAuxv {
        entry: elf.entry_point() as usize,
        phdr,
        phnum,
        pagesz: crate::mm::page::PAGE_SIZE,
    }
}

fn find_phdr_vaddr(elf: &crate::module::Elf64<'_>) -> Option<usize> {
    let phdrs = elf.program_headers()?;

    if let Some(phdr_seg) = phdrs.iter().find(|ph| ph.p_type == program_type::PT_PHDR) {
        return Some(phdr_seg.p_vaddr as usize);
    }

    let phoff = elf.header.e_phoff as usize;
    let phent = core::mem::size_of::<Elf64ProgramHeader>();
    let ph_size = phent.checked_mul(elf.header.e_phnum as usize)?;
    let ph_end = phoff.checked_add(ph_size)?;

    for load in phdrs.iter().filter(|ph| ph.p_type == program_type::PT_LOAD) {
        let load_off = load.p_offset as usize;
        let load_end = load_off.checked_add(load.p_filesz as usize)?;
        if phoff < load_off || ph_end > load_end {
            continue;
        }
        let delta = phoff - load_off;
        return (load.p_vaddr as usize).checked_add(delta);
    }

    None
}

#[cfg(target_arch = "aarch64")]
fn map_user_stack_pages(stack_base: usize, stack_bytes: &[u8]) -> Result<(), ExecError> {
    let page_size = crate::mm::page::PAGE_SIZE;
    let mut frames: Vec<usize> = Vec::new();

    for offset in (0..stack_bytes.len()).step_by(page_size) {
        let frame = if let Some(frame) = crate::mm::page::alloc_frame() {
            frame
        } else {
            cleanup_stack_frames(&mut frames);
            return Err(ExecError::OutOfMemory);
        };
        frames.push(frame);

        if crate::arch::mmu::map_user_page_noflush(stack_base + offset, frame, true, false).is_err() {
            cleanup_stack_frames(&mut frames);
            return Err(ExecError::OutOfMemory);
        }

        let end = core::cmp::min(offset + page_size, stack_bytes.len());
        let copy_len = end - offset;
        unsafe {
            // SAFETY: frame은 alloc_frame()으로 확보한 유효 페이지이고 src 범위는 stack_bytes 내부다.
            core::ptr::copy_nonoverlapping(
                stack_bytes[offset..end].as_ptr(),
                frame as *mut u8,
                copy_len,
            );
        }
    }

    crate::arch::mmu::flush_tlb_all();
    Ok(())
}

#[cfg(target_arch = "aarch64")]
fn cleanup_stack_frames(frames: &mut Vec<usize>) {
    for frame in frames.drain(..) {
        unsafe {
            crate::mm::page::free_frame(frame);
        }
    }
}

/// 간단한 유저 프로그램 (커널 내에 포함)
/// syscall을 사용하여 "Hello from user mode!" 출력 후 종료
#[cfg(target_arch = "aarch64")]
#[unsafe(naked)]
pub unsafe extern "C" fn simple_user_program() -> ! {
    core::arch::naked_asm!(
        // write(1, message, len)
        "mov x0, #1",           // fd = stdout
        "adr x1, 2f",           // buf = message
        "mov x2, #23",          // len
        "mov x8, #64",          // syscall: write
        "svc #0",
        
        // exit(0)
        "mov x0, #0",           // status = 0
        "mov x8, #93",          // syscall: exit
        "svc #0",
        
        // 도달하면 안 됨
        "1: wfi",
        "b 1b",
        
        // 메시지 데이터
        ".balign 8",
        "2: .ascii \"Hello from user mode!\\n\\0\"",
    );
}

#[cfg(target_arch = "riscv64")]
#[unsafe(naked)]
pub unsafe extern "C" fn simple_user_program() -> ! {
    core::arch::naked_asm!(
        // write(1, message, len)
        "li a0, 1",             // fd = stdout
        "la a1, 2f",            // buf = message
        "li a2, 23",            // len
        "li a7, 64",            // syscall: write
        "ecall",
        
        // exit(0)
        "li a0, 0",             // status = 0
        "li a7, 93",            // syscall: exit
        "ecall",
        
        // 도달하면 안 됨
        "1: wfi",
        "j 1b",
        
        // 메시지 데이터
        ".balign 8",
        "2: .ascii \"Hello from user mode!\\n\\0\"",
    );
}

/// 유저 프로그램을 실행하는 커널 스레드 엔트리
fn user_thread_entry() -> ! {
    let entry = simple_user_program as usize;
    crate::kprintln!("[user] User thread started, entry: {:#x}", entry);
    
    let user_proc = UserProcess::new(entry);
    
    unsafe {
        user_proc.run();
    }
}

/// 유저 프로그램 테스트 실행
/// 별도의 커널 스레드를 생성하여 유저 모드 실행
pub fn test_user_mode() {
    kprintln!("\n[user] Testing user mode...");
    kprintln!("[user] Spawning user thread...");
    
    // 별도 스레드에서 유저 프로그램 실행
    let tid = super::spawn("user-test", user_thread_entry);
    kprintln!("[user] User thread spawned (tid={})", tid);
    kprintln!("[user] The user program will run on next schedule.");
}
