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
  │     → target/modules/{arch}/test_log.ko
  │     → target/modules/{arch}/test_proc.ko
  │     → target/modules/{arch}/test_fork.ko
  │     → target/modules/{arch}/test_brk.ko
  │     → target/modules/{arch}/test_mmap.ko
  │     → target/modules/{arch}/test_timer.ko
  │     → target/modules/{arch}/test_signal.ko
  │     → target/modules/{arch}/test_procfs.ko
  │
  ├─ 2) FAT32 디스크 이미지 생성 + .ko 파일 복사
  │     → disk.img (mcopy로 .ko를 FAT32에 넣음, `KERNERS_DISK_IMG`로 경로 override 가능)
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
   → dd + mkfs.vfat/mformat → disk.img (FAT32, 32MB)
   → mcopy -i disk.img target/modules/aarch64/*.ko ::

3) cargo build --release --target aarch64-unknown-none-softfloat --features test_runner

4) qemu-system-aarch64 -machine virt -cpu cortex-a57 \
     -semihosting-config enable=on,target=native \
     -m 512M -nographic \
     -drive file=disk.img,format=raw,if=none,id=hd0 \
     -device virtio-blk-device,drive=hd0 \
     -kernel kerners.bin
```

### QEMU 내부 동작

```
커널 부팅 → VirtIO 초기화 → VFS/DevFS 마운트

[test_runner] FAT32 자동 마운트 (/dev/vda → /mnt)

=== KERNERS TEST SUITE START ===

[test] Found 14 test module(s)

[test] Loading /mnt/TEST_IPC.KO ...     (FAT32 8.3 대문자)
[test_ipc] mq create .................. PASS
[test_ipc] mq send .................... PASS
[test_ipc] mq receive ................. PASS
[test_ipc] mq receive empty ........... PASS

[test] Loading /mnt/TEST_LOG.KO ...
[test_log] all log levels ............. PASS
[test_log] rapid logging (50 msgs) .... PASS
[test_log] long message ............... PASS

[test] Loading /mnt/TEST_MM.KO ...
[test_mm] page alloc/free ............. PASS
[test_mm] heap alloc/free ............. PASS
[test_mm] multiple frames no overlap .. PASS

[test] Loading /mnt/TEST_VFS.KO ...
[test_vfs] mkdir ...................... PASS
[test_vfs] create/write/read/unlink ... PASS

[test] Loading /mnt/test_block.ko ...   (LFN 소문자)
[test_block] ramdisk create ........... PASS
[test_block] write/read/isolation ..... PASS

[test] Loading /mnt/test_thread.ko ...
[test_thread] tid/spawn/worker/yield .. PASS

[test] Loading /mnt/test_fork.ko ...
[test_fork] fork/wait4 status macros .. PASS
[test_fork] waitid(WNOWAIT) + wait4 ... PASS
[test_fork] vfork/waitid consume ...... PASS
[test_fork] uname basics .............. PASS

[test] Loading /mnt/test_proc.ko ...
[test_proc] getpid/gettid/getppid ..... PASS
[test_proc] brk grow/shrink ........... PASS
[test_proc] mmap/munmap ............... PASS

[test] Loading /mnt/test_brk.ko ...
[test_brk] grow pages ................. PASS
[test_brk] shrink + keep current ...... PASS

[test] Loading /mnt/test_mmap.ko ...
[test_mmap] anonymous map/mprotect .... PASS
[test_mmap] MAP_FIXED replace ......... PASS

[test] Loading /mnt/test_timer.ko ...
[test_timer] clock_gettime/gettimeofday PASS
[test_timer] nanosleep baseline ........ PASS

[test] Loading /mnt/test_signal.ko ...
[test_signal] rt_sigtimedwait poll ..... PASS
[test_signal] masked signal wake ....... PASS
[test_signal] EINTR path ............... PASS

[test] Loading /mnt/test_procfs.ko ...
[test_procfs] /proc + fs syscall suite . PASS

=== KERNERS TEST SUITE END ===
RESULT: 14 passed, 0 failed
TEST_STATUS: PASS

→ qemu_exit(0)
```

> **FAT32 파일명**: 8자 이하 이름(test_ipc, test_log 등)은 8.3 대문자로 저장되고,
> 9자 이상(test_block, test_thread)은 LFN으로 소문자 유지됩니다.
> 테스트 러너는 대소문자 무시 비교로 모든 모듈을 탐지합니다.

### QEMU 종료 메커니즘

| 아키텍처 | 방법 | QEMU 플래그 |
|----------|------|-------------|
| aarch64 | semihosting SYS_EXIT (`HLT #0xF000`) | `-semihosting-config enable=on,target=native` |
| riscv64 | sifive_test MMIO (0x100000에 write) | 없음 (기본 내장) |

## 빠른 시작

### 요구 사항

- Rust stable 1.93.0+ (edition 2024)
- QEMU (`qemu-system-aarch64` / `qemu-system-riscv64`)
- mtools (`mcopy`, `mformat` — FAT32 이미지 생성/조작)
  - macOS: `brew install mtools`
  - Linux: `apt install mtools`

### 실행

```bash
# aarch64 커널 테스트 (기본)
make test

# riscv64 커널 테스트
make test ARCH=riscv64

# 아키텍처별 커널 테스트 명시 실행
make test-kernel-aarch64
make test-kernel-riscv64

# 커널 모듈 테스트만 양쪽 아키텍처 실행
make test-all-kernel

# 유저 영역 동적 hello 테스트만 양쪽 아키텍처 실행
make test-user

# 전체(커널 모듈 + 유저 영역 동적 hello) 양쪽 아키텍처 실행
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

## execve / process 테스트 모듈

- `modules/test_execve`
  - 존재하지 않는 경로(`ENOENT`)
  - 비 ELF 파일 실행 시도(`ENOEXEC`)
- `modules/test_proc`
  - `getpid/gettid/getppid`
  - `brk` 증가/감소
  - `mmap/munmap` (anonymous/private + partial unmap)
  - `rt_sigprocmask/rt_sigtimedwait` (pending signal queue, `timespec {0,0}` poll 안정화 포함)
  - `fork/vfork/wait4` 최소 경로
- `modules/test_fork`
  - `fork/vfork` + `wait4/waitid` 호환 경로
  - `rt_sigtimedwait(SIGCHLD)` polling wait (`timespec {0,0}`)
  - `uname` 반환값 검증
- `modules/test_brk`
  - `brk` 페이지 단위 확장/축소
  - 잘못된 범위 요청 시 현재 break 유지
- `modules/test_mmap`
  - `mmap` + `munmap` 부분 해제
  - `mprotect` 권한 변경 호출
  - `MAP_FIXED` 덮어쓰기 매핑
- `modules/test_signal`
  - `rt_sigtimedwait` poll/timeout/blocking/EINTR
  - `SIGKILL`/`SIGSTOP` unmaskable + `rt_sigaction` 제약
  - `SIGCONT` pending wait 경로
- `modules/test_timer`
  - `clock_gettime/getres/gettimeofday/nanosleep` 경로
  - 시작 시 stale `SIGCHLD`를 `timespec {0,0}` poll로 drain

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
| empty recv | 빈 큐 non-blocking receive → 실패(-1) 확인 |

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
| spawn thread | `kernel_thread_spawn()` → tid > 0 (커널 래퍼의 tid-keyed handoff로 동시 spawn 경합 방지) |
| worker execution | 공유 변수(AtomicU32) 변경 확인 (yield 루프로 대기) |
| yield_now | `yield_now()` 호출 성공 |

### modules/test_log — 로깅 시스템

| 테스트 | 설명 |
|--------|------|
| all log levels | ERROR~TRACE 전 레벨 `kernel_log()` 호출 |
| rapid logging | 50개 메시지 연속 출력 (스트레스 테스트) |
| long message | 긴 메시지 링 버퍼 저장 확인 |

### modules/test_execve — exec 준비 경로

| 테스트 | 설명 |
|--------|------|
| missing path | 존재하지 않는 경로에 대해 `kernel_exec_prepare()`가 `ENOENT(-2)` 반환 |
| non-ELF | 일반 파일에 대해 `kernel_exec_prepare()`가 `ENOEXEC(-8)` 반환 |
| missing `DT_NEEDED` | 동적 ELF의 의존 `.so`가 없으면 `ENOENT(-2)` 반환 |
| unresolved dynamic symbol | 동적 재배치(`GLOB_DAT` 계열)에서 강한 심볼 미해결 시 `ENOEXEC(-8)` 반환 |
| resolved `DT_NEEDED` | 의존 `.so`를 `/lib`에 제공하면 동적 ELF 준비가 성공(0) |

### modules/test_proc — process syscall baseline

| 테스트 | 설명 |
|--------|------|
| getpid/gettid/getppid | PID/TID/PPID 기본 조회 검증 |
| brk grow/shrink | `brk(0)` 조회 및 증가/감소 경로 검증 |
| mmap/munmap | anonymous/private 매핑 + 읽기/쓰기 + 해제 검증 |
| file-backed mode | invalid fd에 `EBADF(-9)` 검증 |
| signal queue | `rt_sigprocmask` + `rt_sigtimedwait` pending 소비 검증 (`timespec {0,0}` poll) |
| fork/wait4 | `fork` 후 `wait4`로 자식 회수 및 상태 검증 |
| vfork/wait4 | `vfork` 후 `wait4`로 자식 회수 검증 |
| no child (`WNOHANG`) | 자식이 없을 때 `wait4(..., WNOHANG)`의 `ECHILD(-10)` 검증 |

### modules/test_fork — fork/waitid/uname

| 테스트 | 설명 |
|--------|------|
| fork/wait4 status macros | `WIFEXITED/WEXITSTATUS` 호환 wait status 인코딩 검증 |
| waitid(WNOWAIT) + wait4 | `waitid(..., WNOWAIT)` 이후 `wait4` 회수 가능 여부 검증 |
| SIGCHLD poll loop | `rt_sigtimedwait(..., timespec {0,0})` 폴링 기반으로 자식 종료 시그널 소모 검증 |
| vfork/waitid consume | `waitid`가 자식을 회수하고 재대기 시 `ECHILD` 반환 확인 |
| uname basics | `sys_uname`의 `sysname=Kerners`, machine 필드 기본값 검증 |

### modules/test_brk — brk 고도화

| 테스트 | 설명 |
|--------|------|
| grow pages | `brk` 확장 시 신규 페이지 접근 가능 여부 검증 |
| shrink + keep current | 축소 후 첫 페이지 접근 + invalid range 요청 시 현재 break 유지 확인 |
| shrink to baseline | 초기 break로 되돌리는 경로 검증 |

### modules/test_mmap — mmap/munmap/mprotect

| 테스트 | 설명 |
|--------|------|
| anonymous map + mprotect + partial munmap | 익명 매핑/권한 변경/앞쪽 부분 해제/꼬리 접근 검증 |
| MAP_FIXED replace | 같은 주소에 `MAP_FIXED`로 재매핑해 기존 매핑 교체 확인 |
| file-backed `MAP_SHARED` | 동일 파일 페이지를 다른 fd로 매핑해 변경 공유 확인 |
| file-backed `MAP_PRIVATE` + COW | write fault 후 private 변경이 shared/file에 전파되지 않음 확인 |
| shared writeback | `munmap` 후 재매핑/재읽기에서 flush 결과 반영 확인 |
| invalid args | `fd`, `offset` 정렬, `offset+len > file_size`(`EINVAL`) 검증 |
| riscv file-backed arg check | invalid fd에서 `EBADF` 반환 확인 |

### modules/test_signal — signal syscall

| 테스트 | 설명 |
|--------|------|
| rt_sigtimedwait poll/timeout | 매칭 pending 없음 + 0/유한 timeout에서 `EAGAIN` 검증 |
| masked signal wake + consume | 마스크된 `SIGTERM` pending을 `rt_sigtimedwait`로 수신 검증 |
| EINTR path | waitset 밖 시그널(`SIGCHLD`) 도착 시 `EINTR` 또는 경합 시 `EAGAIN`+즉시 drain 검증 |
| SIGCONT wait | `SIGCONT` pending을 waitset으로 수신 검증 |
| unmaskable check | `SIGKILL`/`SIGSTOP` bit가 `rt_sigprocmask`에서 적용되지 않는지 검증 |
| sigaction restrictions | `SIGKILL`/`SIGSTOP`에 대한 `rt_sigaction`이 `EINVAL`인지 검증 |

### modules/test_timer — time syscall

| 테스트 | 설명 |
|--------|------|
| `clock_gettime` | `CLOCK_MONOTONIC/CLOCK_REALTIME` 값 조회 검증 |
| `clock_getres` | 해상도 조회 및 기본 범위 검증 |
| `gettimeofday` | realtime 기반 `timeval` 반환 검증 |
| `nanosleep` | 시작 시 `SIGCHLD` poll drain 후 유효 인자 sleep + invalid 인자 에러 경로 검증 |

### modules/test_procfs — procfs + fs syscall

| 테스트 | 설명 |
|--------|------|
| `/proc/meminfo` | `MemTotal` 키 존재 확인 |
| `getdents64(/proc)` | `self`, `meminfo`, `cpuinfo`, `uptime` 엔트리 확인 |
| `/proc/self/status` | `Pid/Name` 필드 출력 확인 |
| `/proc/self/maps` | maps 읽기 경로 검증 |
| `statfs(/proc)` | procfs magic (`0x9fa0`) 확인 |
| `pipe2` | pipe read/write roundtrip 검증 |
| `readlinkat` | non-symlink 경로에 대한 `EINVAL` 경로 검증 |

## 커널 심볼 익스포트

테스트 모듈은 `extern "C"` 함수만 호출할 수 있다. 커널 내부 API를 C-compatible 래퍼로 감싸 심볼 테이블에 등록한다.

래퍼 함수는 `src/module/test_symbols.rs`에 구현되어 있다.

### 공통 (symbol.rs 등록)

| 심볼 | 시그니처 | 설명 |
|------|---------|------|
| `kernel_print` | `(s: *const u8, len: usize)` | UART 직접 출력 |
| `yield_now` | `()` | 스케줄러 양보 |
| `current_tid` | `() -> u32` | 현재 스레드 ID |
| `memset` | `(dest: *mut u8, val: i32, count: usize) -> *mut u8` | 컴파일러 intrinsic |
| `memcpy` | `(dest: *mut u8, src: *const u8, count: usize) -> *mut u8` | 컴파일러 intrinsic |
| `memmove` | `(dest: *mut u8, src: *const u8, count: usize) -> *mut u8` | 컴파일러 intrinsic |
| `memcmp` | `(a: *const u8, b: *const u8, count: usize) -> i32` | 컴파일러 intrinsic |

> `memset`/`memcpy`/`memmove`/`memcmp`는 `volatile` 연산으로 구현되어 있습니다.
> 일반 루프로 작성하면 컴파일러가 release 빌드에서 자기 자신을 호출하는 무한 재귀로 최적화합니다.

### MM (test_symbols.rs 등록)

| 심볼 | 시그니처 | 설명 |
|------|---------|------|
| `alloc_frame` | `() -> usize` | C-ABI 래퍼 (0 = 실패) |
| `free_frame` | `(addr: usize)` | 페이지 프레임 해제 |
| `kernel_heap_alloc` | `(size: usize, align: usize) -> usize` | 힙 할당 (0 = 실패) |
| `kernel_heap_dealloc` | `(ptr: usize, size: usize, align: usize)` | 힙 해제 |

> `alloc_frame`은 커널의 `mm::page::alloc_frame() -> Option<usize>`을 C-ABI 래퍼로 감쌉니다.
> `Option<usize>`는 C ABI와 호환되지 않으므로(discriminant가 반환값으로 오인됨) 반드시 래퍼를 거쳐야 합니다.

### IPC

| 심볼 | 시그니처 |
|------|---------|
| `kernel_mq_open` | `(name: *const u8, name_len: usize, create: bool) -> i32` |
| `kernel_mq_send` | `(name: *const u8, name_len: usize, data: *const u8, data_len: usize) -> i32` |
| `kernel_mq_receive` | `(name: *const u8, name_len: usize, buf: *mut u8, buf_len: usize) -> i32` (non-blocking) |

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
| `kernel_exec_prepare` | `(path: *const u8, path_len: usize) -> i32` |

### Process Syscalls

| 심볼 | 시그니처 |
|------|---------|
| `kernel_sys_getpid` | `() -> i64` |
| `kernel_sys_getppid` | `() -> i64` |
| `kernel_sys_gettid` | `() -> i64` |
| `kernel_sys_brk` | `(addr: usize) -> i64` |
| `kernel_sys_mmap` | `(addr, len, prot, flags, fd, offset) -> i64` |
| `kernel_sys_munmap` | `(addr: usize, len: usize) -> i64` |
| `kernel_sys_mprotect` | `(addr: usize, len: usize, prot: usize) -> i64` |
| `kernel_sys_open` | `(path: *const u8, flags: u32, mode: u32) -> i64` |
| `kernel_sys_close` | `(fd: i32) -> i64` |
| `kernel_sys_lseek` | `(fd: i32, offset: i64, whence: i32) -> i64` |
| `kernel_sys_read` | `(fd: i32, buf: *mut u8, count: usize) -> i64` |
| `kernel_sys_write` | `(fd: i32, buf: *const u8, count: usize) -> i64` |
| `kernel_sys_getdents64` | `(fd: i32, dirp: *mut u8, count: usize) -> i64` |
| `kernel_sys_pipe2` | `(pipefd: *mut i32, flags: u32) -> i64` |
| `kernel_sys_readlinkat` | `(dirfd: i32, path: *const u8, buf: *mut u8, bufsiz: usize) -> i64` |
| `kernel_sys_statfs` | `(path: *const u8, statfs_buf: *mut u8) -> i64` |

### Test-only Signal Hooks

| 심볼 | 시그니처 |
|------|---------|
| `kernel_test_enqueue_signal` | `(signum: u32) -> i64` |
| `kernel_test_enqueue_signal_to_tid` | `(tid: i64, signum: u32) -> i64` |

> `modules/test_signal`의 delayed worker는 `kernel_test_enqueue_signal_to_tid`를 사용해
> 지정 tid(필요 시 `tid=0` 포함)에 시그널을 안정적으로 주입합니다.

### Thread

| 심볼 | 시그니처 |
|------|---------|
| `kernel_thread_spawn` | `(entry: extern "C" fn(usize), arg: usize, name: *const u8, name_len: usize) -> i32` |
| `kernel_sleep_ticks` | `(ticks: u32)` |

### Logging

| 심볼 | 시그니처 |
|------|---------|
| `kernel_log` | `(level: u8, msg: *const u8, msg_len: usize)` |

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
| `scripts/prepare_test_disk.sh [ARCH]` | FAT32 `disk.img` 생성 + `.ko` 복사 (`KERNERS_DISK_IMG` 지원) |
| `scripts/build_user_dynamic_c_bins.sh [ARCH] [OUT_DIR]` | C 계열(`clang` + `rust-lld`) 최소 동적 ELF(`hello_dyn`, `ld-kerners-*.so`) 생성 |
| `scripts/prepare_user_disk.sh [ARCH] [BUSYBOX_PATH] [DISK_IMG]` | BusyBox 기반 `disk.img` 생성 (`/sbin/init`, `/bin/init` 포함) |
| `scripts/verify_phase15_3_cdyn.sh [ARCH] [BUSYBOX_PATH] [TIMEOUT]` | FAT32 root + rcS에서 `/bin/hello_dyn` 실행 검증 (`PH15_3_CDYN_*` 마커) |
| `scripts/run_user_tests.sh [ARCH] [TIMEOUT]` | 유저 테스트 오케스트레이션 (`verify_phase15_3_cdyn.sh` 포함, 확장 지점) |
| `scripts/run_busybox_smoke.sh [ARCH] [BUSYBOX_PATH] [RUNS] [TIMEOUT]` | BusyBox init 스모크 + `COW_FORK_TEST` 로그 판정 |
| `scripts/run_tests.sh [ARCH] [TIMEOUT]` | 전체 오케스트레이션 (빌드 → 디스크 → 커널 → QEMU → 결과 파싱) |
| `make test-kernel-aarch64` / `make test-kernel-riscv64` | 커널 테스트 트랙 아키텍처별 실행 |
| `make test-all-kernel` | 커널 모듈 테스트(`run_tests.sh`)를 aarch64/riscv64 모두 실행 |
| `make test-user` | 유저 영역 동적 hello 스모크(`verify_phase15_3_cdyn.sh`)를 aarch64/riscv64 모두 실행 |
| `make test-all` | `test-all-kernel` + `test-user`를 순차 실행 |

## 관련 소스

| 파일 | 설명 |
|------|------|
| `src/test_runner.rs` | QEMU 내 테스트 러너 (FAT32 마운트 → 모듈 로드 → 실행 → 결과 집계) |
| `src/module/test_symbols.rs` | C-compatible 커널 심볼 래퍼 함수 (56개 심볼) |
| `src/module/symbol.rs` | 커널 심볼 테이블 + 컴파일러 intrinsic (memset/memcpy/memmove) |
| `Cargo.toml` | `test_runner` feature 정의 |
