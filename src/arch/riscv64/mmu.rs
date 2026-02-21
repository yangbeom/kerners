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
use crate::sync::Mutex;
use alloc::vec::Vec;
use core::arch::asm;
use core::ptr::write_bytes;
use core::sync::atomic::{AtomicUsize, Ordering};

/// 페이지 크기 (4KB)
const PAGE_SIZE: usize = 4096;
const MMIO_DEFAULT_SIZE: usize = PAGE_SIZE;
const GOLDFISH_RTC_COMPAT: &str = "google,goldfish-rtc";
const GOLDFISH_RTC_FALLBACK_BASE: usize = 0x0010_1000;
const VIRTIO_MMIO_COMPAT: &str = "virtio,mmio";

/// Higher-half 커널 베이스 주소
pub const KERNEL_VIRT_BASE: usize = 0xFFFF_FFFF_8000_0000;

/// 부트 시 생성된 커널 기본 루트 페이지 테이블 주소
static ROOT_TABLE_ADDR: AtomicUsize = AtomicUsize::new(0);
/// 현재 CPU에서 활성화된 루트 페이지 테이블 주소
static ACTIVE_ROOT_TABLE_ADDR: AtomicUsize = AtomicUsize::new(0);
/// 런타임 매핑 수정 시 동기화용 락
static MMU_MAP_LOCK: Mutex<()> = Mutex::new(());

/// 페이지 테이블 엔트리 (PTE)
#[repr(transparent)]
#[derive(Clone, Copy)]
struct PageTableEntry(u64);

impl PageTableEntry {
    // PTE 플래그
    const V: u64 = 1 << 0; // Valid
    const R: u64 = 1 << 1; // Readable
    const W: u64 = 1 << 2; // Writable
    const X: u64 = 1 << 3; // Executable
    const U: u64 = 1 << 4; // User
    const G: u64 = 1 << 5; // Global
    const A: u64 = 1 << 6; // Accessed
    const D: u64 = 1 << 7; // Dirty

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

    fn user_from_segment(write: bool, execute: bool) -> Self {
        Self {
            read: true,
            write,
            exec: execute,
            user: true,
            global: false,
        }
    }

    fn to_bits(&self) -> u64 {
        let mut bits = 0u64;
        if self.read {
            bits |= PageTableEntry::R;
        }
        if self.write {
            bits |= PageTableEntry::W;
        }
        if self.exec {
            bits |= PageTableEntry::X;
        }
        if self.user {
            bits |= PageTableEntry::U;
        }
        if self.global {
            bits |= PageTableEntry::G;
        }
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

    fn entry(&self, index: usize) -> PageTableEntry {
        self.entries[index]
    }

    fn set_entry(&mut self, index: usize, entry: PageTableEntry) {
        self.entries[index] = entry;
    }

    fn entry_mut(&mut self, index: usize) -> &mut PageTableEntry {
        &mut self.entries[index]
    }
}

/// 페이지 테이블 할당
fn alloc_page_table() -> Option<&'static mut PageTable> {
    let frame = alloc_frame()?;

    // 페이지 테이블 초기화
    unsafe {
        // SAFETY: alloc_frame로 확보한 4KB 프레임을 PageTable로 사용하기 전에 0으로 초기화한다.
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
            self.get_or_create_next_level(entry_ptr)?
        };

        // Level 1
        let l0_table = unsafe {
            let entry_ptr = &mut l1_table.entries[vpn1] as *mut PageTableEntry;
            self.get_or_create_next_level(entry_ptr)?
        };

        // Level 0 (리프)
        let ppn = phys >> 12;
        l0_table.entries[vpn0] = PageTableEntry::new_page(ppn, flags);

