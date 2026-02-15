# Process and Thread Management

프로세스/스레드 관리 문서

## Overview

`src/proc/` 모듈은 커널 스레드 추상화와 스케줄링을 제공합니다.

프로세스 관련 syscall의 최소 메타데이터(`ppid/pgid/sid/signal_mask/pending`)는
`src/syscall/process.rs`에서 별도로 관리합니다.

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
    pub user_stack: Option<Vec<u8>>, // execve 후 유저 스택 보관
    pub user_root_table: usize, // aarch64/riscv64: 스레드별 유저 루트 페이지 테이블
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

- aarch64/riscv64에서는 컨텍스트 스위치 직전에 `Thread.user_root_table`로 루트 페이지 테이블을 전환합니다.
- 현재 정책은 ASID 없이 전역 TLB flush를 사용합니다.

### 타이머 인터럽트

타이머 인터럽트 경로에서 sleep queue wakeup을 처리합니다.

- aarch64: wakeup 이벤트가 발생한 틱에서만 `schedule()` 호출 (강제 선점 비활성)
- riscv64: wakeup 처리 후 기존 틱 기반 선점 스케줄링 유지

### sleep queue

- `SleepEntry { tid, deadline_ns, wake_reason }`로 대기 항목을 관리합니다.
- `sleep_current_until(deadline_ns)`는 현재 스레드를 `Blocked`로 전환하고 스케줄링합니다.
- `wake_sleepers_by_timer(now_ns)`는 deadline이 지난 스레드를 `Ready`로 전환합니다.
- `wake_thread_for_signal(tid)`는 signal 사유로 대기 중 스레드를 깨우고 wake reason을 `Signal`로 기록합니다.

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

### execve 전이

- `sys_execve`는 즉시 `eret/mret` 하지 않고 pending 전이 정보를 저장합니다.
- syscall trap 복귀 경로에서 현재 컨텍스트의 `PC/SP`를 새 ELF 이미지의 엔트리/스택으로 교체합니다.
- 유저 스택 메모리는 현재 스레드(`Thread.user_stack`)에 바인딩해 수명을 보장합니다.
- 유저 초기 스택의 auxv에는 최소 호환 키를 포함합니다:
  - `AT_ENTRY`, `AT_PHDR`, `AT_PHNUM`, `AT_PAGESZ`
- aarch64/riscv64 경로에서는 `path/argv/envp` 유저 포인터 범위를 선검증합니다.
- 실행 파일은 static ELF(`ET_EXEC`) 기준이며, `PT_INTERP`를 포함한 동적 ELF는 지원하지 않습니다.
- ELF `PT_LOAD` 세그먼트의 가상주소가 현재 identity-mapped RAM 범위를 벗어나면 exec 준비가 실패합니다.

### fork/vfork/wait 최소 동작

- aarch64/riscv64 유저 syscall 경로에서는 부모 trap context를 복사해 자식이 `sys_clone/fork/vfork`에서 0을 반환하도록 복귀합니다.
- `sys_clone`는 `CLONE_VM/CLONE_FS/CLONE_FILES/CLONE_SIGHAND` 플래그를 리소스 그룹 메타데이터로 추적합니다.
- `sys_exit`는 부모의 zombie 리스트에 종료 상태를 등록하고 `SIGCHLD`를 큐잉합니다.
- `sys_wait4`는 zombie를 회수하고 Linux wait status(`exit_code << 8`)를 기록합니다.
- `sys_waitid`는 `P_ALL/P_PID/P_PGID` + `WEXITED/WNOHANG/WNOWAIT` 최소 조합을 지원합니다.
- 부모가 먼저 종료되면 자식/좀비를 init(`pid=1`)으로 reparent합니다.

### 부팅 시 PID 1 실행 경로

- 커널은 부팅 후 init 후보 경로를 순서대로 탐색합니다:
  - `/sbin/init` → `/etc/init` → `/bin/init` → `/bin/sh`
  - 현재 루트(RamFS)에 없으면 `/mnt/*` 경로를 fallback으로 탐색
- `/dev/vda`가 존재하면 FAT32를 `/mnt`에 자동 마운트하여 외부 ELF 탐색 경로를 확보합니다.
- init 스레드(`tid=1`)는 `prepare_exec_image()`로 준비한 `PreparedExecImage`를 받아
  아키텍처별 `eret/mret`로 직접 유저 모드 진입합니다.
- init 실행에 실패하면 커널 셸로 fallback합니다.

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

## 현재 제약

- aarch64/riscv64는 vm_group 기반 주소공간 분리 + file-backed `mmap` + fork COW를 지원합니다.
- signal core는 구현되어 syscall/interrupt 복귀 직전에 pending unmasked signal 1건을 전달합니다.
- `rt_sigtimedwait`의 완전한 timeout/blocking 모델, `SA_RESTART` 자동 재시작, job-control(`SIGSTOP/SIGCONT`)은 아직 미구현입니다.
