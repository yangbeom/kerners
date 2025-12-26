//! 프로세스/스레드 관리 모듈
//!
//! 커널 스레드 추상화와 컨텍스트 스위칭 구현
//! SMP 환경에서 각 CPU는 per-CPU 데이터를 통해 자신의 현재 스레드를 추적합니다.

pub mod context;
pub mod percpu;
pub mod scheduler;
pub mod user;

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};
use crate::sync::Mutex;

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
            cpu_affinity: Some(cpu_id),
        }
    }
}

/// 전역 스레드 리스트 (모든 CPU가 공유)
pub(crate) static THREADS: Mutex<Vec<Box<Thread>>> = Mutex::new(Vec::new());

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

/// 현재 스레드의 컨텍스트 포인터 반환
pub fn current_context_ptr() -> Option<*mut Context> {
    let idx = percpu::current().current_thread_idx.load(Ordering::Acquire);
    if idx == u32::MAX {
        return None;
    }
    let mut threads = THREADS.lock();
    threads.get_mut(idx as usize).map(|t| &mut t.context as *mut Context)
}

/// 스레드 상태 출력
pub fn dump_threads() {
    let threads = THREADS.lock();
    let online = percpu::online_count();

    kprintln!("\n[proc] Thread list ({} threads, {} CPUs online):", threads.len(), online);
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
            thread.tid, thread.name, thread.state, cpu_mark
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
