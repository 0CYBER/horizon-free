//! Typed bounded ingress. Charges survive moves into actor backlogs and decoders.
//! Each owner supplies its value's allocation/staging policy; this layer never
//! guesses from encoded length or invokes user callbacks under a queue lock.
pub use byte_channel::{OwnedReceipt, ReceiveLease, Stats};
use horizon_mod_runtime::byte_channel;
use std::{cell::Cell, fmt, marker::PhantomData, sync::mpsc::TryRecvError};

pub struct Producer<T> {
    sender: byte_channel::Sender<T>,
    charge: fn(&T) -> usize,
}
impl<T> Clone for Producer<T> {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
            charge: self.charge,
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
}
pub fn channel<T>(
    capacity: usize,
    charge: fn(&T) -> usize,
) -> Result<(Producer<T>, Consumer<T>), byte_channel::InvalidCapacity> {
    let (sender, receiver) = byte_channel::channel(capacity)?;
    Ok((
        Producer { sender, charge },
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
        match self.sender.reserve((self.charge)(&value)) {
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
    pub fn stats(&self) -> Stats {
        self.sender.stats()
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
