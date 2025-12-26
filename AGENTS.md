# AGENTS.md

> Coding Agent Guidelines for kerners - A bare-metal Rust kernel for aarch64 and riscv64

## Project Overview

**kerners** is a minimal Rust kernel skeleton designed for U-Boot, supporting both aarch64 and riscv64 architectures. This is a `#![no_std]` bare-metal environment with custom memory management, interrupt handling, virtual filesystem, threading, IPC, and kernel module loading.

**Tech Stack:**
- Language: Rust 2024 edition
- Environment: `#![no_std]`, `#![no_main]`
- Architectures: aarch64 (ARM64), riscv64 (RISC-V 64)
- Build System: Cargo with custom target JSONs
- Emulator: QEMU (virt machine)
- Allocator: linked_list_allocator 0.10

## Build, Test & Run Commands

### Building the Kernel

```bash
# Build for aarch64 (default)
cargo build --release --target targets/aarch64-unknown-none.json

# Build for riscv64
cargo build --release --target targets/riscv64-unknown-elf.json

# Build with embedded test module
cargo build --release --target targets/aarch64-unknown-none.json --features embed_test_module
```

### Running with QEMU

```bash
# Quick run (default: aarch64, 512MB RAM)
./run.sh

# Run specific architecture
./run.sh aarch64        # ARM64 with 512MB
./run.sh riscv64        # RISC-V with 512MB

# Run with custom memory size
./run.sh aarch64 256    # 256MB RAM
./run.sh riscv64 1024   # 1GB RAM

# Generate DTB only (don't run QEMU)
./run.sh aarch64 512 --dtb-only
```

### Testing

**No formal unit test framework** (bare-metal environment prevents standard `#[test]` usage).

**Prefer macOS default commands:** Testing and validation should primarily use macOS built-in commands without additional tool installations. Available commands include:
- `file` - Identify file types (ELF, binary, etc.)
- `hexdump`, `xxd` - Binary inspection
- `od` - Octal/hex dump
- `strings` - Extract text from binaries
- `ls`, `stat` - File information
- `diff`, `cmp` - Compare files
- `shasum`, `md5` - Checksums
- `otool` - Mach-O analysis (for host binaries)

**Testing approach:**
1. Run kernel in QEMU using `./run.sh`
2. Execute test commands in the interactive shell:

```bash
# Memory tests
meminfo              # Display memory statistics
test_alloc           # Test heap allocation (Box, Vec, String)

# Threading tests
threads              # List all threads
spawn                # Spawn test threads
usertest             # Test user mode transition

# IPC tests
mqtest               # Message queue tests

# Module loading tests
modtest              # Kernel module loader tests

# VFS tests
ls                   # List root directory
cat /hello.txt       # Read file content
mount                # List mount points
```

**Inline test functions** in `src/main.rs`:
- `test_memory_allocation()` - Box, Vec, String, page allocation
- `test_message_queue()` - IPC mechanisms
- `test_module_loader()` - Dynamic module loading
- `test_user_mode()` - User mode execution
- `test_thread_entry()` - Thread spawning

### Building Modules

```bash
# Build test module for specific architecture
cd modules/hello
./build.sh aarch64   # or riscv64
```

## Code Style Guidelines

### Import Organization

Organize imports in this order:
1. External crate imports (`alloc`, dependencies)
2. Core library imports
3. Internal crate imports
4. Module-local imports

```rust
// Example:
use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};
use crate::sync::{Mutex, Semaphore};
```

### Naming Conventions

- **Files:** `snake_case.rs`, use `mod.rs` for module roots
- **Functions:** `snake_case` (e.g., `alloc_frame`, `sys_write`, `handle_irq`)
- **Variables:** `snake_case` for locals
- **Constants:** `SCREAMING_SNAKE_CASE` (e.g., `PAGE_SIZE`, `KERNEL_START`)
- **Static Variables:** `SCREAMING_SNAKE_CASE` (e.g., `HEAP_ALLOCATOR`, `THREADS`)
- **Types:** `CamelCase` for structs, enums, traits (e.g., `Thread`, `VNode`, `MessageQueue`)
- **Type Aliases:** `CamelCase` (e.g., `type Tid = u64`, `type VfsResult<T> = Result<T, VfsError>`)
- **Modules:** `snake_case`

