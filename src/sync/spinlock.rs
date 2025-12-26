//! Spinlock - Busy-waiting 기반 락
//!
//! 특징:
//! - CPU가 락을 얻을 때까지 루프를 돌며 대기 (busy-waiting)
//! - 짧은 임계 구역에 적합 (수십~수백 사이클)
//! - 인터럽트 컨텍스트에서 사용 가능
//! - IRQ-safe 버전 포함 (인터럽트 비활성화)

use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};

/// Spinlock - 기본 스핀락
pub struct Spinlock<T> {
    locked: AtomicBool,
    data: UnsafeCell<T>,
}

// Send + Sync 구현 - T가 Send면 Spinlock도 스레드간 공유 가능
unsafe impl<T: Send> Send for Spinlock<T> {}
unsafe impl<T: Send> Sync for Spinlock<T> {}

impl<T> Spinlock<T> {
    /// 새 스핀락 생성
    pub const fn new(data: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            data: UnsafeCell::new(data),
        }
    }

    /// 락 획득 (블로킹)
    #[inline]
    pub fn lock(&self) -> SpinlockGuard<'_, T> {
        while self
            .locked
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            // 락이 해제될 때까지 스핀
            while self.locked.load(Ordering::Relaxed) {
                core::hint::spin_loop();
            }
        }
        SpinlockGuard { lock: self }
    }

    /// 락 시도 (논블로킹)
    #[inline]
    pub fn try_lock(&self) -> Option<SpinlockGuard<'_, T>> {
        if self
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            Some(SpinlockGuard { lock: self })
        } else {
            None
        }
    }

    /// 락이 현재 잠겨있는지 확인
    #[inline]
    pub fn is_locked(&self) -> bool {
        self.locked.load(Ordering::Relaxed)
    }

    /// 내부 데이터에 대한 가변 참조 (unsafe)
    /// 락 없이 접근 - 단일 스레드 초기화 시에만 사용
    #[inline]
    pub unsafe fn get_mut_unchecked(&self) -> &mut T {
        unsafe { &mut *self.data.get() }
    }
}

/// 스핀락 가드 - RAII 패턴으로 자동 해제
pub struct SpinlockGuard<'a, T> {
    lock: &'a Spinlock<T>,
}

impl<T> Deref for SpinlockGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.lock.data.get() }
    }
}

impl<T> DerefMut for SpinlockGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<T> Drop for SpinlockGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.locked.store(false, Ordering::Release);
    }
}

/// IRQ-safe Spinlock - 인터럽트 비활성화 포함
pub struct IrqSpinlock<T> {
    inner: Spinlock<T>,
}

unsafe impl<T: Send> Send for IrqSpinlock<T> {}
unsafe impl<T: Send> Sync for IrqSpinlock<T> {}

impl<T> IrqSpinlock<T> {
    pub const fn new(data: T) -> Self {
        Self {
            inner: Spinlock::new(data),
        }
    }

    /// 인터럽트 비활성화 후 락 획득
    #[inline]
    pub fn lock(&self) -> IrqSpinlockGuard<'_, T> {
        let irq_enabled = interrupts_enabled();
        disable_interrupts();

        let guard = self.inner.lock();
        IrqSpinlockGuard {
            guard,
            irq_was_enabled: irq_enabled,
        }
    }
}

pub struct IrqSpinlockGuard<'a, T> {
    guard: SpinlockGuard<'a, T>,
    irq_was_enabled: bool,
}

impl<T> Deref for IrqSpinlockGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &*self.guard
    }
}

impl<T> DerefMut for IrqSpinlockGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut *self.guard
    }
}

impl<T> Drop for IrqSpinlockGuard<'_, T> {
    fn drop(&mut self) {
        // guard가 먼저 drop되어 락 해제
        // 그 후 인터럽트 상태 복원
        drop(unsafe { core::ptr::read(&self.guard) });
        if self.irq_was_enabled {
            enable_interrupts();
        }
    }
}

// 아키텍처별 인터럽트 제어
#[cfg(target_arch = "aarch64")]
fn interrupts_enabled() -> bool {
    let daif: u64;
    unsafe {
        core::arch::asm!("mrs {}, DAIF", out(reg) daif);
    }
    (daif & 0x80) == 0 // I bit이 0이면 IRQ 활성화
}

#[cfg(target_arch = "aarch64")]
fn disable_interrupts() {
    unsafe {
        core::arch::asm!("msr DAIFSet, #2");
    }
}

#[cfg(target_arch = "aarch64")]
fn enable_interrupts() {
    unsafe {
        core::arch::asm!("msr DAIFClr, #2");
    }
}

#[cfg(target_arch = "riscv64")]
fn interrupts_enabled() -> bool {
    let mstatus: usize;
    unsafe {
        core::arch::asm!("csrr {}, mstatus", out(reg) mstatus);
    }
    (mstatus & 0x8) != 0 // MIE bit
}

#[cfg(target_arch = "riscv64")]
fn disable_interrupts() {
    unsafe {
        core::arch::asm!("csrc mstatus, {}", in(reg) 0x8usize);
    }
}

#[cfg(target_arch = "riscv64")]
fn enable_interrupts() {
    unsafe {
        core::arch::asm!("csrs mstatus, {}", in(reg) 0x8usize);
    }
}

/// Ticket Spinlock - 공정성 보장 (FIFO)
pub struct TicketLock<T> {
    next_ticket: core::sync::atomic::AtomicU32,
    now_serving: core::sync::atomic::AtomicU32,
    data: UnsafeCell<T>,
}

unsafe impl<T: Send> Send for TicketLock<T> {}
unsafe impl<T: Send> Sync for TicketLock<T> {}

impl<T> TicketLock<T> {
    pub const fn new(data: T) -> Self {
        Self {
            next_ticket: core::sync::atomic::AtomicU32::new(0),
            now_serving: core::sync::atomic::AtomicU32::new(0),
            data: UnsafeCell::new(data),
        }
    }

    pub fn lock(&self) -> TicketLockGuard<'_, T> {
        let ticket = self.next_ticket.fetch_add(1, Ordering::Relaxed);
        while self.now_serving.load(Ordering::Acquire) != ticket {
            core::hint::spin_loop();
        }
        TicketLockGuard { lock: self }
    }
}

pub struct TicketLockGuard<'a, T> {
    lock: &'a TicketLock<T>,
}

impl<T> Deref for TicketLockGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.lock.data.get() }
    }
}

impl<T> DerefMut for TicketLockGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<T> Drop for TicketLockGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.now_serving.fetch_add(1, Ordering::Release);
    }
}
