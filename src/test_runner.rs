//! QEMU 테스트 러너
//!
//! `--features test_runner`로 빌드 시 활성화.
//! 셸 대신 FAT32에서 테스트 모듈(.ko)을 로드/실행하고 결과를 리포팅한 뒤
//! QEMU를 종료한다.

use alloc::string::String;
use alloc::vec::Vec;

use crate::{kprintln, block, fs, module};

/// QEMU 종료
///
/// - aarch64: ARM semihosting SYS_EXIT (HLT #0xF000)
/// - riscv64: sifive_test 디바이스 (0x100000)에 write
pub fn qemu_exit(code: u32) -> ! {
    #[cfg(target_arch = "aarch64")]
    {
        // ARM semihosting: SYS_EXIT (0x18)
        // ADP_Stopped_ApplicationExit = 0x20026
        let block: [u64; 2] = [0x20026, code as u64];
        unsafe {
            core::arch::asm!(
                "mov x1, {0}",
                "mov w0, #0x18",
                "hlt #0xF000",
                in(reg) block.as_ptr(),
                options(noreturn),
            );
        }
    }

    #[cfg(target_arch = "riscv64")]
    {
        // sifive_test 디바이스: 0x100000
        // FINISHER_PASS = 0x5555, FINISHER_FAIL = (code << 16) | 0x3333
        let value: u32 = if code == 0 {
            0x5555
        } else {
            (code << 16) | 0x3333
        };
        unsafe {
            core::ptr::write_volatile(0x10_0000 as *mut u32, value);
        }
        loop {
            core::hint::spin_loop();
        }
    }
}

/// FAT32 자동 마운트
fn mount_fat32() -> bool {
    let device = match block::get_device("vda") {
        Some(d) => d,
        None => {
            kprintln!("[test] ERROR: VirtIO block device 'vda' not found");
            return false;
        }
    };

    // /mnt 디렉토리 생성
    if let Ok(root) = fs::lookup_path("/") {
        let _ = root.create("mnt", fs::VNodeType::Directory, fs::FileMode::default_dir());
    }

    match fs::fat32::mount_fat32(device) {
        Ok(fat32_fs) => {
            match fs::mount("/mnt", fat32_fs) {
                Ok(()) => {
                    kprintln!("[test] FAT32 mounted at /mnt");
                    true
                }
                Err(e) => {
                    kprintln!("[test] Mount failed: {:?}", e);
                    false
                }
            }
        }
        Err(e) => {
            kprintln!("[test] FAT32 mount failed: {:?}", e);
            false
        }
    }
}

/// /mnt 에서 test_*.ko 파일 목록 탐색
fn find_test_modules() -> Vec<String> {
    let mut modules = Vec::new();

    let mnt = match fs::lookup_path("/mnt") {
        Ok(n) => n,
        Err(_) => return modules,
    };

    let entries = match mnt.readdir() {
        Ok(e) => e,
        Err(_) => return modules,
    };

    for entry in &entries {
        // FAT32 8.3 이름은 대문자 — 대소문자 무시 비교
        let name_lower = entry.name.to_ascii_lowercase();
        if name_lower.starts_with("test_") && name_lower.ends_with(".ko") {
            let path = alloc::format!("/mnt/{}", entry.name);
            modules.push(path);
        }
    }

    modules.sort();
    modules
}

/// 단일 테스트 모듈 실행
/// 반환: true = pass, false = fail
fn run_test_module(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or("unknown");
    let module_name = name.trim_end_matches(".ko").trim_end_matches(".o");

    kprintln!("[test] Loading {} ...", path);

    match module::ModuleLoader::load_from_path(path) {
        Ok(_module) => {
            kprintln!("[test] {}: module_init() returned 0 → OK", module_name);
            // 언로드
            let _ = module::ModuleLoader::unload(module_name);
            true
        }
        Err(module::ModuleError::InitFailed(code)) => {
            kprintln!("[test] {}: module_init() returned {} → FAIL", module_name, code);
            false
        }
        Err(e) => {
            kprintln!("[test] {}: load failed: {:?} → FAIL", module_name, e);
            false
        }
    }
}

/// 전체 테스트 스위트 실행 → QEMU 종료
pub fn run_kernel_tests() -> ! {
    kprintln!("");
    kprintln!("=== KERNERS TEST SUITE START ===");
    kprintln!("");

    // 1. FAT32 마운트
    if !mount_fat32() {
        kprintln!("TEST_STATUS: FAIL");
        qemu_exit(1);
    }

    // 2. 테스트 모듈 탐색
    let test_modules = find_test_modules();
    if test_modules.is_empty() {
        kprintln!("[test] No test modules found in /mnt/");
        kprintln!("TEST_STATUS: FAIL");
        qemu_exit(1);
    }

    kprintln!("[test] Found {} test module(s)", test_modules.len());
    kprintln!("");

    // 3. 순서대로 실행
    let mut passed = 0u32;
    let mut failed = 0u32;

    for path in &test_modules {
        if run_test_module(path) {
            passed += 1;
        } else {
            failed += 1;
        }
        kprintln!("");
    }

    // 4. 결과 리포팅
    kprintln!("=== KERNERS TEST SUITE END ===");
    kprintln!("RESULT: {} passed, {} failed", passed, failed);

    if failed == 0 {
        kprintln!("TEST_STATUS: PASS");
        qemu_exit(0);
    } else {
        kprintln!("TEST_STATUS: FAIL");
        qemu_exit(1);
    }
}
