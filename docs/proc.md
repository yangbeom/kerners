# Process and Thread Management

프로세스/스레드 관리 문서

## Overview

`src/proc/` 모듈은 커널 스레드 추상화와 스케줄링을 제공합니다.

## Thread Model

현재 커널은 커널 스레드만 지원하며, 각 스레드는 독립적인 스택과 실행 컨텍스트를 가집니다.

### Thread Control Block (TCB)

```rust
pub struct Thread {
    pub tid: Tid,              // 스레드 ID
    pub name: String,          // 스레드 이름
    pub state: ThreadState,    // 상태
    pub context: Context,      // CPU 컨텍스트
    pub kernel_stack: Vec<u8>, // 커널 스택
}
```

### Thread States

```rust
pub enum ThreadState {
    Ready,       // 실행 대기
    Running,     // 현재 실행 중
    Blocked,     // 대기 중 (I/O, sleep 등)
    Terminated,  // 종료됨
}
```

## API

### 스레드 생성

```rust
use crate::proc;

fn my_thread_entry() -> ! {
    loop {
        // 스레드 작업
        proc::yield_now();
    }
}

let tid = proc::spawn("my_thread", my_thread_entry);
```

### 스레드 제어

```rust
// 현재 스레드 ID 조회
let tid = proc::current_tid();

// 스레드 양보
proc::yield_now();

// 스레드 목록 출력
proc::dump_threads();

// 스레드 종료
proc::exit();
```

## Context Switching

`src/proc/context.rs`에서 CPU 컨텍스트 저장/복원 처리.

### aarch64 Context

```rust
pub struct Context {
    pub x19: u64,
    pub x20: u64,
    // ... x21-x29
    pub x30: u64,  // LR (return address)
    pub sp: u64,   // Stack pointer
}
```

### riscv64 Context

```rust
pub struct Context {
    pub ra: u64,   // Return address
    pub sp: u64,   // Stack pointer
    pub s0: u64,
    // ... s1-s11
}
```

### Context Switch

```rust
// 어셈블리로 구현
// 현재 컨텍스트 저장 → 새 컨텍스트 복원
unsafe fn switch_context(old: *mut Context, new: *const Context);
```

## Scheduler

`src/proc/scheduler.rs`에서 라운드 로빈 스케줄러 구현.

### 스케줄링 알고리즘

1. Ready 상태의 스레드 중 다음 스레드 선택
2. 현재 스레드 컨텍스트 저장
3. 새 스레드 컨텍스트 복원
4. 새 스레드 실행

```rust
pub fn schedule() {
    let (old_ctx, new_ctx) = {
        let mut threads = THREADS.lock();
        let mut current = CURRENT_THREAD.lock();

        // 다음 실행할 스레드 선택
        let next = find_next_runnable(&threads, *current);

        if next == *current {
            return; // 전환 불필요
        }

        // 컨텍스트 포인터 획득
        let old = &mut threads[*current].context as *mut Context;
        let new = &threads[next].context as *const Context;

        *current = next;
        (old, new)
    };

    unsafe {
        switch_context(old_ctx, new_ctx);
    }
}
```

### 타이머 인터럽트

타이머 인터럽트에서 `schedule()` 호출하여 선점형 스케줄링 구현.

## User Mode

`src/proc/user.rs`에서 유저 모드 전환 지원.

### aarch64

```rust
pub fn enter_user_mode(entry: usize, user_sp: usize) -> ! {
    unsafe {
        // SPSR_EL1 설정 (EL0으로 전환)
        // ELR_EL1에 entry 설정
        // SP_EL0에 user_sp 설정
        // eret 실행
    }
}
```

### riscv64

```rust
pub fn enter_user_mode(entry: usize, user_sp: usize) -> ! {
    unsafe {
        // mstatus의 MPP를 User mode로 설정
        // mepc에 entry 설정
        // sp에 user_sp 설정
        // mret 실행
    }
}
```

## Stack Layout

```
┌─────────────────────┐ High address
│   Thread Stack      │
│   (16KB default)    │
├─────────────────────┤
│   Guard Page        │ (optional)
├─────────────────────┤
│   ...               │
└─────────────────────┘ Low address
```

## Future Work

- [ ] 프로세스 추상화 (주소 공간 분리)
- [ ] 우선순위 기반 스케줄링
- [ ] SMP 지원
- [ ] 프로세스 그룹 / 세션
