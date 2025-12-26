use core::fmt::{self, Write};

/// UART로 문자열을 출력하는 함수
pub fn puts(s: &str) {
    for &b in s.as_bytes() {
        putc_arch(b);
    }
}

/// UART로 단일 문자 출력
pub fn putc(c: u8) {
    putc_arch(c);
}

/// UART로 포맷팅된 문자열을 출력하는 구조체
pub struct Console;

impl Write for Console {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        puts(s);
        Ok(())
    }
}

/// 포맷팅된 문자열을 UART로 출력하는 함수
pub fn kprint(args: fmt::Arguments) {
    let _ = Console.write_fmt(args);
}

/// 개행을 포함하여 포맷팅된 문자열을 UART로 출력하는 함수
pub fn kprintln(args: fmt::Arguments) {
    kprint(args);
    puts("\n");
}

/// println! 스타일로 사용 가능한 kprintf 매크로
#[macro_export]
macro_rules! kprint {
    ($($arg:tt)*) => {
        $crate::console::kprint(core::format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! kprintln {
    () => {
        $crate::console::kprintln(core::format_args!(""))
    };
    ($($arg:tt)*) => {
        $crate::console::kprintln(core::format_args!($($arg)*))
    };
}

#[cfg(target_arch = "aarch64")]
fn putc_arch(c: u8) {
    crate::arch::uart::putc(c)
}

#[cfg(target_arch = "riscv64")]
fn putc_arch(c: u8) {
    crate::arch::uart::putc(c)
}
