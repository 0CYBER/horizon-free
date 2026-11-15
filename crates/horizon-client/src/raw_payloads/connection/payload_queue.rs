//! One physical connection's ordered receiving turns, including a main root in flight.
use super::Connection;
use crate::raw_payloads::Incoming;
use std::sync::atomic::Ordering;
use std::{
    collections::VecDeque,
    sync::{Arc, Weak},
    time::Duration,
};

#[derive(Default)]
pub(super) struct PayloadQueue {
    pending: VecDeque<Incoming>,
    active: bool,
    busy: Option<mod_api::PayloadBusy>,
}

/// Finishing or cancelling a root releases the next receiving turn, never invokes it.
pub(in crate::raw_payloads) struct PayloadTurn {
    connection: Weak<Connection>,
    deferred: bool,
}
impl PayloadTurn {
    pub(in crate::raw_payloads) fn busy(&self, holder: &mod_api::PayloadBusy) {
        if let Some(connection) = self.connection.upgrade() {
            connection.payloads.lock().unwrap().busy = Some(holder.clone());
        }
    }
}
impl Drop for PayloadTurn {
    fn drop(&mut self) {
        if let Some(connection) = self.connection.upgrade() {
            let mut queue = connection.payloads.lock().unwrap();
            queue.active = false;
            if !self.deferred {
                queue.busy = None;
            }
            drop(queue);
            connection.payload_ready.notify_all();
        }
    }
}

impl Connection {
    /// An idle modded connection's maintenance turn only reads this flag.
    pub fn payloads_pending(&self) -> bool {
        self.payload_pending.load(Ordering::Acquire)
    }

    pub(in crate::raw_payloads) fn queue_payload(
        self: &Arc<Self>,
        mut incoming: Incoming,
    ) -> Option<Incoming> {
        let mut queue = self.payloads.lock().unwrap();
        if queue.active || !queue.pending.is_empty() {
            queue.pending.push_back(incoming);
            self.payload_pending.store(true, Ordering::Release);
            return None;
        }
        // The normal, uncontended path needs no queue allocation.
        queue.active = true;
        incoming.turn = Some(PayloadTurn {
            connection: Arc::downgrade(self),
            deferred: false,
        });
        Some(incoming)
    }

    pub(in crate::raw_payloads) fn next_payload(self: &Arc<Self>) -> Option<Incoming> {
        let mut queue = self.payloads.lock().unwrap();
        if queue.active {
            return None;
        }
        let mut incoming = queue.pending.pop_front()?;
        self.payload_pending
            .store(!queue.pending.is_empty(), Ordering::Release);
        queue.active = true;
        incoming.turn = Some(PayloadTurn {
            connection: Arc::downgrade(self),
            deferred: false,
        });
        Some(incoming)
    }

    pub(in crate::raw_payloads) fn defer_payload(&self, mut incoming: Incoming) {
        let mut turn = incoming.turn.take();
        if let Some(turn) = turn.as_mut() {
            turn.deferred = true;
        }
        let mut queue = self.payloads.lock().unwrap();
        queue.pending.push_front(incoming);
        self.payload_pending.store(true, Ordering::Release);
        drop(queue);
        drop(turn);
    }

    pub(in crate::raw_payloads) fn wait_payloads(&self) -> bool {
        let queue = self.payloads.lock().unwrap();
        if queue.pending.is_empty() && !queue.active {
            return false;
        }
        // Busy guest calls do not notify a connection on exit. This bounded
        // wake belongs to its existing receiver, not to a detached retry task.
        drop(
            self.payload_ready
                .wait_timeout(queue, Duration::from_millis(1))
                .unwrap(),
        );
        true
    }

    pub(in crate::raw_payloads) fn busy_holder(&self) -> Option<mod_api::PayloadBusy> {
        self.payloads.lock().unwrap().busy.clone()
    }

    pub(super) fn clear_payloads(&self) {
        let mut queue = self.payloads.lock().unwrap();
        let pending = std::mem::take(&mut queue.pending);
        self.payload_pending.store(false, Ordering::Release);
        queue.busy = None;
        drop(queue);
        self.payload_ready.notify_all();
        // Opaque guest work must never be destroyed under the queue mutex.
        drop(pending);
    }
}
