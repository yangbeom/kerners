//! 플랫폼 디바이스 탐색
//!
//! DTB에서 디바이스 정보를 읽고, 실패 시 BoardConfig 폴백을 사용합니다.

use crate::boards::{self, BoardConfig, CurrentBoard};
use crate::drivers::config::*;
use crate::dtb::{self, DeviceTree};

/// 플랫폼 설정 탐색
///
/// DTB에서 디바이스 정보를 읽고, BoardConfig 폴백을 적용하여
/// 완전한 PlatformConfig를 반환합니다.
pub fn probe_platform() -> PlatformConfig {
    let dt = dtb::get();

    crate::kprintln!("[probe] Starting platform device probe...");

    // 인터럽트 컨트롤러 탐색
    #[cfg(target_arch = "aarch64")]
    let interrupt_controller = probe_gic(dt);

    #[cfg(target_arch = "riscv64")]
    let interrupt_controller = probe_plic(dt);

    #[cfg(not(any(target_arch = "aarch64", target_arch = "riscv64")))]
    let interrupt_controller = InterruptControllerConfig::None;

    // UART 탐색
    let uart = probe_uart(dt);

    // 타이머 탐색
    let timer = probe_timer(dt);

    // CLINT 탐색 (RISC-V only)
    #[cfg(target_arch = "riscv64")]
    let clint = probe_clint(dt);

    #[cfg(not(target_arch = "riscv64"))]
    let clint = None;

    // CPU 개수
    let cpu_count = dt.map(|d| d.count_cpus()).unwrap_or(1);

    crate::kprintln!("[probe] Platform probe complete");

    PlatformConfig {
        uart,
        interrupt_controller,
        timer,
        clint,
        cpu_count,
    }
}

/// GIC 탐색 (AArch64)
#[cfg(target_arch = "aarch64")]
fn probe_gic(dt: Option<&DeviceTree>) -> InterruptControllerConfig {
    if let Some(info) = dt.and_then(|d| d.find_gic()) {
        crate::kprintln!(
            "[probe] GIC found via DTB: GICD={:#x}({:#x}), GICC={:#x}({:#x}), version={:?}",
            info.distributor_base,
            info.distributor_size,
            info.cpu_interface_base,
            info.cpu_interface_size,
            info.version
        );

        let version = match info.version {
            dtb::GicVersion::V2 => GicVersion::V2,
            dtb::GicVersion::V3 => GicVersion::V3,
        };

        InterruptControllerConfig::Gic(GicConfig {
            distributor_base: info.distributor_base as usize,
            distributor_size: info.distributor_size as usize,
            cpu_interface_base: info.cpu_interface_base as usize,
            cpu_interface_size: info.cpu_interface_size as usize,
            redistributor_base: info.redistributor_base.map(|b| b as usize),
            redistributor_size: info.redistributor_size.map(|s| s as usize),
            version,
        })
    } else {
        // BoardConfig 폴백
        crate::kprintln!(
            "[probe] GIC not in DTB, using board defaults: GICD={:#x}, GICC={:#x}",
            CurrentBoard::GICD_BASE,
            CurrentBoard::GICC_BASE
        );

        InterruptControllerConfig::Gic(GicConfig {
            distributor_base: CurrentBoard::GICD_BASE,
            distributor_size: 0x1_0000,
            cpu_interface_base: CurrentBoard::GICC_BASE,
            cpu_interface_size: 0x1_0000,
            redistributor_base: None,
            redistributor_size: None,
            version: GicVersion::V2,
        })
    }
}

/// PLIC 탐색 (RISC-V)
#[cfg(target_arch = "riscv64")]
fn probe_plic(dt: Option<&DeviceTree>) -> InterruptControllerConfig {
    if let Some(info) = dt.and_then(|d| d.find_plic()) {
        crate::kprintln!(
            "[probe] PLIC found via DTB: base={:#x}, size={:#x}",
            info.base,
            info.size
        );

        InterruptControllerConfig::Plic(PlicConfig {
            base: info.base as usize,
            size: info.size as usize,
            num_sources: 127, // QEMU virt 기본값
            num_contexts: 2,  // M-mode + S-mode per hart
        })
    } else {
        // BoardConfig 폴백
        crate::kprintln!(
            "[probe] PLIC not in DTB, using board default: base={:#x}",
            CurrentBoard::PLIC_BASE
        );

        InterruptControllerConfig::Plic(PlicConfig {
            base: CurrentBoard::PLIC_BASE,
            size: 0x400000, // 4MB (일반적인 PLIC 크기)
            num_sources: 127,
            num_contexts: 2,
        })
    }
}

