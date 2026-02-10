#![no_std]
#![no_main]

extern crate alloc;

use core::panic::PanicInfo;

mod block;
mod boards;
mod console;
mod drivers;
mod log;
mod dtb;
mod fs;
mod ipc;
mod mm;
mod module;
mod proc;
mod sync;
mod syscall;
mod virtio;

#[cfg(feature = "test_runner")]
mod test_runner;

#[cfg(target_arch = "aarch64")]
#[path = "arch/aarch64/mod.rs"]
mod arch;

#[cfg(target_arch = "riscv64")]
#[path = "arch/riscv64/mod.rs"]
mod arch;

// Global assembly entrypoints (replace external entry/*.S files)
// QEMU는 부팅 시 DTB 주소를 레지스터로 전달:
// - aarch64: x0 = DTB 주소
// - riscv64: a1 = DTB 주소 (a0 = hartid)
#[cfg(target_arch = "aarch64")]
core::arch::global_asm!(
    r#"
    .section .text.boot
    .global _start
    .type _start, %function
_start:
    // Linux ARM64 부트 헤더 (QEMU가 DTB를 x0로 전달하도록 함)
    b       real_start              // 0x00: 점프 명령 (4바이트)
    .long   0                       // 0x04: reserved
    .quad   0x80000                 // 0x08: text_offset (512KB)
    .quad   0                       // 0x10: image_size (0 = 미지정)
    .quad   0                       // 0x18: flags
    .quad   0                       // 0x20: res2
    .quad   0                       // 0x28: res3
    .quad   0                       // 0x30: res4
    .long   0x644d5241              // 0x38: magic ("ARM\x64")
    .long   0                       // 0x3c: res5

real_start:
    // x0 = DTB 주소 (QEMU가 전달) - 먼저 보존!
    mov x20, x0           // DTB 주소를 callee-saved 레지스터에 보관

    // 스택 설정
    ldr x1, =_stack_start
    mov sp, x1
    mov x0, x20           // DTB 주소를 첫 번째 인자로 전달
    bl _entry
9:  b 9b
"#
);

#[cfg(target_arch = "riscv64")]
core::arch::global_asm!(
    r#"
    .section .text.boot
    .global _start
    .type _start, @function
_start:
    // a0 = hartid, a1 = DTB 주소 (OpenSBI/QEMU가 전달)
    
    // Hart 0만 부팅 진행, 나머지는 대기
    csrr t0, mhartid
    bnez t0, park
    
    // Hart 0: DTB 주소 보관
    mv s0, a1
    
    // 스택 포인터 설정
    la sp, _stack_start
    
    // BSS 영역 초기화
    la t0, _bss
    la t1, _ebss
clear_bss:
    beq t0, t1, bss_done
    sd zero, 0(t0)
    addi t0, t0, 8
    j clear_bss

bss_done:
    // DTB 주소를 인자로 전달하고 Rust 코드 호출
    mv a0, s0
    call _entry

park:
    // 다른 하트들은 대기
    wfi
    j park
"#
);

/// 어셈블리 진입점에서 호출되는 Rust 엔트리
/// dtb_addr: QEMU가 전달한 DTB 주소
#[unsafe(no_mangle)]
pub extern "C" fn _entry(dtb_addr: usize) -> ! {
    #[cfg(target_arch = "aarch64")]
    aarch64_start(dtb_addr);

    #[cfg(target_arch = "riscv64")]
    riscv64_start(dtb_addr);

    #[cfg(not(any(target_arch = "aarch64", target_arch = "riscv64")))]
    loop {}
}

