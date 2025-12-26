# 보드 모듈 시스템

kerners의 보드 모듈 시스템은 Linux의 machine descriptor 패턴을 참고하여 런타임에 보드를 선택하고 초기화하는 기능을 제공합니다.

## 개요

- **DTB compatible 기반 보드 선택**: 부팅 시 DTB의 루트 노드 `compatible` 속성을 읽어 적합한 보드 모듈을 자동 선택
- **빌트인 보드 지원**: 커널에 컴파일된 보드 모듈을 레지스트리에 등록하여 사용
- **SMP 지원**: 멀티코어 보드 구성과 보조 CPU/Hart 시작 인터페이스 제공

## 파일 구조

```
src/boards/
├── board_module.rs          # BoardModuleInfo 구조체, BoardPlatformOverrides
├── registry.rs              # 보드 레지스트리 (등록/조회/활성화)
├── qemu_virt_aarch64.rs     # QEMU virt (AArch64) 싱글코어
├── qemu_virt_aarch64_smp.rs # QEMU virt (AArch64) 멀티코어 + PSCI
├── qemu_virt_riscv64.rs     # QEMU virt (RISC-V64) 싱글코어
├── qemu_virt_riscv64_smp.rs # QEMU virt (RISC-V64) 멀티코어 + SBI HSM
└── mod.rs                   # 모듈 연결, 초기화 함수
```

## 핵심 구조체

### BoardModuleInfo

```rust
#[repr(C)]
pub struct BoardModuleInfo {
    /// DTB compatible 값들 (매칭에 사용)
    pub compatible: &'static [&'static str],

    /// 보드 이름 (표시용)
    pub name: &'static str,

    /// 타이머 주파수 (Hz), 0이면 DTB/시스템에서 읽음
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
```

### BoardPlatformOverrides

보드 모듈이 DTB나 기본값 대신 사용할 설정을 지정합니다.

```rust
pub struct BoardPlatformOverrides {
    pub uart_base: Option<usize>,
    pub uart_irq: Option<u32>,
    pub uart_clock_freq: Option<u32>,
    pub gicd_base: Option<usize>,    // aarch64
    pub gicc_base: Option<usize>,    // aarch64
    pub plic_base: Option<usize>,    // riscv64
    pub clint_base: Option<usize>,   // riscv64
    pub ram_base: Option<usize>,
    pub ram_size: Option<usize>,
}
```

## 사용 방법

### 새 보드 추가하기

1. `src/boards/` 디렉토리에 새 파일 생성 (예: `my_board.rs`)

2. BoardConfig trait 구현:
```rust
use super::board_module::{uart_quirks, BoardModuleInfo};
use super::BoardConfig;

pub struct MyBoard;

impl BoardConfig for MyBoard {
    const UART_BASE: usize = 0x1000_0000;
    const UART_IRQ: u32 = 10;
    // ... 기타 설정
}
```

3. BoardModuleInfo 정의:
```rust
fn board_init() -> Result<(), i32> {
    // 보드별 초기화 로직
    Ok(())
}

pub static BOARD_INFO: BoardModuleInfo = BoardModuleInfo {
    compatible: &["vendor,my-board", "vendor,board-v2"],
    name: "My Custom Board",
    timer_freq: 10_000_000,
    uart_quirks: uart_quirks::NONE,
    init_fn: board_init,
    early_init_fn: None,
    platform_config_fn: None,
    cpu_count: 0,
    smp_capable: false,
};
```

4. `src/boards/mod.rs`에서 모듈 연결:
```rust
mod my_board;

fn register_builtin_boards() {
    // ...
    let _ = registry::register_builtin_board(&my_board::BOARD_INFO);
}
```

### 쉘 명령어

```bash
# 현재 활성 보드 정보 확인
kerners> boardinfo
Active board: QEMU virt (AArch64 SMP)
  Compatible: ["linux,dummy-virt", "qemu,virt"]
  Timer freq: 0 Hz
  UART quirks: 0x0
  SMP capable: yes
  CPU count: 4 (from DTB)

# 등록된 보드 목록
kerners> lsboards
Registered board modules:
  * QEMU virt (AArch64 SMP)
    QEMU virt (AArch64)

Total: 2 board(s) (* = active)
```

## 멀티코어 지원

### ARM64 (PSCI)

QEMU virt 머신은 PSCI(Power State Coordination Interface)를 통해 보조 CPU를 제어합니다.

```rust
use crate::boards::qemu_virt_aarch64_smp::{cpu_on, cpu_off, affinity_info};

// 보조 CPU 시작
let result = cpu_on(cpu_id, entry_point, context_id);

// CPU 상태 확인
let status = affinity_info(cpu_id, 0);
```

### RISC-V64 (SBI HSM)

QEMU virt 머신은 SBI HSM(Hart State Management) 확장을 통해 보조 Hart를 제어합니다.

```rust
use crate::boards::qemu_virt_riscv64_smp::{hart_start, hart_stop, hart_get_status};

// 보조 Hart 시작
let result = hart_start(hartid, start_addr, opaque);

// Hart 상태 확인
let status = hart_get_status(hartid);
```

## 실행 방법

```bash
# 싱글 코어
./run.sh aarch64 512      # ARM64, 512MB RAM, 1 CPU
./run.sh riscv64 512      # RISC-V64, 512MB RAM, 1 Hart

# 멀티 코어
./run.sh aarch64 512 4    # ARM64, 512MB RAM, 4 CPUs
./run.sh riscv64 512 4    # RISC-V64, 512MB RAM, 4 Harts
```

## 부팅 시퀀스

1. **DTB 파싱**: 메모리 정보 및 루트 compatible 속성 읽기
2. **메모리 초기화**: mm::init() 호출
3. **보드 시스템 초기화**:
   - `boards::init_early()`: 빌트인 보드 등록, DTB compatible로 보드 선택
   - `boards::init()`: 선택된 보드의 init_fn 호출
4. **드라이버 초기화**: 보드 설정을 참조하여 하드웨어 초기화

## 향후 계획

- [ ] 외부 ELF 모듈에서 보드 정보 로드 지원
- [ ] 보드 모듈 핫 언로드 지원
- [ ] 보조 CPU/Hart 실제 시작 로직 구현
- [ ] Per-CPU 데이터 구조 연동
