# 테스트 인프라

kerners는 `#![no_std]` + `#![no_main]` 베어메탈 커널이므로, 표준 `cargo test`가 동작하지 않는다.
대신 각 테스트를 독립적인 커널 모듈(`.ko`)로 만들고, QEMU에서 자동 실행/검증하는 방식을 사용한다.

## 왜 `cargo test`가 안 되는가

| 문제 | 설명 |
|------|------|
| `#![no_std]` + `#![no_main]` | 표준 테스트 하네스가 `std` + `main()` 요구 |
| `mod arch` 조건부 컴파일 | bare-metal asm, MMIO 접근 → macOS에서 빌드 불가 |
| 링커 스크립트 | `linker_aarch64.ld`로 0x40080000 로드 → macOS 바이너리 불가 |
| `IrqSpinlock` | DAIF/mstatus CSR 접근 → 유저스페이스 불가 |

## 아키텍처

```
make test
  │
  ├─ 1) 테스트 모듈 빌드 (scripts/build_test_modules.sh)
  │     → target/modules/{arch}/test_mm.ko
  │     → target/modules/{arch}/test_ipc.ko
  │     → target/modules/{arch}/test_block.ko
  │     → target/modules/{arch}/test_vfs.ko
  │     → target/modules/{arch}/test_thread.ko
  │
  ├─ 2) FAT32 디스크 이미지 생성 + .ko 파일 복사
  │     → disk_test.img (mcopy로 .ko를 FAT32에 넣음)
  │
  ├─ 3) 커널 빌드 (--features test_runner)
  │
  └─ 4) QEMU 실행 → 테스트 → 종료코드 반환
```

### 동작 과정

```
$ make test ARCH=aarch64

1) scripts/build_test_modules.sh aarch64
   → rustc로 각 테스트 모듈 빌드 → .ko 파일 생성

2) scripts/prepare_test_disk.sh aarch64
   → dd + mkfs.vfat → disk_test.img (FAT32, 32MB)
   → mcopy -i disk_test.img target/modules/aarch64/*.ko ::

3) cargo build --release --target aarch64-unknown-none-softfloat --features test_runner

4) qemu-system-aarch64 -machine virt -cpu cortex-a57 \
     -semihosting-config enable=on,target=native \
     -m 512M -nographic \
     -drive file=disk_test.img,format=raw,if=none,id=hd0 \
     -device virtio-blk-device,drive=hd0 \
     -kernel kerners.bin
```

### QEMU 내부 동작

```
커널 부팅 → VirtIO 초기화 → VFS/DevFS 마운트

[test_runner] FAT32 자동 마운트 (/dev/vda → /mnt)

=== KERNERS TEST SUITE START ===

[test] Loading /mnt/test_mm.ko ...
[test_mm] page alloc/free .............. PASS
[test_mm] heap alloc/free .............. PASS
[test] test_mm: module_init() returned 0 → OK

[test] Loading /mnt/test_ipc.ko ...
[test_ipc] message queue send/recv ..... PASS
[test] test_ipc: module_init() returned 0 → OK

... (각 테스트 모듈 순차 실행)

=== KERNERS TEST SUITE END ===
RESULT: 5 passed, 0 failed
TEST_STATUS: PASS

→ qemu_exit(0)
```

### QEMU 종료 메커니즘

| 아키텍처 | 방법 | QEMU 플래그 |
|----------|------|-------------|
| aarch64 | semihosting SYS_EXIT (`HLT #0xF000`) | `-semihosting-config enable=on,target=native` |
| riscv64 | sifive_test MMIO (0x100000에 write) | 없음 (기본 내장) |

## 빠른 시작

### 요구 사항

- Rust stable 1.93.0+ (edition 2024)
- QEMU (`qemu-system-aarch64` / `qemu-system-riscv64`)
- mtools (`mcopy` — FAT32 이미지 조작)
  - macOS: `brew install mtools`
  - Linux: `apt install mtools`

### 실행

