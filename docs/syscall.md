# 시스템 콜 인터페이스

`src/syscall/` — Linux 호환 시스템 콜 디스패처

## 개요

Linux AArch64/RISC-V의 `asm-generic/unistd.h` 호환 시스템 콜 번호를 사용합니다. 유저 모드에서 `SVC` (aarch64) 또는 `ECALL` (riscv64) 명령으로 커널에 진입하면 `syscall_handler`가 호출됩니다.

```
유저 모드                    커널
┌──────────┐               ┌──────────────────┐
│ SVC/ECALL │──→ 예외 핸들러 ──→ syscall_handler() ──→ 서브시스템
│ x8/a7 = 번호 │            │  ├─ fs.rs (파일 I/O)     │
│ x0-x5/a0-a5 │            │  └─ process.rs (프로세스) │
└──────────┘               └──────────────────┘
```

## 호출 규약

| | aarch64 | riscv64 |
|---|---------|---------|
| 시스템 콜 번호 | `x8` | `a7` |
| 인자 1~6 | `x0`~`x5` | `a0`~`a5` |
| 반환값 | `x0` | `a0` |
| 트랩 명령 | `SVC #0` | `ECALL` |

반환값이 음수이면 에러 코드 (negated errno).

## 구현된 시스템 콜

### 프로세스 관리

| Syscall | 번호 | 시그니처 | 설명 |
|---------|------|----------|------|
| `sys_exit` | 93 | `exit(status)` | 프로세스 종료 |
| `sys_exit_group` | 94 | `exit_group(status)` | 스레드 그룹 종료 |
| `sys_waitid` | 95 | `waitid(idtype, id, infop, options, rusage)` | `P_ALL/P_PID/P_PGID` + `WEXITED/WNOHANG/WNOWAIT` baseline |
| `sys_sched_yield` | 124 | `sched_yield()` | CPU 양보 |
| `sys_getpid` | 172 | `getpid() -> pid` | 현재 PID 조회 |
| `sys_getppid` | 173 | `getppid() -> ppid` | 부모 PID 조회 (추적 테이블 기반) |
| `sys_getuid`/`sys_geteuid` | 174/175 | `getuid()/geteuid()` | baseline: 0(root) 반환 |
| `sys_getgid`/`sys_getegid` | 176/177 | `getgid()/getegid()` | baseline: 0(root) 반환 |
| `sys_gettid` | 178 | `gettid() -> tid` | 현재 스레드 ID 조회 |
| `sys_set_tid_address` | 96 | `set_tid_address(ptr)` | baseline: tid 반환, clear_child_tid 미구현 |
| `sys_clock_gettime` | 113 | `clock_gettime(clockid, tp)` | baseline: monotonic counter 기반 |
| `sys_rt_sigaction` | 134 | `rt_sigaction(...)` | baseline stub (시그널 전달 미구현) |
| `sys_rt_sigprocmask` | 135 | `rt_sigprocmask(...)` | 64-bit 시그널 마스크 추적 (`SIG_BLOCK/UNBLOCK/SETMASK`) |
| `sys_rt_sigtimedwait` | 137 | `rt_sigtimedwait(...)` | pending signal queue에서 매칭 시그널 소비, 없으면 `EAGAIN` |
| `sys_setuid`/`sys_setgid` | 146/144 | `setuid()/setgid()` | baseline no-op 성공 |
| `sys_setpgid`/`sys_getpgid` | 154/155 | `setpgid()/getpgid()` | 최소 pgid 추적 |
| `sys_setsid`/`sys_getsid` | 157/156 | `setsid()/getsid()` | 최소 sid 추적 |
| `sys_uname` | 160 | `uname(buf)` | `struct utsname` 반환 (`Kerners`, arch별 machine) |
| `sys_gettimeofday` | 169 | `gettimeofday(tv, tz)` | baseline: monotonic wrapper |
| `sys_nanosleep` | 101 | `nanosleep(req, rem)` | baseline: yield 기반 최소 동작 |
| `sys_socket` | 198 | `socket(domain, type, proto)` | baseline: `EAFNOSUPPORT` |
| `sys_sendto` | 206 | `sendto(fd, buf, len, ...)` | baseline: `EBADF`/`EAFNOSUPPORT` |
| `sys_brk` | 214 | `brk(addr) -> new_brk` | vm_group별 16MB 힙 윈도우 + 페이지 단위 확장/축소 및 매핑/해제 |
| `sys_clone` | 220 | `clone(flags, ...)` | aarch64/riscv64 user-context fork/clone + vm_group/CLONE_* 리소스 그룹 + non-`CLONE_VM` COW 설정 |
| `sys_execve` | 221 | `execve(path, argv, envp)` | static ELF 실행 준비 + 인자/환경 경계 검증 |
| `sys_mmap` | 222 | `mmap(addr, len, prot, flags, fd, off) -> addr` | anonymous + file-backed 지원(aarch64/riscv64), `MAP_FIXED`, `MAP_SHARED`, `MAP_PRIVATE`(fault COW), `PROT_{R,W,X}` |
| `sys_munmap` | 215 | `munmap(addr, len)` | 부분/전체 unmap + 페이지 테이블 엔트리 해제 + 물리 프레임 반환 |
| `sys_mprotect` | 226 | `mprotect(addr, len, prot)` | 매핑된 페이지 권한(R/W/X) 변경 + TLB flush |
| `sys_wait4` | 260 | `wait4(pid, status, options, rusage)` | zombie 회수 + Linux wait status(`exit<<8`) + `WNOHANG` |

