# kerners

> English | [한국어](README.md)

A bare-metal Rust kernel for aarch64 and riscv64 architectures.

## Features

- **Multi-architecture** — Supports both aarch64 (ARM64) and riscv64 (RISC-V 64)
- **SMP** — Multi-core boot, per-CPU data, IPI, CPU affinity-aware scheduler
- **Memory management** — Bitmap page allocator, linked_list_allocator heap, MMU (aarch64 4-level / riscv64 Sv39)
- **Threading** — Kernel threads, round-robin preemptive scheduler, user mode transition
- **Synchronization** — Spinlock, Mutex, RwLock, Semaphore, SeqLock, RCU
- **Virtual File System** — VFS abstraction with RamFS, DevFS, FAT32 (read/write)
- **Block devices** — BlockDevice trait, RAM disk, VirtIO-blk (interrupt-driven)
- **VirtIO** — MMIO driver framework with Legacy/Modern auto-detection
- **IPC** — Message queues (unbounded/bounded), Channel, POSIX mq API
- **Kernel modules** — Dynamic ELF64 loading with symbol resolution and PLT support
- **Test infrastructure** — Kernel module-based automated testing, runs in QEMU, `make test` automation
- **System calls** — Linux-compatible ABI (process/filesystem)
- **Device Tree** — DTB parsing for runtime hardware discovery
- **Board support** — Runtime board selection via DTB compatible matching

## Quick Start

### Prerequisites

- Rust stable toolchain (1.93.0+, edition 2024)
- QEMU (`qemu-system-aarch64` / `qemu-system-riscv64`)
- mtools (required for tests — `brew install mtools` / `apt install mtools`)

### Build

```bash
# Build for aarch64
cargo build --release --target targets/aarch64-unknown-none.json

# Build for riscv64
cargo build --release --target targets/riscv64-unknown-elf.json
```

### Run with QEMU

```bash
# Default run (aarch64, 512MB)
./run.sh

# Specify architecture
./run.sh aarch64        # ARM64, 512MB
./run.sh riscv64        # RISC-V, 512MB

# Custom memory size
./run.sh aarch64 256    # 256MB RAM
./run.sh riscv64 1024   # 1GB RAM

# Multi-core (SMP)
./run.sh aarch64 512 4  # 4 cores, 512MB
./run.sh riscv64 512 2  # 2 cores, 512MB
```

### Testing

```bash
# aarch64 tests (default)
make test

# riscv64 tests
make test ARCH=riscv64

# Both architectures
make test-all
```

See [docs/testing.md](docs/testing.md) for details.

## Shell Commands

An interactive shell is available after the kernel boots.

| Category | Command | Description |
|----------|---------|-------------|
| Memory | `meminfo` | Display memory statistics |
| | `test_alloc` | Test heap allocation |
| Thread/SMP | `threads` | List all threads (shows CPU assignment) |
| | `spawn` | Spawn test threads |
| | `cpuinfo` | Show CPU status and tick counts |
| Filesystem | `ls [path]` | List directory contents |
| | `cat <path>` | Read file content |
| | `write <path> <text>` | Write text to file |
| | `mount` | Mount FAT32 (`/dev/vda` -> `/mnt`) |
| | `mounts` | List mount points |
| Block Devices | `blkinfo` | List block devices |
| | `blktest` | VirtIO block read/write test |
| Board/Hardware | `boardinfo` | Current board information |
| | `lsboards` | List registered boards |
| IPC/Modules | `mqtest` | Message queue tests |
| | `modtest` | Kernel module loader tests |
| | `lsmod` | List loaded modules |
| | `insmod <path>` | Load kernel module |
| | `rmmod <name>` | Unload kernel module |

## Project Structure

