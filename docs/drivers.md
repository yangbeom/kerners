# 드라이버 프레임워크

`src/drivers/` — DTB 기반 디바이스 탐색 및 플랫폼 설정

## 개요

드라이버 프레임워크는 DTB에서 하드웨어 정보를 읽고, 보드 설정 폴백을 적용하여 완전한 플랫폼 설정을 제공합니다.

```
DTB                    probe.rs              config.rs
┌──────────┐          ┌──────────────┐       ┌──────────────────┐
│ DeviceTree│──→ probe_platform() ──→ │ PlatformConfig     │
└──────────┘          │ probe_gic()  │       │ ├─ UartConfig     │
                      │ probe_plic() │       │ ├─ GicConfig      │
BoardConfig           │ probe_uart() │       │ ├─ PlicConfig     │
┌──────────┐          │ probe_timer()│       │ ├─ TimerConfig    │
│ 기본값    │──(폴백)→ │ probe_clint()│       │ └─ cpu_count      │
└──────────┘          └──────────────┘       └──────────────────┘
                                                    ↓
                                             헬퍼 함수들
                                             uart_base(), gicd_base(),
                                             plic_base(), timer_freq() ...
```

## 파일 구조

| 파일 | 설명 |
|------|------|
| `mod.rs` | Driver trait, 드라이버 레지스트리, 등록/probe API |
| `config.rs` | PlatformConfig 및 디바이스별 설정 구조체, 전역 저장소 |
| `probe.rs` | DTB 기반 플랫폼 프로브, 아키텍처별 디바이스 탐색 |

## Driver Trait

```rust
pub trait Driver: Send + Sync {
    fn name(&self) -> &str;
    fn compatible(&self) -> &[&str];
    fn probe(&self, info: &DeviceInfo) -> DriverResult<()>;
}
```

- `compatible()`: DTB의 compatible 문자열과 매칭할 문자열 목록
- `probe()`: DTB에서 찾은 디바이스 정보로 드라이버 초기화

## 플랫폼 설정 (PlatformConfig)

```rust
pub struct PlatformConfig {
    pub uart: UartConfig,
    pub interrupt_controller: InterruptControllerConfig,
    pub timer: TimerConfig,
    pub clint: Option<ClintConfig>,  // RISC-V only
    pub cpu_count: usize,
}
```

### 디바이스별 설정 구조체

| 구조체 | 아키텍처 | 주요 필드 |
|--------|----------|-----------|
| `UartConfig` | 공통 | base, size, irq, clock_freq |
| `GicConfig` | aarch64 | distributor_base, cpu_interface_base, version |
| `PlicConfig` | riscv64 | base, size, num_sources, num_contexts |
| `ClintConfig` | riscv64 | base, size |
| `TimerConfig` | 공통 | timer_type, frequency, irq |

### InterruptControllerConfig

```rust
pub enum InterruptControllerConfig {
    Gic(GicConfig),     // aarch64
    Plic(PlicConfig),   // riscv64
    None,
}
```

## 디바이스 프로브 흐름

`probe_platform()`이 부팅 시 호출됩니다:

1. DTB에서 인터럽트 컨트롤러 탐색 (GIC 또는 PLIC)
2. DTB에서 UART 탐색
3. 타이머 설정 (aarch64: `CNTFRQ_EL0` 레지스터, riscv64: BoardConfig)
4. CLINT 탐색 (riscv64만)
5. DTB에서 CPU 개수 확인
6. `PlatformConfig` 생성 후 `init_platform_config()`로 전역 저장

**DTB 폴백**: DTB에서 디바이스를 찾지 못하면 `boards/` 모듈의 `BoardConfig` 상수를 사용합니다.

## 헬퍼 함수

각 헬퍼 함수는 PlatformConfig를 먼저 확인하고, 없으면 BoardConfig 기본값을 반환합니다:

```rust
pub fn uart_base() -> usize       // UART 기본 주소
pub fn uart_irq() -> u32          // UART IRQ 번호
pub fn gicd_base() -> usize       // GIC Distributor (aarch64)
pub fn gicc_base() -> usize       // GIC CPU Interface (aarch64)
pub fn plic_base() -> usize       // PLIC 기본 주소 (riscv64)
pub fn clint_base() -> usize      // CLINT 기본 주소 (riscv64)
pub fn timer_freq() -> u64        // 타이머 주파수
pub fn timer_irq() -> u32         // 타이머 IRQ 번호
pub fn cpu_count() -> usize       // CPU 개수
```

## 드라이버 레지스트리 API

```rust
// 드라이버 등록
drivers::register_driver(Arc::new(MyDriver));

// DTB 순회하며 등록된 드라이버와 매칭, probe() 호출
drivers::probe_all();

// 등록된 드라이버 목록 출력
drivers::list_drivers();

// compatible 문자열로 디바이스 정보 조회
let info: Option<DeviceInfo> = drivers::find_device("virtio,mmio");
let infos: Vec<DeviceInfo> = drivers::find_devices("virtio,mmio");
```

## 에러 처리

```rust
pub enum DriverError {
    DeviceNotFound,      // 디바이스 없음
    InitFailed,          // 초기화 실패
    AlreadyInitialized,  // 이미 초기화됨
    NotSupported,        // 지원하지 않는 디바이스
    OutOfResources,      // 리소스 부족
}
```