        Ok(())
    }

    /// 2MB 메가페이지 매핑 (Level 1에서)
    fn map_megapage(
        &mut self,
        virt: usize,
        phys: usize,
        flags: PageFlags,
    ) -> Result<(), &'static str> {
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
        let entry_ref = unsafe {
            // SAFETY: 호출자가 전달한 엔트리 포인터는 현재 페이지 테이블의 유효 엔트리를 가리킨다.
            &mut *entry
        };

        if !entry_ref.is_valid() {
            let new_table = alloc_page_table().ok_or("Failed to allocate page table")?;
            let ppn = (new_table as *const PageTable as usize) >> 12;
            *entry_ref = PageTableEntry::new_table(ppn);
        } else if entry_ref.is_leaf() {
            return Err("Entry is already a leaf page");
        }

        let addr = entry_ref.addr();
        Ok(unsafe {
            // SAFETY: 유효한 non-leaf PTE는 다음 레벨 페이지 테이블의 물리 주소를 보유한다.
            &mut *(addr as *mut PageTable)
        })
    }

    fn root_ppn(&self) -> usize {
        (self.root_table as *const PageTable as usize) >> 12
    }

    fn root_table_addr(&self) -> usize {
        self.root_table as *const PageTable as usize
    }
}

/// Identity mapping + Higher-half kernel mapping 생성
fn create_mapping(ram_start: usize, ram_size: usize) -> Result<PageTableManager, &'static str> {
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
    let uart_base = crate::drivers::config::uart_base();
    let uart_size = crate::drivers::config::uart_size();
    kprintln!(
        "[MMU] Mapping UART MMIO: base={:#x}, size={:#x}",
        uart_base, uart_size
    );
    map_mmio_region(&mut pt_mgr, uart_base, uart_size)?;

    let (rtc_base, rtc_size) = rtc_region_from_dtb_or_fallback();
    kprintln!(
        "[MMU] Mapping RTC MMIO: base={:#x}, size={:#x}",
        rtc_base, rtc_size
    );
    map_mmio_region(&mut pt_mgr, rtc_base, rtc_size)?;

    let clint_base = crate::drivers::config::clint_base();
    let clint_size = crate::drivers::config::clint_size();
    kprintln!(
        "[MMU] Mapping CLINT MMIO: base={:#x}, size={:#x}",
        clint_base, clint_size
    );
    map_mmio_region(&mut pt_mgr, clint_base, clint_size)?;

    let plic_base = crate::drivers::config::plic_base();
    let plic_size = crate::drivers::config::plic_size();
    kprintln!(
        "[MMU] Mapping PLIC MMIO: base={:#x}, size={:#x}",
        plic_base, plic_size
    );
    map_mmio_region(&mut pt_mgr, plic_base, plic_size)?;

    if let Some(dt) = crate::dtb::get() {
        for info in dt.find_compatible(VIRTIO_MMIO_COMPAT) {
            if info.reg_base == 0 {
                continue;
            }
            let base = info.reg_base as usize;
            let size = if info.reg_size != 0 {
                info.reg_size as usize
            } else {
                MMIO_DEFAULT_SIZE
            };
            kprintln!(
                "[MMU] Mapping VirtIO MMIO (DTB): base={:#x}, size={:#x}",
                base, size
            );
            map_mmio_region(&mut pt_mgr, base, size)?;
        }
    }

    kprintln!("[MMU] Mapping created, root PPN: {:#x}", pt_mgr.root_ppn());

    Ok(pt_mgr)
}

#[inline]
fn align_down_page(addr: usize) -> usize {
    addr & !(PAGE_SIZE - 1)
}

#[inline]
fn align_up_page(addr: usize) -> usize {
    (addr.saturating_add(PAGE_SIZE - 1)) & !(PAGE_SIZE - 1)
}

fn map_mmio_region(
    pt_mgr: &mut PageTableManager,
    base: usize,
    size: usize,
) -> Result<(), &'static str> {
    if base == 0 {
        return Ok(());
    }
    let size = if size == 0 { MMIO_DEFAULT_SIZE } else { size };
    let start = align_down_page(base);
    let end = align_up_page(base.saturating_add(size));
    if end <= start {
        return Ok(());
    }
    for addr in (start..end).step_by(PAGE_SIZE) {
        pt_mgr.map_page(addr, addr, PageFlags::kernel_rw())?;
    }
    Ok(())
}

fn rtc_region_from_dtb_or_fallback() -> (usize, usize) {
    if let Some(dt) = crate::dtb::get() {
        if let Some(info) = dt.find_compatible(GOLDFISH_RTC_COMPAT).into_iter().next() {
            if info.reg_base != 0 {
                let size = if info.reg_size != 0 {
                    info.reg_size as usize
                } else {
                    MMIO_DEFAULT_SIZE
                };
                return (info.reg_base as usize, size);
            }
        }
    }
    (GOLDFISH_RTC_FALLBACK_BASE, MMIO_DEFAULT_SIZE)
}