### 파일 I/O

| Syscall | 번호 | 시그니처 | 설명 |
|---------|------|----------|------|
| `sys_dup` | 23 | `dup(oldfd) -> newfd` | baseline FD 복제 |
| `sys_dup3` | 24 | `dup3(oldfd, newfd, flags)` | baseline (`O_CLOEXEC` no-op) |
| `sys_fcntl` | 25 | `fcntl(fd, cmd, arg)` | baseline (`F_GETFD/F_SETFD/F_GETFL/F_SETFL/F_DUPFD*`) |
| `sys_ioctl` | 29 | `ioctl(fd, req, arg)` | baseline TTY (`TCGETS/TCSETS/TIOCGWINSZ/TIOCSCTTY`) |
| `sys_faccessat` | 48 | `faccessat(dirfd, path, mode, flags)` | baseline 경로 존재 확인 |
| `sys_getcwd` | 17 | `getcwd(buf, size) -> len` | baseline 전역 cwd 반환 (NUL 포함 길이, 버퍼 부족 시 `ERANGE`) |
| `sys_openat` | 56 | `openat(dirfd, path, flags, mode) -> fd` | 파일 열기 |
| `sys_close` | 57 | `close(fd)` | 파일 닫기 |
| `sys_chdir` | 49 | `chdir(path)` | baseline 디렉토리 검증 + 전역 cwd 갱신 |
| `sys_lseek` | 62 | `lseek(fd, offset, whence) -> off` | 오프셋 이동 |
| `sys_read` | 63 | `read(fd, buf, count) -> n` | 파일 읽기 |
| `sys_write` | 64 | `write(fd, buf, count) -> n` | 파일 쓰기 |
| `sys_newfstatat` | 79 | `newfstatat(dirfd, path, stat, flags)` | baseline 경로 stat |
| `sys_fstat` | 80 | `fstat(fd, statbuf)` | 파일 상태 조회 |
| `sys_mkdirat` | 34 | `mkdirat(dirfd, path, mode)` | 디렉토리 생성 |
| `sys_unlinkat` | 35 | `unlinkat(dirfd, path, flags)` | 파일 삭제 |

**참고**:
- `openat`, `mkdirat`, `unlinkat`의 `dirfd` 인자는 현재 무시됩니다.
- 상대 경로는 baseline 전역 cwd 기준으로 정규화됩니다.
- `getcwd`는 NUL 종료 문자열 길이를 반환하며, 버퍼 부족 시 `ERANGE`를 반환합니다.

## 파일 구조

| 파일 | 설명 |
|------|------|
| `mod.rs` | syscall 번호 상수, 디스패처, errno 모듈 |
| `fs.rs` | 파일시스템 관련 syscall 구현 (VFS 연동) |
| `process.rs` | 프로세스 관련 syscall 구현 (`execve` 전이 큐 포함) |

## 디스패처

```rust
pub fn syscall_handler(syscall_num: usize, args: [usize; 6]) -> isize {
    match syscall_num {
        SYS_READ  => fs::sys_read(args[0], args[1] as *mut u8, args[2]),
        SYS_WRITE => fs::sys_write(args[0], args[1] as *const u8, args[2]),
        SYS_EXIT  => process::sys_exit(args[0] as i32),
        // ...
        _ => -38, // ENOSYS
    }
}
```

## 에러 코드 (errno)

| 상수 | 값 | 의미 |
|------|-----|------|
| `EPERM` | -1 | 권한 없음 |
| `ENOENT` | -2 | 파일/디렉토리 없음 |
| `EBADF` | -9 | 잘못된 파일 디스크립터 |
| `ECHILD` | -10 | 자식 프로세스 없음 |
| `EAGAIN` | -11 | 즉시 재시도 필요/대기 조건 불충족 |
| `EIO` | -5 | I/O 에러 |
| `ENOEXEC` | -8 | 실행 파일 형식 오류 |
| `E2BIG` | -7 | 인자/환경 크기 초과 |
| `ENOMEM` | -12 | 메모리 부족 |
| `EACCES` | -13 | 접근 거부 |
| `EFAULT` | -14 | 잘못된 주소 |
| `EBUSY` | -16 | 자원 사용 중 |
| `ENOTDIR` | -20 | 디렉토리가 아님 |
| `EISDIR` | -21 | 디렉토리임 |
| `EINVAL` | -22 | 잘못된 인자 |
| `ENOTTY` | -25 | TTY가 아닌 디바이스 |
| `ERANGE` | -34 | 버퍼 크기 부족 |
| `EAFNOSUPPORT` | -97 | 지원하지 않는 주소 체계 |
| `ENOSYS` | -38 | 미구현 syscall |

