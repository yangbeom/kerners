//! Goldfish RTC (RISC-V QEMU virt)

use core::ptr::read_volatile;

const GOLDFISH_RTC_COMPAT: &str = "google,goldfish-rtc";
const GOLDFISH_RTC_FALLBACK_BASE: usize = 0x0010_1000;
const TIME_LOW_OFFSET: usize = 0x0;
const TIME_HIGH_OFFSET: usize = 0x4;

fn rtc_base() -> usize {
    if let Some(dt) = crate::dtb::get() {
        if let Some(info) = dt.find_compatible(GOLDFISH_RTC_COMPAT).into_iter().next() {
            if info.reg_base != 0 {
                return info.reg_base as usize;
            }
        }
    }
    GOLDFISH_RTC_FALLBACK_BASE
}

/// RTC epoch 시간(ns) 조회
pub fn read_epoch_ns() -> Option<u64> {
    let base = rtc_base();

    // high-low-high 순서로 읽어 64비트 값을 안정적으로 구성한다.
    let (high1, low, high2) = unsafe {
        // SAFETY: MMU에 매핑된 goldfish-rtc MMIO 레지스터를 volatile 읽기한다.
        (
            read_volatile((base + TIME_HIGH_OFFSET) as *const u32),
            read_volatile((base + TIME_LOW_OFFSET) as *const u32),
            read_volatile((base + TIME_HIGH_OFFSET) as *const u32),
        )
    };

    let high = if high1 == high2 {
        high1
    } else {
        unsafe {
            // SAFETY: 위와 동일, 값이 바뀐 경우 재읽기한다.
            read_volatile((base + TIME_HIGH_OFFSET) as *const u32)
        }
    } as u64;

    let ns = (high << 32) | (low as u64);
    if ns == 0 {
        None
    } else {
        Some(ns)
    }
}
