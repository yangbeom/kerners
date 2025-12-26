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

## 향후 로드맵

### 단기 목표

- 유저 ELF 바이너리 로딩 (`sys_execve` 구현)
- `sys_fork`/`sys_clone` 구현
- 커널 로그 레벨 시스템 (`error!`, `warn!`, `info!`, `debug!`, `trace!`)
- UART 인터럽트 기반 RX
- 테스트 코드를 커널 모듈로 분리

### 중기 목표

- 메모리 관리 syscall — brk, mmap, munmap, mprotect
- 프로세스 관리 syscall — clone, execve, wait4, gettid
- 시간 관련 syscall — nanosleep, clock_gettime
- 시그널 처리 — kill, rt_sigaction, rt_sigprocmask, rt_sigreturn
- 셸 명령어 유저스페이스 바이너리 분리 (ls, cat, echo 등 → `/bin/`)

### 장기 목표

- I/O 멀티플렉싱 — epoll, poll, select
- 네트워킹 — socket, bind, listen, connect (VirtIO-net)
- 공유 메모리 — POSIX shm, mmap `MAP_SHARED`, futex
- 고급 스케줄러 — CFS (vruntime 기반), EEVDF
- 고급 메모리 관리 — Buddy Allocator, Slab Allocator, Page Cache
- 드라이버 동적 로딩 — DTB 기반 ELF 드라이버 모듈
- LFN (Long File Name) 쓰기 지원
