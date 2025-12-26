//! MMU (Memory Management Unit) for aarch64
//!
//! 4-level 페이지 테이블을 사용한 가상 메모리 관리
//! Higher-half kernel: 커널은 0xFFFF_0000_0000_0000 이상에 매핑

use crate::kprintln;
use crate::mm;
use core::arch::asm;

/// Higher-half 커널 베이스 주소
pub const KERNEL_VIRT_BASE: usize = 0xFFFF_0000_0000_0000;

/// 페이지 테이블 엔트리 (8 bytes)
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct PageTableEntry(u64);

impl PageTableEntry {
    const VALID: u64 = 1 << 0;
    const TABLE: u64 = 1 << 1; // Level 0-2: 다음 레벨 테이블
    const BLOCK: u64 = 0 << 1; // Level 1-2: 블록 매핑 (bit 1 = 0)
    const PAGE: u64 = 1 << 1; // Level 3: 실제 페이지
    const AF: u64 = 1 << 10; // Access Flag
    const ATTR_IDX_SHIFT: u64 = 2;
    const SH_INNER: u64 = 3 << 8; // Inner shareable

    pub const fn empty() -> Self {
        Self(0)
    }

    pub fn is_valid(&self) -> bool {
        self.0 & Self::VALID != 0
    }

    /// 다음 레벨 테이블을 가리키는 엔트리 생성
    pub fn new_table(next_table_addr: usize) -> Self {
        let addr = (next_table_addr as u64) & !0xFFF; // 하위 12비트 제거
        Self(addr | Self::VALID | Self::TABLE)
    }

    /// 2MB 블록 엔트리 생성 (Level 2)
    pub fn new_block(phys_addr: usize, flags: PageFlags) -> Self {
        let addr = (phys_addr as u64) & !0x1F_FFFF; // 2MB 정렬
        let attr = (flags.attr_idx as u64) << Self::ATTR_IDX_SHIFT;
        Self(addr | Self::VALID | Self::BLOCK | Self::AF | Self::SH_INNER | attr | flags.to_bits())
    }

    /// 물리 페이지를 가리키는 엔트리 생성 (Level 3)
    pub fn new_page(phys_addr: usize, flags: PageFlags) -> Self {
        let addr = (phys_addr as u64) & !0xFFF;
        let attr = (flags.attr_idx as u64) << Self::ATTR_IDX_SHIFT;
        Self(addr | Self::VALID | Self::PAGE | Self::AF | Self::SH_INNER | attr | flags.to_bits())
    }

    /// 물리 주소 추출
    pub fn addr(&self) -> usize {
        (self.0 & 0x0000_FFFF_FFFF_F000) as usize
    }
}

/// 페이지 속성
pub struct PageFlags {
    pub attr_idx: u8, // MAIR 인덱스
    pub write: bool,
    pub execute: bool,
}

impl PageFlags {
    pub fn kernel_rwx() -> Self {
        Self {
            attr_idx: 1, // Normal memory
            write: true,
            execute: true,
        }
    }

    pub fn device() -> Self {
        Self {
            attr_idx: 0, // Device memory
            write: true,
            execute: false,
        }
    }

    fn to_bits(&self) -> u64 {
        let mut bits = 0u64;

        // AP[2:1] - Access Permissions
        if !self.write {
            bits |= 1 << 7; // Read-only
        }

        // UXN/PXN - Execute Never
        if !self.execute {
            bits |= 1 << 53; // UXN
            bits |= 1 << 54; // PXN
        }

        bits
    }
}

/// 페이지 테이블 (512 엔트리)
#[repr(C, align(4096))]
pub struct PageTable {
    entries: [PageTableEntry; 512],
}

impl PageTable {
    pub const fn new() -> Self {
        Self {
            entries: [PageTableEntry::empty(); 512],
        }
    }

    pub fn zero(&mut self) {
        for entry in self.entries.iter_mut() {
            *entry = PageTableEntry::empty();
        }
    }

    pub fn entry(&self, index: usize) -> PageTableEntry {
        self.entries[index]
    }

    pub fn set_entry(&mut self, index: usize, entry: PageTableEntry) {
        self.entries[index] = entry;
    }
}