/// CLINT 탐색 (RISC-V)
#[cfg(target_arch = "riscv64")]
fn probe_clint(dt: Option<&DeviceTree>) -> Option<ClintConfig> {
    if let Some(info) = dt.and_then(|d| d.find_clint()) {
        crate::kprintln!(
            "[probe] CLINT found via DTB: base={:#x}, size={:#x}",
            info.base,
            info.size
        );

        Some(ClintConfig {
            base: info.base as usize,
            size: info.size as usize,
        })
    } else {
        // BoardConfig 폴백
        crate::kprintln!(
            "[probe] CLINT not in DTB, using board default: base={:#x}",
            CurrentBoard::CLINT_BASE
        );

        Some(ClintConfig {
            base: CurrentBoard::CLINT_BASE,
            size: 0x10000, // 64KB (일반적인 CLINT 크기)
        })
    }
}

/// UART 탐색
fn probe_uart(dt: Option<&DeviceTree>) -> UartConfig {
    if let Some(info) = dt.and_then(|d| d.find_uart()) {
        crate::kprintln!(
            "[probe] UART found via DTB: base={:#x}, irq={}, clock={}Hz",
            info.base,
            info.irq,
            info.clock_freq
        );

        UartConfig {
            base: info.base as usize,
            size: info.size as usize,
            irq: if info.irq != 0 {
                info.irq
            } else {
                boards::uart_irq()
            },
            clock_freq: if info.clock_freq != 0 {
                info.clock_freq
            } else {
                boards::uart_clock_freq()
            },
        }
    } else {
        // BoardConfig 폴백
        crate::kprintln!(
            "[probe] UART not in DTB, using board default: base={:#x}",
            CurrentBoard::UART_BASE
        );

        UartConfig {
            base: CurrentBoard::UART_BASE,
            size: 0x1000, // 4KB (일반적인 UART 레지스터 영역)
            irq: CurrentBoard::UART_IRQ,
            clock_freq: CurrentBoard::UART_CLOCK_FREQ,
        }
    }
}

/// 타이머 탐색
fn probe_timer(dt: Option<&DeviceTree>) -> TimerConfig {
    #[cfg(target_arch = "aarch64")]
    {
        // ARM Generic Timer는 DTB 노드가 없음 (시스템 레지스터 사용)
        // 주파수는 CNTFRQ_EL0에서 동적으로 읽음
        let freq = unsafe {
            let f: u64;
            core::arch::asm!("mrs {}, CNTFRQ_EL0", out(reg) f);
            f
        };

        crate::kprintln!(
            "[probe] ARM Generic Timer: freq={}Hz (from CNTFRQ_EL0)",
            freq
        );

        let irq = dt
            .and_then(find_arm_timer_irq)
            .unwrap_or_else(boards::timer_irq);

        crate::kprintln!("[probe] ARM Generic Timer IRQ: {}", irq);

        TimerConfig {
            timer_type: TimerType::ArmGenericTimer,
            frequency: freq,
            irq,
        }
    }

    #[cfg(target_arch = "riscv64")]
    {
        // RISC-V CLINT 타이머: /cpus/timebase-frequency 우선, 보드 폴백
        let dtb_freq = dt.and_then(|d| d.get_timebase_frequency());
        let freq = dtb_freq.unwrap_or(CurrentBoard::TIMER_FREQ);
        if dtb_freq.is_some() {
            crate::kprintln!(
                "[probe] RISC-V CLINT Timer: freq={}Hz (from DTB /cpus/timebase-frequency)",
                freq
            );
        } else {
            crate::kprintln!(
                "[probe] RISC-V CLINT Timer: freq={}Hz (from BoardConfig fallback)",
                freq
            );
        }

        TimerConfig {
            timer_type: TimerType::RiscvClint,
            frequency: freq,
            irq: 0, // CLINT은 CSR로 처리, IRQ 없음
        }
    }

    #[cfg(not(any(target_arch = "aarch64", target_arch = "riscv64")))]
    {
        TimerConfig {
            timer_type: TimerType::ArmGenericTimer,
            frequency: 0,
            irq: 0,
        }
    }
}

#[cfg(target_arch = "aarch64")]
fn find_arm_timer_irq(dt: &DeviceTree) -> Option<u32> {
    let timer = dt.find_compatible("arm,armv8-timer").into_iter().next()?;

    // GIC interrupt specifier: <type irq flags>
    // type: 0 = SPI(+32), 1 = PPI(+16)
    let decode = |cells: &[u32]| -> Option<u32> {
        if cells.len() < 2 {
            return None;
        }
        let irq_type = cells[0];
        let irq_num = cells[1];
        Some(match irq_type {
            0 => irq_num.saturating_add(32),
            1 => irq_num.saturating_add(16),
            _ => irq_num,
        })
    };

    // ARM generic timer는 보통 4개의 인터럽트 엔트리를 가진다.
    // non-secure physical timer(PPI 14)를 우선 선택한다.
    for chunk in timer.interrupts.chunks(3) {
        if chunk.len() >= 2 && chunk[0] == 1 && chunk[1] == 14 {
            return decode(chunk);
        }
    }

    timer.interrupts.chunks(3).find_map(decode)
}
