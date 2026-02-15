//! MMU (Memory Management Unit) for aarch64
//!
//! 4-level 페이지 테이블을 사용한 가상 메모리 관리
//! Higher-half kernel: 커널은 0xFFFF_0000_0000_0000 이상에 매핑

use crate::kprintln;
use crate::mm;
use crate::sync::Mutex;
use core::arch::asm;
use core::sync::atomic::{AtomicUsize, Ordering};

/// Higher-half 커널 베이스 주소
pub const KERNEL_VIRT_BASE: usize = 0xFFFF_0000_0000_0000;

/// 활성 커널 루트 페이지 테이블 주소
static ROOT_TABLE_ADDR: AtomicUsize = AtomicUsize::new(0);
/// 현재 CPU에서 활성화된 루트 페이지 테이블 주소
static ACTIVE_ROOT_TABLE_ADDR: AtomicUsize = AtomicUsize::new(0);
/// 런타임 매핑 수정 시 동기화용 락
static MMU_MAP_LOCK: Mutex<()> = Mutex::new(());

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
    pub user: bool,
}

impl PageFlags {
    pub fn kernel_rwx() -> Self {
        Self {
            attr_idx: 1, // Normal memory
            write: true,
            execute: true,
            user: false,
        }
    }

    pub fn user_rwx() -> Self {
        Self {
            attr_idx: 1, // Normal memory
            write: true,
            execute: true,
            user: true,
        }
    }

    pub fn user_from_segment(write: bool, execute: bool) -> Self {
        Self {
            attr_idx: 1, // Normal memory
            write,
            execute,
            user: true,
        }
    }

    pub fn device() -> Self {
        Self {
            attr_idx: 0, // Device memory
            write: true,
            execute: false,
            user: false,
        }
    }