#[cfg(target_arch = "aarch64")]
fn aarch64_start(dtb_addr: usize) -> ! {
    // 예외 핸들러 초기화
    arch::init();

    console::puts("kerners booting...\n\n");

    // QEMU virt (aarch64): RAM starts at 0x40000000
    const RAM_START: usize = 0x4000_0000;
    const DEFAULT_RAM_SIZE: usize = 512 * 1024 * 1024; // 512MB

    kprintln!("[boot] DTB address from register x0: {:#x}", dtb_addr);

    // DTB에서 메모리 정보 획득
    let (ram_base, ram_size) = unsafe {
        // DTB 탐색 전략: 레지스터 → 고정 위치들 확인 → 전체 스캔
        let result = if dtb_addr != 0 && is_valid_dtb(dtb_addr) {
            kprintln!("[DTB] Valid DTB found at register address: {:#x}", dtb_addr);
            dtb::init(dtb_addr)
        } else {
            kprintln!("[DTB] Register address invalid or zero");

            // QEMU virt는 일반적으로 DTB를 RAM 시작 주소나 커널 바로 앞에 배치
            let common_locations = [
                RAM_START,           // 0x40000000 - RAM 시작
                RAM_START + 0x8000,  // 0x40008000 - 일반적인 위치
                RAM_START + 0x10000, // 0x40010000
            ];

            let mut found = None;
            for &addr in &common_locations {
                kprintln!("[DTB] Checking common location: {:#x}", addr);
                if is_valid_dtb(addr) {
                    kprintln!("[DTB] Found valid DTB at: {:#x}", addr);
                    found = Some(addr);
                    break;
                }
            }

            if let Some(addr) = found {
                dtb::init(addr)
            } else {
                kprintln!("[DTB] Not found at common locations, scanning memory...");
                dtb::init_scan(RAM_START, DEFAULT_RAM_SIZE)
            }
        };

        match result {
            Ok(()) => {
                if let Some(dt) = dtb::get() {
                    dt.dump_info();
                    // NOTE: dump_devices()는 힙 할당이 필요하므로 mm::init() 이후에 호출

                    match dt.get_memory() {
                        Ok(mem) => {
                            kprintln!(
                                "[DTB] Memory: base={:#x}, size={:#x} ({} MB)",
                                mem.base,
                                mem.size,
                                mem.size / (1024 * 1024)
                            );
                            (mem.base as usize, mem.size as usize)
                        }
                        Err(_) => {
                            kprintln!("[DTB] Warning: Could not find memory node, using defaults");
                            (RAM_START, DEFAULT_RAM_SIZE)
                        }
                    }
                } else {
                    (RAM_START, DEFAULT_RAM_SIZE)
                }
            }
            Err(_) => {
                kprintln!("[DTB] Warning: Failed to find/parse DTB, using defaults");
                (RAM_START, DEFAULT_RAM_SIZE)
            }
        }
    };

    // 메모리 관리 시스템 초기화
    kprintln!("");
    match mm::init(ram_base, ram_size) {
        Ok(_layout) => {
            // 로깅 시스템 초기화 (힙 사용 가능 후)
            log::init();

            // 힙 초기화 완료 후 DTB 디바이스 정보 출력 (디버깅용)
            if let Some(dt) = dtb::get() {
                dt.dump_devices();
            }

            // 보드 모듈 시스템 초기화 (DTB compatible 기반 보드 선택)
            init_board_system();

            // 플랫폼 설정 탐색 (DTB 기반 + BoardConfig 폴백)
            let platform_config = drivers::probe::probe_platform();
            drivers::config::init_platform_config(platform_config);

            // MMU 초기화
            match arch::mmu::init(ram_base, ram_size) {
                Ok(()) => {
                    // GIC 초기화
                    match arch::gic::init() {
                        Ok(()) => {
                            // 타이머 초기화
                            match arch::timer::init() {
                                Ok(()) => {
                                    // IRQ 활성화
                                    unsafe {
                                        enable_irq();
                                    }

                                    // 메모리 할당 테스트
                                    test_memory_allocation();

                                    // 프로세스 서브시스템 초기화
                                    proc::init();

                                    // VFS 초기화
                                    init_vfs();

                                    // VirtIO 서브시스템 초기화
                                    virtio::init();

                                    // 블록 서브시스템 초기화 (VirtIO 블록 드라이버 포함)
                                    block::init();

                                    // 블록 디바이스를 DevFS에 등록 (/dev/vda 등)
                                    fs::devfs::register_block_devices_to_devfs();

                                    // 테스트: 모듈 로드
                                    #[cfg(feature = "embed_test_module")]
                                    {
                                        kprintln!("\n[test] Loading hello_module...");
                                        match module::ModuleLoader::load_from_path("/modules/hello_module.ko") {
                                            Ok(m) => kprintln!("[test] Module '{}' loaded at 0x{:x}", m.info.name, m.base_addr),
                                            Err(e) => kprintln!("[test] Failed to load module: {:?}", e),
                                        }
                                    }

                                    // SMP 부팅 (secondary CPUs 시작)
                                    start_smp();

                                    kprintln!("\n[boot] Initialization complete!");

                                    #[cfg(feature = "test_runner")]
                                    test_runner::run_kernel_tests();

                                    #[cfg(not(feature = "test_runner"))]
                                    {
                                        kprintln!("Welcome to kerners shell!");
                                        kprintln!("Commands: help, meminfo, uptime, echo <text>");
                                        simple_shell();
                                    }
                                }
                                Err(e) => {
                                    kprintln!("[boot] ERROR: Timer init failed: {}", e);
                                }
                            }
                        }
                        Err(e) => {
                            kprintln!("[boot] ERROR: GIC init failed: {}", e);
                        }
                    }
                }
                Err(e) => {
                    kprintln!("[boot] ERROR: MMU init failed: {}", e);
                }
            }
        }
        Err(e) => {
            kprintln!("[boot] ERROR: Memory management init failed: {}", e);
        }
    }

    kprintln!("\n[boot] Initialization complete!");

    loop {}
}

/// 파일에 텍스트 쓰기 (echo 리다이렉션용)
fn echo_to_file(path: &str, text: &str, append: bool) -> Result<(), &'static str> {
    use fs::{VNodeType, FileMode};

    // 경로 파싱
    let normalized = fs::path::normalize(path).map_err(|_| "invalid path")?;
    let (parent_path, filename) = if let Some(pos) = normalized.rfind('/') {
        if pos == 0 {
            (alloc::string::String::from("/"), alloc::string::String::from(&normalized[1..]))
        } else {
            (alloc::string::String::from(&normalized[..pos]), alloc::string::String::from(&normalized[pos + 1..]))
        }
    } else {
        (alloc::string::String::from("."), normalized.clone())
    };

    // 파일이 없으면 생성
    let file = match fs::lookup_path(&normalized) {
        Ok(node) => node,
        Err(fs::VfsError::NotFound) => {
            // 부모 디렉토리 찾기
            let parent = fs::lookup_path(&parent_path).map_err(|_| "parent directory not found")?;
            // 파일 생성
            parent.create(&filename, VNodeType::File, FileMode::default_file())
                .map_err(|_| "failed to create file")?
        }
        Err(_) => return Err("failed to access file"),
    };

    // 파일인지 확인
    if file.node_type() != VNodeType::File {
        return Err("not a file");
    }

    // 쓰기 오프셋 결정
    let offset = if append {
        file.stat().map(|s| s.size as usize).unwrap_or(0)
    } else {
        // Overwrite: 먼저 truncate
        file.truncate(0).map_err(|_| "failed to truncate")?;
        0
    };

    // 텍스트 + 개행 쓰기
    let mut data = alloc::vec::Vec::from(text.as_bytes());
    data.push(b'\n');

    file.write(offset, &data).map_err(|_| "write failed")?;

    Ok(())
}

