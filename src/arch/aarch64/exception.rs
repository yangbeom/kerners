//! AArch64 Exception Handling
//!
//! Exception Vector Table과 핸들러 구현
//! 
//! AArch64는 4가지 종류의 예외가 있음:
//! - Synchronous: 명령어 실행 중 발생 (syscall, page fault 등)
//! - IRQ: 일반 인터럽트
//! - FIQ: 빠른 인터럽트
//! - SError: 시스템 에러 (비동기)

use crate::kprintln;

/// 예외 발생 시 저장되는 CPU 컨텍스트
#[repr(C)]
pub struct ExceptionContext {
    /// General purpose registers x0-x30
    pub gpr: [u64; 31],
    /// Exception Link Register (복귀 주소)
    pub elr: u64,
    /// Saved Program Status Register
    pub spsr: u64,
    /// Exception Syndrome Register
    pub esr: u64,
    /// Fault Address Register
    pub far: u64,
}

/// ESR_EL1의 Exception Class (EC) 필드 해석
fn exception_class_to_str(ec: u64) -> &'static str {
    match ec {
        0b000000 => "Unknown reason",
        0b000001 => "Trapped WFI/WFE",
        0b000111 => "Trapped SIMD/FP",
        0b001110 => "Illegal Execution state",
        0b010101 => "SVC (AArch64)",
        0b100000 => "Instruction Abort (lower EL)",
        0b100001 => "Instruction Abort (same EL)",
        0b100010 => "PC alignment fault",
        0b100100 => "Data Abort (lower EL)",
        0b100101 => "Data Abort (same EL)",
        0b100110 => "SP alignment fault",
        0b101111 => "SError",
        0b110000 => "Breakpoint (lower EL)",
        0b110001 => "Breakpoint (same EL)",
        0b111100 => "BRK (AArch64)",
        _ => "Unknown",
    }
}

/// 예외 컨텍스트 출력
fn print_exception_context(ctx: &ExceptionContext) {
    let ec = (ctx.esr >> 26) & 0x3F;
    let iss = ctx.esr & 0x1FFFFFF;
    
    kprintln!("Exception Context:");
    kprintln!("  ESR_EL1: {:#018x} (EC: {:#08b}, ISS: {:#027b})", ctx.esr, ec, iss);
    kprintln!("  Exception Class: {}", exception_class_to_str(ec));
    kprintln!("  ELR_EL1: {:#018x}", ctx.elr);
    kprintln!("  SPSR_EL1: {:#018x}", ctx.spsr);
    kprintln!("  FAR_EL1: {:#018x}", ctx.far);
    kprintln!();
    kprintln!("General Purpose Registers:");
    for i in 0..31 {
        kprintln!("  x{:02}: {:#018x}", i, ctx.gpr[i]);
    }
    kprintln!();
}

// ============================================================================
// 기본 예외 핸들러 (Rust에서 호출됨)
// ============================================================================

/// Exception Class 코드
const EC_SVC_AARCH64: u64 = 0b010101;  // SVC from AArch64 (syscall)

