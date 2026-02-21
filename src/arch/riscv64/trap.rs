//! RISC-V 64-bit Trap Handling
//!
//! Trap Handler 설정 및 예외/인터럽트 처리
//! 
//! RISC-V에서 trap은 다음을 포함:
//! - Exceptions: 동기적 이벤트 (illegal instruction, page fault 등)
//! - Interrupts: 비동기적 이벤트 (timer, external 등)

use crate::kprintln;

#[repr(align(16))]
struct TrapStack([u8; 4096]);

static mut TRAP_STACK: TrapStack = TrapStack([0; 4096]);

const MSTATUS_MPP_MASK: u64 = 0x1800;
const MSTATUS_MPP_S: u64 = 0x0800;
const MSTATUS_FS_MASK: u64 = 0x6000;
const MSTATUS_FS_DIRTY: u64 = 0x6000;
const MSTATUS_MPRV: u64 = 1 << 17;
const MSTATUS_SUM: u64 = 1 << 18;

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

#[inline]
fn enter_riscv_syscall_data_access_mode() -> u64 {
    let mut current: u64 = 0;
    unsafe {
        // SAFETY: trap 핸들러(M-mode)에서만 호출하며 mstatus를 임시 조정해
        // 데이터 접근을 S-privilege 기반 주소 변환(MPRV) + SUM 허용으로 수행한다.
        core::arch::asm!("csrr {0}, mstatus", out(reg) current, options(nomem, nostack));
    }
    let updated = (current & !MSTATUS_MPP_MASK) | MSTATUS_MPP_S | MSTATUS_MPRV | MSTATUS_SUM;
    unsafe {
        // SAFETY: 위에서 계산한 유효 mstatus 값으로 현재 trap 처리 구간의 데이터 접근 모드만 바꾼다.
        core::arch::asm!("csrw mstatus, {0}", in(reg) updated, options(nomem, nostack));
    }
    current
}

