//! QEMU virt 보드 설정 (RISC-V64) - 멀티코어 버전
//!
//! QEMU의 virt 머신 타입에 대한 하드웨어 설정입니다.
//! `-smp N` 플래그로 여러 hart를 지원합니다.
//!
//! 참고: https://www.qemu.org/docs/master/system/riscv/virt.html

use super::board_module::{uart_quirks, BoardModuleInfo};
use super::BoardConfig;

/// QEMU virt 보드 (RISC-V64 SMP)
pub struct QemuVirtRiscv64Smp;

impl BoardConfig for QemuVirtRiscv64Smp {
    // UART: NS16550A 호환
    const UART_BASE: usize = 0x1000_0000;
    const UART_IRQ: u32 = 10; // PLIC IRQ #10
    const UART_CLOCK_FREQ: u32 = 3_686_400; // 3.6864MHz

    // Timer: CLINT (Core Local Interruptor)
    const TIMER_FREQ: u64 = 10_000_000; // 10MHz
    const TIMER_IRQ: u32 = 0; // CLINT은 IRQ가 아닌 CSR로 처리

    // PLIC (Platform-Level Interrupt Controller)
    const PLIC_BASE: usize = 0x0C00_0000;

    // CLINT (Core Local Interruptor)
    const CLINT_BASE: usize = 0x0200_0000;

    // Memory
    const RAM_BASE: usize = 0x8000_0000; // 2GB
    const RAM_SIZE: usize = 128 * 1024 * 1024; // 128MB (기본값)
}

/// SBI (Supervisor Binary Interface) HSM (Hart State Management) 확장
///
/// RISC-V에서 hart 전원 관리를 위한 표준 인터페이스입니다.
/// OpenSBI가 이를 구현합니다.
pub mod sbi_hsm {
    /// SBI 확장 ID
    pub const EXT_HSM: usize = 0x48534D; // "HSM"

    /// HSM 함수 ID
    pub const HART_START: usize = 0;
    pub const HART_STOP: usize = 1;
    pub const HART_GET_STATUS: usize = 2;
    pub const HART_SUSPEND: usize = 3;

    /// Hart 상태
    pub mod state {
        pub const STARTED: usize = 0;
        pub const STOPPED: usize = 1;
        pub const START_PENDING: usize = 2;
        pub const STOP_PENDING: usize = 3;
        pub const SUSPENDED: usize = 4;
        pub const SUSPEND_PENDING: usize = 5;
        pub const RESUME_PENDING: usize = 6;
    }

    /// SBI 에러 코드
    pub mod error {
        pub const SUCCESS: isize = 0;
        pub const FAILED: isize = -1;
        pub const NOT_SUPPORTED: isize = -2;
        pub const INVALID_PARAM: isize = -3;
        pub const DENIED: isize = -4;
        pub const INVALID_ADDRESS: isize = -5;
        pub const ALREADY_AVAILABLE: isize = -6;
        pub const ALREADY_STARTED: isize = -7;
        pub const ALREADY_STOPPED: isize = -8;
    }
}

/// SBI Base 확장
pub mod sbi_base {
    /// SBI 확장 ID
    pub const EXT_BASE: usize = 0x10;

    /// Base 함수 ID
    pub const GET_SPEC_VERSION: usize = 0;
    pub const GET_IMPL_ID: usize = 1;
    pub const GET_IMPL_VERSION: usize = 2;
    pub const PROBE_EXTENSION: usize = 3;
    pub const GET_MVENDORID: usize = 4;
    pub const GET_MARCHID: usize = 5;
    pub const GET_MIMPID: usize = 6;
}

/// 보드 초기화 함수
fn board_init() -> Result<(), i32> {
    crate::kprintln!("[board] QEMU virt RISC-V64 SMP initialized");

    // SBI 버전 확인
    let version = sbi_get_spec_version();
    let major = (version >> 24) & 0x7F;
    let minor = version & 0xFFFFFF;
    crate::kprintln!("[board] SBI spec version: {}.{}", major, minor);

    // HSM 확장 지원 여부 확인
    if sbi_probe_extension(sbi_hsm::EXT_HSM) != 0 {
        crate::kprintln!("[board] SBI HSM extension available");
    } else {
        crate::kprintln!("[board] SBI HSM extension NOT available");
    }

    Ok(())
}

