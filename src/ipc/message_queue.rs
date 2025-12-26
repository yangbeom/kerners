//! 메시지 큐 (Message Queue)
//!
//! 프로세스/스레드 간 메시지 기반 통신을 위한 IPC 메커니즘
//!
//! ## 특징
//! - FIFO 순서 보장
//! - 블로킹/논블로킹 송수신 지원
//! - 타입 안전 (제네릭)
//! - 용량 제한 옵션 (BoundedMessageQueue)
//!
//! ## 사용 예시
//! ```rust
//! let mq: MessageQueue<u32> = MessageQueue::new();
//! mq.send(42);
//! let msg = mq.receive();
//! ```

use alloc::collections::VecDeque;
use alloc::boxed::Box;
use alloc::vec::Vec;
use alloc::string::String;
use core::sync::atomic::{AtomicUsize, AtomicBool, Ordering};
use crate::sync::{Mutex, Semaphore};

/// 메시지 우선순위
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    Low = 0,
    Normal = 1,
    High = 2,
    Urgent = 3,
}

impl Default for Priority {
    fn default() -> Self {
        Priority::Normal
    }
}

/// 메시지 래퍼 (우선순위 포함)
#[derive(Debug)]
pub struct Message<T> {
    /// 메시지 데이터
    pub data: T,
    /// 우선순위
    pub priority: Priority,
    /// 송신자 ID (옵션)
    pub sender_id: Option<u64>,
    /// 타임스탬프 (옵션, tick 기준)
    pub timestamp: u64,
}

impl<T> Message<T> {
    /// 기본 메시지 생성
    pub fn new(data: T) -> Self {
        Self {
            data,
            priority: Priority::Normal,
            sender_id: None,
            timestamp: 0,
        }
    }

    /// 우선순위 지정 메시지 생성
    pub fn with_priority(data: T, priority: Priority) -> Self {
        Self {
            data,
            priority,
            sender_id: None,
            timestamp: 0,
        }
    }
}

/// 메시지 큐 에러
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageQueueError {
    /// 큐가 가득 참
    Full,
    /// 큐가 비어있음
    Empty,
    /// 큐가 닫힘
    Closed,
    /// 타임아웃
    Timeout,
}

/// 무제한 메시지 큐
///
/// 용량 제한 없이 메시지를 저장할 수 있는 큐
/// 메모리가 허용하는 한 무한히 저장 가능
pub struct MessageQueue<T> {
    /// 내부 큐 (Mutex로 보호)
    queue: Mutex<VecDeque<Message<T>>>,
    /// 메시지 수 (빠른 조회용)
    count: AtomicUsize,
    /// 큐 닫힘 상태
    closed: AtomicBool,
    /// 수신 대기용 세마포어
    sem_items: Semaphore,
}

unsafe impl<T: Send> Send for MessageQueue<T> {}
unsafe impl<T: Send> Sync for MessageQueue<T> {}

impl<T> MessageQueue<T> {
    /// 새 메시지 큐 생성
    pub fn new() -> Self {
        Self {
            queue: Mutex::new(VecDeque::new()),
            count: AtomicUsize::new(0),
            closed: AtomicBool::new(false),
            sem_items: Semaphore::new(0),
        }
    }

    /// 메시지 송신 (항상 성공, 무제한)
    pub fn send(&self, data: T) -> Result<(), MessageQueueError> {
        self.send_message(Message::new(data))
    }

    /// 우선순위 메시지 송신
    pub fn send_priority(&self, data: T, priority: Priority) -> Result<(), MessageQueueError> {
        self.send_message(Message::with_priority(data, priority))
    }

    /// 메시지 객체 송신
    pub fn send_message(&self, msg: Message<T>) -> Result<(), MessageQueueError> {
        if self.closed.load(Ordering::Relaxed) {
            return Err(MessageQueueError::Closed);
        }

        {
            let mut queue = self.queue.lock();
            // 항상 뒤에 추가 (수신 시 우선순위 처리)
            queue.push_back(msg);
        }

        self.count.fetch_add(1, Ordering::Release);
        self.sem_items.release();
        Ok(())
    }

    /// 우선순위가 가장 높은 메시지의 인덱스 찾기
    fn find_highest_priority(queue: &VecDeque<Message<T>>) -> Option<usize> {
        if queue.is_empty() {
            return None;
        }
        
        let mut best_idx = 0;
        let mut best_priority = queue[0].priority;
        
        for (i, msg) in queue.iter().enumerate() {
            if msg.priority > best_priority {
                best_priority = msg.priority;
                best_idx = i;
            }
        }
        
        Some(best_idx)
    }

