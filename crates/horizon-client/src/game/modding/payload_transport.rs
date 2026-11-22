//! Sole-owner main execution and root-commit raw publication.
use super::Game;
use horizon_client::raw_payloads::{Body, Endpoint, Incoming, Prepared, Side, connection::Epoch};
use horizon_protocol::payload_negotiation::Phase;
use mod_api::{
    HandlerThread, IPayloadContext, PayloadAccepted, PayloadBytes, PayloadContextError,
    PayloadDirection, PayloadId, PayloadProtocol, PayloadSendError, PayloadWork, PayloadWorkStatus,
    PlayerRef, Text,
};
use std::sync::Arc;

/// Fits the journal-growth allowance already charged by each prepared operation.
pub(super) struct Deferred {
    epoch: Arc<Epoch>,
    prepared: Prepared,
    #[cfg(test)]
    probe_server_reply: bool,
}
const _: () = assert!(std::mem::size_of::<Deferred>() <= 256);
impl Deferred {
    fn new(_side: Side, _id: &PayloadId, epoch: Arc<Epoch>, prepared: Prepared) -> Self {
        Self {
            epoch,
            prepared,
            #[cfg(test)]
            probe_server_reply: _side == Side::Server && _id.0 == "raw_probe:receipt",
        }
    }
    pub(super) fn publish(self) {
        #[cfg(test)]
        wasm_tests::publication(self.probe_server_reply);
        let _ = self.epoch.with_active(|| self.prepared.publish());
    }
}