/// 기본 예외 핸들러
#[unsafe(no_mangle)]
pub extern "C" fn exception_handler(ctx: &mut ExceptionContext, exception_type: u64) {
    let ec = (ctx.esr >> 26) & 0x3F;  // Exception Class
    
    // IRQ 처리 (exception_type % 4 == 1)
    if exception_type % 4 == 1 {
        super::gic::handle_irq();
        return;
    }

    // Synchronous from Lower EL (exception_type == 8)
    // SVC (시스템 콜) 처리
    if exception_type == 8 && ec == EC_SVC_AARCH64 {
        let syscall_num = ctx.gpr[8] as usize;  // x8 = syscall number
        let args = [
            ctx.gpr[0] as usize,  // x0
            ctx.gpr[1] as usize,  // x1
            ctx.gpr[2] as usize,  // x2
            ctx.gpr[3] as usize,  // x3
            ctx.gpr[4] as usize,  // x4
            ctx.gpr[5] as usize,  // x5
        ];
        
        let ret = crate::syscall::syscall_handler(syscall_num, args);
        ctx.gpr[0] = ret as u64;  // 반환값을 x0에 저장
        // elr은 이미 svc 다음 명령어를 가리킴 (자동)
        return;
    }

    // 다른 예외는 정보 출력 후 패닉
    let type_str = match exception_type {
        0 => "Synchronous (Current EL, SP_EL0)",
        1 => "IRQ (Current EL, SP_EL0)",
        2 => "FIQ (Current EL, SP_EL0)",
        3 => "SError (Current EL, SP_EL0)",
        4 => "Synchronous (Current EL, SP_ELx)",
        5 => "IRQ (Current EL, SP_ELx)",
        6 => "FIQ (Current EL, SP_ELx)",
        7 => "SError (Current EL, SP_ELx)",
        8 => "Synchronous (Lower EL, AArch64)",
        9 => "IRQ (Lower EL, AArch64)",
        10 => "FIQ (Lower EL, AArch64)",
        11 => "SError (Lower EL, AArch64)",
        12 => "Synchronous (Lower EL, AArch32)",
        13 => "IRQ (Lower EL, AArch32)",
        14 => "FIQ (Lower EL, AArch32)",
        15 => "SError (Lower EL, AArch32)",
        _ => "Unknown",
    };

    kprintln!("\n[EXCEPTION] {}", type_str);
    print_exception_context(ctx);
    panic!("Unhandled exception");
}

// ============================================================================
// Exception Vector Table (어셈블리)
// ============================================================================

