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
| `sys_sysinfo` | 179 | `sysinfo(info)` | uptime + 메모리/프로세스 수 baseline (`mem_unit=1`) |
| `sys_set_tid_address` | 96 | `set_tid_address(ptr)` | baseline: tid 반환, clear_child_tid 미구현 |
| `sys_clock_gettime` | 113 | `clock_gettime(clockid, tp)` | `CLOCK_MONOTONIC/CLOCK_REALTIME` 분리, RTC 폴백 지원 |
| `sys_clock_getres` | 114 | `clock_getres(clockid, tp)` | 시계 해상도 반환 (`tp=NULL` 허용) |
| `sys_kill`/`sys_tkill`/`sys_tgkill` | 129/130/131 | `kill()/tkill()/tgkill()` | 대상 검증 + `sig=0` probe + pending enqueue (`SIGCONT` wake, `SIGSTOP`/`SIGCONT` pending 정리) |
| `sys_rt_sigaction` | 134 | `rt_sigaction(...)` | sighand_group 단위 액션 set/get + 전달 경로 연동 |
| `sys_rt_sigprocmask` | 135 | `rt_sigprocmask(...)` | 64-bit 시그널 마스크 추적 (`SIG_BLOCK/UNBLOCK/SETMASK`) |
| `sys_rt_sigtimedwait` | 137 | `rt_sigtimedwait(...)` | pending 매칭 즉시 소비 + timeout 대기(`EAGAIN`) + 외부 시그널 wake 시 `EINTR` |
| `sys_rt_sigreturn` | 139 | `rt_sigreturn()` | 유저 sigframe 기반 레지스터/마스크 복원 |
| `sys_setuid`/`sys_setgid` | 146/144 | `setuid()/setgid()` | baseline no-op 성공 |
| `sys_setpgid`/`sys_getpgid` | 154/155 | `setpgid()/getpgid()` | 최소 pgid 추적 |
| `sys_setsid`/`sys_getsid` | 157/156 | `setsid()/getsid()` | 최소 sid 추적 |
| `sys_uname` | 160 | `uname(buf)` | `struct utsname` 반환 (`Kerners`, arch별 machine) |
| `sys_gettimeofday` | 169 | `gettimeofday(tv, tz)` | realtime 기반 `timeval`, `timezone` zero-fill |
| `sys_nanosleep` | 101 | `nanosleep(req, rem)` | sleep queue block/wakeup + `EINTR/rem` |
| `sys_socket` | 198 | `socket(domain, type, proto)` | baseline: `EAFNOSUPPORT` |
| `sys_sendto` | 206 | `sendto(fd, buf, len, ...)` | baseline: `EBADF`/`EAFNOSUPPORT` |
| `sys_brk` | 214 | `brk(addr) -> new_brk` | vm_group별 16MB 힙 윈도우 + 페이지 단위 확장/축소 및 매핑/해제 |
| `sys_clone` | 220 | `clone(flags, ...)` | aarch64/riscv64 user-context fork/clone + vm_group/CLONE_* 리소스 그룹 + non-`CLONE_VM` COW 설정 |
| `sys_execve` | 221 | `execve(path, argv, envp)` | `ET_EXEC/ET_DYN` 실행 준비 + shebang/`PT_INTERP` 경로 + 인자/환경 경계 검증 |
| `sys_mmap` | 222 | `mmap(addr, len, prot, flags, fd, off) -> addr` | anonymous + file-backed 지원(aarch64/riscv64), `MAP_FIXED`, `MAP_SHARED`, `MAP_PRIVATE`(fault COW), `PROT_{R,W,X}` |
| `sys_munmap` | 215 | `munmap(addr, len)` | 부분/전체 unmap + 페이지 테이블 엔트리 해제 + 물리 프레임 반환 |
| `sys_mprotect` | 226 | `mprotect(addr, len, prot)` | 매핑된 페이지 권한(R/W/X) 변경 + TLB flush |
| `sys_wait4` | 260 | `wait4(pid, status, options, rusage)` | zombie 회수 + Linux wait status(`exit<<8`) + `WNOHANG` |

### 파일 I/O

