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

---

## 현재 구현된 시스템 콜

| 번호 | 이름 | 상태 | 비고 |
|------|------|------|------|
| 34 | `mkdirat` | ✅ 구현 | dirfd 무시, path만 사용 |
| 35 | `unlinkat` | ✅ 구현 | dirfd/flags 무시 |
| 56 | `openat` | ✅ 구현 | O_CREAT, O_TRUNC 지원 |
| 57 | `close` | ✅ 구현 | |
| 62 | `lseek` | ✅ 구현 | SEEK_SET/CUR/END |
| 63 | `read` | ✅ 구현 | VFS + stdin 폴백 |
| 64 | `write` | ✅ 구현 | VFS + stdout/stderr 폴백 |
| 80 | `fstat` | ✅ 구현 | 간소화된 stat 구조체 (TODO: Linux 호환) |
| 93 | `exit` | ✅ 구현 | |
| 94 | `exit_group` | ✅ 구현 | exit으로 포워딩 |
| 101 | `nanosleep` | ⬜ 번호만 정의 | 미구현 |
| 124 | `sched_yield` | ✅ 구현 | |
| 172 | `getpid` | ✅ 구현 | tid 반환 |
| 214 | `brk` | ⬜ 번호만 정의 | 미구현 |
| 222 | `mmap` | ⬜ 번호만 정의 | 미구현 |

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

### Phase 9: 커널 로깅 시스템 (단기)

- [ ] 로그 레벨: `error!`, `warn!`, `info!`, `debug!`, `trace!`
- [ ] 런타임 로그 레벨 변경 (셸 명령어 `loglevel <N>`)
- [ ] 타임스탬프 + CPU ID 접두사: `[  0.123456] CPU0: message`
- [ ] 커널 링 버퍼 (dmesg 스타일)
- [ ] `dmesg` 셸 명령어
- [ ] 테스트: `modules/test_log`

### Phase 10: 프로세스 관리 강화 (단기)

#### 10-1. 유저 ELF 바이너리 로딩
- [ ] `sys_execve` (NR 221) 구현
  - [ ] 유저 ELF64 파서 (ET_EXEC / ET_DYN)
  - [ ] PT_LOAD 세그먼트 → 유저 주소 공간 매핑
  - [ ] 유저 스택 초기화 (argc, argv, envp, auxv)
  - [ ] 프로세스 주소 공간 교체 후 엔트리 점프
- [ ] 유저 바이너리 빌드 시스템 (cross-compile toolchain)
- [ ] `/init` 프로세스 실행 (PID 1)
- [ ] 테스트: `modules/test_execve`

#### 10-2. 프로세스 생성/복제
- [ ] `sys_clone` (NR 220) 구현
  - [ ] CLONE_VM, CLONE_FS, CLONE_FILES, CLONE_SIGHAND 플래그
  - [ ] 커널 스택 복제
  - [ ] 페이지 테이블 복제 (COW 준비)
  - [ ] 자식 프로세스 tid 반환
- [ ] `sys_fork` — clone(SIGCHLD) wrapper
- [ ] `sys_vfork` — clone(CLONE_VM | CLONE_VFORK | SIGCHLD)
- [ ] 테스트: `modules/test_fork`

#### 10-3. 프로세스 종료/대기
- [ ] `sys_wait4` (NR 260) 구현
  - [ ] 좀비 프로세스 상태 (TASK_ZOMBIE)
  - [ ] WEXITSTATUS, WIFEXITED, WIFSIGNALED 매크로 호환
  - [ ] WNOHANG 옵션
  - [ ] 부모-자식 관계 트래킹 (ppid)
- [ ] `sys_waitid` (NR 95)
- [ ] exit 시 자식 프로세스 init에 입양 (reparenting)

