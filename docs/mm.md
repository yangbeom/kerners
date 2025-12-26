# 메모리 관리 (Memory Management)

kerners의 메모리 관리 서브시스템 문서입니다.

## 개요

메모리 관리 모듈(`src/mm/`)은 커널의 메모리 할당과 관리를 담당합니다.

```
src/mm/
├── mod.rs      # 메모리 관리 초기화 및 레이아웃 계산
├── heap.rs     # 힙 할당자 (linked_list_allocator 연동)
└── page.rs     # 페이지 프레임 할당자 (비트맵 기반)
```

### 주요 기능

| 모듈 | 설명 |
|------|------|
| `mm::init()` | DTB에서 획득한 RAM 정보로 전체 메모리 시스템 초기화 |
| `mm::heap` | `Box`, `Vec`, `String` 등 동적 할당 지원 |
| `mm::page` | 물리 페이지 프레임 할당/해제 |

---

## 메모리 레이아웃

### aarch64 (QEMU virt)

```
RAM 시작: 0x40000000

0x40000000 ┌──────────────────────────┐
           │  QEMU Reserved           │  512KB (QEMU 내부 사용)
0x40080000 ├──────────────────────────┤  ← 커널 시작 (_start)
           │  .text                   │  커널 코드
           │  .rodata                 │  읽기 전용 데이터
           │  .data                   │  초기화된 데이터
           │  .bss                    │  미초기화 데이터
           ├──────────────────────────┤  ← 커널 끝 (_end), 4KB 정렬
           │  Stack                   │  256KB
           ├──────────────────────────┤
           │                          │
           │  Heap                    │  RAM의 1/4, 최대 128MB
           │  (linked_list_allocator) │
           │                          │
           ├──────────────────────────┤  ← 4KB 정렬
           │                          │
           │  Frame Pool              │  페이지 프레임 할당 영역
           │  (bitmap allocator)      │  (비트맵 포함)
           │                          │
           ├──────────────────────────┤
           │  Reserved                │  4MB (DTB 등)
           └──────────────────────────┘  ← RAM 끝
```

### 메모리 크기별 레이아웃 예시

| RAM | 힙 크기 | Frame Pool | 비고 |
|-----|---------|------------|------|
| 128MB | 32MB | ~91MB | 비트맵 1페이지 |
| 256MB | 64MB | ~187MB | 비트맵 2페이지 |
| 512MB | 128MB | ~375MB | 비트맵 3페이지 |
| 1GB | 128MB | ~891MB | 힙 최대 128MB 제한 |

---

## 힙 할당자 (`mm::heap`)

### 구현

`linked_list_allocator` crate를 사용하여 힙 메모리를 관리합니다.

```rust
#[global_allocator]
static HEAP_ALLOCATOR: LockedHeap = LockedHeap::empty();
```

### 초기화

```rust
// mm::init() 내부에서 호출됨
mm::heap::init(heap_start, heap_size)?;
```

- `heap_start`: 커널 끝(`_end`) 이후, 4KB 정렬된 주소
- `heap_size`: RAM의 1/4, 최대 128MB

### API

```rust
// 초기화 확인
mm::heap::is_initialized() -> bool

// 통계 조회
mm::heap::stats() -> HeapStats
mm::heap::dump_stats()  // 콘솔 출력
```

### 사용 예시

```rust
extern crate alloc;
use alloc::vec::Vec;
use alloc::boxed::Box;
use alloc::string::String;

// Box 할당
let boxed = Box::new(42u64);

// Vec 사용
let mut vec: Vec<u32> = Vec::new();
vec.push(1);
vec.push(2);

// String 사용
let s = String::from("Hello, kerners!");
```

---

## 페이지 프레임 할당자 (`mm::page`)

### 구현 방식

비트맵(Bitmap) 기반 할당자입니다.

- **페이지 크기**: 4KB (4096 bytes)
- **비트맵**: 1비트 = 1페이지 (0=free, 1=allocated)
- **할당 전략**: First-fit with next-search 최적화

```
비트맵 구조:
┌────────────────────────────────────────────────────────────────┐
│ u64[0]: 페이지 0-63 │ u64[1]: 페이지 64-127 │ ...              │
└────────────────────────────────────────────────────────────────┘
```

### 비트맵 오버헤드

비트맵은 Frame Pool 영역의 시작에 배치됩니다.

```
페이지 수 = Frame Pool 크기 / 4KB
비트맵 크기 = ceil(페이지 수 / 8) bytes
비트맵 페이지 = ceil(비트맵 크기 / 4KB)
```

| Frame Pool | 페이지 수 | 비트맵 크기 | 비트맵 페이지 |
|------------|-----------|-------------|---------------|
| 91MB | 23,296 | 2,912 bytes | 1 |
| 187MB | 47,872 | 5,984 bytes | 2 |
| 375MB | 96,000 | 12,000 bytes | 3 |

### 초기화

