//! 세대 번호가 붙은 유한 큐.
//!
//! 탐색할 때마다 세대를 올리고 큐를 비웁니다. 낡은 세대의 항목은 넣으려 해도
//! 거절되고, 꺼낼 때도 건너뜁니다. 큐가 차면 넣는 쪽이 잠들고, 그 역압이 파일
//! 읽기까지 거슬러 올라갑니다. Windows API를 부르지 않습니다.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::Duration;

/// 넣기나 꺼내기가 실패한 이유.
#[derive(Debug, PartialEq, Eq)]
pub enum QueueError {
    /// 큐가 닫혔습니다. 스레드를 끝낼 때입니다.
    Closed,
    /// 넣으려던 항목의 세대가 이미 지났습니다. 버리고 다음으로 넘어갑니다.
    Stale,
}

struct Entry<T> {
    epoch: u64,
    cost: usize,
    item: T,
}

struct Inner<T> {
    items: VecDeque<Entry<T>>,
    used: usize,
    epoch: u64,
    closed: bool,
}

/// 크기가 정해진 다중 생산자·다중 소비자 큐.
///
/// 크기는 "비용"의 합으로 잽니다. 패킷 큐는 바이트, 프레임 큐는 장 수를 비용으로 씁니다.
/// 비어 있는 큐에는 비용이 용량보다 큰 항목 하나도 들어갈 수 있습니다. 그렇지 않으면
/// 아주 큰 패킷 하나에 파이프라인 전체가 멈춥니다.
pub struct GenQueue<T> {
    inner: Mutex<Inner<T>>,
    capacity: AtomicUsize,
    not_empty: Condvar,
    not_full: Condvar,
}

