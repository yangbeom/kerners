//! 플랫폼 설정 저장소
//!
//! DTB에서 읽은 디바이스 정보를 저장하고 전역으로 접근할 수 있게 합니다.
//! 드라이버들은 이 모듈을 통해 하드웨어 설정에 접근합니다.

use crate::sync::RwLock;
use core::sync::atomic::{AtomicBool, Ordering};

/// UART 설정
#[derive(Debug, Clone)]
pub struct UartConfig {
    pub base: usize,
    pub size: usize,
    pub irq: u32,
    pub clock_freq: u32,
}

/// GIC (Generic Interrupt Controller) 설정 - AArch64
#[derive(Debug, Clone)]
pub struct GicConfig {
    pub distributor_base: usize,
    pub cpu_interface_base: usize,
    pub redistributor_base: Option<usize>, // GICv3용
    pub version: GicVersion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GicVersion {
    V2,
    V3,
}

/// PLIC (Platform-Level Interrupt Controller) 설정 - RISC-V
#[derive(Debug, Clone)]
pub struct PlicConfig {
    pub base: usize,
    pub size: usize,
    pub num_sources: u32,
    pub num_contexts: u32,
}

/// CLINT (Core Local Interruptor) 설정 - RISC-V
#[derive(Debug, Clone)]
pub struct ClintConfig {
    pub base: usize,
    pub size: usize,
}

/// 타이머 설정
#[derive(Debug, Clone)]
pub struct TimerConfig {
    pub timer_type: TimerType,
    pub frequency: u64,
    pub irq: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimerType {
    /// ARM Generic Timer (시스템 레지스터 기반)
    ArmGenericTimer,
    /// RISC-V CLINT Timer (MMIO 기반)
    RiscvClint,
}

/// 인터럽트 컨트롤러 설정 (아키텍처별)
#[derive(Debug, Clone)]
pub enum InterruptControllerConfig {
    Gic(GicConfig),
    Plic(PlicConfig),
    None,
}

/// 플랫폼 전체 설정
#[derive(Debug, Clone)]
pub struct PlatformConfig {
    pub uart: UartConfig,
    pub interrupt_controller: InterruptControllerConfig,
    pub timer: TimerConfig,
    pub clint: Option<ClintConfig>, // RISC-V only
    pub cpu_count: usize,
}

// 전역 설정 저장소
static CONFIG_INITIALIZED: AtomicBool = AtomicBool::new(false);
static PLATFORM_CONFIG: RwLock<Option<PlatformConfig>> = RwLock::new(None);

/// 플랫폼 설정 초기화 (부팅 초기 1회)
pub fn init_platform_config(config: PlatformConfig) {
    let mut guard = PLATFORM_CONFIG.write();
    crate::kprintln!("[config] Initializing platform config:");
    crate::kprintln!("  UART: base={:#x}, irq={}", config.uart.base, config.uart.irq);

    match &config.interrupt_controller {
        InterruptControllerConfig::Gic(gic) => {
            crate::kprintln!(
                "  GIC: GICD={:#x}, GICC={:#x}",
                gic.distributor_base,
                gic.cpu_interface_base
            );
        }
        InterruptControllerConfig::Plic(plic) => {
            crate::kprintln!("  PLIC: base={:#x}, sources={}", plic.base, plic.num_sources);
        }
        InterruptControllerConfig::None => {
            crate::kprintln!("  Interrupt Controller: None");
        }
    }

    crate::kprintln!(
        "  Timer: type={:?}, freq={}Hz",
        config.timer.timer_type,
        config.timer.frequency
    );
    crate::kprintln!("  CPUs: {}", config.cpu_count);

    *guard = Some(config);
    CONFIG_INITIALIZED.store(true, Ordering::Release);
}

/// 설정이 초기화되었는지 확인
pub fn is_initialized() -> bool {
    CONFIG_INITIALIZED.load(Ordering::Acquire)
}

/// 플랫폼 설정 읽기
pub fn get_platform_config() -> Option<PlatformConfig> {
    if !is_initialized() {
        return None;
    }
    let guard = PLATFORM_CONFIG.read();
    guard.clone()
}

/// UART 설정 읽기
pub fn uart_config() -> Option<UartConfig> {
    get_platform_config().map(|c| c.uart)
}

/// GIC 설정 읽기 (AArch64)
#[cfg(target_arch = "aarch64")]
pub fn gic_config() -> Option<GicConfig> {
    get_platform_config().and_then(|c| match c.interrupt_controller {
        InterruptControllerConfig::Gic(gic) => Some(gic),
        _ => None,
    })
}

/// PLIC 설정 읽기 (RISC-V)
#[cfg(target_arch = "riscv64")]
pub fn plic_config() -> Option<PlicConfig> {
    get_platform_config().and_then(|c| match c.interrupt_controller {
        InterruptControllerConfig::Plic(plic) => Some(plic),
        _ => None,
    })
}

/// CLINT 설정 읽기 (RISC-V)
#[cfg(target_arch = "riscv64")]
pub fn clint_config() -> Option<ClintConfig> {
    get_platform_config().and_then(|c| c.clint)
}

/// 타이머 설정 읽기
pub fn timer_config() -> Option<TimerConfig> {
    get_platform_config().map(|c| c.timer)
}

/// CPU 개수 읽기
pub fn cpu_count() -> usize {
    get_platform_config().map(|c| c.cpu_count).unwrap_or(1)
}

// UART 설정 헬퍼 함수들 (폴백 포함)
pub fn uart_base() -> usize {
    uart_config()
        .map(|c| c.base)
        .unwrap_or_else(crate::boards::uart_base)
}

pub fn uart_irq() -> u32 {
    uart_config()
        .map(|c| c.irq)
        .unwrap_or_else(crate::boards::uart_irq)
}

// GIC 설정 헬퍼 함수들 (폴백 포함)
#[cfg(target_arch = "aarch64")]
pub fn gicd_base() -> usize {
    gic_config()
        .map(|c| c.distributor_base)
        .unwrap_or_else(crate::boards::gicd_base)
}

#[cfg(target_arch = "aarch64")]
pub fn gicc_base() -> usize {
    gic_config()
        .map(|c| c.cpu_interface_base)
        .unwrap_or_else(crate::boards::gicc_base)
}

// PLIC 설정 헬퍼 함수들 (폴백 포함)
#[cfg(target_arch = "riscv64")]
pub fn plic_base() -> usize {
    plic_config()
        .map(|c| c.base)
        .unwrap_or_else(crate::boards::plic_base)
}

// CLINT/Timer 설정 헬퍼 함수들 (폴백 포함)
#[cfg(target_arch = "riscv64")]
pub fn clint_base() -> usize {
    clint_config()
        .map(|c| c.base)
        .unwrap_or_else(crate::boards::clint_base)
}

pub fn timer_freq() -> u64 {
    timer_config()
        .map(|c| c.frequency)
        .unwrap_or_else(crate::boards::timer_freq)
}

pub fn timer_irq() -> u32 {
    timer_config()
        .map(|c| c.irq)
        .unwrap_or_else(crate::boards::timer_irq)
}
