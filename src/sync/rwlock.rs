//! RwLock - Reader-Writer 락
//!
//! 특징:
//! - 다중 reader 동시 접근 허용
//! - Writer는 배타적 접근 (단독)
//! - 읽기 많은 워크로드에 최적화
//!
//! 정책: Writer 우선 (writer 대기 시 새 reader 차단)

use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicI32, AtomicBool, Ordering};

/// RwLock 상태 값
/// - 양수: reader 수
/// - 0: 아무도 없음
/// - -1: writer가 보유
const WRITER: i32 = -1;

/// RwLock - Reader-Writer 락
pub struct RwLock<T> {
    /// 락 상태: 양수=reader수, 0=비어있음, -1=writer
    state: AtomicI32,
    /// Writer가 대기 중인지 (reader 우선순위 낮추기 위해)
    writer_waiting: AtomicBool,
    data: UnsafeCell<T>,
}

unsafe impl<T: Send> Send for RwLock<T> {}
unsafe impl<T: Send + Sync> Sync for RwLock<T> {}

impl<T> RwLock<T> {
    /// 새 RwLock 생성
    pub const fn new(data: T) -> Self {
        Self {
            state: AtomicI32::new(0),
            writer_waiting: AtomicBool::new(false),
            data: UnsafeCell::new(data),
        }
    }

    /// 읽기 락 획득
    ///
    /// Writer가 대기 중이면 양보 (writer 우선 정책)
    #[inline]
    pub fn read(&self) -> ReadGuard<'_, T> {
        loop {
            // Writer가 대기 중이면 양보
            if self.writer_waiting.load(Ordering::Relaxed) {
                core::hint::spin_loop();
                continue;
            }

            let state = self.state.load(Ordering::Relaxed);

            // Writer가 보유 중이면 대기
            if state < 0 {
                core::hint::spin_loop();
                continue;
            }

            // Reader 수 증가 시도
            if self
                .state
                .compare_exchange_weak(state, state + 1, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                return ReadGuard { lock: self };
            }
        }
    }

    /// 읽기 락 시도 (논블로킹)
    #[inline]
    pub fn try_read(&self) -> Option<ReadGuard<'_, T>> {
        if self.writer_waiting.load(Ordering::Relaxed) {
            return None;
        }

        let state = self.state.load(Ordering::Relaxed);
        if state >= 0 {
            if self
                .state
                .compare_exchange(state, state + 1, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                return Some(ReadGuard { lock: self });
            }
        }
        None
    }

    /// 쓰기 락 획득
    ///
    /// 배타적 접근 - 모든 reader와 다른 writer 차단
    #[inline]
    pub fn write(&self) -> WriteGuard<'_, T> {
        // Writer 대기 플래그 설정 (새 reader 차단)
        self.writer_waiting.store(true, Ordering::Relaxed);

        loop {
            // state가 0이면 (아무도 없으면) writer로 설정
            if self
                .state
                .compare_exchange_weak(0, WRITER, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                self.writer_waiting.store(false, Ordering::Relaxed);
                return WriteGuard { lock: self };
            }
            core::hint::spin_loop();
        }
    }

    /// 쓰기 락 시도 (논블로킹)
    #[inline]
    pub fn try_write(&self) -> Option<WriteGuard<'_, T>> {
        if self
            .state
            .compare_exchange(0, WRITER, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            Some(WriteGuard { lock: self })
        } else {
            None
        }
    }

    /// 현재 reader 수 반환 (디버깅용)
    #[inline]
    pub fn reader_count(&self) -> i32 {
        let state = self.state.load(Ordering::Relaxed);
        if state > 0 { state } else { 0 }
    }

    /// Writer가 락을 보유 중인지 확인
    #[inline]
    pub fn is_write_locked(&self) -> bool {
        self.state.load(Ordering::Relaxed) == WRITER
    }
}

/// 읽기 락 가드
pub struct ReadGuard<'a, T> {
    lock: &'a RwLock<T>,
}

impl<T> Deref for ReadGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.lock.data.get() }
    }
}

impl<T> Drop for ReadGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.state.fetch_sub(1, Ordering::Release);
    }
}

/// 쓰기 락 가드
pub struct WriteGuard<'a, T> {
    lock: &'a RwLock<T>,
}

impl<T> Deref for WriteGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.lock.data.get() }
    }
}

impl<T> DerefMut for WriteGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<T> Drop for WriteGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.state.store(0, Ordering::Release);
    }
}

/// RwLock을 읽기 가드에서 쓰기 가드로 업그레이드
/// 
/// 주의: 데드락 위험 - 다른 reader가 있으면 영원히 대기
impl<'a, T> ReadGuard<'a, T> {
    /// 읽기 락을 쓰기 락으로 업그레이드 시도
    /// 
    /// 자신이 유일한 reader일 때만 성공
    pub fn try_upgrade(self) -> Result<WriteGuard<'a, T>, Self> {
        let lock = self.lock;
        // 현재 reader가 1명(자신)이면 writer로 전환
        if lock
            .state
            .compare_exchange(1, WRITER, Ordering::AcqRel, Ordering::Relaxed)
            .is_ok()
        {
            // ReadGuard의 drop 방지 (이미 state 변경함)
            core::mem::forget(self);
            Ok(WriteGuard { lock })
        } else {
            Err(self)
        }
    }
}