### Formatting

- **Indentation:** 4 spaces (Rust standard)
- **Line Length:** ~100 characters (flexible)
- **Braces:** Opening brace on same line (K&R style)
- **Spacing:** Space after commas, around operators, after colons in type annotations

```rust
pub fn init(base: usize, size: usize) -> Result<(), MemError> {
    // implementation
}
```

### Type Usage

- Use explicit type annotations for clarity in complex scenarios
- Prefer `Result<T, E>` for error handling
- Create type aliases for common Result patterns: `pub type VfsResult<T> = Result<T, VfsError>;`
- Use `Option<T>` for nullable values
- Leverage strong typing for safety (e.g., newtype pattern for IDs: `Tid`, `Pid`)

### Error Handling

**Always use custom error types:**

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VfsError {
    NotFound,
    PermissionDenied,
    AlreadyExists,
    NotADirectory,
    IsADirectory,
    InvalidPath,
    IoError,
}

impl fmt::Display for VfsError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            VfsError::NotFound => write!(f, "File or directory not found"),
            // ... more variants
        }
    }
}

pub type VfsResult<T> = Result<T, VfsError>;
```

**Error handling pattern:**

```rust
match mm::init(ram_base, ram_size) {
    Ok(_layout) => {
        // Success path
    }
    Err(e) => {
        kprintln!("[boot] ERROR: Memory init failed: {}", e);
    }
}
```

### Unsafe Code

**CRITICAL:** Always document unsafe code with safety comments.

```rust
/// # Safety
/// This function is only called once during boot with valid DTB address.
pub unsafe fn init(dtb_addr: usize) -> Result<Self, DtbError> {
    // implementation
}

// Or inline:
unsafe {
    // SAFETY: UART_BASE is valid MMIO address for this platform
    write_volatile((UART_BASE + offset) as *mut u32, value);
}
```

### Documentation

**Module-level documentation:**

```rust
//! Memory Management
//!
//! This module provides page and heap allocation for the kernel.
//!
//! ## Features
//! - Bitmap-based page allocator
//! - Linked-list heap allocator
//! - Safe abstractions over unsafe operations
```

**Function documentation:**

```rust
/// Allocates a physical page frame.
///
/// # Returns
/// Physical address of the allocated page, or `None` if no pages available.
///
/// # Examples
/// ```
/// if let Some(addr) = alloc_frame() {
///     // Use page at addr
/// }
/// ```
pub fn alloc_frame() -> Option<usize> {
    // implementation
}
```

## Architecture Patterns

### Architecture-Specific Code

Use conditional compilation for architecture-specific implementations:

```rust
#[cfg(target_arch = "aarch64")]
#[path = "arch/aarch64/mod.rs"]
mod arch;

#[cfg(target_arch = "riscv64")]
#[path = "arch/riscv64/mod.rs"]
mod arch;
```

All architecture-specific code goes in `src/arch/<arch>/`.

### Common Interface via Traits

```rust
pub trait VNode: Send + Sync {
    fn node_type(&self) -> VNodeType;
    fn read(&self, offset: usize, buf: &mut [u8]) -> VfsResult<usize>;
    fn write(&self, offset: usize, buf: &[u8]) -> VfsResult<usize>;
    // ... with default implementations where appropriate
}
```

### Global State with Interior Mutability

```rust
static HEAP_ALLOCATOR: LockedHeap = LockedHeap::empty();
static FRAME_ALLOCATOR: Mutex<FrameAllocator> = Mutex::new(FrameAllocator::new());
```

Use synchronization primitives from `src/sync/`:
- `Spinlock<T>` - Busy-waiting lock
- `Mutex<T>` - Adaptive mutex (spin then yield)
- `RwLock<T>` - Reader-writer lock
- `Semaphore` - Counting semaphore
- `SeqLock<T>` - Sequence lock (writer priority)
- `RcuCell<T>` - Read-Copy-Update (lock-free reads)

### Hardware Register Access

Always use volatile operations:

```rust
#[inline]
unsafe fn read_reg(offset: usize) -> u32 {
    read_volatile((UART_BASE + offset) as *const u32)
}

