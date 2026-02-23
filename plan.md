# kerners 개발 로드맵

## 프로젝트 상태 요약

| Phase | 상태 | 설명 |
|-------|------|------|
| Phase 1: 기본 인프라 | ✅ 완료 | 예외처리, DTB, 메모리관리, MMU |
| Phase 2: 디바이스/드라이버 | ✅ 완료 | Timer, GIC/PLIC, UART |
| Phase 3: 프로세스/스케줄링 | ✅ 완료 | 컨텍스트 스위칭, 스케줄러, 유저모드 |
| Phase 4: 동기화 및 IPC | ✅ 완료 | Spinlock, Mutex, RwLock, 메시지큐 |
| Phase 5: 커널 모듈 | ✅ 완료 | ELF64 로더, 심볼 테이블, PLT |
| Phase 6: 파일시스템/스토리지 | ✅ 완료 | VFS, RamFS, DevFS, FAT32, VirtIO 블록 |
| Phase 7: Multi-core (SMP) | ✅ 완료 | Per-CPU, SMP 부트, IPI, SMP-aware 스케줄러 |
| Phase 8: 테스트 인프라 | ✅ 완료 | 커널 모듈 기반 QEMU 자동 테스트, `make test` |
| Phase 9: 커널 로깅 시스템 | ✅ 완료 | 로그 레벨, 타임스탬프, CPU ID, 링 버퍼, dmesg |
| Phase 10: 프로세스 관리 강화 | ✅ 완료 | BusyBox init 부팅, clone/fork/vfork, wait/pgid/sid 계열 |
| Phase 11: 메모리 관리 syscall | ✅ 완료 | brk/mmap/munmap/mprotect, fork COW, fault COW |
| Phase 12: 시간/타이머 syscall | 🟡 대부분 완료 | core 완료, 12-A BusyBox smoke 회귀만 잔여 |
| Phase 13: 시그널 처리 | ✅ 완료 | `rt_sigtimedwait` 완성, 기본 시그널 동작, `test_signal` |
| Phase 14: 파일시스템 확장 | ✅ 완료 | `getdents64/pipe2/readlinkat/statfs`, FAT32 개선, ProcFS, `test_procfs` |

---

## 완료 리스트 정리 (2026-02-19)

### 100% 완료된 체크리스트 블록

- [x] 운영 원칙 4개 항목
- [x] Phase 10 (10-1A/B/C, 10-2, 10-3, 10-4)
- [x] Phase 11 (11-1, 11-2, 11-3)
- [x] Phase 12 core (`nanosleep`, `clock_gettime/getres`, `gettimeofday`, `test_timer`)
- [x] Phase 13 (`rt_sigtimedwait` 완성, 기본 시그널 동작, `test_signal`)
- [x] Phase 14 (`getdents64/pipe2/readlinkat/statfs`, FAT32 LFN/삭제/truncate, ProcFS, `test_procfs`)
- [x] Post-Phase 안정화 (2026-02-19, signal/thread 테스트 경합 제거 + 양 아키텍처 회귀 통과)

### 완료에 근접한 블록 (잔여만 추적)

- [ ] Phase 12-A: BusyBox smoke 회귀 (`aarch64`, `riscv64`)
- [ ] Phase 8-3: CI/CD (GitHub Actions)

---

## 운영 원칙

- [x] 코드/스크립트 수정 완료 시 `plan.md`를 같은 커밋 단위로 동기화한다.
- [x] BusyBox init 트랙(Phase 10-1)의 진행/실패 원인은 로그 파일 경로와 함께 기록한다.
- [x] `docs/` 문서는 변경 이력보다 현재 구현된 모듈 동작/제약 설명을 우선한다.
- [x] syscall 항목은 `현재 구현된 시스템 콜` 표를 단일 기준으로 유지하고, 이후 Phase는 "고도화/완성 조건"만 기록한다.

---

## 완료된 기능 요약

### Phase 1: 기본 인프라

- 예외/인터럽트 처리 — aarch64 Exception Vector Table (`VBAR_EL1`), riscv64 Trap Handler (`mtvec`/`stvec`)
- DTB 파싱 — FDT 헤더, Structure Block 순회, `/memory` 노드에서 RAM 정보 추출, 디바이스 탐색
- 메모리 관리 — 비트맵 기반 페이지 프레임 할당자, linked_list_allocator 힙, Box/Vec/String 사용 가능
- MMU — aarch64 4-level 페이지 테이블 (Identity mapping, 2MB 블록), riscv64 Sv39 (Identity + Higher-half, 2MB 메가페이지)

### Phase 2: 디바이스 및 드라이버

- Timer — aarch64 Generic Timer, riscv64 CLINT (10ms 주기 인터럽트)
- 인터럽트 컨트롤러 — GICv2 (Physical Timer IRQ 30), PLIC (UART IRQ 10)
- UART — 폴링 방식 입출력, 링 버퍼, 대화형 셸 (20+ 명령어)

### Phase 3: 프로세스/스케줄링

- 컨텍스트 스위칭 — TCB 기반, 아키텍처별 어셈블리 (레지스터 저장/복원)
- Round-robin 선점형 스케줄러 (타이머 인터럽트 기반)
- 유저 모드 — aarch64 EL0 전환, riscv64 U-mode 전환, Linux 호환 시스템 콜

### Phase 4: 동기화 및 IPC

- 동기화 프리미티브 — Spinlock, Mutex, RwLock, Semaphore, SeqLock, RCU
- IPC — MessageQueue (무제한), BoundedMessageQueue (용량 제한), Channel (Go 스타일), POSIX mq API

### Phase 5: 커널 모듈

- ELF64 relocatable 모듈 로더 (aarch64/riscv64 재배치 타입 지원)
- 심볼 테이블/익스포트 관리, 참조 카운팅, load/unload 라이프사이클
- 명령어 캐시 플러시, VFS 경로 로드, 외부 모듈 빌드 시스템 (`modules/hello`)

### Phase 6: 파일시스템 및 스토리지

- VFS — FileSystem/VNode trait, 마운트 테이블, 경로 해석, 파일 디스크립터 테이블
- 파일시스템 — RamFS, DevFS (`/dev/null`, `/dev/zero`, `/dev/console`, `/dev/vda`), FAT32 (읽기/쓰기)
- VirtIO — MMIO 서브시스템, Legacy/Modern 자동 감지, 인터럽트 기반 블록 드라이버
- 시스템 콜 — openat, close, read, write, lseek, fstat, mkdirat, unlinkat

### Phase 7: Multi-core (SMP)

