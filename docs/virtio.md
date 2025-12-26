# VirtIO Driver Framework

VirtIO 드라이버 프레임워크 문서

## Overview

`src/virtio/` 모듈은 VirtIO MMIO 기반 디바이스 드라이버 프레임워크를 제공합니다.

## VirtIO Specification

VirtIO는 가상화 환경에서 효율적인 I/O를 위한 표준화된 인터페이스입니다.

### 지원 디바이스 타입

```rust
pub enum DeviceType {
    Invalid = 0,
    Network = 1,    // virtio-net
    Block = 2,      // virtio-blk
    Console = 3,    // virtio-console
    Entropy = 4,    // virtio-rng
    Gpu = 16,       // virtio-gpu
    Input = 18,     // virtio-input
    // ...
}
```

## MMIO Interface

`src/virtio/mmio.rs`에서 VirtIO MMIO 레지스터 인터페이스 구현.

### 레지스터 레이아웃

| Offset | Name | Description |
|--------|------|-------------|
| 0x000 | MagicValue | "virt" (0x74726976) |
| 0x004 | Version | 디바이스 버전 |
| 0x008 | DeviceID | 디바이스 타입 |
| 0x00c | VendorID | 벤더 ID |
| 0x010 | DeviceFeatures | 디바이스 피처 |
| 0x014 | DeviceFeaturesSel | 피처 선택 |
| 0x020 | DriverFeatures | 드라이버 피처 |
| 0x024 | DriverFeaturesSel | 피처 선택 |
| 0x030 | QueueSel | 큐 선택 |
| 0x034 | QueueNumMax | 최대 큐 크기 |
| 0x038 | QueueNum | 현재 큐 크기 |
| 0x044 | QueueReady | 큐 준비 상태 |
| 0x050 | QueueNotify | 큐 알림 |
| 0x060 | InterruptStatus | 인터럽트 상태 |
| 0x064 | InterruptACK | 인터럽트 확인 |
| 0x070 | Status | 디바이스 상태 |
| 0x080 | QueueDescLow/High | 디스크립터 주소 |
| 0x090 | QueueDriverLow/High | 드라이버 영역 주소 |
| 0x0a0 | QueueDeviceLow/High | 디바이스 영역 주소 |
| 0x100+ | Config | 디바이스별 설정 |

### VirtIOMMIO 구조체

```rust
pub struct VirtIOMMIO {
    base: usize,
}

impl VirtIOMMIO {
    pub unsafe fn new(base: usize) -> Self;
    pub fn is_valid(&self) -> bool;
    pub fn device_type(&self) -> DeviceType;
    pub fn version(&self) -> u32;

    // 초기화 시퀀스
    pub fn reset(&self);
    pub fn set_status(&self, status: u32);
    pub fn negotiate_features(&self, driver_features: u64) -> u64;
    pub fn setup_queue(&self, queue_num: u32, queue: &VirtQueue);
    pub fn notify_queue(&self, queue_num: u32);
}
```

## Virtqueue

`src/virtio/queue.rs`에서 Virtqueue 구현.

### Split Virtqueue 구조

```
┌─────────────────────────────────────────┐
│           Descriptor Table               │
│  ┌─────┬─────┬─────┬─────┬─────┐       │
│  │ D0  │ D1  │ D2  │ ... │ Dn  │       │
│  └─────┴─────┴─────┴─────┴─────┘       │
├─────────────────────────────────────────┤
│           Available Ring                 │
│  ┌──────┬──────┬──────────────┐        │
│  │flags │ idx  │ ring[0..n]   │        │
│  └──────┴──────┴──────────────┘        │
├─────────────────────────────────────────┤
│            Used Ring                     │
│  ┌──────┬──────┬──────────────┐        │
│  │flags │ idx  │ ring[0..n]   │        │
│  └──────┴──────┴──────────────┘        │
└─────────────────────────────────────────┘
```

### Descriptor

