# 커널 로깅 시스템

`src/log/` — 로그 레벨, 타임스탬프, CPU ID, 링 버퍼(dmesg) 기반 커널 로깅

## 개요

커널의 모든 출력(`kprintln!` 포함)을 구조화된 로그로 관리합니다.

- 5단계 로그 레벨 (ERROR ~ TRACE)
- 타임스탬프 + CPU ID 접두사
- 64KB 링 버퍼 (dmesg 스타일)
- 런타임 로그 레벨 변경
- SMP-safe, 재귀 방지

## 로그 출력 포맷

```
[     0.000123] CPU0  INFO: kerners booting...
[     0.001456] CPU0 ERROR: Page fault at 0xdeadbeef
[     1.234567] CPU1 DEBUG: Timer tick
```

포맷: `[{seconds:>6}.{micros:06}] CPU{id} {LEVEL}: {message}`

## 매크로

### 레벨별 로그 매크로

```rust
log_error!("critical failure: {}", reason);   // ERROR (레벨 0)
log_warn!("deprecated API called");            // WARN  (레벨 1)
log_info!("system initialized");               // INFO  (레벨 2)
log_debug!("buffer size: {}", size);           // DEBUG (레벨 3)
log_trace!("entering function foo()");         // TRACE (레벨 4)
```

### kprintln!

`kprintln!`은 `log_info!`와 동일하게 동작합니다:

```rust
kprintln!("hello");
// 출력: [     0.001234] CPU0  INFO: hello
```

### kprint!

`kprint!`는 raw UART 출력으로, 로그 시스템을 거치지 않습니다. 셸 프롬프트 등 접두사가 불필요한 출력에 사용합니다.

## 로그 레벨

| 레벨 | 값 | 설명 |
|------|-----|------|
| ERROR | 0 | 치명적 오류 |
| WARN | 1 | 경고 |
| INFO | 2 | 정보성 메시지 (기본값) |
| DEBUG | 3 | 디버깅 메시지 |
| TRACE | 4 | 상세 추적 |

기본 로그 레벨은 **INFO (2)**입니다. 현재 레벨보다 높은 숫자의 메시지는 필터링됩니다.

## 셸 명령어

### loglevel

```
kerners> loglevel
Current log level: 2 ( INFO)

kerners> loglevel 4
Log level set to: 4 (TRACE)

kerners> loglevel ERROR
Log level set to: 0 (ERROR)
```

인자: `0`-`4` 또는 `ERROR`/`WARN`/`INFO`/`DEBUG`/`TRACE` (대소문자 무시)

### dmesg

```
kerners> dmesg
[     0.000001] CPU0  INFO: [boot] DTB address from register x0: 0x44000000
[     0.000123] CPU0  INFO: [DTB] Memory: base=0x40000000, size=0x20000000
...
```

커널 링 버퍼에 저장된 모든 로그 메시지를 시간순으로 출력합니다.

## 아키텍처

### 모듈 구조

```
src/log/
├── mod.rs      코어 로깅 엔진 (log 함수, 타임스탬프, 재귀 방지)
├── buffer.rs   64KB 링 버퍼 (엔트리 저장/파싱)
└── macros.rs   log_error! ~ log_trace! 매크로
```

### 링 버퍼 엔트리 포맷

```
[4 bytes: total_length (u32 LE)]
[1 byte:  level]
[8 bytes: timestamp_us (u64 LE)]
[1 byte:  cpu_id]
[N bytes: message]
```

헤더: 14바이트, 버퍼 크기: 64KB (정적 할당)

### 타임스탬프 소스

| 아키텍처 | 카운터 | 주파수 |
|----------|--------|--------|
| aarch64 | `CNTPCT_EL0` | `CNTFRQ_EL0` |
| riscv64 | `mtime` (CLINT) | `boards::timer_freq()` |

마이크로초 정밀도: `(counter % freq) * 1_000_000 / freq`

### SMP 안전성

- **Spinlock**: 링 버퍼 접근 보호 (IRQ 컨텍스트에서도 안전)
- **Per-CPU 재귀 방지**: `AtomicBool` × 8개로 CPU별 로깅 중 상태 추적. 로깅 중 다시 로그를 호출하면 무시하여 deadlock 방지

### 초기화 전 동작

`log::init()` 호출 전에 `kprintln!`이 사용될 경우, fallback으로 직접 UART 출력합니다 (타임스탬프/CPU ID/링 버퍼 없이 원본 메시지만 출력).

### 컴파일러 intrinsic 의존성

로깅 시스템은 스택 버퍼(`[0u8; 512]`)와 `copy_from_slice`를 사용합니다. release 빌드에서 컴파일러가 이를 `memset`/`memcpy` 호출로 최적화하므로, 커널에 올바른 구현이 필요합니다.

이 구현들은 `src/module/symbol.rs`에 `volatile` 연산으로 작성되어 있습니다. 일반 루프로 작성하면 컴파일러가 다시 `memcpy` 호출로 최적화하여 무한 재귀가 발생합니다.

## 커널 모듈에서 사용

```c
// extern 선언
extern void kernel_log(uint8_t level, const char* msg, size_t len);

// 사용
kernel_log(0, "error msg", 9);   // ERROR
kernel_log(2, "info msg", 8);    // INFO
```

심볼 `kernel_log`가 커널 심볼 테이블에 등록되어 있습니다.

## 파일 목록

| 파일 | 설명 |
|------|------|
| `src/log/mod.rs` | 코어 로깅 엔진, `log()` 함수, 타임스탬프 계산 |
| `src/log/buffer.rs` | 64KB 링 버퍼 구현, `dump_logs()` |
| `src/log/macros.rs` | `log_error!` ~ `log_trace!` 매크로 정의 |
| `src/console.rs` | `kprintln!` 매크로 (→ `log_info!`로 라우팅) |
| `src/module/test_symbols.rs` | `kernel_log` 심볼 래퍼 |
| `modules/test_log/` | 로깅 시스템 테스트 모듈 |
