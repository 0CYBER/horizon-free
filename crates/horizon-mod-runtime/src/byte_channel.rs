//! Producers that never wait (or, with [`Sender::reserve_waiting`], wait for room) and a
//! blocking consumer sharing a byte budget.
//!
//! Charge each value's owned heap allocations (for a `Vec`, its capacity, not length).
//! The channel adds inline value and queue-node storage to that charge. Allocator
//! metadata is not included, but both allocated bytes and node count are bounded.
//! Values held before sending or after ordinary receiving are outside the budget.
//! Owned receiving transfers the original charge to a lease retained by the consumer.
//! One control value at a time ([`Sender::reserve_over_budget`]) is charged to its own
//! allowance beside the budget, so it never narrows the room ordinary values have.
use std::collections::LinkedList;
use std::sync::mpsc::{RecvError, RecvTimeoutError, TryRecvError};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// A consistent snapshot of queue pressure, including its lifetime high-water mark.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stats {
    // Includes unpublished permits, queued values and owned receive leases.
    pub reserved_bytes: usize,
    pub peak_reserved_bytes: usize,
    /// Published values only; unpublished permits are not receiver-visible.
    pub queued_items: usize,
}

/// The capacity cannot hold even one empty value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidCapacity;

/// Refusal returns ownership; callers decide whether to retry or disconnect.
#[derive(Debug, PartialEq, Eq)]
pub enum TrySendError<T> {
    Full(T),
    Disconnected(T),
}

/// A nonblocking reservation refusal, before a value exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReserveError {
    Full,
    Disconnected,
}

/// Charged capacity awaiting publication. Dropping it cancels the reservation.
/// A permit is not a producer: the last Sender drop cancels all pending permits.
/// Publication after that or receiver closure returns the supplied value.
pub struct Permit<T> {
    shared: Arc<Shared<T>>,
    /// Zero bytes once published.
    charge: Charge,
}

/// A value's charge, and whether the control allowance holds it rather than the budget.
#[derive(Clone, Copy)]
struct Charge {
    bytes: usize,
    control: bool,
}

/// A dequeued value whose original charge stays reserved until its lease drops.
/// Unlike unpublished permits, held receipts survive either endpoint closing.
pub struct OwnedReceipt<T> {
    value: T,
    lease: ReceiveLease<T>,
}
impl<T> OwnedReceipt<T> {
    /// Transfer the value and credit independently without any refund/re-reserve gap.
    pub fn into_parts(self) -> (T, ReceiveLease<T>) {
        (self.value, self.lease)
    }
}
/// Opaque retained receive credit. Drop refunds even after endpoint closure.
pub struct ReceiveLease<T> {
    shared: Arc<Shared<T>>,
    charge: Charge,
}
impl<T> Drop for ReceiveLease<T> {
    fn drop(&mut self) {
        let notify = {
            let mut state = self.shared.lock();
            state.held_bytes -= self.charge.bytes;
            if self.charge.control {
                state.control_held -= self.charge.bytes;
            }
            state.release(self.charge);
            state.room_waiters > 0
        };
        if notify {
            self.shared.room.notify_all();
        }
    }
}

