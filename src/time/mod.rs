//! 커널 시간 코어
//!
//! - MONOTONIC: 아키텍처 타이머 카운터 기반
//! - REALTIME: 부팅 시 RTC 스냅샷 + MONOTONIC 오프셋

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

static REALTIME_OFFSET_NS: AtomicU64 = AtomicU64::new(0);
static CLOCK_RES_NS: AtomicU64 = AtomicU64::new(1);
static RTC_AVAILABLE: AtomicBool = AtomicBool::new(false);

#[inline]
fn monotonic_counter_and_freq() -> (u64, u64) {
    #[cfg(target_arch = "aarch64")]
    {
        (
            crate::arch::timer::get_counter(),
            crate::arch::timer::get_frequency(),
        )
    }

    #[cfg(target_arch = "riscv64")]
    {
        (
            crate::arch::timer::get_time(),
            crate::drivers::config::timer_freq(),
        )
    }

    #[cfg(not(any(target_arch = "aarch64", target_arch = "riscv64")))]
    {
        (0, 0)
    }
}

#[inline]
fn counter_to_ns(counter: u64, freq: u64) -> u64 {
    if freq == 0 {
        return 0;
    }

    let sec = counter / freq;
    let rem = counter % freq;
    sec.saturating_mul(1_000_000_000)
        .saturating_add(rem.saturating_mul(1_000_000_000) / freq)
}

pub fn init() {
    let (_, freq) = monotonic_counter_and_freq();
    let res_ns = if freq == 0 {
        1
    } else {
        // ceil(1e9 / freq), 최소 1ns
        ((1_000_000_000u64.saturating_add(freq - 1)) / freq).max(1)
    };
    CLOCK_RES_NS.store(res_ns, Ordering::Release);

    let boot_mono_ns = monotonic_now_ns();
    let rtc_ns = crate::arch::rtc::read_epoch_ns();
    match rtc_ns {
        Some(epoch_ns) if epoch_ns >= boot_mono_ns => {
            REALTIME_OFFSET_NS.store(epoch_ns - boot_mono_ns, Ordering::Release);
            RTC_AVAILABLE.store(true, Ordering::Release);
            crate::kprintln!(
                "[time] realtime initialized from RTC: epoch_ns={}, mono_ns={}, res_ns={}",
                epoch_ns,
                boot_mono_ns,
                res_ns
            );
        }
        Some(epoch_ns) => {
            REALTIME_OFFSET_NS.store(0, Ordering::Release);
            RTC_AVAILABLE.store(false, Ordering::Release);
            crate::kprintln!(
                "[time] rtc value behind monotonic (rtc_ns={}, mono_ns={}), fallback to monotonic",
                epoch_ns,
                boot_mono_ns
            );
        }
        None => {
            REALTIME_OFFSET_NS.store(0, Ordering::Release);
            RTC_AVAILABLE.store(false, Ordering::Release);
            crate::kprintln!(
                "[time] rtc unavailable, CLOCK_REALTIME falls back to CLOCK_MONOTONIC (res_ns={})",
                res_ns
            );
        }
    }
}

#[inline]
pub fn rtc_available() -> bool {
    RTC_AVAILABLE.load(Ordering::Acquire)
}

#[inline]
pub fn monotonic_now_ns() -> u64 {
    let (counter, freq) = monotonic_counter_and_freq();
    counter_to_ns(counter, freq)
}

#[inline]
pub fn realtime_now_ns() -> u64 {
    monotonic_now_ns().saturating_add(REALTIME_OFFSET_NS.load(Ordering::Acquire))
}

#[inline]
pub fn clock_res_ns() -> u64 {
    CLOCK_RES_NS.load(Ordering::Acquire).max(1)
}