/// 보드 모듈 정보
pub static BOARD_INFO: BoardModuleInfo = BoardModuleInfo {
    compatible: &["riscv-virtio", "qemu,virt"],
    name: "QEMU virt (RISC-V64 SMP)",
    timer_freq: 10_000_000, // 10MHz
    uart_quirks: uart_quirks::NONE,
    init_fn: board_init,
    early_init_fn: None,
    platform_config_fn: None,
    cpu_count: 0, // DTB에서 읽음
    smp_capable: true,
};

// ============================================================================
// SBI 인터페이스
// ============================================================================

/// SBI 호출 결과
#[derive(Debug, Clone, Copy)]
pub struct SbiResult {
    pub error: isize,
    pub value: usize,
}

/// SBI ecall 호출
#[inline(always)]
fn sbi_call(ext: usize, func: usize, arg0: usize, arg1: usize, arg2: usize) -> SbiResult {
    let error: isize;
    let value: usize;
    unsafe {
        core::arch::asm!(
            "ecall",
            in("a7") ext,
            in("a6") func,
            inout("a0") arg0 => error,
            inout("a1") arg1 => value,
            in("a2") arg2,
            options(nomem, nostack)
        );
    }
    SbiResult { error, value }
}

/// SBI 스펙 버전 조회
pub fn sbi_get_spec_version() -> usize {
    sbi_call(sbi_base::EXT_BASE, sbi_base::GET_SPEC_VERSION, 0, 0, 0).value
}

/// SBI 확장 지원 여부 확인
pub fn sbi_probe_extension(ext_id: usize) -> usize {
    sbi_call(sbi_base::EXT_BASE, sbi_base::PROBE_EXTENSION, ext_id, 0, 0).value
}

// ============================================================================
// HSM (Hart State Management) 인터페이스
// ============================================================================

/// 보조 hart 시작
///
/// # Arguments
/// * `hartid` - 시작할 hart의 ID
/// * `start_addr` - hart가 시작할 물리 주소
/// * `opaque` - hart에 전달할 값 (a1 레지스터로 전달됨)
///
/// # Returns
/// SBI 에러 코드 (0 = 성공)
pub fn hart_start(hartid: usize, start_addr: usize, opaque: usize) -> isize {
    sbi_call(sbi_hsm::EXT_HSM, sbi_hsm::HART_START, hartid, start_addr, opaque).error
}

/// 현재 hart 종료
///
/// 이 함수는 반환하지 않습니다.
pub fn hart_stop() -> ! {
    sbi_call(sbi_hsm::EXT_HSM, sbi_hsm::HART_STOP, 0, 0, 0);
    // 반환하지 않아야 하지만, 안전을 위해 무한 루프
    loop {
        unsafe {
            core::arch::asm!("wfi");
        }
    }
}

/// hart 상태 조회
///
/// # Arguments
/// * `hartid` - 조회할 hart의 ID
///
/// # Returns
/// hart 상태 (sbi_hsm::state 참조)
pub fn hart_get_status(hartid: usize) -> isize {
    let result = sbi_call(sbi_hsm::EXT_HSM, sbi_hsm::HART_GET_STATUS, hartid, 0, 0);
    if result.error == 0 {
        result.value as isize
    } else {
        result.error
    }
}

/// hart 일시 중지
///
/// # Arguments
/// * `suspend_type` - 중지 유형 (0 = retentive, 0x80000000 = non-retentive)
/// * `resume_addr` - 재개 시 시작할 주소 (non-retentive의 경우)
/// * `opaque` - 재개 시 전달할 값
pub fn hart_suspend(suspend_type: u32, resume_addr: usize, opaque: usize) -> isize {
    sbi_call(
        sbi_hsm::EXT_HSM,
        sbi_hsm::HART_SUSPEND,
        suspend_type as usize,
        resume_addr,
        opaque,
    )
    .error
}

