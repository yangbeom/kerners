//! RISC-V 64-bit Trap Handling
//!
//! Trap Handler 설정 및 예외/인터럽트 처리
//! 
//! RISC-V에서 trap은 다음을 포함:
//! - Exceptions: 동기적 이벤트 (illegal instruction, page fault 등)
//! - Interrupts: 비동기적 이벤트 (timer, external 등)

use crate::kprintln;

/// Trap 발생 시 저장되는 CPU 컨텍스트 (M-mode)
#[repr(C)]
pub struct TrapContext {
    /// General purpose registers x0-x31 (x0은 항상 0이지만 정렬을 위해 포함)
    pub gpr: [u64; 32],
    /// Machine Status Register
    pub mstatus: u64,
    /// Machine Exception Program Counter (복귀 주소)
    pub mepc: u64,
    /// Machine Cause Register
    pub mcause: u64,
    /// Machine Trap Value (추가 정보, 예: fault 주소)
    pub mtval: u64,
}

/// scause 레지스터의 예외 코드 해석 (인터럽트가 아닌 경우)
fn exception_cause_to_str(cause: u64) -> &'static str {
    match cause {
        0 => "Instruction address misaligned",
        1 => "Instruction access fault",
        2 => "Illegal instruction",
        3 => "Breakpoint",
        4 => "Load address misaligned",
        5 => "Load access fault",
        6 => "Store/AMO address misaligned",
        7 => "Store/AMO access fault",
        8 => "Environment call from U-mode",
        9 => "Environment call from S-mode",
        // 10-11 reserved
        12 => "Instruction page fault",
        13 => "Load page fault",
        // 14 reserved
        15 => "Store/AMO page fault",
        _ => "Unknown/Reserved",
    }
}

/// scause 레지스터의 인터럽트 코드 해석
fn interrupt_cause_to_str(cause: u64) -> &'static str {
    match cause {
        1 => "Supervisor software interrupt",
        3 => "Machine software interrupt",
        5 => "Supervisor timer interrupt",
        7 => "Machine timer interrupt",
        9 => "Supervisor external interrupt",
        11 => "Machine external interrupt",
        _ => "Unknown/Reserved interrupt",
    }
}

/// Trap 컨텍스트 출력
fn print_trap_context(ctx: &TrapContext) {
    let is_interrupt = (ctx.mcause >> 63) & 1 == 1;
    let cause_code = ctx.mcause & 0x7FFFFFFF_FFFFFFFF;
    
    kprintln!("Trap Context:");
    kprintln!("  mcause:  {:#018x} ({})", ctx.mcause, 
        if is_interrupt { "Interrupt" } else { "Exception" });
    kprintln!("  Cause:   {} (code={})", 
        if is_interrupt { interrupt_cause_to_str(cause_code) } else { exception_cause_to_str(cause_code) },
        cause_code);
    kprintln!("  mepc:    {:#018x}", ctx.mepc);
    kprintln!("  mstatus: {:#018x}", ctx.mstatus);
    kprintln!("  mtval:   {:#018x}", ctx.mtval);
    kprintln!();
    kprintln!("General Purpose Registers:");
    for i in 0..32 {
        if i % 4 == 0 {
            kprintln!();
        }
        kprintln!("  x{:02}: {:#018x}", i, ctx.gpr[i]);
    }
    kprintln!();
}

/// 메인 trap 핸들러 (Rust)
/// 어셈블리 트램폴린에서 호출됨
#[unsafe(no_mangle)]
pub extern "C" fn trap_handler(ctx: &mut TrapContext) {
    let is_interrupt = (ctx.mcause >> 63) & 1 == 1;
    let cause_code = ctx.mcause & 0x7FFFFFFF_FFFFFFFF;

    if is_interrupt {
        handle_interrupt(ctx, cause_code);
    } else {
        handle_exception(ctx, cause_code);
    }
}

