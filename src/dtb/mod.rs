//! Device Tree Blob (DTB) Parser
//!
//! Flattened Device Tree (FDT) 포맷 파서 구현
//! 외부 crate 없이 직접 구현

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::str;

/// FDT 매직 넘버 (big endian)
const FDT_MAGIC: u32 = 0xd00dfeed;

/// Structure Block 토큰
const FDT_BEGIN_NODE: u32 = 0x00000001;
const FDT_END_NODE: u32 = 0x00000002;
const FDT_PROP: u32 = 0x00000003;
const FDT_NOP: u32 = 0x00000004;
const FDT_END: u32 = 0x00000009;

/// FDT 헤더 구조체 (40 bytes)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FdtHeader {
    /// 매직 넘버: 0xd00dfeed
    pub magic: u32,
    /// 전체 DTB 크기
    pub totalsize: u32,
    /// Structure Block 오프셋
    pub off_dt_struct: u32,
    /// Strings Block 오프셋
    pub off_dt_strings: u32,
    /// Memory Reservation Block 오프셋
    pub off_mem_rsvmap: u32,
    /// DTB 버전
    pub version: u32,
    /// 하위 호환 버전
    pub last_comp_version: u32,
    /// 부트 CPU ID (v2+)
    pub boot_cpuid_phys: u32,
    /// Strings Block 크기 (v3+)
    pub size_dt_strings: u32,
    /// Structure Block 크기 (v17+)
    pub size_dt_struct: u32,
}

/// 메모리 영역 정보
#[derive(Debug, Clone, Copy)]
pub struct MemoryRegion {
    pub base: u64,
    pub size: u64,
}

/// 디바이스 정보 (드라이버에 전달)
#[derive(Debug, Clone)]
pub struct DeviceInfo {
    /// 노드 이름 (예: "uart@9000000")
    pub name: alloc::string::String,
    /// MMIO 기본 주소
    pub reg_base: u64,
    /// MMIO 크기
    pub reg_size: u64,
    /// 추가 reg 영역들 (GIC처럼 여러 영역이 있는 경우)
    pub reg_extra: alloc::vec::Vec<(u64, u64)>,
    /// 인터럽트 번호들
    pub interrupts: alloc::vec::Vec<u32>,
    /// compatible 문자열
    pub compatible: alloc::string::String,
    /// clock-frequency (있는 경우)
    pub clock_frequency: Option<u32>,
}

impl DeviceInfo {
    /// 빈 DeviceInfo 생성
    pub fn new(name: &str) -> Self {
        Self {
            name: alloc::string::String::from(name),
            reg_base: 0,
            reg_size: 0,
            reg_extra: alloc::vec::Vec::new(),
            interrupts: alloc::vec::Vec::new(),
            compatible: alloc::string::String::new(),
            clock_frequency: None,
        }
    }
}

/// DTB 파싱 결과
#[derive(Debug, Clone, Copy)]
pub struct DeviceTree {
    /// DTB 시작 주소
    base: usize,
    /// 헤더 정보
    header: FdtHeader,
}

/// DTB 파싱 에러
#[derive(Debug, Clone, Copy)]
pub enum DtbError {
    InvalidMagic,
    InvalidVersion,
    NodeNotFound,
}

impl DeviceTree {
    /// DTB 주소에서 DeviceTree 생성
    ///
    /// # Safety
    /// dtb_addr은 유효한 DTB 주소여야 함
    pub unsafe fn from_addr(dtb_addr: usize) -> Result<Self, DtbError> {
        if dtb_addr == 0 {
            return Err(DtbError::InvalidMagic);
        }

        let header = unsafe { Self::read_header(dtb_addr)? };

        Ok(Self {
            base: dtb_addr,
            header,
        })
    }

    /// 헤더 읽기
    unsafe fn read_header(base: usize) -> Result<FdtHeader, DtbError> {
        let ptr = base as *const u32;

        unsafe {
            let magic = u32::from_be(ptr.read_volatile());
            if magic != FDT_MAGIC {
                return Err(DtbError::InvalidMagic);
            }

            let header = FdtHeader {
                magic,
                totalsize: u32::from_be(ptr.add(1).read_volatile()),
                off_dt_struct: u32::from_be(ptr.add(2).read_volatile()),
                off_dt_strings: u32::from_be(ptr.add(3).read_volatile()),
                off_mem_rsvmap: u32::from_be(ptr.add(4).read_volatile()),
                version: u32::from_be(ptr.add(5).read_volatile()),
                last_comp_version: u32::from_be(ptr.add(6).read_volatile()),
                boot_cpuid_phys: u32::from_be(ptr.add(7).read_volatile()),
                size_dt_strings: u32::from_be(ptr.add(8).read_volatile()),
                size_dt_struct: u32::from_be(ptr.add(9).read_volatile()),
            };

            if header.version < 16 {
                return Err(DtbError::InvalidVersion);
            }

            Ok(header)
        }
    }