```
kerners/
├── src/
│   ├── main.rs          # Kernel entry point + shell
│   ├── console.rs       # Console output (kprint!/kprintln!)
│   ├── arch/            # Architecture-specific code
│   │   ├── aarch64/     # GIC, Timer, MMU, Exception
│   │   └── riscv64/     # PLIC, Timer, MMU, Trap
│   ├── boards/          # Board configurations (QEMU virt, SMP variants)
│   ├── mm/              # Memory management (heap, page, mmu)
│   ├── proc/            # Thread management (TCB, scheduler, percpu, context)
│   ├── sync/            # Synchronization primitives
│   ├── fs/              # VFS (ramfs, devfs, fat32)
│   ├── block/           # Block devices (ramdisk, virtio-blk)
│   ├── virtio/          # VirtIO driver framework
│   ├── drivers/         # Driver framework + platform config
│   ├── ipc/             # IPC (message queues)
│   ├── module/          # Kernel module loader (ELF64)
│   ├── syscall/         # System call interface
│   └── dtb/             # Device Tree parsing
├── modules/             # External kernel modules + test modules
├── scripts/             # Test build/run scripts
├── targets/             # Custom target JSON files
├── docs/                # Technical documentation
├── Makefile             # Build/test targets
└── run.sh               # Build & run script
```

## Architecture Overview

### Boot Flow

1. Architecture-specific assembly entry (EL1 / M-mode)
2. `main.rs` kernel entry
3. Subsystem initialization (mm -> proc -> fs -> drivers -> virtio -> block)
4. Board detection via DTB compatible matching
5. SMP boot: secondary CPUs started via PSCI (aarch64) / SBI HSM (riscv64)
6. Interactive shell

### SMP Architecture

- **Per-CPU data**: `PerCpuData` struct (cpu_id, current_thread_idx, idle_thread_idx, tick_count)
- **CPU boot**: Primary CPU initializes all subsystems, then boots secondary CPUs
- **Scheduling**: Global thread list + per-CPU current thread tracking, CPU affinity support
- **IPI**: aarch64 GIC SGI / riscv64 CLINT MSIP for cross-CPU reschedule notifications

### Memory Layout

- Kernel load address: configurable via linker script (aarch64: `0x40080000`)
- Bitmap-based page frame allocator
- linked_list_allocator heap allocator

### Interrupt Handling

- **aarch64**: GICv2 — Timer IRQ, UART IRQ, VirtIO IRQ, SGI (IPI)
- **riscv64**: PLIC + CLINT — Timer, Software Interrupt (IPI), External Interrupt

## Documentation

### Technical Docs

| Document | Description |
|----------|-------------|
| [docs/mm.md](docs/mm.md) | Memory management |
| [docs/proc.md](docs/proc.md) | Process/thread management |
| [docs/sync.md](docs/sync.md) | Synchronization primitives |
| [docs/vfs.md](docs/vfs.md) | Virtual File System |
| [docs/block.md](docs/block.md) | Block device subsystem |
| [docs/virtio.md](docs/virtio.md) | VirtIO driver framework |
| [docs/module.md](docs/module.md) | Kernel module system |
| [docs/plt.md](docs/plt.md) | Procedure Linkage Table |
| [docs/dtb.md](docs/dtb.md) | Device Tree Blob parser |
| [docs/syscall.md](docs/syscall.md) | System call interface |
| [docs/drivers.md](docs/drivers.md) | Driver framework |
| [docs/ipc.md](docs/ipc.md) | IPC (message queues) |
| [docs/console.md](docs/console.md) | Console output |
| [docs/board-module-system.md](docs/board-module-system.md) | Board module system |
| [docs/qemu-guide.md](docs/qemu-guide.md) | QEMU execution guide |
| [docs/testing.md](docs/testing.md) | Test infrastructure |

### Project Docs

- [AGENTS.md](AGENTS.md) — Coding agent guidelines (English)
- [AGENTS-kr.md](AGENTS-kr.md) — Coding agent guidelines (Korean)
- [plan.md](plan.md) — Development roadmap

## License

This project is licensed under [GPL-2.0-or-later](LICENSE).
