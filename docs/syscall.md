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
| `sys_sched_yield` | 124 | `sched_yield()` | CPU 양보 |
| `sys_getpid` | 172 | `getpid() -> pid` | 현재 PID 조회 |

### 파일 I/O

| Syscall | 번호 | 시그니처 | 설명 |
|---------|------|----------|------|
| `sys_openat` | 56 | `openat(dirfd, path, flags, mode) -> fd` | 파일 열기 |
| `sys_close` | 57 | `close(fd)` | 파일 닫기 |
| `sys_lseek` | 62 | `lseek(fd, offset, whence) -> off` | 오프셋 이동 |
| `sys_read` | 63 | `read(fd, buf, count) -> n` | 파일 읽기 |
| `sys_write` | 64 | `write(fd, buf, count) -> n` | 파일 쓰기 |
| `sys_fstat` | 80 | `fstat(fd, statbuf)` | 파일 상태 조회 |
| `sys_mkdirat` | 34 | `mkdirat(dirfd, path, mode)` | 디렉토리 생성 |
| `sys_unlinkat` | 35 | `unlinkat(dirfd, path, flags)` | 파일 삭제 |

**참고**: `openat`, `mkdirat`, `unlinkat`의 `dirfd` 인자는 현재 무시됩니다 (항상 절대 경로 사용).

## 파일 구조

| 파일 | 설명 |
|------|------|
| `mod.rs` | syscall 번호 상수, 디스패처, errno 모듈 |
| `fs.rs` | 파일시스템 관련 syscall 구현 (VFS 연동) |
| `process.rs` | 프로세스 관련 syscall 구현 |

## 디스패처

```rust
pub fn syscall_handler(syscall_num: usize, args: [usize; 6]) -> isize {
    match syscall_num {
        SYS_READ  => fs::sys_read(args[0], args[1] as *mut u8, args[2]),
        SYS_WRITE => fs::sys_write(args[0], args[1] as *const u8, args[2]),
        SYS_EXIT  => process::sys_exit(args[0] as i32),
        // ...
        _ => -1, // EPERM (unknown syscall)
    }
}
```

## 에러 코드 (errno)

| 상수 | 값 | 의미 |
|------|-----|------|
| `EPERM` | -1 | 권한 없음 |
| `ENOENT` | -2 | 파일/디렉토리 없음 |
| `EIO` | -5 | I/O 에러 |
| `ENOMEM` | -12 | 메모리 부족 |
| `EACCES` | -13 | 접근 거부 |
| `EFAULT` | -14 | 잘못된 주소 |
| `EBUSY` | -16 | 자원 사용 중 |
| `ENOTDIR` | -20 | 디렉토리가 아님 |
| `EISDIR` | -21 | 디렉토리임 |
| `EINVAL` | -22 | 잘못된 인자 |
| `ENOSYS` | -38 | 미구현 syscall |

VFS 에러는 `vfs_error_to_errno()` 함수로 자동 변환됩니다.

## 폴백 동작

VFS가 초기화되지 않은 경우:
- `sys_write(1|2, ...)` → 콘솔(UART)로 직접 출력
- `sys_read(0, ...)` → 콘솔(UART)에서 폴링 입력

## 새 syscall 추가 방법

1. `mod.rs`에 syscall 번호 상수 추가 (`pub const SYS_XXX: usize = N;`)
2. `fs.rs` 또는 `process.rs`에 핸들러 함수 구현
3. `syscall_handler`의 match 분기에 추가
4. 이 문서의 테이블에 추가
