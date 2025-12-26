# AGENTS-kr.md

> AI 코딩 에이전트를 위한 프로젝트 컨텍스트 문서 (한국어)

## Project Overview

Kerners는 Rust로 작성된 베어메탈 커널 프로젝트입니다. aarch64와 riscv64 아키텍처를 지원하며, U-Boot 페이로드로 사용할 수 있는 최소한의 커널 스켈레톤입니다.

## Tech Stack

- **Language**: Rust (no_std 환경, 2024 edition)
- **Target Architectures**: aarch64, riscv64
- **Build System**: Cargo with custom target JSONs
- **Emulator**: QEMU (virt machine)
- **Allocator**: linked_list_allocator 0.10

## Project Structure

```
kerners/
├── src/
│   ├── main.rs              # 커널 엔트리 포인트 (부팅, 초기화, 셸)
│   ├── console.rs           # 콘솔 출력 추상화
│   ├── arch/                # 아키텍처별 구현
│   │   ├── aarch64/         # ARM64 구현
│   │   │   ├── mod.rs       # 모듈 정의
│   │   │   ├── exception.rs # 예외 처리
│   │   │   ├── gic.rs       # GIC (인터럽트 컨트롤러)
│   │   │   ├── mmu.rs       # 메모리 관리 유닛
│   │   │   ├── timer.rs     # 타이머
│   │   │   └── uart.rs      # UART 드라이버
│   │   └── riscv64/         # RISC-V 64 구현
│   │       ├── mod.rs       # 모듈 정의
│   │       ├── trap.rs      # 트랩 처리
│   │       ├── plic.rs      # PLIC (인터럽트 컨트롤러)
│   │       ├── mmu.rs       # 메모리 관리 유닛
│   │       ├── timer.rs     # 타이머
│   │       └── uart.rs      # UART 드라이버
│   ├── mm/                  # 메모리 관리
│   │   ├── mod.rs           # 메모리 서브시스템
│   │   ├── heap.rs          # 힙 할당자 (linked_list_allocator)
│   │   └── page.rs          # 페이지 프레임 할당자 (비트맵 기반)
│   ├── proc/                # 프로세스/스레드 관리
│   │   ├── mod.rs           # 스레드 추상화 (TCB)
│   │   ├── context.rs       # CPU 컨텍스트 (레지스터 저장/복원)
│   │   ├── scheduler.rs     # 라운드 로빈 스케줄러
│   │   └── user.rs          # 유저 모드 전환 지원
│   ├── sync/                # 동기화 프리미티브
│   │   ├── mod.rs           # 동기화 모듈
│   │   ├── spinlock.rs      # Busy-waiting 스핀락
│   │   ├── mutex.rs         # 어댑티브 뮤텍스 (spin then yield)
│   │   ├── rwlock.rs        # Reader-Writer 락
│   │   ├── semaphore.rs     # 카운팅 세마포어
│   │   ├── seqlock.rs       # 순차 락 (Writer 우선)
│   │   └── rcu.rs           # Read-Copy-Update (락 프리 읽기)
│   ├── fs/                  # 가상 파일시스템 (VFS)
│   │   ├── mod.rs           # VFS 추상화 (VNode, FileSystem trait)
│   │   ├── path.rs          # 경로 파싱 및 정규화
│   │   ├── fd.rs            # 파일 디스크립터 테이블
│   │   ├── ramfs/           # 메모리 기반 파일시스템
│   │   ├── devfs/           # 장치 파일시스템 (/dev)
│   │   └── fat32/           # FAT32 파일시스템
│   │       ├── mod.rs       # FAT32 구현
│   │       ├── boot.rs      # 부트 섹터 파싱
│   │       ├── fat.rs       # FAT 테이블 처리
│   │       └── dir.rs       # 디렉토리 엔트리
│   ├── block/               # 블록 디바이스 추상화
│   │   ├── mod.rs           # BlockDevice trait
│   │   ├── ramdisk.rs       # RAM 디스크
│   │   └── virtio_blk.rs    # VirtIO 블록 디바이스
│   ├── virtio/              # VirtIO 드라이버 프레임워크
│   │   ├── mod.rs           # VirtIO 디바이스 열거
│   │   ├── mmio.rs          # MMIO 레지스터 인터페이스
│   │   └── queue.rs         # Virtqueue 구현
│   ├── drivers/             # 드라이버 프레임워크
│   │   └── mod.rs           # Driver trait, DTB 기반 probe
│   ├── ipc/                 # 프로세스 간 통신
│   │   ├── mod.rs           # IPC 모듈
│   │   └── message_queue.rs # 메시지 큐 (bounded/unbounded)
│   ├── module/              # 커널 모듈 로더
│   │   ├── mod.rs           # 모듈 시스템
│   │   ├── elf.rs           # ELF64 파서
│   │   ├── loader.rs        # 동적 로딩 및 재배치
│   │   └── symbol.rs        # 심볼 테이블 관리
│   ├── syscall/             # 시스템 콜 인터페이스
│   │   ├── mod.rs           # 시스템 콜 디스패처
│   │   ├── process.rs       # 프로세스 관련 시스템 콜
│   │   └── fs.rs            # 파일시스템 관련 시스템 콜
│   └── dtb/                 # Device Tree Blob 파싱
│       └── mod.rs           # DTB 파서
├── modules/hello/           # 테스트 커널 모듈
├── targets/                 # 커스텀 타겟 JSON 파일
├── docs/                    # 문서 (전체 목록은 docs/README.md 참조)
├── linker_aarch64.ld        # aarch64 링커 스크립트
├── linker_riscv64.ld        # riscv64 링커 스크립트
├── run.sh                   # 빌드 및 QEMU 실행 스크립트
└── plan.md                  # 개발 계획 및 로드맵
```