- Per-CPU 인프라 — `PerCpuData` (cpu_id, current_thread_idx, idle_thread_idx, tick_count), 최대 8 CPU
- SMP 부트 — aarch64 PSCI `CPU_ON`, riscv64 SBI HSM `hart_start`
- SMP-aware 스케줄러 — CPU 친화도, per-CPU idle 스레드, per-CPU current_thread_idx
- IPI — aarch64 GIC SGI (SGI 0 = reschedule), riscv64 CLINT MSIP
- 보드 모듈 시스템 — DTB compatible 기반 런타임 보드 선택, 싱글/멀티코어 보드 설정

### Phase 8: 테스트 인프라

- 커널 모듈 기반 테스트 프레임워크, QEMU 자동 테스트
- 테스트 모듈: test_mm, test_ipc, test_block, test_vfs, test_thread

### Phase 9: 커널 로깅 시스템

- 로그 레벨 매크로 (`log_error!` ~ `log_trace!`), `kprintln!` = `log_info!`
- 타임스탬프 + CPU ID 접두사: `[  0.123456] CPU0  INFO: message`
- 64KB 정적 링 버퍼 (dmesg), Per-CPU 재귀 방지
- 런타임 로그 레벨 변경 (`loglevel` 셸 명령어)
- 모듈 심볼 `kernel_log` 익스포트, 테스트 모듈 `test_log`

---

## 현재 구현된 시스템 콜

| 번호 | 이름 | 상태 | 비고 |
|------|------|------|------|
| 17 | `getcwd` | ✅ 구현 | baseline 전역 cwd 반환 (NUL 포함 길이 반환, 버퍼 부족 시 `ERANGE`) |
| 23 | `dup` | ✅ 구현 | baseline FD 복제 |
| 24 | `dup3` | ✅ 구현 | baseline, `O_CLOEXEC` no-op |
| 25 | `fcntl` | ✅ 구현 | baseline (`F_GETFD/F_SETFD/F_GETFL/F_SETFL/F_DUPFD*`) |
| 29 | `ioctl` | ✅ 구현 | baseline TTY (`TCGETS/TCSETS/TIOCGWINSZ/TIOCSCTTY`) |
| 34 | `mkdirat` | ✅ 구현 | dirfd 무시, path만 사용 |
| 35 | `unlinkat` | ✅ 구현 | dirfd/flags 무시 |
| 43 | `statfs` | ✅ 구현 | mount 기준 `FsStats` 반환 (`procfs` magic 포함) |
| 48 | `faccessat` | ✅ 구현 | baseline 경로 존재 확인 |
| 49 | `chdir` | ✅ 구현 | baseline 디렉토리 검증 + 전역 cwd 갱신 |
| 56 | `openat` | ✅ 구현 | O_CREAT, O_TRUNC 지원 |
| 57 | `close` | ✅ 구현 | |
| 59 | `pipe2` | ✅ 구현 | baseline 익명 파이프 (링 버퍼, 블로킹/`PIPE_BUF` 보장은 Phase 16) |
| 61 | `getdents64` | ✅ 구현 | Linux `linux_dirent64` 포맷 + FD offset cursor |
| 62 | `lseek` | ✅ 구현 | SEEK_SET/CUR/END |
| 63 | `read` | ✅ 구현 | VFS + stdin 폴백 |
| 64 | `write` | ✅ 구현 | VFS + stdout/stderr 폴백 |
| 78 | `readlinkat` | ✅ 구현 | baseline: dirfd 무시, 경로 기반 symlink 대상 복사 |
| 79 | `newfstatat` | ✅ 구현 | baseline 경로 stat (dirfd/flags 제한적) |
| 80 | `fstat` | ✅ 구현 | Linux 호환 `struct stat` baseline |
| 93 | `exit` | ✅ 구현 | |
| 94 | `exit_group` | ✅ 구현 | exit으로 포워딩 |
| 95 | `waitid` | ✅ 구현 | `P_ALL/P_PID/P_PGID` + `WEXITED/WNOHANG/WNOWAIT` baseline |
| 96 | `set_tid_address` | ✅ 구현 | baseline: tid 반환, clear_child_tid 미구현 |
| 101 | `nanosleep` | ✅ 구현 | `timespec` 검증 + sleep queue block/wake + `EINTR/rem` |
| 113 | `clock_gettime` | ✅ 구현 | `CLOCK_MONOTONIC/CLOCK_REALTIME` 분리, RTC 폴백 지원 |
| 114 | `clock_getres` | ✅ 구현 | `CLOCK_MONOTONIC/CLOCK_REALTIME` 해상도 반환 (`tp=NULL` 허용) |
| 124 | `sched_yield` | ✅ 구현 | |
| 129 | `kill` | ✅ 구현 | 존재 검증 + `sig=0` probe + pending enqueue |
| 130 | `tkill` | ✅ 구현 | thread 단위 시그널 전송 |
| 131 | `tgkill` | ✅ 구현 | tgid/tid 검증 후 thread 전달 |
| 134 | `rt_sigaction` | ✅ 구현 | sighand_group 단위 액션 set/get + 핵심 플래그 저장 |
| 135 | `rt_sigprocmask` | ✅ 구현 | 프로세스별 64-bit 마스크 추적 (`SIG_BLOCK/UNBLOCK/SETMASK`) |
| 137 | `rt_sigtimedwait` | ✅ 구현 | pending 즉시 소비 + timeout 대기(`EAGAIN`) + 외부 시그널 wake 시 `EINTR` |
| 139 | `rt_sigreturn` | ✅ 구현 | sigframe 기반 컨텍스트/마스크 복원 |
| 144 | `setgid` | ✅ 구현 | baseline no-op 성공 |
| 146 | `setuid` | ✅ 구현 | baseline no-op 성공 |
| 154 | `setpgid` | ✅ 구현 | 최소 pgid 추적 갱신 |
| 155 | `getpgid` | ✅ 구현 | 추적된 pgid 반환 |
| 156 | `getsid` | ✅ 구현 | 추적된 sid 반환 |
| 157 | `setsid` | ✅ 구현 | sid/pgid를 현재 tid로 갱신 |
| 160 | `uname` | ✅ 구현 | `struct utsname` (`Kerners`, machine/domain 포함) |
| 169 | `gettimeofday` | ✅ 구현 | realtime 기반 `timeval` + `timezone` zero-fill |
| 172 | `getpid` | ✅ 구현 | tid 반환 |
| 173 | `getppid` | ✅ 구현 | 부모 PID 추적 반환 |
| 174 | `getuid` | ✅ 구현 | baseline: 0 반환 |
| 175 | `geteuid` | ✅ 구현 | baseline: 0 반환 |
| 176 | `getgid` | ✅ 구현 | baseline: 0 반환 |
| 177 | `getegid` | ✅ 구현 | baseline: 0 반환 |
| 178 | `gettid` | ✅ 구현 | tid 반환 |
| 198 | `socket` | ✅ 구현 | baseline: `EAFNOSUPPORT` |
| 206 | `sendto` | ✅ 구현 | baseline: `EBADF`/`EAFNOSUPPORT` |
| 214 | `brk` | ✅ 구현 | vm_group별 힙 영역 트래킹 + 페이지 단위 확장/축소 |
| 215 | `munmap` | ✅ 구현 | 부분/전체 unmap + shared dirty writeback flush |
| 220 | `clone` | ✅ 구현 | baseline + aarch64/riscv64 user-context + CLONE_* 리소스 그룹 추적 |
| 221 | `execve` | ✅ 구현 | static ELF(`ET_EXEC`) + argv/env 경계검증 + 확장 auxv |
| 222 | `mmap` | ✅ 구현 | anonymous + file-backed(shared/private COW) |
| 226 | `mprotect` | ✅ 구현 | 매핑 권한(R/W/X) 변경 + TLB flush |
| 260 | `wait4` | ✅ 구현 | zombie 회수 + Linux wait status + `WNOHANG` + 자식 대기/`ECHILD` |

