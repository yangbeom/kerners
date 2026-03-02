//! 모듈 로더
//!
//! ELF64 relocatable object (.o) 및 executable 로딩
//! - 섹션 로딩 및 메모리 할당
//! - 재배치 처리 (PLT 스텁 지원)
//! - 모듈 라이프사이클 관리

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::mem::size_of;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::kprintln;
use crate::mm::page::{self, PAGE_SIZE};
use crate::sync::RwLock;

use super::elf::*;
use super::symbol::lookup_symbol;

// ============================================================================
// PLT (Procedure Linkage Table) 지원
// ============================================================================

/// PLT 스텁 크기 (바이트)
#[cfg(target_arch = "aarch64")]
const PLT_ENTRY_SIZE: usize = 16;

#[cfg(target_arch = "riscv64")]
const PLT_ENTRY_SIZE: usize = 16;

/// 최대 PLT 엔트리 수 (페이지당)
const MAX_PLT_ENTRIES: usize = PAGE_SIZE / PLT_ENTRY_SIZE;

/// PLT 테이블 관리
struct PltTable {
    /// PLT 메모리 시작 주소
    base: usize,
    /// 현재 할당된 엔트리 수
    count: usize,
    /// 심볼별 PLT 엔트리 매핑 (target_addr, plt_addr) - Vec으로 변경
    entries: Vec<(usize, usize)>,
}

impl PltTable {
    /// 새 PLT 테이블 생성
    fn new(base: usize) -> Self {
        Self {
            base,
            count: 0,
            entries: Vec::new(),
        }
    }

    /// PLT 엔트리 할당 또는 기존 엔트리 반환
    fn get_or_create(&mut self, target: usize) -> Option<usize> {
        // 이미 존재하면 반환
        if let Some(&(_, plt_addr)) = self.entries.iter().find(|(t, _)| *t == target) {
            return Some(plt_addr);
        }

        // 새 엔트리 할당
        if self.count >= MAX_PLT_ENTRIES {
            return None; // PLT 공간 부족
        }

        let plt_addr = self.base + self.count * PLT_ENTRY_SIZE;
        self.create_stub(plt_addr, target);
        self.entries.push((target, plt_addr));
        self.count += 1;

        Some(plt_addr)
    }

    /// AArch64 PLT 스텁 생성
    #[cfg(target_arch = "aarch64")]
    fn create_stub(&self, plt_addr: usize, target: usize) {
        unsafe {
            let stub = plt_addr as *mut u32;
            // ldr x16, [pc, #8]  ; PC+8에서 64비트 주소 로드
            *stub.offset(0) = 0x5800_0050;
            // br x16             ; x16으로 분기
            *stub.offset(1) = 0xd61f_0200;
            // .quad target       ; 64비트 타겟 주소
            *(stub.offset(2) as *mut u64) = target as u64;
        }
    }

    /// RISC-V PLT 스텁 생성
    #[cfg(target_arch = "riscv64")]
    fn create_stub(&self, plt_addr: usize, target: usize) {
        unsafe {
            let stub = plt_addr as *mut u32;
            // auipc t3, 0       ; t3 = PC (0x00000e17)
            *stub.offset(0) = 0x0000_0e17;
            // ld t3, 8(t3)      ; t3 = [PC+8] (0x008e3e03)
            *stub.offset(1) = 0x008e_3e03;
            // jr t3             ; jump to t3 (0x000e0067)
            *stub.offset(2) = 0x000e_0067;
            // nop (padding)
            *stub.offset(3) = 0x0000_0013;
            // .quad target      ; 64비트 타겟 주소 (offset 16)
            // 주의: RISC-V는 8바이트 정렬 필요하므로 offset 16에 배치
        }
        // 주소는 별도 위치에 저장 (16바이트 오프셋)
        // 실제로는 스텁 바로 다음에 저장
        unsafe {
            let addr_ptr = (plt_addr + 8) as *mut u64;
            *addr_ptr = target as u64;
        }
    }
}

// ============================================================================
// 모듈 에러 및 상태
// ============================================================================

/// 모듈 에러
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleError {
    /// ELF 파싱 에러
    ElfError(Elf64Error),
    /// 메모리 할당 실패
    OutOfMemory,
    /// 심볼을 찾을 수 없음
    SymbolNotFound,
    /// 지원하지 않는 재배치 타입
    UnsupportedRelocation(u32),
    /// 초기화 함수 실패
    InitFailed(i32),
    /// 모듈이 사용 중
    InUse,
    /// 이미 로드됨
    AlreadyLoaded,
    /// 모듈을 찾을 수 없음
    NotFound,
    /// 잘못된 모듈 포맷
    InvalidFormat,
    /// 모듈이 언로딩 중
    ModuleUnloading,
}

impl From<Elf64Error> for ModuleError {
    fn from(e: Elf64Error) -> Self {
        ModuleError::ElfError(e)
    }
}

/// 모듈 상태
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleState {
    /// 로딩 중
    Loading,
    /// 활성 상태
    Live,
    /// 언로딩 중
    Unloading,
}

/// 모듈 상세 정보 (조회용)
#[derive(Debug, Clone)]
pub struct ModuleInfo {
    /// 모듈 이름
    pub name: String,
    /// 버전
    pub version: String,
    /// 베이스 주소
    pub base_addr: usize,
    /// 메모리 크기
    pub size: usize,
    /// 상태
    pub state: ModuleState,
    /// 참조 카운트
    pub ref_count: usize,
    /// 언로딩 중 여부
    pub is_unloading: bool,
    /// Export된 심볼 수
    pub exported_symbol_count: usize,
}

/// 모듈 메타데이터
#[derive(Debug, Clone)]
pub struct Module {
    /// 모듈 이름
    pub name: String,
    /// 버전
    pub version: String,
}

impl Module {
    pub fn new(name: &str) -> Self {
        Self {
            name: String::from(name),
            version: String::from("0.0.0"),
        }
    }
}

/// 로드된 모듈
pub struct LoadedModule {
    /// 모듈 정보
    pub info: Module,
    /// 로드된 베이스 주소
    pub base_addr: usize,
    /// 할당된 총 메모리 크기
    pub size: usize,
    /// 모듈 상태
    pub state: ModuleState,
    /// 참조 카운트
    pub ref_count: AtomicUsize,
    /// 언로딩 중 플래그 (새 참조 획득 차단)
    unloading: AtomicBool,
    /// 초기화 함수 주소
    init_fn: Option<usize>,
    /// 정리 함수 주소
    exit_fn: Option<usize>,
    /// 할당된 페이지들
    pages: Vec<usize>,
    /// 섹션별 로드 주소 (재배치용)
    section_addrs: Vec<usize>,
    /// 모듈이 export한 심볼들 - Vec으로 변경
    pub exported_symbols: Vec<(String, usize)>,
    /// PLT 페이지 주소 (있으면)
    plt_page: Option<usize>,
}

/// 모듈 참조 가드 (RAII)
/// Drop 시 자동으로 참조 카운트 감소
pub struct ModuleRef {
    module_name: String,
}

impl ModuleRef {
    /// 모듈 이름 반환
    pub fn name(&self) -> &str {
        &self.module_name
    }

    /// 모듈에 접근
    pub fn get(&self) -> Option<&'static LoadedModule> {
        let modules = LOADED_MODULES.read();
        modules
            .iter()
            .find(|m| m.info.name == self.module_name)
            .map(|m| unsafe { &*(&**m as *const LoadedModule) })
    }
}