/// 인터럽트 처리
fn handle_interrupt(_ctx: &mut TrapContext, cause: u64) {
    match cause {
        1 => {
            // Supervisor Software Interrupt
            handle_software_interrupt();
        }
        3 => {
            // Machine Software Interrupt (IPI via CLINT MSIP)
            handle_software_interrupt();
        }
        5 => {
            // Supervisor Timer Interrupt - Machine 모드에서는 7
            super::timer::handle_irq();
        }
        7 => {
            // Machine Timer Interrupt
            super::timer::handle_irq();
        }
        9 => {
            // Supervisor External Interrupt - Machine 모드에서는 11
            super::plic::handle_irq();
        }
        11 => {
            // Machine External Interrupt
            super::plic::handle_irq();
        }
        _ => {
            kprintln!("\n[INTERRUPT] Unhandled interrupt: cause={}", cause);
        }
    }
}

/// 소프트웨어 인터럽트 (IPI) 처리
fn handle_software_interrupt() {
    // CLINT MSIP 클리어 (자신의 hart)
    let hartid = crate::proc::percpu::get_cpu_id();
    let clint_base = if crate::drivers::config::is_initialized() {
        crate::drivers::config::clint_base()
    } else {
        crate::boards::clint_base()
    };
    unsafe {
        let msip_addr = clint_base + (hartid as usize) * 4;
        core::ptr::write_volatile(msip_addr as *mut u32, 0);
    }

    // Reschedule IPI: 스케줄러 호출
    crate::proc::scheduler::schedule();
}

/// 예외 처리
fn handle_exception(ctx: &mut TrapContext, cause: u64) {
    match cause {
        2 => {
            // Illegal instruction
            kprintln!("\n[EXCEPTION] Illegal instruction");
            print_trap_context(ctx);
            panic!("Illegal instruction at {:#x}", ctx.mepc);
        }
        3 => {
            // Breakpoint (ebreak)
            kprintln!("\n[EXCEPTION] Breakpoint at {:#x}", ctx.mepc);
            print_trap_context(ctx);
            // ebreak 이후 다음 명령으로 진행 (ebreak는 2바이트 또는 4바이트)
            // 압축 명령어가 아닌 경우 4바이트
            // ctx.mepc += 4; // 필요시 mepc 증가
        }
        5 => {
            // Load access fault
            kprintln!("\n[EXCEPTION] Load access fault");
            print_trap_context(ctx);
            panic!("Load access fault at {:#x}, address: {:#x}", ctx.mepc, ctx.mtval);
        }
        7 => {
            // Store access fault
            kprintln!("\n[EXCEPTION] Store access fault");
            print_trap_context(ctx);
            panic!("Store access fault at {:#x}, address: {:#x}", ctx.mepc, ctx.mtval);
        }
        8 | 9 | 11 => {
            // Environment call (U-mode: 8, S-mode: 9, M-mode: 11)
            // 시스템 콜 처리
            let syscall_num = ctx.gpr[17] as usize;  // a7 = x17
            let args = [
                ctx.gpr[10] as usize,  // a0 = x10
                ctx.gpr[11] as usize,  // a1 = x11
                ctx.gpr[12] as usize,  // a2 = x12
                ctx.gpr[13] as usize,  // a3 = x13
                ctx.gpr[14] as usize,  // a4 = x14
                ctx.gpr[15] as usize,  // a5 = x15
            ];
            
            let ret = crate::syscall::syscall_handler(syscall_num, args);
            ctx.gpr[10] = ret as u64;  // 반환값을 a0에 저장
            ctx.mepc += 4;  // ecall 다음 명령어로 (ecall은 4바이트)
        }
        12 => {
            // Instruction page fault
            kprintln!("\n[EXCEPTION] Instruction page fault");
            print_trap_context(ctx);
            panic!("Instruction page fault at {:#x}, address: {:#x}", ctx.mepc, ctx.mtval);
        }
        13 => {
            // Load page fault
            kprintln!("\n[EXCEPTION] Load page fault");
            print_trap_context(ctx);
            panic!("Load page fault at {:#x}, address: {:#x}", ctx.mepc, ctx.mtval);
        }
        15 => {
            // Store page fault
            kprintln!("\n[EXCEPTION] Store page fault");
            print_trap_context(ctx);
            panic!("Store page fault at {:#x}, address: {:#x}", ctx.mepc, ctx.mtval);
        }
        _ => {
            kprintln!("\n[EXCEPTION] Unhandled exception");
            print_trap_context(ctx);
            panic!("Unhandled exception: cause={}", cause);
        }
    }
}

