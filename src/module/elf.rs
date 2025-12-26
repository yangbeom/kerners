//! ELF64 파서
//!
//! ELF64 포맷 파싱 및 검증
//! 참조: https://refspecs.linuxfoundation.org/elf/gabi4+/ch4.eheader.html

use core::mem::size_of;

/// ELF 매직 넘버
pub const ELF_MAGIC: [u8; 4] = [0x7f, b'E', b'L', b'F'];

/// ELF 클래스
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElfClass {
    None = 0,
    Elf32 = 1,
    Elf64 = 2,
}

/// ELF 데이터 인코딩 (엔디안)
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElfData {
    None = 0,
    Lsb = 1, // Little Endian
    Msb = 2, // Big Endian
}

/// ELF 타입
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElfType {
    None = 0,
    Rel = 1,  // Relocatable
    Exec = 2, // Executable
    Dyn = 3,  // Shared object
    Core = 4, // Core dump
}

impl From<u16> for ElfType {
    fn from(v: u16) -> Self {
        match v {
            1 => ElfType::Rel,
            2 => ElfType::Exec,
            3 => ElfType::Dyn,
            4 => ElfType::Core,
            _ => ElfType::None,
        }
    }
}

/// ELF 머신 타입
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElfMachine {
    None = 0,
    AArch64 = 183, // ARM 64-bit
    RiscV = 243,   // RISC-V
}

impl From<u16> for ElfMachine {
    fn from(v: u16) -> Self {
        match v {
            183 => ElfMachine::AArch64,
            243 => ElfMachine::RiscV,
            _ => ElfMachine::None,
        }
    }
}

/// ELF64 헤더 (64바이트)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Elf64Header {
    /// 매직 넘버 + 클래스 + 엔디안 + 버전 등
    pub e_ident: [u8; 16],
    /// 파일 타입 (REL, EXEC, DYN 등)
    pub e_type: u16,
    /// 머신 타입 (AArch64, RISC-V 등)
    pub e_machine: u16,
    /// ELF 버전
    pub e_version: u32,
    /// 엔트리 포인트 주소
    pub e_entry: u64,
    /// 프로그램 헤더 테이블 오프셋
    pub e_phoff: u64,
    /// 섹션 헤더 테이블 오프셋
    pub e_shoff: u64,
    /// 프로세서별 플래그
    pub e_flags: u32,
    /// ELF 헤더 크기
    pub e_ehsize: u16,
    /// 프로그램 헤더 엔트리 크기
    pub e_phentsize: u16,
    /// 프로그램 헤더 엔트리 개수
    pub e_phnum: u16,
    /// 섹션 헤더 엔트리 크기
    pub e_shentsize: u16,
    /// 섹션 헤더 엔트리 개수
    pub e_shnum: u16,
    /// 섹션 이름 문자열 테이블 인덱스
    pub e_shstrndx: u16,
}

/// ELF64 섹션 헤더 (64바이트)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Elf64SectionHeader {
    /// 섹션 이름 (문자열 테이블 인덱스)
    pub sh_name: u32,
    /// 섹션 타입
    pub sh_type: u32,
    /// 섹션 플래그
    pub sh_flags: u64,
    /// 메모리 주소 (로드 시)
    pub sh_addr: u64,
    /// 파일 내 오프셋
    pub sh_offset: u64,
    /// 섹션 크기
    pub sh_size: u64,
    /// 연관 섹션 인덱스
    pub sh_link: u32,
    /// 추가 정보
    pub sh_info: u32,
    /// 정렬 요구사항
    pub sh_addralign: u64,
    /// 엔트리 크기 (테이블의 경우)
    pub sh_entsize: u64,
}

