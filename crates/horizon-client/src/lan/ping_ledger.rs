//! The pings the server sent a guest that its client has not answered yet. `player_pong` is heard
//! only for an answer to one of them, once: a client that sends pongs nobody asked for, or the
//! same one over and over, would otherwise make the server swap its state in for each.
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Most pings kept per guest; the oldest goes when a new one comes. The server's own ping at the
/// end of each tick (20 a second) shares the ledger with a mod's, and a slow client has a second
/// of them outstanding.
pub const MAX_PENDING: usize = 64;
/// How long a ping may wait for its answer: the keep-alive timeout.
pub const EXPIRY: Duration = Duration::from_secs(30);

#[derive(Debug, Default)]
pub struct PingLedger {
    sent: VecDeque<(i32, Instant)>,
}

impl PingLedger {
    /// A ping numbered `id` went out at `now`.
    pub fn sent(&mut self, id: i32, now: Instant) {
        self.expire(now);
        if self.sent.len() >= MAX_PENDING {
            self.sent.pop_front();
        }
        self.sent.push_back((id, now));
    }

    /// An answer numbered `id` arrived at `now`: whether it answers a ping still waiting. That ping
    /// is used up, so the same answer again is not.
    pub fn answered(&mut self, id: i32, now: Instant) -> bool {
        self.expire(now);
        match self.sent.iter().position(|(sent, _)| *sent == id) {
            Some(at) => {
                self.sent.remove(at);
                true
            }
            None => false,
        }
    }

    fn expire(&mut self, now: Instant) {
        while let Some((_, at)) = self.sent.front() {
            if now.saturating_duration_since(*at) > EXPIRY {
                self.sent.pop_front();
            } else {
                break;
            }
        }
    }

    pub fn len(&self) -> usize {
        self.sent.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sent.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unsolicited_answer_is_not_one() {
        let mut ledger = PingLedger::default();
        assert!(!ledger.answered(5, Instant::now()));
    }

    #[test]
    fn a_ping_is_answered_once() {
        let (mut ledger, now) = (PingLedger::default(), Instant::now());
        ledger.sent(5, now);
        assert!(!ledger.answered(6, now));
        assert!(ledger.answered(5, now));
        assert!(!ledger.answered(5, now));
        assert!(ledger.is_empty());
    }

    #[test]
    fn the_same_id_sent_twice_is_answered_twice() {
        let (mut ledger, now) = (PingLedger::default(), Instant::now());
        ledger.sent(5, now);
        ledger.sent(5, now);
        assert!(ledger.answered(5, now));
        assert!(ledger.answered(5, now));
        assert!(!ledger.answered(5, now));
    }

    #[test]
    fn the_oldest_goes_when_full() {
        let (mut ledger, now) = (PingLedger::default(), Instant::now());
        for id in 0..MAX_PENDING as i32 + 10 {
            ledger.sent(id, now);
        }
        assert_eq!(ledger.len(), MAX_PENDING);
        assert!(!ledger.answered(0, now));
        assert!(!ledger.answered(9, now));
        assert!(ledger.answered(10, now));
        assert!(ledger.answered(MAX_PENDING as i32 + 9, now));
    }

    #[test]
    fn an_old_ping_expires() {
        let (mut ledger, now) = (PingLedger::default(), Instant::now());
        ledger.sent(1, now);
        ledger.sent(2, now + Duration::from_secs(20));
        let later = now + EXPIRY + Duration::from_secs(1);
        assert!(!ledger.answered(1, later));
        assert!(ledger.answered(2, later));
    }
}
