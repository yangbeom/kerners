# Device Tree Blob (DTB) 파서

`src/dtb/mod.rs` — Flattened Device Tree (FDT) 포맷 파서

## 개요

DTB 파서는 부트로더(QEMU)가 전달하는 Device Tree Blob을 파싱하여 하드웨어 정보를 추출합니다. 외부 crate 없이 직접 구현되었으며, 메모리 영역, 인터럽트 컨트롤러, UART, 타이머 등의 디바이스 정보를 제공합니다.

```
DTB (메모리)                        커널 서브시스템
┌─────────────┐                   ┌──────────────────┐
│ FDT Header  │                   │ drivers/config   │
│ Structure   │──→ DeviceTree ──→ │ (PlatformConfig) │
│ Strings     │    파서           ├──────────────────┤
│ Memory Rsv  │                   │ boards/          │
└─────────────┘                   │ (보드 선택)      │
                                  └──────────────────┘
```

## DTB 전달 방식

- **aarch64**: 부트로더가 `x0` 레지스터에 DTB 주소 전달
- **riscv64**: 부트로더가 `a1` 레지스터에 DTB 주소 전달

## 핵심 타입

### FdtHeader

FDT 포맷 헤더 (40 bytes, big-endian):

| 필드 | 크기 | 설명 |
|------|------|------|
| `magic` | 4 | 매직 넘버 (`0xd00dfeed`) |
| `totalsize` | 4 | 전체 DTB 크기 |
| `off_dt_struct` | 4 | Structure Block 오프셋 |
| `off_dt_strings` | 4 | Strings Block 오프셋 |
| `off_mem_rsvmap` | 4 | Memory Reservation Block 오프셋 |
| `version` | 4 | DTB 버전 (16 이상 필요) |
| `last_comp_version` | 4 | 하위 호환 버전 |
| `boot_cpuid_phys` | 4 | 부트 CPU ID |
| `size_dt_strings` | 4 | Strings Block 크기 |
| `size_dt_struct` | 4 | Structure Block 크기 |

### DeviceTree

DTB 파서 메인 구조체:

```rust
pub struct DeviceTree {
    base: usize,        // DTB 시작 주소
    header: FdtHeader,  // 파싱된 헤더
}
```

### DeviceInfo

파싱된 디바이스 노드 정보:

```rust
pub struct DeviceInfo {
    pub name: String,                    // 노드 이름 (예: "uart@9000000")
    pub reg_base: u64,                   // MMIO 기본 주소
    pub reg_size: u64,                   // MMIO 크기
    pub reg_extra: Vec<(u64, u64)>,      // 추가 reg 영역 (GIC 등)
    pub interrupts: Vec<u32>,            // 인터럽트 번호들
    pub compatible: String,              // compatible 문자열
    pub clock_frequency: Option<u32>,    // 클럭 주파수
}
```

### 디바이스별 결과 타입

| 타입 | 용도 | 주요 필드 |
|------|------|-----------|
| `MemoryRegion` | RAM 영역 | base, size |
| `GicInfo` | GIC (aarch64) | distributor_base, cpu_interface_base, version |
| `PlicInfo` | PLIC (riscv64) | base, size |
| `ClintInfo` | CLINT (riscv64) | base, size |
| `UartInfo` | UART | base, size, irq, clock_freq |

## API

### 초기화

```rust
// DTB 주소로 직접 초기화
unsafe { dtb::init(dtb_addr) }

// 메모리 스캔으로 DTB 찾기 (주소 미확인 시 폴백)
unsafe { dtb::init_scan(ram_start, ram_size) }

// 전역 DTB 참조 얻기
let dt: Option<&DeviceTree> = dtb::get();
```

`init_scan`은 RAM 끝에서 2MB 전 위치(QEMU 기본 배치)를 먼저 확인하고, 실패 시 RAM 시작 512KB 범위를 4KB 단위로 스캔합니다.

### 메모리 탐색

```rust
let region = dt.get_memory()?;
// region.base: RAM 시작 주소
// region.size: RAM 크기
```

### 디바이스 탐색

```rust
// compatible 문자열로 디바이스 검색
let devices: Vec<DeviceInfo> = dt.find_compatible("virtio,mmio");

// 하드웨어별 헬퍼 함수
let gic:   Option<GicInfo>   = dt.find_gic();    // aarch64
let plic:  Option<PlicInfo>  = dt.find_plic();   // riscv64
let clint: Option<ClintInfo> = dt.find_clint();  // riscv64
let uart:  Option<UartInfo>  = dt.find_uart();

// CPU 개수
let cpus: usize = dt.count_cpus();

// 루트 compatible (보드 식별용)
let compat: Vec<String> = dt.get_root_compatible();
```

### 디버깅

```rust
dt.dump_info();    // DTB 헤더 정보 출력
dt.dump_devices(); // 전체 디바이스 노드 출력
```

## FDT Structure Block 파싱

Structure Block은 토큰 기반으로 순회합니다:

| 토큰 | 값 | 의미 |
|------|-----|------|
| `FDT_BEGIN_NODE` | 0x01 | 노드 시작 (이름 뒤따름) |
| `FDT_END_NODE` | 0x02 | 노드 종료 |
| `FDT_PROP` | 0x03 | 프로퍼티 (len, nameoff, data) |
| `FDT_NOP` | 0x04 | 무시 |
| `FDT_END` | 0x09 | Structure Block 종료 |

`#address-cells`와 `#size-cells` 프로퍼티에 따라 `reg` 프로퍼티의 주소/크기 셀 수가 결정됩니다.

## 에러 처리

```rust
pub enum DtbError {
    InvalidMagic,    // 매직 넘버 불일치
    InvalidVersion,  // DTB 버전 16 미만
    NodeNotFound,    // 요청한 노드 없음
}
```
