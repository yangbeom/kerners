//! 익명 파이프 VNode 구현
//!
//! `pipe2`에서 읽기/쓰기 엔드포인트를 생성해 FD 테이블에 넣는다.

use alloc::collections::VecDeque;
use alloc::sync::Arc;
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::sync::Mutex;

use super::{FileMode, Stat, VfsError, VfsResult, VNode, VNodeType};

const PIPE_CAPACITY: usize = 64 * 1024;

struct PipeShared {
    buf: Mutex<VecDeque<u8>>,
    readers: AtomicUsize,
    writers: AtomicUsize,
}

impl PipeShared {
    fn new() -> Self {
        Self {
            buf: Mutex::new(VecDeque::new()),
            readers: AtomicUsize::new(1),
            writers: AtomicUsize::new(1),
        }
    }
}

struct PipeReadEnd {
    shared: Arc<PipeShared>,
}

impl Drop for PipeReadEnd {
    fn drop(&mut self) {
        self.shared.readers.fetch_sub(1, Ordering::AcqRel);
    }
}

impl VNode for PipeReadEnd {
    fn node_type(&self) -> VNodeType {
        VNodeType::Fifo
    }

    fn read(&self, _offset: usize, out: &mut [u8]) -> VfsResult<usize> {
        if out.is_empty() {
            return Ok(0);
        }

        let mut buf = self.shared.buf.lock();
        if buf.is_empty() {
            if self.shared.writers.load(Ordering::Acquire) == 0 {
                return Ok(0);
            }
            return Ok(0);
        }

        let mut n = 0usize;
        while n < out.len() {
            let Some(b) = buf.pop_front() else {
                break;
            };
            out[n] = b;
            n += 1;
        }

        Ok(n)
    }

    fn write(&self, _offset: usize, _buf: &[u8]) -> VfsResult<usize> {
        Err(VfsError::PermissionDenied)
    }

    fn stat(&self) -> VfsResult<Stat> {
        let size = self.shared.buf.lock().len() as u64;
        Ok(Stat {
            node_type: VNodeType::Fifo,
            mode: FileMode::new(0o600),
            size,
            nlink: 1,
            ..Default::default()
        })
    }
}

struct PipeWriteEnd {
    shared: Arc<PipeShared>,
}

impl Drop for PipeWriteEnd {
    fn drop(&mut self) {
        self.shared.writers.fetch_sub(1, Ordering::AcqRel);
    }
}

impl VNode for PipeWriteEnd {
    fn node_type(&self) -> VNodeType {
        VNodeType::Fifo
    }

    fn read(&self, _offset: usize, _buf: &mut [u8]) -> VfsResult<usize> {
        Err(VfsError::PermissionDenied)
    }

    fn write(&self, _offset: usize, input: &[u8]) -> VfsResult<usize> {
        if input.is_empty() {
            return Ok(0);
        }
        if self.shared.readers.load(Ordering::Acquire) == 0 {
            return Err(VfsError::IoError);
        }

        let mut buf = self.shared.buf.lock();
        let available = PIPE_CAPACITY.saturating_sub(buf.len());
        if available == 0 {
            return Ok(0);
        }

        let to_write = core::cmp::min(available, input.len());
        for byte in input.iter().take(to_write) {
            buf.push_back(*byte);
        }
        Ok(to_write)
    }

    fn stat(&self) -> VfsResult<Stat> {
        let size = self.shared.buf.lock().len() as u64;
        Ok(Stat {
            node_type: VNodeType::Fifo,
            mode: FileMode::new(0o600),
            size,
            nlink: 1,
            ..Default::default()
        })
    }
}

/// 파이프 읽기/쓰기 엔드포인트 생성
pub fn create_pipe_pair() -> (Arc<dyn VNode>, Arc<dyn VNode>) {
    let shared = Arc::new(PipeShared::new());
    let read_end: Arc<dyn VNode> = Arc::new(PipeReadEnd {
        shared: shared.clone(),
    });
    let write_end: Arc<dyn VNode> = Arc::new(PipeWriteEnd { shared });
    (read_end, write_end)
}