#[inline]
unsafe fn write_reg(offset: usize, value: u32) {
    write_volatile((UART_BASE + offset) as *mut u32, value);
}
```

## Important Constraints

1. **No Standard Library:** This is a `#![no_std]` environment. Use `alloc::` for heap types (Box, Vec, String, Arc, etc.)
2. **Custom Allocators:** All memory allocation uses custom allocators (heap and page)
3. **No Panicking:** Release profile uses `panic = "abort"`. Custom panic handler loops forever.
4. **Volatile Access Required:** All hardware register access must use `read_volatile`/`write_volatile`
5. **No Interactive Input in Tests:** QEMU tests require manual command entry in the shell
6. **No Nightly Rust:** Do NOT use Rust nightly toolchain. Use stable Rust only. This project is designed to build with stable Rust. **Never use `cargo +nightly` commands.**

## Workflow

1. **Check plan.md first** - Development tasks and roadmap are tracked there
2. **Read relevant docs** - `docs/mm.md` for memory management, etc.
3. **Maintain architecture abstraction** - Keep common code separate from arch-specific code
4. **Test in QEMU** - Always verify changes by running `./run.sh` and using shell commands
5. **Document unsafe code** - Every unsafe block must explain why it's safe
6. **Update documentation** - When adding/modifying modules, update corresponding docs in `docs/`
7. **Keep Project Structure current** - Update AGENTS.md and AGENTS-kr.md when adding new files/directories

## Documentation Guidelines

When developing new features or modifying existing code:

1. **Create/update docs**: Add or update documentation in `docs/` for any significant module changes
2. **Update Project Structure**: Reflect new files/directories in AGENTS.md and AGENTS-kr.md
3. **Update References**: Add new docs to the References section in AGENTS.md, AGENTS-kr.md, and README.md

**Documentation files:**
- `docs/mm.md` - Memory management (heap, page allocator)
- `docs/proc.md` - Process/thread management
- `docs/sync.md` - Synchronization primitives
- `docs/vfs.md` - Virtual File System
- `docs/block.md` - Block device subsystem
- `docs/virtio.md` - VirtIO driver framework
- `docs/module.md` - Kernel module system
- `docs/plt.md` - Procedure Linkage Table
- `docs/dtb.md` - Device Tree Blob parser
- `docs/syscall.md` - System call interface
- `docs/drivers.md` - Driver framework
- `docs/ipc.md` - IPC (message queues)
- `docs/console.md` - Console output
- `docs/board-module-system.md` - Board module system
- `docs/qemu-guide.md` - QEMU execution guide

## Project Structure