impl Drop for ModuleRef {
    fn drop(&mut self) {
        // 참조 카운트 감소
        if let Some(module) = self.get() {
            module.ref_count.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

impl LoadedModule {
    /// 초기화 함수 호출 (PLT를 통해 extern 함수 사용 가능)
    pub fn init(&self) -> Result<(), ModuleError> {
        if let Some(addr) = self.init_fn {
            // module_init() -> i32 (PLT 사용으로 인자 없음)
            type InitFn = extern "C" fn() -> i32;
            let init: InitFn = unsafe { core::mem::transmute(addr) };
            let result = init();
            if result != 0 {
                return Err(ModuleError::InitFailed(result));
            }
        }
        Ok(())
    }

    /// 정리 함수 호출
    pub fn exit(&self) {
        if let Some(addr) = self.exit_fn {
            let exit: fn() = unsafe { core::mem::transmute(addr) };
            exit();
        }
    }

    /// 참조 카운트 증가 (deprecated: try_get 사용 권장)
    pub fn get(&self) -> usize {
        self.ref_count.fetch_add(1, Ordering::SeqCst)
    }

    /// 참조 카운트 감소
    pub fn put(&self) -> usize {
        self.ref_count.fetch_sub(1, Ordering::SeqCst)
    }

    /// 안전한 참조 획득 (언로딩 중이면 None 반환)
    pub fn try_get(&self) -> Option<usize> {
        // 언로딩 중이면 새 참조 획득 거부
        if self.unloading.load(Ordering::SeqCst) {
            return None;
        }
        Some(self.ref_count.fetch_add(1, Ordering::SeqCst))
    }

    /// 언로딩 중인지 확인
    pub fn is_unloading(&self) -> bool {
        self.unloading.load(Ordering::SeqCst)
    }

    /// 현재 참조 카운트
    pub fn ref_count(&self) -> usize {
        self.ref_count.load(Ordering::SeqCst)
    }

    /// 모듈 내 심볼 조회
    pub fn lookup_symbol(&self, name: &str) -> Option<usize> {
        self.exported_symbols.iter().find(|(n, _)| n == name).map(|(_, addr)| *addr)
    }

    /// 심볼 export (모듈이 자신의 심볼을 등록)
    pub fn export_symbol(&mut self, name: &str, address: usize) {
        if let Some(pos) = self.exported_symbols.iter().position(|(n, _)| n == name) {
            self.exported_symbols[pos] = (String::from(name), address);
        } else {
            self.exported_symbols.push((String::from(name), address));
        }
    }
}

/// 로드된 모듈 목록
static LOADED_MODULES: RwLock<Vec<Box<LoadedModule>>> = RwLock::new(Vec::new());
static NEXT_EXEC_DYN_BASE: AtomicUsize = AtomicUsize::new(0);
static EXEC_DYNAMIC_SYMBOLS: RwLock<Vec<(String, usize)>> = RwLock::new(Vec::new());
static EXEC_DYNAMIC_TLS_SYMBOLS: RwLock<Vec<(String, usize)>> = RwLock::new(Vec::new());
static EXEC_DYNAMIC_TLS_MODULES: RwLock<Vec<ExecDynamicTlsModule>> = RwLock::new(Vec::new());

/// 실행 ELF 로드 결과
pub struct ExecutableLoadInfo {
    pub entry: usize,
    pub load_bias: usize,
    pub dynamic: ExecutableDynamicInfo,
    pub tls: ExecutableTlsInfo,
    pub exported_symbols: Vec<(String, usize)>,
}

/// 실행 ELF의 .dynamic/DT_* 요약 정보
#[derive(Debug, Clone, Copy, Default)]
pub struct ExecutableDynamicInfo {
    pub at_dynamic: usize,
    pub needed_count: usize,
    pub strtab: usize,
    pub strsz: usize,
    pub symtab: usize,
    pub syment: usize,
    pub hash: usize,
    pub gnu_hash: usize,
    pub rela: usize,
    pub relasz: usize,
    pub relaent: usize,
    pub rel: usize,
    pub relsz: usize,
    pub relent: usize,
    pub jmprel: usize,
    pub pltrelsz: usize,
    pub pltrel: usize,
    pub pltgot: usize,
    pub init: usize,
    pub fini: usize,
    pub flags: usize,
    pub flags_1: usize,
}

/// 실행 ELF의 PT_TLS 요약 정보
#[derive(Debug, Clone, Default)]
pub struct ExecutableTlsInfo {
    pub has_tls: bool,
    pub mem_size: usize,
    pub align: usize,
    pub template: Vec<u8>,
    pub tprel_base: usize,
}

/// 현재 exec 체인(메인 + preload된 .so) TLS 모듈 요약
#[derive(Debug, Clone)]
pub struct ExecutableTlsModuleInfo {
    pub tprel_base: usize,
    pub mem_size: usize,
    pub align: usize,
    pub template: Vec<u8>,
}

#[derive(Debug, Clone)]
struct ExecDynamicTlsModule {
    tprel_base: usize,
    mem_size: usize,
    align: usize,
    template: Vec<u8>,
}

#[derive(Debug, Clone, Copy)]
struct ExecPageMapping {
    user_page: usize,
    frame: usize,
}

/// 모듈 로더
pub struct ModuleLoader;

impl ModuleLoader {
    #[cfg(target_arch = "aarch64")]
    const EXEC_VADDR_MIN: usize = 0x0010_0000;
    #[cfg(target_arch = "aarch64")]
    const EXEC_VADDR_MAX: usize = 0x0800_0000;

    #[cfg(target_arch = "riscv64")]
    const EXEC_VADDR_MIN: usize = 0x0001_0000;
    #[cfg(target_arch = "riscv64")]
    const EXEC_VADDR_MAX: usize = 0x2000_0000;
    #[cfg(not(any(target_arch = "aarch64", target_arch = "riscv64")))]
    const EXEC_VADDR_MIN: usize = 0x4000_0000;
    #[cfg(not(any(target_arch = "aarch64", target_arch = "riscv64")))]
    const EXEC_VADDR_MAX: usize = 0x8000_0000;

    #[cfg(target_arch = "aarch64")]
    const EXEC_DYN_BASE_START: usize = 0x0200_0000;
    #[cfg(target_arch = "riscv64")]
    const EXEC_DYN_BASE_START: usize = 0x0800_0000;
    #[cfg(not(any(target_arch = "aarch64", target_arch = "riscv64")))]
    const EXEC_DYN_BASE_START: usize = 0x4800_0000;
    const EXEC_DYN_GUARD_SIZE: usize = 2 * 1024 * 1024;

    #[cfg(target_arch = "aarch64")]
    const EXEC_TLS_TCB_SIZE: usize = 16;
    #[cfg(target_arch = "riscv64")]
    const EXEC_TLS_TCB_SIZE: usize = 0;
    #[cfg(not(any(target_arch = "aarch64", target_arch = "riscv64")))]
    const EXEC_TLS_TCB_SIZE: usize = 0;

    fn align_up(value: usize, align: usize) -> Result<usize, ModuleError> {
        let plus = value.checked_add(align - 1).ok_or(ModuleError::InvalidFormat)?;
        Ok(plus & !(align - 1))
    }

    fn compute_dyn_mapping_window(elf: &Elf64) -> Result<(usize, usize), ModuleError> {
        let mut min_addr = usize::MAX;
        let mut max_addr = 0usize;
        let mut found = false;

        for ph in elf.load_segments() {
            let seg_start = (ph.p_vaddr as usize) & !(PAGE_SIZE - 1);
            let raw_end = (ph.p_vaddr as usize)
                .checked_add(ph.p_memsz as usize)
                .ok_or(ModuleError::InvalidFormat)?;
            let seg_end = Self::align_up(raw_end, PAGE_SIZE)?;
            if seg_end < seg_start {
                return Err(ModuleError::InvalidFormat);
            }
            min_addr = core::cmp::min(min_addr, seg_start);
            max_addr = core::cmp::max(max_addr, seg_end);
            found = true;
        }

        if !found || max_addr <= min_addr {
            return Err(ModuleError::InvalidFormat);
        }

        Ok((min_addr, max_addr))
    }

    fn reserve_dyn_base(span: usize) -> Result<usize, ModuleError> {
        let span = Self::align_up(span, PAGE_SIZE)?;
        let step = span
            .checked_add(Self::EXEC_DYN_GUARD_SIZE)
            .ok_or(ModuleError::InvalidFormat)?;

        loop {
            let current = NEXT_EXEC_DYN_BASE.load(Ordering::Acquire);
            let base = if current == 0 {
                Self::EXEC_DYN_BASE_START
            } else {
                current
            };
            let next = base.checked_add(step).ok_or(ModuleError::InvalidFormat)?;
            if next > Self::EXEC_VADDR_MAX {
                return Err(ModuleError::InvalidFormat);
            }

            let previous = if current == 0 {
                NEXT_EXEC_DYN_BASE.compare_exchange(
                    0,
                    next,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
            } else {
                NEXT_EXEC_DYN_BASE.compare_exchange(
                    current,
                    next,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
            };

            if previous.is_ok() {
                return Ok(base);
            }
        }
    }

    pub fn clear_exec_dynamic_symbols() {
        {
            let mut symbols = EXEC_DYNAMIC_SYMBOLS.write();
            symbols.clear();
        }
        {
            let mut tls_symbols = EXEC_DYNAMIC_TLS_SYMBOLS.write();
            tls_symbols.clear();
        }
        {
            let mut tls_modules = EXEC_DYNAMIC_TLS_MODULES.write();
            tls_modules.clear();
        }
    }

    fn register_exec_dynamic_symbols(symbols: &[(String, usize)]) {
        if symbols.is_empty() {
            return;
        }

        let mut global = EXEC_DYNAMIC_SYMBOLS.write();
        for (name, addr) in symbols.iter() {
            if let Some(pos) = global.iter().position(|(n, _)| n == name) {
                global[pos] = (name.clone(), *addr);
            } else {
                global.push((name.clone(), *addr));
            }
        }
    }

    fn lookup_exec_dynamic_symbol(name: &str) -> Option<usize> {
        let global = EXEC_DYNAMIC_SYMBOLS.read();
        global
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .map(|(_, addr)| *addr)
    }

    pub fn exec_tls_modules() -> Vec<ExecutableTlsModuleInfo> {
        let modules = EXEC_DYNAMIC_TLS_MODULES.read();
        modules
            .iter()
            .map(|item| ExecutableTlsModuleInfo {
                tprel_base: item.tprel_base,
                mem_size: item.mem_size,
                align: item.align,
                template: item.template.clone(),
            })
            .collect()
    }

    fn register_exec_dynamic_tls_symbols(symbols: &[(String, usize)]) {
        if symbols.is_empty() {
            return;
        }

        let mut global = EXEC_DYNAMIC_TLS_SYMBOLS.write();
        for (name, offset) in symbols.iter() {
            if let Some(pos) = global.iter().position(|(n, _)| n == name) {
                global[pos] = (name.clone(), *offset);
            } else {
                global.push((name.clone(), *offset));
            }
        }
    }

    fn lookup_exec_dynamic_tls_symbol(name: &str) -> Option<usize> {
        let global = EXEC_DYNAMIC_TLS_SYMBOLS.read();
        global
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .map(|(_, offset)| *offset)
    }

    fn normalize_tls_align(align: usize) -> Result<usize, ModuleError> {
        let normalized = align.max(core::mem::size_of::<usize>());
        if !normalized.is_power_of_two() {
            return Err(ModuleError::InvalidFormat);
        }
        Ok(normalized)
    }

    fn next_tls_tprel_base(align: usize) -> Result<usize, ModuleError> {
        let mut cursor = Self::EXEC_TLS_TCB_SIZE;
        let modules = EXEC_DYNAMIC_TLS_MODULES.read();
        for module in modules.iter() {
            let module_align = Self::normalize_tls_align(module.align)?;
            cursor = Self::align_up(cursor, module_align)?;
            cursor = cursor
                .checked_add(module.mem_size)
                .ok_or(ModuleError::InvalidFormat)?;
        }
        Self::align_up(cursor, align)
    }

    fn assign_tls_tprel_base(mut tls: ExecutableTlsInfo) -> Result<ExecutableTlsInfo, ModuleError> {
        if !tls.has_tls || tls.mem_size == 0 {
            return Ok(tls);
        }
        let align = Self::normalize_tls_align(tls.align)?;
        tls.align = align;
        tls.tprel_base = Self::next_tls_tprel_base(align)?;
        Ok(tls)
    }

    fn register_exec_tls_module(tls: &ExecutableTlsInfo) {
        if !tls.has_tls || tls.mem_size == 0 {
            return;
        }
        EXEC_DYNAMIC_TLS_MODULES.write().push(ExecDynamicTlsModule {
            tprel_base: tls.tprel_base,
            mem_size: tls.mem_size,
            align: tls.align,
            template: tls.template.clone(),
        });
    }

    fn cleanup_exec_frames(frames: &mut Vec<usize>) {
        for frame in frames.drain(..) {
            unsafe {
                page::free_frame(frame);
            }
        }
    }

    fn dyn_ptr_with_bias(raw: usize, load_bias: usize) -> Result<usize, ModuleError> {
        raw.checked_add(load_bias).ok_or(ModuleError::InvalidFormat)
    }

    fn parse_dynamic_info(elf: &Elf64, load_bias: usize) -> Result<ExecutableDynamicInfo, ModuleError> {
        let mut info = ExecutableDynamicInfo::default();

        if let Some(dynamic_phdr) = elf.dynamic_segment() {
            info.at_dynamic = Self::dyn_ptr_with_bias(dynamic_phdr.p_vaddr as usize, load_bias)?;
        }

        let entries = elf
            .dynamic_entries()
            .map_err(|_| ModuleError::InvalidFormat)?;
        let Some(entries) = entries else {
            return Ok(info);
        };

        for entry in entries {
            let value = entry.value() as usize;
            match entry.tag() {
                DynamicTag::Null => break,
                DynamicTag::Needed => {
                    info.needed_count = info.needed_count.saturating_add(1);
                }
                DynamicTag::PltRelSz => {
                    info.pltrelsz = value;
                }
                DynamicTag::PltGot => {
                    info.pltgot = Self::dyn_ptr_with_bias(value, load_bias)?;
                }
                DynamicTag::Hash => {
                    info.hash = Self::dyn_ptr_with_bias(value, load_bias)?;
                }
                DynamicTag::StrTab => {
                    info.strtab = Self::dyn_ptr_with_bias(value, load_bias)?;
                }
                DynamicTag::SymTab => {
                    info.symtab = Self::dyn_ptr_with_bias(value, load_bias)?;
                }
                DynamicTag::Rela => {
                    info.rela = Self::dyn_ptr_with_bias(value, load_bias)?;
                }
                DynamicTag::RelaSz => {
                    info.relasz = value;
                }
                DynamicTag::RelaEnt => {
                    info.relaent = value;
                }
                DynamicTag::StrSz => {
                    info.strsz = value;
                }
                DynamicTag::SymEnt => {
                    info.syment = value;
                }
                DynamicTag::Init => {
                    info.init = Self::dyn_ptr_with_bias(value, load_bias)?;
                }
                DynamicTag::Fini => {
                    info.fini = Self::dyn_ptr_with_bias(value, load_bias)?;
                }
                DynamicTag::Rel => {
                    info.rel = Self::dyn_ptr_with_bias(value, load_bias)?;
                }
                DynamicTag::RelSz => {
                    info.relsz = value;
                }
                DynamicTag::RelEnt => {
                    info.relent = value;
                }
                DynamicTag::PltRel => {
                    info.pltrel = value;
                }
                DynamicTag::JmpRel => {
                    info.jmprel = Self::dyn_ptr_with_bias(value, load_bias)?;
                }
                DynamicTag::Flags => {
                    info.flags = value;
                }
                DynamicTag::Flags1 => {
                    info.flags_1 = value;
                }
                DynamicTag::GnuHash => {
                    info.gnu_hash = Self::dyn_ptr_with_bias(value, load_bias)?;
                }
                DynamicTag::Other(_)
                | DynamicTag::SoName
                | DynamicTag::RPath
                | DynamicTag::Symbolic
                | DynamicTag::Debug
                | DynamicTag::TextRel
                | DynamicTag::BindNow
                | DynamicTag::RunPath => {}
            }
        }

        Ok(info)
    }

    fn remove_load_bias(addr: usize, load_bias: usize) -> Result<usize, ModuleError> {
        addr.checked_sub(load_bias).ok_or(ModuleError::InvalidFormat)
    }

    fn add_signed(base: usize, addend: i64) -> Result<usize, ModuleError> {
        let sum = (base as i128)
            .checked_add(addend as i128)
            .ok_or(ModuleError::InvalidFormat)?;
        if sum < 0 || sum > usize::MAX as i128 {
            return Err(ModuleError::InvalidFormat);
        }
        Ok(sum as usize)
    }

    fn file_slice_for_vaddr<'a>(
        elf: &Elf64,
        data: &'a [u8],
        vaddr: usize,
        len: usize,
    ) -> Result<&'a [u8], ModuleError> {
        if len == 0 {
            return Ok(&[]);
        }

        let end = vaddr.checked_add(len).ok_or(ModuleError::InvalidFormat)?;
        for ph in elf.load_segments() {
            let seg_vaddr = ph.p_vaddr as usize;
            let seg_file_size = ph.p_filesz as usize;
            let seg_file_end = seg_vaddr
                .checked_add(seg_file_size)
                .ok_or(ModuleError::InvalidFormat)?;
            if vaddr < seg_vaddr || end > seg_file_end {
                continue;
            }

            let delta = vaddr - seg_vaddr;
            let file_off = (ph.p_offset as usize)
                .checked_add(delta)
                .ok_or(ModuleError::InvalidFormat)?;
            let file_end = file_off
                .checked_add(len)
                .ok_or(ModuleError::InvalidFormat)?;
            if file_end > data.len() {
                return Err(ModuleError::InvalidFormat);
            }
            return Ok(&data[file_off..file_end]);
        }

        Err(ModuleError::InvalidFormat)
    }

    fn dynamic_strtab<'a>(
        elf: &Elf64,
        data: &'a [u8],
        load_bias: usize,
        dyn_info: &ExecutableDynamicInfo,
    ) -> Result<Option<&'a [u8]>, ModuleError> {
        if dyn_info.strtab == 0 || dyn_info.strsz == 0 {
            return Ok(None);
        }
        let strtab_vaddr = Self::remove_load_bias(dyn_info.strtab, load_bias)?;
        let strtab = Self::file_slice_for_vaddr(elf, data, strtab_vaddr, dyn_info.strsz)?;
        Ok(Some(strtab))
    }

    fn string_at(strtab: &[u8], offset: usize) -> &str {
        if offset >= strtab.len() {
            return "";
        }
        let end = strtab[offset..]
            .iter()
            .position(|b| *b == 0)
            .map(|i| offset + i)
            .unwrap_or(strtab.len());
        core::str::from_utf8(&strtab[offset..end]).unwrap_or("")
    }

    fn dynamic_symbol_at(
        elf: &Elf64,
        data: &[u8],
        load_bias: usize,
        dyn_info: &ExecutableDynamicInfo,
        sym_index: u32,
    ) -> Result<Elf64Symbol, ModuleError> {
        if dyn_info.symtab == 0 {
            return Err(ModuleError::InvalidFormat);
        }
        let sym_ent = if dyn_info.syment == 0 {
            size_of::<Elf64Symbol>()
        } else {
            dyn_info.syment
        };
        if sym_ent < size_of::<Elf64Symbol>() {
            return Err(ModuleError::InvalidFormat);
        }

        let symtab_vaddr = Self::remove_load_bias(dyn_info.symtab, load_bias)?;
        let sym_off = (sym_index as usize)
            .checked_mul(sym_ent)
            .ok_or(ModuleError::InvalidFormat)?;
        let symbol_vaddr = symtab_vaddr
            .checked_add(sym_off)
            .ok_or(ModuleError::InvalidFormat)?;

        let bytes = Self::file_slice_for_vaddr(elf, data, symbol_vaddr, size_of::<Elf64Symbol>())?;
        let symbol = unsafe {
            // SAFETY: ELF file 범위와 엔트리 크기를 검증한 뒤 unaligned read를 수행한다.
            (bytes.as_ptr() as *const Elf64Symbol).read_unaligned()
        };
        Ok(symbol)
    }

    fn dynamic_symbol_count(
        elf: &Elf64,
        data: &[u8],
        load_bias: usize,
        dyn_info: &ExecutableDynamicInfo,
    ) -> Result<usize, ModuleError> {
        if dyn_info.hash != 0 {
            let hash_vaddr = Self::remove_load_bias(dyn_info.hash, load_bias)?;
            let bytes = Self::file_slice_for_vaddr(elf, data, hash_vaddr, 8)?;
            let nchain = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
            if nchain > 0 {
                return Ok(nchain);
            }
        }

        if dyn_info.symtab != 0 && dyn_info.strtab != 0 && dyn_info.strtab > dyn_info.symtab {
            let sym_ent = if dyn_info.syment == 0 {
                size_of::<Elf64Symbol>()
            } else {
                dyn_info.syment
            };
            if sym_ent >= size_of::<Elf64Symbol>() {
                let span = dyn_info
                    .strtab
                    .checked_sub(dyn_info.symtab)
                    .ok_or(ModuleError::InvalidFormat)?;
                return Ok(span / sym_ent);
            }
        }

        Ok(0)
    }

    fn collect_dynamic_exports(
        elf: &Elf64,
        data: &[u8],
        load_bias: usize,
        dyn_info: &ExecutableDynamicInfo,
    ) -> Result<Vec<(String, usize)>, ModuleError> {
        let strtab = Self::dynamic_strtab(elf, data, load_bias, dyn_info)?;
        let Some(strtab) = strtab else {
            return Ok(Vec::new());
        };

        let symbol_count = Self::dynamic_symbol_count(elf, data, load_bias, dyn_info)?;
        if symbol_count == 0 {
            return Ok(Vec::new());
        }

        let mut exports = Vec::new();
        for sym_index in 1..(symbol_count as u32) {
            let sym = match Self::dynamic_symbol_at(elf, data, load_bias, dyn_info, sym_index) {
                Ok(sym) => sym,
                Err(_) => break,
            };

            let binding = sym.binding();
            if binding != 1 && binding != 2 {
                continue;
            }

            if sym.st_shndx == section_index::SHN_UNDEF {
                continue;
            }

            let name = Self::string_at(strtab, sym.st_name as usize);
            if name.is_empty() {
                continue;
            }

            let value = if sym.st_shndx == section_index::SHN_ABS {
                sym.st_value as usize
            } else {
                (sym.st_value as usize)
                    .checked_add(load_bias)
                    .ok_or(ModuleError::InvalidFormat)?
            };

            exports.push((String::from(name), value));
        }

        Ok(exports)
    }

    fn collect_dynamic_tls_exports(
        elf: &Elf64,
        data: &[u8],
        load_bias: usize,
        dyn_info: &ExecutableDynamicInfo,
        tls_info: &ExecutableTlsInfo,
    ) -> Result<Vec<(String, usize)>, ModuleError> {
        if !tls_info.has_tls || tls_info.mem_size == 0 {
            return Ok(Vec::new());
        }

        let strtab = Self::dynamic_strtab(elf, data, load_bias, dyn_info)?;
        let Some(strtab) = strtab else {
            return Ok(Vec::new());
        };

        let symbol_count = Self::dynamic_symbol_count(elf, data, load_bias, dyn_info)?;
        if symbol_count == 0 {
            return Ok(Vec::new());
        }

        let mut exports = Vec::new();
        for sym_index in 1..(symbol_count as u32) {
            let sym = match Self::dynamic_symbol_at(elf, data, load_bias, dyn_info, sym_index) {
                Ok(sym) => sym,
                Err(_) => break,
            };

            let binding = sym.binding();
            if binding != 1 && binding != 2 {
                continue;
            }
            if sym.sym_type() != symbol_type::STT_TLS {
                continue;
            }
            if sym.st_shndx == section_index::SHN_UNDEF {
                continue;
            }

            let name = Self::string_at(strtab, sym.st_name as usize);
            if name.is_empty() {
                continue;
            }

            let symbol_offset = sym.st_value as usize;
            if symbol_offset > tls_info.mem_size {
                return Err(ModuleError::InvalidFormat);
            }
            let tprel = tls_info
                .tprel_base
                .checked_add(symbol_offset)
                .ok_or(ModuleError::InvalidFormat)?;
            exports.push((String::from(name), tprel));
        }

        Ok(exports)
    }

    fn resolve_dynamic_symbol(
        elf: &Elf64,
        data: &[u8],
        load_bias: usize,
        dyn_info: &ExecutableDynamicInfo,
        sym_index: u32,
        strtab: Option<&[u8]>,
    ) -> Result<Option<usize>, ModuleError> {
        if sym_index == 0 {
            return Ok(Some(0));
        }

        let symbol = Self::dynamic_symbol_at(elf, data, load_bias, dyn_info, sym_index)?;
        if symbol.st_shndx == section_index::SHN_UNDEF {
            let name = strtab
                .map(|tab| Self::string_at(tab, symbol.st_name as usize))
                .unwrap_or("");
            let is_weak = symbol.binding() == 2;
            if !name.is_empty() {
                if let Some(addr) = lookup_symbol(name) {
                    return Ok(Some(addr));
                }
                if let Some(addr) = Self::lookup_exec_dynamic_symbol(name) {
                    return Ok(Some(addr));
                }
                if is_weak {
                    kprintln!("[module] dynamic weak unresolved symbol: {}", name);
                    return Ok(Some(0));
                }
                kprintln!("[module] dynamic unresolved symbol: {}", name);
            } else {
                if is_weak {
                    return Ok(Some(0));
                }
                kprintln!("[module] dynamic unresolved symbol index={}", sym_index);
            }
            return Ok(None);
        }

        if symbol.st_shndx == section_index::SHN_ABS {
            return Ok(Some(symbol.st_value as usize));
        }

        let value = (symbol.st_value as usize)
            .checked_add(load_bias)
            .ok_or(ModuleError::InvalidFormat)?;
        Ok(Some(value))
    }

    fn resolve_dynamic_tls_tprel(
        elf: &Elf64,
        data: &[u8],
        load_bias: usize,
        dyn_info: &ExecutableDynamicInfo,
        sym_index: u32,
        strtab: Option<&[u8]>,
        current_tls: &ExecutableTlsInfo,
    ) -> Result<Option<usize>, ModuleError> {
        if sym_index == 0 {
            return Err(ModuleError::InvalidFormat);
        }

        let symbol = Self::dynamic_symbol_at(elf, data, load_bias, dyn_info, sym_index)?;
        let is_weak = symbol.binding() == 2;
        if symbol.sym_type() != symbol_type::STT_TLS {
            return Err(ModuleError::InvalidFormat);
        }

        if symbol.st_shndx == section_index::SHN_UNDEF {
            let name = strtab
                .map(|tab| Self::string_at(tab, symbol.st_name as usize))
                .unwrap_or("");
            if !name.is_empty() {
                if let Some(offset) = Self::lookup_exec_dynamic_tls_symbol(name) {
                    return Ok(Some(offset));
                }
                if is_weak {
                    kprintln!("[module] dynamic weak unresolved TLS symbol: {}", name);
                    return Ok(Some(0));
                }
                kprintln!("[module] dynamic unresolved TLS symbol: {}", name);
            } else if is_weak {
                return Ok(Some(0));
            } else {
                kprintln!("[module] dynamic unresolved TLS symbol index={}", sym_index);
            }
            return Ok(None);
        }

        if !current_tls.has_tls || current_tls.mem_size == 0 {
            return Err(ModuleError::InvalidFormat);
        }
        let symbol_offset = symbol.st_value as usize;
        if symbol_offset > current_tls.mem_size {
            return Err(ModuleError::InvalidFormat);
        }
        let tprel = current_tls
            .tprel_base
            .checked_add(symbol_offset)
            .ok_or(ModuleError::InvalidFormat)?;
        Ok(Some(tprel))
    }

    fn translate_exec_vaddr(page_mappings: &[ExecPageMapping], vaddr: usize) -> Option<usize> {
        let page = vaddr & !(PAGE_SIZE - 1);
        let offset = vaddr & (PAGE_SIZE - 1);
        page_mappings
            .iter()
            .rev()
            .find(|m| m.user_page == page)
            .map(|m| m.frame + offset)
    }

    fn read_exec_u64(page_mappings: &[ExecPageMapping], vaddr: usize) -> Result<u64, ModuleError> {
        let mut bytes = [0u8; 8];
        for (i, b) in bytes.iter_mut().enumerate() {
            let addr = vaddr.checked_add(i).ok_or(ModuleError::InvalidFormat)?;
            let phys = Self::translate_exec_vaddr(page_mappings, addr).ok_or(ModuleError::InvalidFormat)?;
            unsafe {
                // SAFETY: 변환된 물리 프레임 주소는 alloc_frame으로 확보된 유효 매핑 내 바이트다.
                *b = *(phys as *const u8);
            }
        }
        Ok(u64::from_ne_bytes(bytes))
    }

    fn read_exec_bytes(
        page_mappings: &[ExecPageMapping],
        vaddr: usize,
        len: usize,
    ) -> Result<Vec<u8>, ModuleError> {
        if len == 0 {
            return Ok(Vec::new());
        }

        let mut out = Vec::new();
        out.try_reserve_exact(len)
            .map_err(|_| ModuleError::OutOfMemory)?;
        out.resize(len, 0);

        for (i, b) in out.iter_mut().enumerate() {
            let addr = vaddr.checked_add(i).ok_or(ModuleError::InvalidFormat)?;
            let phys =
                Self::translate_exec_vaddr(page_mappings, addr).ok_or(ModuleError::InvalidFormat)?;
            unsafe {
                // SAFETY: 변환된 물리 프레임 주소는 alloc_frame으로 확보된 유효 매핑 내 바이트다.
                *b = *(phys as *const u8);
            }
        }

        Ok(out)
    }

    fn collect_tls_info(
        elf: &Elf64,
        load_bias: usize,
        page_mappings: &[ExecPageMapping],
    ) -> Result<ExecutableTlsInfo, ModuleError> {
        let Some(tls) = elf.tls_segment() else {
            return Ok(ExecutableTlsInfo::default());
        };

        let mem_size = tls.p_memsz as usize;
        if mem_size == 0 {
            return Ok(ExecutableTlsInfo::default());
        }

        let file_size = tls.p_filesz as usize;
        if file_size > mem_size {
            return Err(ModuleError::InvalidFormat);
        }

        let align = if tls.p_align == 0 {
            1
        } else {
            tls.p_align as usize
        };
        if !align.is_power_of_two() {
            return Err(ModuleError::InvalidFormat);
        }

        let tls_vaddr = (tls.p_vaddr as usize)
            .checked_add(load_bias)
            .ok_or(ModuleError::InvalidFormat)?;
        let template = Self::read_exec_bytes(page_mappings, tls_vaddr, file_size)?;

        Ok(ExecutableTlsInfo {
            has_tls: true,
            mem_size,
            align,
            template,
            tprel_base: 0,
        })
    }

    fn write_exec_u64(
        page_mappings: &[ExecPageMapping],
        vaddr: usize,
        value: u64,
    ) -> Result<(), ModuleError> {
        let bytes = value.to_ne_bytes();
        for (i, b) in bytes.iter().enumerate() {
            let addr = vaddr.checked_add(i).ok_or(ModuleError::InvalidFormat)?;
            let phys = Self::translate_exec_vaddr(page_mappings, addr).ok_or(ModuleError::InvalidFormat)?;
            unsafe {
                // SAFETY: 변환된 물리 프레임 주소는 alloc_frame으로 확보된 유효 매핑 내 바이트다.
                *(phys as *mut u8) = *b;
            }
        }
        Ok(())
    }

    fn parse_rela_entries(
        elf: &Elf64,
        data: &[u8],
        load_bias: usize,
        rela_addr: usize,
        rela_size: usize,
        rela_ent: usize,
    ) -> Result<Vec<Elf64Rela>, ModuleError> {
        if rela_addr == 0 || rela_size == 0 {
            return Ok(Vec::new());
        }
        let entry_size = if rela_ent == 0 {
            size_of::<Elf64Rela>()
        } else {
            rela_ent
        };
        if entry_size < size_of::<Elf64Rela>() || rela_size % entry_size != 0 {
            return Err(ModuleError::InvalidFormat);
        }

        let rela_vaddr = Self::remove_load_bias(rela_addr, load_bias)?;
        let bytes = Self::file_slice_for_vaddr(elf, data, rela_vaddr, rela_size)?;
        let mut entries = Vec::new();
        for idx in 0..(rela_size / entry_size) {
            let start = idx * entry_size;
            let end = start
                .checked_add(size_of::<Elf64Rela>())
                .ok_or(ModuleError::InvalidFormat)?;
            if end > bytes.len() {
                return Err(ModuleError::InvalidFormat);
            }
            let entry = unsafe {
                // SAFETY: 엔트리별 경계를 검증했고 unaligned read를 수행한다.
                (bytes.as_ptr().add(start) as *const Elf64Rela).read_unaligned()
            };
            entries.push(entry);
        }
        Ok(entries)
    }

    fn parse_rel_entries(
        elf: &Elf64,
        data: &[u8],
        load_bias: usize,
        rel_addr: usize,
        rel_size: usize,
        rel_ent: usize,
    ) -> Result<Vec<Elf64Rel>, ModuleError> {
        if rel_addr == 0 || rel_size == 0 {
            return Ok(Vec::new());
        }
        let entry_size = if rel_ent == 0 {
            size_of::<Elf64Rel>()
        } else {
            rel_ent
        };
        if entry_size < size_of::<Elf64Rel>() || rel_size % entry_size != 0 {
            return Err(ModuleError::InvalidFormat);
        }

        let rel_vaddr = Self::remove_load_bias(rel_addr, load_bias)?;
        let bytes = Self::file_slice_for_vaddr(elf, data, rel_vaddr, rel_size)?;
        let mut entries = Vec::new();
        for idx in 0..(rel_size / entry_size) {
            let start = idx * entry_size;
            let end = start
                .checked_add(size_of::<Elf64Rel>())
                .ok_or(ModuleError::InvalidFormat)?;
            if end > bytes.len() {
                return Err(ModuleError::InvalidFormat);
            }
            let entry = unsafe {
                // SAFETY: 엔트리별 경계를 검증했고 unaligned read를 수행한다.
                (bytes.as_ptr().add(start) as *const Elf64Rel).read_unaligned()
            };
            entries.push(entry);
        }
        Ok(entries)
    }

    fn apply_dynamic_relocations(
        elf: &Elf64,
        data: &[u8],
        load_bias: usize,
        dyn_info: &ExecutableDynamicInfo,
        current_tls: &ExecutableTlsInfo,
        page_mappings: &[ExecPageMapping],
    ) -> Result<(), ModuleError> {
        if dyn_info.rela == 0
            && dyn_info.rel == 0
            && dyn_info.jmprel == 0
            && dyn_info.pltrelsz == 0
        {
            return Ok(());
        }

        let strtab = Self::dynamic_strtab(elf, data, load_bias, dyn_info)?;
        let mut applied = 0usize;

        if dyn_info.rela != 0 && dyn_info.relasz != 0 {
            let relas = Self::parse_rela_entries(
                elf,
                data,
                load_bias,
                dyn_info.rela,
                dyn_info.relasz,
                dyn_info.relaent,
            )?;
            for rela in relas.iter() {
                if Self::apply_rela_entry(
                    elf,
                    data,
                    load_bias,
                    dyn_info,
                    current_tls,
                    strtab,
                    page_mappings,
                    rela,
                )? {
                    applied += 1;
                }
            }
        }

        if dyn_info.rel != 0 && dyn_info.relsz != 0 {
            let rels = Self::parse_rel_entries(
                elf,
                data,
                load_bias,
                dyn_info.rel,
                dyn_info.relsz,
                dyn_info.relent,
            )?;
            for rel in rels.iter() {
                if Self::apply_rel_entry(
                    elf,
                    data,
                    load_bias,
                    dyn_info,
                    current_tls,
                    strtab,
                    page_mappings,
                    rel,
                )? {
                    applied += 1;
                }
            }
        }

        if dyn_info.jmprel != 0 && dyn_info.pltrelsz != 0 {
            if dyn_info.pltrel == dynamic_tag::DT_RELA as usize {
                let relas = Self::parse_rela_entries(
                    elf,
                    data,
                    load_bias,
                    dyn_info.jmprel,
                    dyn_info.pltrelsz,
                    dyn_info.relaent,
                )?;
                for rela in relas.iter() {
                    if Self::apply_rela_entry(
                        elf,
                        data,
                        load_bias,
                        dyn_info,
                        current_tls,
                        strtab,
                        page_mappings,
                        rela,
                    )? {
                        applied += 1;
                    }
                }
            } else if dyn_info.pltrel == dynamic_tag::DT_REL as usize {
                let rels = Self::parse_rel_entries(
                    elf,
                    data,
                    load_bias,
                    dyn_info.jmprel,
                    dyn_info.pltrelsz,
                    dyn_info.relent,
                )?;
                for rel in rels.iter() {
                    if Self::apply_rel_entry(
                        elf,
                        data,
                        load_bias,
                        dyn_info,
                        current_tls,
                        strtab,
                        page_mappings,
                        rel,
                    )? {
                        applied += 1;
                    }
                }
            } else {
                return Err(ModuleError::InvalidFormat);
            }
        }

        if applied > 0 {
            kprintln!("[module] DYNAMIC relocation: applied={}", applied);
        }
        Ok(())
    }

    fn apply_rela_entry(
        elf: &Elf64,
        data: &[u8],
        load_bias: usize,
        dyn_info: &ExecutableDynamicInfo,
        current_tls: &ExecutableTlsInfo,
        strtab: Option<&[u8]>,
        page_mappings: &[ExecPageMapping],
        rela: &Elf64Rela,
    ) -> Result<bool, ModuleError> {
        let rel_type = rela.rel_type();
        let sym = rela.symbol();
        let place = (rela.r_offset as usize)
            .checked_add(load_bias)
            .ok_or(ModuleError::InvalidFormat)?;
        let addend = rela.r_addend;

        #[cfg(target_arch = "aarch64")]
        {
            match rel_type {
                reloc_aarch64::R_AARCH64_NONE => return Ok(false),
                reloc_aarch64::R_AARCH64_RELATIVE => {
                    if sym != 0 {
                        return Err(ModuleError::InvalidFormat);
                    }
                    let value = Self::add_signed(load_bias, addend)?;
                    Self::write_exec_u64(page_mappings, place, value as u64)?;
                    return Ok(true);
                }
                reloc_aarch64::R_AARCH64_ABS64
                | reloc_aarch64::R_AARCH64_GLOB_DAT
                | reloc_aarch64::R_AARCH64_JUMP_SLOT => {
                    let symbol = Self::resolve_dynamic_symbol(
                        elf,
                        data,
                        load_bias,
                        dyn_info,
                        sym,
                        strtab,
                    )?;
                    let Some(symbol_value) = symbol else {
                        return Err(ModuleError::SymbolNotFound);
                    };
                    let value = Self::add_signed(symbol_value, addend)?;
                    Self::write_exec_u64(page_mappings, place, value as u64)?;
                    return Ok(true);
                }
                reloc_aarch64::R_AARCH64_TLS_TPREL64 => {
                    let symbol = Self::resolve_dynamic_tls_tprel(
                        elf,
                        data,
                        load_bias,
                        dyn_info,
                        sym,
                        strtab,
                        current_tls,
                    )?;
                    let Some(tprel) = symbol else {
                        return Err(ModuleError::SymbolNotFound);
                    };
                    let value = Self::add_signed(tprel, addend)?;
                    Self::write_exec_u64(page_mappings, place, value as u64)?;
                    return Ok(true);
                }
                reloc_aarch64::R_AARCH64_TLS_DTPMOD64
                | reloc_aarch64::R_AARCH64_TLS_DTPREL64
                | reloc_aarch64::R_AARCH64_TLSDESC => {
                    return Err(ModuleError::UnsupportedRelocation(rel_type));
                }
                _ => return Err(ModuleError::UnsupportedRelocation(rel_type)),
            }
        }

        #[cfg(target_arch = "riscv64")]
        {
            match rel_type {
                reloc_riscv::R_RISCV_NONE | reloc_riscv::R_RISCV_RELAX => return Ok(false),
                reloc_riscv::R_RISCV_RELATIVE => {
                    if sym != 0 {
                        return Err(ModuleError::InvalidFormat);
                    }
                    let value = Self::add_signed(load_bias, addend)?;
                    Self::write_exec_u64(page_mappings, place, value as u64)?;
                    return Ok(true);
                }
                reloc_riscv::R_RISCV_64
                | reloc_riscv::R_RISCV_GLOB_DAT
                | reloc_riscv::R_RISCV_JUMP_SLOT => {
                    let symbol = Self::resolve_dynamic_symbol(
                        elf,
                        data,
                        load_bias,
                        dyn_info,
                        sym,
                        strtab,
                    )?;
                    let Some(symbol_value) = symbol else {
                        return Err(ModuleError::SymbolNotFound);
                    };
                    let value = Self::add_signed(symbol_value, addend)?;
                    Self::write_exec_u64(page_mappings, place, value as u64)?;
                    return Ok(true);
                }
                reloc_riscv::R_RISCV_TLS_TPREL64 => {
                    let symbol = Self::resolve_dynamic_tls_tprel(
                        elf,
                        data,
                        load_bias,
                        dyn_info,
                        sym,
                        strtab,
                        current_tls,
                    )?;
                    let Some(tprel) = symbol else {
                        return Err(ModuleError::SymbolNotFound);
                    };
                    let value = Self::add_signed(tprel, addend)?;
                    Self::write_exec_u64(page_mappings, place, value as u64)?;
                    return Ok(true);
                }
                reloc_riscv::R_RISCV_TLS_DTPMOD64
                | reloc_riscv::R_RISCV_TLS_DTPREL32
                | reloc_riscv::R_RISCV_TLS_DTPREL64
                | reloc_riscv::R_RISCV_TLS_TPREL32
                | reloc_riscv::R_RISCV_TLSDESC => {
                    return Err(ModuleError::UnsupportedRelocation(rel_type));
                }
                _ => return Err(ModuleError::UnsupportedRelocation(rel_type)),
            }
        }

        #[allow(unreachable_code)]
        Ok(false)
    }

    fn apply_rel_entry(
        elf: &Elf64,
        data: &[u8],
        load_bias: usize,
        dyn_info: &ExecutableDynamicInfo,
        current_tls: &ExecutableTlsInfo,
        strtab: Option<&[u8]>,
        page_mappings: &[ExecPageMapping],
        rel: &Elf64Rel,
    ) -> Result<bool, ModuleError> {
        let rel_type = rel.rel_type();
        let sym = rel.symbol();
        let place = (rel.r_offset as usize)
            .checked_add(load_bias)
            .ok_or(ModuleError::InvalidFormat)?;
        let addend = Self::read_exec_u64(page_mappings, place)? as i64;

        #[cfg(target_arch = "aarch64")]
        {
            match rel_type {
                reloc_aarch64::R_AARCH64_NONE => return Ok(false),
                reloc_aarch64::R_AARCH64_RELATIVE => {
                    if sym != 0 {
                        return Err(ModuleError::InvalidFormat);
                    }
                    let value = Self::add_signed(load_bias, addend)?;
                    Self::write_exec_u64(page_mappings, place, value as u64)?;
                    return Ok(true);
                }
                reloc_aarch64::R_AARCH64_ABS64
                | reloc_aarch64::R_AARCH64_GLOB_DAT
                | reloc_aarch64::R_AARCH64_JUMP_SLOT => {
                    let symbol = Self::resolve_dynamic_symbol(
                        elf,
                        data,
                        load_bias,
                        dyn_info,
                        sym,
                        strtab,
                    )?;
                    let Some(symbol_value) = symbol else {
                        return Err(ModuleError::SymbolNotFound);
                    };
                    let value = Self::add_signed(symbol_value, addend)?;
                    Self::write_exec_u64(page_mappings, place, value as u64)?;
                    return Ok(true);
                }
                reloc_aarch64::R_AARCH64_TLS_TPREL64 => {
                    let symbol = Self::resolve_dynamic_tls_tprel(
                        elf,
                        data,
                        load_bias,
                        dyn_info,
                        sym,
                        strtab,
                        current_tls,
                    )?;
                    let Some(tprel) = symbol else {
                        return Err(ModuleError::SymbolNotFound);
                    };
                    let value = Self::add_signed(tprel, addend)?;
                    Self::write_exec_u64(page_mappings, place, value as u64)?;
                    return Ok(true);
                }
                reloc_aarch64::R_AARCH64_TLS_DTPMOD64
                | reloc_aarch64::R_AARCH64_TLS_DTPREL64
                | reloc_aarch64::R_AARCH64_TLSDESC => {
                    return Err(ModuleError::UnsupportedRelocation(rel_type));
                }
                _ => return Err(ModuleError::UnsupportedRelocation(rel_type)),
            }
        }

        #[cfg(target_arch = "riscv64")]
        {
            match rel_type {
                reloc_riscv::R_RISCV_NONE | reloc_riscv::R_RISCV_RELAX => return Ok(false),
                reloc_riscv::R_RISCV_RELATIVE => {
                    if sym != 0 {
                        return Err(ModuleError::InvalidFormat);
                    }
                    let value = Self::add_signed(load_bias, addend)?;
                    Self::write_exec_u64(page_mappings, place, value as u64)?;
                    return Ok(true);
                }
                reloc_riscv::R_RISCV_64
                | reloc_riscv::R_RISCV_GLOB_DAT
                | reloc_riscv::R_RISCV_JUMP_SLOT => {
                    let symbol = Self::resolve_dynamic_symbol(
                        elf,
                        data,
                        load_bias,
                        dyn_info,
                        sym,
                        strtab,
                    )?;
                    let Some(symbol_value) = symbol else {
                        return Err(ModuleError::SymbolNotFound);
                    };
                    let value = Self::add_signed(symbol_value, addend)?;
                    Self::write_exec_u64(page_mappings, place, value as u64)?;
                    return Ok(true);
                }
                reloc_riscv::R_RISCV_TLS_TPREL64 => {
                    let symbol = Self::resolve_dynamic_tls_tprel(
                        elf,
                        data,
                        load_bias,
                        dyn_info,
                        sym,
                        strtab,
                        current_tls,
                    )?;
                    let Some(tprel) = symbol else {
                        return Err(ModuleError::SymbolNotFound);
                    };
                    let value = Self::add_signed(tprel, addend)?;
                    Self::write_exec_u64(page_mappings, place, value as u64)?;
                    return Ok(true);
                }
                reloc_riscv::R_RISCV_TLS_DTPMOD64
                | reloc_riscv::R_RISCV_TLS_DTPREL32
                | reloc_riscv::R_RISCV_TLS_DTPREL64
                | reloc_riscv::R_RISCV_TLS_TPREL32
                | reloc_riscv::R_RISCV_TLSDESC => {
                    return Err(ModuleError::UnsupportedRelocation(rel_type));
                }
                _ => return Err(ModuleError::UnsupportedRelocation(rel_type)),
            }
        }

        #[allow(unreachable_code)]
        Ok(false)
    }

    /// Relocatable object (.o) 로드
    pub fn load_object(data: &[u8], name: &str) -> Result<&'static LoadedModule, ModuleError> {
        kprintln!("[module] Loading relocatable object: {}", name);

        // ELF 파싱
        let elf = Elf64::parse(data)?;

        // REL 타입 확인
        if elf.file_type() != ElfType::Rel {
            kprintln!(
                "[module] Error: Not a relocatable object (type={:?})",
                elf.file_type()
            );
            return Err(ModuleError::InvalidFormat);
        }

        // 필요한 메모리 크기 계산
        let mem_size = elf.section_memory_size();
        let num_pages = (mem_size + PAGE_SIZE - 1) / PAGE_SIZE;

        // PLT 페이지 할당 (최대 256개 엔트리, 16바이트씩 = 4KB = 1페이지)
        let plt_page_count = 1;
        let total_pages = num_pages + plt_page_count;

        kprintln!(
            "[module] Memory required: {} bytes ({} pages + {} PLT page)",
            mem_size,
            num_pages,
            plt_page_count
        );

        // 페이지 할당
        let mut pages = Vec::new();
        let mut base_addr = 0usize;

        for i in 0..total_pages {
            match page::alloc_frame() {
                Some(addr) => {
                    if i == 0 {
                        base_addr = addr;
                    }
                    pages.push(addr);
                }
                None => {
                    // 할당 실패 - 이미 할당한 페이지 해제
                    for &page in &pages {
                        unsafe {
                            page::free_frame(page);
                        }
                    }
                    return Err(ModuleError::OutOfMemory);
                }
            }
        }

        kprintln!("[module] Allocated {} pages at 0x{:x}", total_pages, base_addr);

        // PLT 페이지 주소 (마지막으로 할당된 페이지)
        let plt_base = *pages.last().unwrap();
        kprintln!("[module] PLT page at 0x{:x}", plt_base);

        // 메모리 영역을 0으로 초기화
        unsafe {
            core::ptr::write_bytes(base_addr as *mut u8, 0, total_pages * PAGE_SIZE);
        }

        // 섹션 로드 및 주소 매핑
        let section_addrs = Self::load_sections(&elf, base_addr)?;

        // PLT 테이블 생성
        let mut plt = Some(PltTable::new(plt_base));

        // 재배치 적용
        Self::apply_relocations(&elf, &section_addrs, &mut plt)?;

        // PLT 사용 로깅
        if let Some(ref plt_table) = plt {
            kprintln!("[module] PLT entries created: {}", plt_table.count);
        }

        // 캐시 플러시 (명령어 캐시)
        Self::flush_icache(base_addr, mem_size);
        // PLT 영역도 플러시
        Self::flush_icache(plt_base, PAGE_SIZE);

        // init/exit 함수 찾기
        let init_fn = elf
            .find_symbol("module_init")
            .map(|sym| section_addrs[sym.st_shndx as usize] + sym.st_value as usize);
        let exit_fn = elf
            .find_symbol("module_exit")
            .map(|sym| section_addrs[sym.st_shndx as usize] + sym.st_value as usize);

        if init_fn.is_some() {
            kprintln!("[module] Found module_init at 0x{:x}", init_fn.unwrap());
        }
        if exit_fn.is_some() {
            kprintln!("[module] Found module_exit at 0x{:x}", exit_fn.unwrap());
        }

        // GLOBAL 심볼들을 export 목록에 추가
        let mut exported_symbols = Vec::new();
        if let Some((_, symbols)) = elf.symbol_table() {
            for sym in symbols {
                // GLOBAL 바인딩이고 정의된 심볼만 export
                if sym.binding() == 1 && sym.st_shndx != section_index::SHN_UNDEF {
                    let sym_name = elf.symbol_name(sym);
                    if !sym_name.is_empty() {
                        let sym_addr = if sym.st_shndx == section_index::SHN_ABS {
                            sym.st_value as usize
                        } else {
                            let idx = sym.st_shndx as usize;
                            if idx < section_addrs.len() && section_addrs[idx] != 0 {
                                section_addrs[idx] + sym.st_value as usize
                            } else {
                                continue;
                            }
                        };
                        exported_symbols.push((String::from(sym_name), sym_addr));
                    }
                }
            }
        }
        kprintln!("[module] Exported {} symbols", exported_symbols.len());

        // LoadedModule 생성
        let module = Box::new(LoadedModule {
            info: Module::new(name),
            base_addr,
            size: mem_size,
            state: ModuleState::Live,
            ref_count: AtomicUsize::new(0),
            unloading: AtomicBool::new(false),
            init_fn,
            exit_fn,
            pages,
            section_addrs,
            exported_symbols,
            plt_page: Some(plt_base),
        });

        // init 함수 호출
        if let Err(e) = module.init() {
            // 실패 시 정리
            for &page in &module.pages {
                unsafe {
                    page::free_frame(page);
                }
            }
            return Err(e);
        }

        // 모듈 목록에 추가
        let mut modules = LOADED_MODULES.write();
        modules.push(module);

        // 마지막 추가된 모듈 참조 반환
        let module_ref = modules.last().unwrap().as_ref();
        let static_ref: &'static LoadedModule =
            unsafe { &*(module_ref as *const LoadedModule) };

        kprintln!("[module] Module '{}' loaded successfully", name);

        Ok(static_ref)
    }