/// 간단한 쉘
fn simple_shell() -> ! {
    use alloc::string::String;
    use alloc::vec::Vec;

    let mut line = String::new();

    loop {
        // 프롬프트 출력
        console::puts("\nkerners> ");

        // 명령 입력 받기
        line.clear();
        loop {
            // 폴링 방식으로 입력 받기
            if let Some(ch) = arch::uart::getc() {
                if ch == b'\r' || ch == b'\n' {
                    console::puts("\n");
                    break;
                } else if ch == 0x7F || ch == 0x08 {
                    // Backspace
                    if !line.is_empty() {
                        line.pop();
                        // 백스페이스 에코
                        console::puts("\x08 \x08");
                    }
                } else if ch >= 32 && ch < 127 {
                    // 출력 가능한 ASCII 문자
                    line.push(ch as char);
                    // 에코
                    console::putc(ch);
                }
            } else {
                // 입력이 없으면 잠시 대기
                #[cfg(target_arch = "aarch64")]
                unsafe {
                    // AArch64: WFI는 IRQ로 깨어남
                    core::arch::asm!("wfi");
                }
                #[cfg(target_arch = "riscv64")]
                {
                    // RISC-V: 폴링 모드에서는 짧은 스핀 루프 사용
                    // (UART 인터럽트 설정 없이 WFI 사용 시 멈출 수 있음)
                    for _ in 0..1000 {
                        core::hint::spin_loop();
                    }
                }
            }
        }

        // 명령 처리
        let cmd = line.trim();
        if cmd.is_empty() {
            continue;
        }

        let parts: Vec<&str> = cmd.split_whitespace().collect();
        match parts.get(0).map(|s| *s) {
            Some("help") => {
                kprintln!("Available commands:");
                kprintln!("  help     - Show this help");
                kprintln!("  meminfo  - Show memory information");
                kprintln!("  uptime   - Show system uptime");
                kprintln!("  boardinfo - Show active board information");
                kprintln!("  lsboards - List registered boards");
                kprintln!("  threads  - Show thread list");
                kprintln!("  spawn    - Spawn a test thread");
                kprintln!("  usertest - Test user mode");
                kprintln!("  mqtest   - Test message queue");
                kprintln!("  modtest  - Test module loader");
                kprintln!("  lsmod    - List loaded modules");
                kprintln!("  insmod <path> - Load module from path");
                kprintln!("  rmmod <name> - Unload a module");
                kprintln!("  ls [path] - List directory contents");
                kprintln!("  cat <path> - Display file contents");
                kprintln!("  write <path> <text> - Write text to file");
                kprintln!("  echo <text> [> file] - Echo text (optionally to file)");
                kprintln!("  blkinfo  - Show block devices");
                kprintln!("  blktest  - Test VirtIO block device");
                kprintln!("  mount    - Mount FAT32 from /dev/vda to /mnt");
                kprintln!("  mounts   - List mount points");
                kprintln!("  cpuinfo  - Show CPU/SMP status");
                kprintln!("  dmesg    - Display kernel ring buffer");
                kprintln!("  loglevel [level] - Set log level (0-4 or ERROR/WARN/INFO/DEBUG/TRACE)");
            }
            Some("meminfo") => {
                mm::heap::print_stats();
                mm::page::print_stats();
            }
            Some("uptime") => {
                let ticks = arch::timer::ticks();
                let seconds = ticks / 100;
                let minutes = seconds / 60;
                let hours = minutes / 60;
                kprintln!(
                    "Uptime: {}h {}m {}s ({} ticks)",
                    hours,
                    minutes % 60,
                    seconds % 60,
                    ticks
                );
            }
            Some("boardinfo") => {
                if let Some(board) = boards::current_board_info() {
                    kprintln!("Active board: {}", board.name);
                    kprintln!("  Compatible: {:?}", board.compatible);
                    kprintln!("  Timer freq: {} Hz", board.timer_freq);
                    kprintln!("  UART quirks: {:#x}", board.uart_quirks);
                    kprintln!("  SMP capable: {}", if board.smp_capable { "yes" } else { "no" });
                    if board.cpu_count > 0 {
                        kprintln!("  CPU count: {}", board.cpu_count);
                    } else {
                        // DTB에서 CPU 개수 읽기
                        let cpu_count = dtb::get().map(|dt| dt.count_cpus()).unwrap_or(1);
                        kprintln!("  CPU count: {} (from DTB)", cpu_count);
                    }
                } else {
                    kprintln!("No active board module");
                    kprintln!("Using compile-time defaults (BoardConfig)");
                }
            }
            Some("lsboards") => {
                kprintln!("Registered board modules:");
                boards::registry::for_each_board(|name, is_active| {
                    let marker = if is_active { "*" } else { " " };
                    kprintln!("  {} {}", marker, name);
                });
                kprintln!("");
                kprintln!("Total: {} board(s) (* = active)", boards::registry::board_count());
            }
            Some("threads") => {
                proc::dump_threads();
            }
            Some("cpuinfo") => {
                let total = proc::percpu::total_count();
                let online = proc::percpu::online_count();
                let my_cpu = proc::percpu::get_cpu_id();
                kprintln!("CPU info:");
                kprintln!("  Total CPUs: {}", total);
                kprintln!("  Online CPUs: {}", online);
                kprintln!("  Current CPU: {}", my_cpu);
                for cpu in 0..total {
                    let pc = proc::percpu::get(cpu);
                    let status = if pc.is_online() { "online" } else { "offline" };
                    let ticks = pc.tick_count.load(core::sync::atomic::Ordering::Relaxed);
                    kprintln!("  CPU {}: {} (ticks: {})", cpu, status, ticks);
                }
            }
            Some("spawn") => {
                static THREAD_COUNT: core::sync::atomic::AtomicU64 = 
                    core::sync::atomic::AtomicU64::new(1);
                let n = THREAD_COUNT.fetch_add(1, core::sync::atomic::Ordering::SeqCst);
                let name = alloc::format!("test-{}", n);
                let tid = proc::spawn(&name, test_thread_entry);
                kprintln!("Spawned thread '{}' (tid={})", name, tid);
            }
            Some("usertest") => {
                proc::user::test_user_mode();
            }
            Some("mqtest") => {
                test_message_queue();
            }
            Some("modtest") => {
                test_module_loader();
            }
            Some("lsmod") => {
                let modules = module::ModuleLoader::list();
                if modules.is_empty() {
                    kprintln!("No modules loaded");
                } else {
                    kprintln!("Loaded modules:");
                    for name in modules {
                        kprintln!("  - {}", name);
                    }
                }
            }
            Some("rmmod") => {
                if parts.len() > 1 {
                    match module::ModuleLoader::unload(parts[1]) {
                        Ok(()) => kprintln!("Module '{}' unloaded", parts[1]),
                        Err(e) => kprintln!("Failed to unload: {:?}", e),
                    }
                } else {
                    kprintln!("Usage: rmmod <module_name>");
                }
            }
            Some("insmod") => {
                if parts.len() > 1 {
                    match module::ModuleLoader::load_from_path(parts[1]) {
                        Ok(m) => kprintln!("Module '{}' loaded at 0x{:x}", m.info.name, m.base_addr),
                        Err(e) => kprintln!("Failed to load module: {:?}", e),
                    }
                } else {
                    kprintln!("Usage: insmod <path>");
                    kprintln!("Example: insmod /modules/hello_module.ko");
                }
            }
            Some("ls") => {
                let path = if parts.len() > 1 { parts[1] } else { "/" };
                match fs::lookup_path(path) {
                    Ok(node) => {
                        match node.readdir() {
                            Ok(entries) => {
                                kprintln!("Directory: {}", path);
                                for entry in entries {
                                    let type_char = match entry.node_type {
                                        fs::VNodeType::File => '-',
                                        fs::VNodeType::Directory => 'd',
                                        fs::VNodeType::Symlink => 'l',
                                        fs::VNodeType::CharDevice => 'c',
                                        fs::VNodeType::BlockDevice => 'b',
                                        fs::VNodeType::Fifo => 'p',
                                        fs::VNodeType::Socket => 's',
                                    };
                                    kprintln!("  {} {}", type_char, entry.name);
                                }
                            }
                            Err(e) => kprintln!("Failed to read directory: {:?}", e),
                        }
                    }
                    Err(e) => kprintln!("Path not found: {:?}", e),
                }
            }
            Some("cat") => {
                if parts.len() > 1 {
                    match fs::lookup_path(parts[1]) {
                        Ok(node) => {
                            let stat = node.stat().unwrap_or_default();
                            let size = core::cmp::min(stat.size as usize, 4096);
                            let mut buffer = alloc::vec![0u8; size];
                            match node.read(0, &mut buffer) {
                                Ok(n) => {
                                    if let Ok(s) = core::str::from_utf8(&buffer[..n]) {
                                        console::puts(s);
                                        if !s.ends_with('\n') {
                                            console::puts("\n");
                                        }
                                    } else {
                                        kprintln!("<binary data: {} bytes>", n);
                                    }
                                }
                                Err(e) => kprintln!("Failed to read file: {:?}", e),
                            }
                        }
                        Err(e) => kprintln!("File not found: {:?}", e),
                    }
                } else {
                    kprintln!("Usage: cat <path>");
                }
            }
            Some("echo") => {
                if parts.len() > 1 {
                    let args = &parts[1..].join(" ");

                    // 리다이렉션 처리: echo text > file 또는 echo text >> file
                    if let Some(append_pos) = args.find(">>") {
                        // Append 모드
                        let text = args[..append_pos].trim();
                        let path = args[append_pos + 2..].trim();
                        if !path.is_empty() {
                            match echo_to_file(path, text, true) {
                                Ok(_) => {}
                                Err(e) => kprintln!("echo: {}: {}", path, e),
                            }
                        } else {
                            kprintln!("echo: missing file path after '>>'");
                        }
                    } else if let Some(redir_pos) = args.find('>') {
                        // Overwrite 모드
                        let text = args[..redir_pos].trim();
                        let path = args[redir_pos + 1..].trim();
                        if !path.is_empty() {
                            match echo_to_file(path, text, false) {
                                Ok(_) => {}
                                Err(e) => kprintln!("echo: {}: {}", path, e),
                            }
                        } else {
                            kprintln!("echo: missing file path after '>'");
                        }
                    } else {
                        // 일반 echo (화면 출력)
                        kprintln!("{}", args);
                    }
                }
            }
            Some("blkinfo") => {
                let devices = block::list_devices();
                if devices.is_empty() {
                    kprintln!("No block devices found");
                } else {
                    kprintln!("Block devices:");
                    for name in &devices {
                        if let Some(info) = block::device_info(name) {
                            kprintln!("  {}: {} bytes ({} blocks of {} bytes){}",
                                info.name,
                                info.capacity,
                                info.block_count,
                                info.block_size,
                                if info.read_only { " [RO]" } else { "" }
                            );
                        }
                    }
                }
            }
            Some("blktest") => {
                if let Some(device) = block::get_device("vda") {
                    kprintln!("Testing VirtIO block device 'vda'...");

                    // Read first block
                    let block_size = device.block_size();
                    let mut buf = alloc::vec![0u8; block_size];

                    kprintln!("  Reading block 0...");
                    match device.read_block(0, &mut buf) {
                        Ok(()) => {
                            kprintln!("  Read successful! First 16 bytes:");
                            kprintln!("  {:02x?}", &buf[..16.min(block_size)]);

                            // Check if it's a FAT32 disk
                            if buf.len() >= 3 && (buf[0] == 0xEB || buf[0] == 0xE9) {
                                kprintln!("  Looks like a FAT boot sector!");
                            }
                        }
                        Err(e) => kprintln!("  Read failed: {:?}", e),
                    }

                    // Write test (write to a high block number to avoid damaging FAT)
                    if !device.is_read_only() {
                        let test_block = 1000;
                        let test_data = b"KERNERS_BLK_TEST";
                        let mut write_buf = alloc::vec![0u8; block_size];
                        write_buf[..test_data.len()].copy_from_slice(test_data);

                        kprintln!("  Writing test data to block {}...", test_block);
                        match device.write_block(test_block, &write_buf) {
                            Ok(()) => {
                                kprintln!("  Write successful!");

                                // Read back
                                let mut read_buf = alloc::vec![0u8; block_size];
                                match device.read_block(test_block, &mut read_buf) {
                                    Ok(()) => {
                                        if &read_buf[..test_data.len()] == test_data {
                                            kprintln!("  Verify successful: data matches!");
                                        } else {
                                            kprintln!("  Verify failed: data mismatch");
                                        }
                                    }
                                    Err(e) => kprintln!("  Read-back failed: {:?}", e),
                                }
                            }
                            Err(e) => kprintln!("  Write failed: {:?}", e),
                        }
                    } else {
                        kprintln!("  Device is read-only, skipping write test");
                    }
                } else {
                    kprintln!("VirtIO block device 'vda' not found");
                }
            }
            Some("mount") => {
                // FAT32 파일시스템 마운트
                if let Some(device) = block::get_device("vda") {
                    kprintln!("Mounting FAT32 from /dev/vda...");

                    // /mnt 디렉토리 생성
                    if let Ok(root) = fs::lookup_path("/") {
                        let _ = root.create("mnt", fs::VNodeType::Directory, fs::FileMode::default_dir());
                    }

                    match fs::fat32::mount_fat32(device) {
                        Ok(fat32_fs) => {
                            match fs::mount("/mnt", fat32_fs) {
                                Ok(()) => kprintln!("FAT32 mounted at /mnt"),
                                Err(e) => kprintln!("Mount failed: {:?}", e),
                            }
                        }
                        Err(e) => kprintln!("FAT32 mount failed: {:?}", e),
                    }
                } else {
                    kprintln!("Block device 'vda' not found");
                }
            }
            Some("mounts") => {
                let mounts = fs::list_mounts();
                if mounts.is_empty() {
                    kprintln!("No filesystems mounted");
                } else {
                    kprintln!("Mount points:");
                    for (path, fs_type) in mounts {
                        kprintln!("  {} -> {}", path, fs_type);
                    }
                }
            }
            Some("write") => {
                // Usage: write <path> <content>
                if parts.len() > 2 {
                    let path = parts[1];
                    let content = parts[2..].join(" ");

                    // 파일이 없으면 생성
                    let node = match fs::lookup_path(path) {
                        Ok(n) => n,
                        Err(_) => {
                            // 부모 디렉토리 찾기
                            let parent_path = if let Some(pos) = path.rfind('/') {
                                if pos == 0 { "/" } else { &path[..pos] }
                            } else {
                                "/"
                            };
                            let file_name = path.rsplit('/').next().unwrap_or(path);

                            match fs::lookup_path(parent_path) {
                                Ok(parent) => {
                                    match parent.create(file_name, fs::VNodeType::File, fs::FileMode::default_file()) {
                                        Ok(n) => n,
                                        Err(e) => {
                                            kprintln!("Failed to create file: {:?}", e);
                                            continue;
                                        }
                                    }
                                }
                                Err(e) => {
                                    kprintln!("Parent directory not found: {:?}", e);
                                    continue;
                                }
                            }
                        }
                    };

                    // 파일에 쓰기
                    match node.write(0, content.as_bytes()) {
                        Ok(n) => kprintln!("Wrote {} bytes to {}", n, path),
                        Err(e) => kprintln!("Write failed: {:?}", e),
                    }
                } else {
                    kprintln!("Usage: write <path> <content>");
                    kprintln!("Example: write /test.txt Hello World");
                }
            }
            Some("dmesg") => {
                log::dump_logs();
            }
            Some("loglevel") => {
                if parts.len() == 1 {
                    let level = log::get_log_level();
                    kprintln!("Current log level: {} ({})", level as u8, level);
                } else if let Some(level) = log::LogLevel::from_str(parts[1]) {
                    log::set_log_level(level);
                    kprintln!("Log level set to: {} ({})", level as u8, level);
                } else {
                    kprintln!("Invalid log level. Use: 0-4 or ERROR/WARN/INFO/DEBUG/TRACE");
                }
            }
            Some(unknown) => {
                kprintln!("Unknown command: {}", unknown);
                kprintln!("Type 'help' for available commands.");
            }
            None => {}
        }
    }
}

