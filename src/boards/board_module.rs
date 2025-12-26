//! 보드 모듈 정보 구조체
//!
//! 런타임 보드 감지 및 선택을 위한 BoardModuleInfo 구조체를 정의합니다.
//! Linux의 machine descriptor 패턴을 참고하였습니다.

/// 보드 모듈 정보 구조체
///
/// 각 보드 모듈은 이 구조체를 통해 자신의 정보를 커널에 등록합니다.
/// DTB의 compatible 속성과 매칭하여 적절한 보드 모듈을 선택합니다.
#[repr(C)]
pub struct BoardModuleInfo {
    /// DTB compatible 값들 (매칭에 사용)
    /// 예: &["linux,dummy-virt", "qemu,virt"]
    pub compatible: &'static [&'static str],

    /// 보드 이름 (표시용)
    pub name: &'static str,

    /// 타이머 주파수 (Hz)
    /// 0이면 DTB 또는 시스템 레지스터에서 읽음
    pub timer_freq: u64,

    /// UART quirks 플래그
    pub uart_quirks: u32,

    /// 보드 초기화 함수 (MMU 활성화 후 호출)
    pub init_fn: fn() -> Result<(), i32>,

    /// 초기 초기화 함수 (MMU 활성화 전, 선택적)
    pub early_init_fn: Option<fn()>,

    /// 플랫폼 설정 오버라이드 함수 (선택적)
    pub platform_config_fn: Option<fn() -> BoardPlatformOverrides>,

    /// CPU/Hart 개수 (0이면 DTB에서 읽음)
    pub cpu_count: u32,

    /// SMP 지원 여부
    pub smp_capable: bool,
}

/// 플랫폼 설정 오버라이드
///
/// 보드 모듈이 DTB나 기본값 대신 사용할 설정을 지정합니다.
/// None인 필드는 기존 폴백 체인을 사용합니다.
#[repr(C)]
#[derive(Default)]
pub struct BoardPlatformOverrides {
    // UART 설정
    pub uart_base: Option<usize>,
    pub uart_irq: Option<u32>,
    pub uart_clock_freq: Option<u32>,

    // 인터럽트 컨트롤러 (aarch64)
    pub gicd_base: Option<usize>,
    pub gicc_base: Option<usize>,

    // 인터럽트 컨트롤러 (riscv64)
    pub plic_base: Option<usize>,
    pub clint_base: Option<usize>,

    // 메모리 설정
    pub ram_base: Option<usize>,
    pub ram_size: Option<usize>,
}

/// UART quirks 플래그
pub mod uart_quirks {
    /// quirks 없음
    pub const NONE: u32 = 0;
    /// FIFO가 제대로 동작하지 않음
    pub const FIFO_BROKEN: u32 = 1 << 0;
    /// 문자 출력 사이에 딜레이 필요
    pub const NEEDS_DELAY: u32 = 1 << 1;
    /// 32비트 MMIO 접근 필요
    pub const MMIO_32BIT: u32 = 1 << 2;
    /// 8비트 MMIO 접근 필요
    pub const MMIO_8BIT: u32 = 1 << 3;
}

impl BoardModuleInfo {
    /// 이 보드가 주어진 compatible 문자열과 매칭되는지 확인
    pub fn matches_compatible(&self, compat: &str) -> bool {
        self.compatible.iter().any(|c| *c == compat)
    }

    /// 이 보드가 주어진 compatible 목록 중 하나와 매칭되는지 확인
    pub fn matches_any_compatible(&self, compats: &[&str]) -> bool {
        compats.iter().any(|c| self.matches_compatible(c))
    }
}

impl BoardPlatformOverrides {
    /// 빈 오버라이드 (모든 필드가 None)
    pub const fn empty() -> Self {
        Self {
            uart_base: None,
            uart_irq: None,
            uart_clock_freq: None,
            gicd_base: None,
            gicc_base: None,
            plic_base: None,
            clint_base: None,
            ram_base: None,
            ram_size: None,
        }
    }
}