// ============================================================================
// 보조 hart 부팅
// ============================================================================

/// 보조 hart 시작 진입점
///
/// 이 함수는 hart_start로 시작된 보조 hart가 처음 실행하는 코드입니다.
/// opaque 값을 a1으로 받습니다.
#[unsafe(no_mangle)]
pub extern "C" fn secondary_hart_entry(hartid: usize, _opaque: usize) -> ! {
    let cpu_id = hartid as u32;

    // 1. 스택 설정 (primary hart가 미리 할당해둔 스택 사용)
    let stack_top = crate::proc::percpu::stacks::get_stack_top(cpu_id);
    if stack_top != 0 {
        unsafe {
            core::arch::asm!(
                "mv sp, {0}",
                in(reg) stack_top,
            );
        }
    }

    // 2. 트랩 핸들러 설정
    crate::arch::trap::init();

    // 3. PLIC per-hart context 초기화
    crate::arch::plic::init_secondary(cpu_id);

    // 4. 타이머 초기화
    crate::arch::timer::init_secondary();

    // 5. Per-CPU 데이터 초기화 및 온라인 표시
    crate::proc::percpu::init_secondary(cpu_id);

    // 6. 이 hart의 idle 스레드 생성 (스케줄러에서 사용)
    crate::proc::init_on_secondary_cpu(cpu_id);

    crate::kprintln!("[smp] Hart {} online", cpu_id);

    // 7. 인터럽트 활성화
    unsafe {
        core::arch::asm!(
            "li t0, 0x8",      // MIE (Machine Interrupt Enable)
            "csrs mstatus, t0"
        );
    }

    // 8. idle 루프 (타이머 인터럽트가 스케줄러를 호출)
    loop {
        unsafe {
            core::arch::asm!("wfi");
        }
    }
}

/// 모든 보조 hart 시작
///
/// DTB에서 읽은 hart 수만큼 보조 hart를 시작합니다.
#[allow(dead_code)]
pub fn start_secondary_harts(hart_count: usize, entry_point: usize) {
    crate::kprintln!("[smp] Starting {} secondary harts", hart_count - 1);

    // hart 0은 부트 hart이므로 건너뜀
    for hartid in 1..hart_count {
        match hart_start(hartid, entry_point, hartid) {
            0 => {
                crate::kprintln!("[smp] Hart {} started", hartid);
            }
            sbi_hsm::error::ALREADY_STARTED => {
                crate::kprintln!("[smp] Hart {} already started", hartid);
            }
            err => {
                crate::kprintln!("[smp] Failed to start hart {}: error {}", hartid, err);
            }
        }
    }
}

/// 모든 hart 상태 출력
#[allow(dead_code)]
pub fn print_hart_status(hart_count: usize) {
    crate::kprintln!("[smp] Hart status:");
    for hartid in 0..hart_count {
        let status = hart_get_status(hartid);
        let status_str = match status as usize {
            sbi_hsm::state::STARTED => "started",
            sbi_hsm::state::STOPPED => "stopped",
            sbi_hsm::state::START_PENDING => "start_pending",
            sbi_hsm::state::STOP_PENDING => "stop_pending",
            sbi_hsm::state::SUSPENDED => "suspended",
            sbi_hsm::state::SUSPEND_PENDING => "suspend_pending",
            sbi_hsm::state::RESUME_PENDING => "resume_pending",
            _ => "unknown",
        };
        crate::kprintln!("  Hart {}: {}", hartid, status_str);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_qemu_virt_smp_config() {
        assert_eq!(QemuVirtRiscv64Smp::UART_BASE, 0x1000_0000);
        assert_eq!(QemuVirtRiscv64Smp::PLIC_BASE, 0x0C00_0000);
        assert_eq!(QemuVirtRiscv64Smp::CLINT_BASE, 0x0200_0000);
        assert_eq!(QemuVirtRiscv64Smp::RAM_BASE, 0x8000_0000);
        assert!(BOARD_INFO.smp_capable);
    }
}
