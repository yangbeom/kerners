//! QEMU virt 보드 설정 (AArch64)
//!
//! QEMU의 virt 머신 타입에 대한 하드웨어 설정입니다.
//! 참고: https://www.qemu.org/docs/master/system/arm/virt.html

use super::board_module::{uart_quirks, BoardModuleInfo};
use super::BoardConfig;

/// QEMU virt 보드 (AArch64)
pub struct QemuVirtAarch64;

impl BoardConfig for QemuVirtAarch64 {
    // UART: PL011
    const UART_BASE: usize = 0x0900_0000;
    const UART_IRQ: u32 = 33; // SPI #1 (32 + 1)
    const UART_CLOCK_FREQ: u32 = 24_000_000; // 24MHz

    // Timer: ARM Generic Timer
    // 주파수는 CNTFRQ_EL0 시스템 레지스터에서 읽음
    const TIMER_FREQ: u64 = 0; // 0 = 시스템 레지스터에서 동적으로 읽음
    const TIMER_IRQ: u32 = 30; // Physical Timer PPI #14 (16 + 14)

    // GIC: GICv2
    const GICD_BASE: usize = 0x0800_0000; // Distributor
    const GICC_BASE: usize = 0x0801_0000; // CPU Interface

    // Memory
    const RAM_BASE: usize = 0x4000_0000; // 1GB
    const RAM_SIZE: usize = 512 * 1024 * 1024; // 512MB (기본값)
}

/// 보드 초기화 함수
fn board_init() -> Result<(), i32> {
    // 싱글코어 QEMU virt는 특별한 초기화가 필요 없음
    Ok(())
}

/// 보드 모듈 정보
pub static BOARD_INFO: BoardModuleInfo = BoardModuleInfo {
    compatible: &["linux,dummy-virt", "qemu,virt"],
    name: "QEMU virt (AArch64)",
    timer_freq: 0, // CNTFRQ_EL0에서 읽음
    uart_quirks: uart_quirks::NONE,
    init_fn: board_init,
    early_init_fn: None,
    platform_config_fn: None,
    cpu_count: 0, // DTB에서 읽음
    smp_capable: false,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_qemu_virt_config() {
        assert_eq!(QemuVirtAarch64::UART_BASE, 0x0900_0000);
        assert_eq!(QemuVirtAarch64::GICD_BASE, 0x0800_0000);
        assert_eq!(QemuVirtAarch64::RAM_BASE, 0x4000_0000);
    }
}
