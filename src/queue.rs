use std::{
    mem::MaybeUninit,
    sync::{
        Arc,
        atomic::{self, AtomicUsize},
    },
};

use lockfree::queue::Queue;
use log::{error, warn};
use std::sync::atomic::{AtomicPtr, Ordering};

/// A lock-free queue implementation with optional element counting.
///
/// This queue provides thread-safe enqueue and dequeue operations without
/// using locks. It optionally tracks the number of elements using an atomic
/// counter with a baseline offset to handle underflow.
///
/// # Type Parameters
/// * `T` - The type of elements stored in the queue
#[derive(Debug)]
pub struct LocklessQueue<T> {
    /// Optional atomic counter tracking queue size with baseline offset
    count: Option<Arc<AtomicUsize>>,
    /// The underlying lock-free queue storing MaybeUninit elements
    q: Arc<Queue<MaybeUninit<T>>>,
}
impl<T> Default for LocklessQueue<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// Baseline offset for the counter to handle underflow in concurrent operations.
/// The counter starts at this value and actual count = counter - BASELINE.
const BASELINE: usize = 1_000_000;

impl<T> LocklessQueue<T> {
    /// Creates a new lockless queue without element counting.
    ///
    /// # Returns
    /// A new empty queue without a counter
    pub fn new() -> Self {
        let q = Queue::new();
        Self {
            count: None,
            q: Arc::new(q),
        }
    }

    /// Creates a new lockless queue with element counting enabled.
    ///
    /// The counter uses a baseline offset to prevent underflow during
    /// concurrent operations where dequeue might decrement before enqueue increments.
    ///
    /// # Returns
    /// A new empty queue with an atomic counter initialized to BASELINE
    pub fn new_with_counter() -> Self {
        Self {
            count: Some(Arc::new(AtomicUsize::new(BASELINE))),
            q: Arc::new(Queue::new()),
        }
    }