    /// 메시지 수신 (블로킹)
    ///
    /// 메시지가 올 때까지 대기, 우선순위 높은 것 먼저
    pub fn receive(&self) -> Result<Message<T>, MessageQueueError> {
        if self.closed.load(Ordering::Relaxed) && self.is_empty() {
            return Err(MessageQueueError::Closed);
        }

        self.sem_items.acquire();

        if self.closed.load(Ordering::Relaxed) && self.is_empty() {
            return Err(MessageQueueError::Closed);
        }

        let mut queue = self.queue.lock();
        if let Some(idx) = Self::find_highest_priority(&queue) {
            if let Some(msg) = queue.remove(idx) {
                self.count.fetch_sub(1, Ordering::Release);
                return Ok(msg);
            }
        }
        Err(MessageQueueError::Empty)
    }

    /// 메시지 수신 시도 (논블로킹)
    pub fn try_receive(&self) -> Result<Message<T>, MessageQueueError> {
        if self.closed.load(Ordering::Relaxed) && self.is_empty() {
            return Err(MessageQueueError::Closed);
        }

        if !self.sem_items.try_acquire() {
            return Err(MessageQueueError::Empty);
        }

        let mut queue = self.queue.lock();
        if let Some(idx) = Self::find_highest_priority(&queue) {
            if let Some(msg) = queue.remove(idx) {
                self.count.fetch_sub(1, Ordering::Release);
                return Ok(msg);
            }
        }
        Err(MessageQueueError::Empty)
    }

    /// 큐에 있는 메시지 수
    #[inline]
    pub fn len(&self) -> usize {
        self.count.load(Ordering::Relaxed)
    }

    /// 큐가 비어있는지 확인
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 큐 닫기
    ///
    /// 더 이상 송신 불가, 남은 메시지는 수신 가능
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
        // 대기 중인 수신자들 깨우기
        for _ in 0..10 {
            self.sem_items.release();
        }
    }

    /// 큐가 닫혔는지 확인
    #[inline]
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Relaxed)
    }

    /// 모든 메시지 제거
    pub fn clear(&self) {
        let mut queue = self.queue.lock();
        let count = queue.len();
        queue.clear();
        self.count.store(0, Ordering::Release);
        
        // 세마포어 카운트 조정
        for _ in 0..count {
            let _ = self.sem_items.try_acquire();
        }
    }
}

impl<T> Default for MessageQueue<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// 용량 제한 메시지 큐
///
/// 최대 용량을 초과하면 송신이 블로킹되거나 실패
pub struct BoundedMessageQueue<T> {
    /// 내부 큐
    queue: Mutex<VecDeque<Message<T>>>,
    /// 최대 용량
    capacity: usize,
    /// 현재 메시지 수
    count: AtomicUsize,
    /// 큐 닫힘 상태
    closed: AtomicBool,
    /// 사용 가능한 슬롯 (송신자용)
    sem_slots: Semaphore,
    /// 사용 가능한 메시지 (수신자용)
    sem_items: Semaphore,
}

unsafe impl<T: Send> Send for BoundedMessageQueue<T> {}
unsafe impl<T: Send> Sync for BoundedMessageQueue<T> {}

impl<T> BoundedMessageQueue<T> {
    /// 용량 지정 메시지 큐 생성
    pub fn new(capacity: usize) -> Self {
        Self {
            queue: Mutex::new(VecDeque::with_capacity(capacity)),
            capacity,
            count: AtomicUsize::new(0),
            closed: AtomicBool::new(false),
            sem_slots: Semaphore::new(capacity as isize),
            sem_items: Semaphore::new(0),
        }
    }

    /// 메시지 송신 (블로킹)
    ///
    /// 큐가 가득 차면 공간이 생길 때까지 대기
    pub fn send(&self, data: T) -> Result<(), MessageQueueError> {
        self.send_message(Message::new(data))
    }

    /// 메시지 객체 송신 (블로킹)
    pub fn send_message(&self, msg: Message<T>) -> Result<(), MessageQueueError> {
        if self.closed.load(Ordering::Relaxed) {
            return Err(MessageQueueError::Closed);
        }

        // 슬롯 획득 대기
        self.sem_slots.acquire();

        if self.closed.load(Ordering::Relaxed) {
            self.sem_slots.release();
            return Err(MessageQueueError::Closed);
        }

        {
            let mut queue = self.queue.lock();
            queue.push_back(msg);
        }

        self.count.fetch_add(1, Ordering::Release);
        self.sem_items.release();
        Ok(())
    }