---

## 향후 로드맵

> 모든 신규 기능은 커널 모듈 기반 테스트로 검증한다.
> 셸 내장 테스트 코드(`test_alloc`, `mqtest` 등)는 점진적으로 커널 모듈로 분리한다.

### Phase 8: 테스트 인프라 ✅

자세한 문서: [docs/testing.md](docs/testing.md)

#### 8-1. 커널 모듈 기반 테스트 프레임워크
- [x] 테스트 모듈 규약 정의 (module_init → 테스트 실행 → 0/non-zero 반환 → module_exit)
- [x] 테스트 결과 리포팅: `TEST_STATUS: PASS/FAIL` 포맷
- [x] C-compatible 커널 심볼 래퍼 (`src/module/test_symbols.rs`)
- [x] 테스트 러너 (`src/test_runner.rs`) — FAT32 자동 마운트 → 모듈 순차 로드/실행/언로드

#### 8-2. 테스트 모듈
- [x] `modules/test_mm` — 페이지/힙 할당 테스트
- [x] `modules/test_ipc` — 메시지 큐 테스트
- [x] `modules/test_block` — RamDisk 블록 읽기/쓰기 테스트
- [x] `modules/test_vfs` — VFS 파일시스템 테스트
- [x] `modules/test_thread` — 스레드 생성/yield 테스트

#### 8-3. QEMU 자동 테스트
- [x] 빌드 스크립트 (`scripts/build_test_modules.sh`)
- [x] FAT32 디스크 이미지 생성 (`scripts/prepare_test_disk.sh`)
- [x] 전체 오케스트레이션 (`scripts/run_tests.sh`)
- [x] `make test` / `make test-all` 통합
- [x] QEMU 종료 메커니즘: aarch64 semihosting, riscv64 sifive_test
- [ ] CI/CD 파이프라인 (GitHub Actions) — 추후 구현
  - [ ] aarch64 빌드 + 테스트
  - [ ] riscv64 빌드 + 테스트

### Phase 9: 커널 로깅 시스템 ✅

- [x] 로그 레벨: `log_error!`, `log_warn!`, `log_info!`, `log_debug!`, `log_trace!`
- [x] `kprintln!` → `log_info!`와 동일 동작 (기존 호출 자동 통합)
- [x] 런타임 로그 레벨 변경 (셸 명령어 `loglevel <N>`)
- [x] 타임스탬프 + CPU ID 접두사: `[  0.123456] CPU0  INFO: message`
- [x] 64KB 커널 링 버퍼 (정적 할당, SMP-safe)
- [x] `dmesg` 셸 명령어
- [x] Per-CPU 재귀 방지 (로깅 중 로깅 호출 시 deadlock 방지)
- [x] 초기화 전 fallback (log::init() 전에는 직접 UART 출력)
- [x] 모듈 심볼 `kernel_log` 익스포트
- [x] 테스트: `modules/test_log`

### Phase 10: 프로세스 관리 강화 (단기, BusyBox init 우선)

#### 10-1. BusyBox `init` 부팅 트랙 (최우선)
- [x] 목표: static BusyBox `init`를 PID 1로 실행하고 사용자 공간 초기화를 시작
- [x] 1차 범위: static ELF 우선 (`PT_INTERP` 없는 바이너리)
- [x] 1차 마일스톤: BusyBox `init` PID 1 부팅 성공

##### 10-1A. 현재 기준점(Baseline)
- [x] `sys_execve` (NR 221) 구현 (1차 baseline)
  - [x] 유저 ELF64 파서 (`ET_EXEC` 중심, `ET_DYN`은 추후)
  - [x] PT_LOAD 세그먼트 로드/매핑 (aarch64/riscv64 유저 VA 매핑 baseline)
  - [x] 유저 스택 초기화 (argc, argv, envp, auxv)
  - [x] trap 복귀 시 컨텍스트 전이로 새 엔트리 점프 (aarch64/riscv64)
- [x] BusyBox 반입 경로 준비 (`scripts/prepare_user_disk.sh`, `KERNERS_BUSYBOX`)
- [x] PID 1 후보 경로 탐색(`/sbin/init` → `/etc/init` → `/bin/init` → `/bin/sh`, `/mnt/*` fallback)
- [x] PID 1 exec 실패 시 커널 셸 fallback
- [x] `modules/test_execve` 에러 경로 검증 (`ENOENT`, `ENOEXEC`)

##### 10-1B. 완료 기준(Definition of Done)
- [x] 부팅 로그에서 PID 1 시작 확인: `launched PID1 candidate ...`
- [x] fallback 커널 셸로 떨어지지 않고 BusyBox `init` 경로 유지
- [x] `/dev/console` 기반 0/1/2 입출력으로 BusyBox 출력 확인
- [x] 동일 절차 3회 연속 부팅 성공 (aarch64 기준)
- [x] 실패 시 에러 분류 로그 보존 (`logs/busybox-init-*.log`, ENOSYS/EFAULT/기타)
- [x] 최신 실패 로그 기준점 갱신 (2026-02-14): `logs/busybox-init-aarch64-20260214-020627-run1.log`
- [x] 최신 3회 연속 스모크 성공 로그 (2026-02-14): `logs/busybox-init-aarch64-20260214-104935.summary.log`
- [x] 최신 syscall 보강 후 스모크 로그 (2026-02-14): `logs/busybox-init-aarch64-20260214-113411.summary.log`
- [x] 최신 10-1C P2 반영 후 스모크 로그 (2026-02-14): `logs/busybox-init-aarch64-20260214-114631.summary.log`
- [x] 최신 `getcwd(17)` 보강 후 스모크 로그 (2026-02-15): `logs/busybox-init-aarch64-20260215-221817.summary.log`
- [x] 최신 3회 연속 스모크 성공 로그 (2026-02-15): `logs/busybox-init-aarch64-20260215-221930.summary.log`