/// 섹션 타입
pub mod section_type {
    pub const SHT_NULL: u32 = 0;
    pub const SHT_PROGBITS: u32 = 1; // 코드/데이터
    pub const SHT_SYMTAB: u32 = 2; // 심볼 테이블
    pub const SHT_STRTAB: u32 = 3; // 문자열 테이블
    pub const SHT_RELA: u32 = 4; // 재배치 (addend 포함)
    pub const SHT_HASH: u32 = 5; // 심볼 해시 테이블
    pub const SHT_DYNAMIC: u32 = 6; // 동적 링킹 정보
    pub const SHT_NOTE: u32 = 7; // 노트
    pub const SHT_NOBITS: u32 = 8; // BSS (파일에 없음)
    pub const SHT_REL: u32 = 9; // 재배치 (addend 없음)
    pub const SHT_DYNSYM: u32 = 11; // 동적 심볼 테이블
}

/// 섹션 플래그
pub mod section_flags {
    pub const SHF_WRITE: u64 = 0x1; // 쓰기 가능
    pub const SHF_ALLOC: u64 = 0x2; // 메모리 할당 필요
    pub const SHF_EXECINSTR: u64 = 0x4; // 실행 가능
}

/// ELF64 프로그램 헤더 (56바이트)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Elf64ProgramHeader {
    /// 세그먼트 타입
    pub p_type: u32,
    /// 세그먼트 플래그
    pub p_flags: u32,
    /// 파일 내 오프셋
    pub p_offset: u64,
    /// 가상 주소
    pub p_vaddr: u64,
    /// 물리 주소
    pub p_paddr: u64,
    /// 파일 내 크기
    pub p_filesz: u64,
    /// 메모리 내 크기
    pub p_memsz: u64,
    /// 정렬 요구사항
    pub p_align: u64,
}

/// 프로그램 헤더 타입
pub mod program_type {
    pub const PT_NULL: u32 = 0;
    pub const PT_LOAD: u32 = 1; // 로드 가능 세그먼트
    pub const PT_DYNAMIC: u32 = 2; // 동적 링킹 정보
    pub const PT_INTERP: u32 = 3; // 인터프리터 경로
    pub const PT_NOTE: u32 = 4; // 노트
    pub const PT_PHDR: u32 = 6; // 프로그램 헤더 테이블
}

/// ELF64 심볼 테이블 엔트리 (24바이트)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Elf64Symbol {
    /// 심볼 이름 (문자열 테이블 인덱스)
    pub st_name: u32,
    /// 심볼 정보 (바인딩 + 타입)
    pub st_info: u8,
    /// 심볼 가시성
    pub st_other: u8,
    /// 관련 섹션 인덱스
    pub st_shndx: u16,
    /// 심볼 값 (주소)
    pub st_value: u64,
    /// 심볼 크기
    pub st_size: u64,
}

impl Elf64Symbol {
    /// 심볼 바인딩 (상위 4비트)
    pub fn binding(&self) -> u8 {
        self.st_info >> 4
    }

    /// 심볼 타입 (하위 4비트)
    pub fn sym_type(&self) -> u8 {
        self.st_info & 0xf
    }
}

/// 특수 섹션 인덱스
pub mod section_index {
    pub const SHN_UNDEF: u16 = 0; // 미정의
    pub const SHN_ABS: u16 = 0xfff1; // 절대값
    pub const SHN_COMMON: u16 = 0xfff2; // 공통
}

/// ELF64 재배치 엔트리 (Rela, 24바이트)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Elf64Rela {
    /// 재배치 적용 오프셋
    pub r_offset: u64,
    /// 재배치 정보 (심볼 + 타입)
    pub r_info: u64,
    /// 추가값 (Addend)
    pub r_addend: i64,
}

impl Elf64Rela {
    /// 심볼 인덱스 (상위 32비트)
    pub fn symbol(&self) -> u32 {
        (self.r_info >> 32) as u32
    }

    /// 재배치 타입 (하위 32비트)
    pub fn rel_type(&self) -> u32 {
        (self.r_info & 0xffffffff) as u32
    }
}