/// 테스트용 스레드 엔트리 함수
fn test_thread_entry() -> ! {
    let tid = proc::current_tid().unwrap_or(0);
    kprintln!("[thread {}] Started!", tid);
    
    for i in 0..5 {
        kprintln!("[thread {}] Running iteration {}", tid, i);
        // 잠시 대기 후 yield
        for _ in 0..1_000_000 {
            core::hint::spin_loop();
        }
        proc::yield_now();
    }
    
    kprintln!("[thread {}] Exiting!", tid);
    proc::exit();
}

/// 모듈 로더 테스트
fn test_module_loader() {
    kprintln!("\n=== Module Loader Tests ===\n");
    
    // 테스트 1: 심볼 테이블
    kprintln!("[test] Kernel symbol table:");
    for (name, addr) in module::symbol::list_symbols() {
        kprintln!("  {} = 0x{:x}", name, addr);
    }
    kprintln!("  Total: {} symbols\n", module::symbol::symbol_count());
    
    // 테스트 2: 내장 테스트 모듈 로드
    kprintln!("[test] Loading builtin test module...");
    match module::loader::builtin::load_test_module() {
        Ok(()) => kprintln!("[test] Builtin module loaded successfully!"),
        Err(e) => kprintln!("[test] Failed to load builtin module: {:?}", e),
    }
    
    // 테스트 3: 로드된 모듈 목록
    kprintln!("\n[test] Loaded modules:");
    for name in module::ModuleLoader::list() {
        kprintln!("  - {}", name);
    }
    
    kprintln!("\n=== Module Loader Tests Complete ===");
}