/// Interior-mutable staging for calls that only hold `&self` outside a transaction, drained by the game.
/// `Default` and the `push`/`take` pair are the whole contract, so it can be tested alone.
pub(super) struct SharedQueue<T>(std::cell::RefCell<Vec<T>>);
impl<T> Default for SharedQueue<T> {
    fn default() -> Self {
        Self(std::cell::RefCell::new(Vec::new()))
    }
}
impl<T> SharedQueue<T> {
    pub(super) fn push(&self, value: T) {
        self.0.borrow_mut().push(value);
    }
    pub(super) fn take(&self) -> Vec<T> {
        std::mem::take(&mut *self.0.borrow_mut())
    }
}
impl Game<'_> {
    pub(super) fn player_send_payload(
        &mut self,
        player: PlayerRef,
        id: &PayloadId,
        bytes: PayloadBytes,
    ) -> Result<PayloadAccepted, PayloadSendError> {
        let sender = self
            .online_player_record(player)
            .ok_or(PayloadSendError::StaleEpoch)?
            .sender
            .clone();
        self.queue_current_payload(Endpoint::server(sender), id, bytes)
    }
    pub(super) fn send_payload_to_server(
        &mut self,
        id: &PayloadId,
        bytes: PayloadBytes,
    ) -> Result<PayloadAccepted, PayloadSendError> {
        let endpoint = self
            .remote
            .as_ref()
            .ok_or(PayloadSendError::Disconnected)?
            .session
            .payload_endpoint();
        self.queue_current_payload(endpoint, id, bytes)
    }
    /// The shared-host half: validates and stages now, publishes on a later client
    /// tick from a `&mut` host. Screen hooks receive `&dyn ClientHost` and cannot call
    /// `send_payload_to_server`; this keeps their signatures unchanged.
    pub(super) fn queue_payload_to_server(
        &self,
        id: &PayloadId,
        bytes: PayloadBytes,
    ) -> Result<PayloadAccepted, PayloadSendError> {
        let endpoint = self
            .remote
            .as_ref()
            .ok_or(PayloadSendError::Disconnected)?
            .session
            .payload_endpoint();
        let epoch = endpoint
            .transport()
            .epoch()
            .ok_or(PayloadSendError::NotNegotiated)?;
        let prepared = endpoint.prepare(&epoch, id, bytes)?;
        let deferred = Deferred::new(endpoint.side(), id, epoch, prepared);
        // Inside a transaction the send goes with it: a handler that fails and rolls back
        // sends nothing. With none open it waits for the next tick's flush.
        if let Some(super::transactions::After::Payload(deferred)) = self
            .stage_after_commit(super::transactions::After::Payload(deferred))
        {
            self.native_net.shared_payloads.push(deferred);
        }
        Ok(PayloadAccepted::Queued)
    }
    /// Move everything staged by `queue_payload_to_server` into the ordinary
    /// root-commit queue, in submission order. Called once per client tick.
    pub(super) fn flush_shared_payloads(&mut self) {
        for deferred in self.native_net.shared_payloads.take() {
            self.after_commit(super::transactions::After::Payload(deferred));
        }
    }
    pub(super) fn queue_current_payload(
        &mut self,
        endpoint: Endpoint,
        id: &PayloadId,
        bytes: PayloadBytes,
    ) -> Result<PayloadAccepted, PayloadSendError> {
        let epoch = endpoint
            .transport()
            .epoch()
            .ok_or(PayloadSendError::NotNegotiated)?;
        self.queue_payload(endpoint, epoch, id, bytes)
    }
    fn queue_payload(
        &mut self,
        endpoint: Endpoint,
        epoch: Arc<Epoch>,
        id: &PayloadId,
        bytes: PayloadBytes,
    ) -> Result<PayloadAccepted, PayloadSendError> {
        let prepared = endpoint.prepare(&epoch, id, bytes)?;
        self.after_commit(super::transactions::After::Payload(Deferred::new(
            endpoint.side(),
            id,
            epoch,
            prepared,
        )));
        #[cfg(test)]
        wasm_tests::staged_reply(endpoint.side() == Side::Server && id.0 == "raw_probe:receipt");
        Ok(PayloadAccepted::Queued)
    }
    pub(super) fn receive_client_payload(
        &mut self,
        incoming: &mut Incoming,
    ) -> horizon_client::Result<bool> {
        let Some(remote) = self.remote.as_ref() else {
            return Ok(true);
        };
        let endpoint = remote.session.payload_endpoint();
        Ok(self.run_payload(endpoint, incoming))
    }
    /// Endpoint identity was captured on ingress, not resolved from a later guest incarnation.
    pub(super) fn receive_server_payload(
        &mut self,
        endpoint: Endpoint,
        incoming: &mut Incoming,
    ) -> bool {
        self.run_payload(endpoint, incoming)
    }
    /// False only while the original receiving root remains deferred at the head.
    fn run_payload(&mut self, endpoint: Endpoint, incoming: &mut Incoming) -> bool {
        if !incoming.epoch.active() {
            return true;
        }
        if let Some(holder) = incoming.expired_busy(std::time::Instant::now()) {
            log::warn!("{}", incoming.busy_failure(holder));
            if !incoming.raw {
                endpoint.disconnect(Text::translate(
                    "disconnect.genericReason",
                    vec![Text::literal(format!(
                        "a mod's payload handler failed ({})",
                        horizon_client::raw_payloads::mod_of(&incoming.id)
                    ))],
                ));
            }
            return true;
        }
        let body = std::mem::replace(&mut incoming.body, Body::Payload(PayloadBytes(Vec::new())));
        #[cfg(test)]
        let wasm_health_before = wasm_tests::health(self, &endpoint, &incoming.id);
        let seal = self.transaction_open();
        let mut context = MainContext {
            game: self,
            epoch: &incoming.epoch,
            endpoint: endpoint.clone(),
            receiver: &incoming.receiver,
            disconnect: None,
            failure: None,
        };
        let result = match body {
            Body::Payload(bytes) if incoming.receiver.thread == HandlerThread::Main => {
                incoming.receiver.handle(bytes, &mut context)
            }
            Body::Payload(_) => Err(mod_api::PayloadHandleError::Failed(
                "payload was not dispatched on its declared network executor".into(),
            )),
            Body::Work(work) => incoming
                .receiver
                .run_work(work.as_ref(), &mut context)
                .map_err(mod_api::PayloadHandleError::Failed),
        };
        let result = match result {
            Err(mod_api::PayloadHandleError::Busy { payload, holder }) => {
                context.game.transaction_close(&seal, false);
                drop(context);
                if incoming.defer(payload, &holder, std::time::Instant::now()) {
                    return false;
                }
                log::warn!("{}", incoming.busy_failure(&holder));
                if !incoming.raw {
                    endpoint.disconnect(Text::translate(
                        "disconnect.genericReason",
                        vec![Text::literal(format!(
                            "a mod's payload handler failed ({})",
                            horizon_client::raw_payloads::mod_of(&incoming.id)
                        ))],
                    ));
                }
                return true;
            }
            Err(mod_api::PayloadHandleError::Failed(error)) => Err(error),
            Ok(()) => Ok(()),
        };
        let reason = context.disconnect.take();
        let failure = context.failure.take().or_else(|| result.err());
        context.game.transaction_close(&seal, failure.is_none());
        #[cfg(test)]
        wasm_tests::observe(
            wasm_health_before,
            context.game,
            &endpoint,
            &incoming.id,
            failure.is_none(),
        );
        if let Some(error) = failure.as_ref().filter(|_| incoming.raw) {
            // A raw channel's bytes come from any client; one declaring mod's
            // refusal rolls back its own work only, and the next mod still runs.
            log::warn!(
                "Mod {} failed on raw channel {}: {error}",
                incoming.receiver.owner(),
                incoming.id.0
            );
            if let Some(reason) = reason {
                endpoint.disconnect(reason);
            }
        } else if let Some(error) = failure {
            // The reason goes to the log; the player is told only which mod's handler failed.
            log::warn!("payload {} failed: {error}", incoming.id.0);
            endpoint.disconnect(Text::translate(
                "disconnect.genericReason",
                vec![Text::literal(format!(
                    "a mod's payload handler failed ({})",
                    horizon_client::raw_payloads::mod_of(&incoming.id)
                ))],
            ));
        } else if let Some(reason) = reason {
            endpoint.disconnect(reason);
        }
        // Both the finish barrier and ingress credit cover root publication/rollback.
        drop(context);
        true
    }
}
struct MainContext<'a, 'resources> {
    game: &'a mut Game<'resources>,
    epoch: &'a Arc<Epoch>,
    endpoint: Endpoint,
    receiver: &'a mod_api::__private::game::PayloadReceiver,
    disconnect: Option<Text>,
    failure: Option<String>,
}
impl IPayloadContext for MainContext<'_, '_> {
    fn player(&self) -> Result<PlayerRef, PayloadContextError> {
        if !self.epoch.active() || self.disconnect.is_some() {
            return Err(PayloadContextError::Disconnected);
        }
        if self.epoch.phase == Phase::Configuration {
            return Err(PayloadContextError::NoPlayer);
        }
        self.endpoint
            .transport()
            .player()
            .ok_or(PayloadContextError::NoPlayer)
    }
    fn flow(&self) -> PayloadDirection {
        match self.endpoint.side() {
            Side::Client => PayloadDirection::ToClient,
            Side::Server => PayloadDirection::ToServer,
        }
    }
    fn protocol(&self) -> PayloadProtocol {
        match self.epoch.phase {
            Phase::Configuration => PayloadProtocol::Configuration,
            Phase::Play => PayloadProtocol::Play,
        }
    }
    fn server_host(&mut self) -> Result<&mut dyn mod_api::ServerHost, PayloadContextError> {
        if !self.epoch.active() || self.disconnect.is_some() {
            return Err(PayloadContextError::Disconnected);
        }
        if self.endpoint.side() != Side::Server {
            return Err(PayloadContextError::WrongSide);
        }
        Ok(self.game)
    }
    fn client_host(&mut self) -> Result<&mut dyn mod_api::ClientHost, PayloadContextError> {
        if !self.epoch.active() || self.disconnect.is_some() {
            return Err(PayloadContextError::Disconnected);
        }
        if self.endpoint.side() != Side::Client {
            return Err(PayloadContextError::WrongSide);
        }
        Ok(self.game)
    }
    fn reply(
        &mut self,
        id: &PayloadId,
        bytes: PayloadBytes,
    ) -> Result<PayloadAccepted, PayloadSendError> {
        if self.disconnect.is_some() {
            return Err(PayloadSendError::Disconnected);
        }
        self.game
            .queue_payload(self.endpoint.clone(), self.epoch.clone(), id, bytes)
    }
    fn check_payload_token(&self, token: &mod_api::PayloadToken) -> Result<(), PayloadSendError> {
        mod_api::__private::game::check_payload_token(token)
    }
    fn disconnect(&mut self, reason: Text) {
        self.disconnect = Some(reason);
    }
    fn enqueue_work(
        &mut self,
        work: Arc<dyn PayloadWork>,
    ) -> Result<PayloadWorkStatus, PayloadContextError> {
        if !self.epoch.active() || self.disconnect.is_some() {
            return Err(PayloadContextError::Disconnected);
        }
        let receiver = self.receiver.clone();
        if let Err(error) = receiver.run_work(work.as_ref(), self) {
            self.failure = Some(error);
            return Err(PayloadContextError::Disconnected);
        }
        Ok(PayloadWorkStatus::Complete)
    }
}
#[cfg(test)]
#[path = "payload_transport/config_tests.rs"]
mod config_tests;
#[cfg(test)]
#[path = "payload_transport/disconnect_tests.rs"]
mod disconnect_tests;
#[cfg(test)]
#[path = "payload_transport/raw_fault_tests.rs"]
mod raw_fault_tests;
#[cfg(test)]
#[path = "payload_transport/sender_tests.rs"]
mod sender_tests;
#[cfg(test)]
#[path = "payload_transport/tests.rs"]
mod tests;
#[cfg(test)]
#[path = "payload_transport/typed_tests.rs"]
mod typed_tests;
#[cfg(test)]
#[path = "payload_transport/typed_wasm_tests.rs"]
mod typed_wasm_tests;
#[cfg(test)]
#[path = "payload_transport/wasm_config_tests.rs"]
mod wasm_config_tests;
#[cfg(test)]
#[path = "payload_transport/wasm_tests.rs"]
mod wasm_tests;

#[cfg(test)]
mod shared_queue_tests {
    use super::SharedQueue;

    #[test]
    fn staged_sends_drain_once_in_submission_order() {
        let queue = SharedQueue::<u8>::default();
        queue.push(1);
        queue.push(2);
        queue.push(3);
        assert_eq!(queue.take(), vec![1, 2, 3]);
        assert!(queue.take().is_empty());
        // A send staged after a drain waits for the next one, not lost.
        queue.push(4);
        assert_eq!(queue.take(), vec![4]);
    }
}