    /// 실행 파일 로드 (ELF executable)
    pub fn load_executable(data: &[u8]) -> Result<ExecutableLoadInfo, ModuleError> {
        kprintln!("[module] Loading executable");
        const PF_X: u32 = 0x1;
        const PF_W: u32 = 0x2;

        // ELF 파싱
        let elf = Elf64::parse(data)?;

        // EXEC 또는 DYN 타입 확인
        let mut load_bias = 0usize;
        match elf.file_type() {
            ElfType::Exec => {}
            ElfType::Dyn => {
                let (dyn_min, dyn_max) = Self::compute_dyn_mapping_window(&elf)?;
                if dyn_min < Self::EXEC_VADDR_MIN || dyn_max > Self::EXEC_VADDR_MAX {
                    let span = dyn_max
                        .checked_sub(dyn_min)
                        .ok_or(ModuleError::InvalidFormat)?;
                    let dyn_base = Self::reserve_dyn_base(span)?;
                    load_bias = dyn_base
                        .checked_sub(dyn_min)
                        .ok_or(ModuleError::InvalidFormat)?;
                }
                kprintln!("[module] ET_DYN load bias: 0x{:x}", load_bias);
            }
            _ => {
                kprintln!(
                    "[module] Error: Not an executable (type={:?})",
                    elf.file_type()
                );
                return Err(ModuleError::InvalidFormat);
            }
        }

        let mut allocated_frames: Vec<usize> = Vec::new();
        let mut page_mappings: Vec<ExecPageMapping> = Vec::new();

        // LOAD 세그먼트 로드
        for ph in elf.load_segments() {
            let file_offset = ph.p_offset as usize;
            let file_size = ph.p_filesz as usize;
            let mem_size = ph.p_memsz as usize;
            let vaddr = ph.p_vaddr as usize;
            let mapped_vaddr = vaddr
                .checked_add(load_bias)
                .ok_or(ModuleError::InvalidFormat)?;
            let writable = (ph.p_flags & PF_W) != 0;
            let executable = (ph.p_flags & PF_X) != 0;
            let vend = match mapped_vaddr.checked_add(mem_size) {
                Some(end) => end,
                None => {
                    Self::cleanup_exec_frames(&mut allocated_frames);
                    return Err(ModuleError::InvalidFormat);
                }
            };

            kprintln!(
                "[module] LOAD segment: vaddr=0x{:x}, filesz={}, memsz={}",
                mapped_vaddr,
                file_size,
                mem_size
            );

            if file_size > mem_size {
                Self::cleanup_exec_frames(&mut allocated_frames);
                return Err(ModuleError::InvalidFormat);
            }

            let file_end = match file_offset.checked_add(file_size) {
                Some(v) => v,
                None => {
                    Self::cleanup_exec_frames(&mut allocated_frames);
                    return Err(ModuleError::InvalidFormat);
                }
            };
            if file_end > data.len() {
                Self::cleanup_exec_frames(&mut allocated_frames);
                return Err(ModuleError::InvalidFormat);
            }

            // 사용자 공간 로드 허용 범위 검사
            if mapped_vaddr < Self::EXEC_VADDR_MIN || vend > Self::EXEC_VADDR_MAX {
                kprintln!(
                    "[module] executable segment vaddr out of supported range: 0x{:x}-0x{:x}",
                    mapped_vaddr,
                    vend
                );
                Self::cleanup_exec_frames(&mut allocated_frames);
                return Err(ModuleError::InvalidFormat);
            }

            let seg_page_start = mapped_vaddr & !(PAGE_SIZE - 1);
            let seg_page_end = (vend + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);

            for page_addr in (seg_page_start..seg_page_end).step_by(PAGE_SIZE) {
                let frame = if let Some(frame) = page::alloc_frame() {
                    frame
                } else {
                    Self::cleanup_exec_frames(&mut allocated_frames);
                    return Err(ModuleError::OutOfMemory);
                };
                allocated_frames.push(frame);

                unsafe {
                    // SAFETY: alloc_frame()로 받은 유효한 4KB 프레임을 초기화한다.
                    core::ptr::write_bytes(frame as *mut u8, 0, PAGE_SIZE);
                }

                #[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
                if crate::arch::mmu::map_user_page_noflush(page_addr, frame, writable, executable)
                    .is_err()
                {
                    Self::cleanup_exec_frames(&mut allocated_frames);
                    return Err(ModuleError::InvalidFormat);
                }

                page_mappings.push(ExecPageMapping {
                    user_page: page_addr,
                    frame,
                });

                // 파일 데이터 복사 (BSS는 0으로 유지)
                let copy_start = core::cmp::max(page_addr, mapped_vaddr);
                let file_vend = mapped_vaddr + file_size;
                let copy_end = core::cmp::min(page_addr + PAGE_SIZE, file_vend);
                if copy_start < copy_end {
                    let copy_len = copy_end - copy_start;
                    let src_off = file_offset + (copy_start - mapped_vaddr);
                    let src_end = src_off + copy_len;
                    if src_end > data.len() {
                        Self::cleanup_exec_frames(&mut allocated_frames);
                        return Err(ModuleError::InvalidFormat);
                    }

                    let dst_off = copy_start - page_addr;
                    unsafe {
                        // SAFETY: src는 ELF 버퍼 내 유효 범위이며 dst는 할당한 프레임 내 유효 범위다.
                        core::ptr::copy_nonoverlapping(
                            data[src_off..src_end].as_ptr(),
                            (frame + dst_off) as *mut u8,
                            copy_len,
                        );
                    }
                }
            }
        }

        #[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
        crate::arch::mmu::flush_tlb_all();

        let dynamic_info = Self::parse_dynamic_info(&elf, load_bias)?;
        let tls_info = Self::assign_tls_tprel_base(Self::collect_tls_info(
            &elf,
            load_bias,
            &page_mappings,
        )?)?;
        Self::apply_dynamic_relocations(
            &elf,
            data,
            load_bias,
            &dynamic_info,
            &tls_info,
            &page_mappings,
        )?;
        let exported_symbols = Self::collect_dynamic_exports(&elf, data, load_bias, &dynamic_info)?;
        let exported_tls_symbols =
            Self::collect_dynamic_tls_exports(&elf, data, load_bias, &dynamic_info, &tls_info)?;
        Self::register_exec_dynamic_symbols(&exported_symbols);
        Self::register_exec_dynamic_tls_symbols(&exported_tls_symbols);
        Self::register_exec_tls_module(&tls_info);
        if dynamic_info.at_dynamic != 0 {
            kprintln!(
                "[module] DYNAMIC: at={:#x}, needed={}, strtab={:#x}, symtab={:#x}, rela={:#x}, rel={:#x}, jmprel={:#x}",
                dynamic_info.at_dynamic,
                dynamic_info.needed_count,
                dynamic_info.strtab,
                dynamic_info.symtab,
                dynamic_info.rela,
                dynamic_info.rel,
                dynamic_info.jmprel
            );
        }
        if tls_info.has_tls {
            kprintln!(
                "[module] TLS: memsz={}, filesz={}, align={}, tprel_base={:#x}",
                tls_info.mem_size,
                tls_info.template.len(),
                tls_info.align,
                tls_info.tprel_base
            );
        }

        // 엔트리 포인트 반환
        let entry = (elf.entry_point() as usize)
            .checked_add(load_bias)
            .ok_or(ModuleError::InvalidFormat)?;
        kprintln!("[module] Entry point: 0x{:x}", entry);

        Ok(ExecutableLoadInfo {
            entry,
            load_bias,
            dynamic: dynamic_info,
            tls: tls_info,
            exported_symbols,
        })
    }

