//! ARM Generic Interrupt Controller (GICv2) 드라이버
//! 
//! QEMU virt 머신은 GICv2를 사용합니다.
//! 
//! 주요 구성 요소:
//! - Distributor (GICD): 인터럽트 라우팅 및 우선순위 관리
//! - CPU Interface (GICC): CPU별 인터럽트 처리
//! 
//! QEMU virt 머신의 GIC 주소:
//! - GICD: 0x0800_0000
//! - GICC: 0x0801_0000

use core::ptr::{read_volatile, write_volatile};
use crate::kprintln;

/// GIC Distributor 베이스 주소 얻기
#[inline]
fn gicd_base() -> usize {
    if crate::drivers::config::is_initialized() {
        crate::drivers::config::gicd_base()
    } else {
        crate::boards::gicd_base()
    }
}

/// GIC CPU Interface 베이스 주소 얻기
#[inline]
fn gicc_base() -> usize {
    if crate::drivers::config::is_initialized() {
        crate::drivers::config::gicc_base()
    } else {
        crate::boards::gicc_base()
    }
}

/// Distributor 레지스터 오프셋
const GICD_CTLR: usize = 0x000;        // Distributor Control
const GICD_TYPER: usize = 0x004;       // Interrupt Controller Type
const GICD_ISENABLER: usize = 0x100;   // Interrupt Set-Enable
const GICD_IPRIORITYR: usize = 0x400;  // Interrupt Priority
const GICD_ITARGETSR: usize = 0x800;   // Interrupt Processor Targets
const GICD_ICFGR: usize = 0xC00;       // Interrupt Configuration

/// CPU Interface 레지스터 오프셋
const GICC_CTLR: usize = 0x000;        // CPU Interface Control
const GICC_PMR: usize = 0x004;         // Interrupt Priority Mask
const GICC_IAR: usize = 0x00C;         // Interrupt Acknowledge
const GICC_EOIR: usize = 0x010;        // End of Interrupt

/// Physical Timer IRQ 번호 (QEMU virt)
pub const IRQ_PHYS_TIMER: u32 = 30;

/// UART IRQ 번호 (QEMU virt)
pub const IRQ_UART: u32 = 33;

/// GIC 레지스터 읽기
#[inline]
unsafe fn gicd_read(offset: usize) -> u32 {
    read_volatile((gicd_base() + offset) as *const u32)
}

/// GIC 레지스터 쓰기
#[inline]
unsafe fn gicd_write(offset: usize, value: u32) {
    write_volatile((gicd_base() + offset) as *mut u32, value);
}

/// CPU Interface 레지스터 읽기
#[inline]
unsafe fn gicc_read(offset: usize) -> u32 {
    read_volatile((gicc_base() + offset) as *const u32)
}

/// CPU Interface 레지스터 쓰기
#[inline]
unsafe fn gicc_write(offset: usize, value: u32) {
    write_volatile((gicc_base() + offset) as *mut u32, value);
}

/// 특정 인터럽트 활성화
pub unsafe fn enable_irq(irq: u32) {
    let reg_idx = (irq / 32) as usize;
    let bit_idx = irq % 32;
    let offset = GICD_ISENABLER + reg_idx * 4;
    
    gicd_write(offset, 1 << bit_idx);
}

/// 인터럽트 우선순위 설정 (0 = 최고 우선순위)
pub unsafe fn set_priority(irq: u32, priority: u8) {
    let reg_idx = (irq / 4) as usize;
    let byte_idx = (irq % 4) as usize;
    let offset = GICD_IPRIORITYR + reg_idx * 4;
    
    let mut val = gicd_read(offset);
    let shift = byte_idx * 8;
    val &= !(0xFF << shift);
    val |= (priority as u32) << shift;
    gicd_write(offset, val);
}

/// 인터럽트 타겟 CPU 설정
pub unsafe fn set_target(irq: u32, cpu_mask: u8) {
    let reg_idx = (irq / 4) as usize;
    let byte_idx = (irq % 4) as usize;
    let offset = GICD_ITARGETSR + reg_idx * 4;
    
    let mut val = gicd_read(offset);
    let shift = byte_idx * 8;
    val &= !(0xFF << shift);
    val |= (cpu_mask as u32) << shift;
    gicd_write(offset, val);
}

/// 인터럽트 acknowledge
pub unsafe fn ack_irq() -> u32 {
    gicc_read(GICC_IAR)
}

