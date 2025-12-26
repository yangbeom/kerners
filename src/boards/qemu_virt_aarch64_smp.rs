//! QEMU virt 보드 설정 (AArch64) - 멀티코어 버전
//!
//! QEMU의 virt 머신 타입에 대한 하드웨어 설정입니다.
//! `-smp N` 플래그로 여러 CPU를 지원합니다.
//!
//! 참고: https://www.qemu.org/docs/master/system/arm/virt.html

use super::board_module::{uart_quirks, BoardModuleInfo};
use super::BoardConfig;

/// QEMU virt 보드 (AArch64 SMP)
pub struct QemuVirtAarch64Smp;

impl BoardConfig for QemuVirtAarch64Smp {
    // UART: PL011
    const UART_BASE: usize = 0x0900_0000;
    const UART_IRQ: u32 = 33; // SPI #1 (32 + 1)
    const UART_CLOCK_FREQ: u32 = 24_000_000; // 24MHz

    // Timer: ARM Generic Timer
    const TIMER_FREQ: u64 = 0; // CNTFRQ_EL0에서 동적으로 읽음
    const TIMER_IRQ: u32 = 30; // Physical Timer PPI #14 (16 + 14)

    // GIC: GICv2
    const GICD_BASE: usize = 0x0800_0000; // Distributor
    const GICC_BASE: usize = 0x0801_0000; // CPU Interface

    // Memory
    const RAM_BASE: usize = 0x4000_0000; // 1GB
    const RAM_SIZE: usize = 512 * 1024 * 1024; // 512MB (기본값)
}

/// PSCI (Power State Coordination Interface) 함수 ID
///
/// ARM에서 CPU 전원 관리를 위한 표준 인터페이스입니다.
/// QEMU virt는 HVC 호출을 통해 PSCI를 지원합니다.
pub mod psci {
    /// PSCI 버전 조회
    pub const VERSION: u32 = 0x8400_0000;

    /// CPU 전원 켜기 (64비트)
    pub const CPU_ON_64: u32 = 0xC400_0003;

    /// CPU 전원 끄기
    pub const CPU_OFF: u32 = 0x8400_0002;

    /// CPU 일시 중지
    pub const CPU_SUSPEND_64: u32 = 0xC400_0001;

    /// CPU 상태 조회 (64비트)
    pub const AFFINITY_INFO_64: u32 = 0xC400_0004;

    /// 시스템 리셋
    pub const SYSTEM_RESET: u32 = 0x8400_0009;

    /// 시스템 종료
    pub const SYSTEM_OFF: u32 = 0x8400_0008;

    /// PSCI 반환 코드
    pub mod error {
        pub const SUCCESS: i32 = 0;
        pub const NOT_SUPPORTED: i32 = -1;
        pub const INVALID_PARAMS: i32 = -2;
        pub const DENIED: i32 = -3;
        pub const ALREADY_ON: i32 = -4;
        pub const ON_PENDING: i32 = -5;
        pub const INTERNAL_FAILURE: i32 = -6;
        pub const NOT_PRESENT: i32 = -7;
        pub const DISABLED: i32 = -8;
        pub const INVALID_ADDRESS: i32 = -9;
    }
}

/// 보드 초기화 함수
fn board_init() -> Result<(), i32> {
    crate::kprintln!("[board] QEMU virt AArch64 SMP initialized");

    // PSCI 버전 확인
    let version = psci_version();
    let major = (version >> 16) & 0xFFFF;
    let minor = version & 0xFFFF;
    crate::kprintln!("[board] PSCI version: {}.{}", major, minor);

    Ok(())
}

/// 보드 모듈 정보
pub static BOARD_INFO: BoardModuleInfo = BoardModuleInfo {
    // 같은 compatible이지만 SMP 버전은 나중에 등록되어
    // DTB에서 CPU가 여러 개일 때 선택됨
    compatible: &["linux,dummy-virt", "qemu,virt"],
    name: "QEMU virt (AArch64 SMP)",
    timer_freq: 0, // CNTFRQ_EL0에서 읽음
    uart_quirks: uart_quirks::NONE,
    init_fn: board_init,
    early_init_fn: None,
    platform_config_fn: None,
    cpu_count: 0, // DTB에서 읽음
    smp_capable: true,
};

// ============================================================================
// PSCI 인터페이스
// ============================================================================

/// PSCI 버전 조회
pub fn psci_version() -> u32 {
    psci_call(psci::VERSION as u64, 0, 0, 0) as u32
}

/// 보조 CPU 시작
///
/// # Arguments
/// * `cpu_id` - 대상 CPU의 MPIDR 값
/// * `entry_point` - CPU가 시작할 물리 주소
/// * `context_id` - CPU에 전달할 컨텍스트 값 (x0 레지스터로 전달됨)
///
/// # Returns
/// PSCI 에러 코드 (0 = 성공)
pub fn cpu_on(cpu_id: u64, entry_point: usize, context_id: usize) -> i32 {
    psci_call(
        psci::CPU_ON_64 as u64,
        cpu_id,
        entry_point as u64,
        context_id as u64,
    ) as i32
}