    /// Structure Block 시작 주소
    fn struct_base(&self) -> usize {
        self.base + self.header.off_dt_struct as usize
    }

    /// Strings Block 시작 주소
    fn strings_base(&self) -> usize {
        self.base + self.header.off_dt_strings as usize
    }

    /// Strings Block에서 문자열 읽기
    unsafe fn get_string(&self, offset: u32) -> &str {
        unsafe {
            let ptr = (self.strings_base() + offset as usize) as *const u8;
            let mut len = 0;
            while *ptr.add(len) != 0 {
                len += 1;
            }
            let slice = core::slice::from_raw_parts(ptr, len);
            str::from_utf8_unchecked(slice)
        }
    }

    /// 4바이트 정렬
    fn align4(offset: usize) -> usize {
        (offset + 3) & !3
    }

    /// 메모리 영역 찾기 (/memory 노드의 reg 프로퍼티)
    pub fn get_memory(&self) -> Result<MemoryRegion, DtbError> {
        unsafe { self.find_memory_region() }
    }

    /// /memory 노드에서 메모리 정보 추출
    unsafe fn find_memory_region(&self) -> Result<MemoryRegion, DtbError> {
        unsafe {
            let mut offset = 0usize;
            let struct_base = self.struct_base();
            let mut in_memory_node = false;
            let mut address_cells: u32 = 2; // 기본값
            let mut size_cells: u32 = 1; // 기본값
            let mut depth = 0;

            loop {
                let token_ptr = (struct_base + offset) as *const u32;
                let token = u32::from_be(token_ptr.read_volatile());
                offset += 4;

                match token {
                    FDT_BEGIN_NODE => {
                        // 노드 이름 읽기
                        let name_ptr = (struct_base + offset) as *const u8;
                        let name = self.read_cstring(name_ptr);
                        let name_len = name.len() + 1; // null terminator 포함
                        offset = Self::align4(offset + name_len);

                        // memory 또는 memory@xxxx 노드 확인
                        if depth == 1 && (name == "memory" || name.starts_with("memory@")) {
                            in_memory_node = true;
                        }
                        depth += 1;
                    }
                    FDT_END_NODE => {
                        depth -= 1;
                        if depth == 1 {
                            in_memory_node = false;
                        }
                    }
                    FDT_PROP => {
                        let len =
                            u32::from_be(((struct_base + offset) as *const u32).read_volatile());
                        offset += 4;
                        let nameoff =
                            u32::from_be(((struct_base + offset) as *const u32).read_volatile());
                        offset += 4;

                        let prop_name = self.get_string(nameoff);
                        let prop_data = (struct_base + offset) as *const u8;

                        // 루트 노드에서 #address-cells, #size-cells 읽기
                        if depth == 1 {
                            if prop_name == "#address-cells" && len == 4 {
                                address_cells =
                                    u32::from_be((prop_data as *const u32).read_volatile());
                            } else if prop_name == "#size-cells" && len == 4 {
                                size_cells =
                                    u32::from_be((prop_data as *const u32).read_volatile());
                            }
                        }

                        // memory 노드의 reg 프로퍼티
                        if in_memory_node && prop_name == "reg" {
                            let base = self.read_cells(prop_data, address_cells);
                            let size = self.read_cells(
                                prop_data.add((address_cells * 4) as usize),
                                size_cells,
                            );
                            return Ok(MemoryRegion { base, size });
                        }

                        offset = Self::align4(offset + len as usize);
                    }
                    FDT_NOP => {
                        // 무시
                    }
                    FDT_END => {
                        break;
                    }
                    _ => {
                        // 알 수 없는 토큰
                        break;
                    }
                }
            }

            Err(DtbError::NodeNotFound)
        }
    }

    /// 지정된 셀 수만큼 값 읽기 (big endian)
    unsafe fn read_cells(&self, ptr: *const u8, cells: u32) -> u64 {
        unsafe {
            match cells {
                1 => u32::from_be((ptr as *const u32).read_volatile()) as u64,
                2 => {
                    let high = u32::from_be((ptr as *const u32).read_volatile()) as u64;
                    let low = u32::from_be((ptr as *const u32).add(1).read_volatile()) as u64;
                    (high << 32) | low
                }
                _ => 0,
            }
        }
    }