##### 10-1C. 우선순위 실행 계획 (Critical Path)
- [x] P0. 부팅 스모크 경로 고정 (이번 단계)
  - [x] prebuilt static BusyBox ELF 디스크 반입 스크립트
  - [x] `run.sh`의 `KERNERS_BUSYBOX` 자동 연동
  - [x] 디스크 이미지 경로 통합 (`disk.img` + `KERNERS_DISK_IMG`, run/test 공통)
  - [x] QEMU 스모크 로그 자동 수집 스크립트 (`scripts/run_busybox_smoke.sh`)
  - [x] BusyBox PID 1 실패 원인 1차 분류(ENOSYS/EFAULT/EXEC_FAIL/NO_INIT_FALLBACK)
  - [x] 스모크 분류 오탐 제거 (`EFAULT` 단어 매칭으로 `DEFAULT` 오탐 방지)
  - [x] 스모크 스크립트 안정화: run별 분리 디스크 + QEMU 락/잔존 프로세스 정리
  - [x] panic 1차 원인 수정: bootstrap stack 영역을 heap에서 제외
  - [x] 디스크 이미지 무결성 확인: `/bin/busybox`, `/bin/init`, `/bin/sh`, `/sbin/init` 존재
  - [x] ELF `PT_LOAD` 유저 가상주소(`p_vaddr`) 실제 매핑
    - [x] aarch64 런타임 유저 VA 매핑 API 추가
    - [x] `load_executable`에서 `p_vaddr -> frame` 페이지 단위 매핑 + 권한 반영
    - [x] 매핑 완료 후 BusyBox init 스모크 3회 재검증
  - [x] BusyBox PID1 후보 경로 보강: `/mnt/init` 우선 탐색 + user disk에 `/init` 엔트리 추가
- [x] P1. BusyBox init 최소 syscall 세트 구현 (의존 순서 고정)
  - [x] 1순위: `sys_getppid`, `sys_gettid`, `sys_brk`
  - [x] 2순위: `sys_mmap`, `sys_munmap` (anonymous/private 우선)
  - [x] 3순위(1차): `sys_rt_sigaction`, `sys_rt_sigprocmask` baseline stub
  - [x] 3순위(완료): `SIGCHLD` 전달(실제 signal queue 기반)
  - [x] 4순위(1차): `sys_clone`, `sys_wait4` baseline
  - [x] 4순위(완료): `sys_fork`/`sys_vfork` + 실제 프로세스 복제 의미
  - [x] 5순위(1차): `sys_setsid`, `sys_getsid`, `sys_setpgid`, `sys_getpgid` baseline stub
  - [x] 6순위(1차): `sys_ioctl`(TCGETS/TCSETS/TIOCGWINSZ/TIOCSCTTY), `sys_dup`/`sys_dup3`/`sys_fcntl`
  - [x] BusyBox 조기부팅 보강: `sys_getuid/geteuid/getgid/getegid`, `sys_set_tid_address`, `sys_chdir`, `sys_newfstatat`, `sys_faccessat`
  - [x] BusyBox 조기부팅 보강(2026-02-15): `sys_getcwd(17)` 구현으로 `Unknown syscall: 17` 제거
  - [x] ENOSYS blocker 1차 해소(2026-02-14): `113/137/169/198/206/220/260/48`
    - 기준 로그: `logs/busybox-init-aarch64-20260214-104935-run1.log`
    - 결과: 45초 스모크 구간에서 `Unknown syscall` 0건
    - 구현 전략: **호환성 우선**
      - [x] Step 1 (빠른 생존성): `clock_gettime(113)`, `gettimeofday(169)` 최소 호환 구현
      - [x] Step 2 (시그널 대기 경로): `rt_sigtimedwait(137)` 최소 호환(stub/적절 errno) 구현
      - [x] Step 3 (네트워크 호출 차단): `socket(198)`, `sendto(206)` 최소 호환(stub/적절 errno) 구현
      - [x] Step 4 (프로세스 수명주기): `clone(220)` + `wait4(260)` 최소 동작 구현
      - [x] Step 5 (회귀 확인): BusyBox init 스모크 재실행 후 ENOSYS 잔여 번호 갱신
    - 수용 기준(이번 전략 완료 기준)
      - [x] `logs/busybox-init-*.log`에서 `Unknown syscall: 113/137/169/198/206`가 제거됨
      - [x] PID 1이 즉시 종료하지 않고 다음 단계(`clone/wait4`)로 진행하는 로그가 확인됨
  - [x] 각 syscall군별 모듈 테스트 추가 (`modules/test_proc` 확장: signal/fork/vfork/wait 경로 검증)
- [x] P2. BusyBox 호환성 안정화
  - [x] exec 인자/환경 경계조건 강화 (개수/길이 상한 + fault-safe user pointer)
  - [x] auxv 최소 호환 확장 (`AT_ENTRY`, `AT_PHDR`, `AT_PHNUM`, `AT_PAGESZ`)
  - [x] `fstat` Linux 호환 구조체 baseline 정비
  - [x] exit/wait/reparenting 일관성 보강 (고아 프로세스 init 입양)
  - [x] 회귀 검증: `./scripts/run_tests.sh aarch64 60` (`RESULT: 9 passed, 0 failed`)
    - 로그: `logs/test-full-phase10-20260214-120323.log`

#### 10-2. 프로세스 생성/복제
- [x] `sys_clone` (NR 220) baseline 구현 (fake child/zombie)
- [x] `sys_clone` (NR 220) aarch64 trap-context 기반 자식 복귀 구현
- [x] `sys_clone` (NR 220) Phase 10 범위 완성
  - [x] CLONE_VM, CLONE_FS, CLONE_FILES, CLONE_SIGHAND 플래그 기반 리소스 그룹 추적
  - [x] fake child/aarch64 user-context 경로 모두 parent/pgid/sid/signal mask 동기화
  - [x] 자식 프로세스 tid 반환
  - [x] 커널 스택/페이지 테이블 복제(COW)는 Phase 11-3에서 완료
- [x] `sys_fork` — clone(SIGCHLD) wrapper
- [x] `sys_vfork` — clone(CLONE_VM | CLONE_VFORK | SIGCHLD)
- [x] 테스트: `modules/test_fork`

