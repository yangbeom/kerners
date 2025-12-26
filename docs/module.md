# Kernel Module System

커널 모듈 시스템 문서

## Overview

`src/module/` 모듈은 ELF64 relocatable object 파일을 동적으로 로드하고 실행하는 기능을 제공합니다.

## Architecture

```
┌─────────────────────────────────────────┐
│            Module Loader                 │
│         (module/loader.rs)               │
├──────────┬──────────────────────────────┤
│ ELF Parser│    Symbol Resolution        │
│ (elf.rs)  │     (symbol.rs)             │
├──────────┴──────────────────────────────┤
│           Memory Allocation              │
│              (mm module)                 │
└─────────────────────────────────────────┘
```

## Module Format

모듈은 ELF64 relocatable object 파일 형식 (.o)을 사용합니다.

### 필수 심볼

```rust
// 모듈 초기화 함수 (필수)
#[no_mangle]
pub extern "C" fn module_init() -> i32 {
    // 초기화 코드
    0 // 성공
}

// 모듈 정리 함수 (선택)
#[no_mangle]
pub extern "C" fn module_exit() {
    // 정리 코드
}

// 모듈 이름 (선택)
#[no_mangle]
pub static MODULE_NAME: &str = "hello";
```

## ELF64 Parser

`src/module/elf.rs`에서 ELF64 파싱 처리.

### 지원 섹션

- `.text` - 코드
- `.rodata` - 읽기 전용 데이터
- `.data` - 초기화된 데이터
- `.bss` - 미초기화 데이터
- `.symtab` - 심볼 테이블
- `.strtab` - 문자열 테이블
- `.rela.*` - 재배치 정보

### Relocation Types

**aarch64:**
- `R_AARCH64_CALL26` - 함수 호출
- `R_AARCH64_ADR_PREL_PG_HI21` - 페이지 상대 주소
- `R_AARCH64_ADD_ABS_LO12_NC` - 12비트 오프셋
- `R_AARCH64_ABS64` - 64비트 절대 주소

**riscv64:**
- `R_RISCV_CALL` - 함수 호출
- `R_RISCV_PCREL_HI20` - PC 상대 상위 20비트
- `R_RISCV_PCREL_LO12_I` - PC 상대 하위 12비트
- `R_RISCV_64` - 64비트 절대 주소

## Symbol Table

`src/module/symbol.rs`에서 커널 심볼 관리.

### 심볼 등록

```rust
use crate::module::symbol;

// 커널 함수를 모듈에 노출
symbol::register_symbol("kprintln", kprintln as usize);
symbol::register_symbol("alloc_page", mm::alloc_frame as usize);
```

### 심볼 조회

```rust
if let Some(addr) = symbol::lookup_symbol("kprintln") {
    // addr를 함수 포인터로 사용
}
```

## Module Loading

### 로딩 과정

1. **ELF 검증**: 매직 넘버, 아키텍처 확인
2. **메모리 할당**: 각 섹션에 메모리 할당
3. **섹션 로드**: 코드, 데이터 복사
4. **심볼 해석**: 외부 심볼 주소 해석
5. **재배치**: 심볼 참조 패치
6. **초기화**: `module_init()` 호출

### API

```rust
use crate::module::{ModuleLoader, ModuleInfo};

// 모듈 로더 생성
let loader = ModuleLoader::new();

// ELF 바이트에서 모듈 로드
let module = loader.load(elf_bytes)?;

// 모듈 정보 조회
let info: ModuleInfo = module.info();
println!("Module: {} at {:p}", info.name, info.base);

// 모듈 언로드
module.unload()?;
```

## Module States

```rust
pub enum ModuleState {
    Loading,    // 로딩 중
    Live,       // 실행 중
    Unloading,  // 언로드 중
    Failed,     // 로딩 실패
}
```

## Building Modules

### 모듈 소스 예시

```rust
// modules/hello/src/lib.rs
#![no_std]

extern "C" {
    fn kprintln(s: *const u8);
}

#[no_mangle]
pub extern "C" fn module_init() -> i32 {
    unsafe {
        kprintln(b"Hello from module!\0".as_ptr());
    }
    0
}

#[no_mangle]
pub extern "C" fn module_exit() {
    unsafe {
        kprintln(b"Goodbye from module!\0".as_ptr());
    }
}
```

### 빌드 명령

```bash
cd modules/hello
./build.sh aarch64  # 또는 riscv64
```

### 빌드 스크립트

```bash
#!/bin/bash
ARCH=${1:-aarch64}

if [ "$ARCH" = "aarch64" ]; then
    TARGET="aarch64-unknown-none"
else
    TARGET="riscv64gc-unknown-none-elf"
fi

cargo build --release --target $TARGET
```

## PLT (Procedure Linkage Table)

외부 함수 호출을 위한 PLT 생성. 자세한 내용은 [plt.md](plt.md) 참조.

## Error Handling

```rust
pub enum ModuleError {
    InvalidElf,         // 잘못된 ELF 형식
    UnsupportedArch,    // 지원하지 않는 아키텍처
    SymbolNotFound,     // 심볼을 찾을 수 없음
    RelocationFailed,   // 재배치 실패
    OutOfMemory,        // 메모리 부족
    InitFailed,         // 초기화 실패
}
```

## Debugging

```bash
# QEMU 셸에서
modtest              # 모듈 로더 테스트 실행
```

## Security Considerations

- 모듈은 커널 권한으로 실행됨
- 신뢰할 수 있는 모듈만 로드해야 함
- 심볼 노출은 필요한 것만
