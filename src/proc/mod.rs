//! 프로세스/스레드 관리 모듈
//!
//! 커널 스레드 추상화와 컨텍스트 스위칭 구현
//! SMP 환경에서 각 CPU는 per-CPU 데이터를 통해 자신의 현재 스레드를 추적합니다.

pub mod context;
pub mod percpu;
pub mod scheduler;
pub mod user;

use crate::sync::IrqSpinlock;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::kprintln;
use context::Context;

/// 스레드 ID 타입
pub type Tid = u64;

/// 다음 스레드 ID 생성을 위한 카운터
static NEXT_TID: AtomicU64 = AtomicU64::new(1);

/// 스레드 상태
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadState {
    /// 실행 가능 상태
    Ready,
    /// 현재 실행 중
    Running,
    /// 대기 중 (Sleep 등)
    Blocked,
    /// 종료됨
    Terminated,
}

/// sleep 해제 사유
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SleepWakeReason {
    Timer,
    Signal,
}

/// sleep 대기 항목
#[derive(Debug, Clone, Copy)]
pub struct SleepEntry {
    pub tid: Tid,
    pub deadline_ns: u64,
    pub wake_reason: SleepWakeReason,
}

/// 스레드 제어 블록 (TCB)
pub struct Thread {
    /// 스레드 ID
    pub tid: Tid,
    /// 스레드 이름
    pub name: String,
    /// 스레드 상태
    pub state: ThreadState,
    /// CPU 컨텍스트 (레지스터 상태)
    pub context: Context,
    /// 커널 스택 (Box로 관리)
    pub kernel_stack: Vec<u8>,
    /// 유저 스택 (execve 등으로 유저 모드 진입 시 보관)
    pub user_stack: Option<Vec<u8>>,
    /// 사용자 주소공간 루트 페이지 테이블 (aarch64)
    pub user_root_table: usize,
    /// CPU 친화도 (None = 모든 CPU에서 실행 가능, Some(id) = 특정 CPU에 고정)
    pub cpu_affinity: Option<u32>,
}

impl Thread {
    /// 스택 크기 (16KB)
    pub const STACK_SIZE: usize = 16 * 1024;

    /// 새 스레드 생성
    pub fn new(name: &str, entry: fn() -> !) -> Self {
        let tid = NEXT_TID.fetch_add(1, Ordering::SeqCst);

        // 커널 스택 할당 (16KB, 16바이트 정렬)
        let mut kernel_stack = Vec::with_capacity(Self::STACK_SIZE);
        kernel_stack.resize(Self::STACK_SIZE, 0);

        // 스택 포인터 계산 (스택은 아래로 자람)
        let stack_top = kernel_stack.as_ptr() as usize + Self::STACK_SIZE;
        // 16바이트 정렬
        let stack_top = stack_top & !0xF;

        // 컨텍스트 초기화
        let context = Context::new(entry as usize, stack_top);

        Thread {
            tid,
            name: String::from(name),
            state: ThreadState::Ready,
            context,
            kernel_stack,
            user_stack: None,
            #[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
            user_root_table: crate::arch::mmu::current_root_table(),
            #[cfg(not(any(target_arch = "aarch64", target_arch = "riscv64")))]
            user_root_table: 0,
            cpu_affinity: None, // 모든 CPU에서 실행 가능
        }
    }

    /// idle 스레드 생성 (부트스트랩용, CPU 0)
    pub fn idle() -> Self {
        let tid = 0;

        let mut kernel_stack = Vec::with_capacity(Self::STACK_SIZE);
        kernel_stack.resize(Self::STACK_SIZE, 0);

        Thread {
            tid,
            name: String::from("idle/0"),
            state: ThreadState::Running,
            context: Context::empty(),
            kernel_stack,
            user_stack: None,
            #[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
            user_root_table: crate::arch::mmu::kernel_root_table(),
            #[cfg(not(any(target_arch = "aarch64", target_arch = "riscv64")))]
            user_root_table: 0,
            cpu_affinity: Some(0),
        }
    }

