//! Connection epochs and main-executor barriers, independent of gameplay ownership.
mod payload_queue;
pub(super) use payload_queue::PayloadTurn;
#[cfg(test)]
mod tests;
use super::{Local, Peer, Side};
use horizon_mod_runtime::byte_channel;
use horizon_protocol::payload_negotiation::{Entry, Phase};
use mod_api::{PayloadSendError, PlayerRef};
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

/// A frozen phase of one connection. Work carries this identity, not just a reused player id.
pub struct Epoch {
    pub peer: Arc<Peer>,
    pub phase: Phase,
    active: AtomicBool,
    publication: Mutex<()>,
    pending: Mutex<usize>,
    drained: Condvar,
}
impl Epoch {
    fn new(peer: Arc<Peer>, phase: Phase) -> Arc<Self> {
        Arc::new(Self {
            peer,
            phase,
            active: AtomicBool::new(true),
            publication: Mutex::new(()),
            pending: Mutex::new(0),
            drained: Condvar::new(),
        })
    }
    pub fn active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }
    /// Atomically publish against invalidation. The closure may only perform nonblocking
    /// queue publication, never user callbacks, waits or connection-state operations.
    pub fn with_active<T>(&self, publish: impl FnOnce() -> T) -> Option<T> {
        let publication = self.publication.lock().unwrap();
        if !self.active() {
            // A cancelled closure may own guest resources. Release it only after
            // the publication lock, never while invoking their destructors under it.
            drop(publication);
            return None;
        }
        let result = publish();
        drop(publication);
        Some(result)
    }
    fn invalidate(&self) {
        let _publication = self.publication.lock().unwrap();
        let pending = self.pending.lock().unwrap();
        self.active.store(false, Ordering::Release);
        drop(pending);
        self.drained.notify_all();
    }
    /// Keep a configuration barrier pending until the handler and its root commit finish.
    pub fn main_task(self: &Arc<Self>) -> MainTask {
        *self.pending.lock().unwrap() += 1;
        MainTask(self.clone())
    }
    /// Wait for main-thread work without advancing this epoch; false on invalidation.
    pub fn wait_main(&self) -> bool {
        let mut pending = self.pending.lock().unwrap();
        while *pending != 0 && self.active() {
            pending = self.drained.wait(pending).unwrap();
        }
        self.active()
    }
}
/// Dropping queued work on cancellation releases the same barrier as executing it.
pub struct MainTask(Arc<Epoch>);
impl Drop for MainTask {
    fn drop(&mut self) {
        let mut pending = self.0.pending.lock().unwrap();
        *pending -= 1;
        drop(pending);
        self.0.drained.notify_all();
    }
}

struct Current {
    closed: bool,
    epoch: Option<Arc<Epoch>>,
    player: Option<PlayerRef>,
}
/// Physical-connection retained ingress capacity. Moving this lease between queues
/// preserves its reservation; dropping it refunds capacity, independent of epochs.
pub struct IngressLease {
    _permit: byte_channel::Permit<()>,
    pub(super) owned_bytes: usize,
    received: Instant,
}
pub(super) const PAYLOAD_DEADLINE: Duration =
    Duration::from_millis(2 * horizon_mod_runtime::MAX_GUEST_CALL_BUDGET_MILLIS);
impl IngressLease {
    /// A retained payload may wait longer than the VM's largest (30 s) load call,
    /// but never extend its deadline by moving between receiving queues.
    pub(super) fn expired(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.received) >= PAYLOAD_DEADLINE
    }
}
// Four slots cover Vec's initial growth and later geometric staging. Provider
// queue storage is separately charged by the typed ingress channel. This charge
// covers both main routes and network-staged work, including boxed identities.
const INGRESS_RECORD_OVERHEAD: usize = 4
    * (std::mem::size_of::<crate::lan::NetEvent>()
        + std::mem::size_of::<crate::lan::hub::HostEvent>()
        + std::mem::size_of::<crate::net::Event>()
        + std::mem::size_of::<super::Incoming>())
    + std::mem::size_of::<super::Delivery>()
    + 128;
const INGRESS_BUDGET: usize = 16 << 20;