/// 메시지 큐 테스트
fn test_message_queue() {
    use ipc::message_queue::{MessageQueue, BoundedMessageQueue, Priority, Channel};
    
    kprintln!("\n=== Message Queue Tests ===\n");
    
    // 테스트 1: 기본 MessageQueue
    kprintln!("[Test 1] Basic MessageQueue<i32>");
    {
        let mq: MessageQueue<i32> = MessageQueue::new();
        
        mq.send(10).unwrap();
        mq.send(20).unwrap();
        mq.send(30).unwrap();
        
        kprintln!("  Sent: 10, 20, 30");
        kprintln!("  Queue length: {}", mq.len());
        
        let m1 = mq.try_receive().unwrap();
        let m2 = mq.try_receive().unwrap();
        let m3 = mq.try_receive().unwrap();
        
        kprintln!("  Received: {}, {}, {}", m1.data, m2.data, m3.data);
        kprintln!("  Queue empty: {}", mq.is_empty());
        
        assert_eq!(m1.data, 10);
        assert_eq!(m2.data, 20);
        assert_eq!(m3.data, 30);
        kprintln!("  [PASS]");
    }
    
    // 테스트 2: 우선순위
    kprintln!("\n[Test 2] Priority Queue");
    {
        let mq: MessageQueue<&str> = MessageQueue::new();
        
        mq.send_priority("low", Priority::Low).unwrap();
        mq.send_priority("normal", Priority::Normal).unwrap();
        mq.send_priority("urgent", Priority::Urgent).unwrap();
        mq.send_priority("high", Priority::High).unwrap();
        
        kprintln!("  Sent: low(L), normal(N), urgent(U), high(H)");
        
        // Urgent와 High가 먼저 나와야 함
        let m1 = mq.try_receive().unwrap();
        let m2 = mq.try_receive().unwrap();
        let m3 = mq.try_receive().unwrap();
        let m4 = mq.try_receive().unwrap();
        
        kprintln!("  Received order: {}, {}, {}, {}", m1.data, m2.data, m3.data, m4.data);
        kprintln!("  [PASS]");
    }
    
    // 테스트 3: BoundedMessageQueue
    kprintln!("\n[Test 3] BoundedMessageQueue (capacity=3)");
    {
        let bmq: BoundedMessageQueue<u32> = BoundedMessageQueue::new(3);
        
        bmq.try_send(1).unwrap();
        bmq.try_send(2).unwrap();
        bmq.try_send(3).unwrap();
        
        kprintln!("  Sent 3 items, queue full: {}", bmq.is_full());
        
        // 4번째는 실패해야 함
        let result = bmq.try_send(4);
        kprintln!("  4th send result: {:?}", result);
        assert!(result.is_err());
        
        // 하나 수신 후 다시 송신
        let _ = bmq.try_receive().unwrap();
        bmq.try_send(4).unwrap();
        kprintln!("  After receive, 4th send: OK");
        kprintln!("  [PASS]");
    }
    
    // 테스트 4: 채널
    kprintln!("\n[Test 4] Channel (Go style)");
    {
        let (tx, rx) = Channel::<u64>::bounded(5);
        
        tx.send(100).unwrap();
        tx.send(200).unwrap();
        
        let m1 = rx.try_recv().unwrap();
        let m2 = rx.try_recv().unwrap();
        
        kprintln!("  Sent via tx: 100, 200");
        kprintln!("  Received via rx: {}, {}", m1.data, m2.data);
        kprintln!("  [PASS]");
    }
    
    // 테스트 5: POSIX mq 스타일 API
    kprintln!("\n[Test 5] POSIX mq_* API");
    {
        use ipc::message_queue::{mq_open, mq_send, mq_receive, mq_unlink};
        
        let _ = mq_open("/test_queue", true).unwrap();
        
        mq_send("/test_queue", b"Hello").unwrap();
        mq_send("/test_queue", b"World").unwrap();
        
        let msg1 = mq_receive("/test_queue").unwrap();
        let msg2 = mq_receive("/test_queue").unwrap();
        
        kprintln!("  Sent: 'Hello', 'World'");
        kprintln!("  Received: '{}', '{}'", 
            core::str::from_utf8(&msg1).unwrap_or("?"),
            core::str::from_utf8(&msg2).unwrap_or("?"));
        
        mq_unlink("/test_queue").unwrap();
        kprintln!("  Queue unlinked");
        kprintln!("  [PASS]");
    }
    
    kprintln!("\n=== All Message Queue Tests Passed! ===\n");
}