| Syscall | 번호 | 시그니처 | 설명 |
|---------|------|----------|------|
| `sys_dup` | 23 | `dup(oldfd) -> newfd` | baseline FD 복제 |
| `sys_dup3` | 24 | `dup3(oldfd, newfd, flags)` | baseline (`O_CLOEXEC` no-op) |
| `sys_fcntl` | 25 | `fcntl(fd, cmd, arg)` | baseline (`F_GETFD/F_SETFD/F_GETFL/F_SETFL/F_DUPFD*`; `F_DUPFD*`는 `arg` 이상 최소 빈 FD 선택) |
| `sys_ioctl` | 29 | `ioctl(fd, req, arg)` | baseline TTY (`TCGETS/TCSETS/TIOCGWINSZ/TIOCSCTTY`) |
| `sys_statfs` | 43 | `statfs(path, buf)` | 마운트 기준 파일시스템 통계 조회 |
| `sys_faccessat` | 48 | `faccessat(dirfd, path, mode, flags)` | baseline 경로 존재 확인 |
| `sys_getcwd` | 17 | `getcwd(buf, size) -> len` | baseline 전역 cwd 반환 (NUL 포함 길이, 버퍼 부족 시 `ERANGE`) |
| `sys_openat` | 56 | `openat(dirfd, path, flags, mode) -> fd` | 파일 열기 |
| `sys_close` | 57 | `close(fd)` | 파일 닫기 |
| `sys_pipe2` | 59 | `pipe2(pipefd, flags)` | baseline 익명 파이프 생성 |
| `sys_getdents64` | 61 | `getdents64(fd, dirp, count)` | Linux `linux_dirent64` 포맷 디렉토리 엔트리 조회 |
| `sys_chdir` | 49 | `chdir(path)` | baseline 디렉토리 검증 + 전역 cwd 갱신 |
| `sys_lseek` | 62 | `lseek(fd, offset, whence) -> off` | 오프셋 이동 |
| `sys_read` | 63 | `read(fd, buf, count) -> n` | 파일 읽기 |
| `sys_write` | 64 | `write(fd, buf, count) -> n` | 파일 쓰기 |
| `sys_readlinkat` | 78 | `readlinkat(dirfd, path, buf, bufsiz)` | baseline 심볼릭 링크 대상 읽기 |
| `sys_newfstatat` | 79 | `newfstatat(dirfd, path, stat, flags)` | baseline 경로 stat |
| `sys_fstat` | 80 | `fstat(fd, statbuf)` | 파일 상태 조회 |
| `sys_mkdirat` | 34 | `mkdirat(dirfd, path, mode)` | 디렉토리 생성 |
| `sys_unlinkat` | 35 | `unlinkat(dirfd, path, flags)` | 파일/디렉토리 삭제 (`AT_REMOVEDIR` 지원) |

**참고**:
- `openat`, `mkdirat`, `unlinkat`의 `dirfd` 인자는 현재 무시됩니다.
- 상대 경로는 baseline 전역 cwd 기준으로 정규화됩니다.
- `getcwd`는 NUL 종료 문자열 길이를 반환하며, 버퍼 부족 시 `ERANGE`를 반환합니다.
- `readlinkat`는 baseline에서 `dirfd`를 무시하고 경로 기반으로 동작합니다.
- `pipe2`는 ring buffer 기반 baseline 구현이며, 블로킹/`PIPE_BUF` 원자성은 아직 범위 밖입니다.
- `fcntl(F_DUPFD/F_DUPFD_CLOEXEC)`는 Linux와 동일하게 `arg` 이상 첫 빈 FD로 복제합니다 (`F_DUPFD_CLOEXEC`의 close-on-exec 비트 자체는 아직 no-op).
- `unlinkat`는 `flags==0`(unlink)와 `flags==AT_REMOVEDIR`(rmdir)만 지원하며, 그 외 플래그는 `EINVAL`을 반환합니다.

## 파일 구조

| 파일 | 설명 |
|------|------|
| `mod.rs` | syscall 번호 상수, 디스패처, errno 모듈 |
| `fs.rs` | 파일시스템 관련 syscall 구현 (VFS 연동) |
| `process.rs` | 프로세스 관련 syscall 구현 (`execve` 전이 큐 포함) |
| `uaccess.rs` | 사용자 포인터 접근 헬퍼 (범위 검증, copy, riscv64 access-mode 전환) |

## 테스트 전용 헬퍼 (non-Linux syscall ABI)

`src/syscall/mod.rs`에는 테스트 모듈 경로에서만 사용하는 helper가 포함됩니다.
이 함수들은 유저 모드 `SVC/ECALL` ABI로 노출되지 않으며, `src/module/test_symbols.rs`를 통해서만 접근합니다.

- `enqueue_signal_for_test(signum)`:
  - 현재 tid의 pending signal 큐에 삽입
