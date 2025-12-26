//! RISC-V Sv39 MMU 드라이버
//! 
//! Sv39: 39-bit 가상 주소, 3-level 페이지 테이블
//! - Level 2 (VPN[2]): 9비트
//! - Level 1 (VPN[1]): 9비트  
//! - Level 0 (VPN[0]): 9비트
//! - Page offset: 12비트
//! 
//! PTE 형식:
//! [63:54] Reserved
//! [53:28] PPN[2]
//! [27:19] PPN[1]
//! [18:10] PPN[0]
//! [9:0] Flags (D/A/G/U/X/W/R/V)

use crate::kprintln;
use crate::mm::page::alloc_frame;
use core::ptr::write_bytes;

/// 페이지 크기 (4KB)
const PAGE_SIZE: usize = 4096;

/// Higher-half 커널 베이스 주소
pub const KERNEL_VIRT_BASE: usize = 0xFFFF_FFFF_8000_0000;

/// 페이지 테이블 엔트리 (PTE)
#[repr(transparent)]
#[derive(Clone, Copy)]
struct PageTableEntry(u64);

impl PageTableEntry {
    // PTE 플래그
    const V: u64 = 1 << 0;  // Valid
    const R: u64 = 1 << 1;  // Readable
    const W: u64 = 1 << 2;  // Writable
    const X: u64 = 1 << 3;  // Executable
    const U: u64 = 1 << 4;  // User
    const G: u64 = 1 << 5;  // Global
    const A: u64 = 1 << 6;  // Accessed
    const D: u64 = 1 << 7;  // Dirty

    const fn empty() -> Self {
        Self(0)
    }

    fn is_valid(&self) -> bool {
        self.0 & Self::V != 0
    }

    fn is_leaf(&self) -> bool {
        self.0 & (Self::R | Self::W | Self::X) != 0
    }

    /// 다음 레벨 페이지 테이블 생성
    fn new_table(next_table_ppn: usize) -> Self {
        Self((next_table_ppn << 10) as u64 | Self::V)
    }

    /// 리프 페이지 생성 (4KB)
    fn new_page(ppn: usize, flags: PageFlags) -> Self {
        let ppn_bits = (ppn << 10) as u64;
        Self(ppn_bits | flags.to_bits() | Self::V | Self::A | Self::D)
    }

    /// 2MB 메가페이지 생성 (Level 1)
    fn new_megapage(ppn: usize, flags: PageFlags) -> Self {
        let ppn_bits = (ppn << 10) as u64;
        Self(ppn_bits | flags.to_bits() | Self::V | Self::A | Self::D)
    }

    /// PPN 추출
    fn ppn(&self) -> usize {
        ((self.0 >> 10) & 0xFFF_FFFF_FFFF) as usize
    }

    /// 물리 주소 추출
    fn addr(&self) -> usize {
        self.ppn() << 12
    }
}

/// 페이지 플래그
#[derive(Clone, Copy)]
struct PageFlags {
    read: bool,
    write: bool,
    exec: bool,
    user: bool,
    global: bool,
}

impl PageFlags {
    fn kernel_rwx() -> Self {
        Self {
            read: true,
            write: true,
            exec: true,
            user: false,
            global: true,
        }
    }

    fn kernel_rw() -> Self {
        Self {
            read: true,
            write: true,
            exec: false,
            user: false,
            global: true,
        }
    }

    fn to_bits(&self) -> u64 {
        let mut bits = 0u64;
        if self.read { bits |= PageTableEntry::R; }
        if self.write { bits |= PageTableEntry::W; }
        if self.exec { bits |= PageTableEntry::X; }
        if self.user { bits |= PageTableEntry::U; }
        if self.global { bits |= PageTableEntry::G; }
        bits
    }
}

/// 페이지 테이블 (512 엔트리)
#[repr(C, align(4096))]
struct PageTable {
    entries: [PageTableEntry; 512],
}

impl PageTable {
    const fn new() -> Self {
        Self {
            entries: [PageTableEntry::empty(); 512],
        }
    }
}

/// 페이지 테이블 할당
fn alloc_page_table() -> Option<&'static mut PageTable> {
    let frame = alloc_frame()?;
    
    // 페이지 테이블 초기화
    unsafe {
        write_bytes(frame as *mut u8, 0, PAGE_SIZE);
        Some(&mut *(frame as *mut PageTable))
    }
}

/// 페이지 테이블 매니저
struct PageTableManager {
    root_table: &'static mut PageTable,
}

impl PageTableManager {
    fn new() -> Option<Self> {
        let root_table = alloc_page_table()?;
        Some(Self { root_table })
    }

    /// 가상 주소를 물리 주소로 매핑 (4KB 페이지)
    fn map_page(&mut self, virt: usize, phys: usize, flags: PageFlags) -> Result<(), &'static str> {
        let vpn2 = (virt >> 30) & 0x1FF;
        let vpn1 = (virt >> 21) & 0x1FF;
        let vpn0 = (virt >> 12) & 0x1FF;

        // Level 2
        let l1_table = unsafe {
            let entry_ptr = &mut self.root_table.entries[vpn2] as *mut PageTableEntry;
            self.get_or_create_next_level(&mut *entry_ptr)?
        };

        // Level 1
        let l0_table = unsafe {
            let entry_ptr = &mut l1_table.entries[vpn1] as *mut PageTableEntry;
            self.get_or_create_next_level(&mut *entry_ptr)?
        };