/// IRQ 활성화 (DAIF 레지스터 조작)
#[cfg(target_arch = "aarch64")]
unsafe fn enable_irq() {
    kprintln!("[boot] Enabling IRQ...");
    unsafe {
        core::arch::asm!("msr DAIFClr, #2"); // IRQ 마스크 해제 (bit 1)
    }
}

#[cfg(target_arch = "riscv64")]
unsafe fn enable_irq() {
    // TODO: riscv64 IRQ 활성화
}

/// 메모리 할당 테스트
fn test_memory_allocation() {
    use alloc::boxed::Box;
    use alloc::string::String;
    use alloc::vec::Vec;

    kprintln!("\n[test] Testing memory allocation...");

    // 1. Box 할당 테스트
    let boxed_value = Box::new(42u64);
    kprintln!(
        "[test] Box<u64> allocated: value={}, addr={:p}",
        *boxed_value,
        &*boxed_value
    );

    // 2. Vec 할당 테스트
    let mut vec: Vec<u32> = Vec::with_capacity(10);
    for i in 0..10 {
        vec.push(i * 2);
    }
    kprintln!(
        "[test] Vec<u32> allocated: len={}, capacity={}",
        vec.len(),
        vec.capacity()
    );

    // 3. String 할당 테스트
    let s = String::from("Hello, kerners!");
    kprintln!("[test] String allocated: '{}', len={}", s, s.len());

    // 4. 페이지 프레임 할당 테스트
    if let Some(frame1) = mm::page::alloc_frame() {
        kprintln!("[test] Page frame allocated: {:#x}", frame1);

        if let Some(frames) = mm::page::alloc_frames(4) {
            kprintln!("[test] 4 contiguous frames allocated: {:#x}", frames);

            // 해제
            unsafe {
                mm::page::free_frames(frames, 4);
            }
            kprintln!("[test] 4 frames freed");
        }

        unsafe {
            mm::page::free_frame(frame1);
        }
        kprintln!("[test] 1 frame freed");
    }

    // 5. 통계 출력
    kprintln!("");
    mm::heap::dump_stats();
    mm::page::dump_stats();

    kprintln!("[test] Memory allocation tests passed!");
}

