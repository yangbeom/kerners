# kerners

> [English](README-en.md) | 한국어

Rust로 작성된 aarch64/riscv64 베어메탈 커널

## 주요 기능

- **멀티 아키텍처** — aarch64 (ARM64), riscv64 (RISC-V 64) 동시 지원
- **SMP** — 멀티코어 부팅, Per-CPU 데이터, IPI, CPU 친화도 기반 스케줄러
- **메모리 관리** — 비트맵 페이지 할당자, linked_list_allocator 힙, MMU (aarch64 4-level / riscv64 Sv39)
- **스레딩** — 커널 스레드, Round-robin 선점형 스케줄러, 유저 모드 전환
- **동기화** — Spinlock, Mutex, RwLock, Semaphore, SeqLock, RCU
- **가상 파일시스템** — VFS 추상화, RamFS, DevFS, FAT32 (읽기/쓰기)
- **블록 디바이스** — BlockDevice trait, RAM 디스크, VirtIO-blk (인터럽트 기반)
- **VirtIO** — MMIO 드라이버 프레임워크, Legacy/Modern 자동 감지
- **IPC** — 메시지 큐 (무제한/용량제한), Channel, POSIX mq API
- **커널 모듈** — ELF64 동적 로딩, 심볼 해석, PLT 지원
- **시스템 콜** — Linux 호환 ABI (프로세스/파일시스템)
- **Device Tree** — DTB 파싱, 런타임 하드웨어 탐색
- **보드 지원** — DTB compatible 기반 런타임 보드 선택

## 빠른 시작

### 요구 사항

- Rust stable 툴체인 (1.93.0+, edition 2024)
- QEMU (`qemu-system-aarch64` / `qemu-system-riscv64`)

### 빌드

```bash
# aarch64 빌드
cargo build --release --target targets/aarch64-unknown-none.json

# riscv64 빌드
cargo build --release --target targets/riscv64-unknown-elf.json
```

### QEMU 실행

```bash
# 기본 실행 (aarch64, 512MB)
./run.sh

# 아키텍처 지정
./run.sh aarch64        # ARM64, 512MB
./run.sh riscv64        # RISC-V, 512MB

# 메모리 크기 지정
./run.sh aarch64 256    # 256MB RAM
./run.sh riscv64 1024   # 1GB RAM

# 멀티코어 (SMP)
./run.sh aarch64 512 4  # 4코어, 512MB
./run.sh riscv64 512 2  # 2코어, 512MB
```

## 셸 명령어

커널 부팅 후 대화형 셸을 사용할 수 있습니다.

| 분류 | 명령어 | 설명 |
|------|--------|------|
| 메모리 | `meminfo` | 메모리 통계 출력 |
| | `test_alloc` | 힙 할당 테스트 |
| 스레드/SMP | `threads` | 전체 스레드 목록 (CPU 할당 표시) |
| | `spawn` | 테스트 스레드 생성 |
| | `cpuinfo` | CPU 상태 및 틱 카운트 |
| 파일시스템 | `ls [path]` | 디렉토리 내용 |
| | `cat <path>` | 파일 읽기 |
| | `write <path> <text>` | 파일 쓰기 |
| | `mount` | FAT32 마운트 (`/dev/vda` -> `/mnt`) |
| | `mounts` | 마운트 포인트 목록 |
| 블록 디바이스 | `blkinfo` | 블록 디바이스 목록 |
| | `blktest` | VirtIO 블록 읽기/쓰기 테스트 |
| 보드/하드웨어 | `boardinfo` | 현재 보드 정보 |
| | `lsboards` | 등록된 보드 목록 |
| IPC/모듈 | `mqtest` | 메시지 큐 테스트 |
| | `modtest` | 커널 모듈 로더 테스트 |
| | `lsmod` | 로드된 모듈 목록 |
| | `insmod <path>` | 커널 모듈 로드 |
| | `rmmod <name>` | 커널 모듈 언로드 |

## 프로젝트 구조