```
kerners/
├── src/
│   ├── main.rs              # Kernel entry point (boot, init, shell)
│   ├── console.rs           # Console output abstraction
│   ├── arch/                # Architecture-specific implementations
│   │   ├── aarch64/         # ARM64 implementation
│   │   │   ├── mod.rs       # Module definition
│   │   │   ├── exception.rs # Exception handling
│   │   │   ├── gic.rs       # GIC (interrupt controller)
│   │   │   ├── mmu.rs       # Memory management unit
│   │   │   ├── timer.rs     # Timer driver
│   │   │   └── uart.rs      # UART driver
│   │   └── riscv64/         # RISC-V 64 implementation
│   │       ├── mod.rs       # Module definition
│   │       ├── trap.rs      # Trap handling
│   │       ├── plic.rs      # PLIC (interrupt controller)
│   │       ├── mmu.rs       # Memory management unit
│   │       ├── timer.rs     # Timer driver
│   │       └── uart.rs      # UART driver
│   ├── mm/                  # Memory management
│   │   ├── mod.rs           # Memory subsystem
│   │   ├── heap.rs          # Heap allocator (linked_list_allocator)
│   │   └── page.rs          # Page frame allocator (bitmap-based)
│   ├── proc/                # Process/thread management
│   │   ├── mod.rs           # Thread abstraction (TCB)
│   │   ├── context.rs       # CPU context (register save/restore)
│   │   ├── scheduler.rs     # Round-robin scheduler
│   │   └── user.rs          # User mode transition support
│   ├── sync/                # Synchronization primitives
│   │   ├── mod.rs           # Sync module
│   │   ├── spinlock.rs      # Busy-waiting spinlock
│   │   ├── mutex.rs         # Adaptive mutex (spin then yield)
│   │   ├── rwlock.rs        # Reader-writer lock
│   │   ├── semaphore.rs     # Counting semaphore
│   │   ├── seqlock.rs       # Sequence lock (writer priority)
│   │   └── rcu.rs           # Read-Copy-Update (lock-free reads)
│   ├── fs/                  # Virtual file system (VFS)
│   │   ├── mod.rs           # VFS abstraction (VNode, FileSystem trait)
│   │   ├── path.rs          # Path parsing and normalization
│   │   ├── fd.rs            # File descriptor table
│   │   ├── ramfs/           # Memory-based filesystem
│   │   ├── devfs/           # Device filesystem (/dev)
│   │   └── fat32/           # FAT32 filesystem
│   │       ├── mod.rs       # FAT32 implementation
│   │       ├── boot.rs      # Boot sector parsing
│   │       ├── fat.rs       # FAT table handling
│   │       └── dir.rs       # Directory entries
│   ├── block/               # Block device abstraction
│   │   ├── mod.rs           # BlockDevice trait
│   │   ├── ramdisk.rs       # RAM disk
│   │   └── virtio_blk.rs    # VirtIO block device
│   ├── virtio/              # VirtIO driver framework
│   │   ├── mod.rs           # VirtIO device enumeration
│   │   ├── mmio.rs          # MMIO register interface
│   │   └── queue.rs         # Virtqueue implementation
│   ├── drivers/             # Driver framework
│   │   └── mod.rs           # Driver trait, DTB-based probe
│   ├── ipc/                 # Inter-process communication
│   │   ├── mod.rs           # IPC module
│   │   └── message_queue.rs # Message queue (bounded/unbounded)
│   ├── module/              # Kernel module loader
│   │   ├── mod.rs           # Module system
│   │   ├── elf.rs           # ELF64 parser
│   │   ├── loader.rs        # Dynamic loading and relocation
│   │   └── symbol.rs        # Symbol table management
│   ├── syscall/             # System call interface
│   │   ├── mod.rs           # System call dispatcher
│   │   ├── process.rs       # Process-related syscalls
│   │   └── fs.rs            # Filesystem-related syscalls
│   └── dtb/                 # Device Tree Blob parsing
│       └── mod.rs           # DTB parser
├── modules/hello/           # Test kernel module
├── targets/                 # Custom target JSON files
├── docs/                    # Documentation (see docs/README.md for full list)
├── linker_aarch64.ld        # aarch64 linker script
├── linker_riscv64.ld        # riscv64 linker script
├── run.sh                   # Build & QEMU execution script
└── plan.md                  # Development roadmap
```

## References

- [AGENTS-kr.md](AGENTS-kr.md) - Korean version of this document
- [plan.md](plan.md) - Project development plan and task tracking

### Documentation

- [docs/mm.md](docs/mm.md) - Memory management
- [docs/proc.md](docs/proc.md) - Process/thread management
- [docs/sync.md](docs/sync.md) - Synchronization primitives
- [docs/vfs.md](docs/vfs.md) - Virtual File System
- [docs/block.md](docs/block.md) - Block device subsystem
- [docs/virtio.md](docs/virtio.md) - VirtIO driver framework
- [docs/module.md](docs/module.md) - Kernel module system
- [docs/plt.md](docs/plt.md) - Procedure Linkage Table
- [docs/dtb.md](docs/dtb.md) - Device Tree Blob parser
- [docs/syscall.md](docs/syscall.md) - System call interface
- [docs/drivers.md](docs/drivers.md) - Driver framework
- [docs/ipc.md](docs/ipc.md) - IPC (message queues)
- [docs/console.md](docs/console.md) - Console output
- [docs/board-module-system.md](docs/board-module-system.md) - Board module system
- [docs/qemu-guide.md](docs/qemu-guide.md) - QEMU execution guide
