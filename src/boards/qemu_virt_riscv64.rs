//! QEMU virt 보드 설정 (RISC-V 64)
//!
//! QEMU의 virt 머신 타입에 대한 하드웨어 설정입니다.
//! 참고: https://www.qemu.org/docs/master/system/riscv/virt.html

use super::board_module::{uart_quirks, BoardModuleInfo};
use super::BoardConfig;

/// QEMU virt 보드 (RISC-V 64)
pub struct QemuVirtRiscv64;

impl BoardConfig for QemuVirtRiscv64 {
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

/// 보드 초기화 함수
fn board_init() -> Result<(), i32> {
    // 싱글코어 QEMU virt는 특별한 초기화가 필요 없음
    Ok(())
}

/// 보드 모듈 정보
pub static BOARD_INFO: BoardModuleInfo = BoardModuleInfo {
    compatible: &["riscv-virtio", "qemu,virt"],
    name: "QEMU virt (RISC-V64)",
    timer_freq: 10_000_000, // 10MHz
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
        assert_eq!(QemuVirtRiscv64::UART_BASE, 0x1000_0000);
        assert_eq!(QemuVirtRiscv64::PLIC_BASE, 0x0C00_0000);
        assert_eq!(QemuVirtRiscv64::CLINT_BASE, 0x0200_0000);
        assert_eq!(QemuVirtRiscv64::RAM_BASE, 0x8000_0000);
    }
}
