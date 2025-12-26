//! Semaphore - 카운팅 세마포어
//!
//! 특징:
//! - 카운터 기반: N개의 스레드가 동시 접근 가능
//! - 소유권 없음: 다른 스레드가 해제 가능
//! - 생산자-소비자 패턴에 적합
//! - 리소스 풀 관리에 사용

use core::sync::atomic::{AtomicIsize, Ordering};

/// Semaphore - 카운팅 세마포어
pub struct Semaphore {
    /// 현재 카운터 값 (0 이상이면 획득 가능)
    count: AtomicIsize,
}

impl Semaphore {
    /// 새 세마포어 생성
    ///
    /// # Arguments
    /// * `initial` - 초기 카운터 값 (동시 접근 가능한 수)
    pub const fn new(initial: isize) -> Self {
        Self {
            count: AtomicIsize::new(initial),
        }
    }

    /// Binary 세마포어 생성 (Mutex와 유사)
    pub const fn binary() -> Self {
        Self::new(1)
    }

    /// P 연산 (wait, acquire, down)
    ///
    /// 카운터가 양수가 될 때까지 대기 후 1 감소
    #[inline]
    pub fn acquire(&self) {
        loop {
            let count = self.count.load(Ordering::Relaxed);

            if count > 0 {
                // 카운터 감소 시도
                if self
                    .count
                    .compare_exchange_weak(
                        count,
                        count - 1,
                        Ordering::Acquire,
                        Ordering::Relaxed,
                    )
                    .is_ok()
                {
                    return;
                }
            } else {
                // 카운터가 0 이하면 대기
                // TODO: 대기 큐 구현 후 sleep으로 변경
                core::hint::spin_loop();
            }
        }
    }

    /// P 연산 시도 (논블로킹)
    #[inline]
    pub fn try_acquire(&self) -> bool {
        let count = self.count.load(Ordering::Relaxed);
        if count > 0 {
            self.count
                .compare_exchange(count, count - 1, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
        } else {
            false
        }
    }

    /// V 연산 (signal, release, up)
    ///
    /// 카운터를 1 증가
    #[inline]
    pub fn release(&self) {
        self.count.fetch_add(1, Ordering::Release);
        // TODO: 대기 큐에서 하나 깨우기
    }

    /// 현재 카운터 값 조회
    #[inline]
    pub fn available(&self) -> isize {
        self.count.load(Ordering::Relaxed)
    }

    /// P 연산 (다른 이름들)
    #[inline]
    pub fn wait(&self) {
        self.acquire();
    }

    #[inline]
    pub fn down(&self) {
        self.acquire();
    }

    #[inline]
    pub fn p(&self) {
        self.acquire();
    }

    /// V 연산 (다른 이름들)
    #[inline]
    pub fn signal(&self) {
        self.release();
    }

    #[inline]
    pub fn up(&self) {
        self.release();
    }

    #[inline]
    pub fn v(&self) {
        self.release();
    }
}

/// BinarySemaphore - 0 또는 1만 허용
///
/// Mutex와 유사하지만 소유권 개념이 없음
/// 다른 스레드가 해제 가능
pub struct BinarySemaphore {
    inner: Semaphore,
}

impl BinarySemaphore {
    pub const fn new(available: bool) -> Self {
        Self {
            inner: Semaphore::new(if available { 1 } else { 0 }),
        }
    }

    /// 세마포어 획득 (0이면 대기)
    #[inline]
    pub fn acquire(&self) {
        self.inner.acquire();
    }

    /// 세마포어 해제 (1로 설정)
    /// 
    /// 주의: 이미 1이어도 1 유지 (오버플로우 방지)
    #[inline]
    pub fn release(&self) {
        // 최대 1까지만 허용
        loop {
            let count = self.inner.count.load(Ordering::Relaxed);
            if count >= 1 {
                return; // 이미 available
            }
            if self
                .inner
                .count
                .compare_exchange_weak(count, 1, Ordering::Release, Ordering::Relaxed)
                .is_ok()
            {
                return;
            }
        }
    }

    /// 현재 사용 가능한지 확인
    #[inline]
    pub fn is_available(&self) -> bool {
        self.inner.available() > 0
    }
}

/// ResourcePool - 세마포어를 이용한 리소스 풀 관리
///
/// 사용 예: 데이터베이스 연결 풀, 버퍼 풀
pub struct ResourcePool<T, const N: usize> {
    semaphore: Semaphore,
    resources: [core::cell::UnsafeCell<Option<T>>; N],
    available: [core::sync::atomic::AtomicBool; N],
}

unsafe impl<T: Send, const N: usize> Send for ResourcePool<T, N> {}
unsafe impl<T: Send, const N: usize> Sync for ResourcePool<T, N> {}

impl<T, const N: usize> ResourcePool<T, N> {
    /// 빈 리소스 풀 생성
    pub const fn empty() -> Self {
        const NONE: core::cell::UnsafeCell<Option<()>> = core::cell::UnsafeCell::new(None);
        const FALSE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

        // 타입 캐스팅을 위한 트릭
        Self {
            semaphore: Semaphore::new(0),
            resources: unsafe { core::mem::transmute_copy(&[NONE; N]) },
            available: [FALSE; N],
        }
    }

    /// 리소스 추가
    pub fn add(&self, resource: T) -> Result<(), T> {
        for i in 0..N {
            if !self.available[i].load(Ordering::Relaxed) {
                unsafe {
                    *self.resources[i].get() = Some(resource);
                }
                self.available[i].store(true, Ordering::Release);
                self.semaphore.release();
                return Ok(());
            }
        }
        Err(resource) // 풀이 가득 참
    }

    /// 리소스 획득 (블로킹)
    pub fn acquire(&self) -> PoolGuard<'_, T, N> {
        self.semaphore.acquire();

        // 사용 가능한 리소스 찾기
        loop {
            for i in 0..N {
                if self.available[i]
                    .compare_exchange(true, false, Ordering::Acquire, Ordering::Relaxed)
                    .is_ok()
                {
                    return PoolGuard { pool: self, index: i };
                }
            }
            core::hint::spin_loop();
        }
    }

    /// 리소스 획득 시도 (논블로킹)
    pub fn try_acquire(&self) -> Option<PoolGuard<'_, T, N>> {
        if !self.semaphore.try_acquire() {
            return None;
        }

        for i in 0..N {
            if self.available[i]
                .compare_exchange(true, false, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                return Some(PoolGuard { pool: self, index: i });
            }
        }
        
        // 실패 시 세마포어 복원
        self.semaphore.release();
        None
    }
}

/// 리소스 풀 가드 - RAII 패턴으로 자동 반환
pub struct PoolGuard<'a, T, const N: usize> {
    pool: &'a ResourcePool<T, N>,
    index: usize,
}

impl<T, const N: usize> core::ops::Deref for PoolGuard<'_, T, N> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { (*self.pool.resources[self.index].get()).as_ref().unwrap() }
    }
}

impl<T, const N: usize> core::ops::DerefMut for PoolGuard<'_, T, N> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { (*self.pool.resources[self.index].get()).as_mut().unwrap() }
    }
}

impl<T, const N: usize> Drop for PoolGuard<'_, T, N> {
    fn drop(&mut self) {
        self.pool.available[self.index].store(true, Ordering::Release);
        self.pool.semaphore.release();
    }
}