```bash
# aarch64 테스트 (기본)
make test

# riscv64 테스트
make test ARCH=riscv64

# 양쪽 아키텍처 모두
make test-all

# 스크립트 직접 실행 (타임아웃 지정)
./scripts/run_tests.sh aarch64 60   # 60초 타임아웃
```

### 결과 판정

| stdout 패턴 | 종료 코드 | 의미 |
|-------------|-----------|------|
| `TEST_STATUS: PASS` | 0 | 전체 테스트 통과 |
| `TEST_STATUS: FAIL` | 1 | 하나 이상 실패 |
| (없음) | 2 | 타임아웃 또는 크래시 |

## 테스트 모듈

각 테스트 모듈은 `modules/hello/`와 동일한 구조의 독립 커널 모듈이다.
`module_init()`이 테스트를 실행하고, 반환값으로 결과를 알린다 (0 = pass, non-zero = fail).

### modules/test_mm — 메모리 관리

| 테스트 | 설명 |
|--------|------|
| page alloc/free | `alloc_frame()` → 유효한 주소 → `free_frame()` |
| heap alloc/free | `kernel_heap_alloc(1024, 8)` → 쓰기/읽기 검증 → `kernel_heap_dealloc()` |
| multiple frames | 여러 프레임 할당 → 주소 겹침 없음 확인 → 전부 해제 |

### modules/test_ipc — 메시지 큐

| 테스트 | 설명 |
|--------|------|
| mq create | `kernel_mq_open("test_q", create=true)` |
| mq send | 메시지 전송 → 성공 확인 |
| mq receive | 메시지 수신 → 내용 일치 확인 |
| empty recv | 빈 큐 receive → 실패(-1) 확인 |

### modules/test_block — 블록 디바이스

| 테스트 | 설명 |
|--------|------|
| ramdisk create | `kernel_ramdisk_create("test_disk", 4096)` |
| write/read | block 0에 쓰기 → 읽기 → 데이터 일치 |
| block isolation | block 1 쓰기가 block 0에 영향 없음 확인 |

### modules/test_vfs — 파일시스템

| 테스트 | 설명 |
|--------|------|
| mkdir | `kernel_vfs_mkdir("/test_vfs_dir")` |
| create file | 파일 생성 |
| write file | 데이터 쓰기 → 쓴 바이트 수 확인 |
| read file | 읽기 → 원본 데이터 일치 확인 |
| unlink | 파일 삭제 → 삭제 후 읽기 실패 확인 |

### modules/test_thread — 스레드

| 테스트 | 설명 |
|--------|------|
| current_tid | 현재 스레드 ID 조회 |
| spawn thread | `kernel_thread_spawn()` → tid > 0 |
| worker execution | 공유 변수(AtomicU32) 변경 확인 |
| yield_now | `yield_now()` 호출 성공 |

## 커널 심볼 익스포트

테스트 모듈은 `extern "C"` 함수만 호출할 수 있다. 커널 내부 API를 C-compatible 래퍼로 감싸 심볼 테이블에 등록한다.

래퍼 함수는 `src/module/test_symbols.rs`에 구현되어 있다.

### 공통

| 심볼 | 시그니처 |
|------|---------|
| `kernel_print` | `(s: *const u8, len: usize)` |
| `alloc_frame` | `() -> usize` |
| `free_frame` | `(addr: usize)` |
| `yield_now` | `()` |
| `current_tid` | `() -> u32` |

### MM

| 심볼 | 시그니처 |
|------|---------|
| `kernel_heap_alloc` | `(size: usize, align: usize) -> usize` |
| `kernel_heap_dealloc` | `(ptr: usize, size: usize, align: usize)` |

### IPC

| 심볼 | 시그니처 |
|------|---------|
| `kernel_mq_open` | `(name: *const u8, name_len: usize, create: bool) -> i32` |
| `kernel_mq_send` | `(name: *const u8, name_len: usize, data: *const u8, data_len: usize) -> i32` |
| `kernel_mq_receive` | `(name: *const u8, name_len: usize, buf: *mut u8, buf_len: usize) -> i32` |

### Block