    /// Secondary CPU용 idle 스레드 생성
    pub fn idle_for_cpu(cpu_id: u32) -> Self {
        let tid = NEXT_TID.fetch_add(1, Ordering::SeqCst);

        Thread {
            tid,
            name: alloc::format!("idle/{}", cpu_id),
            state: ThreadState::Running,
            context: Context::empty(),
            kernel_stack: Vec::new(), // 스택은 percpu::stacks에서 관리
            user_stack: None,
            #[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
            user_root_table: crate::arch::mmu::kernel_root_table(),
            #[cfg(not(any(target_arch = "aarch64", target_arch = "riscv64")))]
            user_root_table: 0,
            cpu_affinity: Some(cpu_id),
        }
    }
}

/// 전역 스레드 리스트 (모든 CPU가 공유)
pub(crate) static THREADS: IrqSpinlock<Vec<Box<Thread>>> = IrqSpinlock::new(Vec::new());
static SLEEP_QUEUE: IrqSpinlock<Vec<SleepEntry>> = IrqSpinlock::new(Vec::new());
static SLEEP_WAKE_REASONS: IrqSpinlock<Vec<(Tid, SleepWakeReason)>> = IrqSpinlock::new(Vec::new());

/// 프로세스 서브시스템 초기화
pub fn init() {
    kprintln!("[proc] Initializing process subsystem...");

    // Per-CPU 서브시스템 초기화 (단일 CPU로 시작, SMP 시 갱신)
    percpu::init(1);

    // idle 스레드 생성 (tid=0, CPU 0 전용)
    let idle = Box::new(Thread::idle());

    {
        let mut threads = THREADS.lock();
        threads.push(idle);
    }

    // Per-CPU 데이터에 현재/idle 스레드 인덱스 설정
    let pc = percpu::current();
    pc.current_thread_idx.store(0, Ordering::Release);
    pc.idle_thread_idx.store(0, Ordering::Release);

    kprintln!("[proc] Idle thread created (tid=0)");
}

/// Secondary CPU에서 호출: idle 스레드를 생성하고 per-CPU 데이터 설정
pub fn init_on_secondary_cpu(cpu_id: u32) {
    let idle = Box::new(Thread::idle_for_cpu(cpu_id));

    let mut threads = THREADS.lock();
    let idx = threads.len();
    threads.push(idle);

    let pc = percpu::get(cpu_id);
    pc.current_thread_idx.store(idx as u32, Ordering::Release);
    pc.idle_thread_idx.store(idx as u32, Ordering::Release);
}

/// 새 커널 스레드 생성
pub fn spawn(name: &str, entry: fn() -> !) -> Tid {
    let thread = Box::new(Thread::new(name, entry));
    let tid = thread.tid;

    kprintln!("[proc] Spawning thread '{}' (tid={})", name, tid);

    {
        let mut threads = THREADS.lock();
        threads.push(thread);
    }

    // SMP: idle 중인 다른 CPU를 깨워서 새 스레드를 실행하도록 IPI 전송
    kick_idle_cpu();

    tid
}

/// idle 중인 다른 CPU에 reschedule IPI 전송
fn kick_idle_cpu() {
    let my_cpu = percpu::get_cpu_id();
    let online = percpu::online_count();

    // 온라인 CPU 중 idle 상태인 CPU를 찾아 IPI 전송
    for cpu in 0..online {
        if cpu == my_cpu {
            continue;
        }
        let pc = percpu::get(cpu);
        let current = pc.current_thread_idx.load(Ordering::Relaxed);
        let idle = pc.idle_thread_idx.load(Ordering::Relaxed);
        if current == idle {
            // 이 CPU는 idle → reschedule IPI 전송
            #[cfg(target_arch = "aarch64")]
            crate::arch::gic::send_reschedule_ipi(cpu);

            #[cfg(target_arch = "riscv64")]
            crate::arch::plic::send_reschedule_ipi(cpu);

            break; // 하나만 깨우면 됨
        }
    }
}

/// 현재 스레드 ID 반환
pub fn current_tid() -> Option<Tid> {
    let idx = percpu::current().current_thread_idx.load(Ordering::Acquire);
    if idx == u32::MAX {
        return None;
    }
    let threads = THREADS.lock();
    threads.get(idx as usize).map(|t| t.tid)
}