    fn to_bits(&self) -> u64 {
        let mut bits = 0u64;

        // AP[2:1] - Access Permissions
        // Kernel:
        //   write=true  -> AP=00 (EL1 RW)
        //   write=false -> AP=10 (EL1 RO)
        // User:
        //   write=true  -> AP=01 (EL0 RW)
        //   write=false -> AP=11 (EL0 RO)
        if self.user {
            bits |= 1 << 6; // AP[1] = 1 (EL0 접근 허용)
            if !self.write {
                bits |= 1 << 7;
            }
        } else if !self.write {
            bits |= 1 << 7;
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

    pub fn entry_mut(&mut self, index: usize) -> &mut PageTableEntry {
        &mut self.entries[index]
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
    // 커널 기본 RAM 영역은 EL1 전용으로 유지한다.
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
    ROOT_TABLE_ADDR.store(pt_mgr.root_table_addr(), Ordering::Release);
    ACTIVE_ROOT_TABLE_ADDR.store(pt_mgr.root_table_addr(), Ordering::Release);

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

/// TLB 전체 flush
pub fn flush_tlb_all() {
    unsafe {
        // SAFETY: 페이지 테이블 업데이트 후 전역 TLB 무효화를 수행한다.
        asm!(
            "dsb ishst",
            "tlbi vmalle1is",
            "dsb ish",
            "isb",
            options(nostack)
        );
    }
}

fn map_user_page_inner(
    root: usize,
    virt_addr: usize,
    phys_addr: usize,
    write: bool,
    execute: bool,
) -> Result<(), &'static str> {
    if virt_addr & 0xFFF != 0 || phys_addr & 0xFFF != 0 {
        return Err("Address must be 4KB aligned");
    }

    if root == 0 {
        return Err("MMU root table not initialized");
    }

    let l0_table = unsafe { &mut *(root as *mut PageTable) };
    let mut pt_mgr = PageTableManager { l0_table };
    pt_mgr.map_page(
        virt_addr,
        phys_addr,
        PageFlags::user_from_segment(write, execute),
    )
}

fn resolve_user_l3_entry_mut(
    root: usize,
    virt_addr: usize,
) -> Result<(*mut PageTableEntry, PageTableEntry), &'static str> {
    if virt_addr & 0xFFF != 0 {
        return Err("Address must be 4KB aligned");
    }

    if root == 0 {
        return Err("MMU root table not initialized");
    }

    let l0_idx = (virt_addr >> 39) & 0x1FF;
    let l1_idx = (virt_addr >> 30) & 0x1FF;
    let l2_idx = (virt_addr >> 21) & 0x1FF;
    let l3_idx = (virt_addr >> 12) & 0x1FF;

    let l0_table = unsafe {
        // SAFETY: ROOT_TABLE_ADDR는 init 이후 유효한 L0 테이블을 가리킨다.
        &mut *(root as *mut PageTable)
    };
    let l0e = l0_table.entry(l0_idx);
    if !l0e.is_valid() {
        return Err("L0 entry is invalid");
    }

    let l1_table = unsafe {
        // SAFETY: 유효한 table descriptor의 물리 주소를 PageTable로 해석한다.
        &mut *(l0e.addr() as *mut PageTable)
    };
    let l1e = l1_table.entry(l1_idx);
    if !l1e.is_valid() {
        return Err("L1 entry is invalid");
    }

    let l2_table = unsafe {
        // SAFETY: 유효한 table descriptor의 물리 주소를 PageTable로 해석한다.
        &mut *(l1e.addr() as *mut PageTable)
    };
    let l2e = l2_table.entry(l2_idx);
    if !l2e.is_valid() {
        return Err("L2 entry is invalid");
    }

    let l3_table = unsafe {
        // SAFETY: 유효한 table descriptor의 물리 주소를 PageTable로 해석한다.
        &mut *(l2e.addr() as *mut PageTable)
    };
    let entry_ref = l3_table.entry_mut(l3_idx) as *mut PageTableEntry;
    let current = unsafe {
        // SAFETY: entry_ref는 위에서 얻은 l3_table 내부 엔트리를 가리킨다.
        *entry_ref
    };
    if !current.is_valid() {
        return Err("L3 entry is invalid");
    }

    Ok((entry_ref, current))
}

/// 부트 시 생성된 커널 기본 루트 페이지 테이블
pub fn kernel_root_table() -> usize {
    ROOT_TABLE_ADDR.load(Ordering::Acquire)
}

/// 현재 활성 루트 페이지 테이블
pub fn current_root_table() -> usize {
    ACTIVE_ROOT_TABLE_ADDR.load(Ordering::Acquire)
}

/// 루트 페이지 테이블 전환
pub fn switch_root_table(root: usize) -> Result<(), &'static str> {
    if root == 0 || root & 0xFFF != 0 {
        return Err("Invalid root table address");
    }
    unsafe {
        // SAFETY: 호출자는 유효한 L0 페이지 테이블 물리 주소를 전달한다.
        asm!(
            "msr TTBR0_EL1, {pt}",
            "isb",
            pt = in(reg) root as u64,
            options(nostack)
        );
    }
    ACTIVE_ROOT_TABLE_ADDR.store(root, Ordering::Release);
    flush_tlb_all();
    Ok(())
}

fn clone_page_table_level(src_addr: usize, level: usize) -> Result<usize, &'static str> {
    let new_table = alloc_page_table().ok_or("Failed to allocate page table")?;
    let new_addr = new_table as *mut PageTable as usize;
    let src = unsafe {
        // SAFETY: src_addr는 호출자가 제공한 유효한 페이지 테이블 주소여야 한다.
        &*(src_addr as *const PageTable)
    };

    for i in 0..512 {
        let entry = src.entry(i);
        if !entry.is_valid() {
            continue;
        }

        let is_table = level < 3 && (entry.0 & PageTableEntry::TABLE) != 0;
        if is_table {
            let child = clone_page_table_level(entry.addr(), level + 1)?;
            new_table.set_entry(i, PageTableEntry::new_table(child));
        } else {
            new_table.set_entry(i, entry);
        }
    }

    Ok(new_addr)
}

/// 루트 페이지 테이블 깊은 복제
pub fn clone_root_table(src_root: usize) -> Result<usize, &'static str> {
    if src_root == 0 || src_root & 0xFFF != 0 {
        return Err("Invalid source root table");
    }
    clone_page_table_level(src_root, 0)
}

/// root 지정 유저 페이지 물리 주소 조회
pub fn get_user_page_phys_for_root(root: usize, virt_addr: usize) -> Result<usize, &'static str> {
    let (_entry_ptr, current) = resolve_user_l3_entry_mut(root, virt_addr)?;
    Ok(current.addr())
}