impl<T> GenQueue<T> {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(Inner {
                items: VecDeque::new(),
                used: 0,
                epoch: 0,
                closed: false,
            }),
            capacity: AtomicUsize::new(capacity),
            not_empty: Condvar::new(),
            not_full: Condvar::new(),
        }
    }

    /// 항목을 넣습니다. 큐가 차 있으면 자리가 날 때까지 기다립니다.
    /// 기다리는 동안 세대가 바뀌거나 큐가 닫히면 항목을 버리고 돌아옵니다.
    pub fn push(&self, epoch: u64, cost: usize, item: T) -> Result<(), QueueError> {
        let mut g = self.inner.lock().unwrap();
        loop {
            if g.closed {
                return Err(QueueError::Closed);
            }
            if epoch != g.epoch {
                return Err(QueueError::Stale);
            }
            if g.items.is_empty() || g.used + cost <= self.capacity.load(Ordering::Relaxed) {
                break;
            }
            g = self.not_full.wait(g).unwrap();
        }
        g.used += cost;
        g.items.push_back(Entry { epoch, cost, item });
        drop(g);
        self.not_empty.notify_all();
        Ok(())
    }

    /// 현재 세대의 항목을 꺼냅니다. 비어 있으면 들어올 때까지 기다립니다.
    /// 돌려주는 값은 (세대, 항목)입니다.
    pub fn pop(&self) -> Result<(u64, T), QueueError> {
        let mut g = self.inner.lock().unwrap();
        loop {
            if g.closed {
                return Err(QueueError::Closed);
            }
            if let Some(e) = Self::take_front(&mut g) {
                drop(g);
                self.not_full.notify_all();
                return Ok((e.epoch, e.item));
            }
            g = self.not_empty.wait(g).unwrap();
        }
    }

    /// 기다리지 않고 꺼냅니다. 비어 있으면 `None`입니다.
    pub fn try_pop(&self) -> Option<(u64, T)> {
        let mut g = self.inner.lock().unwrap();
        let e = Self::take_front(&mut g)?;
        drop(g);
        self.not_full.notify_all();
        Some((e.epoch, e.item))
    }

    /// 맨 앞 항목을 꺼내지 않고 들여다봅니다.
    pub fn peek_with<R>(&self, f: impl FnOnce(&T) -> R) -> Option<R> {
        let g = self.inner.lock().unwrap();
        g.items
            .iter()
            .find(|e| e.epoch == g.epoch)
            .map(|e| f(&e.item))
    }

    /// 최대 `timeout`만큼 기다려 항목을 꺼냅니다.
    pub fn pop_timeout(&self, timeout: Duration) -> Result<Option<(u64, T)>, QueueError> {
        let mut g = self.inner.lock().unwrap();
        if g.closed {
            return Err(QueueError::Closed);
        }
        if let Some(e) = Self::take_front(&mut g) {
            drop(g);
            self.not_full.notify_all();
            return Ok(Some((e.epoch, e.item)));
        }
        let (mut g, _) = self.not_empty.wait_timeout(g, timeout).unwrap();
        if g.closed {
            return Err(QueueError::Closed);
        }
        let e = Self::take_front(&mut g);
        drop(g);
        if e.is_some() {
            self.not_full.notify_all();
        }
        Ok(e.map(|e| (e.epoch, e.item)))
    }

    /// 맨 앞의 현재 세대 항목을 꺼냅니다. 그 앞의 낡은 항목은 이 스레드에서 해제합니다.
    /// 낡은 항목의 비용은 `advance`할 때 이미 빼 두었습니다.
    fn take_front(g: &mut Inner<T>) -> Option<Entry<T>> {
        while let Some(e) = g.items.pop_front() {
            if e.epoch == g.epoch {
                g.used -= e.cost;
                return Some(e);
            }
        }
        None
    }

    /// 세대만 올립니다. 남은 항목은 소비하는 스레드가 꺼낼 때 버립니다.
    /// FFmpeg 객체를 담은 큐에서, 해제를 UI 스레드가 아닌 작업 스레드에 맡길 때 씁니다.
    pub fn advance(&self, new_epoch: u64) {
        let mut g = self.inner.lock().unwrap();
        g.epoch = new_epoch;
        g.used = 0;
        drop(g);
        self.not_empty.notify_all();
        self.not_full.notify_all();
    }

    /// 낡은 세대의 항목을 모두 꺼내 돌려줍니다. 호출한 스레드에서 해제됩니다.
    pub fn purge_stale(&self) -> Vec<T> {
        let mut g = self.inner.lock().unwrap();
        let epoch = g.epoch;
        let (keep, stale): (VecDeque<_>, VecDeque<_>) =
            g.items.drain(..).partition(|e| e.epoch == epoch);
        g.items = keep;
        drop(g);
        stale.into_iter().map(|e| e.item).collect()
    }

    /// 세대를 올리고 큐를 비웁니다. 기다리던 생산자와 소비자를 모두 깨웁니다.
    /// 버린 항목을 돌려주므로, 호출하는 쪽이 원하는 스레드에서 해제할 수 있습니다.
    pub fn flush(&self, new_epoch: u64) -> Vec<T> {
        let mut g = self.inner.lock().unwrap();
        g.epoch = new_epoch;
        g.used = 0;
        let dropped = g.items.drain(..).map(|e| e.item).collect();
        drop(g);
        self.not_empty.notify_all();
        self.not_full.notify_all();
        dropped
    }

    /// 큐를 닫습니다. 이후의 넣기와 꺼내기는 모두 `Closed`를 돌려줍니다.
    pub fn close(&self) {
        self.inner.lock().unwrap().closed = true;
        self.not_empty.notify_all();
        self.not_full.notify_all();
    }

    /// 용량을 바꿉니다. 기다리던 생산자를 깨웁니다.
    pub fn set_capacity(&self, capacity: usize) {
        self.capacity.store(capacity, Ordering::Relaxed);
        self.not_full.notify_all();
    }

    pub fn epoch(&self) -> u64 {
        self.inner.lock().unwrap().epoch
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().items.len()
    }

    /// 현재 쓰고 있는 비용의 합.
    pub fn used(&self) -> usize {
        self.inner.lock().unwrap().used
    }

    #[cfg(test)]
    pub fn is_full(&self) -> bool {
        let g = self.inner.lock().unwrap();
        !g.items.is_empty() && g.used >= self.capacity.load(Ordering::Relaxed)
    }
}