#### 10-3. 프로세스 종료/대기
- [x] `sys_wait4` (NR 260) baseline 구현 (fake child 회수)
- [x] `sys_wait4` (NR 260) 완성 구현
  - [x] 좀비 프로세스 상태 (최소 모델)
  - [x] Linux wait status 인코딩(`exit_code << 8`)으로 `WEXITSTATUS/WIFEXITED` 호환
  - [x] signal 종료(`WIFSIGNALED`) 완성은 Phase 13 시그널 종료 경로와 연계 (Phase 10 범위에서 defer 확정)
  - [x] WNOHANG 옵션
  - [x] 부모-자식 관계 트래킹 (ppid)
- [x] `sys_waitid` (NR 95)
- [x] exit 시 자식 프로세스 init에 입양 (reparenting)

#### 10-4. 프로세스 정보
- [x] `sys_getppid` (NR 173) — 부모 PID 추적 반환
- [x] `sys_gettid` (NR 178)
- [x] `sys_getuid` / `sys_getgid` (NR 174/176) — baseline: 0 반환
- [x] `sys_set_tid_address` (NR 96) — baseline: tid 반환
- [x] `sys_setsid` (NR 157) / `sys_getsid` (NR 156) — 최소 sid 추적
- [x] `sys_setpgid` (NR 154) / `sys_getpgid` (NR 155) — 최소 pgid 추적
- [x] `sys_uname` (NR 160) — "Kerners" 커널명 반환

### Phase 11: 메모리 관리 시스템 콜 (단기)

#### 11-1. brk / sbrk 고도화
- [x] `sys_brk` (NR 214) baseline 구현
- [x] `sys_brk` (NR 214) 고도화
  - [x] 프로세스별 program break 트래킹
  - [x] 힙 영역 확장/축소
  - [x] 페이지 단위 매핑/해제
- [x] 테스트: `modules/test_brk`

#### 11-2. mmap / munmap 고도화
- [x] `sys_mmap` (NR 222) baseline 구현
- [x] `sys_mmap` (NR 222) 고도화
  - [x] MAP_ANONYMOUS | MAP_PRIVATE — 익명 페이지 매핑
  - [x] MAP_FIXED — 지정 주소 매핑
  - [x] PROT_READ, PROT_WRITE, PROT_EXEC 페이지 권한
  - [x] 파일 backed mmap (fd + offset, aarch64/riscv64 구현)
- [x] `sys_munmap` (NR 215) baseline 구현
- [x] `sys_munmap` (NR 215) 고도화
  - [x] 페이지 테이블 엔트리 해제
  - [x] 물리 페이지 반환
- [x] `sys_mprotect` (NR 226)
  - [x] 페이지 권한 변경 (RWX)
  - [x] 페이지 테이블 업데이트 + TLB flush
- [x] 테스트: `modules/test_mmap`

#### 11-3. Copy-on-Write (COW)
- [x] 페이지 참조 카운트
- [x] fork 시 부모/자식 페이지를 read-only로 공유
- [x] 페이지 폴트 핸들러에서 COW 처리
  - [x] 새 페이지 할당 → 복사 → 쓰기 권한 부여

### Phase 12: 시간 및 타이머 시스템 콜 (단기)

- [x] `sys_nanosleep` (NR 101) 완성 구현
  - [x] `timespec(tv_sec/tv_nsec)` 파싱 및 범위 검증
  - [x] sleep queue 기반 block/wakeup (`Blocked` ↔ `Ready`)
  - [x] signal interrupt 시 `EINTR` + `rem` 기록
- [x] `sys_clock_gettime` (NR 113) 고도화 완료
  - [x] `CLOCK_REALTIME` — RTC 스냅샷 + monotonic 오프셋(폴백 포함)
  - [x] `CLOCK_MONOTONIC` — 아키텍처 타이머 카운터 기반
- [x] `sys_clock_getres` (NR 114)
- [x] `sys_gettimeofday` (NR 169) 고도화 완료 (realtime + timezone zero-fill)
- [x] 시간 코어 분리: `src/time/mod.rs` + arch RTC(`pl031`/`goldfish-rtc`)
- [x] 테스트: `modules/test_timer`

#### 12-A. DTB 기반 주소대역 동적화 (구현 완료, 회귀 검증 대기)

- [x] P0. MMU MMIO 매핑 주소 하드코딩 제거 (aarch64/riscv64 공통)
  - [x] aarch64: `src/arch/aarch64/mmu.rs`의 UART/RTC/GIC/VirtIO MMIO 매핑을 DTB/`drivers::config` 기반으로 전환
  - [x] riscv64: `src/arch/riscv64/mmu.rs`의 UART/RTC/CLINT/PLIC MMIO 매핑을 DTB/`drivers::config` 기반으로 전환
  - [x] PLIC/GIC 크기/범위는 DTB `reg` 크기 우선, 없으면 보드 폴백 유지

- [x] P1. IRQ/타이머 주파수 하드코딩 제거
  - [x] aarch64: `src/arch/aarch64/gic.rs`의 `IRQ_PHYS_TIMER`/`IRQ_UART` 상수 의존 제거, `drivers::config::{timer_irq, uart_irq}` 사용
  - [x] riscv64: `src/arch/riscv64/plic.rs`의 `IRQ_UART` 상수 의존 제거, `drivers::config::uart_irq()` 사용
  - [x] riscv64: `src/drivers/probe.rs`에서 `/cpus/timebase-frequency` 파싱 우선, 미존재 시 보드 폴백

- [x] P2. 런타임 메모리 범위 기반 검증으로 전환
  - [x] `src/fs/fat32/mod.rs`, `src/fs/fat32/fat.rs`의 `is_probably_kernel_ptr` 하드코딩 범위를 런타임 RAM/커널 매핑 범위 기반으로 교체
  - [x] `src/mm/mod.rs`의 프레임 풀 끝단 고정 4MB 예약을 DTB 실제 위치/크기 반영 방식으로 개선

- [x] P3. 부트/테스트 보조 하드코딩 정리
  - [x] `src/main.rs`의 DTB 탐색 fallback RAM 시작 상수 경로를 보드/플랫폼 설정 경로와 정합화
  - [x] `src/test_runner.rs`의 QEMU finisher 주소(0x100000) 하드코딩은 테스트 전용 정책으로 유지 여부 문서화 (또는 DTB 기반 탐색으로 전환)

- [x] 범위 제외(정책 고정)
  - [x] `USER_STACK_BASE`, `BRK/MMAP` 베이스, `KERNEL_VIRT_BASE` 등 가상주소 레이아웃 상수는 DTB 동적화 대상에서 제외

##### 테스트 케이스 및 시나리오 (해당 우선순위 구현 후 수용 기준)