    /// 섹션들을 메모리에 로드
    fn load_sections(elf: &Elf64, base_addr: usize) -> Result<Vec<usize>, ModuleError> {
        let mut section_addrs = Vec::new();
        let mut current_offset = 0usize;

        for sh in elf.sections().iter() {
            // ALLOC 플래그가 있는 섹션만 로드
            if sh.sh_flags & section_flags::SHF_ALLOC != 0 {
                // 정렬
                let align = sh.sh_addralign as usize;
                if align > 0 && current_offset % align != 0 {
                    current_offset = (current_offset + align - 1) & !(align - 1);
                }

                let load_addr = base_addr + current_offset;
                section_addrs.push(load_addr);

                let section_name = elf.section_name(sh);
                kprintln!(
                    "[module] Loading section '{}' ({} bytes) at 0x{:x}",
                    section_name,
                    sh.sh_size,
                    load_addr
                );

                // NOBITS가 아니면 데이터 복사
                if sh.sh_type != section_type::SHT_NOBITS {
                    let data = elf.section_data(sh);
                    unsafe {
                        core::ptr::copy_nonoverlapping(data.as_ptr(), load_addr as *mut u8, data.len());
                    }
                }

                current_offset += sh.sh_size as usize;
            } else {
                // 로드하지 않는 섹션은 0으로 표시
                section_addrs.push(0);
            }
        }

        Ok(section_addrs)
    }