// 컨텍스트 저장/복원을 포함한 완전한 벡터 테이블
core::arch::global_asm!(
    r#"
.section .text.exception_vectors, "ax"
.balign 2048
.global exception_vectors
exception_vectors:

// ============================================================================
// Current EL with SP_EL0 (EL1t) - entries 0-3
// ============================================================================

// Entry 0: Synchronous
.balign 128
    sub sp, sp, #288
    stp x0, x1, [sp, #0]
    stp x2, x3, [sp, #16]
    stp x4, x5, [sp, #32]
    stp x6, x7, [sp, #48]
    stp x8, x9, [sp, #64]
    stp x10, x11, [sp, #80]
    stp x12, x13, [sp, #96]
    stp x14, x15, [sp, #112]
    stp x16, x17, [sp, #128]
    stp x18, x19, [sp, #144]
    stp x20, x21, [sp, #160]
    stp x22, x23, [sp, #176]
    stp x24, x25, [sp, #192]
    stp x26, x27, [sp, #208]
    stp x28, x29, [sp, #224]
    str x30, [sp, #240]
    mrs x0, elr_el1
    mrs x1, spsr_el1
    mrs x2, esr_el1
    mrs x3, far_el1
    stp x0, x1, [sp, #248]
    stp x2, x3, [sp, #264]
    mov x0, sp
    mov x1, #0
    bl exception_handler
    b __exception_restore

// Entry 1: IRQ
.balign 128
    sub sp, sp, #288
    stp x0, x1, [sp, #0]
    stp x2, x3, [sp, #16]
    stp x4, x5, [sp, #32]
    stp x6, x7, [sp, #48]
    stp x8, x9, [sp, #64]
    stp x10, x11, [sp, #80]
    stp x12, x13, [sp, #96]
    stp x14, x15, [sp, #112]
    stp x16, x17, [sp, #128]
    stp x18, x19, [sp, #144]
    stp x20, x21, [sp, #160]
    stp x22, x23, [sp, #176]
    stp x24, x25, [sp, #192]
    stp x26, x27, [sp, #208]
    stp x28, x29, [sp, #224]
    str x30, [sp, #240]
    mrs x0, elr_el1
    mrs x1, spsr_el1
    mrs x2, esr_el1
    mrs x3, far_el1
    stp x0, x1, [sp, #248]
    stp x2, x3, [sp, #264]
    mov x0, sp
    mov x1, #1
    bl exception_handler
    b __exception_restore

// Entry 2: FIQ
.balign 128
    sub sp, sp, #288
    stp x0, x1, [sp, #0]
    stp x2, x3, [sp, #16]
    stp x4, x5, [sp, #32]
    stp x6, x7, [sp, #48]
    stp x8, x9, [sp, #64]
    stp x10, x11, [sp, #80]
    stp x12, x13, [sp, #96]
    stp x14, x15, [sp, #112]
    stp x16, x17, [sp, #128]
    stp x18, x19, [sp, #144]
    stp x20, x21, [sp, #160]
    stp x22, x23, [sp, #176]
    stp x24, x25, [sp, #192]
    stp x26, x27, [sp, #208]
    stp x28, x29, [sp, #224]
    str x30, [sp, #240]
    mrs x0, elr_el1
    mrs x1, spsr_el1
    mrs x2, esr_el1
    mrs x3, far_el1
    stp x0, x1, [sp, #248]
    stp x2, x3, [sp, #264]
    mov x0, sp
    mov x1, #2
    bl exception_handler
    b __exception_restore

// Entry 3: SError
.balign 128
    sub sp, sp, #288
    stp x0, x1, [sp, #0]
    stp x2, x3, [sp, #16]
    stp x4, x5, [sp, #32]
    stp x6, x7, [sp, #48]
    stp x8, x9, [sp, #64]
    stp x10, x11, [sp, #80]
    stp x12, x13, [sp, #96]
    stp x14, x15, [sp, #112]
    stp x16, x17, [sp, #128]
    stp x18, x19, [sp, #144]
    stp x20, x21, [sp, #160]
    stp x22, x23, [sp, #176]
    stp x24, x25, [sp, #192]
    stp x26, x27, [sp, #208]
    stp x28, x29, [sp, #224]
    str x30, [sp, #240]
    mrs x0, elr_el1
    mrs x1, spsr_el1
    mrs x2, esr_el1
    mrs x3, far_el1
    stp x0, x1, [sp, #248]
    stp x2, x3, [sp, #264]
    mov x0, sp
    mov x1, #3
    bl exception_handler
    b __exception_restore

// ============================================================================
// Current EL with SP_ELx (EL1h) - entries 4-7
// ============================================================================

// Entry 4: Synchronous
.balign 128
    sub sp, sp, #288
    stp x0, x1, [sp, #0]
    stp x2, x3, [sp, #16]
    stp x4, x5, [sp, #32]
    stp x6, x7, [sp, #48]
    stp x8, x9, [sp, #64]
    stp x10, x11, [sp, #80]
    stp x12, x13, [sp, #96]
    stp x14, x15, [sp, #112]
    stp x16, x17, [sp, #128]
    stp x18, x19, [sp, #144]
    stp x20, x21, [sp, #160]
    stp x22, x23, [sp, #176]
    stp x24, x25, [sp, #192]
    stp x26, x27, [sp, #208]
    stp x28, x29, [sp, #224]
    str x30, [sp, #240]
    mrs x0, elr_el1
    mrs x1, spsr_el1
    mrs x2, esr_el1
    mrs x3, far_el1
    stp x0, x1, [sp, #248]
    stp x2, x3, [sp, #264]
    mov x0, sp
    mov x1, #4
    bl exception_handler
    b __exception_restore

// Entry 5: IRQ
.balign 128
    sub sp, sp, #288
    stp x0, x1, [sp, #0]
    stp x2, x3, [sp, #16]
    stp x4, x5, [sp, #32]
    stp x6, x7, [sp, #48]
    stp x8, x9, [sp, #64]
    stp x10, x11, [sp, #80]
    stp x12, x13, [sp, #96]
    stp x14, x15, [sp, #112]
    stp x16, x17, [sp, #128]
    stp x18, x19, [sp, #144]
    stp x20, x21, [sp, #160]
    stp x22, x23, [sp, #176]
    stp x24, x25, [sp, #192]
    stp x26, x27, [sp, #208]
    stp x28, x29, [sp, #224]
    str x30, [sp, #240]
    mrs x0, elr_el1
    mrs x1, spsr_el1
    mrs x2, esr_el1
    mrs x3, far_el1
    stp x0, x1, [sp, #248]
    stp x2, x3, [sp, #264]
    mov x0, sp
    mov x1, #5
    bl exception_handler
    b __exception_restore

// Entry 6: FIQ
.balign 128
    sub sp, sp, #288
    stp x0, x1, [sp, #0]
    stp x2, x3, [sp, #16]
    stp x4, x5, [sp, #32]
    stp x6, x7, [sp, #48]
    stp x8, x9, [sp, #64]
    stp x10, x11, [sp, #80]
    stp x12, x13, [sp, #96]
    stp x14, x15, [sp, #112]
    stp x16, x17, [sp, #128]
    stp x18, x19, [sp, #144]
    stp x20, x21, [sp, #160]
    stp x22, x23, [sp, #176]
    stp x24, x25, [sp, #192]
    stp x26, x27, [sp, #208]
    stp x28, x29, [sp, #224]
    str x30, [sp, #240]
    mrs x0, elr_el1
    mrs x1, spsr_el1
    mrs x2, esr_el1
    mrs x3, far_el1
    stp x0, x1, [sp, #248]
    stp x2, x3, [sp, #264]
    mov x0, sp
    mov x1, #6
    bl exception_handler
    b __exception_restore

// Entry 7: SError
.balign 128
    sub sp, sp, #288
    stp x0, x1, [sp, #0]
    stp x2, x3, [sp, #16]
    stp x4, x5, [sp, #32]
    stp x6, x7, [sp, #48]
    stp x8, x9, [sp, #64]
    stp x10, x11, [sp, #80]
    stp x12, x13, [sp, #96]
    stp x14, x15, [sp, #112]
    stp x16, x17, [sp, #128]
    stp x18, x19, [sp, #144]
    stp x20, x21, [sp, #160]
    stp x22, x23, [sp, #176]
    stp x24, x25, [sp, #192]
    stp x26, x27, [sp, #208]
    stp x28, x29, [sp, #224]
    str x30, [sp, #240]
    mrs x0, elr_el1
    mrs x1, spsr_el1
    mrs x2, esr_el1
    mrs x3, far_el1
    stp x0, x1, [sp, #248]
    stp x2, x3, [sp, #264]
    mov x0, sp
    mov x1, #7
    bl exception_handler
    b __exception_restore

// ============================================================================
// Lower EL using AArch64 - entries 8-11
// ============================================================================

// Entry 8: Synchronous
.balign 128
    sub sp, sp, #288
    stp x0, x1, [sp, #0]
    stp x2, x3, [sp, #16]
    stp x4, x5, [sp, #32]
    stp x6, x7, [sp, #48]
    stp x8, x9, [sp, #64]
    stp x10, x11, [sp, #80]
    stp x12, x13, [sp, #96]
    stp x14, x15, [sp, #112]
    stp x16, x17, [sp, #128]
    stp x18, x19, [sp, #144]
    stp x20, x21, [sp, #160]
    stp x22, x23, [sp, #176]
    stp x24, x25, [sp, #192]
    stp x26, x27, [sp, #208]
    stp x28, x29, [sp, #224]
    str x30, [sp, #240]
    mrs x0, elr_el1
    mrs x1, spsr_el1
    mrs x2, esr_el1
    mrs x3, far_el1
    stp x0, x1, [sp, #248]
    stp x2, x3, [sp, #264]
    mov x0, sp
    mov x1, #8
    bl exception_handler
    b __exception_restore

// Entry 9: IRQ
.balign 128
    sub sp, sp, #288
    stp x0, x1, [sp, #0]
    stp x2, x3, [sp, #16]
    stp x4, x5, [sp, #32]
    stp x6, x7, [sp, #48]
    stp x8, x9, [sp, #64]
    stp x10, x11, [sp, #80]
    stp x12, x13, [sp, #96]
    stp x14, x15, [sp, #112]
    stp x16, x17, [sp, #128]
    stp x18, x19, [sp, #144]
    stp x20, x21, [sp, #160]
    stp x22, x23, [sp, #176]
    stp x24, x25, [sp, #192]
    stp x26, x27, [sp, #208]
    stp x28, x29, [sp, #224]
    str x30, [sp, #240]
    mrs x0, elr_el1
    mrs x1, spsr_el1
    mrs x2, esr_el1
    mrs x3, far_el1
    stp x0, x1, [sp, #248]
    stp x2, x3, [sp, #264]
    mov x0, sp
    mov x1, #9
    bl exception_handler
    b __exception_restore

// Entry 10: FIQ
.balign 128
    sub sp, sp, #288
    stp x0, x1, [sp, #0]
    stp x2, x3, [sp, #16]
    stp x4, x5, [sp, #32]
    stp x6, x7, [sp, #48]
    stp x8, x9, [sp, #64]
    stp x10, x11, [sp, #80]
    stp x12, x13, [sp, #96]
    stp x14, x15, [sp, #112]
    stp x16, x17, [sp, #128]
    stp x18, x19, [sp, #144]
    stp x20, x21, [sp, #160]
    stp x22, x23, [sp, #176]
    stp x24, x25, [sp, #192]
    stp x26, x27, [sp, #208]
    stp x28, x29, [sp, #224]
    str x30, [sp, #240]
    mrs x0, elr_el1
    mrs x1, spsr_el1
    mrs x2, esr_el1
    mrs x3, far_el1
    stp x0, x1, [sp, #248]
    stp x2, x3, [sp, #264]
    mov x0, sp
    mov x1, #10
    bl exception_handler
    b __exception_restore

// Entry 11: SError
.balign 128
    sub sp, sp, #288
    stp x0, x1, [sp, #0]
    stp x2, x3, [sp, #16]
    stp x4, x5, [sp, #32]
    stp x6, x7, [sp, #48]
    stp x8, x9, [sp, #64]
    stp x10, x11, [sp, #80]
    stp x12, x13, [sp, #96]
    stp x14, x15, [sp, #112]
    stp x16, x17, [sp, #128]
    stp x18, x19, [sp, #144]
    stp x20, x21, [sp, #160]
    stp x22, x23, [sp, #176]
    stp x24, x25, [sp, #192]
    stp x26, x27, [sp, #208]
    stp x28, x29, [sp, #224]
    str x30, [sp, #240]
    mrs x0, elr_el1
    mrs x1, spsr_el1
    mrs x2, esr_el1
    mrs x3, far_el1
    stp x0, x1, [sp, #248]
    stp x2, x3, [sp, #264]
    mov x0, sp
    mov x1, #11
    bl exception_handler
    b __exception_restore

// ============================================================================
// Lower EL using AArch32 - entries 12-15
// ============================================================================

// Entry 12: Synchronous
.balign 128
    sub sp, sp, #288
    stp x0, x1, [sp, #0]
    stp x2, x3, [sp, #16]
    stp x4, x5, [sp, #32]
    stp x6, x7, [sp, #48]
    stp x8, x9, [sp, #64]
    stp x10, x11, [sp, #80]
    stp x12, x13, [sp, #96]
    stp x14, x15, [sp, #112]
    stp x16, x17, [sp, #128]
    stp x18, x19, [sp, #144]
    stp x20, x21, [sp, #160]
    stp x22, x23, [sp, #176]
    stp x24, x25, [sp, #192]
    stp x26, x27, [sp, #208]
    stp x28, x29, [sp, #224]
    str x30, [sp, #240]
    mrs x0, elr_el1
    mrs x1, spsr_el1
    mrs x2, esr_el1
    mrs x3, far_el1
    stp x0, x1, [sp, #248]
    stp x2, x3, [sp, #264]
    mov x0, sp
    mov x1, #12
    bl exception_handler
    b __exception_restore

// Entry 13: IRQ
.balign 128
    sub sp, sp, #288
    stp x0, x1, [sp, #0]
    stp x2, x3, [sp, #16]
    stp x4, x5, [sp, #32]
    stp x6, x7, [sp, #48]
    stp x8, x9, [sp, #64]
    stp x10, x11, [sp, #80]
    stp x12, x13, [sp, #96]
    stp x14, x15, [sp, #112]
    stp x16, x17, [sp, #128]
    stp x18, x19, [sp, #144]
    stp x20, x21, [sp, #160]
    stp x22, x23, [sp, #176]
    stp x24, x25, [sp, #192]
    stp x26, x27, [sp, #208]
    stp x28, x29, [sp, #224]
    str x30, [sp, #240]
    mrs x0, elr_el1
    mrs x1, spsr_el1
    mrs x2, esr_el1
    mrs x3, far_el1
    stp x0, x1, [sp, #248]
    stp x2, x3, [sp, #264]
    mov x0, sp
    mov x1, #13
    bl exception_handler
    b __exception_restore

// Entry 14: FIQ
.balign 128
    sub sp, sp, #288
    stp x0, x1, [sp, #0]
    stp x2, x3, [sp, #16]
    stp x4, x5, [sp, #32]
    stp x6, x7, [sp, #48]
    stp x8, x9, [sp, #64]
    stp x10, x11, [sp, #80]
    stp x12, x13, [sp, #96]
    stp x14, x15, [sp, #112]
    stp x16, x17, [sp, #128]
    stp x18, x19, [sp, #144]
    stp x20, x21, [sp, #160]
    stp x22, x23, [sp, #176]
    stp x24, x25, [sp, #192]
    stp x26, x27, [sp, #208]
    stp x28, x29, [sp, #224]
    str x30, [sp, #240]
    mrs x0, elr_el1
    mrs x1, spsr_el1
    mrs x2, esr_el1
    mrs x3, far_el1
    stp x0, x1, [sp, #248]
    stp x2, x3, [sp, #264]
    mov x0, sp
    mov x1, #14
    bl exception_handler
    b __exception_restore

// Entry 15: SError
.balign 128
    sub sp, sp, #288
    stp x0, x1, [sp, #0]
    stp x2, x3, [sp, #16]
    stp x4, x5, [sp, #32]
    stp x6, x7, [sp, #48]
    stp x8, x9, [sp, #64]
    stp x10, x11, [sp, #80]
    stp x12, x13, [sp, #96]
    stp x14, x15, [sp, #112]
    stp x16, x17, [sp, #128]
    stp x18, x19, [sp, #144]
    stp x20, x21, [sp, #160]
    stp x22, x23, [sp, #176]
    stp x24, x25, [sp, #192]
    stp x26, x27, [sp, #208]
    stp x28, x29, [sp, #224]
    str x30, [sp, #240]
    mrs x0, elr_el1
    mrs x1, spsr_el1
    mrs x2, esr_el1
    mrs x3, far_el1
    stp x0, x1, [sp, #248]
    stp x2, x3, [sp, #264]
    mov x0, sp
    mov x1, #15
    bl exception_handler
    b __exception_restore

// ============================================================================
// 컨텍스트 복원 공통 코드
// ============================================================================
.balign 16
__exception_restore:
    ldp x0, x1, [sp, #248]
    msr elr_el1, x0
    msr spsr_el1, x1

    ldp x0, x1, [sp, #0]
    ldp x2, x3, [sp, #16]
    ldp x4, x5, [sp, #32]
    ldp x6, x7, [sp, #48]
    ldp x8, x9, [sp, #64]
    ldp x10, x11, [sp, #80]
    ldp x12, x13, [sp, #96]
    ldp x14, x15, [sp, #112]
    ldp x16, x17, [sp, #128]
    ldp x18, x19, [sp, #144]
    ldp x20, x21, [sp, #160]
    ldp x22, x23, [sp, #176]
    ldp x24, x25, [sp, #192]
    ldp x26, x27, [sp, #208]
    ldp x28, x29, [sp, #224]
    ldr x30, [sp, #240]

    add sp, sp, #288
    eret
"#
);

/// 예외 벡터 테이블 초기화
/// VBAR_EL1 레지스터에 벡터 테이블 주소 설정
pub fn init() {
    unsafe extern "C" {
        static exception_vectors: u8;
    }

    unsafe {
        let vectors = &raw const exception_vectors as u64;
        core::arch::asm!(
            "msr vbar_el1, {0}",
            "isb",
            in(reg) vectors,
            options(nomem, nostack)
        );

        crate::kprintln!("[aarch64] Exception vectors initialized at {:#x}", vectors);
    }
}

/// 테스트용: 소프트웨어 브레이크포인트 발생
#[allow(dead_code)]
pub fn test_exception() {
    unsafe {
        core::arch::asm!("brk #0");
    }
}