VFS 에러는 `vfs_error_to_errno()` 함수로 자동 변환됩니다.

## `execve` 동작과 제약

- 대상 실행 파일: static ELF 중심 (`ET_EXEC`)
- 동적 로더 체인: `PT_INTERP`가 있으면 `ENOEXEC` 반환
- 동작 순서:
  1. `path/argv/envp`를 유저 포인터에서 읽기
  2. ELF 검증 및 `PT_LOAD` 세그먼트를 유저 가상주소(`p_vaddr`)로 매핑
  3. 유저 스택(`argc/argv/envp/auxv`) 구성
  4. trap 복귀 시점에 `PC/SP`를 새 이미지로 전환
- 현재 제약:
  - `argv/envp`는 개수/길이 + 총량(현재 32KiB) 상한을 검사하며, 초과 시 `E2BIG`
  - aarch64/riscv64에서는 `path/argv/envp`가 유저 VA 범위인지 선검증(범위 밖은 `EFAULT`)
  - 유저 포인터 fault 복구(페이지 폴트 복귀)는 아직 미구현
  - auxv 최소 호환 키(`AT_ENTRY/AT_PHDR/AT_PHNUM/AT_PAGESZ`)를 스택에 제공
  - 동적 ELF(`PT_INTERP`, `ET_DYN`)는 아직 미지원

## `mmap`/`fork` COW 동작 (aarch64/riscv64)

- file-backed `mmap`:
  - `MAP_SHARED`: 파일 page cache 프레임을 공유 매핑합니다(다른 fd/open 경로 포함).
  - `MAP_PRIVATE`: 초기 RO 매핑 + write fault 시 COW 분리.
  - 검증: `fd < 0`은 `EBADF`, `offset` 비정렬/`offset+len > file_size`는 `EINVAL`.
- shared writeback:
  - `munmap`, `exit`, `execve` 시점에 dirty 페이지를 파일에 flush합니다.
  - `msync`는 아직 범위 밖입니다.
- fork COW:
  - non-`CLONE_VM` fork는 부모 root table 복제 후 writable private 페이지를 양쪽 RO로 강등하고 COW 메타를 등록합니다.
  - page fault에서 copy-or-promote 후 쓰기 권한을 복구합니다.

## `exit` / `wait` 최소 모델

- `sys_exit`는 부모의 zombie 리스트에 종료 상태를 등록하고 `SIGCHLD`를 큐잉합니다.
- `sys_wait4`는 zombie를 회수해 Linux 호환 wait status(`exit_code << 8`)를 기록합니다.
- `sys_waitid`는 `P_ALL/P_PID/P_PGID` + `WEXITED/WNOHANG/WNOWAIT` 조합을 지원합니다.
- 부모가 먼저 종료되면 고아 자식/좀비는 init(`pid=1`)에 재부모화(reparent)됩니다.

## 부팅 PID 1 실행 경로

- 커널 부팅 후 init 후보(`/sbin/init`, `/etc/init`, `/bin/init`, `/bin/sh`)를 순차 탐색합니다.
- RamFS 루트에서 찾지 못하면 `/mnt/*` 경로를 fallback으로 탐색합니다.
- 실행 준비는 `proc::user::prepare_exec_image()`가 담당하며,
  실제 유저 모드 진입은 init 전용 스레드에서 `eret/mret`로 수행됩니다.

## BusyBox init 스모크 테스트

- 스크립트: `scripts/run_busybox_smoke.sh`
- 출력:
  - run별 로그 `logs/busybox-init-*.log`
  - summary 로그 `logs/busybox-init-*.summary.log`
- 실패 원인 분류:
  - `ENOSYS`, `EFAULT`, `EXEC_FAIL`, `NO_INIT_FALLBACK`, `QEMU_LOCK`, `PANIC`, `TIMEOUT`, `COW_FORK_MISSING`

## 폴백 동작

VFS가 초기화되지 않은 경우:
- `sys_write(1|2, ...)` → 콘솔(UART)로 직접 출력
- `sys_read(0, ...)` → 콘솔(UART)에서 폴링 입력

## 새 syscall 추가 방법

1. `mod.rs`에 syscall 번호 상수 추가 (`pub const SYS_XXX: usize = N;`)
2. `fs.rs` 또는 `process.rs`에 핸들러 함수 구현
3. `syscall_handler`의 match 분기에 추가
4. 이 문서의 테이블에 추가