- `enqueue_signal_to_tid_for_test(tid, signum)`:
  - 지정 tid의 pending signal 큐에 삽입
  - 대상 tid 존재 여부를 검증하고, 필요 시 최소 process metadata를 생성한 뒤 enqueue

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
| `EMFILE` | -24 | 프로세스 FD 한도 초과 |
| `ENOTTY` | -25 | TTY가 아닌 디바이스 |
| `ERANGE` | -34 | 버퍼 크기 부족 |
| `EAFNOSUPPORT` | -97 | 지원하지 않는 주소 체계 |
| `ENOSYS` | -38 | 미구현 syscall |

VFS 에러는 `vfs_error_to_errno()` 함수로 자동 변환됩니다.

## 유저 포인터 접근 (`uaccess`)

- 사용자 메모리 접근은 `src/syscall/uaccess.rs`로 통일되어 있습니다.
- 공통 helper:
  - `read_unaligned` / `write_unaligned`
  - `read_byte` / `write_byte`
  - `copy_from_user` / `copy_to_user`
  - `read_c_string`
- 모든 helper는 사용자 VA 범위 + 길이 오버플로를 먼저 검증하고, 실패 시 `EFAULT`를 반환합니다.
- `riscv64`에서는 접근 단위로 `mstatus`를 `MPRV+SUM+MPP=S`로 임시 전환 후 즉시 복원합니다. 이 방식으로 syscall 중 `yield/schedule` 이후에도 사용자 포인터 접근 일관성을 유지합니다.
- `fs.rs`/`process.rs`의 주요 사용자 버퍼 경로(`read/write`, `getdents64`, `pipe2`, `wait4/waitid`, `sigaction` 등)가 동일 정책을 따릅니다.

## `execve` 동작과 제약

- 대상 실행 파일: `ET_EXEC`/`ET_DYN`
- 동적 로더 체인: `PT_INTERP`가 있으면 인터프리터 ELF를 함께 로드하고 인터프리터 엔트리로 진입
- 동작 순서:
  1. `path/argv/envp`를 유저 포인터에서 읽기
  2. 실행 경로 해석(PATH fallback) + shebang(`#!`)인 경우 인터프리터/argv 재구성
  3. (공유 vm_group일 때) `vfork(CLONE_VM)` 안전성을 위해 exec 전용 루트 테이블로 격리
  4. ELF 검증 및 `PT_LOAD` 세그먼트를 유저 가상주소로 매핑 (`ET_DYN`은 load bias 적용)
  5. `PT_DYNAMIC` 기반 `REL/RELA` baseline 재배치 적용 (`RELATIVE/GLOB_DAT/JUMP_SLOT`)
  6. 유저 스택(`argc/argv/envp/auxv`) 구성
  7. trap 복귀 시점에 `PC/SP`를 새 이미지로 전환
- 현재 제약:
  - `argv/envp`는 개수/길이 + 총량(현재 32KiB) 상한을 검사하며, 초과 시 `E2BIG`
  - aarch64/riscv64에서는 `path/argv/envp`가 유저 VA 범위인지 선검증(범위 밖은 `EFAULT`)
  - 유저 포인터 fault 복구는 제한적이며, 현재는 COW write fault 경로 중심으로만 복구
  - 동적 재배치에서 강한 외부 심볼 미해결 또는 미지원 재배치 타입은 `ENOEXEC`으로 반환
  - 약한(weak) 외부 심볼 미해결은 값 `0`으로 해석
  - auxv 최소 호환 키(`AT_ENTRY/AT_PHDR/AT_PHENT/AT_PHNUM/AT_PAGESZ/AT_BASE/AT_FLAGS`)를 스택에 제공
  - 공유 vm_group 격리 중 `prepare_exec_image`가 실패하면 기존 루트 테이블로 롤백 후 에러를 반환
  - TLS(`PT_TLS`, TLS relocation, thread pointer)는 별도 phase에서 지원 예정

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

## 시그널 기본 동작

- 기본 액션(`SIG_DFL`) 처리:
  - `SIGKILL(9)`, `SIGTERM(15)`, `SIGSEGV(11)` → 종료
  - `SIGSTOP(19)` → stop (blocked 상태 전환)
  - `SIGCONT(18)` → continue (`SIGSTOP` pending 제거 + wake)
  - `SIGCHLD(17)` → 기본 무시
- `rt_sigtimedwait`는 `SIGKILL`/`SIGSTOP` 비대기 규칙을 적용합니다.

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
