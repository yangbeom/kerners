//! RCU (Read-Copy-Update) - 락 프리 읽기
//!
//! 특징:
//! - 락 없는 읽기: Reader는 완전히 락 프리
//! - 지연된 해제: 모든 reader가 끝날 때까지 old 데이터 유지
//! - 읽기 극도로 많은 경우 최적 (라우팅 테이블, 시스템 콜 테이블)
//!
//! 동작 원리:
//! 1. Writer가 새 데이터 복사본 생성
//! 2. 원자적으로 포인터 교체
//! 3. Grace period 대기 (모든 reader 종료)
//! 4. 이전 데이터 해제
//!
//! 현재 구현: 단순화된 버전 (선점 비활성화 기반)

use core::sync::atomic::{AtomicPtr, AtomicUsize, Ordering, fence};
use core::marker::PhantomData;
use alloc::boxed::Box;

/// RCU 글로벌 카운터 - grace period 추적
static RCU_GLOBAL_EPOCH: AtomicUsize = AtomicUsize::new(0);

/// RCU protected cell
/// 
/// T 타입의 데이터를 RCU로 보호
pub struct RcuCell<T> {
    ptr: AtomicPtr<T>,
    _marker: PhantomData<T>,
}

unsafe impl<T: Send + Sync> Send for RcuCell<T> {}
unsafe impl<T: Send + Sync> Sync for RcuCell<T> {}

impl<T> RcuCell<T> {
    /// 새 RcuCell 생성
    pub fn new(data: T) -> Self {
        let ptr = Box::into_raw(Box::new(data));
        Self {
            ptr: AtomicPtr::new(ptr),
            _marker: PhantomData,
        }
    }

    /// RCU 읽기 임계 구역 시작
    /// 
    /// 이 가드가 활성화된 동안에는 데이터가 해제되지 않음
    #[inline]
    pub fn read(&self) -> RcuReadGuard<'_, T> {
        // 선점 비활성화 (간단한 구현)
        // 실제 커널에서는 per-CPU 카운터 사용
        rcu_read_lock();
        
        let ptr = self.ptr.load(Ordering::Acquire);
        RcuReadGuard {
            cell: self,
            ptr,
        }
    }

    /// 데이터 업데이트 (Copy-on-Write)
    ///
    /// 1. 현재 데이터 복사
    /// 2. 수정 함수 적용
    /// 3. 원자적 교체
    /// 4. Grace period 후 이전 데이터 해제
    pub fn update<F>(&self, f: F)
    where
        T: Clone,
        F: FnOnce(&mut T),
    {
        // 1. 현재 데이터 복사
        let old_ptr = self.ptr.load(Ordering::Acquire);
        let mut new_data = unsafe { (*old_ptr).clone() };

        // 2. 수정 함수 적용
        f(&mut new_data);

        // 3. 새 데이터로 교체
        let new_ptr = Box::into_raw(Box::new(new_data));
        let old_ptr = self.ptr.swap(new_ptr, Ordering::AcqRel);

        // 4. Grace period 대기 후 해제
        synchronize_rcu();
        unsafe {
            drop(Box::from_raw(old_ptr));
        }
    }

    /// 데이터 교체 (새 값으로)
    pub fn replace(&self, new_data: T) {
        let new_ptr = Box::into_raw(Box::new(new_data));
        let old_ptr = self.ptr.swap(new_ptr, Ordering::AcqRel);

        synchronize_rcu();
        unsafe {
            drop(Box::from_raw(old_ptr));
        }
    }

    /// 데이터 교체 (해제 콜백 사용)
    ///
    /// Grace period를 비동기적으로 처리 (call_rcu 패턴)
    pub fn replace_async(&self, new_data: T, callback: fn(*mut T)) {
        let new_ptr = Box::into_raw(Box::new(new_data));
        let old_ptr = self.ptr.swap(new_ptr, Ordering::AcqRel);

        // TODO: 비동기 grace period 후 callback 호출
        // 현재는 동기적으로 처리
        synchronize_rcu();
        callback(old_ptr);
    }
}

impl<T> Drop for RcuCell<T> {
    fn drop(&mut self) {
        let ptr = self.ptr.load(Ordering::Relaxed);
        if !ptr.is_null() {
            unsafe {
                drop(Box::from_raw(ptr));
            }
        }
    }
}

/// RCU 읽기 가드
///
/// 이 가드가 살아있는 동안 데이터는 해제되지 않음
pub struct RcuReadGuard<'a, T> {
    cell: &'a RcuCell<T>,
    ptr: *const T,
}

impl<T> core::ops::Deref for RcuReadGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.ptr }
    }
}

impl<T> Drop for RcuReadGuard<'_, T> {
    fn drop(&mut self) {
        rcu_read_unlock();
    }
}

// RCU 구현 함수들

/// RCU 읽기 락 (선점 비활성화)
#[inline]
fn rcu_read_lock() {
    // 간단한 구현: 메모리 배리어만 사용
    // 실제 커널에서는 per-CPU 카운터 증가
    fence(Ordering::Acquire);
}