/// tid에 해당하는 스레드가 존재하는지 확인한다.
pub fn thread_exists(tid: Tid) -> bool {
    let threads = THREADS.lock();
    threads.iter().any(|thread| thread.tid == tid)
}

/// tid에 해당하는 스레드를 Blocked 상태로 전환한다.
pub fn block_thread_for_signal_stop(tid: Tid) -> bool {
    let mut threads = THREADS.lock();
    let Some(thread) = threads.iter_mut().find(|thread| thread.tid == tid) else {
        return false;
    };
    if thread.state != ThreadState::Terminated {
        thread.state = ThreadState::Blocked;
    }
    true
}

/// tid에 해당하는 스레드를 Terminated 상태로 전환한다.
pub fn terminate_thread_for_signal(tid: Tid) -> bool {
    let mut threads = THREADS.lock();
    let Some(thread) = threads.iter_mut().find(|thread| thread.tid == tid) else {
        return false;
    };
    if thread.state != ThreadState::Terminated {
        thread.state = ThreadState::Terminated;
    }
    true
}

/// 현재 스레드의 컨텍스트 포인터 반환
pub fn current_context_ptr() -> Option<*mut Context> {
    let idx = percpu::current().current_thread_idx.load(Ordering::Acquire);
    if idx == u32::MAX {
        return None;
    }
    let mut threads = THREADS.lock();
    threads
        .get_mut(idx as usize)
        .map(|t| &mut t.context as *mut Context)
}

/// 현재 스레드의 유저 스택을 설정
///
/// execve 이후 유저 스택 메모리를 스레드 수명과 함께 유지하기 위해 사용한다.
pub fn set_current_user_stack(user_stack: Vec<u8>) -> bool {
    let idx = percpu::current().current_thread_idx.load(Ordering::Acquire);
    if idx == u32::MAX {
        return false;
    }

    let mut threads = THREADS.lock();
    if let Some(thread) = threads.get_mut(idx as usize) {
        thread.user_stack = Some(user_stack);
        true
    } else {
        false
    }
}

/// 현재 스레드의 사용자 주소공간 루트 페이지 테이블 조회
pub fn current_user_root_table() -> Option<usize> {
    let idx = percpu::current().current_thread_idx.load(Ordering::Acquire);
    if idx == u32::MAX {
        return None;
    }
    let threads = THREADS.lock();
    threads.get(idx as usize).map(|t| t.user_root_table)
}

/// tid 기준 사용자 주소공간 루트 페이지 테이블 설정
pub fn set_thread_user_root_table(tid: Tid, root: usize) -> bool {
    let mut threads = THREADS.lock();
    if let Some(thread) = threads.iter_mut().find(|t| t.tid == tid) {
        thread.user_root_table = root;
        true
    } else {
        false
    }
}

/// 스레드 상태 출력
pub fn dump_threads() {
    let threads = THREADS.lock();
    let online = percpu::online_count();

    kprintln!(
        "\n[proc] Thread list ({} threads, {} CPUs online):",
        threads.len(),
        online
    );
    for (i, thread) in threads.iter().enumerate() {
        // 이 스레드가 어느 CPU에서 실행 중인지 확인
        let mut running_on = None;
        for cpu in 0..online {
            let pc = percpu::get(cpu);
            if pc.current_thread_idx.load(Ordering::Relaxed) == i as u32 {
                running_on = Some(cpu);
                break;
            }
        }
        let cpu_mark = match running_on {
            Some(cpu) => alloc::format!(" [CPU {}]", cpu),
            None => String::new(),
        };
        kprintln!(
            "  tid={}, name='{}', state={:?}{}",
            thread.tid,
            thread.name,
            thread.state,
            cpu_mark
        );
    }
}

/// 스레드 yield (다음 스레드로 전환)
pub fn yield_now() {
    scheduler::schedule();
}