## Build & Run

```bash
# aarch64 빌드
cargo build --release --target targets/aarch64-unknown-none.json

# riscv64 빌드
cargo build --release --target targets/riscv64-unknown-elf.json

# QEMU로 실행 (기본: aarch64, 512MB RAM)
./run.sh

# 아키텍처 및 메모리 크기 지정
./run.sh aarch64 512    # ARM64, 512MB
./run.sh riscv64 1024   # RISC-V, 1GB
```

## Shell Commands

QEMU에서 실행 후 대화형 셸에서 사용 가능한 명령어:

```bash
# 메모리
meminfo              # 메모리 사용량 통계
test_alloc           # 힙 할당 테스트

# 스레드
threads              # 스레드 목록
spawn                # 테스트 스레드 생성
usertest             # 유저 모드 전환 테스트

# IPC
mqtest               # 메시지 큐 테스트

# 모듈
modtest              # 커널 모듈 로더 테스트

# 파일시스템
ls                   # 루트 디렉토리 목록
cat <path>           # 파일 내용 출력
mount                # 마운트 목록
```

## Coding Conventions

- Rust 표준 네이밍 컨벤션 사용 (snake_case for functions/variables, CamelCase for types)
- Rust 2024 edition 사용
- `unsafe` 블록 사용 시 반드시 주석으로 safety 설명
- 아키텍처별 코드는 `src/arch/<arch>/` 디렉토리에 위치
- 공통 인터페이스는 trait으로 추상화

## Architecture Notes

- **부팅 흐름**: 아키텍처별 어셈블리 → `main.rs` → 초기화 루틴 → 셸
- **인터럽트 처리**: aarch64(GIC), riscv64(PLIC)
- **메모리 관리**: 페이지 할당자 (비트맵) + 힙 할당자 (linked_list)
- **스케줄링**: 라운드 로빈 선점형 스케줄러
- **파일시스템**: VFS 추상화 → ramfs, devfs, fat32
- **블록 디바이스**: BlockDevice trait → ramdisk, virtio-blk
- **모듈 로더**: ELF64 relocatable object 동적 로딩

## Important Considerations

- `#![no_std]` 환경이므로 표준 라이브러리 사용 불가
- `alloc` 크레이트 사용 가능 (Box, Vec, String, Arc 등)
- 모든 메모리 할당은 커스텀 할당자 사용
- 하드웨어 접근은 volatile 읽기/쓰기 필수
- plan.md 파일에 계획을 우선적으로 세운 후 개발진행
- **Rust nightly 사용 금지**: nightly 툴체인을 사용하지 마세요. 이 프로젝트는 stable Rust로 빌드되도록 설계되었습니다. **`cargo +nightly` 명령어를 사용하지 마세요.**

## Testing

