pub mod exception;
pub mod gic;
pub mod mmu;
pub mod timer;
pub mod uart;

/// aarch64 아키텍처 초기화
pub fn init() {
    exception::init();
}
