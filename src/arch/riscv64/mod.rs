pub mod mmu;
pub mod plic;
pub mod rtc;
pub mod timer;
pub mod trap;
pub mod uart;

use core::arch::asm;

use crate::kprintln;

fn init_pmp_allow_all() {
    // NAPOT + RWX로 전체 물리 주소 공간을 U-mode에도 허용한다.
    // pmpaddr0는 물리 주소의 [XLEN-1:2]를 저장하므로 최댓값을 우측 2비트 쉬프트한다.
    let pmpaddr0 = usize::MAX >> 2;
    let pmpcfg0 = 0x1fusize; // R|W|X|A=NAPOT

    unsafe {
        // SAFETY: 단일 hart 부트 초기화 시점에 PMP 엔트리 0을 전체 허용 규칙으로 설정한다.
        asm!(
            "csrw pmpaddr0, {addr}",
            "csrw pmpcfg0, {cfg}",
            addr = in(reg) pmpaddr0,
            cfg = in(reg) pmpcfg0,
            options(nostack, nomem)
        );
    }

    kprintln!("[riscv64] PMP configured: entry0 NAPOT RWX allow-all");
}

/// riscv64 아키텍처 초기화
pub fn init() {
    init_pmp_allow_all();
    trap::init();
}