    /// null-terminated 문자열 읽기
    unsafe fn read_cstring(&self, ptr: *const u8) -> &str {
        unsafe {
            let mut len = 0;
            while *ptr.add(len) != 0 {
                len += 1;
            }
            let slice = core::slice::from_raw_parts(ptr, len);
            str::from_utf8_unchecked(slice)
        }
    }

    /// DTB 정보 출력 (디버깅용)
    pub fn dump_info(&self) {
        crate::kprintln!("[DTB] Device Tree Info:");
        crate::kprintln!("  Base address: {:#x}", self.base);
        crate::kprintln!("  Total size: {} bytes", self.header.totalsize);
        crate::kprintln!("  Version: {}", self.header.version);
        crate::kprintln!("  Structure block: offset={:#x}", self.header.off_dt_struct);
        crate::kprintln!("  Strings block: offset={:#x}", self.header.off_dt_strings);
    }

    /// compatible 문자열로 디바이스 찾기
    ///
    /// DTB를 순회하며 지정된 compatible 문자열을 가진 모든 노드를 찾아 반환
    pub fn find_compatible(&self, target_compatible: &str) -> Vec<DeviceInfo> {
        let mut devices = Vec::new();
        unsafe {
            self.scan_nodes(|info| {
                // compatible 문자열이 일치하거나 포함되는지 확인
                // compatible은 여러 개일 수 있음 (null로 구분)
                for compat in info.compatible.split('\0') {
                    if compat == target_compatible {
                        devices.push(info.clone());
                        break;
                    }
                }
            });
        }
        devices
    }

    /// 모든 디바이스 노드 스캔
    ///
    /// # Safety
    /// DTB 메모리가 유효해야 함
    unsafe fn scan_nodes<F>(&self, mut callback: F)
    where
        F: FnMut(&DeviceInfo),
    {
        unsafe {
            let struct_base = self.struct_base();
            let mut offset = 0usize;
            let mut depth = 0i32;

            // 현재 노드 스택 (이름, 시작 오프셋)
            let mut node_stack: Vec<(String, usize)> = Vec::new();

            // 루트의 #address-cells, #size-cells
            let mut root_address_cells: u32 = 2;
            let mut root_size_cells: u32 = 1;

            // 현재 파싱 중인 노드 정보
            let mut current_info: Option<DeviceInfo> = None;
            let mut current_address_cells: u32 = 2;
            let mut current_size_cells: u32 = 1;

            loop {
                let token_ptr = (struct_base + offset) as *const u32;
                let token = u32::from_be(token_ptr.read_volatile());
                offset += 4;

                match token {
                    FDT_BEGIN_NODE => {
                        let name_ptr = (struct_base + offset) as *const u8;
                        let name = self.read_cstring(name_ptr);
                        let name_len = name.len() + 1;
                        offset = Self::align4(offset + name_len);

                        // 루트 노드가 아닌 경우에만 처리
                        if depth > 0 {
                            current_info = Some(DeviceInfo::new(name));
                            current_address_cells = root_address_cells;
                            current_size_cells = root_size_cells;
                        }

                        node_stack.push((String::from(name), offset));
                        depth += 1;
                    }
                    FDT_END_NODE => {
                        // 노드 종료 시 콜백 호출
                        if let Some(info) = current_info.take() {
                            // reg가 있는 디바이스 노드만 콜백
                            if info.reg_base != 0 || !info.compatible.is_empty() {
                                callback(&info);
                            }
                        }

                        depth -= 1;
                        node_stack.pop();
                    }
                    FDT_PROP => {
                        let len =
                            u32::from_be(((struct_base + offset) as *const u32).read_volatile());
                        offset += 4;
                        let nameoff =
                            u32::from_be(((struct_base + offset) as *const u32).read_volatile());
                        offset += 4;

                        let prop_name = self.get_string(nameoff);
                        let prop_data = (struct_base + offset) as *const u8;

                        // 루트 노드의 #address-cells, #size-cells
                        if depth == 1 {
                            if prop_name == "#address-cells" && len == 4 {
                                root_address_cells =
                                    u32::from_be((prop_data as *const u32).read_volatile());
                                current_address_cells = root_address_cells;
                            } else if prop_name == "#size-cells" && len == 4 {
                                root_size_cells =
                                    u32::from_be((prop_data as *const u32).read_volatile());
                                current_size_cells = root_size_cells;
                            }
                        }

                        // 현재 노드의 프로퍼티 파싱
                        if let Some(ref mut info) = current_info {
                            match prop_name {
                                "compatible" => {
                                    // compatible 문자열들 (null 구분)
                                    let slice =
                                        core::slice::from_raw_parts(prop_data, len as usize);
                                    if let Ok(s) = core::str::from_utf8(slice) {
                                        info.compatible = String::from(s.trim_end_matches('\0'));
                                    }
                                }
                                "reg" => {
                                    // reg 프로퍼티: (base, size) 쌍
                                    let entry_size =
                                        (current_address_cells + current_size_cells) as usize * 4;
                                    let num_entries = len as usize / entry_size;

                                    for i in 0..num_entries {
                                        let entry_ptr = prop_data.add(i * entry_size);
                                        let base =
                                            self.read_cells(entry_ptr, current_address_cells);
                                        let size = self.read_cells(
                                            entry_ptr.add(current_address_cells as usize * 4),
                                            current_size_cells,
                                        );

                                        if i == 0 {
                                            info.reg_base = base;
                                            info.reg_size = size;
                                        } else {
                                            info.reg_extra.push((base, size));
                                        }
                                    }
                                }
                                "interrupts" => {
                                    // 인터럽트 번호들
                                    let num_ints = len as usize / 4;
                                    for i in 0..num_ints {
                                        let irq = u32::from_be(
                                            (prop_data.add(i * 4) as *const u32).read_volatile(),
                                        );
                                        info.interrupts.push(irq);
                                    }
                                }
                                "clock-frequency" => {
                                    if len == 4 {
                                        info.clock_frequency = Some(u32::from_be(
                                            (prop_data as *const u32).read_volatile(),
                                        ));
                                    }
                                }
                                "#address-cells" => {
                                    if len == 4 {
                                        current_address_cells =
                                            u32::from_be((prop_data as *const u32).read_volatile());
                                    }
                                }
                                "#size-cells" => {
                                    if len == 4 {
                                        current_size_cells =
                                            u32::from_be((prop_data as *const u32).read_volatile());
                                    }
                                }
                                _ => {}
                            }
                        }

                        offset = Self::align4(offset + len as usize);
                    }
                    FDT_NOP => {}
                    FDT_END => break,
                    _ => break,
                }
            }
        }
    }