```
kerners/
├── src/
│   ├── main.rs          # 커널 엔트리 포인트 + 셸
│   ├── console.rs       # 콘솔 출력 (kprint!/kprintln!)
│   ├── arch/            # 아키텍처별 코드
│   │   ├── aarch64/     # GIC, Timer, MMU, Exception
│   │   └── riscv64/     # PLIC, Timer, MMU, Trap
│   ├── boards/          # 보드 설정 (QEMU virt, SMP 변형)
│   ├── mm/              # 메모리 관리 (heap, page, mmu)
│   ├── proc/            # 스레드 관리 (TCB, scheduler, percpu, context)
│   ├── sync/            # 동기화 프리미티브
│   ├── fs/              # VFS (ramfs, devfs, fat32)
│   ├── block/           # 블록 디바이스 (ramdisk, virtio-blk)
│   ├── virtio/          # VirtIO 드라이버 프레임워크
│   ├── drivers/         # 드라이버 프레임워크 + 플랫폼 설정
│   ├── ipc/             # IPC (메시지 큐)
│   ├── module/          # 커널 모듈 로더 (ELF64)
│   ├── syscall/         # 시스템 콜 인터페이스
│   └── dtb/             # Device Tree 파싱
├── modules/             # 외부 커널 모듈
├── targets/             # 커스텀 타겟 JSON
├── docs/                # 기술 문서
└── run.sh               # 빌드 및 실행 스크립트
```

## 아키텍처 개요

### 부팅 흐름

1. 아키텍처별 어셈블리 엔트리 (EL1 / M-mode)
2. `main.rs` 커널 진입
3. 서브시스템 초기화 (mm -> proc -> fs -> drivers -> virtio -> block)
4. DTB compatible 기반 보드 탐지
5. SMP 부트: PSCI (aarch64) / SBI HSM (riscv64)로 secondary CPU 시작
6. 대화형 셸 진입

### SMP 아키텍처

- **Per-CPU 데이터**: `PerCpuData` 구조체 (cpu_id, current_thread_idx, idle_thread_idx, tick_count)
- **CPU 부트**: Primary CPU가 서브시스템 초기화 후 Secondary CPU 기동
- **스케줄링**: 전역 스레드 리스트 + per-CPU current_thread_idx, CPU 친화도 지원
- **IPI**: aarch64 GIC SGI / riscv64 CLINT MSIP (크로스 CPU 리스케줄)

### 메모리 레이아웃

- 커널 로드 주소: 링커 스크립트로 설정 (aarch64: `0x40080000`)
- 비트맵 기반 페이지 프레임 할당자
- linked_list_allocator 힙 할당자

### 인터럽트 처리

- **aarch64**: GICv2 — Timer IRQ, UART IRQ, VirtIO IRQ, SGI (IPI)
- **riscv64**: PLIC + CLINT — Timer, Software Interrupt (IPI), External Interrupt

## 문서

### 기술 문서

| 문서 | 설명 |
|------|------|
| [docs/mm.md](docs/mm.md) | 메모리 관리 |
| [docs/proc.md](docs/proc.md) | 프로세스/스레드 관리 |
| [docs/sync.md](docs/sync.md) | 동기화 프리미티브 |
| [docs/vfs.md](docs/vfs.md) | 가상 파일시스템 |
| [docs/block.md](docs/block.md) | 블록 디바이스 서브시스템 |
| [docs/virtio.md](docs/virtio.md) | VirtIO 드라이버 프레임워크 |
| [docs/module.md](docs/module.md) | 커널 모듈 시스템 |
| [docs/plt.md](docs/plt.md) | Procedure Linkage Table |
| [docs/dtb.md](docs/dtb.md) | Device Tree Blob 파서 |
| [docs/syscall.md](docs/syscall.md) | 시스템 콜 인터페이스 |
| [docs/drivers.md](docs/drivers.md) | 드라이버 프레임워크 |
| [docs/ipc.md](docs/ipc.md) | IPC (메시지 큐) |
| [docs/console.md](docs/console.md) | 콘솔 출력 |
| [docs/board-module-system.md](docs/board-module-system.md) | 보드 모듈 시스템 |
| [docs/qemu-guide.md](docs/qemu-guide.md) | QEMU 실행 가이드 |

### 프로젝트 문서

- [AGENTS.md](AGENTS.md) — 코딩 에이전트 가이드라인 (English)
- [AGENTS-kr.md](AGENTS-kr.md) — 코딩 에이전트 가이드라인 (한국어)
- [plan.md](plan.md) — 개발 로드맵

## 라이선스

이 프로젝트는 [GPL-2.0-or-later](LICENSE) 라이선스로 배포됩니다.