    /// Adds an element to the back of the queue.
    ///
    /// This operation is lock-free and thread-safe. If counting is enabled,
    /// the counter is atomically incremented.
    ///
    /// # Arguments
    /// * `value` - The element to add to the queue
    pub fn enqueue(&self, value: T) {
        self.q.push(MaybeUninit::new(value));
        if let Some(counter) = self.count.as_ref() {
            counter.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// Removes and returns an element from the front of the queue.
    ///
    /// If counting is enabled, decrements the counter. If the counter goes
    /// below the baseline (underflow), it's reset to the baseline.
    ///
    /// # Returns
    /// `Some(T)` if an element was available, `None` if the queue was empty
    ///
    /// # Safety
    /// Uses `assume_init()` on values that were initialized via `MaybeUninit::new()`
    pub fn dequeue(&self) -> Option<T> {
        // Update counter if tracking is enabled
        if let Some(counter) = self.count.as_ref() {
            let old_value = counter.fetch_sub(1, Ordering::SeqCst);
            // Reset if underflow occurred
            if old_value < BASELINE {
                counter.store(BASELINE, Ordering::SeqCst);
            }
        }
        self.q.pop().map(|x| unsafe { x.assume_init() })
    }

    /// Returns the approximate number of elements in the queue.
    ///
    /// This is only available if the queue was created with counting enabled.
    /// Due to concurrent operations, the count may be slightly inaccurate.
    ///
    /// # Returns
    /// * `Some(usize)` - The approximate queue size if counting is enabled
    /// * `None` - If counting is not enabled
    pub fn len(&self) -> Option<usize> {
        let counter = self.count.as_ref()?;
        let mut count = counter.load(Ordering::SeqCst);
        // Handle underflow by resetting to baseline
        if count < BASELINE {
            counter.store(BASELINE, Ordering::SeqCst);
            count = 0;
        } else {
            count -= BASELINE;
        }
        Some(count)
    }

    /// Checks if the queue is empty.
    ///
    /// If counting is not enabled, returns `false` as the state is unknown.
    ///
    /// # Returns
    /// `true` if the queue is empty (when counting is enabled), otherwise `false`
    pub fn is_empty(&self) -> bool {
        if let Some(n) = self.len() {
            n == 0
        } else {
            false
        }
    }
}

impl<T> Clone for LocklessQueue<T> {
    fn clone(&self) -> Self {
        Self {
            q: self.q.clone(),
            count: self.count.clone(),
        }
    }
}

/// A ring of lock-free queues for batched concurrent processing.
///
/// This structure provides a way to concurrently enqueue items while
/// allowing batched dequeuing. It uses two underlying queues and switches
/// between them, enabling one queue to be drained while the other accepts
/// new items.
///
/// This is useful for batching operations where you want to process groups
/// of items together without blocking producers.
#[derive(Debug)]
pub struct QueueRing<T> {
    /// Index of the next queue to use for dequeuing
    next_i: Arc<AtomicUsize>,
    /// Length counters for each queue in the ring
    l_len: [Arc<AtomicUsize>; 2],
    /// Array of two lock-free queues
    l_q: [LocklessQueue<T>; 2],
}
impl<T> Clone for QueueRing<T> {
    fn clone(&self) -> Self {
        let l_len = std::array::from_fn(|i| self.l_len[i].clone());
        let l_q = std::array::from_fn(|i| self.l_q[i].clone());
        Self {
            next_i: self.next_i.clone(),
            l_len,
            l_q,
        }
    }
}
impl<T> Default for QueueRing<T> {
    fn default() -> Self {
        let l_q = std::array::from_fn(|_i| LocklessQueue::new());
        let l_len = std::array::from_fn(|_i| Arc::new(AtomicUsize::new(0)));
        Self {
            next_i: Arc::new(AtomicUsize::new(0)),
            l_len,
            l_q,
        }
    }
}
impl<T> QueueRing<T> {
    /// Adds an element to the currently active queue.
    ///
    /// Items are enqueued to whichever queue is currently accepting new elements.
    /// This operation is thread-safe and lock-free.
    ///
    /// # Arguments
    /// * `value` - The element to add to the queue ring
    pub fn enqueue(&self, value: T) {
        let k = self.next_i.load(Ordering::Relaxed) % self.l_q.len();
        self.l_len[k].fetch_add(1, Ordering::SeqCst);
        self.l_q[k].enqueue(value);
    }

    /// Returns an iterator over a batch of items from one of the queues.
    ///
    /// This switches to the next queue in the ring and returns an iterator
    /// over all items currently in that queue. The queue's length counter
    /// is swapped to 0, though there may be a slight undercount due to
    /// concurrent operations.
    ///
    /// # Returns
    /// A `QueueIterator` that will drain one of the underlying queues
    pub fn dequeue(&self) -> QueueIterator<T> {
        let old_k = self.next_i.fetch_add(1, Ordering::SeqCst) % self.l_q.len();
        // There is a slight chance the length will undercount due to concurrent enqueues
        let length = self.l_len[old_k].swap(0, Ordering::SeqCst);
        let q = &self.l_q[old_k];
        QueueIterator {
            q: q.clone(),
            len: length,
            saftey_count: 0,
        }
    }
}

/// Iterator over a batch of items from a queue.
///
/// This iterator drains items from a single queue in the ring, providing
/// access to approximately `len()` items. It retries a few times when the
/// queue appears empty to handle race conditions.
pub struct QueueIterator<T> {
    /// Expected number of items (may be slightly inaccurate)
    len: usize,
    /// Number of items actually yielded (for validation)
    saftey_count: usize,
    /// The underlying queue being drained
    q: LocklessQueue<T>,
}

impl<T> QueueIterator<T> {
    /// Returns an estimate of the number of items in this batch.
    ///
    /// Due to concurrent operations, this may not exactly match the number
    /// of items that will be yielded by the iterator.
    ///
    /// # Returns
    /// The estimated number of items to be iterated
    pub fn len(&self) -> usize {
        self.len
    }
}
impl<T> Iterator for QueueIterator<T> {
    type Item = T;

    /// Returns the next item from the queue.
    ///
    /// This attempts to dequeue an item. If the first attempt returns None,
    /// it retries up to 3 times to handle race conditions where items may
    /// still be in flight due to concurrent operations.
    ///
    /// # Returns
    /// * `Some(T)` - The next item if available
    /// * `None` - If the queue is empty after retries
    fn next(&mut self) -> Option<Self::Item> {
        match self.q.dequeue() {
            Some(x) => {
                self.saftey_count += 1;
                Some(x)
            }
            None => {
                // Queue appears empty, but retry a few times due to possible race conditions
                let mut tries = 0;
                while tries < 3 {
                    tries += 1;
                    if let Some(x) = self.q.dequeue() {
                        return Some(x);
                    }
                }
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use num_cpus;
    use std::{
        collections::VecDeque,
        thread::{self, sleep},
        time::Duration,
    };

    #[test]
    fn test_queue() {
        let q: LocklessQueue<i32> = LocklessQueue::new();
        let q2 = q.clone();
        let handle_checksum = thread::spawn(move || -> u64 {
            let mut tries = 0;
            let mut sum: u64 = 0;
            while tries < 10 {
                if let Some(r) = q2.dequeue() {
                    if 0 < r {
                        sum += r as u64;
                    } else {
                        break;
                    }
                } else {
                    sleep(Duration::from_secs(3));
                    tries += 1;
                }
            }
            if 10 <= tries { 0 } else { sum }
        });
        let mut handle_list = VecDeque::new();
        for _i in 0..num_cpus::get() {
            let q2 = q.clone();
            handle_list.push_back(thread::spawn(move || -> u64 {
                let mut checksum: u64 = 0;
                for i in 1..100_000 {
                    q2.enqueue(i);
                    checksum += i as u64;
                }
                checksum
            }));
        }
        let mut out_checksum: u64 = 0;
        while let Some(h) = handle_list.pop_front() {
            out_checksum += h.join().unwrap();
        }
        // tell the handle_checksum thread to stop dequeing.
        q.enqueue(-1);
        assert_eq!(handle_checksum.join().unwrap(), out_checksum);
    }
}