    /// 모든 디바이스 노드 정보 출력 (디버깅용)
    pub fn dump_devices(&self) {
        crate::kprintln!("[DTB] Scanning all devices...");
        unsafe {
            self.scan_nodes(|info| {
                crate::kprintln!(
                    "  {} @ {:#x} (size={:#x})",
                    info.name,
                    info.reg_base,
                    info.reg_size
                );
                if !info.compatible.is_empty() {
                    crate::kprintln!("    compatible: {}", info.compatible.replace('\0', ", "));
                }
                if !info.interrupts.is_empty() {
                    crate::kprintln!("    interrupts: {:?}", info.interrupts);
                }
            });
        }
    }

    // =========================================================================
    // 디바이스별 탐색 헬퍼 함수들
    // =========================================================================

    /// GIC (Generic Interrupt Controller) 찾기 - AArch64
    ///
    /// GICv2/v3의 GICD, GICC (또는 GICR) 주소를 반환
    pub fn find_gic(&self) -> Option<GicInfo> {
        // GIC compatible 문자열들
        const GIC_COMPATIBLES: &[&str] = &[
            "arm,gic-400",
            "arm,cortex-a15-gic",
            "arm,gic-v3",
        ];

        for compat in GIC_COMPATIBLES {
            let devices = self.find_compatible(compat);
            if let Some(info) = devices.into_iter().next() {
                let version = if compat.contains("v3") {
                    GicVersion::V3
                } else {
                    GicVersion::V2
                };

                // GIC는 여러 개의 reg 영역을 가짐
                // GICv2: [0]=GICD, [1]=GICC
                // GICv3: [0]=GICD, [1]=GICR
                let cpu_interface_base = info.reg_extra.first().map(|(b, _)| *b).unwrap_or(0);
                let redistributor_base = if version == GicVersion::V3 {
                    info.reg_extra.get(1).map(|(b, _)| *b)
                } else {
                    None
                };

                return Some(GicInfo {
                    distributor_base: info.reg_base,
                    cpu_interface_base,
                    redistributor_base,
                    version,
                });
            }
        }
        None
    }

