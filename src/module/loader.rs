//! 모듈 로더
//!
//! ELF64 relocatable object (.o) 및 executable 로딩
//! - 섹션 로딩 및 메모리 할당
//! - 재배치 처리 (PLT 스텁 지원)
//! - 모듈 라이프사이클 관리

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
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

/// 모듈 로더
pub struct ModuleLoader;

impl ModuleLoader {
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
    pub fn load_executable(data: &[u8]) -> Result<usize, ModuleError> {
        kprintln!("[module] Loading executable");

        // ELF 파싱
        let elf = Elf64::parse(data)?;

        // EXEC 또는 DYN 타입 확인
        match elf.file_type() {
            ElfType::Exec | ElfType::Dyn => {}
            _ => {
                kprintln!(
                    "[module] Error: Not an executable (type={:?})",
                    elf.file_type()
                );
                return Err(ModuleError::InvalidFormat);
            }
        }

        // LOAD 세그먼트 로드
        for ph in elf.load_segments() {
            let file_offset = ph.p_offset as usize;
            let file_size = ph.p_filesz as usize;
            let mem_size = ph.p_memsz as usize;
            let vaddr = ph.p_vaddr as usize;

            kprintln!(
                "[module] LOAD segment: vaddr=0x{:x}, filesz={}, memsz={}",
                vaddr,
                file_size,
                mem_size
            );

            // 페이지 할당 및 데이터 복사
            let num_pages = (mem_size + PAGE_SIZE - 1) / PAGE_SIZE;

            for i in 0..num_pages {
                let page_addr = vaddr + i * PAGE_SIZE;

                // 페이지 할당 (가상 주소에 매핑 필요 - 현재는 identity mapping 가정)
                if let Some(_frame) = page::alloc_frame() {
                    // 파일 데이터 복사
                    let copy_start = i * PAGE_SIZE;
                    let copy_end = core::cmp::min((i + 1) * PAGE_SIZE, file_size);

                    if copy_start < file_size {
                        let src = &data[file_offset + copy_start..file_offset + copy_end];
                        unsafe {
                            core::ptr::copy_nonoverlapping(
                                src.as_ptr(),
                                page_addr as *mut u8,
                                src.len(),
                            );
                        }
                    }
                } else {
                    return Err(ModuleError::OutOfMemory);
                }
            }
        }

        // 엔트리 포인트 반환
        let entry = elf.entry_point() as usize;
        kprintln!("[module] Entry point: 0x{:x}", entry);

        Ok(entry)
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