        // Level 0 (리프)
        let ppn = phys >> 12;
        l0_table.entries[vpn0] = PageTableEntry::new_page(ppn, flags);

        Ok(())
    }

    /// 2MB 메가페이지 매핑 (Level 1에서)
    fn map_megapage(&mut self, virt: usize, phys: usize, flags: PageFlags) -> Result<(), &'static str> {
        if virt & 0x1F_FFFF != 0 || phys & 0x1F_FFFF != 0 {
            return Err("Address must be 2MB aligned");
        }

        let vpn2 = (virt >> 30) & 0x1FF;
        let vpn1 = (virt >> 21) & 0x1FF;

        // Level 2
        let l1_table = unsafe {
            let entry_ptr = &mut self.root_table.entries[vpn2] as *mut PageTableEntry;
            self.get_or_create_next_level(entry_ptr)?
        };

        // Level 1 (리프 - 메가페이지)
        let ppn = phys >> 12;
        l1_table.entries[vpn1] = PageTableEntry::new_megapage(ppn, flags);

        Ok(())
    }

    unsafe fn get_or_create_next_level(
        &mut self,
        entry: *mut PageTableEntry,
    ) -> Result<&'static mut PageTable, &'static str> {
        let entry_ref = unsafe { &mut *entry };
        
        if !entry_ref.is_valid() {
            let new_table = alloc_page_table().ok_or("Failed to allocate page table")?;
            let ppn = (new_table as *const PageTable as usize) >> 12;
            *entry_ref = PageTableEntry::new_table(ppn);
        } else if entry_ref.is_leaf() {
            return Err("Entry is already a leaf page");
        }

        let addr = entry_ref.addr();
        Ok(unsafe { &mut *(addr as *mut PageTable) })
    }

    fn root_ppn(&self) -> usize {
        (self.root_table as *const PageTable as usize) >> 12
    }
}

/// Identity mapping + Higher-half kernel mapping 생성
pub fn create_mapping(
    ram_start: usize,
    ram_size: usize,
) -> Result<PageTableManager, &'static str> {
    let mut pt_mgr = PageTableManager::new().ok_or("Failed to create page table manager")?;

    kprintln!("[MMU] Creating identity + higher-half mapping...");

    let megapage_size = 2 * 1024 * 1024; // 2MB

    // 1. Identity mapping (물리 주소 = 가상 주소)
    kprintln!("[MMU] Identity mapping RAM...");
    for offset in (0..ram_size).step_by(megapage_size) {
        let addr = ram_start + offset;
        let aligned_addr = addr & !0x1F_FFFF;
        pt_mgr.map_megapage(aligned_addr, aligned_addr, PageFlags::kernel_rwx())?;
    }

    // 2. Higher-half mapping (가상: 0xFFFF_FFFF_8000_0000 -> 물리: ram_start)
    kprintln!("[MMU] Higher-half kernel mapping...");
    for offset in (0..ram_size).step_by(megapage_size) {
        let phys_addr = ram_start + offset;
        let virt_addr = KERNEL_VIRT_BASE + offset;
        let aligned_phys = phys_addr & !0x1F_FFFF;
        let aligned_virt = virt_addr & !0x1F_FFFF;
        pt_mgr.map_megapage(aligned_virt, aligned_phys, PageFlags::kernel_rwx())?;
    }

    // 3. MMIO 영역 매핑
    // UART: 0x1000_0000
    kprintln!("[MMU] Mapping UART MMIO...");
    pt_mgr.map_page(0x1000_0000, 0x1000_0000, PageFlags::kernel_rw())?;

    // CLINT: 0x0200_0000
    kprintln!("[MMU] Mapping CLINT MMIO...");
    pt_mgr.map_page(0x0200_0000, 0x0200_0000, PageFlags::kernel_rw())?;

    // PLIC: 0x0C00_0000 - 0x0C20_0000 (여러 페이지)
    kprintln!("[MMU] Mapping PLIC MMIO...");
    for offset in (0..0x20_0000).step_by(PAGE_SIZE) {
        pt_mgr.map_page(0x0C00_0000 + offset, 0x0C00_0000 + offset, PageFlags::kernel_rw())?;
    }

    kprintln!("[MMU] Mapping created, root PPN: {:#x}", pt_mgr.root_ppn());

    Ok(pt_mgr)
}

/// MMU 활성화
pub unsafe fn enable_mmu(root_ppn: usize) {
    kprintln!("[MMU] Enabling Sv39 MMU with root PPN {:#x}...", root_ppn);

    // satp 설정
    // Mode=8 (Sv39) | ASID=0 | PPN
    let satp = (8u64 << 60) | (root_ppn as u64);

    unsafe {
        // satp 레지스터 설정
        core::arch::asm!(
            "csrw satp, {}",
            "sfence.vma",
            in(reg) satp
        );
    }

    kprintln!("[MMU] MMU enabled!");
}

/// MMU 초기화
pub fn init(ram_start: usize, ram_size: usize) -> Result<(), &'static str> {
    kprintln!("\n[MMU] Initializing Sv39 MMU...");

    let pt_mgr = create_mapping(ram_start, ram_size)?;

    unsafe {
        enable_mmu(pt_mgr.root_ppn());
    }

    // 테스트: 메모리 접근
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
