//! Connection-scoped authoritative custom book synchronization.
use super::{BookFlags, CustomBooks};
use std::collections::BTreeSet;
use std::time::{Duration, Instant};

pub use horizon_protocol::custom_book::CHANNEL;
pub const MAX_BYTES: usize = 512 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    Busy,
    Hello,
    Ready,
    Offer(Vec<String>),
    Snapshot {
        epoch: u64,
        revision: u64,
        entries: Vec<(String, BookFlags)>,
    },
    Request {
        epoch: u64,
        number: u64,
        key: String,
        flags: BookFlags,
    },
    Update {
        epoch: u64,
        revision: u64,
        key: String,
        flags: BookFlags,
    },
    Resnapshot,
    Placement(super::placement_sync::Frame),
}

/// Connection payload budget, checked before decoding or allocating replies.
pub struct RequestBudget {
    window: Instant,
    count: u32,
}
impl Default for RequestBudget {
    fn default() -> Self {
        Self {
            window: Instant::now(),
            count: 0,
        }
    }
}
impl RequestBudget {
    pub fn accept(&mut self, now: Instant) -> bool {
        if now.saturating_duration_since(self.window) >= Duration::from_secs(1) {
            self.window = now;
            self.count = 0;
        }
        if self.count >= 20 {
            return false;
        }
        self.count += 1;
        true
    }
}

/// A new value is required for every connection, including reconnects.
pub struct Authority {
    pub epoch: u64,
    pub revision: u64,
    keys: BTreeSet<String>,
    last_request: u64,
    window: Instant,
    requests: u32,
}

impl Authority {
    /// Book membership was negotiated once for this connection, not inferred from a request.
    pub fn contains(&self, key: &str) -> bool {
        self.keys.contains(key)
    }
    pub fn negotiate(epoch: u64, offered: Vec<String>, registered: &BTreeSet<String>) -> Self {
        Self {
            epoch,
            revision: 0,
            keys: offered
                .into_iter()
                .filter(|id| registered.contains(id))
                .collect(),
            last_request: 0,
            window: Instant::now(),
            requests: 0,
        }
    }
    pub fn server_update(&mut self, key: &str, flags: BookFlags) -> Option<Message> {
        if !self.keys.contains(key) {
            return None;
        }
        self.revision = self.revision.checked_add(1)?;
        Some(Message::Update {
            epoch: self.epoch,
            revision: self.revision,
            key: key.into(),
            flags,
        })
    }
    pub fn snapshot(&self, books: &CustomBooks) -> Message {
        Message::Snapshot {
            epoch: self.epoch,
            revision: self.revision,
            entries: self
                .keys
                .iter()
                .map(|id| (id.clone(), books.get(id)))
                .collect(),
        }
    }
    /// Rejections return an authoritative snapshot, never mutate saved state.
    pub fn request(
        &mut self,
        books: &mut CustomBooks,
        epoch: u64,
        number: u64,
        key: &str,
        flags: BookFlags,
        now: Instant,
    ) -> (Message, bool) {
        if now.saturating_duration_since(self.window) >= Duration::from_secs(1) {
            self.window = now;
            self.requests = 0;
        }
        self.requests = self.requests.saturating_add(1);
        if epoch != self.epoch
            || number <= self.last_request
            || self.requests > 20
            || !self.keys.contains(key)
        {
            return (self.snapshot(books), false);
        }
        self.last_request = number;
        self.apply(books, key, flags)
    }
    pub fn apply(
        &mut self,
        books: &mut CustomBooks,
        key: &str,
        flags: BookFlags,
    ) -> (Message, bool) {
        if !self.keys.contains(key) || self.revision == u64::MAX {
            return (self.snapshot(books), false);
        }
        match books.set(key, flags) {
            Ok(true) => {
                self.revision += 1;
                (
                    Message::Update {
                        epoch: self.epoch,
                        revision: self.revision,
                        key: key.into(),
                        flags,
                    },
                    true,
                )
            }
            _ => (self.snapshot(books), false),
        }
    }
}

/// Pending intent is deliberately absent: only snapshots and ordered updates
/// enter this view, and callers cannot save it as the server's player record.
#[derive(Default)]
pub struct ClientView {
    pub epoch: Option<u64>,
    pub revision: u64,
    pub ready: bool,
    pub books: CustomBooks,
    keys: BTreeSet<String>,
}
impl ClientView {
    pub fn contains(&self, key: &str) -> bool {
        self.keys.contains(key)
    }
    pub fn receive(&mut self, message: Message, registered: &BTreeSet<String>) -> bool {
        match message {
            Message::Snapshot {
                epoch,
                revision,
                entries,
            } => {
                if self.epoch.is_some_and(|current| current != epoch)
                    || (self.epoch == Some(epoch) && revision < self.revision)
                    || entries.len() > 1024
                {
                    return false;
                }
                let mut books = CustomBooks::default();
                let mut keys = BTreeSet::new();
                for (key, flags) in entries {
                    if !registered.contains(&key)
                        || !keys.insert(key.clone())
                        || books.set(&key, flags).is_err()
                    {
                        self.ready = false;
                        return false;
                    }
                }
                self.epoch = Some(epoch);
                self.revision = revision;
                self.books = books;
                self.keys = keys;
                self.ready = true;
                true
            }
            Message::Update {
                epoch,
                revision,
                key,
                flags,
            } => {
                if self.epoch != Some(epoch) {
                    return false;
                }
                if !self.ready
                    || self.revision.checked_add(1) != Some(revision)
                    || !self.keys.contains(&key)
                {
                    self.ready = false;
                    return false;
                }
                if self.books.set(&key, flags).is_err() {
                    self.ready = false;
                    return false;
                }
                self.revision = revision;
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
#[path = "settings_sync_tests.rs"]
mod tests;
