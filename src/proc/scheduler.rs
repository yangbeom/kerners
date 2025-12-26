//! SMP-aware 스케줄러 구현
//!
//! 라운드-로빈 스케줄러. 각 CPU는 per-CPU 데이터를 통해 자신의 현재 스레드를 추적하며,
//! 전역 THREADS 리스트에서 Ready 상태의 스레드를 선택합니다.
//! CPU 친화도(cpu_affinity)가 설정된 스레드는 지정된 CPU에서만 실행됩니다.

use super::{ThreadState, THREADS};
use super::context::{Context, context_switch};
use super::percpu;
use core::sync::atomic::Ordering;

/// 스케줄러: 현재 CPU에서 다음 실행할 스레드를 선택하고 컨텍스트 스위칭 수행
pub fn schedule() {
    let cpu_id = percpu::get_cpu_id();
    let pc = percpu::current();

    let current_idx = pc.current_thread_idx.load(Ordering::Acquire) as usize;
    if current_idx == u32::MAX as usize {
        return; // 아직 초기화되지 않음
    }

    let (old_ctx, new_ctx) = {
        let mut threads = THREADS.lock();

        if current_idx >= threads.len() {
            return;
        }

        // 현재 스레드가 Running이면 Ready로 변경
        if let Some(thread) = threads.get_mut(current_idx) {
            if thread.state == ThreadState::Running {
                thread.state = ThreadState::Ready;
            }
        }

        // 다음 실행할 스레드 찾기 (라운드-로빈, CPU 친화도 존중)
        let num_threads = threads.len();
        let mut next_idx = None;

        for offset in 1..=num_threads {
            let idx = (current_idx + offset) % num_threads;
            if let Some(thread) = threads.get(idx) {
                if thread.state == ThreadState::Ready {
                    // CPU 친화도 확인: 다른 CPU에 고정된 스레드는 건너뜀
                    if let Some(affinity) = thread.cpu_affinity {
                        if affinity != cpu_id {
                            continue;
                        }
                    }
                    next_idx = Some(idx);
                    break;
                }
            }
        }

        // 실행할 스레드가 없으면 현재 스레드 계속 또는 idle로 전환
        let next_idx = match next_idx {
            Some(idx) => idx,
            None => {
                if let Some(thread) = threads.get_mut(current_idx) {
                    if thread.state == ThreadState::Terminated {
                        // 종료된 스레드 → 이 CPU의 idle 스레드로 전환
                        let idle_idx = pc.idle_thread_idx.load(Ordering::Relaxed) as usize;
                        if idle_idx < threads.len() {
                            idle_idx
                        } else {
                            return;
                        }
                    } else {
                        // Ready나 다른 상태 → 그대로 계속
                        thread.state = ThreadState::Running;
                        return;
                    }
                } else {
                    return;
                }
            }
        };

        // 같은 스레드면 스위칭 불필요
        if next_idx == current_idx {
            if let Some(thread) = threads.get_mut(current_idx) {
                thread.state = ThreadState::Running;
            }
            return;
        }

        // 새 스레드를 Running으로 변경
        if let Some(thread) = threads.get_mut(next_idx) {
            thread.state = ThreadState::Running;
        }

        // 컨텍스트 포인터 얻기
        let old_ctx = threads.get_mut(current_idx)
            .map(|t| &mut t.context as *mut Context);
        let new_ctx = threads.get(next_idx)
            .map(|t| &t.context as *const Context);

        // Per-CPU 현재 스레드 인덱스 업데이트
        pc.current_thread_idx.store(next_idx as u32, Ordering::Release);

        match (old_ctx, new_ctx) {
            (Some(old), Some(new)) => (old, new),
            _ => return,
        }
    };

    // 락을 해제한 후 컨텍스트 스위칭
    unsafe {
        context_switch(old_ctx, new_ctx);
    }
}

/// 실행 가능한 스레드 수 반환
pub fn ready_count() -> usize {
    let threads = THREADS.lock();
    threads.iter().filter(|t| t.state == ThreadState::Ready).count()
}

/// 활성 스레드 수 반환 (종료되지 않은)
pub fn active_count() -> usize {
    let threads = THREADS.lock();
    threads.iter().filter(|t| t.state != ThreadState::Terminated).count()
}
