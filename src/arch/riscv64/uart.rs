//! RISC-V NS16550A UART 드라이버
//!
//! QEMU virt 보드는 16550A 호환 UART를 사용합니다.

use core::ptr::{read_volatile, write_volatile};

/// UART 기본 주소 얻기
#[inline]
fn uart_base() -> usize {
    if crate::drivers::config::is_initialized() {
        crate::drivers::config::uart_base()
    } else {
        crate::boards::uart_base()
    }
}

// NS16550A 레지스터 오프셋
const RBR: usize = 0x00;  // Receive Buffer Register (읽기)
const THR: usize = 0x00;  // Transmit Holding Register (쓰기)
const IER: usize = 0x01;  // Interrupt Enable Register
const FCR: usize = 0x02;  // FIFO Control Register (쓰기)
const LCR: usize = 0x03;  // Line Control Register
const LSR: usize = 0x05;  // Line Status Register

// LSR 비트
const LSR_RX_READY: u8 = 0x01;  // 데이터 수신 가능
const LSR_TX_EMPTY: u8 = 0x20;  // TX 버퍼 비어있음

/// UART 초기화
pub fn init() {
    unsafe {
        let base = uart_base() as *mut u8;

        // 인터럽트 비활성화
        write_volatile(base.add(IER), 0x00);

        // FIFO 활성화, FIFO 리셋
        write_volatile(base.add(FCR), 0x07);

        // 8 데이터 비트, 1 스톱 비트, 패리티 없음
        write_volatile(base.add(LCR), 0x03);
    }
}

#[inline(always)]
pub fn putc(c: u8) {
    unsafe {
        let base = uart_base() as *mut u8;
        // TX 버퍼가 비어있을 때까지 대기
        while read_volatile(base.add(LSR)) & LSR_TX_EMPTY == 0 {}
        write_volatile(base.add(THR), c);
    }
}

/// 문자 입력 (폴링 방식)
pub fn getc() -> Option<u8> {
    unsafe {
        let base = uart_base() as *mut u8;

        // 데이터가 수신되었는지 확인
        if read_volatile(base.add(LSR)) & LSR_RX_READY != 0 {
            Some(read_volatile(base.add(RBR)))
        } else {
            None
        }
    }
}

/// UART 인터럽트 핸들러 (현재는 폴링만 사용)
pub fn handle_irq() {
    // UART 인터럽트 처리 (필요시 구현)
}
