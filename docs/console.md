# 콘솔 출력

`src/console.rs` — 커널 콘솔 I/O 추상화

## 개요

아키텍처별 UART 드라이버를 래핑하여 `kprint!`/`kprintln!` 매크로를 제공합니다. `core::fmt::Write` trait을 구현하여 Rust의 포맷 문자열을 지원합니다.

## 매크로

```rust
// 포맷 출력 (개행 없음)
kprint!("Hello {}", name);

// 포맷 출력 (개행 포함)
kprintln!("Value: {:#x}", addr);

// 빈 줄
kprintln!();
```

## 함수

| 함수 | 설명 |
|------|------|
| `puts(s: &str)` | 문자열을 UART로 출력 |
| `putc(c: u8)` | 단일 바이트를 UART로 출력 |
| `kprint(args: fmt::Arguments)` | 포맷 문자열 출력 |
| `kprintln(args: fmt::Arguments)` | 포맷 문자열 + 개행 출력 |

## 아키텍처 연동

`putc_arch()` 내부 함수가 `#[cfg(target_arch)]`로 분기하여 해당 아키텍처의 `crate::arch::uart::putc(c)` 를 호출합니다.

- **aarch64**: `src/arch/aarch64/uart.rs`
- **riscv64**: `src/arch/riscv64/uart.rs`
