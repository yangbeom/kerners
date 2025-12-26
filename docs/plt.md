# PLT (Procedure Linkage Table) 구현

## 개요

PLT(Procedure Linkage Table)는 모듈이 커널 함수를 호출할 때 주소 범위 제한을 우회하기 위한 간접 점프 테이블입니다.

### 문제점

- **AArch64**: `BL` 명령어는 26비트 오프셋만 지원 (±128MB 범위)
- **RISC-V**: `auipc + jalr` 조합은 32비트 오프셋 지원 (±2GB 범위)

모듈이 커널 함수를 호출할 때, 모듈 코드와 커널 함수 사이의 거리가 이 범위를 초과하면 직접 호출이 불가능합니다.

### 해결책: PLT

PLT 스텁을 모듈 근처에 배치하고, 스텁이 64비트 절대 주소로 간접 점프합니다:

```
모듈 코드 ---(BL, 범위 내)--> PLT 스텁 ---(간접 점프, 64비트)--> 커널 함수
```

## 아키텍처별 PLT 스텁

### AArch64 (16바이트)

```asm
0x00: ldr x16, [pc, #8]   ; PC+8 위치에서 64비트 주소 로드
0x04: br  x16             ; x16으로 무조건 분기
0x08: .quad target        ; 64비트 타겟 주소
```

**명령어 인코딩:**
- `ldr x16, [pc, #8]` = `0x58000050`
- `br x16` = `0xd61f0200`

### RISC-V (16바이트)

```asm
0x00: auipc t3, 0         ; t3 = PC (현재 명령어 주소)
0x04: ld    t3, 8(t3)     ; t3 = [PC+8] (64비트 주소 로드)
0x08: jr    t3            ; t3로 점프
0x0c: nop                 ; 패딩 (정렬용)
0x10: .quad target        ; 64비트 타겟 주소
```

**명령어 인코딩:**
- `auipc t3, 0` = `0x00000e17`
- `ld t3, 8(t3)` = `0x008e3e03`
- `jr t3` = `0x000e0067`

## 구현 상세

### PltTable 구조체

```rust
struct PltTable {
    base: usize,                      // PLT 메모리 시작 주소
    count: usize,                     // 현재 할당된 엔트리 수
    entries: BTreeMap<usize, usize>,  // target_addr -> plt_addr 매핑
}
```

### 주요 메서드

#### `get_or_create(target: usize) -> Option<usize>`

심볼의 PLT 엔트리를 반환하거나 새로 생성합니다:

1. 이미 존재하면 캐시된 주소 반환
2. 새 엔트리 할당 및 스텁 생성
3. 매핑 테이블에 저장

#### `create_stub(plt_addr: usize, target: usize)`

아키텍처별 PLT 스텁을 메모리에 작성합니다.

### 재배치 처리 흐름

```
apply_relocations()
    ├─ R_AARCH64_CALL26 / R_AARCH64_JUMP26
    │   ├─ 오프셋 계산: (target - P) >> 2
    │   ├─ 범위 체크: ±0x2000000 (±128MB)
    │   └─ 범위 초과 시:
    │       ├─ PLT 엔트리 생성/조회
    │       └─ BL 명령을 PLT 주소로 재배치
    │
    └─ R_RISCV_CALL / R_RISCV_CALL_PLT
        ├─ 오프셋 계산: target - P
        ├─ 범위 체크: ±0x80000000 (±2GB)
        └─ 범위 초과 시:
            ├─ PLT 엔트리 생성/조회
            └─ auipc+jalr를 PLT 주소로 재배치
```

## 모듈 로드 과정

```
load_object()
    │
    ├─ 1. 메모리 크기 계산
    │      mem_size = section_memory_size()
    │      num_pages = (mem_size + PAGE_SIZE - 1) / PAGE_SIZE
    │      plt_pages = 1  (최대 256개 엔트리)
    │
    ├─ 2. 페이지 할당
    │      [코드 페이지들...][PLT 페이지]
    │      base_addr        plt_base
    │
    ├─ 3. 섹션 로드
    │      load_sections()
    │
    ├─ 4. PLT 테이블 생성
    │      plt = PltTable::new(plt_base)
    │
    ├─ 5. 재배치 적용
    │      apply_relocations(&elf, &section_addrs, &mut plt)
    │
    ├─ 6. 캐시 플러시
    │      flush_icache(base_addr, mem_size)
    │      flush_icache(plt_base, PAGE_SIZE)
    │
    └─ 7. LoadedModule 생성
           plt_page: Some(plt_base)
```

## 모듈 작성 방법

### extern 함수 선언

```rust
// 커널에서 제공하는 함수 선언
unsafe extern "C" {
    fn kernel_print(s: *const u8, len: usize);
    fn kernel_alloc(size: usize) -> *mut u8;
    // 다른 커널 함수들...
}
```

### 사용 예시

```rust
fn print(s: &str) {
    unsafe {
        kernel_print(s.as_ptr(), s.len());
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn module_init() -> i32 {
    print("[my_module] Initialized!\n");
    0
}
```

### 커널 심볼 테이블 등록

모듈에서 호출할 함수는 커널 심볼 테이블에 등록되어야 합니다:

```rust
// src/module/symbol.rs
pub fn init_symbols() {
    register_symbol("kernel_print", kernel_print as usize);
    // 다른 심볼들...
}

#[unsafe(no_mangle)]
pub extern "C" fn kernel_print(s: *const u8, len: usize) {
    // 구현...
}
```

## 용량 및 제한

| 항목 | 값 |
|------|-----|
| PLT 엔트리 크기 | 16 바이트 |
| 페이지당 최대 엔트리 | 256개 (4KB / 16) |
| 현재 할당 | 1 페이지 (4KB) |

256개 이상의 외부 심볼을 호출하는 모듈은 추가 PLT 페이지가 필요합니다.

## 디버깅

모듈 로드 시 PLT 관련 로그:

```
[module] Memory required: 1234 bytes (1 pages + 1 PLT page)
[module] Allocated 2 pages at 0x40100000
[module] PLT page at 0x40101000
[module] PLT entries created: 3
```

## 관련 파일

- [src/module/loader.rs](../src/module/loader.rs) - PLT 구현
- [src/module/symbol.rs](../src/module/symbol.rs) - 커널 심볼 테이블
- [modules/hello/src/lib.rs](../modules/hello/src/lib.rs) - 예제 모듈
