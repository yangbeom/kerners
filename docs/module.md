# Kernel Module System

커널 모듈 시스템(`src/module/`)은 ELF64 기반 로딩 기능을 제공합니다.

## 개요

```
src/module/
├── elf.rs      # ELF64 파서
├── loader.rs   # 모듈/실행파일 로더
├── symbol.rs   # 커널 심볼 테이블
└── test_symbols.rs
```

현재 모듈 시스템은 두 가지 로딩 경로를 제공합니다.

- 로더블 커널 모듈 로드: ELF64 relocatable(`ET_REL`)
- 사용자 실행 파일 준비용 로드: ELF64 executable(`ET_EXEC` 중심)

## 모듈 형식

### 1) 로더블 커널 모듈 (`ET_REL`)

- 입력: `.ko`/`.o` (ELF64 relocatable)
- 섹션 로드 + 재배치 + `module_init()` 호출
- 필요 시 `module_exit()` 호출 후 언로드

권장 심볼 형식:

```rust
#[unsafe(no_mangle)]
pub extern "C" fn module_init() -> i32 {
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn module_exit() {}

#[unsafe(no_mangle)]
pub extern "C" fn module_name() -> *const u8 {
    b"my_module\0".as_ptr()
}

#[unsafe(no_mangle)]
pub extern "C" fn module_version() -> *const u8 {
    b"0.1.0\0".as_ptr()
}
```

### 2) 실행 파일 로드 (`execve` 경로)

- 입력: ELF64 executable (`ET_EXEC`/`ET_DYN` 헤더 파싱)
- `PT_LOAD` 세그먼트를 읽어 엔트리 포인트를 반환
- `PT_DYNAMIC`/`DT_*`를 파싱해 런타임 링크 메타데이터를 수집한다
- `proc::user::prepare_exec_image()`에서 호출됨

현재 제약:

- `PT_LOAD`의 `p_vaddr`를 유저 페이지 테이블로 매핑한다.
- `ET_DYN`은 실행 시 load bias를 적용해 유효 사용자 주소 범위로 이동 매핑한다.
- `.dynamic` 처리의 현재 범위는 `DT_*` 메타데이터 수집까지이며,
  실제 `REL/RELA` 적용 및 `DT_NEEDED` 라이브러리 해석은 후속 단계다.
- 가드 범위를 벗어난 세그먼트는 로드 실패한다.
  - aarch64: `0x0010_0000..0x0800_0000`
  - riscv64: `0x4000_0000..0x8000_0000`

## 로딩/언로딩 흐름

### 로더블 모듈 로드

1. ELF 파싱 (`ET_REL` 확인)
2. 메모리 페이지 할당(섹션 + PLT)
3. 섹션 복사 및 재배치 적용
4. export 심볼 등록
5. `module_init()` 호출
6. 성공 시 `LOADED_MODULES`에 등록

### 언로드

1. 모듈 언로딩 플래그 설정
2. 참조 카운트 확인
3. `module_exit()` 호출
4. 할당 페이지 해제 및 목록에서 제거

## 주요 API

| API | 설명 |
|-----|------|
| `ModuleLoader::load_from_path(path)` | VFS 경로에서 모듈 파일을 읽어 로드 |
| `ModuleLoader::load_object(data, name)` | 메모리 버퍼의 `ET_REL` 모듈 로드 |
| `ModuleLoader::unload(name)` | 모듈 언로드 |
| `ModuleLoader::unload_wait(name, max_wait_ms)` | 참조 해제 대기 후 언로드 |
| `ModuleLoader::list()` | 로드된 모듈 이름 목록 |
| `ModuleLoader::info(name)` | 모듈 상세 정보 조회 |
| `ModuleLoader::acquire(name)` | 모듈 참조 가드(`ModuleRef`) 획득 |
| `ModuleLoader::lookup_symbol_in(module, symbol)` | 특정 모듈에서 심볼 조회 |
| `ModuleLoader::lookup_symbol_global(symbol)` | 커널 + 모듈 전체에서 심볼 조회 |
| `ModuleLoader::list_module_symbols(module)` | 모듈 export 심볼 목록 조회 |
| `ModuleLoader::export_symbol(module, symbol, addr)` | 런타임 export 심볼 추가 |
| `ModuleLoader::load_executable(data)` | 실행 ELF 로드 후 `entry/load_bias/.dynamic 요약` 반환 |

## 심볼/재배치/PLT

- 커널 심볼은 `symbol.rs`에서 관리합니다.
- 외부 함수 호출 재배치를 위해 PLT 페이지를 생성합니다.
- 아키텍처별 재배치 타입(`aarch64`, `riscv64`)을 처리합니다.

## 상태 및 에러

### ModuleState

- `Loading`
- `Live`
- `Unloading`

### ModuleError (주요)

- `ElfError(...)`
- `InvalidFormat`
- `OutOfMemory`
- `SymbolNotFound`
- `UnsupportedRelocation(...)`
- `InitFailed(...)`
- `InUse`
- `AlreadyLoaded`
- `NotFound`
- `ModuleUnloading`

## 디버깅

QEMU 셸에서:

```bash
modtest
```

- 모듈 로드/언로드 경로를 점검할 수 있습니다.
