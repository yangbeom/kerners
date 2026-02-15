pub mod mmu;
pub mod plic;
pub mod rtc;
pub mod timer;
pub mod trap;
pub mod uart;

/// riscv64 아키텍처 초기화
pub fn init() {
    trap::init();
}