/// 스레드 종료
pub fn exit() -> ! {
    {
        let idx = percpu::current().current_thread_idx.load(Ordering::Acquire);
        let mut threads = THREADS.lock();

        if idx != u32::MAX {
            if let Some(thread) = threads.get_mut(idx as usize) {
                thread.state = ThreadState::Terminated;
                kprintln!("[proc] Thread {} terminated", thread.tid);
            }
        }
    }

    // 다른 스레드로 전환
    scheduler::schedule();

    // 여기에 도달하면 안 됨
    loop {
        #[cfg(target_arch = "aarch64")]
        unsafe {
            core::arch::asm!("wfi");
        }
        #[cfg(target_arch = "riscv64")]
        unsafe {
            core::arch::asm!("wfi");
        }
    }
}

fn record_sleep_wake_reason(tid: Tid, reason: SleepWakeReason) {
    let mut reasons = SLEEP_WAKE_REASONS.lock();
    if let Some(item) = reasons.iter_mut().find(|item| item.0 == tid) {
        item.1 = reason;
    } else {
        reasons.push((tid, reason));
    }
}

fn take_sleep_wake_reason(tid: Tid) -> Option<SleepWakeReason> {
    let mut reasons = SLEEP_WAKE_REASONS.lock();
    let pos = reasons.iter().position(|item| item.0 == tid)?;
    Some(reasons.swap_remove(pos).1)
}

/// 현재 스레드를 deadline까지 sleep 상태로 전환한다.
pub fn sleep_current_until(deadline_ns: u64) -> SleepWakeReason {
    let tid = match current_tid() {
        Some(tid) => tid,
        None => return SleepWakeReason::Timer,
    };

    {
        let mut threads = THREADS.lock();
        if let Some(thread) = threads.iter_mut().find(|thread| thread.tid == tid) {
            if thread.state != ThreadState::Terminated {
                thread.state = ThreadState::Blocked;
            }
        } else {
            return SleepWakeReason::Timer;
        }
    }

    {
        let mut queue = SLEEP_QUEUE.lock();
        if let Some(entry) = queue.iter_mut().find(|entry| entry.tid == tid) {
            entry.deadline_ns = deadline_ns;
            entry.wake_reason = SleepWakeReason::Timer;
        } else {
            queue.push(SleepEntry {
                tid,
                deadline_ns,
                wake_reason: SleepWakeReason::Timer,
            });
        }
    }

    scheduler::schedule();
    take_sleep_wake_reason(tid).unwrap_or(SleepWakeReason::Timer)
}

/// 타이머 만료된 sleep 스레드를 깨운다.
pub fn wake_sleepers_by_timer(now_ns: u64) -> usize {
    let mut ready_tids: Vec<Tid> = Vec::new();
    {
        let mut queue = SLEEP_QUEUE.lock();
        let mut i = 0usize;
        while i < queue.len() {
            if queue[i].deadline_ns <= now_ns {
                let entry = queue.swap_remove(i);
                ready_tids.push(entry.tid);
            } else {
                i += 1;
            }
        }
    }

    if ready_tids.is_empty() {
        return 0;
    }

    {
        let mut threads = THREADS.lock();
        for tid in ready_tids.iter().copied() {
            if let Some(thread) = threads.iter_mut().find(|thread| thread.tid == tid) {
                if thread.state == ThreadState::Blocked {
                    thread.state = ThreadState::Ready;
                }
            }
            record_sleep_wake_reason(tid, SleepWakeReason::Timer);
        }
    }

    ready_tids.len()
}

/// 지정 스레드를 시그널 사유로 깨운다.
pub fn wake_thread_for_signal(tid: Tid) -> bool {
    let mut removed_from_sleep_queue = false;
    {
        let mut queue = SLEEP_QUEUE.lock();
        if let Some(pos) = queue.iter().position(|entry| entry.tid == tid) {
            queue.swap_remove(pos);
            removed_from_sleep_queue = true;
        }
    }

    let mut threads = THREADS.lock();
    if let Some(thread) = threads.iter_mut().find(|thread| thread.tid == tid) {
        let mut woke = removed_from_sleep_queue;
        if thread.state == ThreadState::Blocked {
            thread.state = ThreadState::Ready;
            woke = true;
        }
        if woke {
            record_sleep_wake_reason(tid, SleepWakeReason::Signal);
        }
        woke
    } else {
        false
    }
}