    /// PLIC (Platform-Level Interrupt Controller) 찾기 - RISC-V
    pub fn find_plic(&self) -> Option<PlicInfo> {
        const PLIC_COMPATIBLES: &[&str] = &[
            "riscv,plic0",
            "sifive,plic-1.0.0",
        ];

        for compat in PLIC_COMPATIBLES {
            let devices = self.find_compatible(compat);
            if let Some(info) = devices.into_iter().next() {
                return Some(PlicInfo {
                    base: info.reg_base,
                    size: info.reg_size,
                });
            }
        }
        None
    }

    /// CLINT (Core Local Interruptor) 찾기 - RISC-V
    pub fn find_clint(&self) -> Option<ClintInfo> {
        const CLINT_COMPATIBLES: &[&str] = &[
            "riscv,clint0",
            "sifive,clint0",
        ];

        for compat in CLINT_COMPATIBLES {
            let devices = self.find_compatible(compat);
            if let Some(info) = devices.into_iter().next() {
                return Some(ClintInfo {
                    base: info.reg_base,
                    size: info.reg_size,
                });
            }
        }
        None
    }

    /// UART 찾기
    ///
    /// PL011 (ARM) 또는 NS16550A (일반) UART를 찾음
    pub fn find_uart(&self) -> Option<UartInfo> {
        const UART_COMPATIBLES: &[&str] = &[
            "arm,pl011",
            "arm,primecell",
            "ns16550a",
            "ns16550",
            "snps,dw-apb-uart",
        ];

        for compat in UART_COMPATIBLES {
            let devices = self.find_compatible(compat);
            if let Some(info) = devices.into_iter().next() {
                // 첫 번째 인터럽트가 IRQ 번호
                let irq = info.interrupts.first().copied().unwrap_or(0);

                return Some(UartInfo {
                    base: info.reg_base,
                    size: info.reg_size,
                    irq,
                    clock_freq: info.clock_frequency.unwrap_or(0),
                });
            }
        }
        None
    }

    /// CPU 개수 세기
    ///
    /// /cpus 노드 아래의 cpu 노드 개수를 반환
    pub fn count_cpus(&self) -> usize {
        let mut cpu_count = 0;
        unsafe {
            self.scan_nodes(|info| {
                // "cpu@0", "cpu@1" 등의 노드를 찾음
                if info.name.starts_with("cpu@") {
                    cpu_count += 1;
                }
            });
        }
        // 최소 1개
        if cpu_count == 0 {
            1
        } else {
            cpu_count
        }
    }

    /// 루트 노드의 compatible 속성 읽기
    ///
    /// 보드 식별에 사용됩니다. 예: ["linux,dummy-virt", "qemu,virt"]
    pub fn get_root_compatible(&self) -> Vec<String> {
        let mut compatibles = Vec::new();
        unsafe {
            self.scan_root_compatible(&mut compatibles);
        }
        compatibles
    }

    /// 루트 노드의 compatible 속성 스캔
    unsafe fn scan_root_compatible(&self, result: &mut Vec<String>) {
        unsafe {
            let struct_base = self.struct_base();
            let mut offset = 0usize;
            let mut depth = 0i32;

            loop {
                let token_ptr = (struct_base + offset) as *const u32;
                let token = u32::from_be(token_ptr.read_volatile());
                offset += 4;

                match token {
                    FDT_BEGIN_NODE => {
                        let name_ptr = (struct_base + offset) as *const u8;
                        let name = self.read_cstring(name_ptr);
                        let name_len = name.len() + 1;
                        offset = Self::align4(offset + name_len);
                        depth += 1;
                    }
                    FDT_END_NODE => {
                        depth -= 1;
                        if depth == 0 {
                            // 루트 노드 끝 - 탐색 종료
                            return;
                        }
                    }
                    FDT_PROP => {
                        let len =
                            u32::from_be(((struct_base + offset) as *const u32).read_volatile());
                        offset += 4;
                        let nameoff =
                            u32::from_be(((struct_base + offset) as *const u32).read_volatile());
                        offset += 4;

                        let prop_name = self.get_string(nameoff);
                        let prop_data = (struct_base + offset) as *const u8;

                        // 루트 노드(depth==1)의 compatible 속성 찾기
                        if depth == 1 && prop_name == "compatible" {
                            let slice = core::slice::from_raw_parts(prop_data, len as usize);
                            if let Ok(s) = core::str::from_utf8(slice) {
                                // null로 구분된 문자열들을 분리
                                for compat in s.split('\0') {
                                    if !compat.is_empty() {
                                        result.push(String::from(compat));
                                    }
                                }
                            }
                            return; // compatible 찾았으므로 종료
                        }

                        offset = Self::align4(offset + len as usize);
                    }
                    FDT_NOP => {}
                    FDT_END => return,
                    _ => return,
                }
            }
        }
    }
}