/// AArch64 재배치 타입
pub mod reloc_aarch64 {
    pub const R_AARCH64_NONE: u32 = 0;
    pub const R_AARCH64_ABS64: u32 = 257; // S + A
    pub const R_AARCH64_ABS32: u32 = 258; // S + A
    pub const R_AARCH64_CALL26: u32 = 283; // S + A - P (BL)
    pub const R_AARCH64_JUMP26: u32 = 282; // S + A - P (B)
    pub const R_AARCH64_ADR_PREL_PG_HI21: u32 = 275; // Page(S+A) - Page(P)
    pub const R_AARCH64_ADD_ABS_LO12_NC: u32 = 277; // S + A (하위 12비트)
    pub const R_AARCH64_LDST64_ABS_LO12_NC: u32 = 286; // S + A (하위 12비트, 8바이트 정렬)
    pub const R_AARCH64_PREL32: u32 = 261; // S + A - P
    pub const R_AARCH64_PREL64: u32 = 260; // S + A - P
}

/// RISC-V 재배치 타입
pub mod reloc_riscv {
    pub const R_RISCV_NONE: u32 = 0;
    pub const R_RISCV_32: u32 = 1; // S + A
    pub const R_RISCV_64: u32 = 2; // S + A
    pub const R_RISCV_BRANCH: u32 = 16; // S + A - P (B-type)
    pub const R_RISCV_JAL: u32 = 17; // S + A - P (J-type)
    pub const R_RISCV_CALL: u32 = 18; // S + A - P (auipc+jalr)
    pub const R_RISCV_CALL_PLT: u32 = 19; // S + A - P (auipc+jalr, PLT)
    pub const R_RISCV_PCREL_HI20: u32 = 23; // S + A - P (상위 20비트)
    pub const R_RISCV_PCREL_LO12_I: u32 = 24; // S - P (하위 12비트, I-type) - 주의: 실제로는 auipc를 참조
    pub const R_RISCV_PCREL_LO12_S: u32 = 25; // S - P (하위 12비트, S-type)
    pub const R_RISCV_HI20: u32 = 26; // S + A (상위 20비트)
    pub const R_RISCV_LO12_I: u32 = 27; // S + A (하위 12비트, I-type)
    pub const R_RISCV_LO12_S: u32 = 28; // S + A (하위 12비트, S-type)
    pub const R_RISCV_RELAX: u32 = 51; // 링커 최적화 힌트
}

/// ELF64 파서 에러
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Elf64Error {
    /// 데이터가 너무 작음
    TooSmall,
    /// 잘못된 매직 넘버
    InvalidMagic,
    /// 지원하지 않는 클래스 (32비트)
    Not64Bit,
    /// 지원하지 않는 엔디안
    UnsupportedEndian,
    /// 지원하지 않는 머신 타입
    UnsupportedMachine,
    /// 잘못된 섹션 헤더
    InvalidSectionHeader,
    /// 잘못된 프로그램 헤더
    InvalidProgramHeader,
    /// 섹션을 찾을 수 없음
    SectionNotFound,
    /// 심볼을 찾을 수 없음
    SymbolNotFound,
}

/// 파싱된 ELF64 파일
pub struct Elf64<'a> {
    /// 원본 데이터
    data: &'a [u8],
    /// ELF 헤더
    pub header: &'a Elf64Header,
    /// 섹션 헤더 테이블
    section_headers: &'a [Elf64SectionHeader],
    /// 프로그램 헤더 테이블 (있는 경우)
    program_headers: Option<&'a [Elf64ProgramHeader]>,
    /// 섹션 이름 문자열 테이블
    shstrtab: &'a [u8],
}