/// MMU 활성화
pub unsafe fn enable_mmu(root_ppn: usize) {
    kprintln!("[MMU] Enabling Sv39 MMU with root PPN {:#x}...", root_ppn);

    // satp 설정
    // Mode=8 (Sv39) | ASID=0 | PPN
    let satp = (8u64 << 60) | (root_ppn as u64);

    unsafe {
        // SAFETY: root_ppn은 유효한 최상위 페이지 테이블 PPN이다.
        asm!(
            "csrw satp, {satp}",
            "sfence.vma",
            satp = in(reg) satp,
            options(nostack)
        );
    }

    kprintln!("[MMU] MMU enabled!");
}

/// MMU 초기화
pub fn init(ram_start: usize, ram_size: usize) -> Result<(), &'static str> {
    kprintln!("\n[MMU] Initializing Sv39 MMU...");

    let pt_mgr = create_mapping(ram_start, ram_size)?;
    let root_addr = pt_mgr.root_table_addr();
    ROOT_TABLE_ADDR.store(root_addr, Ordering::Release);
    ACTIVE_ROOT_TABLE_ADDR.store(root_addr, Ordering::Release);

    unsafe {
        // SAFETY: 방금 생성한 루트 테이블의 PPN으로 MMU를 활성화한다.
        enable_mmu(pt_mgr.root_ppn());
    }

    // 테스트: 메모리 접근
    let test_addr = (ram_start + 0x87000) as *mut u32;
    unsafe {
        // SAFETY: 테스트 주소는 RAM 범위 내 정렬된 유효 주소다.
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
        // SAFETY: 페이지 테이블 갱신 후 전역 TLB 무효화를 수행한다.
        asm!("sfence.vma", options(nostack));
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

    if root == 0 || root & 0xFFF != 0 {
        return Err("MMU root table not initialized");
    }

    let root_table = unsafe {
        // SAFETY: root는 유효한 최상위 페이지 테이블 주소다.
        &mut *(root as *mut PageTable)
    };
    let mut pt_mgr = PageTableManager { root_table };
    pt_mgr.map_page(
        virt_addr,
        phys_addr,
        PageFlags::user_from_segment(write, execute),
    )
}

fn resolve_user_l0_entry_mut(
    root: usize,
    virt_addr: usize,
) -> Result<(*mut PageTableEntry, PageTableEntry), &'static str> {
    if virt_addr & 0xFFF != 0 {
        return Err("Address must be 4KB aligned");
    }

    if root == 0 || root & 0xFFF != 0 {
        return Err("MMU root table not initialized");
    }

    let vpn2 = (virt_addr >> 30) & 0x1FF;
    let vpn1 = (virt_addr >> 21) & 0x1FF;
    let vpn0 = (virt_addr >> 12) & 0x1FF;

    let l2_table = unsafe {
        // SAFETY: root는 유효한 L2(루트) 페이지 테이블 주소다.
        &mut *(root as *mut PageTable)
    };
    let l2e = l2_table.entry(vpn2);
    if !l2e.is_valid() || l2e.is_leaf() {
        return Err("L2 entry is invalid");
    }

    let l1_table = unsafe {
        // SAFETY: 유효한 non-leaf L2 엔트리는 L1 테이블 주소를 가진다.
        &mut *(l2e.addr() as *mut PageTable)
    };
    let l1e = l1_table.entry(vpn1);
    if !l1e.is_valid() || l1e.is_leaf() {
        return Err("L1 entry is invalid");
    }

    let l0_table = unsafe {
        // SAFETY: 유효한 non-leaf L1 엔트리는 L0 테이블 주소를 가진다.
        &mut *(l1e.addr() as *mut PageTable)
    };
    let entry_ptr = l0_table.entry_mut(vpn0) as *mut PageTableEntry;
    let current = unsafe {
        // SAFETY: entry_ptr는 위에서 확보한 L0 테이블 내부 엔트리다.
        *entry_ptr
    };
    if !current.is_valid() || !current.is_leaf() {
        return Err("L0 entry is invalid");
    }

    Ok((entry_ptr, current))
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

    let satp = (8u64 << 60) | ((root as u64) >> 12);
    unsafe {
        // SAFETY: root는 유효한 페이지 정렬 루트 페이지 테이블 주소다.
        asm!(
            "csrw satp, {satp}",
            "sfence.vma",
            satp = in(reg) satp,
            options(nostack)
        );
    }

    ACTIVE_ROOT_TABLE_ADDR.store(root, Ordering::Release);
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

        let is_table = level > 0 && !entry.is_leaf();
        if is_table {
            let child = clone_page_table_level(entry.addr(), level - 1)?;
            new_table.set_entry(i, PageTableEntry::new_table(child >> 12));
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
    clone_page_table_level(src_root, 2)
}

