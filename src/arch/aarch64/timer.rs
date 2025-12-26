//! ARM Generic Timer 드라이버
//! 
//! ARM Generic Timer는 시스템 타이머로, EL1에서 사용할 수 있는 Physical Timer를 제공합니다.
//! 
//! 주요 레지스터:
//! - CNTFRQ_EL0: 타이머 주파수 (Hz)
//! - CNTPCT_EL0: 현재 카운터 값 (읽기 전용)
//! - CNTP_TVAL_EL0: 타이머 값 (다운카운터)
//! - CNTP_CTL_EL0: 타이머 제어 레지스터
//! - CNTP_CVAL_EL0: 비교 값 (절대 시간)

use core::arch::asm;
use core::sync::atomic::{AtomicU64, Ordering};
use crate::kprintln;

/// 타이머 틱 간격 (밀리초)
const TIMER_TICK_MS: u64 = 10;

/// 전역 타이머 틱 카운터 (SMP-safe)
static TIMER_TICKS: AtomicU64 = AtomicU64::new(0);

/// 타이머 주파수 읽기
#[inline]
pub fn get_frequency() -> u64 {
    let freq: u64;
    unsafe {
        asm!("mrs {}, CNTFRQ_EL0", out(reg) freq);
    }
    freq
}

/// 현재 카운터 값 읽기
#[inline]
pub fn get_counter() -> u64 {
    let cnt: u64;
    unsafe {
        asm!("mrs {}, CNTPCT_EL0", out(reg) cnt);
    }
    cnt
}

/// 타이머 제어 레지스터 읽기
#[inline]
fn get_ctl() -> u32 {
    let ctl: u64;
    unsafe {
        asm!("mrs {}, CNTP_CTL_EL0", out(reg) ctl);
    }
    ctl as u32
}

/// 타이머 제어 레지스터 쓰기
#[inline]
fn set_ctl(ctl: u32) {
    unsafe {
        asm!("msr CNTP_CTL_EL0, {}", in(reg) ctl as u64);
    }
}

/// 타이머 값 설정 (다운카운터)
#[inline]
fn set_tval(tval: u32) {
    unsafe {
        asm!("msr CNTP_TVAL_EL0, {}", in(reg) tval as u64);
    }
}

/// 타이머 비교 값 설정 (절대 시간)
#[inline]
fn set_cval(cval: u64) {
    unsafe {
        asm!("msr CNTP_CVAL_EL0, {}", in(reg) cval);
    }
}

/// 타이머 활성화
pub fn enable() {
    let mut ctl = get_ctl();
    ctl |= 1; // ENABLE bit
    ctl &= !2; // IMASK bit clear (인터럽트 활성화)
    set_ctl(ctl);
}

/// 타이머 비활성화
pub fn disable() {
    let mut ctl = get_ctl();
    ctl &= !1; // ENABLE bit clear
    set_ctl(ctl);
}

/// 타이머 상태 확인
pub fn is_enabled() -> bool {
    get_ctl() & 1 != 0
}

/// 타이머 인터럽트 pending 확인
pub fn is_pending() -> bool {
    get_ctl() & 4 != 0 // ISTATUS bit
}

/// 다음 타이머 인터럽트 설정
pub fn set_next_tick() {
    let freq = get_frequency();
    let ticks = (freq * TIMER_TICK_MS) / 1000;
    set_tval(ticks as u32);
}

/// 타이머 초기화
pub fn init() -> Result<(), &'static str> {
    kprintln!("\n[Timer] Initializing Generic Timer...");
    
    let freq = get_frequency();
    if freq == 0 {
        return Err("Timer frequency is 0");
    }
    
    kprintln!("[Timer] Frequency: {} Hz", freq);
    kprintln!("[Timer] Tick interval: {} ms", TIMER_TICK_MS);
    
    // 타이머 비활성화
    disable();
    
    // 다음 틱 설정
    set_next_tick();
    
    // 타이머 활성화
    enable();
    
    kprintln!("[Timer] Timer enabled");
    
    Ok(())
}

/// Secondary CPU 타이머 초기화
///
/// Secondary CPU는 자체 Physical Timer를 가지고 있으므로
/// 각각 독립적으로 타이머를 설정합니다.
pub fn init_secondary() {
    disable();
    set_next_tick();
    enable();
}

/// 타이머 인터럽트 핸들러
pub fn handle_irq() {
    TIMER_TICKS.fetch_add(1, Ordering::Relaxed);

    // 다음 틱 설정
    set_next_tick();

    // Per-CPU 틱 카운터 업데이트
    crate::proc::percpu::current().tick_count.fetch_add(1, Ordering::Relaxed);

    // 선점 스케줄링: 타이머 틱마다 스케줄러 호출
    crate::proc::scheduler::schedule();
}

/// 현재 틱 수 반환
pub fn ticks() -> u64 {
    TIMER_TICKS.load(Ordering::Relaxed)
}

/// 밀리초 단위로 대기 (폴링 방식)
pub fn delay_ms(ms: u64) {
    let freq = get_frequency();
    let start = get_counter();
    let ticks = (freq * ms) / 1000;
    
    while get_counter() - start < ticks {
        core::hint::spin_loop();
    }
}

/// 마이크로초 단위로 대기 (폴링 방식)
pub fn delay_us(us: u64) {
    let freq = get_frequency();
    let start = get_counter();
    let ticks = (freq * us) / 1_000_000;
    
    while get_counter() - start < ticks {
        core::hint::spin_loop();
    }
}
