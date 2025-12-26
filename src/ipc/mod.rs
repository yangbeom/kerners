//! IPC (Inter-Process Communication) 모듈
//!
//! 프로세스/스레드 간 통신 메커니즘 제공:
//! - 메시지 큐: 메시지 기반 통신
//! - (향후) 공유 메모리, 파이프 등

pub mod message_queue;

pub use message_queue::{MessageQueue, BoundedMessageQueue, Message};
