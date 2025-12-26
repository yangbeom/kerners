//! 보드별 설정
//!
//! 각 보드(QEMU virt, Raspberry Pi 등)의 하드웨어 설정을 정의합니다.
//! DTB에서 정보를 얻을 수 없는 경우 폴백으로 사용됩니다.
//!
//! ## 보드 모듈 시스템
//!
//! 런타임에 DTB의 compatible 속성을 기반으로 보드를 선택합니다:
//! 1. 부팅 시 DTB에서 루트 노드의 compatible 읽기
//! 2. 등록된 보드 모듈 중 매칭되는 것 선택
//! 3. 없으면 컴파일 타임 기본값(CurrentBoard) 사용

pub mod board_module;
pub mod registry;

#[cfg(target_arch = "aarch64")]
mod qemu_virt_aarch64;
#[cfg(target_arch = "aarch64")]
pub mod qemu_virt_aarch64_smp;

#[cfg(target_arch = "riscv64")]
mod qemu_virt_riscv64;
#[cfg(target_arch = "riscv64")]
pub mod qemu_virt_riscv64_smp;

pub use board_module::{BoardModuleInfo, BoardPlatformOverrides};

/// 보드 설정 trait
///
/// 각 보드는 이 trait을 구현하여 하드웨어 기본값을 제공합니다.
/// 폴백 우선순위: DTB → BoardConfig → 아키텍처 기본값
pub trait BoardConfig {
    // UART 설정
    const UART_BASE: usize;
    const UART_IRQ: u32;
    const UART_CLOCK_FREQ: u32 = 0; // 0 = 알 수 없음

    // 타이머 설정
    const TIMER_FREQ: u64;
    const TIMER_IRQ: u32 = 0; // 0 = 아키텍처 기본값 사용

    // 인터럽트 컨트롤러 (아키텍처별)
    const GICD_BASE: usize = 0;
    const GICC_BASE: usize = 0;
    const PLIC_BASE: usize = 0;
    const CLINT_BASE: usize = 0;

    // 메모리 (DTB에서 못 읽을 경우 폴백)
    const RAM_BASE: usize;
    const RAM_SIZE: usize;
}

/// 현재 보드 타입 (컴파일 타임 선택)
#[cfg(target_arch = "aarch64")]
pub type CurrentBoard = qemu_virt_aarch64::QemuVirtAarch64;

#[cfg(target_arch = "riscv64")]
pub type CurrentBoard = qemu_virt_riscv64::QemuVirtRiscv64;

// 편의 함수들: CurrentBoard의 설정에 접근
pub fn uart_base() -> usize {
    CurrentBoard::UART_BASE
}

pub fn uart_irq() -> u32 {
    CurrentBoard::UART_IRQ
}

pub fn uart_clock_freq() -> u32 {
    CurrentBoard::UART_CLOCK_FREQ
}

pub fn timer_freq() -> u64 {
    CurrentBoard::TIMER_FREQ
}

pub fn timer_irq() -> u32 {
    CurrentBoard::TIMER_IRQ
}

#[cfg(target_arch = "aarch64")]
pub fn gicd_base() -> usize {
    CurrentBoard::GICD_BASE
}

#[cfg(target_arch = "aarch64")]
pub fn gicc_base() -> usize {
    CurrentBoard::GICC_BASE
}

#[cfg(target_arch = "riscv64")]
pub fn plic_base() -> usize {
    CurrentBoard::PLIC_BASE
}

#[cfg(target_arch = "riscv64")]
pub fn clint_base() -> usize {
    CurrentBoard::CLINT_BASE
}

pub fn ram_base() -> usize {
    CurrentBoard::RAM_BASE
}

pub fn ram_size() -> usize {
    CurrentBoard::RAM_SIZE
}

// ============================================================================
// 보드 모듈 시스템 초기화
// ============================================================================

/// 초기 보드 서브시스템 초기화 (MMU 활성화 전)
///
/// 빌트인 보드 모듈을 레지스트리에 등록하고,
/// DTB compatible 문자열이 주어지면 매칭되는 보드를 활성화합니다.
pub fn init_early(dtb_compatibles: Option<&[&str]>) {
    // 빌트인 보드 등록
    register_builtin_boards();

    // DTB compatible로 보드 선택
    if let Some(compats) = dtb_compatibles {
        if let Some(board) = registry::find_best_board_by_compatibles(compats) {
            registry::set_active_board(board);

            // early_init_fn 호출
            if let Some(early_init) = board.early_init_fn {
                early_init();
            }
        }
    }

    // 활성 보드가 없으면 첫 번째 등록된 보드 사용
    if registry::active_board().is_none() {
        registry::for_each_board_info(|info, _| {
            if registry::active_board().is_none() {
                registry::set_active_board(info);
            }
        });
    }
}

/// 보드 서브시스템 초기화 (MMU 활성화 후)
///
/// 활성 보드의 init_fn을 호출합니다.
pub fn init() -> Result<(), i32> {
    if let Some(board) = registry::active_board() {
        (board.init_fn)()?;
    }
    Ok(())
}

/// 빌트인 보드 모듈 등록
fn register_builtin_boards() {
    #[cfg(target_arch = "aarch64")]
    {
        let _ = registry::register_builtin_board(&qemu_virt_aarch64::BOARD_INFO);
        let _ = registry::register_builtin_board(&qemu_virt_aarch64_smp::BOARD_INFO);
    }

    #[cfg(target_arch = "riscv64")]
    {
        let _ = registry::register_builtin_board(&qemu_virt_riscv64::BOARD_INFO);
        let _ = registry::register_builtin_board(&qemu_virt_riscv64_smp::BOARD_INFO);
    }
}

/// 현재 활성 보드 정보 가져오기
pub fn current_board_info() -> Option<&'static BoardModuleInfo> {
    registry::active_board()
}

/// 활성 보드가 SMP를 지원하는지 확인
pub fn is_smp_capable() -> bool {
    registry::active_board().map_or(false, |b| b.smp_capable)
}
