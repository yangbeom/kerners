# Block Device Subsystem

블록 디바이스 서브시스템 문서

## Overview

`src/block/` 모듈은 블록 디바이스에 대한 추상화 레이어를 제공합니다.

## Architecture

```
┌─────────────────────────────────────────┐
│           Filesystem Layer               │
│         (fs/fat32, etc.)                 │
├─────────────────────────────────────────┤
│        Block Device Interface            │
│           (block/mod.rs)                 │
├──────────┬──────────────────────────────┤
│  RAMDisk │       VirtIO Block           │
│          │    (virtio_blk.rs)           │
└──────────┴──────────────────────────────┘
```

## BlockDevice Trait

모든 블록 디바이스가 구현해야 하는 인터페이스:

```rust
pub trait BlockDevice: Send + Sync {
    /// 디바이스 이름
    fn name(&self) -> &str;

    /// 블록 크기 (바이트, 일반적으로 512)
    fn block_size(&self) -> usize;

    /// 총 블록 수
    fn block_count(&self) -> u64;

    /// 단일 블록 읽기
    fn read_block(&self, block_num: u64, buf: &mut [u8]) -> BlockResult<()>;

    /// 단일 블록 쓰기
    fn write_block(&self, block_num: u64, buf: &[u8]) -> BlockResult<()>;

    /// 여러 블록 읽기 (기본 구현 제공)
    fn read_blocks(&self, start_block: u64, buf: &mut [u8]) -> BlockResult<()>;

    /// 여러 블록 쓰기 (기본 구현 제공)
    fn write_blocks(&self, start_block: u64, buf: &[u8]) -> BlockResult<()>;

    /// 캐시 동기화
    fn sync(&self) -> BlockResult<()>;

    /// 읽기 전용 여부
    fn is_read_only(&self) -> bool;

    /// 총 용량 (바이트)
    fn capacity(&self) -> u64;
}
```

## Device Registry

### 디바이스 등록

```rust
use crate::block;

// 디바이스 등록
block::register_device("mydevice", Arc::new(my_device));

// 디바이스 조회
if let Some(dev) = block::get_device("mydevice") {
    // 사용
}

// 디바이스 목록
let devices = block::list_devices();

// 디바이스 정보
if let Some(info) = block::device_info("mydevice") {
    println!("Capacity: {} bytes", info.capacity);
}
```

### 디바이스 정보

```rust
pub struct BlockDeviceInfo {
    pub name: String,
    pub block_size: usize,
    pub block_count: u64,
    pub capacity: u64,
    pub read_only: bool,
}
```

## Implementations

### RAMDisk

메모리 기반 블록 디바이스. 테스트 및 임시 저장소에 유용.

```rust
use crate::block::ramdisk::RamDisk;

// 1MB RAM 디스크 생성
let ramdisk = RamDisk::new("ram0", 1024 * 1024);
block::register_device("ram0", Arc::new(ramdisk));
```

**특징:**
- 휘발성 (재부팅 시 데이터 손실)
- 빠른 접근 속도
- 크기 고정

### VirtIO Block

QEMU virtio-blk 디바이스 드라이버.

```rust
use crate::block::virtio_blk;

// 초기화 (block::init()에서 자동 호출)
if let Some(vda) = virtio_blk::init() {
    block::register_device("vda", vda);
}
```

**특징:**
- QEMU에서 실제 디스크 이미지 접근
- MMIO 기반 통신
- Virtqueue를 통한 비동기 I/O (현재는 동기식)

## Error Handling

```rust
pub enum BlockError {
    InvalidBlock,       // 잘못된 블록 번호
    IoError,            // I/O 에러
    DeviceNotFound,     // 디바이스 없음
    BufferSizeMismatch, // 버퍼 크기 불일치
    ReadOnly,           // 읽기 전용
    NotReady,           // 디바이스 준비 안됨
}
```

## Usage Example

```rust
use crate::block;

fn read_boot_sector() -> BlockResult<[u8; 512]> {
    let device = block::get_device("vda")
        .ok_or(BlockError::DeviceNotFound)?;

    let mut buf = [0u8; 512];
    device.read_block(0, &mut buf)?;

    Ok(buf)
}
```

## Adding a New Block Device

1. `src/block/` 하위에 새 모듈 생성
2. `BlockDevice` trait 구현
3. `block/mod.rs`에 모듈 추가
4. 초기화 시 `block::register_device()` 호출
5. `docs/block.md` 문서 업데이트

## VirtIO Block Details

자세한 VirtIO 구현은 [virtio.md](virtio.md) 참조.

### VirtIO Block Config

```rust
struct VirtIOBlkConfig {
    capacity: u64,      // 섹터 수
    size_max: u32,      // 최대 세그먼트 크기
    seg_max: u32,       // 최대 세그먼트 수
    // ...
}
```

### Request Format

```rust
struct VirtIOBlkReq {
    type_: u32,         // 읽기/쓰기
    reserved: u32,
    sector: u64,        // 시작 섹터
    // data[]           // 데이터 버퍼
    // status           // 결과 상태
}
```