- [x] `./scripts/run_tests.sh aarch64 60` 통과
- [x] `./scripts/run_tests.sh riscv64 60` 통과
- [x] 부팅 로그에서 MMIO/IRQ/timer 설정이 DTB 또는 `drivers::config` 값으로 출력되어 하드코딩 경로 미사용 확인
- [ ] 기존 BusyBox smoke 회귀 통과 (`aarch64`, `riscv64`)

##### 가정 및 기본값

- 기본 정책: **DTB 우선, BoardConfig 폴백 유지**
- DTB 미존재/파싱 실패 환경을 계속 지원
- 주소공간 정책 상수(유저 VA 레이아웃)는 이번 동적화 범위에서 제외

### Phase 13: 시그널 처리 (중기)

#### 13-1. 시그널 인프라
- [x] 프로세스별 시그널 마스크 (sigset_t, 최소 64-bit)
- [x] 시그널 핸들러 테이블 (64개 시그널)
- [x] 시그널 큐 (pending signals, 최소 FIFO)
- [x] 시그널 전달 시점: syscall 복귀 / 인터럽트 복귀

#### 13-2. 시그널 시스템 콜
- [x] `sys_kill` (NR 129) — 프로세스에 시그널 전송
- [x] `sys_tkill` (NR 130) — 스레드에 시그널 전송
- [x] `sys_tgkill` (NR 131)
- [x] `sys_rt_sigaction` (NR 134) 완성 구현 — 시그널 핸들러 등록
  - [x] SA_SIGINFO, SA_RESTART, SA_NODEFER 플래그 저장/조회
  - [x] SA_RESTART 정책 baseline 정리 (현재 interruptible syscall은 `EINTR` 유지)
- [x] `sys_rt_sigprocmask` (NR 135) 최소 구현 — 시그널 마스크 변경
- [x] `sys_rt_sigprocmask` (NR 135) 완성 구현 — 시그널 마스크 변경
  - [x] SIG_BLOCK, SIG_UNBLOCK, SIG_SETMASK
- [x] `sys_rt_sigtimedwait` (NR 137) 최소 구현 — pending queue 조회/소비 + `EAGAIN`
- [x] `sys_rt_sigtimedwait` (NR 137) 완성 구현
  - [x] `timeout` 파싱/범위 검증 + blocking wait
  - [x] waitset 매칭 wake + non-waitset signal wake 시 `EINTR`
- [x] `sys_rt_sigreturn` (NR 139) — 시그널 핸들러 복귀
  - [x] 유저 스택 sigframe 기반 컨텍스트/마스크 복원

#### 13-3. 기본 시그널 동작
- [x] SIGKILL (9) — 무조건 종료
- [x] SIGTERM (15) — 종료 요청
- [x] SIGSEGV (11) — 잘못된 메모리 접근
- [x] SIGCHLD (17) — 자식 종료 통지
- [x] SIGSTOP / SIGCONT — 프로세스 정지/재개
- [x] 테스트: `modules/test_signal`

### Phase 14: 파일시스템 확장 (중기)

#### 14-1. 추가 파일 시스템 콜
- [x] `sys_getdents64` (NR 61) — 디렉토리 엔트리 읽기
  - [x] struct linux_dirent64 포맷 호환
- [x] `sys_dup` (NR 23) / `sys_dup3` (NR 24) baseline
  - [x] FD 복제 (stdout 리다이렉션 등)
- [x] `sys_fcntl` (NR 25) baseline — FD 플래그 조작
  - [x] F_DUPFD, F_GETFD, F_SETFD, F_GETFL, F_SETFL
- [x] `sys_ioctl` (NR 29) baseline — 디바이스 제어
  - [x] TIOCGWINSZ (터미널 크기)
  - [x] TCGETS/TCSETS (터미널 속성)
  - [x] TIOCSCTTY (제어 터미널 설정)
- [x] `sys_pipe2` (NR 59) — 파이프 생성
- [x] `sys_readlinkat` (NR 78)
- [x] `sys_fstatat` (NR 79, `newfstatat`) baseline — 경로 기반 stat
- [x] `sys_statfs` (NR 43) — 파일시스템 정보
- [x] `sys_getcwd` (NR 17) — 전역 cwd baseline 반환(`ERANGE` 포함)
- [x] `sys_chdir` (NR 49) baseline

#### 14-2. fstat Linux 호환
- [x] struct stat 구조체 baseline 구현 (Linux asm-generic 호환)
  - [x] st_dev, st_ino, st_mode, st_nlink, st_uid, st_gid
  - [x] st_rdev, st_size, st_blksize, st_blocks
  - [x] st_atime, st_mtime, st_ctime (timespec, nsec=0 baseline)

#### 14-3. FAT32 개선
- [x] LFN (Long File Name) 쓰기 지원
- [x] 파일 삭제 (클러스터 체인 해제)
- [x] 디렉토리 삭제 (재귀)
- [x] 파일 크기 변경 (truncate)
- [x] 타임스탬프 업데이트

#### 14-4. ProcFS
- [x] `/proc/self/` — 현재 프로세스 정보
- [x] `/proc/[pid]/status` — 프로세스 상태
- [x] `/proc/[pid]/maps` — 메모리 매핑 정보
- [x] `/proc/meminfo` — 시스템 메모리 정보
- [x] `/proc/cpuinfo` — CPU 정보
- [x] `/proc/uptime` — 부팅 시간
- [x] 테스트: `modules/test_procfs`

### Post-Phase 안정화 (2026-02-19)

#### S-1. 테스트 회귀 안정화 (aarch64/riscv64)
- [x] `modules/test_fork`, `modules/test_proc`, `modules/test_timer`의 `rt_sigtimedwait` 폴링을 `timespec {0,0}` 기반으로 고정해 블로킹 경합 제거
- [x] `modules/test_signal` delayed worker 시그널 주입을 tid 지정 helper 경로로 전환 (`tid==0` 포함)
- [x] `modules/test_signal`의 `EINTR` 검증을 타이밍 경합 허용형(`EINTR` 또는 `EAGAIN`+즉시 drain)으로 보강
- [x] 테스트 훅 추가: `src/syscall/process.rs::test_enqueue_signal_for_tid`, `src/syscall/mod.rs::enqueue_signal_to_tid_for_test`
- [x] 테스트 심볼 추가: `kernel_test_enqueue_signal_to_tid`
- [x] `kernel_thread_spawn` 테스트 래퍼를 전역 단일 slot 방식에서 `tid` keyed pending queue 방식으로 교체해 동시 spawn 경합 제거
- [x] 회귀 검증: `make test`, `make test-riscv64` 모두 `RESULT: 14 passed, 0 failed`

