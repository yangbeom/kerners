//! RISC-V CLINT (Core Local Interruptor) 타이머 드라이버
//! 
//! CLINT는 타이머와 소프트웨어 인터럽트를 제공합니다.
//! 
//! 레지스터:
//! - mtime (0x0200_BFF8): 현재 시간 카운터
//! - mtimecmp (0x0200_4000 + hartid*8): 비교 값

use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{AtomicU64, Ordering};
use crate::kprintln;

/// CLINT 베이스 주소 얻기
#[inline]
fn clint_base() -> usize {
    if crate::drivers::config::is_initialized() {
        crate::drivers::config::clint_base()
    } else {
        crate::boards::clint_base()
    }
}

/// 타이머 주파수 얻기
#[inline]
fn timer_freq() -> u64 {
    if crate::drivers::config::is_initialized() {
        crate::drivers::config::timer_freq()
    } else {
        crate::boards::timer_freq()
    }
}

/// mtime 레지스터 오프셋 (64-bit)
const MTIME_OFFSET: usize = 0xBFF8;

/// mtimecmp 레지스터 오프셋 (64-bit, hartid=0)
const MTIMECMP_OFFSET: usize = 0x4000;

/// 타이머 틱 간격 (밀리초)
const TIMER_TICK_MS: u64 = 10;

/// 전역 타이머 틱 카운터 (SMP-safe)
static TIMER_TICKS: AtomicU64 = AtomicU64::new(0);

/// mtime 읽기
#[inline]
fn read_mtime() -> u64 {
    unsafe { read_volatile((clint_base() + MTIME_OFFSET) as *const u64) }
}

/// mtimecmp 쓰기
#[inline]
fn write_mtimecmp(value: u64) {
    unsafe { write_volatile((clint_base() + MTIMECMP_OFFSET) as *mut u64, value); }
}

/// 다음 타이머 인터럽트 설정
pub fn set_next_tick() {
    let current = read_mtime();
    let ticks = (timer_freq() * TIMER_TICK_MS) / 1000;
    write_mtimecmp(current + ticks);
}

/// 타이머 초기화
pub fn init() -> Result<(), &'static str> {
    kprintln!("\n[Timer] Initializing CLINT timer...");

    kprintln!("[Timer] Frequency: {} Hz", timer_freq());
    kprintln!("[Timer] Tick interval: {} ms", TIMER_TICK_MS);
    
    // 첫 타이머 인터럽트 설정
    set_next_tick();
    
    // MIE (Machine Interrupt Enable)에서 타이머 인터럽트 활성화
    unsafe {
        core::arch::asm!(
            "li t0, 0x80",      // MTIE (Machine Timer Interrupt Enable)
            "csrs mie, t0"
        );
    }
    
    kprintln!("[Timer] Timer enabled");
    
    Ok(())
}

/// Secondary hart의 mtimecmp 쓰기
#[inline]
fn write_mtimecmp_hart(hartid: u32, value: u64) {
    let offset = MTIMECMP_OFFSET + (hartid as usize) * 8;
    unsafe { write_volatile((clint_base() + offset) as *mut u64, value); }
}

/// Secondary hart 타이머 초기화
pub fn init_secondary() {
    let hartid = crate::proc::percpu::get_cpu_id();
    let current = read_mtime();
    let ticks = (timer_freq() * TIMER_TICK_MS) / 1000;
    write_mtimecmp_hart(hartid, current + ticks);

    // MIE에서 타이머 인터럽트 활성화
    unsafe {
        core::arch::asm!(
            "li t0, 0x80",      // MTIE
            "csrs mie, t0"
        );
    }
}

/// 타이머 인터럽트 핸들러
pub fn handle_irq() {
    let ticks = TIMER_TICKS.fetch_add(1, Ordering::Relaxed) + 1;

    // 다음 틱 설정
    set_next_tick();

    // Per-CPU 틱 카운터 업데이트
    crate::proc::percpu::current().tick_count.fetch_add(1, Ordering::Relaxed);

    // 1초마다 출력 (디버그용, primary hart만)
    if ticks % 100 == 0 && crate::proc::percpu::get_cpu_id() == 0 {
        let seconds = ticks / 100;
        kprintln!("[Timer] {} seconds elapsed", seconds);
    }

    let _ = crate::proc::wake_sleepers_by_timer(crate::time::monotonic_now_ns());

    // NOTE:
    // RISC-V trap 컨텍스트 안에서 직접 context_switch를 수행하면
    // 스레드의 복귀 주소가 trap 복귀 경로로 오염될 수 있어
    // 테스트 중 instruction access fault가 발생한다.
    //
    // 따라서 타이머 IRQ에서는 wakeup/tick 처리만 수행하고,
    // 실제 스케줄링 전환은 yield/syscall 경로에서 이뤄지도록 둔다.
}

/// 현재 틱 수 반환
pub fn ticks() -> u64 {
    TIMER_TICKS.load(Ordering::Relaxed)
}

/// 현재 시간 카운터 읽기
pub fn get_time() -> u64 {
    read_mtime()
}

/// 밀리초 단위로 대기 (폴링 방식)
pub fn delay_ms(ms: u64) {
    let start = read_mtime();
    let ticks = (timer_freq() * ms) / 1000;

    while read_mtime() - start < ticks {
        core::hint::spin_loop();
    }
}
