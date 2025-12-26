# Synchronization Primitives

커널 동기화 프리미티브 문서

## Overview

`src/sync/` 모듈은 커널 내 동시성 제어를 위한 다양한 동기화 프리미티브를 제공합니다.

## Primitives

### Spinlock

Busy-waiting 기반의 가장 기본적인 락.

```rust
use crate::sync::Spinlock;

static DATA: Spinlock<u32> = Spinlock::new(0);

fn example() {
    let mut guard = DATA.lock();
    *guard += 1;
    // guard가 drop되면 자동으로 unlock
}
```

**특징:**
- 인터럽트 비활성화 없이 동작
- 짧은 critical section에 적합
- 컨텍스트 스위칭 없음 (busy-wait)

### Mutex

어댑티브 뮤텍스 (spin then yield).

```rust
use crate::sync::Mutex;

static RESOURCE: Mutex<Vec<u8>> = Mutex::new(Vec::new());

fn example() {
    let mut guard = RESOURCE.lock();
    guard.push(42);
}
```

**특징:**
- 일정 횟수 스핀 후 yield
- 스케줄러와 연동하여 효율적인 대기
- 긴 critical section에 적합

### RwLock

Reader-Writer 락. 다수의 reader 또는 단일 writer 허용.

```rust
use crate::sync::RwLock;

static CONFIG: RwLock<Config> = RwLock::new(Config::default());

fn read_config() {
    let guard = CONFIG.read();
    // 여러 스레드가 동시에 읽기 가능
}

fn write_config() {
    let mut guard = CONFIG.write();
    // 단일 스레드만 쓰기 가능
}
```

**특징:**
- 읽기 위주 워크로드에 최적화
- Writer starvation 가능성 있음

### Semaphore

카운팅 세마포어. 리소스 풀 관리에 적합.

```rust
use crate::sync::Semaphore;

static POOL: Semaphore = Semaphore::new(10); // 10개 리소스

fn acquire_resource() {
    POOL.acquire();
    // 리소스 사용
    POOL.release();
}
```

**특징:**
- 초기 카운트 지정 가능
- 리소스 제한에 유용

### SeqLock

순차 락. Writer 우선, 읽기 시 재시도 필요.

```rust
use crate::sync::SeqLock;

static STATS: SeqLock<Statistics> = SeqLock::new(Statistics::default());

fn read_stats() -> Statistics {
    loop {
        let seq = STATS.read_begin();
        let stats = STATS.read();
        if STATS.read_retry(seq) {
            continue;
        }
        return stats;
    }
}

fn update_stats() {
    STATS.write(|stats| {
        stats.count += 1;
    });
}
```

**특징:**
- Writer가 reader를 block하지 않음
- Reader는 inconsistent read 감지 시 재시도
- 통계, 타임스탬프 등에 적합

### RCU (Read-Copy-Update)

락-프리 읽기를 제공하는 동기화 메커니즘.

```rust
use crate::sync::RcuCell;

static DATA: RcuCell<Config> = RcuCell::new(Config::default());

fn read_data() {
    let guard = DATA.read();
    // 락 없이 읽기
}

fn update_data(new_config: Config) {
    DATA.update(new_config);
    // 이전 데이터는 grace period 후 해제
}
```

**특징:**
- 읽기 경로에 락 없음
- 쓰기 시 새 복사본 생성
- 읽기 위주 워크로드에 최적

## Usage Guidelines

| Primitive | Use Case | Overhead |
|-----------|----------|----------|
| Spinlock | 매우 짧은 critical section | 낮음 (busy-wait) |
| Mutex | 일반적인 mutual exclusion | 중간 |
| RwLock | 읽기 위주 데이터 | 중간 |
| Semaphore | 리소스 풀, 생산자-소비자 | 중간 |
| SeqLock | 통계, 타임스탬프 | 낮음 (reader) |
| RCU | 읽기 위주, 락-프리 필요 | 낮음 (reader) |

## Deadlock Prevention

1. 항상 일정한 순서로 락 획득
2. 락을 보유한 채로 sleep하지 않기 (Spinlock)
3. 중첩 락 최소화
4. try_lock 활용하여 타임아웃 구현