/// root 지정 유저 페이지 매핑 (flush 없음)
pub fn map_user_page_for_root_noflush(
    root: usize,
    virt_addr: usize,
    phys_addr: usize,
    write: bool,
    execute: bool,
) -> Result<(), &'static str> {
    map_user_page_inner(root, virt_addr, phys_addr, write, execute)
}

/// root 지정 유저 페이지 매핑 해제 (flush 없음)
pub fn unmap_user_page_for_root_noflush(
    root: usize,
    virt_addr: usize,
) -> Result<usize, &'static str> {
    let (entry_ptr, current) = resolve_user_l3_entry_mut(root, virt_addr)?;
    unsafe {
        // SAFETY: resolve_user_l3_entry_mut에서 유효한 엔트리 포인터를 보장한다.
        *entry_ptr = PageTableEntry::empty();
    }
    Ok(current.addr())
}

/// root 지정 유저 페이지 권한 변경 (flush 없음)
pub fn update_user_page_flags_for_root_noflush(
    root: usize,
    virt_addr: usize,
    write: bool,
    execute: bool,
) -> Result<(), &'static str> {
    let (entry_ptr, current) = resolve_user_l3_entry_mut(root, virt_addr)?;
    let phys = current.addr();
    unsafe {
        // SAFETY: resolve_user_l3_entry_mut에서 유효한 엔트리 포인터를 보장한다.
        *entry_ptr = PageTableEntry::new_page(phys, PageFlags::user_from_segment(write, execute));
    }
    Ok(())
}

/// 유저 페이지 1개 매핑 (flush 없음)
///
/// 주로 다수 페이지를 연속 매핑할 때 사용한다.
pub fn map_user_page_noflush(
    virt_addr: usize,
    phys_addr: usize,
    write: bool,
    execute: bool,
) -> Result<(), &'static str> {
    let _guard = MMU_MAP_LOCK.lock();
    let root = current_root_table();
    map_user_page_inner(root, virt_addr, phys_addr, write, execute)
}

/// 유저 페이지 1개 매핑 해제 (flush 없음)
///
/// 성공 시 기존 매핑의 물리 주소를 반환한다.
pub fn unmap_user_page_noflush(virt_addr: usize) -> Result<usize, &'static str> {
    let _guard = MMU_MAP_LOCK.lock();
    let root = current_root_table();
    let (entry_ptr, current) = resolve_user_l3_entry_mut(root, virt_addr)?;
    unsafe {
        // SAFETY: resolve_user_l3_entry_mut에서 유효한 L3 엔트리 포인터를 보장한다.
        *entry_ptr = PageTableEntry::empty();
    }
    Ok(current.addr())
}

/// 유저 페이지 1개 매핑 해제 + TLB flush
pub fn unmap_user_page(virt_addr: usize) -> Result<usize, &'static str> {
    let phys = unmap_user_page_noflush(virt_addr)?;
    flush_tlb_all();
    Ok(phys)
}

/// 유저 페이지 1개 권한 업데이트 (flush 없음)
pub fn update_user_page_flags_noflush(
    virt_addr: usize,
    write: bool,
    execute: bool,
) -> Result<(), &'static str> {
    let _guard = MMU_MAP_LOCK.lock();
    let root = current_root_table();
    let (entry_ptr, current) = resolve_user_l3_entry_mut(root, virt_addr)?;
    let phys = current.addr();
    unsafe {
        // SAFETY: resolve_user_l3_entry_mut에서 유효한 L3 엔트리 포인터를 보장한다.
        *entry_ptr = PageTableEntry::new_page(phys, PageFlags::user_from_segment(write, execute));
    }
    Ok(())
}

/// 유저 페이지 1개 권한 업데이트 + TLB flush
pub fn update_user_page_flags(
    virt_addr: usize,
    write: bool,
    execute: bool,
) -> Result<(), &'static str> {
    update_user_page_flags_noflush(virt_addr, write, execute)?;
    flush_tlb_all();
    Ok(())
}

/// 유저 페이지 1개 매핑 + TLB flush
pub fn map_user_page(
    virt_addr: usize,
    phys_addr: usize,
    write: bool,
    execute: bool,
) -> Result<(), &'static str> {
    let _guard = MMU_MAP_LOCK.lock();
    let root = current_root_table();
    map_user_page_inner(root, virt_addr, phys_addr, write, execute)?;
    flush_tlb_all();
    Ok(())
}
