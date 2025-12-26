//! RISC-V PLIC (Platform-Level Interrupt Controller) 드라이버
//! 
//! PLIC는 외부 인터럽트를 관리합니다.
//! 
//! 레지스터:
//! - Priority (0x0C00_0000): 인터럽트 우선순위
//! - Pending (0x0C00_1000): Pending 인터럽트
//! - Enable (0x0C00_2000): 인터럽트 활성화
//! - Threshold (0x0C20_0000): 우선순위 임계값
//! - Claim/Complete (0x0C20_0004): 인터럽트 클레임/완료

use core::ptr::{read_volatile, write_volatile};
use crate::kprintln;

/// PLIC 베이스 주소 얻기
#[inline]
fn plic_base() -> usize {
    if crate::drivers::config::is_initialized() {
        crate::drivers::config::plic_base()
    } else {
        crate::boards::plic_base()
    }
}

/// Priority 레지스터 베이스 오프셋
const PLIC_PRIORITY_OFFSET: usize = 0x0;

/// Enable 레지스터 베이스 오프셋 (Context 0 = Machine mode, Hart 0)
const PLIC_ENABLE_OFFSET: usize = 0x2000;

/// Threshold 레지스터 오프셋 (Context 0)
const PLIC_THRESHOLD_OFFSET: usize = 0x20_0000;

/// Claim/Complete 레지스터 오프셋 (Context 0)
const PLIC_CLAIM_OFFSET: usize = 0x20_0004;

/// UART IRQ 번호 (QEMU virt)
pub const IRQ_UART: u32 = 10;

/// 인터럽트 우선순위 설정
pub unsafe fn set_priority(irq: u32, priority: u32) {
    let addr = plic_base() + PLIC_PRIORITY_OFFSET + (irq as usize) * 4;
    write_volatile(addr as *mut u32, priority);
}

/// 인터럽트 활성화
pub unsafe fn enable_irq(irq: u32) {
    let reg_idx = (irq / 32) as usize;
    let bit_idx = irq % 32;
    let addr = plic_base() + PLIC_ENABLE_OFFSET + reg_idx * 4;

    let mut val = read_volatile(addr as *const u32);
    val |= 1 << bit_idx;
    write_volatile(addr as *mut u32, val);
}

/// Threshold 설정
unsafe fn set_threshold(threshold: u32) {
    let addr = plic_base() + PLIC_THRESHOLD_OFFSET;
    write_volatile(addr as *mut u32, threshold);
}

/// 인터럽트 클레임
unsafe fn claim_irq() -> u32 {
    let addr = plic_base() + PLIC_CLAIM_OFFSET;
    read_volatile(addr as *const u32)
}

/// 인터럽트 완료
unsafe fn complete_irq(irq: u32) {
    let addr = plic_base() + PLIC_CLAIM_OFFSET;
    write_volatile(addr as *mut u32, irq);
}

/// PLIC 초기화
pub fn init() -> Result<(), &'static str> {
    kprintln!("\n[PLIC] Initializing PLIC...");
    
    unsafe {
        // Threshold 설정 (0 = 모든 우선순위 허용)
        set_threshold(0);
        
        // UART 인터럽트 설정
        set_priority(IRQ_UART, 1);
        enable_irq(IRQ_UART);
        
        kprintln!("[PLIC] UART IRQ {} enabled", IRQ_UART);
        
        // MIE에서 외부 인터럽트 + 소프트웨어 인터럽트(IPI) 활성화
        core::arch::asm!(
            "li t0, 0x808",     // MEIE (0x800) | MSIE (0x8)
            "csrs mie, t0"
        );
    }
    
    kprintln!("[PLIC] PLIC initialized");
    Ok(())
}

/// Secondary hart의 PLIC context 초기화
///
/// 각 hart는 자신만의 PLIC context를 가집니다.
/// M-mode context = hart_id * 2, S-mode context = hart_id * 2 + 1
pub fn init_secondary(hart_id: u32) {
    let context = (hart_id as usize) * 2; // M-mode context

    unsafe {
        // Per-context threshold 설정
        let threshold_addr = plic_base() + 0x20_0000 + context * 0x1000;
        write_volatile(threshold_addr as *mut u32, 0);

        // Per-context enable 레지스터에서 UART IRQ 활성화
        let enable_base = plic_base() + 0x2000 + context * 0x80;
        let reg_idx = (IRQ_UART / 32) as usize;
        let bit_idx = IRQ_UART % 32;
        let enable_addr = enable_base + reg_idx * 4;
        let mut val = read_volatile(enable_addr as *const u32);
        val |= 1 << bit_idx;
        write_volatile(enable_addr as *mut u32, val);

        // MIE에서 외부 인터럽트 + 소프트웨어 인터럽트(IPI) 활성화
        core::arch::asm!(
            "li t0, 0x808",     // MEIE (0x800) | MSIE (0x8)
            "csrs mie, t0"
        );
    }
}

/// 외부 인터럽트 핸들러
pub fn handle_irq() {
    unsafe {
        let irq = claim_irq();

        if irq == 0 {
            return; // spurious interrupt
        }

        if irq == IRQ_UART {
            super::uart::handle_irq();
        } else if crate::virtio::irq::handle_virtio_irq(irq) {
            // VirtIO 디바이스가 처리함
        }

        complete_irq(irq);
    }
}

/// CLINT MSIP를 통해 다른 hart에 소프트웨어 인터럽트(IPI) 전송
///
/// CLINT의 MSIP 레지스터를 1로 설정하면 해당 hart에
/// Machine Software Interrupt가 발생합니다.
#[allow(dead_code)]
pub fn send_ipi(target_hartid: u32) {
    let clint_base = if crate::drivers::config::is_initialized() {
        crate::drivers::config::clint_base()
    } else {
        crate::boards::clint_base()
    };
    unsafe {
        let msip_addr = clint_base + (target_hartid as usize) * 4;
        core::ptr::write_volatile(msip_addr as *mut u32, 1);
    }
}

/// 다른 hart에 reschedule IPI 전송
#[allow(dead_code)]
pub fn send_reschedule_ipi(target_hartid: u32) {
    send_ipi(target_hartid);
}