### Phase 15: 유저스페이스 분리 (중기)
- [x] 2026-02-19 회귀: `make test-all` (`aarch64`/`riscv64`) PASS 유지
- [x] 기반 구현 완료: `ET_DYN` load bias, `PT_INTERP` 체인, `PATH` 탐색, shebang, auxv(`AT_PHENT`/`AT_BASE`)
- [x] 15-1 외부 Linux static ELF end-to-end 수용 테스트 완료 (2026-02-21, `aarch64`/`riscv64`)
- [ ] 잔여 검증: 15-2 static ELF 기반 유저 공간 시나리오 + 15-3 동적 ELF 런타임 링크

#### 15-1. 외부 Linux static ELF bring-up (필수 1단계)
- [x] 상태 메모: 외부 static ELF 실물 검증 완료. 검증 로그: `logs/phase15-1-aarch64-20260221-101515.log`, `logs/phase15-1-riscv64-20260221-101556.log`
- [x] 목표 바이너리 고정: 외부 툴체인 산출물(`aarch64`/`riscv64`) static ELF 2종 이상(`hello`, `busybox`)
- [x] `sys_execve` 경로로 `/bin/*` static ELF 직접 실행 검증
- [x] 실행 실패 경계조건 회귀 점검: `ENOENT`, `ENOEXEC`, `E2BIG`, `EFAULT`
- [x] user disk 배치 규칙 정리 (`scripts/prepare_user_disk.sh` 기준 `/bin`, `/sbin`, `/usr/bin`)
- [x] 수용 기준: 양 아키텍처에서 외부 static ELF 실행 + 정상 종료코드/출력 확인, 커널 panic 없음

#### 15-2. static ELF 기반 유저 공간 최소 운영 (필수 2단계)
- [ ] 상태 메모: PATH/shebang은 완료, 실제 `/bin/sh` 기반 사용자 명령 시나리오 검증이 남아 있음
- [x] PATH 탐색 최소 구현 (`/bin:/sbin:/usr/bin:/usr/sbin`)
- [x] Shebang (`#!`) 1차 지원 (인터프리터 경로 + 인자 전달, 최대 depth 제한 포함)
- [ ] `/bin/sh` baseline 동작 (파이프/리다이렉션은 현재 syscall 범위 내 최소 동작)
- [ ] 기본 유저 명령 경로 정착: `/bin/ls`, `/bin/cat`, `/bin/echo`, `/bin/mkdir`, `/bin/rm`
- [ ] `/bin/ps` — procfs 기반 프로세스 목록 조회
- [ ] 수용 기준: 셸에서 파일 생성/조회/삭제 + `/proc` 조회 + 프로세스 목록 확인 시나리오 통과

#### 15-3. 동적 ELF 로더 (필수 3단계, 추후)
- [ ] 착수 조건: 15-1/15-2 완료 + BusyBox static init 회귀 안정화
- [ ] 상태 메모: 엔트리 체인(`PT_INTERP`)과 `ET_DYN` 매핑은 완료, 동적 심볼/재배치/so 의존성 해석이 남아 있음
- [x] `PT_INTERP` 지원 (`/lib/ld-linux-*.so.*` 로더 체인, 인터프리터 엔트리 전이)
- [x] 동적 링크 ELF 실행 경로 (`ET_DYN`/PIE 포함, load bias 기반 매핑)
- [ ] `.dynamic` / `DT_*` 처리 및 런타임 링크 정보 해석
- [ ] 런타임 재배치 (`REL`/`RELA`, `JUMP_SLOT`, `GLOB_DAT`)
- [ ] 공유 라이브러리 의존성 로딩 (`/lib`, `/usr/lib`)
- [x] 최소 런타임 ABI 정비 (TLS 제외: `AT_PHENT`/`AT_BASE` 포함 auxv 확장, `PT_TLS`/thread pointer/TLS reloc는 Phase 15.5에서 구현)
- [ ] 수용 기준: 동적 링크 hello 1종 이상 + busybox dyn 경로 부팅/명령 실행

#### 15-4. (선택) 유저 ELF 빌드 환경 구축 (보조 트랙)
- [ ] Rust `no_std` + thin syscall wrapper 기반 로컬 유저 ELF 빌드 템플릿 제공
- [ ] 최소 libc 대체 또는 wrapper 라이브러리 정리 (테스트/디버그 목적)
- [ ] 샘플 유저 프로그램(`hello`, `syscall smoke`)과 빌드 스크립트 제공
- [ ] CI 회귀용으로만 사용하고, 외부 Linux ELF 호환의 선행조건으로 강제하지 않음

### Phase 15.5: TLS (Thread Local Storage) 지원 (중기, 별도 phase)

#### 15.5-1. TLS 로더/메모리 모델
- [ ] 착수 조건: Phase 15-3 완료 (동적 ELF 기본 경로 안정화)
- [ ] ELF `PT_TLS` 파싱 및 TLS 초기 이미지(`.tdata`) + zero-fill(`.tbss`) 모델 도입
- [ ] 프로세스/스레드별 TLS 블록 레이아웃 정의 (정렬/크기/모듈 오프셋 포함)
- [ ] 스레드 생성/복제(`spawn`/`clone`) 시 TLS 블록 할당/초기화 경로 추가

#### 15.5-2. 아키텍처별 thread pointer 활성화
- [ ] aarch64: `TPIDR_EL0` 설정/복원 경로 추가
- [ ] riscv64: `tp` 레지스터 설정/복원 경로 추가
- [ ] 컨텍스트 스위치 시 thread pointer 일관성 보장

#### 15.5-3. TLS 재배치 및 동적 로더 연동
- [ ] 최소 TLS 재배치 타입 지원(local-exec/initial-exec 우선)
- [ ] 미지원 TLS 재배치 타입은 명시적 실패(`ENOEXEC` 또는 `ENOTSUP`) 처리
- [ ] 동적 로딩된 `.so`의 TLS metadata 등록/해제 라이프사이클 연동

#### 15.5-4. 검증/수용 기준
- [ ] `__thread`/`thread_local` 변수 읽기/쓰기 smoke (단일 스레드)
- [ ] 멀티 스레드에서 TLS 독립성 검증 (스레드별 값 분리)
- [ ] 양 아키텍처(`aarch64`, `riscv64`) 동적 ELF + TLS 샘플 실행
- [ ] 기존 `make test`, `make test-riscv64` 회귀 PASS 유지

### Phase 16: I/O 멀티플렉싱 및 IPC 확장 (중기)

#### 16-1. I/O 멀티플렉싱
- [ ] `sys_ppoll` (NR 73) — poll with timeout
  - [ ] POLLIN, POLLOUT, POLLERR, POLLHUP 이벤트
  - [ ] 파일/파이프/소켓 대기 큐
