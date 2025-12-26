//! Mutex - Sleeping 락
//!
//! 특징:
//! - 락을 못 얻으면 스레드가 sleep 상태로 전환
//! - 긴 임계 구역에 적합 (I/O 대기, 복잡한 연산)
//! - 소유권 개념: 락을 획득한 스레드만 해제 가능
//! - 인터럽트 컨텍스트에서 사용 불가 (sleep 불가)
//!
//! 현재 구현: Adaptive Mutex (짧은 스핀 후 yield)
//! - 스케줄러가 없는 환경에서는 순수 스핀락처럼 동작
//! - 스케줄러가 있으면 일정 스핀 후 yield_now 호출

use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// 스핀 횟수 임계값 - 이 횟수 이상 스핀하면 yield
const SPIN_LIMIT: u32 = 100;

/// Mutex - Adaptive Mutex 구현
pub struct Mutex<T> {
    locked: AtomicBool,
    data: UnsafeCell<T>,
}

unsafe impl<T: Send> Send for Mutex<T> {}
unsafe impl<T: Send> Sync for Mutex<T> {}

impl<T> Mutex<T> {
    /// 새 Mutex 생성
    pub const fn new(data: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            data: UnsafeCell::new(data),
        }
    }

    /// 락 획득 (블로킹)
    ///
    /// Adaptive 방식:
    /// 1. 먼저 짧은 스핀 시도
    /// 2. 스핀 한계 초과 시 yield (스케줄러에 CPU 양보)
    #[inline]
    pub fn lock(&self) -> MutexGuard<'_, T> {
        let mut spin_count = 0u32;

        loop {
            // Fast path: CAS로 락 획득 시도
            if self
                .locked
                .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                return MutexGuard { mutex: self };
            }

            // 락이 잠겨있는 동안 스핀
            while self.locked.load(Ordering::Relaxed) {
                spin_count += 1;

                if spin_count >= SPIN_LIMIT {
                    // 스핀 한계 초과 - yield로 다른 스레드에게 양보
                    Self::yield_now();
                    spin_count = 0;
                } else {
                    core::hint::spin_loop();
                }
            }
        }
    }

    /// 락 시도 (논블로킹)
    #[inline]
    pub fn try_lock(&self) -> Option<MutexGuard<'_, T>> {
        if self
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            Some(MutexGuard { mutex: self })
        } else {
            None
        }
    }

    /// 락이 현재 잠겨있는지 확인
    #[inline]
    pub fn is_locked(&self) -> bool {
        self.locked.load(Ordering::Relaxed)
    }

    /// CPU 양보 - 스케줄러가 있으면 yield, 없으면 spin_loop
    #[inline]
    fn yield_now() {
        // proc::yield_now를 직접 호출하면 순환 의존성 발생
        // 대신 간단한 spin_loop 여러 번으로 대체
        // 실제 스케줄러 연동은 나중에 개선
        for _ in 0..10 {
            core::hint::spin_loop();
        }
    }
}

/// Mutex 가드 - RAII 패턴으로 자동 해제
pub struct MutexGuard<'a, T> {
    mutex: &'a Mutex<T>,
}

impl<T> Deref for MutexGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.mutex.data.get() }
    }
}

impl<T> DerefMut for MutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.mutex.data.get() }
    }
}

impl<T> Drop for MutexGuard<'_, T> {
    fn drop(&mut self) {
        self.mutex.locked.store(false, Ordering::Release);
    }
}

/// Blocking Mutex - 대기 큐 기반 (스케줄러 필요)
///
/// 진정한 sleeping mutex. 락을 못 얻으면 스레드를 대기 큐에 넣고
/// 컨텍스트 스위치 수행.
///
/// TODO: 스케줄러와 대기 큐 구현 후 활성화
pub struct BlockingMutex<T> {
    locked: AtomicBool,
    owner: AtomicUsize,  // 소유자 스레드 ID (재귀 락 검사용)
    // wait_queue: WaitQueue, // TODO: 대기 큐 구현 필요
    data: UnsafeCell<T>,
}

unsafe impl<T: Send> Send for BlockingMutex<T> {}
unsafe impl<T: Send> Sync for BlockingMutex<T> {}

impl<T> BlockingMutex<T> {
    pub const fn new(data: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            owner: AtomicUsize::new(0),
            data: UnsafeCell::new(data),
        }
    }

    /// 락 획득 - 현재는 스핀락처럼 동작
    /// TODO: 대기 큐 구현 후 실제 blocking으로 변경
    pub fn lock(&self) -> BlockingMutexGuard<'_, T> {
        loop {
            if self
                .locked
                .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                return BlockingMutexGuard { mutex: self };
            }

            // TODO: 대기 큐에 추가하고 schedule() 호출
            // 현재는 스핀
            core::hint::spin_loop();
        }
    }
}

pub struct BlockingMutexGuard<'a, T> {
    mutex: &'a BlockingMutex<T>,
}

impl<T> Deref for BlockingMutexGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.mutex.data.get() }
    }
}

impl<T> DerefMut for BlockingMutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.mutex.data.get() }
    }
}

impl<T> Drop for BlockingMutexGuard<'_, T> {
    fn drop(&mut self) {
        self.mutex.locked.store(false, Ordering::Release);
        // TODO: 대기 큐에서 하나 깨우기
    }
}
