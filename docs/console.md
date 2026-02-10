# 콘솔 출력

`src/console.rs` — 커널 콘솔 I/O 추상화

## 개요

아키텍처별 UART 드라이버를 래핑하여 `kprint!`/`kprintln!` 매크로를 제공합니다. `core::fmt::Write` trait을 구현하여 Rust의 포맷 문자열을 지원합니다.

## 매크로

### kprintln!

`kprintln!`은 커널 로깅 시스템(`src/log/`)을 통해 출력됩니다. `log_info!`와 동일하게 동작하며, 타임스탬프와 CPU ID 접두사가 자동으로 붙고 링 버퍼에 저장됩니다.

```rust
kprintln!("Value: {:#x}", addr);
// 출력: [     0.001234] CPU0  INFO: Value: 0x1000

kprintln!();
// 빈 줄 출력
```

로깅 시스템 초기화 전(`log::init()` 호출 전)에는 fallback으로 직접 UART에 출력됩니다.

### kprint!

`kprint!`는 로깅 시스템을 거치지 않고 UART에 직접 출력합니다. 셸 프롬프트 등 접두사가 불필요한 출력에 사용합니다.

```rust
kprint!("kerners> ");   // 접두사 없이 raw 출력
```

## 함수

| 함수 | 설명 |
|------|------|
| `puts(s: &str)` | 문자열을 UART로 직접 출력 (로그 시스템 비경유) |
| `putc(c: u8)` | 단일 바이트를 UART로 출력 |
| `kprint(args: fmt::Arguments)` | 포맷 문자열 직접 출력 |
| `kprintln(args: fmt::Arguments)` | 포맷 문자열 + 개행 직접 출력 |

> `puts`, `putc`, `kprint`, `kprintln` 함수는 로그 시스템을 거치지 않는 raw 출력입니다. 로그 시스템 내부에서 UART 출력용으로 사용됩니다.

## 아키텍처 연동

`putc_arch()` 내부 함수가 `#[cfg(target_arch)]`로 분기하여 해당 아키텍처의 `crate::arch::uart::putc(c)` 를 호출합니다.

- **aarch64**: `src/arch/aarch64/uart.rs`
- **riscv64**: `src/arch/riscv64/uart.rs`

## 관련 문서

- [log.md](log.md) — 커널 로깅 시스템 (로그 레벨, 타임스탬프, 링 버퍼)