#### 10-4. 프로세스 정보
- [ ] `sys_getppid` (NR 173)
- [ ] `sys_gettid` (NR 178)
- [ ] `sys_getuid` / `sys_getgid` (NR 174/176) — 단순히 0 반환
- [ ] `sys_set_tid_address` (NR 96)
- [ ] `sys_uname` (NR 160) — "Kerners" 커널명 반환

### Phase 11: 메모리 관리 시스템 콜 (단기)

#### 11-1. brk / sbrk
- [ ] `sys_brk` (NR 214) 구현
  - [ ] 프로세스별 program break 트래킹
  - [ ] 힙 영역 확장/축소
  - [ ] 페이지 단위 매핑/해제
- [ ] 테스트: `modules/test_brk`

#### 11-2. mmap / munmap
- [ ] `sys_mmap` (NR 222) 구현
  - [ ] MAP_ANONYMOUS | MAP_PRIVATE — 익명 페이지 매핑
  - [ ] MAP_FIXED — 지정 주소 매핑
  - [ ] PROT_READ, PROT_WRITE, PROT_EXEC 페이지 권한
  - [ ] 파일 backed mmap (fd + offset)
- [ ] `sys_munmap` (NR 215)
  - [ ] 페이지 테이블 엔트리 해제
  - [ ] 물리 페이지 반환
- [ ] `sys_mprotect` (NR 226)
  - [ ] 페이지 권한 변경 (RWX)
  - [ ] 페이지 테이블 업데이트 + TLB flush
- [ ] 테스트: `modules/test_mmap`

#### 11-3. Copy-on-Write (COW)
- [ ] 페이지 참조 카운트
- [ ] fork 시 부모/자식 페이지를 read-only로 공유
- [ ] 페이지 폴트 핸들러에서 COW 처리
  - [ ] 새 페이지 할당 → 복사 → 쓰기 권한 부여

### Phase 12: 시간 및 타이머 시스템 콜 (단기)

- [ ] `sys_nanosleep` (NR 101) 구현
  - [ ] struct timespec {tv_sec, tv_nsec} 파싱
  - [ ] 스레드를 SLEEPING 상태로 전환
  - [ ] 타이머 만료 시 READY로 복귀
- [ ] `sys_clock_gettime` (NR 113)
  - [ ] CLOCK_REALTIME — 부팅 후 경과 시간 (에폭 타임 미지원 시 부팅 기준)
  - [ ] CLOCK_MONOTONIC — 아키텍처 타이머 카운터 기반
- [ ] `sys_clock_getres` (NR 114)
- [ ] `sys_gettimeofday` (NR 169) — clock_gettime wrapper
- [ ] 테스트: `modules/test_timer`

### Phase 13: 시그널 처리 (중기)

#### 13-1. 시그널 인프라
- [ ] 프로세스별 시그널 마스크 (sigset_t)
- [ ] 시그널 핸들러 테이블 (64개 시그널)
- [ ] 시그널 큐 (pending signals)
- [ ] 시그널 전달 시점: syscall 복귀 / 인터럽트 복귀

#### 13-2. 시그널 시스템 콜
- [ ] `sys_kill` (NR 129) — 프로세스에 시그널 전송
- [ ] `sys_tkill` (NR 130) — 스레드에 시그널 전송
- [ ] `sys_tgkill` (NR 131)
- [ ] `sys_rt_sigaction` (NR 134) — 시그널 핸들러 등록
  - [ ] SA_SIGINFO, SA_RESTART, SA_NODEFER 플래그
- [ ] `sys_rt_sigprocmask` (NR 135) — 시그널 마스크 변경
  - [ ] SIG_BLOCK, SIG_UNBLOCK, SIG_SETMASK
- [ ] `sys_rt_sigreturn` (NR 139) — 시그널 핸들러 복귀
  - [ ] 유저 스택에 저장한 컨텍스트 복원