// =========================================================================
// DTB 탐색 결과 타입들
// =========================================================================

/// GIC 정보
#[derive(Debug, Clone)]
pub struct GicInfo {
    pub distributor_base: u64,
    pub cpu_interface_base: u64,
    pub redistributor_base: Option<u64>,
    pub version: GicVersion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GicVersion {
    V2,
    V3,
}

/// PLIC 정보
#[derive(Debug, Clone)]
pub struct PlicInfo {
    pub base: u64,
    pub size: u64,
}

/// CLINT 정보
#[derive(Debug, Clone)]
pub struct ClintInfo {
    pub base: u64,
    pub size: u64,
}

/// UART 정보
#[derive(Debug, Clone)]
pub struct UartInfo {
    pub base: u64,
    pub size: u64,
    pub irq: u32,
    pub clock_freq: u32,
}

/// 전역 DTB 저장소 (내부 가변성 사용)
struct DtbHolder {
    inner: UnsafeCell<Option<DeviceTree>>,
}

unsafe impl Sync for DtbHolder {}

static DTB_HOLDER: DtbHolder = DtbHolder {
    inner: UnsafeCell::new(None),
};

/// DTB 초기화
///
/// # Safety
/// 부팅 초기에 한 번만 호출되어야 함
pub unsafe fn init(dtb_addr: usize) -> Result<(), DtbError> {
    let dt = unsafe { DeviceTree::from_addr(dtb_addr)? };
    unsafe {
        *DTB_HOLDER.inner.get() = Some(dt);
    }
    Ok(())
}

/// 메모리 스캔으로 DTB 찾기 및 초기화
/// QEMU에서 DTB 주소가 레지스터로 전달되지 않을 때 사용
///
/// # Safety
/// 부팅 초기에 한 번만 호출되어야 함
pub unsafe fn init_scan(ram_start: usize, ram_size: usize) -> Result<(), DtbError> {
    // QEMU는 DTB를 RAM 끝 - 2MB 위치에 배치함
    // 일반적인 메모리 크기들을 시도해보고, 그 다음 스캔
    const DTB_OFFSET_FROM_END: usize = 2 * 1024 * 1024; // 2MB

    // 일반적인 메모리 크기들 (64MB ~ 4GB)
    let common_sizes: &[usize] = &[
        64 * 1024 * 1024,   // 64MB
        128 * 1024 * 1024,  // 128MB
        256 * 1024 * 1024,  // 256MB
        512 * 1024 * 1024,  // 512MB
        1024 * 1024 * 1024, // 1GB
        2048 * 1024 * 1024, // 2GB
    ];

    // 각 일반적인 메모리 크기에 대해 DTB 위치 확인
    for &size in common_sizes {
        if size > ram_size {
            continue; // 요청된 최대 RAM 크기보다 큰 것은 건너뛰기
        }

        let dtb_addr = ram_start + size - DTB_OFFSET_FROM_END;
        let ptr = dtb_addr as *const u32;
        let magic = unsafe { u32::from_be(ptr.read_volatile()) };

        if magic == FDT_MAGIC {
            crate::kprintln!(
                "[DTB] Found DTB at {:#x} (common size {}MB)",
                dtb_addr,
                size / (1024 * 1024)
            );
            return unsafe { init(dtb_addr) };
        }
    }

    // 커널 로드 주소 근처 스캔 (RAM 시작부터 512KB)
    let forward_scan_size = core::cmp::min(ram_size, 0x80000);
    for offset in (0..forward_scan_size).step_by(0x1000) {
        // 4KB 단위
        let addr = ram_start + offset;
        let ptr = addr as *const u32;
        let magic = unsafe { u32::from_be(ptr.read_volatile()) };

        if magic == FDT_MAGIC {
            crate::kprintln!("[DTB] Found DTB at {:#x} (forward scan)", addr);
            return unsafe { init(addr) };
        }
    }

    crate::kprintln!("[DTB] DTB not found in common locations");
    Err(DtbError::InvalidMagic)
}

/// 전역 DTB 참조 얻기
pub fn get() -> Option<&'static DeviceTree> {
    unsafe { (*DTB_HOLDER.inner.get()).as_ref() }
}