/// 새 페이지 테이블 할당
pub fn alloc_page_table() -> Option<&'static mut PageTable> {
    // 페이지 프레임 할당자에서 4KB 메모리 할당
    let frame = mm::page::alloc_frame()?;

    kprintln!("[MMU] Allocated page table at {:#x}", frame);

    // 물리 주소를 PageTable 구조체로 변환
    let page_table = unsafe { &mut *(frame as *mut PageTable) };

    // 0으로 초기화
    page_table.zero();

    Some(page_table)
}

/// 페이지 테이블 매니저
pub struct PageTableManager {
    l0_table: &'static mut PageTable,
}

impl PageTableManager {
    /// 새 페이지 테이블 매니저 생성
    pub fn new() -> Option<Self> {
        let l0_table = alloc_page_table()?;
        Some(Self { l0_table })
    }

    /// 2MB 블록 매핑 (Level 2에서)
    pub fn map_2mb_block(
        &mut self,
        virt_addr: usize,
        phys_addr: usize,
        flags: PageFlags,
    ) -> Result<(), &'static str> {
        // 2MB 정렬 확인
        if virt_addr & 0x1F_FFFF != 0 || phys_addr & 0x1F_FFFF != 0 {
            return Err("Address must be 2MB aligned");
        }

        let l0_idx = (virt_addr >> 39) & 0x1FF;
        let l1_idx = (virt_addr >> 30) & 0x1FF;
        let l2_idx = (virt_addr >> 21) & 0x1FF;

        // l0_table 포인터를 직접 사용
        let l0_ptr = self.l0_table as *mut PageTable;
        let l1_table = unsafe { Self::get_or_create_next_level_raw(l0_ptr, l0_idx)? };
        let l2_table = unsafe { Self::get_or_create_next_level_raw(l1_table, l1_idx)? };

        // Level 2에 블록 엔트리 생성
        let entry = PageTableEntry::new_block(phys_addr, flags);
        unsafe { (*l2_table).set_entry(l2_idx, entry) };

        Ok(())
    }

    /// 4KB 페이지 매핑
    pub fn map_page(
        &mut self,
        virt_addr: usize,
        phys_addr: usize,
        flags: PageFlags,
    ) -> Result<(), &'static str> {
        let l0_idx = (virt_addr >> 39) & 0x1FF;
        let l1_idx = (virt_addr >> 30) & 0x1FF;
        let l2_idx = (virt_addr >> 21) & 0x1FF;
        let l3_idx = (virt_addr >> 12) & 0x1FF;

        let l0_ptr = self.l0_table as *mut PageTable;
        let l1_table = unsafe { Self::get_or_create_next_level_raw(l0_ptr, l0_idx)? };
        let l2_table = unsafe { Self::get_or_create_next_level_raw(l1_table, l1_idx)? };
        let l3_table = unsafe { Self::get_or_create_next_level_raw(l2_table, l2_idx)? };

        // Level 3에 최종 페이지 매핑
        let entry = PageTableEntry::new_page(phys_addr, flags);
        unsafe { (*l3_table).set_entry(l3_idx, entry) };

        Ok(())
    }

    /// 다음 레벨 테이블 가져오기 또는 생성 (raw 포인터 버전)
    unsafe fn get_or_create_next_level_raw(
        table: *mut PageTable,
        index: usize,
    ) -> Result<*mut PageTable, &'static str> {
        let entry = unsafe { (*table).entry(index) };

        if entry.is_valid() {
            // 이미 존재하는 테이블
            let addr = entry.addr();
            Ok(addr as *mut PageTable)
        } else {
            // 새 테이블 할당
            let new_table = alloc_page_table().ok_or("Failed to allocate page table")?;

            let new_table_addr = new_table as *mut PageTable as usize;
            let new_entry = PageTableEntry::new_table(new_table_addr);
            unsafe { (*table).set_entry(index, new_entry) };

            Ok(new_table as *mut PageTable)
        }
    }

    /// 물리 주소 반환 (루트 테이블)
    pub fn root_table_addr(&self) -> usize {
        self.l0_table as *const PageTable as usize
    }
}