    /// 재배치 적용
    fn apply_relocations(elf: &Elf64, section_addrs: &[usize], plt: &mut Option<PltTable>) -> Result<(), ModuleError> {
        let (_symtab_sh, symbols) = elf.symbol_table().ok_or(ModuleError::SymbolNotFound)?;

        // RISC-V: PCREL_HI20 결과를 저장하여 PCREL_LO12에서 사용 - Vec으로 변경
        #[cfg(target_arch = "riscv64")]
        let mut hi20_results: Vec<(usize, i64)> = Vec::new();

        for (rela_sh, relas) in elf.relocations() {
            // 재배치 대상 섹션
            let target_section_idx = rela_sh.sh_info as usize;
            if target_section_idx >= section_addrs.len() {
                continue;
            }
            let section_base = section_addrs[target_section_idx];
            if section_base == 0 {
                continue; // 로드되지 않은 섹션
            }

            kprintln!(
                "[module] Processing {} relocations for section {}",
                relas.len(),
                target_section_idx
            );

            for rela in relas {
                let sym_idx = rela.symbol() as usize;
                let rel_type = rela.rel_type();

                if sym_idx >= symbols.len() {
                    continue;
                }

                let sym = &symbols[sym_idx];
                let sym_name = elf.symbol_name(sym);

                // 심볼 값 결정
                let sym_value = if sym.st_shndx == section_index::SHN_UNDEF {
                    // 외부 심볼 - 커널 심볼 테이블에서 찾기
                    lookup_symbol(sym_name).ok_or_else(|| {
                        kprintln!("[module] Undefined symbol: {}", sym_name);
                        ModuleError::SymbolNotFound
                    })?
                } else if sym.st_shndx == section_index::SHN_ABS {
                    // 절대값
                    sym.st_value as usize
                } else {
                    // 로컬 심볼
                    let sym_section = sym.st_shndx as usize;
                    if sym_section < section_addrs.len() && section_addrs[sym_section] != 0 {
                        section_addrs[sym_section] + sym.st_value as usize
                    } else {
                        sym.st_value as usize
                    }
                };

                // 재배치 적용 위치
                let reloc_addr = section_base + rela.r_offset as usize;
                let addend = rela.r_addend;

                // 아키텍처별 재배치 처리
                #[cfg(target_arch = "aarch64")]
                Self::apply_relocation_aarch64(reloc_addr, sym_value, addend, rel_type, plt)?;

                #[cfg(target_arch = "riscv64")]
                Self::apply_relocation_riscv(reloc_addr, sym_value, addend, rel_type, &mut hi20_results, plt)?;
            }
        }

        Ok(())
    }