```rust
// mm::init() 내부에서 호출됨
mm::page::init(frame_alloc_start, frame_alloc_size)?;
```

초기화 시:
1. 비트맵을 0으로 클리어 (모든 페이지 free)
2. 비트맵 자체가 차지하는 페이지들을 allocated로 표시
3. `next_search`를 비트맵 이후로 설정

### API

```rust
/// 단일 페이지 할당 (4KB)
mm::page::alloc_frame() -> Option<usize>

/// 연속 n개 페이지 할당
mm::page::alloc_frames(count: usize) -> Option<usize>

/// 단일 페이지 해제
unsafe { mm::page::free_frame(addr); }

/// 연속 n개 페이지 해제
unsafe { mm::page::free_frames(addr, count); }

/// 통계 조회
mm::page::stats() -> FrameAllocatorStats
mm::page::dump_stats()  // 콘솔 출력
```

### 사용 예시

```rust
// 단일 페이지 할당
if let Some(frame) = mm::page::alloc_frame() {
    kprintln!("Allocated frame at {:#x}", frame);
    
    // 페이지 사용...
    let ptr = frame as *mut u8;
    
    // 해제
    unsafe { mm::page::free_frame(frame); }
}

// 연속 4개 페이지 할당 (16KB)
if let Some(frames) = mm::page::alloc_frames(4) {
    kprintln!("Allocated 4 frames starting at {:#x}", frames);
    
    // 해제
    unsafe { mm::page::free_frames(frames, 4); }
}
```

### 할당 알고리즘

**First-fit with next-search optimization**:

1. `next_search` 위치부터 순차 탐색
2. 연속 `count`개의 free 페이지 발견 시 할당
3. 전체 순회 후에도 못 찾으면 `None` 반환
4. 해제 시 `next_search`를 해제 위치로 갱신하여 재사용 촉진

```
할당 전:  [1][1][0][0][0][0][1][0][0]...
           ↑ bitmap    ↑ next_search

alloc_frames(3) 호출:

할당 후:  [1][1][1][1][1][0][1][0][0]...
           ↑ bitmap        ↑ next_search (갱신)
```

---

## 초기화 흐름

```
_entry()
  └─► aarch64_start() / riscv64_start()
        │
        ├─► DTB 파싱하여 RAM 정보 획득
        │     └─► (ram_base, ram_size)
        │
        └─► mm::init(ram_base, ram_size)
              │
              ├─► 메모리 레이아웃 계산
              │     ├─► kernel_end = _end (링커 심볼)
              │     ├─► heap_start = align_4k(kernel_end)
              │     ├─► heap_size = min(ram_size/4, 128MB)
              │     └─► frame_alloc = heap_end ~ (ram_end - 4MB)
              │
              ├─► heap::init(heap_start, heap_size)
              │     └─► linked_list_allocator 초기화
              │
              └─► page::init(frame_alloc_start, frame_alloc_size)
                    ├─► 비트맵 초기화
                    └─► 비트맵 페이지들 allocated 표시
```

---

## 설정 가능한 값

| 상수/변수 | 위치 | 기본값 | 설명 |
|-----------|------|--------|------|
| `max_heap_size` | `mm/mod.rs` | 128MB | 힙 최대 크기 |
| `reserved_at_end` | `mm/mod.rs` | 4MB | RAM 끝 예약 영역 (DTB 등) |
| `PAGE_SIZE` | `mm/page.rs` | 4096 | 페이지 크기 |

---

## 향후 개선 사항

- [ ] **Buddy Allocator**: 현재 비트맵 방식을 Buddy 시스템으로 대체하여 단편화 감소
- [ ] **NUMA 지원**: 다중 메모리 노드 지원
- [ ] **Memory Zones**: DMA, Normal, High 영역 구분
- [ ] **Page Cache**: 파일 시스템 캐시 지원
- [ ] **Slab Allocator**: 커널 객체 캐싱

---

## 테스트 결과

```
[MM] Kernel Memory Layout:
  Kernel:      0x40080000 - 0x40087000 (28 KB)
  RAM:         0x40000000 - 0x48000000 (128 MB)
  Heap:        0x40087000 - 0x42087000 (32 MB)
  Frame Pool:  0x42087000 - 0x47c00000 (91 MB)
[Heap] Initialized: 0x40087000 - 0x42087000 (32 MB)
[PageAlloc] Initialized: 23417 pages (91 MB), bitmap uses 1 pages

[test] Box<u64> allocated: value=42, addr=0x40087000
[test] Vec<u32> allocated: len=10, capacity=10
[test] String allocated: 'Hello, kerners!', len=15
[test] Page frame allocated: 0x42088000
[test] 4 contiguous frames allocated: 0x42089000

[Heap] Stats: total=32768 KB, used=0 KB, free=32767 KB
[PageAlloc] Stats: total=23417, allocated=1, free=23416 (91 MB free)
```
