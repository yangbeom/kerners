//! 커널 로깅 시스템
//!
//! - 로그 레벨: ERROR, WARN, INFO, DEBUG, TRACE
//! - 타임스탬프 + CPU ID 접두사
//! - 64KB 링 버퍼 (dmesg)
//! - Per-CPU 재귀 방지

mod buffer;
mod macros;

use core::fmt;
use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};

const MAX_CPUS: usize = 8;

// 로깅 시스템 초기화 여부
static INITIALIZED: AtomicBool = AtomicBool::new(false);

// 전역 로그 레벨 (기본: Info = 2)
static CURRENT_LOG_LEVEL: AtomicU8 = AtomicU8::new(2);

// Per-CPU 재귀 방지 플래그
static LOGGING_IN_PROGRESS: [AtomicBool; MAX_CPUS] = [
    AtomicBool::new(false),
    AtomicBool::new(false),
    AtomicBool::new(false),
    AtomicBool::new(false),
    AtomicBool::new(false),
    AtomicBool::new(false),
    AtomicBool::new(false),
    AtomicBool::new(false),
];

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Error = 0,
    Warn = 1,
    Info = 2,
    Debug = 3,
    Trace = 4,
}

impl LogLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            LogLevel::Error => "ERROR",
            LogLevel::Warn => " WARN",
            LogLevel::Info => " INFO",
            LogLevel::Debug => "DEBUG",
            LogLevel::Trace => "TRACE",
        }
    }

    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => LogLevel::Error,
            1 => LogLevel::Warn,
            2 => LogLevel::Info,
            3 => LogLevel::Debug,
            4 => LogLevel::Trace,
            _ => LogLevel::Info,
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        // 숫자로 먼저 시도
        if let Some(c) = s.as_bytes().first() {
            if *c >= b'0' && *c <= b'4' {
                return Some(Self::from_u8(*c - b'0'));
            }
        }
        // 이름으로 시도 (대소문자 무시 — 수동 비교)
        let bytes = s.as_bytes();
        if eq_ignore_case(bytes, b"error") {
            Some(LogLevel::Error)
        } else if eq_ignore_case(bytes, b"warn") {
            Some(LogLevel::Warn)
        } else if eq_ignore_case(bytes, b"info") {
            Some(LogLevel::Info)
        } else if eq_ignore_case(bytes, b"debug") {
            Some(LogLevel::Debug)
        } else if eq_ignore_case(bytes, b"trace") {
            Some(LogLevel::Trace)
        } else {
            None
        }
    }
}

fn eq_ignore_case(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    for i in 0..a.len() {
        let ca = if a[i] >= b'A' && a[i] <= b'Z' {
            a[i] + 32
        } else {
            a[i]
        };
        let cb = if b[i] >= b'A' && b[i] <= b'Z' {
            b[i] + 32
        } else {
            b[i]
        };
        if ca != cb {
            return false;
        }
    }
    true
}

impl fmt::Display for LogLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

pub fn set_log_level(level: LogLevel) {
    CURRENT_LOG_LEVEL.store(level as u8, Ordering::Relaxed);
}

pub fn get_log_level() -> LogLevel {
    LogLevel::from_u8(CURRENT_LOG_LEVEL.load(Ordering::Relaxed))
}

/// 로깅 시스템 초기화
pub fn init() {
    buffer::init();
    INITIALIZED.store(true, Ordering::Release);
}

/// 로그 메시지 출력
pub fn log(level: LogLevel, args: fmt::Arguments) {
    // 초기화 전이면 직접 UART 출력 (fallback)
    if !INITIALIZED.load(Ordering::Acquire) {
        crate::console::kprint(args);
        crate::console::puts("\n");
        return;
    }

    // 레벨 필터링
    if (level as u8) > CURRENT_LOG_LEVEL.load(Ordering::Relaxed) {
        return;
    }

    // Per-CPU 재귀 방지
    let cpu_id = crate::proc::percpu::get_cpu_id() as usize;
    if cpu_id >= MAX_CPUS {
        return;
    }
    if LOGGING_IN_PROGRESS[cpu_id].swap(true, Ordering::Acquire) {
        // 이미 이 CPU에서 로깅 중 — 재귀 방지
        return;
    }

    // 타임스탬프 계산
    let (seconds, micros) = get_timestamp();

    // 스택 버퍼에 접두사 포매팅
    let mut prefix_buf = [0u8; 40];
    let prefix_len = format_prefix(&mut prefix_buf, seconds, micros, cpu_id as u32, level);
    let prefix = unsafe { core::str::from_utf8_unchecked(&prefix_buf[..prefix_len]) };

    // 메시지를 스택 버퍼에 포매팅
    let mut msg_buf = [0u8; 512];
    let msg_len = format_to_buf(&mut msg_buf, args);
    let msg = unsafe { core::str::from_utf8_unchecked(&msg_buf[..msg_len]) };

    // UART 출력
    crate::console::puts(prefix);
    crate::console::puts(msg);
    crate::console::puts("\n");

    // 링 버퍼에 저장
    buffer::append(level, seconds, micros, cpu_id as u32, msg);

    // 재귀 방지 해제
    LOGGING_IN_PROGRESS[cpu_id].store(false, Ordering::Release);
}

/// dmesg — 링 버퍼 내용 출력
pub fn dump_logs() {
    buffer::dump_logs();
}

// 타임스탬프 (초, 마이크로초) 계산
fn get_timestamp() -> (u64, u64) {
    #[cfg(target_arch = "aarch64")]
    {
        let counter = crate::arch::timer::get_counter();
        let freq = crate::arch::timer::get_frequency();
        counter_to_time(counter, freq)
    }
    #[cfg(target_arch = "riscv64")]
    {
        let counter = crate::arch::timer::get_time();
        let freq = crate::boards::timer_freq();
        counter_to_time(counter, freq)
    }
}

fn counter_to_time(counter: u64, freq: u64) -> (u64, u64) {
    if freq == 0 {
        return (0, 0);
    }
    let seconds = counter / freq;
    let remainder = counter % freq;
    let micros = (remainder * 1_000_000) / freq;
    (seconds, micros)
}

// 접두사 포매팅: "[  123.456789] CPU0  INFO: "
fn format_prefix(buf: &mut [u8], seconds: u64, micros: u64, cpu_id: u32, level: LogLevel) -> usize {
    let mut writer = BufWriter::new(buf);
    let _ = fmt::write(
        &mut writer,
        format_args!("[{:>6}.{:06}] CPU{} {}: ", seconds, micros, cpu_id, level),
    );
    writer.pos
}

// fmt::Arguments를 바이트 버퍼에 포매팅
fn format_to_buf(buf: &mut [u8], args: fmt::Arguments) -> usize {
    let mut writer = BufWriter::new(buf);
    let _ = fmt::write(&mut writer, args);
    writer.pos
}

// 스택 버퍼에 쓰는 fmt::Write 구현
struct BufWriter<'a> {
    buf: &'a mut [u8],
    pos: usize,
}

impl<'a> BufWriter<'a> {
    fn new(buf: &'a mut [u8]) -> Self {
        Self { buf, pos: 0 }
    }
}

impl fmt::Write for BufWriter<'_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let bytes = s.as_bytes();
        let remaining = self.buf.len() - self.pos;
        let copy_len = bytes.len().min(remaining);
        self.buf[self.pos..self.pos + copy_len].copy_from_slice(&bytes[..copy_len]);
        self.pos += copy_len;
        Ok(())
    }
}
