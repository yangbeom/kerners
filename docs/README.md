# kerners 문서

kerners 커널의 서브시스템별 설계 및 구현 문서입니다.

## 서브시스템 문서

| 문서 | 설명 |
|------|------|
| [mm.md](mm.md) | 메모리 관리 - 페이지 할당자, 힙 할당자, MMU |
| [proc.md](proc.md) | 프로세스 관리 - 스레드, 스케줄러, 사용자 모드 |
| [sync.md](sync.md) | 동기화 프리미티브 - Spinlock, Mutex, RwLock, Semaphore, RCU |
| [vfs.md](vfs.md) | 가상 파일시스템 - VNode, VFS 인터페이스, RamFS, DevFS |
| [block.md](block.md) | 블록 디바이스 레이어 - BlockDevice trait, VirtIO 블록 |
| [virtio.md](virtio.md) | VirtIO 서브시스템 - MMIO 트랜스포트, 블록 드라이버 |
| [module.md](module.md) | 모듈 로더 - ELF64 파싱, 심볼 해석, PLT 지원 |
| [plt.md](plt.md) | PLT(Procedure Linkage Table) 구현 상세 |
| [dtb.md](dtb.md) | Device Tree Blob 파서 - FDT 파싱, 디바이스 탐색 |
| [syscall.md](syscall.md) | 시스템 콜 인터페이스 - Linux 호환 ABI, 디스패처 |
| [drivers.md](drivers.md) | 드라이버 프레임워크 - DTB 탐색, 플랫폼 설정 |
| [ipc.md](ipc.md) | IPC - 메시지 큐, 채널, POSIX mq API |
| [console.md](console.md) | 콘솔 출력 - kprint!/kprintln! 매크로 |
| [board-module-system.md](board-module-system.md) | 보드 모듈 시스템 - DTB compatible 기반 런타임 보드 선택 |
| [qemu-guide.md](qemu-guide.md) | QEMU 실행 가이드 - DTB 설정, 문제 해결 |

## 아키텍처 지원

- **AArch64** (ARM64): QEMU virt 머신, PSCI 기반 SMP
- **RISC-V64**: QEMU virt 머신, SBI HSM 기반 SMP

## 빠른 시작

```bash
# 빌드 및 실행
./run.sh aarch64 512      # ARM64, 512MB
./run.sh riscv64 512      # RISC-V64, 512MB
./run.sh aarch64 512 4    # ARM64, 512MB, 4 CPUs
```

## 개발 로드맵

전체 개발 계획은 [plan.md](../plan.md)를 참조하세요.