// ============================================================================
// Trap Vector (어셈블리)
// ============================================================================

core::arch::global_asm!(
    r#"
.section .text.trap_vector, "ax"
.balign 4
.global trap_vector
trap_vector:
    // 컨텍스트 저장을 위한 스택 공간 확보
    // 32 GPRs + mstatus + mepc + mcause + mtval = 36 * 8 = 288 bytes
    addi sp, sp, -288

    // x1-x31 저장 (x0은 항상 0)
    sd x1, 8(sp)
    sd x2, 16(sp)
    sd x3, 24(sp)
    sd x4, 32(sp)
    sd x5, 40(sp)
    sd x6, 48(sp)
    sd x7, 56(sp)
    sd x8, 64(sp)
    sd x9, 72(sp)
    sd x10, 80(sp)
    sd x11, 88(sp)
    sd x12, 96(sp)
    sd x13, 104(sp)
    sd x14, 112(sp)
    sd x15, 120(sp)
    sd x16, 128(sp)
    sd x17, 136(sp)
    sd x18, 144(sp)
    sd x19, 152(sp)
    sd x20, 160(sp)
    sd x21, 168(sp)
    sd x22, 176(sp)
    sd x23, 184(sp)
    sd x24, 192(sp)
    sd x25, 200(sp)
    sd x26, 208(sp)
    sd x27, 216(sp)
    sd x28, 224(sp)
    sd x29, 232(sp)
    sd x30, 240(sp)
    sd x31, 248(sp)

    // CSR 레지스터 저장 (M-mode 레지스터 사용)
    csrr t0, mstatus
    csrr t1, mepc
    csrr t2, mcause
    csrr t3, mtval
    sd t0, 256(sp)   // mstatus
    sd t1, 264(sp)   // mepc
    sd t2, 272(sp)   // mcause
    sd t3, 280(sp)   // mtval

    // 핸들러 호출 (a0 = sp = TrapContext 포인터)
    mv a0, sp
    call trap_handler

    // CSR 레지스터 복원
    ld t0, 256(sp)
    ld t1, 264(sp)
    csrw mstatus, t0
    csrw mepc, t1

    // x1-x31 복원
    ld x1, 8(sp)
    ld x2, 16(sp)
    ld x3, 24(sp)
    ld x4, 32(sp)
    ld x5, 40(sp)
    ld x6, 48(sp)
    ld x7, 56(sp)
    ld x8, 64(sp)
    ld x9, 72(sp)
    ld x10, 80(sp)
    ld x11, 88(sp)
    ld x12, 96(sp)
    ld x13, 104(sp)
    ld x14, 112(sp)
    ld x15, 120(sp)
    ld x16, 128(sp)
    ld x17, 136(sp)
    ld x18, 144(sp)
    ld x19, 152(sp)
    ld x20, 160(sp)
    ld x21, 168(sp)
    ld x22, 176(sp)
    ld x23, 184(sp)
    ld x24, 192(sp)
    ld x25, 200(sp)
    ld x26, 208(sp)
    ld x27, 216(sp)
    ld x28, 224(sp)
    ld x29, 232(sp)
    ld x30, 240(sp)
    ld x31, 248(sp)

    // 스택 복원
    addi sp, sp, 288

    // trap에서 복귀 (M-mode)
    mret
"#
);

/// Trap 벡터 초기화
/// mtvec 레지스터에 trap 핸들러 주소 설정 (M-mode)
pub fn init() {
    unsafe extern "C" {
        fn trap_vector();
    }

    unsafe {
        let vector = trap_vector as usize;
        // mtvec의 MODE 비트: 0 = Direct, 1 = Vectored
        // Direct 모드 사용 (모든 trap이 같은 주소로)
        core::arch::asm!(
            "csrw mtvec, {0}",
            in(reg) vector,
            options(nomem, nostack)
        );

        crate::kprintln!("[riscv64] Trap vector initialized at {:#x} (M-mode)", vector);
    }
}

/// 테스트용: 브레이크포인트 발생
#[allow(dead_code)]
pub fn test_exception() {
    unsafe {
        core::arch::asm!("ebreak");
    }
}