/// 유저 페이지 매핑 스냅샷
#[derive(Clone, Copy)]
pub struct UserPageMapping {
    pub virt_addr: usize,
    pub phys_addr: usize,
    pub writable: bool,
    pub executable: bool,
}

/// root 기준 유저 페이지 매핑 목록 수집
pub fn collect_user_page_mappings_for_root(
    root: usize,
) -> Result<Vec<UserPageMapping>, &'static str> {
    if root == 0 || root & 0xFFF != 0 {
        return Err("Invalid root table address");
    }

    let mut mappings = Vec::new();
    let l2 = unsafe {
        // SAFETY: root는 호출자가 전달한 유효한 L2(루트) 페이지 테이블 주소여야 한다.
        &*(root as *const PageTable)
    };

    for vpn2 in 0..512 {
        let l2e = l2.entry(vpn2);
        if !l2e.is_valid() || l2e.is_leaf() {
            continue;
        }

        let l1 = unsafe {
            // SAFETY: 유효한 non-leaf L2 엔트리는 L1 테이블 주소를 가진다.
            &*(l2e.addr() as *const PageTable)
        };
        for vpn1 in 0..512 {
            let l1e = l1.entry(vpn1);
            if !l1e.is_valid() || l1e.is_leaf() {
                continue;
            }

            let l0 = unsafe {
                // SAFETY: 유효한 non-leaf L1 엔트리는 L0 테이블 주소를 가진다.
                &*(l1e.addr() as *const PageTable)
            };
            for vpn0 in 0..512 {
                let l0e = l0.entry(vpn0);
                if !l0e.is_valid() || !l0e.is_leaf() {
                    continue;
                }
                if (l0e.0 & PageTableEntry::U) == 0 {
                    continue;
                }

                let virt_addr = (vpn2 << 30) | (vpn1 << 21) | (vpn0 << 12);
                mappings.push(UserPageMapping {
                    virt_addr,
                    phys_addr: l0e.addr(),
                    writable: (l0e.0 & PageTableEntry::W) != 0,
                    executable: (l0e.0 & PageTableEntry::X) != 0,
                });
            }
        }
    }

    Ok(mappings)
}

/// root 지정 유저 페이지 물리 주소 조회
pub fn get_user_page_phys_for_root(root: usize, virt_addr: usize) -> Result<usize, &'static str> {
    let (_entry_ptr, current) = resolve_user_l0_entry_mut(root, virt_addr)?;
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
    let (entry_ptr, current) = resolve_user_l0_entry_mut(root, virt_addr)?;
    unsafe {
        // SAFETY: resolve_user_l0_entry_mut에서 유효한 엔트리 포인터를 보장한다.
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
    let (entry_ptr, current) = resolve_user_l0_entry_mut(root, virt_addr)?;
    let phys = current.addr();
    unsafe {
        // SAFETY: resolve_user_l0_entry_mut에서 유효한 엔트리 포인터를 보장한다.
        *entry_ptr = PageTableEntry::new_page(phys >> 12, PageFlags::user_from_segment(write, execute));
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
    let (entry_ptr, current) = resolve_user_l0_entry_mut(root, virt_addr)?;
    unsafe {
        // SAFETY: resolve_user_l0_entry_mut에서 유효한 L0 엔트리 포인터를 보장한다.
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
    let (entry_ptr, current) = resolve_user_l0_entry_mut(root, virt_addr)?;
    let phys = current.addr();
    unsafe {
        // SAFETY: resolve_user_l0_entry_mut에서 유효한 L0 엔트리 포인터를 보장한다.
        *entry_ptr = PageTableEntry::new_page(phys >> 12, PageFlags::user_from_segment(write, execute));
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