impl<'a> Elf64<'a> {
    /// ELF64 파일 파싱
    pub fn parse(data: &'a [u8]) -> Result<Self, Elf64Error> {
        // 최소 크기 확인
        if data.len() < size_of::<Elf64Header>() {
            return Err(Elf64Error::TooSmall);
        }

        // 헤더 파싱
        let header = unsafe { &*(data.as_ptr() as *const Elf64Header) };

        // 매직 넘버 확인
        if header.e_ident[0..4] != ELF_MAGIC {
            return Err(Elf64Error::InvalidMagic);
        }

        // 64비트 확인
        if header.e_ident[4] != ElfClass::Elf64 as u8 {
            return Err(Elf64Error::Not64Bit);
        }

        // 리틀 엔디안 확인
        if header.e_ident[5] != ElfData::Lsb as u8 {
            return Err(Elf64Error::UnsupportedEndian);
        }

        // 머신 타입 확인
        let machine = ElfMachine::from(header.e_machine);
        #[cfg(target_arch = "aarch64")]
        if machine != ElfMachine::AArch64 {
            return Err(Elf64Error::UnsupportedMachine);
        }
        #[cfg(target_arch = "riscv64")]
        if machine != ElfMachine::RiscV {
            return Err(Elf64Error::UnsupportedMachine);
        }

        // 섹션 헤더 테이블 파싱
        let sh_offset = header.e_shoff as usize;
        let sh_count = header.e_shnum as usize;
        let sh_size = header.e_shentsize as usize;

        if sh_offset + sh_count * sh_size > data.len() {
            return Err(Elf64Error::InvalidSectionHeader);
        }

        let section_headers = unsafe {
            core::slice::from_raw_parts(
                data.as_ptr().add(sh_offset) as *const Elf64SectionHeader,
                sh_count,
            )
        };

        // 프로그램 헤더 테이블 파싱 (있는 경우)
        let program_headers = if header.e_phoff != 0 && header.e_phnum != 0 {
            let ph_offset = header.e_phoff as usize;
            let ph_count = header.e_phnum as usize;
            let ph_size = header.e_phentsize as usize;

            if ph_offset + ph_count * ph_size > data.len() {
                return Err(Elf64Error::InvalidProgramHeader);
            }

            Some(unsafe {
                core::slice::from_raw_parts(
                    data.as_ptr().add(ph_offset) as *const Elf64ProgramHeader,
                    ph_count,
                )
            })
        } else {
            None
        };

        // 섹션 이름 문자열 테이블
        let shstrtab_idx = header.e_shstrndx as usize;
        let shstrtab = if shstrtab_idx < sh_count {
            let sh = &section_headers[shstrtab_idx];
            let start = sh.sh_offset as usize;
            let end = start + sh.sh_size as usize;
            if end <= data.len() {
                &data[start..end]
            } else {
                &[]
            }
        } else {
            &[]
        };

        Ok(Self {
            data,
            header,
            section_headers,
            program_headers,
            shstrtab,
        })
    }

    /// 파일 타입 반환
    pub fn file_type(&self) -> ElfType {
        ElfType::from(self.header.e_type)
    }

    /// 머신 타입 반환
    pub fn machine(&self) -> ElfMachine {
        ElfMachine::from(self.header.e_machine)
    }

    /// 엔트리 포인트 주소
    pub fn entry_point(&self) -> u64 {
        self.header.e_entry
    }

    /// 섹션 헤더 목록
    pub fn sections(&self) -> &[Elf64SectionHeader] {
        self.section_headers
    }

    /// 프로그램 헤더 목록
    pub fn program_headers(&self) -> Option<&[Elf64ProgramHeader]> {
        self.program_headers
    }

    /// 섹션 이름 조회
    pub fn section_name(&self, sh: &Elf64SectionHeader) -> &str {
        let start = sh.sh_name as usize;
        if start >= self.shstrtab.len() {
            return "";
        }

        let end = self.shstrtab[start..]
            .iter()
            .position(|&b| b == 0)
            .map(|p| start + p)
            .unwrap_or(self.shstrtab.len());

        core::str::from_utf8(&self.shstrtab[start..end]).unwrap_or("")
    }

    /// 이름으로 섹션 찾기
    pub fn find_section(&self, name: &str) -> Option<&Elf64SectionHeader> {
        self.section_headers
            .iter()
            .find(|sh| self.section_name(sh) == name)
    }