    /// 메시지 송신 시도 (논블로킹)
    pub fn try_send(&self, data: T) -> Result<(), MessageQueueError> {
        self.try_send_message(Message::new(data))
    }

    /// 메시지 객체 송신 시도 (논블로킹)
    pub fn try_send_message(&self, msg: Message<T>) -> Result<(), MessageQueueError> {
        if self.closed.load(Ordering::Relaxed) {
            return Err(MessageQueueError::Closed);
        }

        if !self.sem_slots.try_acquire() {
            return Err(MessageQueueError::Full);
        }

        {
            let mut queue = self.queue.lock();
            queue.push_back(msg);
        }

        self.count.fetch_add(1, Ordering::Release);
        self.sem_items.release();
        Ok(())
    }

    /// 메시지 수신 (블로킹)
    pub fn receive(&self) -> Result<Message<T>, MessageQueueError> {
        if self.closed.load(Ordering::Relaxed) && self.is_empty() {
            return Err(MessageQueueError::Closed);
        }

        self.sem_items.acquire();

        if self.closed.load(Ordering::Relaxed) && self.is_empty() {
            return Err(MessageQueueError::Closed);
        }

        let msg = {
            let mut queue = self.queue.lock();
            queue.pop_front()
        };

        if let Some(msg) = msg {
            self.count.fetch_sub(1, Ordering::Release);
            self.sem_slots.release();
            Ok(msg)
        } else {
            Err(MessageQueueError::Empty)
        }
    }

    /// 메시지 수신 시도 (논블로킹)
    pub fn try_receive(&self) -> Result<Message<T>, MessageQueueError> {
        if self.closed.load(Ordering::Relaxed) && self.is_empty() {
            return Err(MessageQueueError::Closed);
        }

        if !self.sem_items.try_acquire() {
            return Err(MessageQueueError::Empty);
        }

        let msg = {
            let mut queue = self.queue.lock();
            queue.pop_front()
        };

        if let Some(msg) = msg {
            self.count.fetch_sub(1, Ordering::Release);
            self.sem_slots.release();
            Ok(msg)
        } else {
            Err(MessageQueueError::Empty)
        }
    }

    /// 큐에 있는 메시지 수
    #[inline]
    pub fn len(&self) -> usize {
        self.count.load(Ordering::Relaxed)
    }

    /// 최대 용량
    #[inline]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// 큐가 비어있는지 확인
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 큐가 가득 찼는지 확인
    #[inline]
    pub fn is_full(&self) -> bool {
        self.len() >= self.capacity
    }

    /// 큐 닫기
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
        // 대기 중인 스레드들 깨우기
        for _ in 0..self.capacity {
            self.sem_slots.release();
            self.sem_items.release();
        }
    }

    /// 큐가 닫혔는지 확인
    #[inline]
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Relaxed)
    }
}

/// 채널 (Go 스타일)
///
/// 송신자와 수신자 분리
pub struct Channel<T: 'static> {
    inner: BoundedMessageQueue<T>,
}

impl<T: 'static> Channel<T> {
    /// 버퍼 없는 채널 (동기식)
    pub fn unbuffered() -> (Sender<T>, Receiver<T>) {
        Self::bounded(1)
    }

    /// 버퍼 있는 채널
    pub fn bounded(capacity: usize) -> (Sender<T>, Receiver<T>) {
        let inner = Box::leak(Box::new(BoundedMessageQueue::new(capacity)));
        (
            Sender { queue: inner },
            Receiver { queue: inner },
        )
    }
}

/// 채널 송신자
pub struct Sender<T: 'static> {
    queue: &'static BoundedMessageQueue<T>,
}

impl<T: 'static> Sender<T> {
    /// 메시지 송신
    pub fn send(&self, data: T) -> Result<(), MessageQueueError> {
        self.queue.send(data)
    }

    /// 메시지 송신 시도 (논블로킹)
    pub fn try_send(&self, data: T) -> Result<(), MessageQueueError> {
        self.queue.try_send(data)
    }
}

impl<T: 'static> Clone for Sender<T> {
    fn clone(&self) -> Self {
        Self { queue: self.queue }
    }
}

/// 채널 수신자
pub struct Receiver<T: 'static> {
    queue: &'static BoundedMessageQueue<T>,
}