    /// AArch64 재배치 적용
    #[cfg(target_arch = "aarch64")]
    fn apply_relocation_aarch64(
        reloc_addr: usize,
        sym_value: usize,
        addend: i64,
        rel_type: u32,
        plt: &mut Option<PltTable>,
    ) -> Result<(), ModuleError> {
        use super::elf::reloc_aarch64::*;

        let s = sym_value as i64;
        let a = addend;
        let p = reloc_addr as i64;

        match rel_type {
            R_AARCH64_NONE => {}

            R_AARCH64_ABS64 => {
                // S + A
                let value = (s + a) as u64;
                unsafe {
                    *(reloc_addr as *mut u64) = value;
                }
            }

            R_AARCH64_ABS32 => {
                // S + A (32비트)
                let value = (s + a) as u32;
                unsafe {
                    *(reloc_addr as *mut u32) = value;
                }
            }

            R_AARCH64_PREL32 => {
                // S + A - P (32비트 PC 상대)
                let value = (s + a - p) as i32;
                unsafe {
                    *(reloc_addr as *mut i32) = value;
                }
            }

            R_AARCH64_PREL64 => {
                // S + A - P (64비트 PC 상대)
                let value = s + a - p;
                unsafe {
                    *(reloc_addr as *mut i64) = value;
                }
            }

            R_AARCH64_CALL26 | R_AARCH64_JUMP26 => {
                // S + A - P, 26비트 오프셋 (BL/B 명령)
                let target = (s + a) as usize;
                let offset = ((target as i64 - p) >> 2) as i32;
                
                // ±128MB 범위 체크
                let final_offset = if offset > 0x1ffffff || offset < -0x2000000 {
                    // 범위 초과 시 PLT 사용
                    if let Some(plt_table) = plt {
                        let plt_addr = plt_table.get_or_create(target).ok_or_else(|| {
                            kprintln!("[module] PLT table full");
                            ModuleError::UnsupportedRelocation(rel_type)
                        })?;
                        ((plt_addr as i64 - p) >> 2) as i32
                    } else {
                        kprintln!("[module] CALL26 offset out of range and no PLT available: {}", offset);
                        return Err(ModuleError::UnsupportedRelocation(rel_type));
                    }
                } else {
                    offset
                };
                
                unsafe {
                    let insn = *(reloc_addr as *mut u32);
                    let new_insn = (insn & 0xfc000000) | ((final_offset as u32) & 0x03ffffff);
                    *(reloc_addr as *mut u32) = new_insn;
                }
            }

            R_AARCH64_ADR_PREL_PG_HI21 => {
                // Page(S+A) - Page(P), ADRP 명령
                let page_s = (s + a) & !0xfff;
                let page_p = p & !0xfff;
                let offset = ((page_s - page_p) >> 12) as i32;

                if offset > 0xfffff || offset < -0x100000 {
                    kprintln!("[module] ADRP offset out of range");
                    return Err(ModuleError::UnsupportedRelocation(rel_type));
                }

                unsafe {
                    let insn = *(reloc_addr as *mut u32);
                    let immlo = ((offset & 0x3) as u32) << 29;
                    let immhi = (((offset >> 2) & 0x7ffff) as u32) << 5;
                    let new_insn = (insn & 0x9f00001f) | immlo | immhi;
                    *(reloc_addr as *mut u32) = new_insn;
                }
            }

            R_AARCH64_ADD_ABS_LO12_NC => {
                // S + A, 하위 12비트 (ADD 명령)
                let value = ((s + a) & 0xfff) as u32;
                unsafe {
                    let insn = *(reloc_addr as *mut u32);
                    let new_insn = (insn & 0xffc003ff) | (value << 10);
                    *(reloc_addr as *mut u32) = new_insn;
                }
            }

            R_AARCH64_LDST64_ABS_LO12_NC => {
                // S + A, 하위 12비트, 8바이트 정렬 (LDR/STR 64비트)
                let value = (((s + a) & 0xfff) >> 3) as u32;
                unsafe {
                    let insn = *(reloc_addr as *mut u32);
                    let new_insn = (insn & 0xffc003ff) | (value << 10);
                    *(reloc_addr as *mut u32) = new_insn;
                }
            }

            _ => {
                kprintln!("[module] Unsupported AArch64 relocation type: {}", rel_type);
                return Err(ModuleError::UnsupportedRelocation(rel_type));
            }
        }

        Ok(())
    }