```rust
#[repr(C)]
struct VirtqDesc {
    addr: u64,      // 버퍼 물리 주소
    len: u32,       // 버퍼 길이
    flags: u16,     // 플래그 (NEXT, WRITE, INDIRECT)
    next: u16,      // 다음 디스크립터 인덱스
}
```

### VirtQueue API

```rust
pub struct VirtQueue {
    // ...
}

impl VirtQueue {
    pub fn new(size: u16) -> Self;

    // 디스크립터 추가
    pub fn add_buf(&mut self,
                   output: &[&[u8]],    // 디바이스로 보낼 데이터
                   input: &mut [&mut [u8]]) // 디바이스에서 받을 데이터
                   -> Option<u16>;

    // 완료된 요청 확인
    pub fn get_buf(&mut self) -> Option<(u16, u32)>;

    // 사용 가능한 슬롯 수
    pub fn available_slots(&self) -> u16;
}
```

## Device Discovery

DTB에서 VirtIO 디바이스 탐색:

```rust
use crate::virtio;

// 모든 VirtIO 디바이스 검색
let devices = virtio::find_virtio_devices();

for dev in devices {
    println!("{:?} @ {:#x}", dev.device_type, dev.mmio_base);
}
```

## Initialization Sequence

1. **매직 넘버 확인** (0x74726976 = "virt")
2. **버전 확인** (레거시 또는 1.0+)
3. **디바이스 타입 확인**
4. **리셋** (status = 0)
5. **ACKNOWLEDGE** 설정
6. **DRIVER** 설정
7. **피처 협상**
8. **FEATURES_OK** 설정
9. **큐 설정**
10. **DRIVER_OK** 설정

```rust
unsafe fn init_virtio_device(mmio: &VirtIOMMIO) -> Result<(), VirtIOError> {
    // 1-3. 검증
    if !mmio.is_valid() {
        return Err(VirtIOError::InvalidMagic);
    }

    // 4. 리셋
    mmio.reset();

    // 5-6. 상태 설정
    mmio.set_status(STATUS_ACKNOWLEDGE);
    mmio.set_status(STATUS_DRIVER);

    // 7. 피처 협상
    let features = mmio.negotiate_features(DRIVER_FEATURES);

    // 8. FEATURES_OK
    mmio.set_status(STATUS_FEATURES_OK);

    // 9. 큐 설정
    let queue = VirtQueue::new(QUEUE_SIZE);
    mmio.setup_queue(0, &queue);

    // 10. DRIVER_OK
    mmio.set_status(STATUS_DRIVER_OK);

    Ok(())
}
```

## Error Handling

```rust
pub enum VirtIOError {
    InvalidMagic,               // 매직 넘버 불일치
    UnsupportedVersion,         // 지원하지 않는 버전
    NoDevice,                   // 디바이스 없음
    FeatureNegotiationFailed,   // 피처 협상 실패
    QueueSetupFailed,           // 큐 설정 실패
    IoError,                    // I/O 에러
    BufferTooSmall,             // 버퍼 크기 부족
    Timeout,                    // 타임아웃
}
```

## Interrupt Handling

```rust
fn handle_virtio_interrupt(dev: &VirtIODevice) {
    let status = dev.mmio.interrupt_status();

    if status & VIRTIO_INTERRUPT_USED_RING != 0 {
        // 완료된 요청 처리
        while let Some((idx, len)) = dev.queue.get_buf() {
            // 요청 완료 처리
        }
    }

    // 인터럽트 확인
    dev.mmio.interrupt_ack(status);
}
```

## Adding a New VirtIO Device Driver

1. `src/virtio/` 또는 관련 서브시스템에 드라이버 추가
2. `VirtIOMMIO`를 사용하여 MMIO 접근
3. `VirtQueue`를 사용하여 요청 처리
4. 인터럽트 핸들러 등록
5. `docs/virtio.md` 문서 업데이트