/// 현재 CPU 종료
///
/// 이 함수는 반환하지 않습니다.
pub fn cpu_off() -> ! {
    psci_call(psci::CPU_OFF as u64, 0, 0, 0);
    // 반환하지 않아야 하지만, 안전을 위해 무한 루프
    loop {
        unsafe {
            core::arch::asm!("wfi");
        }
    }
}

/// CPU 상태 조회
///
/// # Arguments
/// * `cpu_id` - 대상 CPU의 MPIDR 값
/// * `lowest_affinity_level` - 조회할 affinity 레벨 (보통 0)
///
/// # Returns
/// * 0: CPU가 켜져 있음
/// * 1: CPU가 꺼져 있음
/// * 2: 전환 중
pub fn affinity_info(cpu_id: u64, lowest_affinity_level: u64) -> i32 {
    psci_call(
        psci::AFFINITY_INFO_64 as u64,
        cpu_id,
        lowest_affinity_level,
        0,
    ) as i32
}

/// 시스템 리셋
pub fn system_reset() -> ! {
    psci_call(psci::SYSTEM_RESET as u64, 0, 0, 0);
    loop {
        unsafe {
            core::arch::asm!("wfi");
        }
    }
}

/// 시스템 종료
pub fn system_off() -> ! {
    psci_call(psci::SYSTEM_OFF as u64, 0, 0, 0);
    loop {
        unsafe {
            core::arch::asm!("wfi");
        }
    }
}

/// PSCI HVC 호출
///
/// ARM에서 PSCI는 SMC나 HVC를 통해 호출됩니다.
/// QEMU virt는 HVC를 사용합니다.
#[inline(always)]
fn psci_call(func_id: u64, arg0: u64, arg1: u64, arg2: u64) -> i64 {
    let result: i64;
    unsafe {
        core::arch::asm!(
            "hvc #0",
            inout("x0") func_id => result,
            in("x1") arg0,
            in("x2") arg1,
            in("x3") arg2,
            options(nomem, nostack)
        );
    }
    result
}

// ============================================================================
// 보조 CPU 부팅
// ============================================================================

/// 보조 CPU 시작 진입점
///
/// 이 함수는 cpu_on으로 시작된 보조 CPU가 처음 실행하는 코드입니다.
/// context_id를 x0으로 받습니다.
#[unsafe(no_mangle)]
pub extern "C" fn secondary_cpu_entry(context_id: usize) -> ! {
    let cpu_id = context_id as u32;

    // 1. 스택 설정 (primary CPU가 미리 할당해둔 스택 사용)
    let stack_top = crate::proc::percpu::stacks::get_stack_top(cpu_id);
    if stack_top != 0 {
        unsafe {
            core::arch::asm!(
                "mov sp, {0}",
                in(reg) stack_top,
            );
        }
    }

    // 2. 예외 벡터 설정
    crate::arch::exception::init();

    // 3. GIC CPU Interface 초기화 (GICD는 primary가 이미 완료)
    crate::arch::gic::init_secondary();

    // 4. 타이머 초기화
    crate::arch::timer::init_secondary();

    // 5. Per-CPU 데이터 초기화 및 온라인 표시
    crate::proc::percpu::init_secondary(cpu_id);

    // 6. 이 CPU의 idle 스레드 생성 (스케줄러에서 사용)
    crate::proc::init_on_secondary_cpu(cpu_id);

    crate::kprintln!("[smp] CPU {} online", cpu_id);

    // 7. 인터럽트 활성화
    unsafe {
        core::arch::asm!("msr DAIFClr, #2"); // IRQ unmask
    }

    // 8. idle 루프 (타이머 인터럽트가 스케줄러를 호출)
    loop {
        unsafe {
            core::arch::asm!("wfi");
        }
    }
}

/// 모든 보조 CPU 시작
///
/// DTB에서 읽은 CPU 수만큼 보조 CPU를 시작합니다.
#[allow(dead_code)]
pub fn start_secondary_cpus(cpu_count: usize, entry_point: usize) {
    crate::kprintln!("[smp] Starting {} secondary CPUs", cpu_count - 1);

    for cpu_id in 1..cpu_count {
        let mpidr = cpu_id as u64; // 간단한 경우 cpu_id == mpidr

        match cpu_on(mpidr, entry_point, cpu_id) {
            0 => {
                crate::kprintln!("[smp] CPU {} started", cpu_id);
            }
            psci::error::ALREADY_ON => {
                crate::kprintln!("[smp] CPU {} already on", cpu_id);
            }
            err => {
                crate::kprintln!("[smp] Failed to start CPU {}: error {}", cpu_id, err);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_qemu_virt_smp_config() {
        assert_eq!(QemuVirtAarch64Smp::UART_BASE, 0x0900_0000);
        assert_eq!(QemuVirtAarch64Smp::GICD_BASE, 0x0800_0000);
        assert_eq!(QemuVirtAarch64Smp::RAM_BASE, 0x4000_0000);
        assert!(BOARD_INFO.smp_capable);
    }
}