/// Allocate only for a nonempty local installation; unmodded connections need no dispatch state.
pub struct Connection {
    local: Arc<Local>,
    side: Side,
    current: Mutex<Current>,
    ingress: byte_channel::Sender<()>,
    _ingress_receiver: byte_channel::Receiver<()>,
    payloads: Mutex<payload_queue::PayloadQueue>,
    payload_ready: Condvar,
    payload_pending: AtomicBool,
}
impl Connection {
    #[cfg(any(test, feature = "test-payload-probe"))]
    pub(super) fn diagnostics(&self) -> String {
        // Channel stats uses its existing short internal mutex; do not hold
        // connection or epoch locks while taking that snapshot.
        let ingress = self.ingress.stats();
        let window = INGRESS_BUDGET.saturating_sub(ingress.reserved_bytes);
        match self.current.try_lock() {
            Ok(c) => {
                let epoch = c.epoch.as_ref().map(|e| {
                    let pending = e.pending.try_lock().ok().map(|p| *p);
                    format!(
                        "phase={:?} active={} pending_main={pending:?}",
                        e.phase,
                        e.active()
                    )
                });
                format!(
                    "closed={} epoch={epoch:?} ingress_window={window} ingress_capacity={INGRESS_BUDGET} ingress={ingress:?}",
                    c.closed
                )
            }
            Err(_) => format!(
                "connection busy/poisoned ingress_window={window} ingress_capacity={INGRESS_BUDGET} ingress={ingress:?}"
            ),
        }
    }
    pub fn new(local: Arc<Local>, side: Side) -> Arc<Self> {
        let (ingress, receiver) = byte_channel::channel(INGRESS_BUDGET).unwrap();
        Arc::new(Self {
            ingress,
            _ingress_receiver: receiver,
            payloads: Mutex::default(),
            payload_ready: Condvar::new(),
            payload_pending: AtomicBool::new(false),
            local,
            side,
            current: Mutex::new(Current {
                closed: false,
                epoch: None,
                player: None,
            }),
        })
    }
    /// Retain bytes across arbitrary ingress staging queues and phase changes.
    /// The layout-derived allowance covers envelope and geometric actor staging;
    /// this is resource pressure, not a protocol body or record-count limit.
    pub fn reserve_ingress(&self, owned_bytes: usize) -> Result<IngressLease, PayloadSendError> {
        let current = self.current.lock().unwrap();
        if current.closed {
            return Err(PayloadSendError::Disconnected);
        }
        let charge = owned_bytes
            .checked_add(INGRESS_RECORD_OVERHEAD)
            .ok_or(PayloadSendError::Backpressure)?;
        self.ingress
            .reserve(charge)
            .map(|permit| IngressLease {
                _permit: permit,
                owned_bytes,
                received: Instant::now(),
            })
            .map_err(|e| match e {
                byte_channel::ReserveError::Full => PayloadSendError::Backpressure,
                byte_channel::ReserveError::Disconnected => PayloadSendError::Disconnected,
            })
    }
    /// Bind a Game-assigned handle; never manufacture handles on an I/O executor.
    pub fn bind_player(&self, player: PlayerRef) {
        let mut current = self.current.lock().unwrap();
        if !current.closed {
            current.player = Some(player);
        }
    }
    pub fn closed(&self) -> bool {
        self.current.lock().unwrap().closed
    }
    pub fn player(&self) -> Option<PlayerRef> {
        self.current.lock().unwrap().player
    }
    pub fn epoch(&self) -> Option<Arc<Epoch>> {
        self.current.lock().unwrap().epoch.clone()
    }
    /// The I/O executor starts a fresh configuration before accepting application bytes.
    pub fn begin_configuration(&self) {
        if let Some(old) = self.current.lock().unwrap().epoch.take() {
            old.invalidate();
        }
        self.clear_payloads();
    }
    pub fn accept(&self, entries: &[Entry]) -> Result<(), String> {
        let mut peer = self.local.freeze(entries, self.side)?;
        let bytes = peer.owned_snapshot_bytes();
        let physical = self
            .reserve_ingress(bytes)
            .map_err(|error| format!("payload negotiation resource pressure: {error:?}"))?;
        let shared = self
            .local
            .snapshot_budget
            .as_ref()
            .map(|(budget, _)| budget.reserve(bytes))
            .transpose()
            .map_err(|error| format!("payload negotiation snapshot pressure: {error:?}"))?;
        peer.credit = Some((physical, shared));
        let peer = Arc::new(peer);
        let mut current = self.current.lock().unwrap();
        if current.closed {
            return Err("payload connection disconnected".into());
        }
        if let Some(old) = current
            .epoch
            .replace(Epoch::new(peer, Phase::Configuration))
        {
            old.invalidate();
        }
        Ok(())
    }
    /// No configuration acknowledgement overtakes a queued main-thread handler or its replies.
    /// Never hold the connection-state or writer lock while waiting for gameplay.
    pub fn finish_configuration(&self) -> Result<(), String> {
        self.finish_configuration_with(|| Ok(()))
    }
    /// Publish the phase-transition acknowledgement after invalidating old replies,
    /// before exposing the new Play epoch. The closure only queues engine bytes.
    pub fn finish_configuration_with(
        &self,
        publish: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        let old = self.epoch().ok_or("payload negotiation is not complete")?;
        if !old.wait_main() {
            return Err("payload configuration epoch ended".into());
        }
        let mut current = self.current.lock().unwrap();
        if current.closed {
            return Err("payload connection disconnected".into());
        }
        if current
            .epoch
            .as_ref()
            .is_none_or(|epoch| !Arc::ptr_eq(epoch, &old))
        {
            return Err("payload configuration epoch was replaced".into());
        }
        old.invalidate();
        publish()?;
        current.epoch = Some(Epoch::new(old.peer.clone(), Phase::Play));
        Ok(())
    }
    pub fn close(&self) {
        let mut current = self.current.lock().unwrap();
        current.closed = true;
        current.player = None;
        if let Some(old) = current.epoch.take() {
            old.invalidate();
        }
        drop(current);
        self.clear_payloads();
    }
}
impl Drop for Connection {
    fn drop(&mut self) {
        self.close();
    }
}