/// 비트레이트(bit/s)로 `seconds`초 분량의 바이트를 `[min, max]` 안에서 정합니다.
/// 비트레이트를 모르면 `max`를 씁니다. 로컬 파일은 많이 미리 읽을 필요가 없습니다.
pub fn read_ahead_bytes(bit_rate: i64, seconds: f64, min: usize, max: usize) -> usize {
    if bit_rate <= 0 {
        return max;
    }
    ((bit_rate as f64 / 8.0 * seconds) as usize).clamp(min, max)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn read_ahead_is_clamped() {
        assert_eq!(read_ahead_bytes(0, 2.0, 10, 100), 100);
        assert_eq!(read_ahead_bytes(8_000, 2.0, 10, 100_000), 2_000);
        assert_eq!(read_ahead_bytes(8, 2.0, 10, 100), 10);
        assert_eq!(read_ahead_bytes(800_000_000, 2.0, 10, 100), 100);
    }

    #[test]
    fn raising_capacity_wakes_producer() {
        let q = Arc::new(GenQueue::new(1));
        q.push(0, 1, 1).unwrap();
        let q2 = q.clone();
        let t = thread::spawn(move || q2.push(0, 1, 2));
        thread::sleep(Duration::from_millis(30));
        q.set_capacity(2);
        assert_eq!(t.join().unwrap(), Ok(()));
    }

    #[test]
    fn push_pop_in_order() {
        let q = GenQueue::new(10);
        q.push(0, 1, 'a').unwrap();
        q.push(0, 1, 'b').unwrap();
        assert_eq!(q.pop().unwrap(), (0, 'a'));
        assert_eq!(q.pop().unwrap(), (0, 'b'));
        assert!(q.try_pop().is_none());
    }

    #[test]
    fn stale_push_is_rejected() {
        let q = GenQueue::new(10);
        q.flush(3);
        assert_eq!(q.push(2, 1, ()), Err(QueueError::Stale));
        assert_eq!(q.push(3, 1, ()), Ok(()));
    }

    #[test]
    fn flush_drops_items_and_resets_cost() {
        let q = GenQueue::new(10);
        q.push(0, 6, 1).unwrap();
        q.push(0, 4, 2).unwrap();
        assert!(q.is_full());
        let dropped = q.flush(1);
        assert_eq!(dropped, vec![1, 2]);
        assert_eq!(q.used(), 0);
        assert_eq!(q.len(), 0);
        assert_eq!(q.epoch(), 1);
    }

    #[test]
    fn oversize_item_fits_into_empty_queue() {
        let q = GenQueue::new(4);
        q.push(0, 100, ()).unwrap();
        assert_eq!(q.used(), 100);
    }

    #[test]
    fn full_queue_blocks_until_pop() {
        let q = Arc::new(GenQueue::new(2));
        q.push(0, 1, 1).unwrap();
        q.push(0, 1, 2).unwrap();
        let q2 = q.clone();
        let t = thread::spawn(move || q2.push(0, 1, 3));
        thread::sleep(Duration::from_millis(30));
        assert_eq!(q.len(), 2);
        assert_eq!(q.pop().unwrap().1, 1);
        assert_eq!(t.join().unwrap(), Ok(()));
        assert_eq!(q.len(), 2);
    }

    #[test]
    fn flush_wakes_blocked_producer_with_stale() {
        let q = Arc::new(GenQueue::new(1));
        q.push(0, 1, 1).unwrap();
        let q2 = q.clone();
        let t = thread::spawn(move || q2.push(0, 1, 2));
        thread::sleep(Duration::from_millis(30));
        q.flush(1);
        assert_eq!(t.join().unwrap(), Err(QueueError::Stale));
    }

    #[test]
    fn advance_keeps_stale_items_until_consumed() {
        let q = GenQueue::new(2);
        q.push(0, 1, 1).unwrap();
        q.push(0, 1, 2).unwrap();
        q.advance(1);
        // 낡은 항목이 남아 있어도 새 세대는 용량을 다 쓸 수 있습니다.
        q.push(1, 1, 3).unwrap();
        q.push(1, 1, 4).unwrap();
        assert_eq!(q.len(), 4);
        assert_eq!(q.pop().unwrap(), (1, 3));
        assert_eq!(q.used(), 1);
        assert_eq!(q.len(), 1);
    }

    #[test]
    fn purge_stale_returns_only_old_items() {
        let q = GenQueue::new(4);
        q.push(0, 1, 1).unwrap();
        q.advance(1);
        q.push(1, 1, 2).unwrap();
        assert_eq!(q.purge_stale(), vec![1]);
        assert_eq!(q.len(), 1);
        assert_eq!(q.used(), 1);
    }

    #[test]
    fn close_wakes_blocked_consumer() {
        let q: Arc<GenQueue<u8>> = Arc::new(GenQueue::new(1));
        let q2 = q.clone();
        let t = thread::spawn(move || q2.pop());
        thread::sleep(Duration::from_millis(30));
        q.close();
        assert_eq!(t.join().unwrap(), Err(QueueError::Closed));
    }

    #[test]
    fn pop_timeout_returns_none_when_empty() {
        let q: GenQueue<u8> = GenQueue::new(1);
        assert_eq!(q.pop_timeout(Duration::from_millis(5)), Ok(None));
    }

    #[test]
    fn peek_does_not_remove() {
        let q = GenQueue::new(3);
        q.push(0, 1, 7).unwrap();
        assert_eq!(q.peek_with(|v| *v), Some(7));
        assert_eq!(q.len(), 1);
    }
}
