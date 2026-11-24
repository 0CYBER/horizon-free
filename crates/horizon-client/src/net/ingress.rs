//! Typed bounded ingress. Charges survive moves into actor backlogs and decoders.
//! Each owner supplies its value's allocation/staging policy; this layer never
//! guesses from encoded length or invokes user callbacks under a queue lock.
pub use byte_channel::{OwnedReceipt, ReceiveLease, Stats};
use horizon_mod_runtime::byte_channel;
use std::{cell::Cell, fmt, marker::PhantomData, sync::mpsc::TryRecvError};

pub struct Producer<T> {
    sender: byte_channel::Sender<T>,
    charge: fn(&T) -> usize,
    capacity: usize,
}
impl<T> Clone for Producer<T> {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
            charge: self.charge,
            capacity: self.capacity,
        }
    }
}
/// One actor owns receiving turns. Like std's Receiver, this is Send but not Sync.
pub struct Consumer<T> {
    receiver: byte_channel::Receiver<T>,
    _single: PhantomData<Cell<()>>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    Pressure,
    Closed,
}
pub struct Refused<T> {
    pub reason: Refusal,
    pub value: T,
}
impl<T> fmt::Debug for Refused<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.reason, f)
    }
}
impl<T> fmt::Display for Refused<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self.reason {
            Refusal::Pressure => "incoming queue resource pressure",
            Refusal::Closed => "incoming receiver closed",
        })
    }
}
impl<T> std::error::Error for Refused<T> {}
pub struct Prepared<T> {
    value: T,
    permit: byte_channel::Permit<T>,
}
impl<T> Prepared<T> {
    /// Failure returns the owned value, so its resources can be dropped after
    /// releasing any publication gate held by the caller.
    pub fn publish(self) -> Result<(), Refused<T>> {
        self.permit.publish(self.value).map_err(|value| Refused {
            reason: Refusal::Closed,
            value,
        })
    }
    /// Take the prepared value back without publishing it. The reservation is
    /// released for the caller to re-charge the value wherever it is staged.
    pub fn defuse(self) -> T {
        let Self { value, permit } = self;
        drop(permit);
        value
    }
}
pub fn channel<T>(
    capacity: usize,
    charge: fn(&T) -> usize,
) -> Result<(Producer<T>, Consumer<T>), byte_channel::InvalidCapacity> {
    let (sender, receiver) = byte_channel::channel(capacity)?;
    Ok((
        Producer {
            sender,
            charge,
            capacity,
        },
        Consumer {
            receiver,
            _single: PhantomData,
        },
    ))
}
impl<T> Producer<T> {
    /// Reserve before reporting acceptance. Allocation traversal occurs before
    /// taking the channel lock, and all failure values leave that lock intact.
    pub fn prepare(&self, value: T) -> Result<Prepared<T>, Refused<T>> {
        let charge = (self.charge)(&value);
        Self::prepared(self.sender.reserve(charge), value)
    }
    fn prepared(
        permit: Result<byte_channel::Permit<T>, byte_channel::ReserveError>,
        value: T,
    ) -> Result<Prepared<T>, Refused<T>> {
        match permit {
            Ok(permit) => Ok(Prepared { value, permit }),
            Err(error) => Err(Refused {
                reason: match error {
                    byte_channel::ReserveError::Full => Refusal::Pressure,
                    byte_channel::ReserveError::Disconnected => Refusal::Closed,
                },
                value,
            }),
        }
    }
    pub fn send(&self, value: T) -> Result<(), Refused<T>> {
        self.prepare(value)?.publish()
    }
    /// Waits for room instead of refusing for pressure, so a network reader stops
    /// reading and the transport's own flow control holds the peer back. Here
    /// [`Refusal::Pressure`] means the value's charge exceeds the whole budget.
    /// Never call it from the consumer's thread, or while holding a lock it needs.
    pub fn send_waiting(&self, value: T) -> Result<(), Refused<T>> {
        let charge = (self.charge)(&value);
        Self::prepared(self.sender.reserve_waiting(charge), value)?.publish()
    }
    /// Publishes a small control value in order even when the queue is full, still
    /// charging it. [`Refusal::Pressure`] means its charge exceeds the whole budget.
    pub fn send_over_budget(&self, value: T) -> Result<(), Refused<T>> {
        let charge = (self.charge)(&value);
        Self::prepared(self.sender.reserve_over_budget(charge), value)?.publish()
    }
    pub fn stats(&self) -> Stats {
        self.sender.stats()
    }
    /// The channel's byte budget. Retained staging that has not been published
    /// yet (a play bundle's packets) is bounded by what this budget still has
    /// free, so the budget stays meaningful across the staging step.
    pub fn capacity(&self) -> usize {
        self.capacity
    }
}
impl<T> Consumer<T> {
    pub fn stats(&self) -> Stats {
        self.receiver.stats()
    }
    pub fn try_recv_owned(&self) -> Result<OwnedReceipt<T>, TryRecvError> {
        self.receiver.try_recv_owned()
    }
    #[cfg(test)]
    pub fn recv_timeout_owned(
        &self,
        timeout: std::time::Duration,
    ) -> Result<OwnedReceipt<T>, std::sync::mpsc::RecvTimeoutError> {
        self.receiver.recv_timeout_owned(timeout)
    }
    /// Only values already published at the beginning of this turn. New replies
    /// append behind these values and cannot feed back into the same actor turn.
    pub fn take_turn(&self) -> Vec<OwnedReceipt<T>> {
        let count = self.stats().queued_items;
        (0..count)
            .map_while(|_| self.try_recv_owned().ok())
            .collect()
    }
}