| 심볼 | 시그니처 |
|------|---------|
| `kernel_ramdisk_create` | `(name: *const u8, name_len: usize, size: usize) -> i32` |
| `kernel_block_read` | `(name: *const u8, name_len: usize, block_idx: usize, buf: *mut u8, buf_len: usize) -> i32` |
| `kernel_block_write` | `(name: *const u8, name_len: usize, block_idx: usize, data: *const u8, data_len: usize) -> i32` |

### VFS

| 심볼 | 시그니처 |
|------|---------|
| `kernel_vfs_mkdir` | `(path: *const u8, path_len: usize) -> i32` |
| `kernel_vfs_create_file` | `(path: *const u8, path_len: usize) -> i32` |
| `kernel_vfs_write` | `(path: *const u8, path_len: usize, offset: usize, data: *const u8, data_len: usize) -> i32` |
| `kernel_vfs_read` | `(path: *const u8, path_len: usize, offset: usize, buf: *mut u8, buf_len: usize) -> i32` |
| `kernel_vfs_unlink` | `(path: *const u8, path_len: usize) -> i32` |

### Thread

| 심볼 | 시그니처 |
|------|---------|
| `kernel_thread_spawn` | `(entry: extern "C" fn(usize), arg: usize, name: *const u8, name_len: usize) -> i32` |
| `kernel_sleep_ticks` | `(ticks: u32)` |

## 새 테스트 모듈 추가하기

1. `modules/test_<name>/` 디렉토리 생성
2. `Cargo.toml` 작성 (`crate-type = ["staticlib"]`)
3. `src/lib.rs`에 `module_init()`, `module_exit()`, `module_name()`, `module_version()` 구현
4. `module_init()`에서 테스트 실행, 0(pass) / non-zero(fail) 반환
5. 필요한 커널 심볼은 `extern "C"` 블록에 선언
6. 새 커널 심볼이 필요하면 `src/module/test_symbols.rs`에 래퍼 추가 + `register_test_symbols()`에 등록

### 모듈 템플릿

```rust
#![no_std]
#![no_main]

use core::panic::PanicInfo;

unsafe extern "C" {
    fn kernel_print(s: *const u8, len: usize);
    // 필요한 심볼 추가
}

fn print(s: &str) {
    unsafe { kernel_print(s.as_ptr(), s.len()); }
}

#[unsafe(no_mangle)]
pub extern "C" fn module_init() -> i32 {
    print("[test_xxx] === My Tests ===\n");

    // 테스트 1
    print("[test_xxx] test: something ... ");
    // ... 테스트 로직 ...
    print("PASS\n");

    print("[test_xxx] All tests passed\n");
    0 // 성공
}

#[unsafe(no_mangle)]
pub extern "C" fn module_exit() {
    print("[test_xxx] Module unloaded\n");
}

#[unsafe(no_mangle)]
pub extern "C" fn module_name() -> *const u8 {
    b"test_xxx\0".as_ptr()
}

#[unsafe(no_mangle)]
pub extern "C" fn module_version() -> *const u8 {
    b"0.1.0\0".as_ptr()
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    print("[test_xxx] PANIC!\n");
    loop {}
}
```

## 스크립트

| 스크립트 | 설명 |
|----------|------|
| `scripts/build_test_modules.sh [ARCH]` | `modules/test_*/`를 순회하여 `.ko` 빌드 |
| `scripts/prepare_test_disk.sh [ARCH]` | FAT32 `disk_test.img` 생성 + `.ko` 복사 |
| `scripts/run_tests.sh [ARCH] [TIMEOUT]` | 전체 오케스트레이션 (빌드 → 디스크 → 커널 → QEMU → 결과 파싱) |

## 관련 소스

| 파일 | 설명 |
|------|------|
| `src/test_runner.rs` | QEMU 내 테스트 러너 (FAT32 마운트 → 모듈 로드 → 실행 → 결과 집계) |
| `src/module/test_symbols.rs` | C-compatible 커널 심볼 래퍼 함수 |
| `Cargo.toml` | `test_runner` feature 정의 |