/// VFS 및 파일시스템 초기화
fn init_vfs() {
    use alloc::sync::Arc;

    kprintln!("\n[vfs] Initializing Virtual File System...");

    // VFS 초기화
    fs::init();

    // RamFS를 루트 파일시스템으로 설정
    let ramfs = fs::ramfs::create_ramfs();
    fs::set_root_fs(ramfs.clone());
    kprintln!("[vfs] RamFS mounted as root (/)");

    // DevFS를 /dev에 마운트
    let devfs = fs::devfs::create_devfs();

    // 먼저 루트에 dev 디렉토리 생성
    if let Ok(root) = fs::lookup_path("/") {
        let _ = root.create("dev", fs::VNodeType::Directory, fs::FileMode::default_dir());
    }

    if let Err(e) = fs::mount("/dev", devfs) {
        kprintln!("[vfs] Warning: Failed to mount /dev: {:?}", e);
    } else {
        kprintln!("[vfs] DevFS mounted at /dev");
    }

    // /modules 디렉토리 생성 및 내장 모듈 복사
    if let Ok(root) = fs::lookup_path("/") {
        if root.create("modules", fs::VNodeType::Directory, fs::FileMode::default_dir()).is_ok() {
            kprintln!("[vfs] Created /modules directory");
            
            // 빌드 시 임베드된 테스트 모듈이 있으면 복사
            #[cfg(feature = "embed_test_module")]
            {
                static TEST_MODULE_DATA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/hello_module.ko"));
                
                if let Ok(modules_dir) = fs::lookup_path("/modules") {
                    if let Ok(file) = modules_dir.create("hello_module.ko", fs::VNodeType::File, fs::FileMode::default_file()) {
                        match file.write(0, TEST_MODULE_DATA) {
                            Ok(n) => kprintln!("[vfs] Copied hello_module.ko ({} bytes) to /modules", n),
                            Err(e) => kprintln!("[vfs] Failed to write module: {:?}", e),
                        }
                    }
                }
            }
            
            // Test lookup without embed feature
            #[cfg(not(feature = "embed_test_module"))]
            {
                // Verify /modules is accessible
                if fs::lookup_path("/modules").is_ok() {
                    kprintln!("[vfs] /modules directory ready");
                }
            }
        }
    }

    // 테스트: /dev 내용 확인
    if let Ok(dev) = fs::lookup_path("/dev") {
        if let Ok(entries) = dev.readdir() {
            kprintln!("[vfs] /dev contents: {} entries", entries.len());
            for entry in entries.iter().take(5) {
                kprintln!("  - {}: {:?}", entry.name, entry.node_type);
            }
        }
    }

    // 콘솔 디바이스로 FD 테이블 초기화
    if let Ok(console) = fs::lookup_path("/dev/console") {
        fs::fd::init_kernel_fd_table(console);
        kprintln!("[vfs] Kernel FD table initialized (stdin/stdout/stderr -> /dev/console)");
    }

    kprintln!("[vfs] VFS initialization complete!");
}

/// SMP 부팅: Per-CPU 초기화 + Secondary CPU/hart 시작
fn start_smp() {
    let cpu_count = drivers::config::cpu_count();
    if cpu_count <= 1 {
        kprintln!("\n[smp] Single CPU detected, skipping SMP boot");
        return;
    }

    kprintln!("\n[smp] Starting SMP with {} CPUs...", cpu_count);

    // 1. Per-CPU 총 CPU 수 업데이트 (proc::init()에서 이미 1로 초기화됨)
    proc::percpu::set_total_cpu_count(cpu_count as u32);

    // 2. Secondary CPU 스택 할당
    proc::percpu::stacks::allocate_secondary_stacks(cpu_count as u32);

    // 3. 아키텍처별 secondary CPU/hart 시작
    #[cfg(target_arch = "aarch64")]
    {
        let entry = boards::qemu_virt_aarch64_smp::secondary_cpu_entry as usize;
        boards::qemu_virt_aarch64_smp::start_secondary_cpus(cpu_count, entry);
    }

    #[cfg(target_arch = "riscv64")]
    {
        let entry = boards::qemu_virt_riscv64_smp::secondary_hart_entry as usize;
        boards::qemu_virt_riscv64_smp::start_secondary_harts(cpu_count, entry);
    }

    // 4. Secondary CPU들이 온라인될 때까지 대기 (최대 ~100ms)
    for _ in 0..1000u32 {
        if proc::percpu::online_count() as usize >= cpu_count {
            break;
        }
        // 짧은 대기
        for _ in 0..10000u32 {
            core::hint::spin_loop();
        }
    }

    let online = proc::percpu::online_count();
    kprintln!("[smp] {}/{} CPUs online", online, cpu_count);
}

