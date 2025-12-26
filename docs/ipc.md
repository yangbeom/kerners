# IPC (Inter-Process Communication)

`src/ipc/` — 메시지 기반 프로세스/스레드 간 통신

## 개요

메시지 큐 기반 IPC 메커니즘을 제공합니다. 타입 안전한 제네릭 구현으로, 블로킹/논블로킹 송수신과 우선순위를 지원합니다.

```
스레드 A                    스레드 B
┌────────┐                 ┌────────┐
│ send() │──→ MessageQueue ──→ receive() │
└────────┘   (Mutex+Semaphore)  └────────┘
```

## 컴포넌트

### MessageQueue\<T\> (무제한)

메모리가 허용하는 한 무한히 메시지를 저장할 수 있는 큐입니다.

```rust
let mq: MessageQueue<u32> = MessageQueue::new();

// 송신 (항상 성공)
mq.send(42)?;
mq.send_priority(99, Priority::Urgent)?;

// 수신 (블로킹 — 메시지가 올 때까지 대기)
let msg = mq.receive()?;
println!("{}", msg.data);  // 99 (Urgent 우선)

// 수신 (논블로킹)
match mq.try_receive() {
    Ok(msg) => println!("{}", msg.data),
    Err(MessageQueueError::Empty) => println!("비어있음"),
}

// 큐 상태
mq.len();       // 메시지 수
mq.is_empty();  // 비어있는지

// 큐 닫기 (더 이상 송신 불가, 남은 메시지는 수신 가능)
mq.close();
```

수신 시 우선순위가 가장 높은 메시지를 먼저 반환합니다.

### BoundedMessageQueue\<T\> (용량 제한)

최대 용량을 지정하는 큐입니다. 가득 차면 송신이 블로킹됩니다.

```rust
let mq: BoundedMessageQueue<u32> = BoundedMessageQueue::new(10);

// 블로킹 송신 (공간이 생길 때까지 대기)
mq.send(42)?;

// 논블로킹 송신 (가득 차면 Err(Full) 반환)
mq.try_send(43)?;

// 수신 (블로킹/논블로킹 동일)
let msg = mq.receive()?;

// 큐 상태
mq.capacity();  // 최대 용량
mq.is_full();   // 가득 찼는지
```

**동기화**: Semaphore 2개 사용 — `sem_slots` (송신자용, 초기값=capacity), `sem_items` (수신자용, 초기값=0).

### Channel\<T\> (Go 스타일)

Sender/Receiver를 분리한 채널입니다.

```rust
// 버퍼 없는 채널 (동기식, capacity=1)
let (tx, rx) = Channel::<u32>::unbuffered();

// 버퍼 있는 채널
let (tx, rx) = Channel::<u32>::bounded(16);

// Sender (Clone 가능)
tx.send(42)?;
tx.try_send(43)?;
let tx2 = tx.clone();

// Receiver
let msg = rx.recv()?;
let msg = rx.try_recv()?;
```

### POSIX mq API

이름 기반 글로벌 메시지 큐 레지스트리입니다 (`ByteMessage = Vec<u8>`).

```rust
// 메시지 큐 열기/생성
let mq = mq_open("my_queue", true)?;  // create=true

// 이름으로 송수신
mq_send("my_queue", b"hello")?;
let data: Vec<u8> = mq_receive("my_queue")?;

// 메시지 큐 삭제
mq_unlink("my_queue")?;
```

## Message 구조체

```rust
pub struct Message<T> {
    pub data: T,                   // 페이로드
    pub priority: Priority,        // 우선순위
    pub sender_id: Option<u64>,    // 송신자 ID (옵션)
    pub timestamp: u64,            // 타임스탬프 (tick 기준)
}
```

### Priority

```rust
pub enum Priority {
    Low    = 0,
    Normal = 1,  // 기본값
    High   = 2,
    Urgent = 3,
}
```

## 에러 처리

```rust
pub enum MessageQueueError {
    Full,     // 큐가 가득 참 (BoundedMessageQueue)
    Empty,    // 큐가 비어있음
    Closed,   // 큐가 닫힘
    Timeout,  // 타임아웃
}
```

## 내부 구현

| 컴포넌트 | 큐 자료구조 | 동기화 |
|----------|-------------|--------|
| MessageQueue | `Mutex<VecDeque<Message<T>>>` | Semaphore (수신 대기) |
| BoundedMessageQueue | `Mutex<VecDeque<Message<T>>>` | Semaphore x2 (송신/수신) |
| Channel | BoundedMessageQueue 래퍼 | 동일 |
| POSIX mq | `RwLock<Vec<(String, &MessageQueue)>>` | MessageQueue 내부 동기화 |

## 향후 계획

- 공유 메모리
- 파이프