/// RCU 읽기 언락 (선점 활성화)
#[inline]
fn rcu_read_unlock() {
    fence(Ordering::Release);
}

/// Grace period 동기화
///
/// 모든 CPU에서 현재 진행 중인 RCU 읽기 구역이 끝날 때까지 대기
#[inline]
pub fn synchronize_rcu() {
    // 간단한 구현: epoch 증가 후 배리어
    // 실제 커널에서는 복잡한 quiescent state 추적
    RCU_GLOBAL_EPOCH.fetch_add(1, Ordering::SeqCst);
    fence(Ordering::SeqCst);
    
    // 스케줄러가 있다면 모든 CPU에서 컨텍스트 스위치 발생 대기
    // 현재는 단순 배리어로 대체
    for _ in 0..100 {
        core::hint::spin_loop();
    }
}

/// RCU 보호 리스트
///
/// 락 프리 읽기가 가능한 연결 리스트
pub struct RcuList<T> {
    head: AtomicPtr<RcuNode<T>>,
}

struct RcuNode<T> {
    data: T,
    next: AtomicPtr<RcuNode<T>>,
}

unsafe impl<T: Send + Sync> Send for RcuList<T> {}
unsafe impl<T: Send + Sync> Sync for RcuList<T> {}

impl<T> RcuList<T> {
    pub const fn new() -> Self {
        Self {
            head: AtomicPtr::new(core::ptr::null_mut()),
        }
    }

    /// 리스트 순회 (락 프리)
    pub fn iter(&self) -> RcuListIter<'_, T> {
        rcu_read_lock();
        RcuListIter {
            current: self.head.load(Ordering::Acquire),
            _marker: PhantomData,
        }
    }

    /// 앞에 삽입
    pub fn push_front(&self, data: T) {
        let new_node = Box::into_raw(Box::new(RcuNode {
            data,
            next: AtomicPtr::new(core::ptr::null_mut()),
        }));

        loop {
            let head = self.head.load(Ordering::Relaxed);
            unsafe {
                (*new_node).next.store(head, Ordering::Relaxed);
            }
            
            if self
                .head
                .compare_exchange_weak(head, new_node, Ordering::Release, Ordering::Relaxed)
                .is_ok()
            {
                return;
            }
        }
    }

    /// 앞에서 제거 (grace period 후 해제)
    pub fn pop_front(&self) -> Option<T> {
        loop {
            let head = self.head.load(Ordering::Acquire);
            if head.is_null() {
                return None;
            }

            let next = unsafe { (*head).next.load(Ordering::Relaxed) };
            
            if self
                .head
                .compare_exchange_weak(head, next, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
            {
                synchronize_rcu();
                let node = unsafe { Box::from_raw(head) };
                return Some(node.data);
            }
        }
    }
}

impl<T> Drop for RcuList<T> {
    fn drop(&mut self) {
        let mut current = self.head.load(Ordering::Relaxed);
        while !current.is_null() {
            let node = unsafe { Box::from_raw(current) };
            current = node.next.load(Ordering::Relaxed);
        }
    }
}

pub struct RcuListIter<'a, T> {
    current: *const RcuNode<T>,
    _marker: PhantomData<&'a T>,
}

impl<'a, T> Iterator for RcuListIter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current.is_null() {
            return None;
        }
        
        let node = unsafe { &*self.current };
        self.current = node.next.load(Ordering::Acquire);
        Some(&node.data)
    }
}

impl<T> Drop for RcuListIter<'_, T> {
    fn drop(&mut self) {
        rcu_read_unlock();
    }
}

/// SRCU (Sleepable RCU)
///
/// 읽기 구역에서 sleep 가능
/// per-domain 카운터 사용
pub struct SrcuDomain {
    /// 읽기 카운터 [even, odd]
    counters: [AtomicUsize; 2],
    /// 현재 epoch (0 또는 1)
    epoch: AtomicUsize,
}

impl SrcuDomain {
    pub const fn new() -> Self {
        Self {
            counters: [AtomicUsize::new(0), AtomicUsize::new(0)],
            epoch: AtomicUsize::new(0),
        }
    }

    /// SRCU 읽기 락 - epoch 인덱스 반환
    pub fn read_lock(&self) -> usize {
        let epoch = self.epoch.load(Ordering::Relaxed);
        self.counters[epoch & 1].fetch_add(1, Ordering::Acquire);
        epoch & 1
    }

    /// SRCU 읽기 언락
    pub fn read_unlock(&self, idx: usize) {
        self.counters[idx].fetch_sub(1, Ordering::Release);
    }

    /// SRCU grace period 동기화
    pub fn synchronize(&self) {
        // epoch 전환
        let old_epoch = self.epoch.fetch_add(1, Ordering::SeqCst);
        let old_idx = old_epoch & 1;
        
        // 이전 epoch의 모든 reader가 끝날 때까지 대기
        while self.counters[old_idx].load(Ordering::Acquire) > 0 {
            core::hint::spin_loop();
        }
    }
}