#[cfg(target_arch = "riscv64")]
fn riscv64_start(dtb_addr: usize) -> ! {
    // UART 초기화 (가장 먼저!)
    arch::uart::init();
    
    // 예외 핸들러 초기화
    arch::init();

    console::puts("kerners booting...\n\n");

    // QEMU virt (riscv64): RAM starts at 0x80000000
    const RAM_START: usize = 0x8000_0000;
    const DEFAULT_RAM_SIZE: usize = 512 * 1024 * 1024; // 512MB

    kprintln!("[boot] DTB address from register: {:#x}", dtb_addr);

    // DTB에서 메모리 정보 획득
    let (ram_base, ram_size) = unsafe {
        let result = if dtb_addr != 0 && is_valid_dtb(dtb_addr) {
            kprintln!("[DTB] Using address from register: {:#x}", dtb_addr);
            dtb::init(dtb_addr)
        } else {
            kprintln!("[DTB] Register address invalid, scanning memory...");
            dtb::init_scan(RAM_START, DEFAULT_RAM_SIZE)
        };

        match result {
            Ok(()) => {
                if let Some(dt) = dtb::get() {
                    dt.dump_info();

                    match dt.get_memory() {
                        Ok(mem) => {
                            kprintln!(
                                "[DTB] Memory: base={:#x}, size={:#x} ({} MB)",
                                mem.base,
                                mem.size,
                                mem.size / (1024 * 1024)
                            );
                            (mem.base as usize, mem.size as usize)
                        }
                        Err(_) => {
                            kprintln!("[DTB] Warning: Could not find memory node, using defaults");
                            (RAM_START, DEFAULT_RAM_SIZE)
                        }
                    }
                } else {
                    (RAM_START, DEFAULT_RAM_SIZE)
                }
            }
            Err(_) => {
                kprintln!("[DTB] Warning: Failed to find/parse DTB, using defaults");
                (RAM_START, DEFAULT_RAM_SIZE)
            }
        }
    };

    // 메모리 관리 초기화
    kprintln!("");
    match mm::init(ram_base, ram_size) {
        Ok(_layout) => {
            // 로깅 시스템 초기화 (힙 사용 가능 후)
            log::init();

            // 보드 모듈 시스템 초기화 (DTB compatible 기반 보드 선택)
            init_board_system();

            // 플랫폼 설정 탐색 (DTB 기반 + BoardConfig 폴백)
            let platform_config = drivers::probe::probe_platform();
            drivers::config::init_platform_config(platform_config);

            // MMU 초기화
            match arch::mmu::init(ram_base, ram_size) {
                Ok(()) => {
                    // PLIC 초기화
                    match arch::plic::init() {
                        Ok(()) => {
                            // 타이머 초기화
                            match arch::timer::init() {
                                Ok(()) => {
                                    // 인터럽트 활성화
                                    unsafe {
                                        enable_irq_riscv();
                                    }

                                    // 메모리 할당 테스트
                                    test_memory_allocation();

                                    // 프로세스 서브시스템 초기화
                                    proc::init();

                                    // VFS 초기화
                                    init_vfs();

                                    // VirtIO 서브시스템 초기화
                                    virtio::init();

                                    // 블록 서브시스템 초기화 (VirtIO 블록 드라이버 포함)
                                    block::init();

                                    // 블록 디바이스를 DevFS에 등록 (/dev/vda 등)
                                    fs::devfs::register_block_devices_to_devfs();

                                    // SMP 부팅 (secondary harts 시작)
                                    start_smp();

                                    kprintln!("\n[boot] Initialization complete!");

                                    #[cfg(feature = "test_runner")]
                                    test_runner::run_kernel_tests();

                                    #[cfg(not(feature = "test_runner"))]
                                    {
                                        kprintln!("Welcome to kerners shell!");
                                        kprintln!("Commands: help, meminfo, uptime, echo <text>");
                                        simple_shell();
                                    }
                                }
                                Err(e) => {
                                    kprintln!("[boot] ERROR: Timer init failed: {}", e);
                                }
                            }
                        }
                        Err(e) => {
                            kprintln!("[boot] ERROR: PLIC init failed: {}", e);
                        }
                    }
                }
                Err(e) => {
                    kprintln!("[boot] ERROR: MMU init failed: {}", e);
                }
            }
        }
        Err(e) => {
            kprintln!("[boot] ERROR: Memory management init failed: {}", e);
        }
    }

    kprintln!("\n[boot] Initialization complete!");

    loop {}
}

/// IRQ 활성화 (riscv64)
#[cfg(target_arch = "riscv64")]
unsafe fn enable_irq_riscv() {
    kprintln!("[boot] Enabling interrupts...");
    unsafe {
        // mstatus.MIE = 1 (Machine Interrupt Enable)
        core::arch::asm!("li t0, 0x8", "csrs mstatus, t0");
    }
}

/// 보드 모듈 시스템 초기화
///
/// DTB에서 root compatible 속성을 읽어 적절한 보드 모듈을 선택합니다.
fn init_board_system() {
    use alloc::vec::Vec;

    // DTB에서 root compatible 읽기
    let compatibles: Option<Vec<&str>> = dtb::get().map(|dt| {
        let compat_strings = dt.get_root_compatible();
        // String -> &'static str 변환을 위해 leak 사용 (부팅 시 한 번만 호출)
        compat_strings
            .into_iter()
            .map(|s| -> &'static str {
                let leaked: &'static str = alloc::boxed::Box::leak(s.into_boxed_str());
                leaked
            })
            .collect()
    });

    // CPU 개수 확인 (SMP 보드 선택에 사용)
    let cpu_count = dtb::get().map(|dt| dt.count_cpus()).unwrap_or(1);

    kprintln!("[board] Detected {} CPU(s)", cpu_count);

    // 보드 레지스트리 초기화
    if let Some(ref compats) = compatibles {
        kprintln!("[board] DTB compatible: {:?}", compats);
        boards::init_early(Some(compats.as_slice()));
    } else {
        kprintln!("[board] No DTB compatible found, using defaults");
        boards::init_early(None);
    }

    // 보드 초기화 함수 호출
    if let Err(e) = boards::init() {
        kprintln!("[board] Warning: Board init failed with error {}", e);
    }

    // 선택된 보드 정보 출력
    if let Some(board) = boards::current_board_info() {
        kprintln!("[board] Active board: {}", board.name);
        if board.smp_capable {
            kprintln!("[board] SMP capable: yes");
        }
    }
}

/// DTB 매직 넘버 검증
/// DTB 주소가 유효한지 확인 (0xd00dfeed 매직 넘버)
fn is_valid_dtb(addr: usize) -> bool {
    if addr == 0 {
        return false;
    }
    // DTB 주소가 정렬되어 있는지 확인
    if addr % 4 != 0 {
        return false;
    }
    unsafe {
        let magic = u32::from_be((addr as *const u32).read_volatile());
        magic == 0xd00dfeed
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    kprintln!("Kernels panic: {}\n", _info);
    #[cfg(feature = "test_runner")]
    {
        kprintln!("TEST_STATUS: FAIL");
        test_runner::qemu_exit(1);
    }
    #[cfg(not(feature = "test_runner"))]
    loop {}
}