    /// RISC-V 재배치 적용
    #[cfg(target_arch = "riscv64")]
    fn apply_relocation_riscv(
        reloc_addr: usize,
        sym_value: usize,
        addend: i64,
        rel_type: u32,
        hi20_results: &mut Vec<(usize, i64)>,
        plt: &mut Option<PltTable>,
    ) -> Result<(), ModuleError> {
        use super::elf::reloc_riscv::*;

        let s = sym_value as i64;
        let a = addend;
        let p = reloc_addr as i64;

        match rel_type {
            R_RISCV_NONE | R_RISCV_RELAX => {}

            R_RISCV_64 => {
                // S + A
                let value = (s + a) as u64;
                unsafe {
                    *(reloc_addr as *mut u64) = value;
                }
            }

            R_RISCV_32 => {
                // S + A (32비트)
                let value = (s + a) as u32;
                unsafe {
                    *(reloc_addr as *mut u32) = value;
                }
            }

            R_RISCV_BRANCH => {
                // S + A - P, B-type 명령 (조건 분기)
                let offset = (s + a - p) as i32;
                if offset > 0xfff || offset < -0x1000 {
                    return Err(ModuleError::UnsupportedRelocation(rel_type));
                }
                unsafe {
                    let insn = *(reloc_addr as *mut u32);
                    let imm12 = ((offset >> 12) & 0x1) as u32;
                    let imm10_5 = ((offset >> 5) & 0x3f) as u32;
                    let imm4_1 = ((offset >> 1) & 0xf) as u32;
                    let imm11 = ((offset >> 11) & 0x1) as u32;
                    let new_insn =
                        (insn & 0x01fff07f) | (imm12 << 31) | (imm10_5 << 25) | (imm4_1 << 8) | (imm11 << 7);
                    *(reloc_addr as *mut u32) = new_insn;
                }
            }

            R_RISCV_JAL => {
                // S + A - P, J-type 명령 (jal)
                let offset = (s + a - p) as i32;
                if offset > 0xfffff || offset < -0x100000 {
                    return Err(ModuleError::UnsupportedRelocation(rel_type));
                }
                unsafe {
                    let insn = *(reloc_addr as *mut u32);
                    let imm20 = ((offset >> 20) & 0x1) as u32;
                    let imm10_1 = ((offset >> 1) & 0x3ff) as u32;
                    let imm11 = ((offset >> 11) & 0x1) as u32;
                    let imm19_12 = ((offset >> 12) & 0xff) as u32;
                    let new_insn =
                        (insn & 0xfff) | (imm20 << 31) | (imm10_1 << 21) | (imm11 << 20) | (imm19_12 << 12);
                    *(reloc_addr as *mut u32) = new_insn;
                }
            }

            R_RISCV_HI20 => {
                // S + A, 상위 20비트 (lui)
                let value = ((s + a + 0x800) >> 12) as i32;
                unsafe {
                    let insn = *(reloc_addr as *mut u32);
                    let new_insn = (insn & 0xfff) | ((value as u32) << 12);
                    *(reloc_addr as *mut u32) = new_insn;
                }
            }

            R_RISCV_LO12_I => {
                // S + A, 하위 12비트 (I-type)
                let value = ((s + a) & 0xfff) as u32;
                unsafe {
                    let insn = *(reloc_addr as *mut u32);
                    let new_insn = (insn & 0xfffff) | (value << 20);
                    *(reloc_addr as *mut u32) = new_insn;
                }
            }

            R_RISCV_LO12_S => {
                // S + A, 하위 12비트 (S-type)
                let value = ((s + a) & 0xfff) as i32;
                unsafe {
                    let insn = *(reloc_addr as *mut u32);
                    let imm11_5 = ((value >> 5) & 0x7f) as u32;
                    let imm4_0 = (value & 0x1f) as u32;
                    let new_insn = (insn & 0x01fff07f) | (imm11_5 << 25) | (imm4_0 << 7);
                    *(reloc_addr as *mut u32) = new_insn;
                }
            }

            R_RISCV_PCREL_HI20 => {
                // S + A - P, 상위 20비트 (auipc)
                let offset = s + a - p;
                // HI20 결과를 저장 (LO12에서 참조)
                hi20_results.push((reloc_addr, offset));
                let value = ((offset + 0x800) >> 12) as i32;
                unsafe {
                    let insn = *(reloc_addr as *mut u32);
                    let new_insn = (insn & 0xfff) | ((value as u32) << 12);
                    *(reloc_addr as *mut u32) = new_insn;
                }
            }

            R_RISCV_PCREL_LO12_I => {
                // 하위 12비트 (I-type: load, addi 등)
                // 심볼은 대응하는 HI20의 주소를 가리킴
                // sym_value는 HI20 명령어의 주소
                let hi20_addr = sym_value;
                let offset = hi20_results.iter().find(|(addr, _)| *addr == hi20_addr).map(|(_, off)| *off).unwrap_or_else(|| {
                    // HI20 결과가 없으면 직접 계산 (fallback)
                    kprintln!("[module] Warning: PCREL_LO12_I without matching HI20 at 0x{:x}", hi20_addr);
                    s + a - (hi20_addr as i64)
                });
                let lo = (offset & 0xfff) as i32;
                unsafe {
                    let insn = *(reloc_addr as *mut u32);
                    let new_insn = (insn & 0xfffff) | ((lo as u32) << 20);
                    *(reloc_addr as *mut u32) = new_insn;
                }
            }

            R_RISCV_PCREL_LO12_S => {
                // 하위 12비트 (S-type: store)
                // 심볼은 대응하는 HI20의 주소를 가리킴
                let hi20_addr = sym_value;
                let offset = hi20_results.iter().find(|(addr, _)| *addr == hi20_addr).map(|(_, off)| *off).unwrap_or_else(|| {
                    kprintln!("[module] Warning: PCREL_LO12_S without matching HI20 at 0x{:x}", hi20_addr);
                    s + a - (hi20_addr as i64)
                });
                let lo = (offset & 0xfff) as i32;
                let imm11_5 = ((lo >> 5) & 0x7f) as u32;
                let imm4_0 = (lo & 0x1f) as u32;
                unsafe {
                    let insn = *(reloc_addr as *mut u32);
                    let new_insn = (insn & 0x01fff07f) | (imm11_5 << 25) | (imm4_0 << 7);
                    *(reloc_addr as *mut u32) = new_insn;
                }
            }

            R_RISCV_CALL | R_RISCV_CALL_PLT => {
                // auipc + jalr 쌍 (CALL_PLT는 CALL과 동일하게 처리)
                let target = (s + a) as usize;
                let mut offset = target as i64 - p;
                
                // ±2GB 범위 체크 (auipc의 20비트 + jalr의 12비트 = 32비트)
                if offset > 0x7FFFFFFF || offset < -0x80000000 {
                    // 범위 초과 시 PLT 사용
                    if let Some(plt_table) = plt {
                        let plt_addr = plt_table.get_or_create(target).ok_or_else(|| {
                            kprintln!("[module] PLT table full");
                            ModuleError::UnsupportedRelocation(rel_type)
                        })?;
                        offset = plt_addr as i64 - p;
                    } else {
                        kprintln!("[module] RISCV_CALL offset out of range and no PLT available");
                        return Err(ModuleError::UnsupportedRelocation(rel_type));
                    }
                }
                
                let hi = ((offset + 0x800) >> 12) as i32;
                let lo = (offset & 0xfff) as i32;

                unsafe {
                    // auipc
                    let auipc = *(reloc_addr as *mut u32);
                    let new_auipc = (auipc & 0xfff) | ((hi as u32) << 12);
                    *(reloc_addr as *mut u32) = new_auipc;

                    // jalr (다음 명령어)
                    let jalr_addr = reloc_addr + 4;
                    let jalr = *(jalr_addr as *mut u32);
                    let new_jalr = (jalr & 0xfffff) | ((lo as u32) << 20);
                    *(jalr_addr as *mut u32) = new_jalr;
                }
            }

            _ => {
                kprintln!("[module] Unsupported RISC-V relocation type: {}", rel_type);
                return Err(ModuleError::UnsupportedRelocation(rel_type));
            }
        }

        Ok(())
    }

