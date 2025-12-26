//! VirtIO IRQ 디스패치 레지스트리
//!
//! PLIC/GIC 핸들러에서 VirtIO 디바이스의 인터럽트를 라우팅합니다.
//! 인터럽트 컨텍스트에서 호출되므로 고정 크기 배열 사용 (heap 할당 불가).

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use crate::virtio::mmio::VirtIOMMIO;

/// 최대 VirtIO 디바이스 수
const MAX_VIRTIO_DEVICES: usize = 8;

/// IRQ 등록 엔트리
struct IrqEntry {
    /// IRQ 번호 (0 = 미사용)
    irq_num: AtomicU32,
    /// MMIO 베이스 주소 (0 = 미사용)
    mmio_base: AtomicU32,
    /// 디바이스의 interrupt_flag 포인터
    flag: AtomicU32, // *const AtomicBool을 u32로 저장 (주소)
    flag_high: AtomicU32, // 상위 32비트 (64비트 주소 지원)
}

impl IrqEntry {
    const fn new() -> Self {
        Self {
            irq_num: AtomicU32::new(0),
            mmio_base: AtomicU32::new(0),
            flag: AtomicU32::new(0),
            flag_high: AtomicU32::new(0),
        }
    }

    fn is_used(&self) -> bool {
        self.irq_num.load(Ordering::Relaxed) != 0
    }

    fn get_flag_ptr(&self) -> *const AtomicBool {
        let lo = self.flag.load(Ordering::Relaxed) as u64;
        let hi = self.flag_high.load(Ordering::Relaxed) as u64;
        (lo | (hi << 32)) as *const AtomicBool
    }
}

/// 전역 IRQ 디스패치 테이블 (lock-free)
static IRQ_TABLE: [IrqEntry; MAX_VIRTIO_DEVICES] = [
    IrqEntry::new(), IrqEntry::new(), IrqEntry::new(), IrqEntry::new(),
    IrqEntry::new(), IrqEntry::new(), IrqEntry::new(), IrqEntry::new(),
];

/// VirtIO 디바이스 IRQ 등록
///
/// # Safety
/// `flag`는 디바이스가 살아있는 동안 유효해야 합니다 (Arc로 보호되는 경우 안전).
pub fn register_irq(irq_num: u32, mmio_base: usize, flag: &AtomicBool) {
    if irq_num == 0 {
        return;
    }

    let flag_addr = flag as *const AtomicBool as u64;

    for entry in IRQ_TABLE.iter() {
        if !entry.is_used() {
            entry.mmio_base.store(mmio_base as u32, Ordering::Relaxed);
            entry.flag.store(flag_addr as u32, Ordering::Relaxed);
            entry.flag_high.store((flag_addr >> 32) as u32, Ordering::Relaxed);
            // irq_num을 마지막에 저장하여 다른 필드가 먼저 설정되도록 보장
            entry.irq_num.store(irq_num, Ordering::Release);
            crate::kprintln!("[VirtIO-IRQ] Registered IRQ {} (MMIO {:#x})", irq_num, mmio_base);
            return;
        }
    }

    crate::kprintln!("[VirtIO-IRQ] Warning: IRQ table full, cannot register IRQ {}", irq_num);
}

/// VirtIO 디바이스 IRQ 해제
pub fn unregister_irq(irq_num: u32) {
    for entry in IRQ_TABLE.iter() {
        if entry.irq_num.load(Ordering::Relaxed) == irq_num {
            entry.irq_num.store(0, Ordering::Release);
            entry.mmio_base.store(0, Ordering::Relaxed);
            entry.flag.store(0, Ordering::Relaxed);
            entry.flag_high.store(0, Ordering::Relaxed);
            return;
        }
    }
}

/// VirtIO IRQ 핸들러 — PLIC/GIC에서 호출
///
/// 해당 IRQ를 처리했으면 true, 아니면 false 반환
pub fn handle_virtio_irq(irq_num: u32) -> bool {
    for entry in IRQ_TABLE.iter() {
        if entry.irq_num.load(Ordering::Acquire) == irq_num {
            let mmio_base = entry.mmio_base.load(Ordering::Relaxed) as usize;
            let mmio = VirtIOMMIO::new(mmio_base);

            let status = mmio.interrupt_status();
            if status != 0 {
                mmio.ack_interrupt(status);
                let flag_ptr = entry.get_flag_ptr();
                if !flag_ptr.is_null() {
                    // Safety: flag_ptr은 Arc<VirtIOBlock> 내부의 AtomicBool을 가리킴
                    unsafe {
                        (*flag_ptr).store(true, Ordering::SeqCst);
                    }
                }
                return true;
            }
        }
    }
    false
}
