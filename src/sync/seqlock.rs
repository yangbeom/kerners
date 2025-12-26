//! SeqLock - 순차 락
//!
//! 특징:
//! - Write 우선: Reader가 writer를 절대 차단하지 않음
//! - 락 없는 읽기: Reader는 락 획득 없이 읽고 나중에 검증
//! - Read-mostly 데이터 + 빠른 쓰기가 필요할 때 사용
//! - 예: 시간 정보 (jiffies, xtime), 통계 카운터
//!
//! 동작 원리:
//! 1. Writer가 sequence를 홀수로 변경 (쓰기 시작)
//! 2. 데이터 쓰기
//! 3. Writer가 sequence를 짝수로 변경 (쓰기 완료)
//! 4. Reader는 읽기 전후 sequence 비교로 유효성 검증

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicU64, Ordering, fence};
use super::spinlock::Spinlock;

/// SeqLock - 순차 락
///
/// T는 Copy여야 함 (읽기 중 변경될 수 있으므로)
pub struct SeqLock<T: Copy> {
    /// 순차 번호: 홀수=쓰기 중, 짝수=안정
    sequence: AtomicU64,
    /// Writer 간 동기화용 스핀락
    write_lock: Spinlock<()>,
    data: UnsafeCell<T>,
}

unsafe impl<T: Copy + Send> Send for SeqLock<T> {}
unsafe impl<T: Copy + Send> Sync for SeqLock<T> {}

impl<T: Copy> SeqLock<T> {
    /// 새 SeqLock 생성
    pub const fn new(data: T) -> Self {
        Self {
            sequence: AtomicU64::new(0),
            write_lock: Spinlock::new(()),
            data: UnsafeCell::new(data),
        }
    }

    /// 낙관적 읽기 (락 없음)
    ///
    /// 읽기 중 쓰기가 발생하면 자동으로 재시도
    #[inline]
    pub fn read(&self) -> T {
        loop {
            // 1. 쓰기 중이 아닐 때까지 대기
            let seq1 = self.read_begin();
            if seq1 == u64::MAX {
                core::hint::spin_loop();
                continue;
            }

            // 2. 데이터 복사
            let data = unsafe { *self.data.get() };

            // 3. 읽기 완료 후 sequence 확인
            if self.read_validate(seq1) {
                return data;
            }
            // 4. 변경되었으면 재시도
        }
    }

    /// 읽기 시작 - sequence 반환
    ///
    /// 쓰기 중이면 u64::MAX 반환
    #[inline]
    pub fn read_begin(&self) -> u64 {
        let seq = self.sequence.load(Ordering::Acquire);
        if seq & 1 != 0 {
            u64::MAX // 쓰기 중
        } else {
            seq
        }
    }

    /// 읽기 검증 - 읽기 중 변경 없었는지 확인
    #[inline]
    pub fn read_validate(&self, start_seq: u64) -> bool {
        fence(Ordering::Acquire);
        self.sequence.load(Ordering::Relaxed) == start_seq
    }

    /// 쓰기 (락 획득)
    #[inline]
    pub fn write(&self, value: T) {
        let _guard = self.write_lock.lock();
        self.write_begin();
        unsafe { *self.data.get() = value; }
        self.write_end();
    }

    /// 쓰기 시작 (sequence를 홀수로)
    #[inline]
    fn write_begin(&self) {
        self.sequence.fetch_add(1, Ordering::Release);
        fence(Ordering::Release);
    }

    /// 쓰기 완료 (sequence를 짝수로)
    #[inline]
    fn write_end(&self) {
        fence(Ordering::Release);
        self.sequence.fetch_add(1, Ordering::Release);
    }

    /// 쓰기 가드 획득 (RAII 패턴)
    #[inline]
    pub fn write_guard(&self) -> SeqLockWriteGuard<'_, T> {
        let guard = self.write_lock.lock();
        self.write_begin();
        SeqLockWriteGuard { 
            lock: self,
            _guard: guard,
        }
    }

    /// 현재 sequence 값 조회 (디버깅용)
    #[inline]
    pub fn sequence(&self) -> u64 {
        self.sequence.load(Ordering::Relaxed)
    }
}

/// SeqLock 쓰기 가드
pub struct SeqLockWriteGuard<'a, T: Copy> {
    lock: &'a SeqLock<T>,
    _guard: super::spinlock::SpinlockGuard<'a, ()>,
}

impl<T: Copy> core::ops::Deref for SeqLockWriteGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.lock.data.get() }
    }
}

impl<T: Copy> core::ops::DerefMut for SeqLockWriteGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<T: Copy> Drop for SeqLockWriteGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.write_end();
        // _guard가 drop되면서 write_lock 해제
    }
}

/// SeqCounter - 단순 증가 카운터용 특수화
///
/// 데이터 복사 없이 sequence만 사용
pub struct SeqCounter {
    sequence: AtomicU64,
}

impl SeqCounter {
    pub const fn new() -> Self {
        Self {
            sequence: AtomicU64::new(0),
        }
    }

    /// 카운터 증가 (쓰기)
    #[inline]
    pub fn increment(&self) {
        self.sequence.fetch_add(2, Ordering::Release);
    }

    /// 현재 값 읽기
    #[inline]
    pub fn read(&self) -> u64 {
        self.sequence.load(Ordering::Acquire) / 2
    }

    /// 변경 감지용 스냅샷
    #[inline]
    pub fn snapshot(&self) -> u64 {
        self.sequence.load(Ordering::Acquire)
    }

    /// 스냅샷 이후 변경 여부 확인
    #[inline]
    pub fn changed_since(&self, snapshot: u64) -> bool {
        self.sequence.load(Ordering::Acquire) != snapshot
    }
}

/// 시간 정보를 위한 SeqLock 특수화
#[repr(C)]
#[derive(Clone, Copy)]
pub struct TimeSpec {
    pub sec: u64,
    pub nsec: u64,
}

impl TimeSpec {
    pub const fn new(sec: u64, nsec: u64) -> Self {
        Self { sec, nsec }
    }

    pub const fn zero() -> Self {
        Self { sec: 0, nsec: 0 }
    }
}

/// SeqLock 기반 시스템 시간
pub type TimeSeqLock = SeqLock<TimeSpec>;

impl TimeSeqLock {
    /// 현재 시간 조회 (락 프리)
    pub fn get_time(&self) -> TimeSpec {
        self.read()
    }

    /// 시간 설정
    pub fn set_time(&self, sec: u64, nsec: u64) {
        self.write(TimeSpec::new(sec, nsec));
    }

    /// 시간 업데이트 (원자적 증가)
    pub fn update_time(&self, add_nsec: u64) {
        let guard = self.write_guard();
        let mut time = *guard;
        time.nsec += add_nsec;
        while time.nsec >= 1_000_000_000 {
            time.sec += 1;
            time.nsec -= 1_000_000_000;
        }
        drop(guard);
        self.write(time);
    }
}
