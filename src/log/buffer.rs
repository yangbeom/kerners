//! 커널 로그 링 버퍼 (dmesg)
//!
//! 64KB 정적 배열 기반 순환 버퍼.
//! 엔트리 포맷: [4:length][1:level][8:timestamp_us][1:cpu_id][N:msg]

use crate::sync::Spinlock;
use super::LogLevel;

const RING_BUFFER_SIZE: usize = 64 * 1024; // 64KB
const ENTRY_HEADER_SIZE: usize = 14; // 4 + 1 + 8 + 1

struct RingBuffer {
    buffer: [u8; RING_BUFFER_SIZE],
    write_pos: usize,
    total_written: usize, // 총 기록 바이트 수 (wrap 감지용)
}

impl RingBuffer {
    const fn new() -> Self {
        Self {
            buffer: [0u8; RING_BUFFER_SIZE],
            write_pos: 0,
            total_written: 0,
        }
    }

    fn append(&mut self, level: LogLevel, seconds: u64, micros: u64, cpu_id: u32, msg: &str) {
        let msg_bytes = msg.as_bytes();
        let total_len = ENTRY_HEADER_SIZE + msg_bytes.len();

        // 버퍼 절반보다 큰 메시지는 무시
        if total_len > RING_BUFFER_SIZE / 2 {
            return;
        }

        // length (u32 LE)
        let len_bytes = (total_len as u32).to_le_bytes();
        self.write_bytes(&len_bytes);

        // level (u8)
        self.write_bytes(&[level as u8]);

        // timestamp_us (u64 LE)
        let timestamp_us = seconds * 1_000_000 + micros;
        self.write_bytes(&timestamp_us.to_le_bytes());

        // cpu_id (u8)
        self.write_bytes(&[cpu_id as u8]);

        // message
        self.write_bytes(msg_bytes);
    }

    fn write_bytes(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.buffer[self.write_pos] = byte;
            self.write_pos += 1;
            if self.write_pos >= RING_BUFFER_SIZE {
                self.write_pos = 0;
            }
        }
        self.total_written += bytes.len();
    }

    fn has_wrapped(&self) -> bool {
        self.total_written > RING_BUFFER_SIZE
    }
}

static RING_BUFFER: Spinlock<RingBuffer> = Spinlock::new(RingBuffer::new());

pub fn init() {
    // 이미 const 초기화되어 있으므로 추가 작업 불필요
}

pub fn append(level: LogLevel, seconds: u64, micros: u64, cpu_id: u32, msg: &str) {
    let mut buf = RING_BUFFER.lock();
    buf.append(level, seconds, micros, cpu_id, msg);
}

pub fn dump_logs() {
    let buf = RING_BUFFER.lock();

    // 유효한 데이터 영역 결정
    let (data_start, data_len) = if !buf.has_wrapped() {
        (0, buf.write_pos)
    } else {
        (buf.write_pos, RING_BUFFER_SIZE)
    };

    if data_len == 0 {
        crate::console::puts("(empty log buffer)\n");
        return;
    }

    // 엔트리 파싱 및 출력
    let mut offset = 0;
    while offset + ENTRY_HEADER_SIZE <= data_len {
        // length 읽기
        let mut len_bytes = [0u8; 4];
        for i in 0..4 {
            len_bytes[i] = buf.buffer[(data_start + offset + i) % RING_BUFFER_SIZE];
        }
        let total_len = u32::from_le_bytes(len_bytes) as usize;

        // 유효성 검사
        if total_len < ENTRY_HEADER_SIZE || total_len > RING_BUFFER_SIZE / 2 {
            break;
        }
        if offset + total_len > data_len {
            break;
        }

        // level 읽기
        let level = buf.buffer[(data_start + offset + 4) % RING_BUFFER_SIZE];

        // timestamp 읽기
        let mut ts_bytes = [0u8; 8];
        for i in 0..8 {
            ts_bytes[i] = buf.buffer[(data_start + offset + 5 + i) % RING_BUFFER_SIZE];
        }
        let timestamp_us = u64::from_le_bytes(ts_bytes);
        let seconds = timestamp_us / 1_000_000;
        let micros = timestamp_us % 1_000_000;

        // cpu_id 읽기
        let cpu_id = buf.buffer[(data_start + offset + 13) % RING_BUFFER_SIZE];

        // message 읽기
        let msg_len = total_len - ENTRY_HEADER_SIZE;
        let mut msg_buf = [0u8; 512];
        let copy_len = msg_len.min(msg_buf.len());
        for i in 0..copy_len {
            msg_buf[i] = buf.buffer[(data_start + offset + ENTRY_HEADER_SIZE + i) % RING_BUFFER_SIZE];
        }

        // 출력
        let level_str = LogLevel::from_u8(level).as_str();
        if let Ok(msg) = core::str::from_utf8(&msg_buf[..copy_len]) {
            // 접두사 포매팅
            let mut prefix_buf = [0u8; 40];
            let prefix_len = {
                let mut w = super::BufWriter::new(&mut prefix_buf);
                let _ = core::fmt::write(
                    &mut w,
                    format_args!("[{:>6}.{:06}] CPU{} {}: ", seconds, micros, cpu_id, level_str),
                );
                w.pos
            };
            let prefix = unsafe { core::str::from_utf8_unchecked(&prefix_buf[..prefix_len]) };
            crate::console::puts(prefix);
            crate::console::puts(msg);
            crate::console::puts("\n");
        }

        offset += total_len;
    }
}
