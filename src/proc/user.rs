//! 유저 모드 지원
//!
//! EL0 (AArch64) / U-mode (RISC-V) 전환 및 유저 프로세스 관리

use alloc::vec::Vec;
use crate::kprintln;

/// 유저 스택 크기 (64KB)
pub const USER_STACK_SIZE: usize = 64 * 1024;

/// 유저 스택 베이스 주소 (가상 주소, 높은 주소에서 시작)
/// 실제로는 물리 메모리를 매핑해야 하지만, 현재는 identity mapping 사용
#[cfg(target_arch = "aarch64")]
pub const USER_STACK_BASE: usize = 0x0000_0000_8000_0000;  // 2GB

#[cfg(target_arch = "riscv64")]
pub const USER_STACK_BASE: usize = 0x0000_0000_C000_0000;  // 3GB

/// 유저 프로세스 구조체
pub struct UserProcess {
    /// 유저 스택 (커널에서 할당)
    pub user_stack: Vec<u8>,
    /// 스택 탑 주소
    pub stack_top: usize,
    /// 엔트리 포인트
    pub entry: usize,
}

impl UserProcess {
    /// 새 유저 프로세스 생성
    pub fn new(entry: usize) -> Self {
        let mut user_stack = Vec::with_capacity(USER_STACK_SIZE);
        user_stack.resize(USER_STACK_SIZE, 0);
        
        // 스택 탑 계산 (16바이트 정렬)
        let stack_top = user_stack.as_ptr() as usize + USER_STACK_SIZE;
        let stack_top = stack_top & !0xF;
        
        kprintln!("[user] Created user process: entry={:#x}, stack_top={:#x}", 
                  entry, stack_top);
        
        UserProcess {
            user_stack,
            stack_top,
            entry,
        }
    }
    
    /// 유저 모드로 전환하여 실행
    /// 
    /// # Safety
    /// 유저 코드가 유효한 주소를 가리켜야 함
    #[cfg(target_arch = "aarch64")]
    pub unsafe fn run(&self) -> ! {
        kprintln!("[user] Switching to EL0...");
        
        // EL0로 전환
        // SPSR_EL1: 0 = EL0t (EL0, SP_EL0 사용)
        // ELR_EL1: 유저 엔트리 포인트
        // SP_EL0: 유저 스택
        unsafe {
            core::arch::asm!(
                // SPSR_EL1 = 0 (EL0t, 모든 인터럽트 활성화)
                "msr spsr_el1, xzr",
                
                // ELR_EL1 = 유저 엔트리 포인트
                "msr elr_el1, {entry}",
                
                // SP_EL0 = 유저 스택
                "msr sp_el0, {sp}",
                
                // EL0로 전환
                "eret",
                entry = in(reg) self.entry,
                sp = in(reg) self.stack_top,
                options(noreturn)
            );
        }
    }
    
    /// 유저 모드로 전환하여 실행 (RISC-V)
    #[cfg(target_arch = "riscv64")]
    pub unsafe fn run(&self) -> ! {
        kprintln!("[user] Switching to U-mode...");
        
        // M-mode에서 U-mode로 전환
        // mstatus.MPP = 0 (U-mode)
        // mepc = 유저 엔트리 포인트
        unsafe {
            core::arch::asm!(
                // mstatus.MPP 클리어 (U-mode로 설정)
                "li t0, 0x1800",        // MPP 비트 마스크 (bits 11-12)
                "csrc mstatus, t0",     // MPP = 0 (U-mode)
                
                // mstatus.MPIE 설정 (mret 후 인터럽트 활성화)
                "li t0, 0x80",          // MPIE 비트
                "csrs mstatus, t0",
                
                // mepc = 유저 엔트리
                "csrw mepc, {entry}",
                
                // 스택 설정
                "mv sp, {sp}",
                
                // U-mode로 전환
                "mret",
                entry = in(reg) self.entry,
                sp = in(reg) self.stack_top,
                options(noreturn)
            );
        }
    }
}

/// 간단한 유저 프로그램 (커널 내에 포함)
/// syscall을 사용하여 "Hello from user mode!" 출력 후 종료
#[cfg(target_arch = "aarch64")]
#[unsafe(naked)]
pub unsafe extern "C" fn simple_user_program() -> ! {
    core::arch::naked_asm!(
        // write(1, message, len)
        "mov x0, #1",           // fd = stdout
        "adr x1, 2f",           // buf = message
        "mov x2, #23",          // len
        "mov x8, #64",          // syscall: write
        "svc #0",
        
        // exit(0)
        "mov x0, #0",           // status = 0
        "mov x8, #93",          // syscall: exit
        "svc #0",
        
        // 도달하면 안 됨
        "1: wfi",
        "b 1b",
        
        // 메시지 데이터
        ".balign 8",
        "2: .ascii \"Hello from user mode!\\n\\0\"",
    );
}

#[cfg(target_arch = "riscv64")]
#[unsafe(naked)]
pub unsafe extern "C" fn simple_user_program() -> ! {
    core::arch::naked_asm!(
        // write(1, message, len)
        "li a0, 1",             // fd = stdout
        "la a1, 2f",            // buf = message
        "li a2, 23",            // len
        "li a7, 64",            // syscall: write
        "ecall",
        
        // exit(0)
        "li a0, 0",             // status = 0
        "li a7, 93",            // syscall: exit
        "ecall",
        
        // 도달하면 안 됨
        "1: wfi",
        "j 1b",
        
        // 메시지 데이터
        ".balign 8",
        "2: .ascii \"Hello from user mode!\\n\\0\"",
    );
}

/// 유저 프로그램을 실행하는 커널 스레드 엔트리
fn user_thread_entry() -> ! {
    let entry = simple_user_program as usize;
    crate::kprintln!("[user] User thread started, entry: {:#x}", entry);
    
    let user_proc = UserProcess::new(entry);
    
    unsafe {
        user_proc.run();
    }
}

/// 유저 프로그램 테스트 실행
/// 별도의 커널 스레드를 생성하여 유저 모드 실행
pub fn test_user_mode() {
    kprintln!("\n[user] Testing user mode...");
    kprintln!("[user] Spawning user thread...");
    
    // 별도 스레드에서 유저 프로그램 실행
    let tid = super::spawn("user-test", user_thread_entry);
    kprintln!("[user] User thread spawned (tid={})", tid);
    kprintln!("[user] The user program will run on next schedule.");
}