/// 인터럽트 처리 완료
pub unsafe fn end_irq(irq: u32) {
    gicc_write(GICC_EOIR, irq);
}

/// GIC 초기화
pub fn init() -> Result<(), &'static str> {
    kprintln!("\n[GIC] Initializing GICv2...");
    
    unsafe {
        // 1. Distributor 정보 확인
        let typer = gicd_read(GICD_TYPER);
        let it_lines_number = typer & 0x1F;
        let max_irqs = (it_lines_number + 1) * 32;
        kprintln!("[GIC] Max IRQs: {}", max_irqs);
        
        // 2. Distributor 활성화
        gicd_write(GICD_CTLR, 1);
        
        // 3. CPU Interface 활성화
        gicc_write(GICC_CTLR, 1);
        
        // 4. Priority Mask 설정 (모든 우선순위 허용)
        gicc_write(GICC_PMR, 0xFF);
        
        // 5. Physical Timer IRQ 설정
        set_priority(IRQ_PHYS_TIMER, 0xA0); // 중간 우선순위
        set_target(IRQ_PHYS_TIMER, 1);      // CPU 0에 전달
        enable_irq(IRQ_PHYS_TIMER);
        
        kprintln!("[GIC] Physical Timer IRQ {} enabled", IRQ_PHYS_TIMER);
        
        // 6. UART IRQ 설정
        set_priority(IRQ_UART, 0x80);       // 높은 우선순위
        set_target(IRQ_UART, 1);            // CPU 0에 전달
        enable_irq(IRQ_UART);
        
        kprintln!("[GIC] UART IRQ {} enabled", IRQ_UART);
    }
    
    kprintln!("[GIC] GICv2 initialized");
    Ok(())
}

/// Secondary CPU의 GIC CPU Interface 초기화
///
/// GICD(Distributor)는 primary CPU에서 이미 초기화되었으므로,
/// 각 secondary CPU는 자신의 GICC(CPU Interface)만 초기화합니다.
pub fn init_secondary() {
    unsafe {
        // CPU Interface 활성화
        gicc_write(GICC_CTLR, 1);

        // Priority Mask 설정 (모든 우선순위 허용)
        gicc_write(GICC_PMR, 0xFF);
    }
}

/// SGI (Software Generated Interrupt) 전송 — IPI 용도
///
/// # Arguments
/// * `target_cpu` - 대상 CPU 번호 (0-7)
/// * `sgi_id` - SGI 번호 (0-15)
#[allow(dead_code)]
pub fn send_sgi(target_cpu: u32, sgi_id: u32) {
    const GICD_SGIR: usize = 0xF00;
    unsafe {
        // TargetListFilter=0 (use target list), CPU target list, SGI ID
        let value = ((1u32 << target_cpu) << 16) | (sgi_id & 0xF);
        gicd_write(GICD_SGIR, value);
    }
}

/// IPI용 SGI 번호 (reschedule 요청)
pub const SGI_RESCHEDULE: u32 = 0;

/// IRQ 핸들러에서 호출
pub fn handle_irq() {
    unsafe {
        let irq = ack_irq();
        let irq_num = irq & 0x3FF; // 하위 10비트가 IRQ 번호

        if irq_num >= 1020 {
            return; // spurious interrupt
        }

        // SGI (IPI) 처리 (IRQ 0-15)
        if irq_num < 16 {
            if irq_num == SGI_RESCHEDULE {
                // Reschedule IPI: 스케줄러가 타이머 틱에서 자동 호출되므로
                // 여기서는 추가 처리 없이 EOI만 수행
                crate::proc::scheduler::schedule();
            }
        }
        // 타이머 인터럽트인 경우
        else if irq_num == IRQ_PHYS_TIMER {
            super::timer::handle_irq();
        }
        // UART 인터럽트인 경우
        else if irq_num == IRQ_UART {
            super::uart::handle_irq();
        }
        // VirtIO 디바이스 인터럽트
        else if crate::virtio::irq::handle_virtio_irq(irq_num) {
            // VirtIO 디바이스가 처리함
        }

        // EOI (End of Interrupt)
        end_irq(irq);
    }
}

/// 다른 CPU에 reschedule IPI 전송
#[allow(dead_code)]
pub fn send_reschedule_ipi(target_cpu: u32) {
    send_sgi(target_cpu, SGI_RESCHEDULE);
}