**macOS 기본 명령어 위주로 사용:** 테스트 및 검증 시 추가 도구 설치 없이 macOS 기본 명령어 위주로 사용합니다:
- `file` - 파일 타입 확인 (ELF, 바이너리 등)
- `hexdump`, `xxd` - 바이너리 검사
- `od` - Octal/Hex 덤프
- `strings` - 바이너리에서 텍스트 추출
- `ls`, `stat` - 파일 정보
- `diff`, `cmp` - 파일 비교
- `shasum`, `md5` - 체크섬
- `otool` - Mach-O 분석 (호스트 바이너리용)

```bash
# QEMU에서 테스트 실행
./run.sh

# 테스트 모듈 포함 빌드
cargo build --release --target targets/aarch64-unknown-none.json --features embed_test_module
```

## Development Workflow

1. **plan.md 확인** - 개발 계획 및 로드맵 확인
2. **관련 docs 읽기** - `docs/` 하위 문서 참조
3. **아키텍처 추상화 유지** - 공통 코드와 아키텍처별 코드 분리
4. **QEMU 테스트** - `./run.sh`로 변경사항 검증
5. **unsafe 코드 문서화** - 모든 unsafe 블록에 safety 설명
6. **문서 업데이트** - 모듈 추가/수정 시 `docs/` 하위 문서 업데이트
7. **프로젝트 구조 반영** - 새 파일/디렉토리 추가 시 AGENTS.md 및 AGENTS-kr.md 업데이트

## Documentation Guidelines

새 기능 개발 또는 기존 코드 수정 시:

1. **docs 생성/업데이트**: 주요 모듈 변경 시 `docs/`에 문서 추가 또는 업데이트
2. **프로젝트 구조 업데이트**: 새 파일/디렉토리를 AGENTS.md와 AGENTS-kr.md에 반영
3. **References 업데이트**: 새 문서를 AGENTS.md, AGENTS-kr.md, README.md의 References에 추가

**문서 파일 목록:**
- `docs/mm.md` - 메모리 관리 (힙, 페이지 할당자)
- `docs/proc.md` - 프로세스/스레드 관리
- `docs/sync.md` - 동기화 프리미티브
- `docs/vfs.md` - 가상 파일시스템
- `docs/block.md` - 블록 디바이스 서브시스템
- `docs/virtio.md` - VirtIO 드라이버 프레임워크
- `docs/module.md` - 커널 모듈 시스템
- `docs/plt.md` - Procedure Linkage Table
- `docs/dtb.md` - Device Tree Blob 파서
- `docs/syscall.md` - 시스템 콜 인터페이스
- `docs/drivers.md` - 드라이버 프레임워크
- `docs/ipc.md` - IPC (메시지 큐)
- `docs/console.md` - 콘솔 출력
- `docs/board-module-system.md` - 보드 모듈 시스템
- `docs/qemu-guide.md` - QEMU 실행 가이드

## References

- [AGENTS.md](AGENTS.md) - 영어 버전 문서
- [plan.md](plan.md) - 프로젝트 계획

### Documentation

- [docs/mm.md](docs/mm.md) - 메모리 관리
- [docs/proc.md](docs/proc.md) - 프로세스/스레드 관리
- [docs/sync.md](docs/sync.md) - 동기화 프리미티브
- [docs/vfs.md](docs/vfs.md) - 가상 파일시스템
- [docs/block.md](docs/block.md) - 블록 디바이스 서브시스템
- [docs/virtio.md](docs/virtio.md) - VirtIO 드라이버 프레임워크
- [docs/module.md](docs/module.md) - 커널 모듈 시스템
- [docs/plt.md](docs/plt.md) - Procedure Linkage Table
- [docs/dtb.md](docs/dtb.md) - Device Tree Blob 파서
- [docs/syscall.md](docs/syscall.md) - 시스템 콜 인터페이스
- [docs/drivers.md](docs/drivers.md) - 드라이버 프레임워크
- [docs/ipc.md](docs/ipc.md) - IPC (메시지 큐)
- [docs/console.md](docs/console.md) - 콘솔 출력
- [docs/board-module-system.md](docs/board-module-system.md) - 보드 모듈 시스템
- [docs/qemu-guide.md](docs/qemu-guide.md) - QEMU 실행 가이드