#### 13-3. 기본 시그널 동작
- [ ] SIGKILL (9) — 무조건 종료
- [ ] SIGTERM (15) — 종료 요청
- [ ] SIGSEGV (11) — 잘못된 메모리 접근
- [ ] SIGCHLD (17) — 자식 종료 통지
- [ ] SIGSTOP / SIGCONT — 프로세스 정지/재개
- [ ] 테스트: `modules/test_signal`

### Phase 14: 파일시스템 확장 (중기)

#### 14-1. 추가 파일 시스템 콜
- [ ] `sys_getdents64` (NR 61) — 디렉토리 엔트리 읽기
  - [ ] struct linux_dirent64 포맷 호환
- [ ] `sys_dup` (NR 23) / `sys_dup3` (NR 24)
  - [ ] FD 복제 (stdout 리다이렉션 등)
- [ ] `sys_fcntl` (NR 25) — FD 플래그 조작
  - [ ] F_DUPFD, F_GETFD, F_SETFD, F_GETFL, F_SETFL
- [ ] `sys_ioctl` (NR 29) — 디바이스 제어
  - [ ] TIOCGWINSZ (터미널 크기)
  - [ ] TCGETS/TCSETS (터미널 속성)
- [ ] `sys_pipe2` (NR 59) — 파이프 생성
- [ ] `sys_readlinkat` (NR 78)
- [ ] `sys_fstatat` (NR 79) — 경로 기반 stat
- [ ] `sys_statfs` (NR 43) — 파일시스템 정보
- [ ] `sys_getcwd` (NR 17)
- [ ] `sys_chdir` (NR 49)

#### 14-2. fstat Linux 호환
- [ ] struct stat 구조체 완전 구현 (Linux asm-generic 호환)
  - [ ] st_dev, st_ino, st_mode, st_nlink, st_uid, st_gid
  - [ ] st_rdev, st_size, st_blksize, st_blocks
  - [ ] st_atime, st_mtime, st_ctime (timespec)

#### 14-3. FAT32 개선
- [ ] LFN (Long File Name) 쓰기 지원
- [ ] 파일 삭제 (클러스터 체인 해제)
- [ ] 디렉토리 삭제 (재귀)
- [ ] 파일 크기 변경 (truncate)
- [ ] 타임스탬프 업데이트

#### 14-4. ProcFS
- [ ] `/proc/self/` — 현재 프로세스 정보
- [ ] `/proc/[pid]/status` — 프로세스 상태
- [ ] `/proc/[pid]/maps` — 메모리 매핑 정보
- [ ] `/proc/meminfo` — 시스템 메모리 정보
- [ ] `/proc/cpuinfo` — CPU 정보
- [ ] `/proc/uptime` — 부팅 시간
- [ ] 테스트: `modules/test_procfs`

### Phase 15: 유저스페이스 분리 (중기)

#### 15-1. 유저 바이너리 기반 셸
- [ ] 유저 ELF 빌드 환경 구축 (Rust no_std + 커스텀 syscall wrapper)
- [ ] 최소 libc 구현 또는 thin syscall wrapper 라이브러리
- [ ] `/bin/sh` — 기본 셸 (파이프, 리다이렉션)
- [ ] `/bin/ls`, `/bin/cat`, `/bin/echo`, `/bin/mkdir`, `/bin/rm`
- [ ] `/bin/ps` — 프로세스 목록 (procfs 읽기)

#### 15-2. 프로그램 로더
- [ ] `sys_execve`로 `/bin/` 바이너리 실행
- [ ] PATH 환경변수 탐색
- [ ] Shebang (`#!`) 지원

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
- [ ] `sys_socket` (NR 198) — AF_INET, SOCK_STREAM/SOCK_DGRAM
- [ ] `sys_bind` (NR 200)
- [ ] `sys_listen` (NR 201)
- [ ] `sys_accept` (NR 202)
- [ ] `sys_connect` (NR 203)
- [ ] `sys_sendto` (NR 206) / `sys_recvfrom` (NR 207)
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