/// Identity mapping 생성 (커널 영역)
pub fn create_identity_mapping(
    ram_start: usize,
    ram_size: usize,
) -> Result<PageTableManager, &'static str> {
    let mut pt_mgr = PageTableManager::new().ok_or("Failed to create page table manager")?;

    kprintln!("[MMU] Creating identity mapping...");

    // 커널 영역을 2MB 블록으로 매핑
    let block_size = 2 * 1024 * 1024; // 2MB

    // Identity mapping (물리 주소 = 가상 주소)
    kprintln!("[MMU] Identity mapping RAM...");
    for offset in (0..ram_size).step_by(block_size) {
        let addr = ram_start + offset;
        let aligned_addr = addr & !0x1F_FFFF;
        pt_mgr.map_2mb_block(aligned_addr, aligned_addr, PageFlags::kernel_rwx())?;
    }

    // MMIO 영역: UART (0x09000000)
    kprintln!("[MMU] Mapping UART MMIO...");
    pt_mgr.map_page(0x0900_0000, 0x0900_0000, PageFlags::device())?;

    // MMIO 영역: GIC (0x08000000 - 0x08020000)
    kprintln!("[MMU] Mapping GIC MMIO...");
    pt_mgr.map_page(0x0800_0000, 0x0800_0000, PageFlags::device())?; // GICD
    pt_mgr.map_page(0x0801_0000, 0x0801_0000, PageFlags::device())?; // GICC

    // MMIO 영역: VirtIO (0x0a000000 - 0x0a004000, 32개 슬롯)
    kprintln!("[MMU] Mapping VirtIO MMIO...");
    for i in 0..4 {
        let addr = 0x0a00_0000 + i * 0x1000;
        pt_mgr.map_page(addr, addr, PageFlags::device())?;
    }

    kprintln!("[MMU] Identity mapping created");
    kprintln!("      Root table at: {:#x}", pt_mgr.root_table_addr());

    Ok(pt_mgr)
}

/// MMU 활성화
pub unsafe fn enable_mmu(pt_addr: usize) {
    kprintln!("[MMU] Enabling MMU with page table at {:#x}...", pt_addr);
    kprintln!("[MMU] Step 1: Setting MAIR_EL1");

    let pt_addr_u64 = pt_addr as u64;

    unsafe {
        // 1. MAIR_EL1 설정 (Memory Attribute Indirection Register)
        let mair_value: u64 = (0x00 << 0) |  // Attr0: Device-nGnRnE
            (0xFF << 8); // Attr1: Normal, Inner/Outer WB

        asm!("msr MAIR_EL1, {}", in(reg) mair_value);

        kprintln!("[MMU] Step 2: Setting TCR_EL1");

        // 2. TCR_EL1 설정 (Translation Control Register) - Identity mapping only
        let tcr_value: u64 = (16 << 0) |  // T0SZ: 48비트 VA
            (0 << 14) |  // TG0: 4KB
            (5 << 32); // IPS: 48비트 PA

        asm!("msr TCR_EL1, {}", in(reg) tcr_value);

        kprintln!("[MMU] Step 3: Setting TTBR0_EL1");

        // 3. TTBR0 설정 (identity mapping only)
        asm!(
            "msr TTBR0_EL1, {pt}",
            pt = in(reg) pt_addr_u64,
        );

        // 4. 배리어
        asm!("isb");

        kprintln!("[MMU] Step 4: Enabling MMU bit in SCTLR_EL1");

        // 5. MMU 켜기 (캐시는 일단 끄기)
        let mut sctlr: u64;
        asm!("mrs {}, SCTLR_EL1", out(reg) sctlr);

        sctlr |= 1 << 0; // M: MMU enable
        // 캐시는 일단 비활성화 (디버깅용)
        // sctlr |= 1 << 2; // C: Cache enable
        // sctlr |= 1 << 12; // I: Instruction cache

        asm!(
            "dsb sy",        // 메모리 작업 완료 보장
            "msr SCTLR_EL1, {}",
            "isb",           // 명령 동기화
            in(reg) sctlr
        );
    }

    kprintln!("[MMU] MMU enabled!");
}

/// MMU 초기화
pub fn init(ram_start: usize, ram_size: usize) -> Result<(), &'static str> {
    kprintln!("\n[MMU] Initializing...");

    // 1. Identity mapping 페이지 테이블 생성
    let pt_mgr = create_identity_mapping(ram_start, ram_size)?;

    // 2. MMU 활성화
    unsafe {
        enable_mmu(pt_mgr.root_table_addr());
    }

    // 3. 테스트: 메모리 접근
    let test_addr = (ram_start + 0x87000) as *mut u32;
    unsafe {
        *test_addr = 0xDEADBEEF;
        let read_val = *test_addr;
        if read_val != 0xDEADBEEF {
            return Err("MMU test failed: memory access incorrect");
        }
    }

    kprintln!("[MMU] Test passed: Memory access works!");

    Ok(())
}