    /// 명령어 캐시 플러시
    fn flush_icache(addr: usize, size: usize) {
        #[cfg(target_arch = "aarch64")]
        unsafe {
            let end = addr + size;
            let mut ptr = addr;
            while ptr < end {
                // 데이터 캐시 클린
                core::arch::asm!("dc cvau, {}", in(reg) ptr);
                ptr += 64; // 캐시 라인 크기
            }
            // 데이터 캐시 동기화
            core::arch::asm!("dsb ish");

            // 명령어 캐시 무효화
            ptr = addr;
            while ptr < end {
                core::arch::asm!("ic ivau, {}", in(reg) ptr);
                ptr += 64;
            }
            // 명령어 캐시 동기화
            core::arch::asm!("dsb ish");
            core::arch::asm!("isb");
        }

        #[cfg(target_arch = "riscv64")]
        unsafe {
            // RISC-V는 fence.i 명령으로 동기화
            core::arch::asm!("fence.i");
        }
    }

    /// 모듈 언로드
    /// 
    /// 안전한 unload 프로토콜:
    /// 1. unloading 플래그 설정 (새 참조 획득 차단)
    /// 2. 기존 참조가 모두 해제될 때까지 대기 (또는 즉시 실패)
    /// 3. exit 함수 호출
    /// 4. 메모리 해제
    pub fn unload(name: &str) -> Result<(), ModuleError> {
        // 1. 먼저 unloading 플래그 설정
        {
            let modules = LOADED_MODULES.read();
            let module = modules
                .iter()
                .find(|m| m.info.name == name)
                .ok_or(ModuleError::NotFound)?;

            // 이미 언로딩 중인지 확인
            if module.unloading.swap(true, Ordering::SeqCst) {
                return Err(ModuleError::ModuleUnloading);
            }
        }

        // 2. 참조 카운트 확인 (즉시 실패 방식)
        {
            let modules = LOADED_MODULES.read();
            let module = modules
                .iter()
                .find(|m| m.info.name == name)
                .ok_or(ModuleError::NotFound)?;

            if module.ref_count.load(Ordering::SeqCst) > 0 {
                // 플래그 롤백
                module.unloading.store(false, Ordering::SeqCst);
                return Err(ModuleError::InUse);
            }
        }

        // 3. 실제 언로드 수행
        let mut modules = LOADED_MODULES.write();

        let idx = modules
            .iter()
            .position(|m| m.info.name == name)
            .ok_or(ModuleError::NotFound)?;

        let module = &modules[idx];

        // exit 함수 호출
        module.exit();

        // 메모리 해제
        for &page in &module.pages {
            unsafe {
                page::free_frame(page);
            }
        }

        // 목록에서 제거
        modules.remove(idx);

        kprintln!("[module] Module '{}' unloaded", name);

        Ok(())
    }

    /// 모듈 언로드 (참조 해제 대기)
    /// 
    /// max_wait_ms: 최대 대기 시간 (밀리초), 0이면 무한 대기
    /// 반환: 성공 시 Ok(()), 타임아웃 시 Err(InUse)
    pub fn unload_wait(name: &str, max_wait_ms: usize) -> Result<(), ModuleError> {
        // 1. unloading 플래그 설정
        {
            let modules = LOADED_MODULES.read();
            let module = modules
                .iter()
                .find(|m| m.info.name == name)
                .ok_or(ModuleError::NotFound)?;

            if module.unloading.swap(true, Ordering::SeqCst) {
                return Err(ModuleError::ModuleUnloading);
            }
        }

        // 2. 참조 카운트가 0이 될 때까지 대기
        let _start = 0usize; // TODO: 실제 타이머 사용
        let mut waited = 0usize;
        loop {
            {
                let modules = LOADED_MODULES.read();
                if let Some(module) = modules.iter().find(|m| m.info.name == name) {
                    if module.ref_count.load(Ordering::SeqCst) == 0 {
                        break; // 참조 해제됨
                    }
                } else {
                    return Err(ModuleError::NotFound);
                }
            }

            // 스핀 대기 (TODO: yield 또는 sleep 사용)
            for _ in 0..1000 {
                core::hint::spin_loop();
            }
            waited += 1;

            if max_wait_ms > 0 && waited >= max_wait_ms {
                // 타임아웃: 플래그 롤백
                let modules = LOADED_MODULES.read();
                if let Some(module) = modules.iter().find(|m| m.info.name == name) {
                    module.unloading.store(false, Ordering::SeqCst);
                }
                return Err(ModuleError::InUse);
            }
        }

        // 3. 실제 언로드
        let mut modules = LOADED_MODULES.write();

        let idx = modules
            .iter()
            .position(|m| m.info.name == name)
            .ok_or(ModuleError::NotFound)?;

        let module = &modules[idx];
        module.exit();

        for &page in &module.pages {
            unsafe {
                page::free_frame(page);
            }
        }

        modules.remove(idx);

        kprintln!("[module] Module '{}' unloaded (waited {} iterations)", name, waited);

        Ok(())
    }

    /// 모듈 참조 획득 (RAII 가드 반환)
    /// 
    /// 모듈이 존재하고 언로딩 중이 아니면 참조를 획득하고 ModuleRef 반환
    /// ModuleRef가 drop되면 자동으로 참조 카운트 감소
    pub fn acquire(name: &str) -> Result<ModuleRef, ModuleError> {
        let modules = LOADED_MODULES.read();
        let module = modules
            .iter()
            .find(|m| m.info.name == name)
            .ok_or(ModuleError::NotFound)?;

        // 안전한 참조 획득 시도
        if module.try_get().is_none() {
            return Err(ModuleError::ModuleUnloading);
        }

        Ok(ModuleRef {
            module_name: String::from(name),
        })
    }

    /// 로드된 모듈 목록
    pub fn list() -> Vec<String> {
        let modules = LOADED_MODULES.read();
        modules.iter().map(|m| m.info.name.clone()).collect()
    }

    /// 모듈 상세 정보
    pub fn info(name: &str) -> Option<ModuleInfo> {
        let modules = LOADED_MODULES.read();
        modules.iter().find(|m| m.info.name == name).map(|m| ModuleInfo {
            name: m.info.name.clone(),
            version: m.info.version.clone(),
            base_addr: m.base_addr,
            size: m.size,
            state: m.state,
            ref_count: m.ref_count.load(Ordering::SeqCst),
            is_unloading: m.unloading.load(Ordering::SeqCst),
            exported_symbol_count: m.exported_symbols.len(),
        })
    }

    /// 특정 모듈에서 심볼 검색
    pub fn lookup_symbol_in(module_name: &str, symbol_name: &str) -> Option<usize> {
        let modules = LOADED_MODULES.read();
        for module in modules.iter() {
            if module.info.name == module_name {
                return module.lookup_symbol(symbol_name);
            }
        }
        None
    }

    /// 모든 모듈에서 심볼 검색 (커널 심볼 포함)
    /// 검색 순서: 커널 → 로드된 모듈들 (로드 순서)
    pub fn lookup_symbol_global(name: &str) -> Option<usize> {
        // 1. 커널 심볼 테이블에서 검색
        if let Some(addr) = lookup_symbol(name) {
            return Some(addr);
        }

        // 2. 로드된 모듈들에서 검색
        let modules = LOADED_MODULES.read();
        for module in modules.iter() {
            if let Some(addr) = module.lookup_symbol(name) {
                return Some(addr);
            }
        }

        None
    }

    /// 특정 모듈의 export된 심볼 목록
    pub fn list_module_symbols(module_name: &str) -> Vec<(String, usize)> {
        let modules = LOADED_MODULES.read();
        for module in modules.iter() {
            if module.info.name == module_name {
                return module.exported_symbols.clone();
            }
        }
        Vec::new()
    }

    /// 모듈에 심볼 export (외부에서 호출용)
    pub fn export_symbol(module_name: &str, symbol_name: &str, address: usize) -> bool {
        let mut modules = LOADED_MODULES.write();
        for module in modules.iter_mut() {
            if module.info.name == module_name {
                module.export_symbol(symbol_name, address);
                return true;
            }
        }
        false
    }

    /// VFS 파일 경로에서 모듈 로드
    /// RamFS, DevFS 등에서 모듈 파일을 읽어 로드
    pub fn load_from_path(path: &str) -> Result<&'static LoadedModule, ModuleError> {
        use alloc::vec::Vec;
        use crate::fs;

        kprintln!("[module] Loading module from path: {}", path);

        // VFS에서 파일 조회
        let node = fs::lookup_path(path).map_err(|e| {
            kprintln!("[module] Failed to lookup path: {:?}", e);
            ModuleError::NotFound
        })?;

        // 파일 크기 확인
        let stat = node.stat().map_err(|e| {
            kprintln!("[module] Failed to stat file: {:?}", e);
            ModuleError::NotFound
        })?;

        if stat.size == 0 {
            kprintln!("[module] File is empty");
            return Err(ModuleError::InvalidFormat);
        }

        kprintln!("[module] File size: {} bytes", stat.size);

        // 파일 내용 읽기
        let mut buffer = Vec::new();
        buffer.resize(stat.size as usize, 0u8);

        let bytes_read = node.read(0, &mut buffer).map_err(|e| {
            kprintln!("[module] Failed to read file: {:?}", e);
            ModuleError::NotFound
        })?;

        if bytes_read != stat.size as usize {
            kprintln!("[module] Partial read: {} of {} bytes", bytes_read, stat.size);
        }

        // 모듈 이름 추출 (경로에서 파일명)
        let name = path.rsplit('/').next().unwrap_or("unknown");
        let name = name.trim_end_matches(".ko");
        let name = name.trim_end_matches(".o");

        // ELF 모듈 로드
        Self::load_object(&buffer, name)
    }
}

/// 내장 테스트 모듈 (파일시스템 없이 테스트용)
pub mod builtin {
    use super::*;

    /// 테스트 모듈 init 함수
    fn test_module_init() -> i32 {
        crate::kprintln!("[test_module] Initialized!");
        crate::kprintln!("[test_module] Hello from dynamically loaded code!");
        0 // 성공
    }

    /// 테스트 모듈 exit 함수
    fn test_module_exit() {
        crate::kprintln!("[test_module] Exiting!");
    }

    /// 내장 테스트 모듈 로드 (ELF 파싱 없이 직접 로드)
    pub fn load_test_module() -> Result<(), ModuleError> {
        kprintln!("[module] Loading builtin test module...");

        // 페이지 할당
        let base_addr = page::alloc_frame().ok_or(ModuleError::OutOfMemory)?;

        kprintln!("[module] Test module at 0x{:x}", base_addr);

        // LoadedModule 생성
        let module = Box::new(LoadedModule {
            info: Module::new("test_builtin"),
            base_addr,
            size: PAGE_SIZE,
            state: ModuleState::Live,
            ref_count: AtomicUsize::new(0),
            unloading: AtomicBool::new(false),
            init_fn: Some(test_module_init as usize),
            exit_fn: Some(test_module_exit as usize),
            pages: alloc::vec![base_addr],
            section_addrs: alloc::vec![],
            exported_symbols: Vec::new(),
            plt_page: None, // 테스트 모듈은 PLT 불필요
        });

        // init 호출
        module.init()?;

        // 목록에 추가
        let mut modules = LOADED_MODULES.write();
        modules.push(module);

        kprintln!("[module] Builtin test module loaded successfully");
        Ok(())
    }
}
