//! Each player's play packets counted by id and direction, while a mod asks for it
//! (PacketEvents' send and receive events, summed): off, a packet costs one relaxed atomic read.
use crate::fast_hash::FxHashMap;
use std::sync::{
    Mutex, PoisonError,
    atomic::{AtomicBool, Ordering},
};

/// Whether packets are counted, for every connection: one switch for the server.
static COUNTING: AtomicBool = AtomicBool::new(false);

/// Count (or stop counting) every player's packets from now on.
pub fn set_counting(on: bool) {
    COUNTING.store(on, Ordering::Relaxed);
}

/// Whether packets are counted.
pub fn counting() -> bool {
    COUNTING.load(Ordering::Relaxed)
}

/// What one packet id came to in one direction since the counts were last taken.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Count {
    /// How many packets there were.
    pub packets: u64,
    /// The bytes of id and payload, before compression and encryption.
    pub bytes: u64,
}

/// One connection's counts, keyed by (sent by the server, packet id).
#[derive(Debug, Default)]
pub struct PacketCounts {
    counts: Mutex<FxHashMap<(bool, i32), Count>>,
}
impl PacketCounts {
    /// A packet of `bytes` (id and payload) with id `id`, sent (`outbound`) or received.
    pub fn add(&self, outbound: bool, id: i32, bytes: usize) {
        if !counting() {
            return;
        }
        let mut counts = self.counts.lock().unwrap_or_else(PoisonError::into_inner);
        let count = counts.entry((outbound, id)).or_default();
        count.packets += 1;
        count.bytes += bytes as u64;
    }
    /// A sent body: its leading varint is the id.
    pub fn add_body(&self, body: &[u8]) {
        if !counting() {
            return;
        }
        if let Ok(id) = horizon_protocol::buf::Reader::new(body).varint() {
            self.add(true, id, body.len());
        }
    }
    /// The counts since the last take, by direction then id; clears them.
    pub fn take(&self) -> Vec<(bool, i32, Count)> {
        let counts =
            std::mem::take(&mut *self.counts.lock().unwrap_or_else(PoisonError::into_inner));
        let mut out: Vec<_> = counts.into_iter().map(|((o, id), c)| (o, id, c)).collect();
        out.sort_unstable_by_key(|&(outbound, id, _)| (!outbound, id));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packets_are_counted_only_while_counting_is_on() {
        let counts = PacketCounts::default();
        set_counting(false);
        counts.add_body(&[0x05, 1, 2, 3]);
        assert!(counts.take().is_empty());
        set_counting(true);
        counts.add_body(&[0x05, 1, 2, 3]);
        counts.add_body(&[0x05, 9]);
        counts.add(false, 0x1a, 10);
        set_counting(false);
        assert_eq!(
            counts.take(),
            [
                (
                    true,
                    5,
                    Count {
                        packets: 2,
                        bytes: 6
                    }
                ),
                (
                    false,
                    0x1a,
                    Count {
                        packets: 1,
                        bytes: 10
                    }
                ),
            ]
        );
        assert!(counts.take().is_empty());
    }
}