    /// 섹션 데이터 반환
    pub fn section_data(&self, sh: &Elf64SectionHeader) -> &[u8] {
        if sh.sh_type == section_type::SHT_NOBITS {
            return &[];
        }
        let start = sh.sh_offset as usize;
        let end = start + sh.sh_size as usize;
        if end <= self.data.len() {
            &self.data[start..end]
        } else {
            &[]
        }
    }

    /// 심볼 테이블 섹션 찾기
    pub fn symbol_table(&self) -> Option<(&Elf64SectionHeader, &[Elf64Symbol])> {
        for sh in self.section_headers {
            if sh.sh_type == section_type::SHT_SYMTAB {
                let data = self.section_data(sh);
                let count = data.len() / size_of::<Elf64Symbol>();
                let symbols = unsafe {
                    core::slice::from_raw_parts(data.as_ptr() as *const Elf64Symbol, count)
                };
                return Some((sh, symbols));
            }
        }
        None
    }

    /// 문자열 테이블에서 문자열 조회
    pub fn string_at<'b>(&self, strtab: &'b [u8], offset: u32) -> &'b str {
        let start = offset as usize;
        if start >= strtab.len() {
            return "";
        }

        let end = strtab[start..]
            .iter()
            .position(|&b| b == 0)
            .map(|p| start + p)
            .unwrap_or(strtab.len());

        core::str::from_utf8(&strtab[start..end]).unwrap_or("")
    }

    /// 심볼 이름 조회
    pub fn symbol_name(&self, sym: &Elf64Symbol) -> &str {
        // 심볼 테이블의 sh_link가 문자열 테이블을 가리킴
        if let Some((symtab_sh, _)) = self.symbol_table() {
            let strtab_idx = symtab_sh.sh_link as usize;
            if strtab_idx < self.section_headers.len() {
                let strtab_sh = &self.section_headers[strtab_idx];
                let strtab = self.section_data(strtab_sh);
                return self.string_at(strtab, sym.st_name);
            }
        }
        ""
    }

    /// 이름으로 심볼 찾기
    pub fn find_symbol(&self, name: &str) -> Option<&Elf64Symbol> {
        if let Some((_, symbols)) = self.symbol_table() {
            for sym in symbols {
                if self.symbol_name(sym) == name {
                    return Some(sym);
                }
            }
        }
        None
    }

    /// 재배치 섹션들 순회
    pub fn relocations(&self) -> impl Iterator<Item = (&Elf64SectionHeader, &[Elf64Rela])> + '_ {
        self.section_headers.iter().filter_map(|sh| {
            if sh.sh_type == section_type::SHT_RELA {
                let data = self.section_data(sh);
                let count = data.len() / size_of::<Elf64Rela>();
                let relas =
                    unsafe { core::slice::from_raw_parts(data.as_ptr() as *const Elf64Rela, count) };
                Some((sh, relas))
            } else {
                None
            }
        })
    }

    /// LOAD 세그먼트들 순회 (실행 파일용)
    pub fn load_segments(&self) -> impl Iterator<Item = &Elf64ProgramHeader> + '_ {
        self.program_headers
            .into_iter()
            .flatten()
            .filter(|ph| ph.p_type == program_type::PT_LOAD)
    }

    /// 전체 메모리 요구량 계산 (LOAD 세그먼트 기준)
    pub fn memory_size(&self) -> usize {
        let mut max_addr = 0usize;
        for ph in self.load_segments() {
            let end = (ph.p_vaddr + ph.p_memsz) as usize;
            if end > max_addr {
                max_addr = end;
            }
        }
        max_addr
    }

    /// 전체 메모리 요구량 계산 (섹션 기준, relocatable용)
    pub fn section_memory_size(&self) -> usize {
        let mut total = 0usize;
        for sh in self.section_headers {
            if sh.sh_flags & section_flags::SHF_ALLOC != 0 {
                total += sh.sh_size as usize;
                // 정렬 고려
                let align = sh.sh_addralign as usize;
                if align > 0 {
                    total = (total + align - 1) & !(align - 1);
                }
            }
        }
        total
    }
}
