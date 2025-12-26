//! CPU 컨텍스트 구조체
//!
//! 컨텍스트 스위칭에 필요한 레지스터 저장/복원을 위한 구조체

/// CPU 컨텍스트 - 컨텍스트 스위칭 시 저장되는 레지스터들
///
/// AArch64에서는 callee-saved 레지스터만 저장하면 됨:
/// - x19-x28: Callee-saved registers
/// - x29 (FP): Frame pointer
/// - x30 (LR): Link register (return address)
/// - SP: Stack pointer
///
/// RISC-V에서는:
/// - s0-s11 (x8-x9, x18-x27): Saved registers
/// - ra (x1): Return address
/// - sp (x2): Stack pointer
#[cfg(target_arch = "aarch64")]
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Context {
    /// x19-x28 (callee-saved)
    pub x19: u64,
    pub x20: u64,
    pub x21: u64,
    pub x22: u64,
    pub x23: u64,
    pub x24: u64,
    pub x25: u64,
    pub x26: u64,
    pub x27: u64,
    pub x28: u64,
    /// Frame pointer (x29)
    pub fp: u64,
    /// Link register (x30) - return address
    pub lr: u64,
    /// Stack pointer
    pub sp: u64,
}

#[cfg(target_arch = "aarch64")]
impl Context {
    /// 빈 컨텍스트 생성
    pub const fn empty() -> Self {
        Context {
            x19: 0, x20: 0, x21: 0, x22: 0, x23: 0,
            x24: 0, x25: 0, x26: 0, x27: 0, x28: 0,
            fp: 0,
            lr: 0,
            sp: 0,
        }
    }

    /// 새 스레드를 위한 컨텍스트 생성
    ///
    /// entry: 스레드 시작 주소
    /// stack_top: 스택 최상위 주소
    pub fn new(entry: usize, stack_top: usize) -> Self {
        Context {
            x19: 0, x20: 0, x21: 0, x22: 0, x23: 0,
            x24: 0, x25: 0, x26: 0, x27: 0, x28: 0,
            fp: 0,
            lr: entry as u64,  // 컨텍스트 스위치 후 "ret"이 이 주소로 점프
            sp: stack_top as u64,
        }
    }
}

/// RISC-V 컨텍스트
#[cfg(target_arch = "riscv64")]
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Context {
    /// Return address (x1)
    pub ra: u64,
    /// Stack pointer (x2)
    pub sp: u64,
    /// Saved registers s0-s11 (x8-x9, x18-x27)
    pub s0: u64,  // x8 / fp
    pub s1: u64,  // x9
    pub s2: u64,  // x18
    pub s3: u64,  // x19
    pub s4: u64,  // x20
    pub s5: u64,  // x21
    pub s6: u64,  // x22
    pub s7: u64,  // x23
    pub s8: u64,  // x24
    pub s9: u64,  // x25
    pub s10: u64, // x26
    pub s11: u64, // x27
}

#[cfg(target_arch = "riscv64")]
impl Context {
    /// 빈 컨텍스트 생성
    pub const fn empty() -> Self {
        Context {
            ra: 0,
            sp: 0,
            s0: 0, s1: 0, s2: 0, s3: 0, s4: 0, s5: 0,
            s6: 0, s7: 0, s8: 0, s9: 0, s10: 0, s11: 0,
        }
    }

    /// 새 스레드를 위한 컨텍스트 생성
    pub fn new(entry: usize, stack_top: usize) -> Self {
        Context {
            ra: entry as u64,  // ret이 이 주소로 점프
            sp: stack_top as u64,
            s0: 0, s1: 0, s2: 0, s3: 0, s4: 0, s5: 0,
            s6: 0, s7: 0, s8: 0, s9: 0, s10: 0, s11: 0,
        }
    }
}

// 어셈블리로 구현된 컨텍스트 스위칭 함수
unsafe extern "C" {
    /// 컨텍스트 스위칭
    ///
    /// 현재 컨텍스트를 old_ctx에 저장하고, new_ctx를 로드하여 실행을 전환합니다.
    ///
    /// # Safety
    /// - old_ctx와 new_ctx는 유효한 Context 구조체를 가리켜야 합니다.
    /// - 호출 후 new_ctx의 스레드에서 실행이 계속됩니다.
    pub fn context_switch(old_ctx: *mut Context, new_ctx: *const Context);
}

/// AArch64 컨텍스트 스위칭 어셈블리
#[cfg(target_arch = "aarch64")]
core::arch::global_asm!(
    r#"
.section .text
.global context_switch
.type context_switch, %function
context_switch:
    // 현재 컨텍스트 저장 (x0 = old_ctx)
    // callee-saved 레지스터 x19-x28 저장
    stp x19, x20, [x0, #0]
    stp x21, x22, [x0, #16]
    stp x23, x24, [x0, #32]
    stp x25, x26, [x0, #48]
    stp x27, x28, [x0, #64]
    // fp (x29), lr (x30) 저장
    stp x29, x30, [x0, #80]
    // sp 저장
    mov x9, sp
    str x9, [x0, #96]

    // 새 컨텍스트 로드 (x1 = new_ctx)
    // callee-saved 레지스터 x19-x28 복원
    ldp x19, x20, [x1, #0]
    ldp x21, x22, [x1, #16]
    ldp x23, x24, [x1, #32]
    ldp x25, x26, [x1, #48]
    ldp x27, x28, [x1, #64]
    // fp (x29), lr (x30) 복원
    ldp x29, x30, [x1, #80]
    // sp 복원
    ldr x9, [x1, #96]
    mov sp, x9

    // lr로 점프 (ret은 x30으로 점프)
    ret
"#
);

/// RISC-V 컨텍스트 스위칭 어셈블리
#[cfg(target_arch = "riscv64")]
core::arch::global_asm!(
    r#"
.section .text
.global context_switch
.type context_switch, @function
context_switch:
    // 현재 컨텍스트 저장 (a0 = old_ctx)
    sd ra, 0(a0)
    sd sp, 8(a0)
    sd s0, 16(a0)
    sd s1, 24(a0)
    sd s2, 32(a0)
    sd s3, 40(a0)
    sd s4, 48(a0)
    sd s5, 56(a0)
    sd s6, 64(a0)
    sd s7, 72(a0)
    sd s8, 80(a0)
    sd s9, 88(a0)
    sd s10, 96(a0)
    sd s11, 104(a0)

    // 새 컨텍스트 로드 (a1 = new_ctx)
    ld ra, 0(a1)
    ld sp, 8(a1)
    ld s0, 16(a1)
    ld s1, 24(a1)
    ld s2, 32(a1)
    ld s3, 40(a1)
    ld s4, 48(a1)
    ld s5, 56(a1)
    ld s6, 64(a1)
    ld s7, 72(a1)
    ld s8, 80(a1)
    ld s9, 88(a1)
    ld s10, 96(a1)
    ld s11, 104(a1)

    // ra로 점프
    ret
"#
);