impl<T: 'static> Receiver<T> {
    /// 메시지 수신
    pub fn recv(&self) -> Result<Message<T>, MessageQueueError> {
        self.queue.receive()
    }

    /// 메시지 수신 시도 (논블로킹)
    pub fn try_recv(&self) -> Result<Message<T>, MessageQueueError> {
        self.queue.try_receive()
    }
}

/// 이름 기반 메시지 큐 관리자 (POSIX mq 스타일)
///
/// 전역적으로 이름으로 메시지 큐를 조회/생성
use crate::sync::RwLock;

/// 바이트 메시지 (범용)
pub type ByteMessage = Vec<u8>;

/// 글로벌 메시지 큐 레지스트리 - Vec으로 변경
static MESSAGE_QUEUES: RwLock<Option<Vec<(String, &'static MessageQueue<ByteMessage>)>>> = 
    RwLock::new(None);

/// 메시지 큐 열기/생성 (POSIX mq_open 스타일)
pub fn mq_open(name: &str, create: bool) -> Result<&'static MessageQueue<ByteMessage>, MessageQueueError> {
    // 먼저 읽기 락으로 조회
    {
        let queues = MESSAGE_QUEUES.read();
        if let Some(ref list) = *queues {
            if let Some((_, mq)) = list.iter().find(|(n, _)| n == name) {
                return Ok(*mq);
            }
        }
    }

    if !create {
        return Err(MessageQueueError::Empty);
    }

    // 쓰기 락으로 생성
    let mut queues = MESSAGE_QUEUES.write();
    
    // 초기화 안됐으면 초기화
    if queues.is_none() {
        *queues = Some(Vec::new());
    }

    let list = queues.as_mut().unwrap();
    
    // 다시 확인 (다른 스레드가 생성했을 수 있음)
    if let Some((_, mq)) = list.iter().find(|(n, _)| n == name) {
        return Ok(*mq);
    }

    // 새로 생성
    let mq = Box::leak(Box::new(MessageQueue::new()));
    list.push((String::from(name), mq));
    Ok(mq)
}

/// 메시지 큐 삭제 (POSIX mq_unlink 스타일)
pub fn mq_unlink(name: &str) -> Result<(), MessageQueueError> {
    let mut queues = MESSAGE_QUEUES.write();
    
    if let Some(ref mut list) = *queues {
        if let Some(pos) = list.iter().position(|(n, _)| n == name) {
            list.remove(pos);
            // 실제 메모리 해제는 하지 않음 (leak 됨)
            // 실제 구현에서는 참조 카운팅 필요
            return Ok(());
        }
    }
    
    Err(MessageQueueError::Empty)
}

/// 메시지 송신 (POSIX mq_send 스타일)
pub fn mq_send(name: &str, msg: &[u8]) -> Result<(), MessageQueueError> {
    let mq = mq_open(name, false)?;
    mq.send(msg.to_vec())
}

/// 메시지 수신 (POSIX mq_receive 스타일)
pub fn mq_receive(name: &str) -> Result<Vec<u8>, MessageQueueError> {
    let mq = mq_open(name, false)?;
    mq.receive().map(|m| m.data)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_basic_send_receive() {
        let mq: MessageQueue<i32> = MessageQueue::new();
        
        mq.send(1).unwrap();
        mq.send(2).unwrap();
        mq.send(3).unwrap();
        
        assert_eq!(mq.len(), 3);
        
        assert_eq!(mq.receive().unwrap().data, 1);
        assert_eq!(mq.receive().unwrap().data, 2);
        assert_eq!(mq.receive().unwrap().data, 3);
        
        assert!(mq.is_empty());
    }

    fn test_bounded_queue() {
        let mq: BoundedMessageQueue<i32> = BoundedMessageQueue::new(2);
        
        mq.try_send(1).unwrap();
        mq.try_send(2).unwrap();
        
        // 가득 참
        assert!(mq.try_send(3).is_err());
        
        // 하나 수신
        assert_eq!(mq.receive().unwrap().data, 1);
        
        // 이제 송신 가능
        mq.try_send(3).unwrap();
    }

    fn test_priority() {
        let mq: MessageQueue<&str> = MessageQueue::new();
        
        mq.send_priority("low", Priority::Low).unwrap();
        mq.send_priority("normal", Priority::Normal).unwrap();
        mq.send_priority("urgent", Priority::Urgent).unwrap();
        
        // Urgent가 먼저 나옴
        assert_eq!(mq.receive().unwrap().data, "urgent");
    }
}