- [ ] `sys_pselect6` (NR 72)
- [ ] `sys_epoll_create1` (NR 20) / `sys_epoll_ctl` (NR 21) / `sys_epoll_pwait` (NR 22)
  - [ ] epoll 인스턴스 (레드블랙 트리 또는 해시맵)
  - [ ] Edge-triggered / Level-triggered 모드
  - [ ] 대기 큐 연동

#### 16-2. IPC 확장
- [ ] `sys_pipe2` (NR 59) — 익명 파이프
  - [ ] 링 버퍼 기반
  - [ ] 읽기/쓰기 블로킹 (빈/꽉 찬 경우)
  - [ ] PIPE_BUF (4096) 원자적 쓰기 보장
- [ ] 공유 메모리
  - [ ] `sys_shmget` / `sys_shmat` / `sys_shmdt` (POSIX)
  - [ ] mmap MAP_SHARED 지원
- [ ] `sys_futex` (NR 98)
  - [ ] FUTEX_WAIT — 값 비교 후 대기
  - [ ] FUTEX_WAKE — 대기자 깨우기
  - [ ] 유저스페이스 뮤텍스/컨디션변수의 기반
- [ ] 테스트: `modules/test_pipe`, `modules/test_futex`

### Phase 17: 네트워킹 (장기)

#### 17-1. VirtIO-net 드라이버
- [ ] VirtIO 네트워크 디바이스 초기화
- [ ] TX/RX 큐 설정
- [ ] MAC 주소 읽기
- [ ] 패킷 송수신 (인터럽트 기반)

#### 17-2. TCP/IP 스택
- [ ] 이더넷 프레임 파싱
- [ ] ARP (Address Resolution Protocol)
- [ ] IPv4 — 패킷 송수신, ICMP (ping)
- [ ] UDP — 데이터그램 송수신
- [ ] TCP — 3-way handshake, 데이터 전송, 연결 종료
  - [ ] TCP 상태 머신 (LISTEN, SYN_SENT, ESTABLISHED, FIN_WAIT, ...)
  - [ ] 재전송 타이머, 슬라이딩 윈도우
- [ ] DHCP 클라이언트 (IP 자동 할당)

#### 17-3. 소켓 시스템 콜
- [x] `sys_socket` (NR 198) baseline 구현 (`EAFNOSUPPORT`)
- [ ] `sys_socket` (NR 198) 완성 구현 — AF_INET, SOCK_STREAM/SOCK_DGRAM
- [ ] `sys_bind` (NR 200)
- [ ] `sys_listen` (NR 201)
- [ ] `sys_accept` (NR 202)
- [ ] `sys_connect` (NR 203)
- [x] `sys_sendto` (NR 206) baseline 구현 (`EBADF`/`EAFNOSUPPORT`)
- [ ] `sys_sendto` (NR 206) / `sys_recvfrom` (NR 207) 완성 구현
- [ ] `sys_setsockopt` (NR 208) / `sys_getsockopt` (NR 209)
- [ ] `sys_shutdown` (NR 210)
- [ ] 테스트: `modules/test_net`

### Phase 18: 고급 스케줄러 (장기)

- [ ] CFS (Completely Fair Scheduler)
  - [ ] vruntime 기반 공정 스케줄링
  - [ ] 레드블랙 트리로 스레드 관리
  - [ ] nice 값 → 가중치 변환
  - [ ] 최소 granularity, 스케줄링 latency 파라미터
- [ ] EEVDF (Earliest Eligible Virtual Deadline First)
  - [ ] 가상 데드라인 기반 선택
  - [ ] lag 기반 공정성
- [ ] `sys_sched_setscheduler` (NR 119) / `sys_sched_getscheduler` (NR 120)
- [ ] `sys_sched_setaffinity` (NR 122) / `sys_sched_getaffinity` (NR 123)
- [ ] `sys_nice` (NR 정의 필요)
- [ ] 실시간 스케줄링 클래스 (SCHED_FIFO, SCHED_RR)
- [ ] 테스트: `modules/test_sched`

### Phase 19: 고급 메모리 관리 (장기)

- [ ] Buddy Allocator — O(log n) 페이지 할당, 외부 단편화 감소
- [ ] Slab Allocator — 자주 사용되는 크기의 오브젝트 캐싱
- [ ] Page Cache — 파일 I/O 캐싱
  - [ ] 읽기 캐시: VNode → 페이지 매핑
  - [ ] 쓰기 캐시: dirty 페이지 추적, writeback
  - [ ] 메모리 부족 시 LRU 기반 페이지 회수
- [ ] Demand Paging — 페이지 폴트 시 lazy 할당
- [ ] Swap — 메모리 부족 시 디스크로 페이지 스왑
  - [ ] VirtIO-blk 기반 스왑 파티션/파일
- [ ] 테스트: `modules/test_mm_advanced`

### Phase 20: 드라이버 확장 (장기)

- [ ] UART 인터럽트 기반 RX (폴링 → IRQ)
- [ ] UART TX FIFO 활용
- [ ] 셸 라인 에디팅 (화살표, Home/End, Ctrl+A/E)
- [ ] VirtIO-console
- [ ] VirtIO-gpu (프레임버퍼)
- [ ] VirtIO-input (키보드/마우스)
- [ ] RTC (Real-Time Clock) — 실제 시간
- [ ] DTB 기반 ELF 드라이버 모듈 동적 로딩

### Phase 21: 추가 파일시스템 지원 (중장기, 추후)

- [ ] 착수 조건: rootfs → 실루트 전환(switch_root) 안정화 + mount-aware 경로 생성/삭제 일관성 확보
- [ ] ext4 (우선순위 1) — read/write baseline (`create`, `unlink`, `mkdir`, `rmdir`, `truncate`)
- [ ] xfs (우선순위 2) — 대용량 파일/디렉토리 중심 baseline read/write
- [ ] btrfs (우선순위 3) — 우선 read-only + 기본 subvolume 조회, 이후 CoW write 단계 확장
- [ ] f2fs (우선순위 4) — flash-friendly 워크로드용 baseline read/write
- [ ] exFAT (우선순위 5) — 이동식 미디어 호환성 baseline read/write
- [ ] 공통 VFS 기능 점검: `statfs`, `getdents64`, `mmap(file-backed)`, page cache/writeback 연동
- [ ] 테스트/이미지 파이프라인: 파일시스템별 QEMU 디스크 이미지 생성 스크립트 + smoke 시나리오
- [ ] 수용 기준: `/` 실루트 + 하위 마운트 조합에서 BusyBox 기본 명령(생성/조회/삭제) 회귀 PASS