#[inline]
fn restore_riscv_syscall_data_access_mode(saved_mstatus: u64) {
    unsafe {
        // SAFETY: enter_riscv_syscall_data_access_mode에서 저장한 원래 mstatus를 즉시 복원한다.
        core::arch::asm!(
            "csrw mstatus, {0}",
            in(reg) saved_mstatus,
            options(nomem, nostack)
        );
    }
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
        // 이전 privilege가 U-mode면 복귀 직전에 pending signal 전달
        let from_user = ((ctx.mstatus >> 11) & 0x3) == 0;
        if from_user {
            let saved_mstatus = enter_riscv_syscall_data_access_mode();
            if crate::syscall::apply_pending_sigreturn_riscv64(ctx) {
                restore_riscv_syscall_data_access_mode(saved_mstatus);
                return;
            }
            let _ = crate::syscall::deliver_pending_signal_riscv64(ctx);
            restore_riscv_syscall_data_access_mode(saved_mstatus);
        }
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
            
            let saved_mstatus = if cause == 8 {
                Some(enter_riscv_syscall_data_access_mode())
            } else {
                None
            };

            let ret = if cause == 8 {
                crate::syscall::syscall_handler_riscv64_with_user_context(
                    syscall_num,
                    args,
                    ctx.gpr,
                    ctx.mstatus,
                    ctx.mepc,
                )
            } else {
                crate::syscall::syscall_handler(syscall_num, args)
            };

            if let Some(exec) = crate::syscall::take_exec_transition_for_current() {
                if crate::proc::set_current_user_stack(exec.user_stack) {
                    // execve 성공: 다음 복귀 지점을 새 엔트리로 교체
                    ctx.mepc = exec.entry as u64;
                    ctx.gpr[2] = exec.stack_top as u64; // sp
                    ctx.gpr[10] = exec.argc as u64; // a0
                    ctx.gpr[11] = exec.argv as u64; // a1
                    ctx.gpr[12] = exec.envp as u64; // a2
                    kprintln!(
                        "[syscall] execve applied: entry={:#x}, sp={:#x}, argc={}",
                        exec.entry,
                        exec.stack_top,
                        exec.argc
                    );
                } else {
                    ctx.gpr[10] = crate::syscall::errno::EPERM as u64;
                    ctx.mepc += 4;
                }
            } else {
                ctx.gpr[10] = ret as u64; // 반환값을 a0에 저장
                ctx.mepc += 4; // ecall 다음 명령어로 (ecall은 4바이트)
            }

            let _ = crate::syscall::deliver_pending_signal_riscv64(ctx);

            if cause == 8 {
                // 사용자 코드가 부동소수점 명령을 사용할 수 있도록 FS 상태를 활성화한다.
                ctx.mstatus = (ctx.mstatus & !MSTATUS_FS_MASK) | MSTATUS_FS_DIRTY;
            }

            if let Some(saved) = saved_mstatus {
                restore_riscv_syscall_data_access_mode(saved);
            }
        }
        12 => {
            // Instruction page fault
            kprintln!("\n[EXCEPTION] Instruction page fault");
            print_trap_context(ctx);
            panic!("Instruction page fault at {:#x}, address: {:#x}", ctx.mepc, ctx.mtval);
        }
        13 => {
            // Load page fault
            if crate::syscall::handle_user_page_fault_riscv64(ctx.mtval as usize, cause) {
                return;
            }
            kprintln!("\n[EXCEPTION] Load page fault");
            print_trap_context(ctx);
            panic!("Load page fault at {:#x}, address: {:#x}", ctx.mepc, ctx.mtval);
        }
        15 => {
            // Store page fault
            if crate::syscall::handle_user_page_fault_riscv64(ctx.mtval as usize, cause) {
                return;
            }
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
    // trap 전용 스택으로 전환
    // - 평상시 mscratch: trap stack top
    // - trap 진입 직후 mscratch: 기존 sp(유저/커널)
    csrrw sp, mscratch, sp

    // 컨텍스트 저장을 위한 스택 공간 확보
    // 32 GPRs + mstatus + mepc + mcause + mtval = 36 * 8 = 288 bytes
    addi sp, sp, -288

    // x1-x31 저장 (x0은 항상 0)
    // 주의: x2(sp)는 트랩 진입 시점의 원래 사용자 SP를 저장해야 한다.
    sd x1, 8(sp)
    sd x3, 24(sp)
    sd x4, 32(sp)
    sd x5, 40(sp)
    csrr t0, mscratch
    sd t0, 16(sp)
    // 중첩 trap 대비: 다음 trap 진입 시 사용할 안전한 trap stack top 갱신
    addi t0, sp, 288
    csrw mscratch, t0
    // 커널 Rust 코드 진입 전 gp(x3)를 커널 global pointer로 맞춘다.
    // 사용자 모드 trap에서는 x3에 사용자 gp가 들어올 수 있어 전역 접근이 깨질 수 있다.
    .option push
    .option norelax
    la gp, __global_pointer$
    .option pop
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

    // 중첩 trap 대비 mscratch를 현재 frame top으로 되돌린 뒤, 원래 sp(유저/커널) 복원
    addi x2, sp, 288        // x2 = frame top
    csrw mscratch, x2
    ld x2, -272(x2)         // x2 = saved original sp (frame+16)

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
        let trap_stack_top = {
            // SAFETY: 단일 CPU 부트 단계에서 정적 trap 스택 버퍼 끝 주소를 계산한다.
            let base = core::ptr::addr_of_mut!(TRAP_STACK) as *mut u8;
            unsafe { base.add(core::mem::size_of::<TrapStack>()) as usize }
        };
        // mtvec의 MODE 비트: 0 = Direct, 1 = Vectored
        // Direct 모드 사용 (모든 trap이 같은 주소로)
        core::arch::asm!(
            "csrw mtvec, {0}",
            in(reg) vector,
            options(nomem, nostack)
        );
        core::arch::asm!(
            "csrw mscratch, {0}",
            in(reg) trap_stack_top,
            options(nomem, nostack)
        );

        crate::kprintln!(
            "[riscv64] Trap vector initialized at {:#x} (M-mode, trap_stack={:#x})",
            vector,
            trap_stack_top
        );
    }
}

/// 테스트용: 브레이크포인트 발생
#[allow(dead_code)]
pub fn test_exception() {
    unsafe {
        core::arch::asm!("ebreak");
    }
}
