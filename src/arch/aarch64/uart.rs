//! ARM PL011 UART 드라이버
//! 
//! QEMU virt 보드의 UART는 PL011을 사용합니다.
//! 베이스 주소: 0x0900_0000
//! 
//! 주요 레지스터:
//! - UARTDR (0x000): Data Register
//! - UARTFR (0x018): Flag Register
//! - UARTIMSC (0x038): Interrupt Mask Set/Clear
//! - UARTRIS (0x03C): Raw Interrupt Status
//! - UARTMIS (0x040): Masked Interrupt Status
//! - UARTICR (0x044): Interrupt Clear

use core::ptr::{read_volatile, write_volatile};
use crate::sync::Mutex;
use crate::kprintln;

/// UART 기본 주소 얻기
///
/// config가 초기화되었으면 config에서 읽고, 아니면 boards 폴백 사용
#[inline]
fn uart_base() -> usize {
    if crate::drivers::config::is_initialized() {
        crate::drivers::config::uart_base()
    } else {
        // 초기화 전에는 보드 기본값 사용
        crate::boards::uart_base()
    }
}

// 레지스터 오프셋
const UARTDR: usize = 0x000;    // Data Register
const UARTFR: usize = 0x018;    // Flag Register
const UARTIMSC: usize = 0x038;  // Interrupt Mask Set/Clear
const UARTRIS: usize = 0x03C;   // Raw Interrupt Status
const UARTMIS: usize = 0x040;   // Masked Interrupt Status
const UARTICR: usize = 0x044;   // Interrupt Clear

// Flag Register 비트
const FR_TXFF: u32 = 1 << 5;    // Transmit FIFO full
const FR_RXFE: u32 = 1 << 4;    // Receive FIFO empty

// Interrupt 비트
const INT_RX: u32 = 1 << 4;     // Receive interrupt
const INT_TX: u32 = 1 << 5;     // Transmit interrupt

/// 입력 버퍼 크기
const INPUT_BUFFER_SIZE: usize = 256;

/// 입력 버퍼
static INPUT_BUFFER: Mutex<InputBuffer> = Mutex::new(InputBuffer::new());

/// 순환 버퍼
struct InputBuffer {
    buffer: [u8; INPUT_BUFFER_SIZE],
    read_pos: usize,
    write_pos: usize,
    count: usize,
}

impl InputBuffer {
    const fn new() -> Self {
        Self {
            buffer: [0; INPUT_BUFFER_SIZE],
            read_pos: 0,
            write_pos: 0,
            count: 0,
        }
    }

    fn push(&mut self, byte: u8) -> bool {
        if self.count >= INPUT_BUFFER_SIZE {
            return false; // 버퍼 가득 찼음
        }

        self.buffer[self.write_pos] = byte;
        self.write_pos = (self.write_pos + 1) % INPUT_BUFFER_SIZE;
        self.count += 1;
        true
    }

    fn pop(&mut self) -> Option<u8> {
        if self.count == 0 {
            return None;
        }

        let byte = self.buffer[self.read_pos];
        self.read_pos = (self.read_pos + 1) % INPUT_BUFFER_SIZE;
        self.count -= 1;
        Some(byte)
    }

    fn is_empty(&self) -> bool {
        self.count == 0
    }
}

/// UART 레지스터 읽기
#[inline]
unsafe fn read_reg(offset: usize) -> u32 {
    read_volatile((uart_base() + offset) as *const u32)
}

/// UART 레지스터 쓰기
#[inline]
unsafe fn write_reg(offset: usize, value: u32) {
    write_volatile((uart_base() + offset) as *mut u32, value);
}

/// 문자 출력 (폴링 방식)
#[inline(always)]
pub fn putc(c: u8) {
    unsafe {
        // TX FIFO가 가득 찰 때까지 대기
        while read_reg(UARTFR) & FR_TXFF != 0 {
            core::hint::spin_loop();
        }
        write_reg(UARTDR, c as u32);
    }
}

/// 문자 입력 (폴링 방식)
pub fn getc() -> Option<u8> {
    unsafe {
        if read_reg(UARTFR) & FR_RXFE != 0 {
            None // RX FIFO 비어있음
        } else {
            Some((read_reg(UARTDR) & 0xFF) as u8)
        }
    }
}

/// 버퍼에서 문자 읽기
pub fn read_char() -> Option<u8> {
    INPUT_BUFFER.lock().pop()
}

/// 버퍼가 비어있는지 확인
pub fn is_input_empty() -> bool {
    INPUT_BUFFER.lock().is_empty()
}

/// UART 초기화 (인터럽트 활성화)
pub fn init() -> Result<(), &'static str> {
    kprintln!("\n[UART] Initializing with interrupt support...");
    
    unsafe {
        // 모든 pending 인터럽트 클리어
        write_reg(UARTICR, 0x7FF);  // 모든 인터럽트 클리어
        
        // FIFO 비우기
        while read_reg(UARTFR) & FR_RXFE == 0 {
            let _ = read_reg(UARTDR);  // 데이터 읽어서 버림
        }
        
        // RX 인터럽트 활성화
        let mut imsc = read_reg(UARTIMSC);
        imsc |= INT_RX;
        write_reg(UARTIMSC, imsc);
    }
    
    kprintln!("[UART] RX interrupt enabled");
    Ok(())
}

/// UART 인터럽트 핸들러
pub fn handle_irq() {
    unsafe {
        let mis = read_reg(UARTMIS);
        
        // RX 인터럽트 처리
        if mis & INT_RX != 0 {
            // FIFO에서 모든 문자 읽기
            while read_reg(UARTFR) & FR_RXFE == 0 {
                let ch = (read_reg(UARTDR) & 0xFF) as u8;
                
                // 에코백
                putc(ch);
                if ch == b'\r' {
                    putc(b'\n');
                }
                
                // 버퍼에 저장
                if let Some(mut buffer) = INPUT_BUFFER.try_lock() {
                    if !buffer.push(ch) {
                        // 버퍼 가득 참 - 조용히 무시
                    }
                }
            }
            
            // 인터럽트 클리어 (반드시 해야 함)
            write_reg(UARTICR, INT_RX);
        }
    }
}