struct State<T> {
    pending_bytes: usize,
    held_bytes: usize,
    queue: LinkedList<(T, Charge)>,
    stats: Stats,
    senders: usize,
    receiver_alive: bool,
    room_waiters: usize,
    ready_waiters: usize,
    /// The control allowance's share of `stats.reserved_bytes` (at most one value's
    /// charge), and of that the part still pending and the part held in a lease.
    control_bytes: usize,
    control_pending: usize,
    control_held: usize,
}
impl<T> State<T> {
    /// Ordinary reservations' room: the control allowance doesn't take from it.
    fn room(&self, capacity: usize) -> usize {
        capacity.saturating_sub(self.stats.reserved_bytes - self.control_bytes)
    }
    fn release(&mut self, charge: Charge) {
        self.stats.reserved_bytes -= charge.bytes;
        if charge.control {
            self.control_bytes -= charge.bytes;
        }
    }
}
struct Shared<T> {
    capacity: usize,
    state: Mutex<State<T>>,
    ready: Condvar,
    /// Signalled whenever reserved bytes are released or the receiver closes.
    room: Condvar,
}
impl<T> Shared<T> {
    fn lock(&self) -> MutexGuard<'_, State<T>> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Many producers share the same connection budget.
pub struct Sender<T>(Arc<Shared<T>>);
/// Single consumer; queued values are freed when it is dropped.
pub struct Receiver<T>(Arc<Shared<T>>);

/// Fixed per-message charge, including list links, inline value and byte charge.
/// This is nonzero even for zero-sized values and empty payloads.
pub const fn message_overhead<T>() -> usize {
    size_of::<((T, Charge), [usize; 2])>()
}

/// Creates a channel. Capacity is connection pressure, not a per-body protocol cap.
pub fn channel<T>(capacity: usize) -> Result<(Sender<T>, Receiver<T>), InvalidCapacity> {
    if capacity < message_overhead::<T>() {
        return Err(InvalidCapacity);
    }
    let shared = Arc::new(Shared {
        capacity,
        state: Mutex::new(State {
            pending_bytes: 0,
            held_bytes: 0,
            queue: LinkedList::new(),
            stats: Stats {
                reserved_bytes: 0,
                peak_reserved_bytes: 0,
                queued_items: 0,
            },
            senders: 1,
            receiver_alive: true,
            room_waiters: 0,
            ready_waiters: 0,
            control_bytes: 0,
            control_pending: 0,
            control_held: 0,
        }),
        ready: Condvar::new(),
        room: Condvar::new(),
    });
    Ok((Sender(shared.clone()), Receiver(shared)))
}

impl<T> Sender<T> {
    /// Never waits for capacity. Overflowing charges are refused as full.
    /// `owned_bytes` must cover all heap storage retained by `value`.
    pub fn try_send(&self, value: T, owned_bytes: usize) -> Result<(), TrySendError<T>> {
        match self.reserve(owned_bytes) {
            Ok(permit) => permit.publish(value).map_err(TrySendError::Disconnected),
            Err(ReserveError::Full) => Err(TrySendError::Full(value)),
            Err(ReserveError::Disconnected) => Err(TrySendError::Disconnected(value)),
        }
    }
    /// Charges owned heap bytes plus inline value and node overhead immediately.
    /// Never waits for capacity. The future value must fit this declared charge.
    pub fn reserve(&self, owned_bytes: usize) -> Result<Permit<T>, ReserveError> {
        let mut state = self.0.lock();
        if !state.receiver_alive {
            return Err(ReserveError::Disconnected);
        }
        let charge = owned_bytes
            .checked_add(message_overhead::<T>())
            .ok_or(ReserveError::Full)?;
        if charge > state.room(self.0.capacity) {
            return Err(ReserveError::Full);
        }
        self.take(&mut state, charge, false)
    }
    /// Like [`Self::reserve`], but waits until released charges make room. Waits
    /// without a deadline, so the consumer must keep receiving and dropping leases.
    /// `Full` here means the charge exceeds the whole capacity and could never fit;
    /// `Disconnected` means the receiver closed, before or while waiting.
    pub fn reserve_waiting(&self, owned_bytes: usize) -> Result<Permit<T>, ReserveError> {
        let charge = owned_bytes
            .checked_add(message_overhead::<T>())
            .filter(|&charge| charge <= self.0.capacity)
            .ok_or(ReserveError::Full)?;
        let mut state = self.0.lock();
        loop {
            if !state.receiver_alive {
                return Err(ReserveError::Disconnected);
            }
            if charge <= state.room(self.0.capacity) {
                return self.take(&mut state, charge, false);
            }
            state.room_waiters += 1;
            state = self.0.room.wait(state).unwrap_or_else(|e| e.into_inner());
            state.room_waiters -= 1;
        }
    }
    /// Admits a control value that must reach the consumer in order, however full the
    /// budget is. It is charged to a separate allowance that holds one value at a time,
    /// so ordinary room is exactly what it would be without it. While another control
    /// value's charge is held, this waits for its release, as [`Self::reserve_waiting`]
    /// waits for room. `Full` means the charge exceeds the whole capacity, or the total
    /// reserved bytes would overflow; `Disconnected` means the receiver closed.
    pub fn reserve_over_budget(&self, owned_bytes: usize) -> Result<Permit<T>, ReserveError> {
        let charge = owned_bytes
            .checked_add(message_overhead::<T>())
            .filter(|&charge| charge <= self.0.capacity)
            .ok_or(ReserveError::Full)?;
        let mut state = self.0.lock();
        loop {
            if !state.receiver_alive {
                return Err(ReserveError::Disconnected);
            }
            if state.control_bytes == 0 {
                return self.take(&mut state, charge, true);
            }
            state.room_waiters += 1;
            state = self.0.room.wait(state).unwrap_or_else(|e| e.into_inner());
            state.room_waiters -= 1;
        }
    }
    fn take(
        &self,
        state: &mut State<T>,
        bytes: usize,
        control: bool,
    ) -> Result<Permit<T>, ReserveError> {
        // Checked before any counter moves: the control allowance can take the total
        // past the capacity.
        let reserved = state
            .stats
            .reserved_bytes
            .checked_add(bytes)
            .ok_or(ReserveError::Full)?;
        state.stats.reserved_bytes = reserved;
        state.pending_bytes += bytes;
        if control {
            state.control_bytes += bytes;
            state.control_pending += bytes;
        }
        state.stats.peak_reserved_bytes = state.stats.peak_reserved_bytes.max(reserved);
        Ok(Permit {
            shared: self.0.clone(),
            charge: Charge { bytes, control },
        })
    }
    pub fn stats(&self) -> Stats {
        self.0.lock().stats
    }
}
impl<T> Permit<T> {
    /// Publishes exactly once without charging again. On closure returns ownership.
    pub fn publish(mut self, value: T) -> Result<(), T> {
        let notify = {
            let mut state = self.shared.lock();
            if !state.receiver_alive || state.senders == 0 {
                return Err(value);
            }
            state.queue.push_back((value, self.charge));
            state.pending_bytes -= self.charge.bytes;
            if self.charge.control {
                state.control_pending -= self.charge.bytes;
            }
            state.stats.queued_items += 1;
            self.charge.bytes = 0;
            state.ready_waiters > 0
        };
        if notify {
            self.shared.ready.notify_one();
        }
        Ok(())
    }
}
impl<T> Drop for Permit<T> {
    fn drop(&mut self) {
        let notify = {
            let mut state = self.shared.lock();
            if state.receiver_alive && state.senders != 0 && self.charge.bytes != 0 {
                state.pending_bytes -= self.charge.bytes;
                if self.charge.control {
                    state.control_pending -= self.charge.bytes;
                }
                state.release(self.charge);
                state.room_waiters > 0
            } else {
                false
            }
        };
        if notify {
            self.shared.room.notify_all();
        }
    }
}
impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        self.0.lock().senders += 1;
        Self(self.0.clone())
    }
}
impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        let mut state = self.0.lock();
        state.senders -= 1;
        let closed = state.senders == 0;
        if closed {
            state.stats.reserved_bytes -= state.pending_bytes;
            state.control_bytes -= state.control_pending;
            state.pending_bytes = 0;
            state.control_pending = 0;
        }
        drop(state);
        if closed {
            self.0.ready.notify_all();
            self.0.room.notify_all();
        }
    }
}
impl<T> Receiver<T> {
    fn pop(state: &mut State<T>) -> Option<T> {
        let (value, charge) = state.queue.pop_front()?;
        state.release(charge);
        state.stats.queued_items -= 1;
        Some(value)
    }
    fn pop_owned(&self, state: &mut State<T>) -> Option<OwnedReceipt<T>> {
        let (value, charge) = state.queue.pop_front()?;
        state.stats.queued_items -= 1;
        state.held_bytes += charge.bytes;
        if charge.control {
            state.control_held += charge.bytes;
        }
        Some(OwnedReceipt {
            value,
            lease: ReceiveLease {
                shared: self.0.clone(),
                charge,
            },
        })
    }
    /// Dequeue without refunding reserved bytes. Published count decreases immediately;
    /// the returned receipt owns credit until dropped or transferred with `into_parts`.
    pub fn try_recv_owned(&self) -> Result<OwnedReceipt<T>, TryRecvError> {
        let mut state = self.0.lock();
        self.pop_owned(&mut state).ok_or(if state.senders == 0 {
            TryRecvError::Disconnected
        } else {
            TryRecvError::Empty
        })
    }
    /// Blocking owned receive. Last-sender drop still permits draining published values;
    /// held receipts remain charged independently of producer/receiver lifetime.
    pub fn recv_owned(&self) -> Result<OwnedReceipt<T>, RecvError> {
        let mut state = self.0.lock();
        loop {
            if let Some(receipt) = self.pop_owned(&mut state) {
                return Ok(receipt);
            }
            if state.senders == 0 {
                return Err(RecvError);
            }
            state.ready_waiters += 1;
            state = self.0.ready.wait(state).unwrap_or_else(|e| e.into_inner());
            state.ready_waiters -= 1;
        }
    }
    pub fn stats(&self) -> Stats {
        self.0.lock().stats
    }
    pub fn try_recv(&self) -> Result<T, TryRecvError> {
        let (value, notify) = {
            let mut state = self.0.lock();
            let value = Self::pop(&mut state).ok_or(if state.senders == 0 {
                TryRecvError::Disconnected
            } else {
                TryRecvError::Empty
            });
            let notify = value.is_ok() && state.room_waiters > 0;
            (value, notify)
        };
        if notify {
            self.0.room.notify_all();
        }
        value
    }
    pub fn recv(&self) -> Result<T, RecvError> {
        let mut state = self.0.lock();
        loop {
            if let Some(value) = Self::pop(&mut state) {
                let notify = state.room_waiters > 0;
                drop(state);
                if notify {
                    self.0.room.notify_all();
                }
                return Ok(value);
            }
            if state.senders == 0 {
                return Err(RecvError);
            }
            state.ready_waiters += 1;
            state = self.0.ready.wait(state).unwrap_or_else(|e| e.into_inner());
            state.ready_waiters -= 1;
        }
    }
    pub fn recv_timeout(&self, timeout: Duration) -> Result<T, RecvTimeoutError> {
        self.recv_timeout_owned(timeout)
            .map(|receipt| receipt.into_parts().0)
    }
    /// Timed receive that transfers, rather than refunds, the input's credit.
    /// A zero timeout still receives an already published value.
    pub fn recv_timeout_owned(
        &self,
        timeout: Duration,
    ) -> Result<OwnedReceipt<T>, RecvTimeoutError> {
        let start = Instant::now();
        let mut state = self.0.lock();
        loop {
            if let Some(receipt) = self.pop_owned(&mut state) {
                return Ok(receipt);
            }
            if state.senders == 0 {
                return Err(RecvTimeoutError::Disconnected);
            }
            let Some(remaining) = timeout.checked_sub(start.elapsed()) else {
                return Err(RecvTimeoutError::Timeout);
            };
            state.ready_waiters += 1;
            state = self
                .0
                .ready
                .wait_timeout(state, remaining)
                .unwrap_or_else(|e| e.into_inner())
                .0;
            state.ready_waiters -= 1;
        }
    }
}
impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        let queue = {
            let mut state = self.0.lock();
            state.receiver_alive = false;
            state.pending_bytes = 0;
            state.stats.reserved_bytes = state.held_bytes;
            state.control_pending = 0;
            state.control_bytes = state.control_held;
            state.stats.queued_items = 0;
            std::mem::take(&mut state.queue)
        };
        self.0.room.notify_all();
        // Arbitrary value destructors must run outside the lock.
        drop(queue);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn owned_dequeue_keeps_credit_until_lease_drop() {
        let charge = message_overhead::<Vec<u8>>() + 100;
        let (tx, rx) = channel::<Vec<u8>>(charge).unwrap();
        tx.try_send(Vec::with_capacity(100), 100).unwrap();
        let receipt = rx.try_recv_owned().unwrap();
        assert_eq!(rx.stats().queued_items, 0);
        assert_eq!(rx.stats().reserved_bytes, charge);
        assert!(matches!(tx.reserve(0), Err(ReserveError::Full)));
        let (value, lease) = receipt.into_parts();
        assert_eq!(value.capacity(), 100);
        drop(value);
        assert_eq!(tx.stats().reserved_bytes, charge);
        drop(lease);
        assert_eq!(tx.stats().reserved_bytes, 0);
        tx.try_send(Vec::new(), 0).unwrap();
        drop(rx.recv_owned().unwrap());
        assert_eq!(tx.stats().reserved_bytes, 0);
    }

    #[test]
    fn owned_receipts_survive_both_endpoint_closures() {
        let (tx, rx) = channel::<()>(1024).unwrap();
        tx.try_send((), 0).unwrap();
        let receipt = rx.recv_owned().unwrap();
        let pending = tx.reserve(10).unwrap();
        tx.try_send((), 0).unwrap();
        drop(rx);
        assert_eq!(tx.stats().reserved_bytes, message_overhead::<()>());
        drop(pending);
        let shared = tx.0.clone();
        drop(tx);
        assert_eq!(shared.lock().stats.reserved_bytes, message_overhead::<()>());
        drop(receipt);
        assert_eq!(shared.lock().stats.reserved_bytes, 0);

        let (tx, rx) = channel::<()>(1024).unwrap();
        tx.try_send((), 0).unwrap();
        drop(tx);
        let receipt = rx.recv_owned().unwrap();
        assert!(matches!(
            rx.try_recv_owned(),
            Err(TryRecvError::Disconnected)
        ));
        assert_eq!(rx.stats().reserved_bytes, message_overhead::<()>());
        drop(receipt);
        assert_eq!(rx.stats().reserved_bytes, 0);
    }

    #[test]
    fn frozen_published_count_excludes_receipts_and_concurrent_replies() {
        let (tx, rx) = channel::<u32>(4096).unwrap();
        tx.try_send(1, 0).unwrap();
        tx.try_send(2, 0).unwrap();
        let turn = rx.stats().queued_items;
        let (ready, go) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            for _ in 0..turn {
                go.recv().unwrap();
                tx.try_send(99, 0).unwrap();
            }
        });
        let mut held = Vec::new();
        for _ in 0..turn {
            held.push(rx.recv_owned().unwrap());
            ready.send(()).unwrap();
        }
        worker.join().unwrap();
        let (values, leases): (Vec<_>, Vec<_>) =
            held.into_iter().map(OwnedReceipt::into_parts).unzip();
        assert_eq!(values, [1, 2]);
        assert_eq!(rx.stats().queued_items, 2);
        assert_eq!(rx.stats().reserved_bytes, 4 * message_overhead::<u32>());
        drop(leases);
        assert_eq!(rx.stats().reserved_bytes, 2 * message_overhead::<u32>());
    }

    #[test]
    fn pending_publication_and_abort_share_budget() {
        let charge = message_overhead::<u32>() + 10;
        let (tx, rx) = channel(charge * 2).unwrap();
        let outer = tx.reserve(10).unwrap();
        let inner = tx.clone().reserve(10).unwrap();
        assert!(matches!(tx.reserve(0), Err(ReserveError::Full)));
        assert_eq!(tx.try_send(9, 0), Err(TrySendError::Full(9)));
        assert_eq!(rx.try_recv(), Err(TryRecvError::Empty));
        assert_eq!(tx.stats().reserved_bytes, charge * 2);
        assert_eq!(tx.stats().queued_items, 0);
        drop(inner);
        outer.publish(42).unwrap();
        assert_eq!(tx.stats().reserved_bytes, charge);
        assert_eq!(tx.stats().queued_items, 1);
        assert_eq!(rx.recv(), Ok(42));
        assert_eq!(tx.stats().reserved_bytes, 0);
        assert_eq!(tx.stats().peak_reserved_bytes, charge * 2);
    }

    #[test]
    fn permits_cancel_on_either_endpoint_close() {
        let (tx, rx) = channel::<u32>(1024).unwrap();
        let first = tx.reserve(10).unwrap();
        let second = tx.reserve(10).unwrap();
        drop(rx);
        assert_eq!(tx.stats().reserved_bytes, 0);
        assert_eq!(first.publish(1), Err(1));
        drop(second);
        assert_eq!(tx.stats().reserved_bytes, 0);
        assert!(matches!(tx.reserve(0), Err(ReserveError::Disconnected)));

        let (tx, rx) = channel::<u32>(1024).unwrap();
        tx.try_send(7, 0).unwrap();
        let first = tx.reserve(10).unwrap();
        let second = tx.reserve(10).unwrap();
        drop(tx);
        assert_eq!(rx.stats().reserved_bytes, message_overhead::<u32>());
        assert_eq!(first.publish(1), Err(1));
        drop(second);
        assert_eq!(rx.recv(), Ok(7));
        assert_eq!(rx.recv(), Err(RecvError));
        assert_eq!(rx.stats().reserved_bytes, 0);
    }

    #[test]
    fn concurrent_pending_flood_is_bounded() {
        for bytes in [0, 257] {
            let charge = message_overhead::<Vec<u8>>() + bytes;
            let (tx, rx) = channel::<Vec<u8>>(31 * charge).unwrap();
            let held = std::thread::scope(|scope| {
                let workers: Vec<_> = (0..8)
                    .map(|_| {
                        let tx = tx.clone();
                        scope.spawn(move || {
                            (0..1000)
                                .filter_map(|_| tx.reserve(bytes).ok())
                                .collect::<Vec<_>>()
                        })
                    })
                    .collect();
                workers
                    .into_iter()
                    .flat_map(|worker| worker.join().unwrap())
                    .collect::<Vec<_>>()
            });
            assert_eq!(held.len(), 31);
            assert_eq!(rx.try_recv(), Err(TryRecvError::Empty));
            assert_eq!(tx.stats().reserved_bytes, 31 * charge);
            assert_eq!(tx.stats().peak_reserved_bytes, 31 * charge);
            drop(held);
            assert_eq!(tx.stats().reserved_bytes, 0);
        }
    }

    #[test]
    fn concurrent_flood_bounds_heap_and_empty_messages() {
        for payload in [0, 257] {
            let charge = message_overhead::<Vec<u8>>() + payload;
            let capacity = charge * 31;
            let (tx, rx) = channel(capacity).unwrap();
            std::thread::scope(|scope| {
                for _ in 0..8 {
                    let tx = tx.clone();
                    scope.spawn(move || {
                        for _ in 0..10_000 {
                            let value = vec![0; payload];
                            let _ = tx.try_send(value, payload);
                        }
                    });
                }
            });
            assert_eq!(
                tx.stats(),
                Stats {
                    reserved_bytes: capacity,
                    peak_reserved_bytes: capacity,
                    queued_items: 31
                }
            );
            let mut retained_heap = 0;
            for _ in 0..31 {
                retained_heap += rx.try_recv().unwrap().capacity();
            }
            assert_eq!(retained_heap, payload * 31);
            assert_eq!(tx.stats().reserved_bytes, 0);
            tx.try_send(Vec::new(), 0).unwrap();
            assert!(rx.recv().unwrap().is_empty());
            assert_eq!(rx.try_recv(), Err(TryRecvError::Empty));
            assert_eq!(
                tx.try_send(Vec::new(), usize::MAX),
                Err(TrySendError::Full(Vec::new()))
            );
            assert_eq!(tx.stats().reserved_bytes, 0);
        }
    }

    #[test]
    fn disconnect_releases_values_and_budget() {
        struct Counted(Arc<AtomicUsize>);
        impl Drop for Counted {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        let drops = Arc::new(AtomicUsize::new(0));
        let (tx, rx) = channel::<Counted>(1024).unwrap();
        assert!(tx.try_send(Counted(drops.clone()), 0).is_ok());
        drop(rx);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert_eq!(tx.stats().reserved_bytes, 0);
        assert!(matches!(
            tx.try_send(Counted(drops.clone()), 0),
            Err(TrySendError::Disconnected(_))
        ));
        assert_eq!(drops.load(Ordering::SeqCst), 2);
        assert_eq!(tx.stats().queued_items, 0);
    }

    #[test]
    fn timed_owned_receive_keeps_credit_through_timeout_and_endpoint_close() {
        let capacity = message_overhead::<u32>() + 4;
        let (tx, rx) = channel::<u32>(capacity).unwrap();
        assert!(matches!(
            rx.recv_timeout_owned(Duration::ZERO),
            Err(RecvTimeoutError::Timeout)
        ));
        tx.try_send(42, 4).unwrap();
        let (value, credit) = rx.recv_timeout_owned(Duration::ZERO).unwrap().into_parts();
        assert_eq!(value, 42);
        assert_eq!(tx.stats().reserved_bytes, capacity);
        assert!(matches!(tx.reserve(4), Err(ReserveError::Full)));
        assert!(matches!(
            rx.recv_timeout_owned(Duration::ZERO),
            Err(RecvTimeoutError::Timeout)
        ));
        drop(rx);
        assert_eq!(tx.stats().reserved_bytes, capacity);
        drop(credit);
        assert_eq!(tx.stats().reserved_bytes, 0);
    }

    #[test]
    fn waiting_reserve_wakes_when_room_returns_or_the_receiver_closes() {
        let charge = message_overhead::<u32>() + 4;
        let (tx, rx) = channel::<u32>(charge).unwrap();
        let woken = Duration::from_secs(5);
        // Larger than the whole budget: refused at once instead of waiting forever.
        assert_eq!(
            waiting(&tx, charge, 0).recv_timeout(woken),
            Ok(Err(ReserveError::Full))
        );
        tx.try_send(1, 4).unwrap();
        let receipt = rx.recv_timeout_owned(woken).unwrap();
        let waiter = waiting(&tx, 4, 2);
        assert_waits(&waiter);
        drop(receipt);
        assert_eq!(waiter.recv_timeout(woken), Ok(Ok(true)));
        let waiter = waiting(&tx, 4, 3);
        assert_waits(&waiter);
        assert_eq!(rx.recv_timeout(woken), Ok(2));
        assert_eq!(waiter.recv_timeout(woken), Ok(Ok(true)));
        let waiter = waiting(&tx, 4, 4);
        assert_waits(&waiter);
        drop(rx);
        assert_eq!(
            waiter.recv_timeout(woken),
            Ok(Err(ReserveError::Disconnected))
        );
    }

    #[test]
    fn a_control_value_has_its_own_allowance_and_leaves_ordinary_room_alone() {
        let charge = message_overhead::<u32>() + 4;
        let (tx, rx) = channel::<u32>(2 * charge).unwrap();
        let woken = Duration::from_secs(5);
        tx.try_send(1, 4).unwrap();
        tx.try_send(2, 4).unwrap();
        // A full budget still admits the control value, in order, and charges it.
        let first = control(&tx, 4, 9);
        assert_eq!(first.recv_timeout(woken), Ok(Ok(true)));
        assert_eq!(tx.stats().reserved_bytes, 3 * charge);
        assert!(matches!(tx.reserve(4), Err(ReserveError::Full)));
        // With it held, ordinary admission has exactly the room it has without it: one
        // released value's charge, all of it.
        assert_eq!(rx.recv_timeout(woken), Ok(1));
        tx.try_send(3, 4).unwrap();
        assert!(matches!(tx.reserve(0), Err(ReserveError::Full)));
        assert_eq!(rx.recv_timeout(woken), Ok(2));
        let (value, lease) = rx.recv_timeout_owned(woken).unwrap().into_parts();
        assert_eq!(value, 9);
        // One control value at a time: a second waits for the first's charge.
        let second = control(&tx, 4, 10);
        assert_waits(&second);
        drop(lease);
        assert_eq!(second.recv_timeout(woken), Ok(Ok(true)));
        assert_eq!(rx.recv_timeout(woken), Ok(3));
        assert_eq!(rx.recv_timeout(woken), Ok(10));
        assert_eq!(tx.stats().reserved_bytes, 0);
        assert_eq!(
            control(&tx, 2 * charge, 0).recv_timeout(woken),
            Ok(Err(ReserveError::Full))
        );
        drop(rx);
        assert_eq!(
            control(&tx, 0, 0).recv_timeout(woken),
            Ok(Err(ReserveError::Disconnected))
        );
    }

    #[test]
    fn reservations_refuse_a_total_that_would_overflow() {
        let (tx, rx) = channel::<u32>(usize::MAX).unwrap();
        let woken = Duration::from_secs(5);
        let most = usize::MAX - message_overhead::<u32>();
        let held = tx.reserve(most).unwrap();
        // The control allowance would take the total past usize::MAX.
        assert_eq!(
            control(&tx, most, 0).recv_timeout(woken),
            Ok(Err(ReserveError::Full))
        );
        assert_eq!(tx.stats().reserved_bytes, usize::MAX);
        drop(held);
        assert_eq!(control(&tx, most, 7).recv_timeout(woken), Ok(Ok(true)));
        assert!(matches!(tx.reserve(most), Err(ReserveError::Full)));
        assert_eq!(rx.recv_timeout(woken), Ok(7));
        assert_eq!(tx.stats().reserved_bytes, 0);
    }

    /// A worker that reserves `bytes` with [`Sender::reserve_over_budget`] and publishes
    /// `value`. Read with a timeout, so a missed wakeup fails instead of hanging.
    fn control(
        tx: &Sender<u32>,
        bytes: usize,
        value: u32,
    ) -> std::sync::mpsc::Receiver<Result<bool, ReserveError>> {
        let (done, result) = std::sync::mpsc::channel();
        let tx = tx.clone();
        std::thread::spawn(move || {
            let _ = done.send(
                tx.reserve_over_budget(bytes)
                    .map(|permit| permit.publish(value).is_ok()),
            );
        });
        result
    }

    /// A thread that reserves `bytes` with [`Sender::reserve_waiting`] and publishes
    /// `value`. Read with a timeout, so a missed wakeup fails instead of hanging.
    fn waiting(
        tx: &Sender<u32>,
        bytes: usize,
        value: u32,
    ) -> std::sync::mpsc::Receiver<Result<bool, ReserveError>> {
        let (done, result) = std::sync::mpsc::channel();
        let tx = tx.clone();
        std::thread::spawn(move || {
            let _ = done.send(
                tx.reserve_waiting(bytes)
                    .map(|permit| permit.publish(value).is_ok()),
            );
        });
        result
    }

    fn assert_waits(waiter: &std::sync::mpsc::Receiver<Result<bool, ReserveError>>) {
        std::thread::sleep(Duration::from_millis(50));
        assert!(waiter.try_recv().is_err(), "it did not wait");
    }

    #[test]
    fn blocking_delivery_timeout_and_last_sender_close() {
        assert!(channel::<()>(0).is_err());
        let (tx, rx) = channel::<u32>(1024).unwrap();
        assert_eq!(
            rx.recv_timeout(Duration::ZERO),
            Err(RecvTimeoutError::Timeout)
        );
        std::thread::scope(|scope| {
            scope.spawn(move || {
                tx.try_send(42, 0).unwrap();
            });
            assert_eq!(rx.recv_timeout(Duration::from_secs(5)), Ok(42));
            assert_eq!(rx.recv(), Err(RecvError));
            assert_eq!(
                rx.recv_timeout(Duration::ZERO),
                Err(RecvTimeoutError::Disconnected)
            );
        });
    }

    fn wait_for_ready_waiters<T>(tx: &Sender<T>, expected: usize, timeout: Duration) {
        let deadline = std::time::Instant::now() + timeout;
        while tx.0.lock().ready_waiters != expected {
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for ready_waiters == {expected} (got {})",
                tx.0.lock().ready_waiters
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn waiter_counts_wake_blocked_producers_and_consumers() {
        let charge = message_overhead::<u32>() + 4;
        let (tx, rx) = channel::<u32>(charge).unwrap();
        let timeout = Duration::from_secs(5);

        // Move `tx` into `scope` so any panic or timed-out `recv_timeout` drops `tx` before
        // `scope` joins, guaranteeing blocked consumers never hang the test on failure.
        let tx = std::thread::scope(|scope| {
            let tx = tx;
            let rx_ref = &rx;

            // Consumer blocked in `recv`, `recv_owned`, and `recv_timeout` must wake when published.
            let (done1_tx, done1_rx) = std::sync::mpsc::channel();
            let h1 = scope.spawn(move || {
                let _ = done1_tx.send(rx_ref.recv());
            });
            wait_for_ready_waiters(&tx, 1, timeout);
            let p1 = tx.reserve(4).unwrap();
            p1.publish(11).unwrap();
            assert_eq!(done1_rx.recv_timeout(timeout), Ok(Ok(11)));
            h1.join().unwrap();

            let (done2_tx, done2_rx) = std::sync::mpsc::channel();
            let h2 = scope.spawn(move || {
                let _ = done2_tx.send(rx_ref.recv_owned());
            });
            wait_for_ready_waiters(&tx, 1, timeout);
            let p2 = tx.reserve(4).unwrap();
            p2.publish(22).unwrap();
            let owned2 = done2_rx
                .recv_timeout(timeout)
                .expect("recv_owned wakeup timed out")
                .expect("recv_owned failed");
            let (v2, lease2) = owned2.into_parts();
            h2.join().unwrap();
            assert_eq!(v2, 22);

            // Producer blocked in `reserve_waiting` wakes when `ReceiveLease` drops, and
            // producer blocked in `reserve_over_budget` wakes when `Permit::drop` cancels.
            let prod = waiting(&tx, 4, 33);
            assert_waits(&prod);
            drop(lease2);
            assert_eq!(prod.recv_timeout(timeout), Ok(Ok(true)));
            assert_eq!(rx.recv_timeout(timeout), Ok(33));

            let (done3_tx, done3_rx) = std::sync::mpsc::channel();
            let h3 = scope.spawn(move || {
                let _ = done3_tx.send(rx_ref.recv_timeout(timeout));
            });
            wait_for_ready_waiters(&tx, 1, timeout);
            tx.try_send(34, 4).unwrap();
            assert_eq!(done3_rx.recv_timeout(timeout), Ok(Ok(34)));
            h3.join().unwrap();

            let ctrl1 = tx.reserve_over_budget(4).unwrap();
            let ctrl2 = control(&tx, 4, 44);
            assert_waits(&ctrl2);
            drop(ctrl1);
            assert_eq!(ctrl2.recv_timeout(timeout), Ok(Ok(true)));
            assert_eq!(rx.recv_timeout(timeout), Ok(44));
            tx
        });
        assert_eq!(tx.0.lock().room_waiters, 0);
        assert_eq!(tx.0.lock().ready_waiters, 0);
    }
}
