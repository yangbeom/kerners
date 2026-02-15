//! PL031 RTC (AArch64 QEMU virt)

use core::ptr::read_volatile;

const PL031_COMPAT: &str = "arm,pl031";
const PL031_FALLBACK_BASE: usize = 0x0901_0000;
const PL031_DR_OFFSET: usize = 0x000;

fn rtc_base() -> usize {
    if let Some(dt) = crate::dtb::get() {
        if let Some(info) = dt.find_compatible(PL031_COMPAT).into_iter().next() {
            if info.reg_base != 0 {
                return info.reg_base as usize;
            }
        }
    }
    PL031_FALLBACK_BASE
}

/// RTC epoch 시간(ns) 조회
pub fn read_epoch_ns() -> Option<u64> {
    let base = rtc_base();
    let seconds = unsafe {
        // SAFETY: MMU에 매핑된 RTC MMIO DR 레지스터(32bit)를 volatile 읽기한다.
        read_volatile((base + PL031_DR_OFFSET) as *const u32)
    } as u64;

    if seconds == 0 {
        return None;
    }

    Some(seconds.saturating_mul(1_000_000_000))
}
