//! Per-CPU 데이터 구조
//!
//! 각 CPU/hart별로 독립적인 데이터를 관리합니다.
//! SMP 환경에서 CPU별 스케줄링, 인터럽트 처리에 사용됩니다.

use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

/// 최대 지원 CPU 수
pub const MAX_CPUS: usize = 8;

/// Per-CPU 데이터
#[repr(C)]
pub struct PerCpuData {
    /// CPU ID (aarch64: MPIDR 기반, riscv64: hartid)
    pub cpu_id: AtomicU32,
    /// 현재 실행 중인 스레드 인덱스 (THREADS 배열 내 인덱스)
    pub current_thread_idx: AtomicU32, // u32::MAX = 없음
    /// 이 CPU의 idle 스레드 인덱스 (THREADS 배열 내 인덱스)
    pub idle_thread_idx: AtomicU32,
    /// 이 CPU가 온라인(부팅 완료)인지 여부
    pub online: AtomicBool,
    /// 타이머 틱 카운터
    pub tick_count: AtomicU64,
}

impl PerCpuData {
    pub const fn new() -> Self {
        Self {
            cpu_id: AtomicU32::new(0),
            current_thread_idx: AtomicU32::new(u32::MAX),
            idle_thread_idx: AtomicU32::new(u32::MAX),
            online: AtomicBool::new(false),
            tick_count: AtomicU64::new(0),
        }
    }

    pub fn init(&self, cpu_id: u32) {
        self.cpu_id.store(cpu_id, Ordering::Relaxed);
        self.current_thread_idx.store(u32::MAX, Ordering::Relaxed);
        self.idle_thread_idx.store(u32::MAX, Ordering::Relaxed);
        self.online.store(false, Ordering::Relaxed);
        self.tick_count.store(0, Ordering::Relaxed);
    }

    pub fn set_online(&self) {
        self.online.store(true, Ordering::Release);
    }

    pub fn is_online(&self) -> bool {
        self.online.load(Ordering::Acquire)
    }
}

/// 전역 Per-CPU 데이터 배열
static PER_CPU: [PerCpuData; MAX_CPUS] = [
    PerCpuData::new(), PerCpuData::new(), PerCpuData::new(), PerCpuData::new(),
    PerCpuData::new(), PerCpuData::new(), PerCpuData::new(), PerCpuData::new(),
];

/// 온라인 CPU 수
static NUM_CPUS_ONLINE: AtomicU32 = AtomicU32::new(0);

/// 전체 CPU 수 (DTB에서 감지된)
static TOTAL_CPU_COUNT: AtomicU32 = AtomicU32::new(1);

/// Per-CPU 서브시스템 초기화 (primary CPU에서 호출)
pub fn init(cpu_count: u32) {
    TOTAL_CPU_COUNT.store(cpu_count, Ordering::Relaxed);

    // Primary CPU (CPU 0) 초기화
    let cpu_id = get_cpu_id();
    PER_CPU[cpu_id as usize].init(cpu_id);
    PER_CPU[cpu_id as usize].set_online();
    NUM_CPUS_ONLINE.store(1, Ordering::Release);

    crate::kprintln!("[percpu] Initialized for {} CPUs (primary CPU {})", cpu_count, cpu_id);
}

/// Secondary CPU 초기화 (secondary CPU에서 호출)
pub fn init_secondary(cpu_id: u32) {
    if (cpu_id as usize) >= MAX_CPUS {
        return;
    }
    PER_CPU[cpu_id as usize].init(cpu_id);
    PER_CPU[cpu_id as usize].set_online();
    NUM_CPUS_ONLINE.fetch_add(1, Ordering::AcqRel);
}

/// 현재 CPU의 Per-CPU 데이터 참조
pub fn current() -> &'static PerCpuData {
    let cpu_id = get_cpu_id() as usize;
    if cpu_id < MAX_CPUS {
        &PER_CPU[cpu_id]
    } else {
        &PER_CPU[0] // fallback
    }
}

/// 특정 CPU의 Per-CPU 데이터 참조
pub fn get(cpu_id: u32) -> &'static PerCpuData {
    if (cpu_id as usize) < MAX_CPUS {
        &PER_CPU[cpu_id as usize]
    } else {
        &PER_CPU[0]
    }
}

/// 온라인 CPU 수 반환
pub fn online_count() -> u32 {
    NUM_CPUS_ONLINE.load(Ordering::Acquire)
}

/// 전체 CPU 수 반환
pub fn total_count() -> u32 {
    TOTAL_CPU_COUNT.load(Ordering::Relaxed)
}

/// 전체 CPU 수 업데이트 (SMP 부팅 시 호출)
pub fn set_total_cpu_count(count: u32) {
    TOTAL_CPU_COUNT.store(count, Ordering::Relaxed);
}

/// 현재 CPU ID 가져오기
#[cfg(target_arch = "aarch64")]
pub fn get_cpu_id() -> u32 {
    let mpidr: u64;
    unsafe {
        core::arch::asm!("mrs {}, MPIDR_EL1", out(reg) mpidr);
    }
    // Aff0 필드 (비트 [7:0])가 CPU 번호
    (mpidr & 0xFF) as u32
}

/// 현재 CPU ID 가져오기
#[cfg(target_arch = "riscv64")]
pub fn get_cpu_id() -> u32 {
    let hartid: u64;
    unsafe {
        core::arch::asm!("csrr {}, mhartid", out(reg) hartid);
    }
    hartid as u32
}

/// Per-CPU 스택 관리
pub mod stacks {
    use super::MAX_CPUS;
    use core::sync::atomic::{AtomicUsize, Ordering};

    /// Secondary CPU 스택 크기 (32KB)
    pub const SECONDARY_STACK_SIZE: usize = 32 * 1024;

    /// Secondary CPU 스택 포인터 배열 (스택 top 주소)
    static STACK_TOPS: [AtomicUsize; MAX_CPUS] = [
        AtomicUsize::new(0), AtomicUsize::new(0), AtomicUsize::new(0), AtomicUsize::new(0),
        AtomicUsize::new(0), AtomicUsize::new(0), AtomicUsize::new(0), AtomicUsize::new(0),
    ];

    /// Secondary CPU 스택 할당 (primary CPU에서 호출)
    ///
    /// 각 secondary CPU의 커널 스택을 힙에서 할당합니다.
    pub fn allocate_secondary_stacks(cpu_count: u32) {
        use alloc::vec::Vec;

        for cpu_id in 1..cpu_count.min(MAX_CPUS as u32) {
            let mut stack = Vec::<u8>::with_capacity(SECONDARY_STACK_SIZE);
            stack.resize(SECONDARY_STACK_SIZE, 0);

            // 스택 top 계산 (16바이트 정렬)
            let stack_top = (stack.as_ptr() as usize + SECONDARY_STACK_SIZE) & !0xF;

            STACK_TOPS[cpu_id as usize].store(stack_top, Ordering::Release);

            // Vec의 메모리를 leak하여 커널이 계속 사용할 수 있도록 함
            core::mem::forget(stack);

            crate::kprintln!("[percpu] CPU {} stack allocated: top={:#x}", cpu_id, stack_top);
        }
    }

    /// 특정 CPU의 스택 top 주소 반환
    pub fn get_stack_top(cpu_id: u32) -> usize {
        if (cpu_id as usize) < MAX_CPUS {
            STACK_TOPS[cpu_id as usize].load(Ordering::Acquire)
        } else {
            0
        }
    }
}
