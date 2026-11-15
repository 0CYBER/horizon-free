//! NETWORK callbacks have no World borrow. Their replies and main work publish on success.
use super::{
    Body, Endpoint, Incoming, MainQueue, Prepared, Side,
    main_queue::{FailedMain, PreparedMain},
};
use mod_api::{
    HandlerThread, IPayloadContext, PayloadAccepted, PayloadBytes, PayloadContextError,
    PayloadDirection, PayloadId, PayloadProtocol, PayloadSendError, PayloadWork, PayloadWorkStatus,
    PlayerRef, Text,
};
use std::sync::Arc;

enum After {
    Reply(Prepared),
    Work(PreparedMain),
}
enum FailedPublication {
    Reply(PayloadSendError),
    Main(FailedMain),
}
const _: () = assert!(std::mem::size_of::<After>() <= 256);

/// Called by the physical receiving executor. Main events use its ordinary ordered stream.
pub fn dispatch(incoming: Incoming, endpoint: Endpoint, main: MainQueue) {
    let Some(connection) = endpoint.transport().connection.as_ref() else {
        return;
    };
    if let Some(incoming) = connection.queue_payload(incoming)
        && let Some(incoming) = execute(incoming, &endpoint, &main, std::time::Instant::now())
    {
        connection.defer_payload(incoming);
    }
    dispatch_pending(&endpoint, &main);
    if !matches!(main, MainQueue::Server { conn: Some(_), .. }) {
        // Dedicated configuration/client receivers already own their physical
        // ordered stream. Park that receiver, never a guest or the pooled server.
        while !endpoint.transport().closed() && connection.wait_payloads() {
            dispatch_pending(&endpoint, &main);
        }
    }
}

/// The pooled play receiver calls this from its existing maintenance turn.
pub fn dispatch_pending(endpoint: &Endpoint, main: &MainQueue) {
    dispatch_pending_at(endpoint, main, std::time::Instant::now());
}

pub(super) fn dispatch_pending_at(endpoint: &Endpoint, main: &MainQueue, now: std::time::Instant) {
    let Some(connection) = endpoint.transport().connection.as_ref() else {
        return;
    };
    if !connection.payloads_pending() {
        return;
    }
    while let Some(incoming) = connection.next_payload() {
        if let Some(incoming) = execute(incoming, endpoint, main, now) {
            connection.defer_payload(incoming);
            break;
        }
    }
}

fn execute(
    mut incoming: Incoming,
    endpoint: &Endpoint,
    main: &MainQueue,
    now: std::time::Instant,
) -> Option<Incoming> {
    if !incoming.epoch.active() {
        return None;
    }
    if let Some(holder) = incoming.expired_busy(now) {
        log::warn!("{}", incoming.busy_failure(holder));
        endpoint.disconnect(failure(&format!(
            "a mod's network handler failed ({})",
            mod_of(&incoming.id)
        )));
        return None;
    }
    if matches!(incoming.body, Body::Work(_)) || incoming.receiver.thread == HandlerThread::Main {
        let epoch = incoming.epoch.clone();
        let prepared = match main.prepare(incoming) {
            Ok(prepared) => prepared,
            Err(error) => {
                endpoint.disconnect(failure(&format!("payload main refused: {error:?}")));
                return None;
            }
        };
        // The value stays outside the closure if invalidated before publication.
        let mut prepared = Some(prepared);
        let result = epoch.with_active(|| prepared.take().unwrap().publish());
        drop(prepared);
        if let Some(Err(error)) = result {
            let reason = error.reason();
            drop(error);
            endpoint.disconnect(failure(&format!("payload main refused: {reason:?}")));
        }
        return None;
    }
    let Body::Payload(bytes) =
        std::mem::replace(&mut incoming.body, Body::Payload(PayloadBytes(Vec::new())))
    else {
        unreachable!("work is sent to main above");
    };
    let mut context = NetworkContext {
        incoming: &incoming,
        endpoint,
        main,
        after: Vec::new(),
        disconnect: None,
    };
    match incoming.receiver.handle(bytes, &mut context) {
        Err(mod_api::PayloadHandleError::Busy { payload, holder }) => {
            drop(context);
            if incoming.defer(payload, &holder, now) {
                return Some(incoming);
            }
            log::warn!("{}", incoming.busy_failure(&holder));
            endpoint.disconnect(failure(&format!(
                "a mod's network handler failed ({})",
                mod_of(&incoming.id)
            )));
        }
        Err(mod_api::PayloadHandleError::Failed(error)) => {
            // Drop reservations and guest work before closing, publishing no partial actions.
            drop(context);
            // The reason (a guest's deadline, trap or refusal) goes to the log; the player is told
            // only which mod's handler failed.
            log::warn!("payload {} failed: {error}", incoming.id.0);
            #[cfg(test)]
            super::staging_probe::failure(&incoming.id.0, &error);
            endpoint.disconnect(failure(&format!(
                "a mod's network handler failed ({})",
                mod_of(&incoming.id)
            )));
        }
        Ok(()) => {
            let reason = context.disconnect.take();
            let mut after = std::mem::take(&mut context.after);
            drop(context);
            let result = incoming.epoch.with_active(|| {
                // Never consume the container here: an early error must not drop
                // remaining opaque Work captures while the epoch gate is held.
                for action in &mut after {
                    match action.take().unwrap() {
                        After::Reply(reply) => reply.publish().map_err(FailedPublication::Reply)?,
                        After::Work(work) => work.publish().map_err(FailedPublication::Main)?,
                    }
                }
                Ok::<(), FailedPublication>(())
            });
            drop(after);
            match result {
                Some(Err(error)) => {
                    let message = match &error {
                        FailedPublication::Reply(error) => {
                            format!("payload {} reply refused: {error:?}", incoming.id.0)
                        }
                        FailedPublication::Main(error) => {
                            format!("payload main refused: {:?}", error.reason())
                        }
                    };
                    drop(error);
                    endpoint.disconnect(failure(&message));
                }
                Some(Ok(())) => {
                    if let Some(reason) = reason {
                        endpoint.disconnect(reason);
                    }
                }
                // Phase/connection cancellation cancels the staged disconnect too.
                None => {}
            }
        }
    }
    None
}
/// The mod a channel belongs to: its namespace.
pub fn mod_of(id: &PayloadId) -> &str {
    id.0.split_once(':')
        .map_or(&*id.0, |(namespace, _)| namespace)
}
fn failure(message: &str) -> Text {
    Text::translate("disconnect.genericReason", vec![Text::literal(message)])
}
struct NetworkContext<'a> {
    incoming: &'a Incoming,
    endpoint: &'a Endpoint,
    main: &'a MainQueue,
    after: Vec<Option<After>>,
    disconnect: Option<Text>,
}
impl IPayloadContext for NetworkContext<'_> {
    fn player(&self) -> Result<PlayerRef, PayloadContextError> {
        if !self.incoming.epoch.active() || self.disconnect.is_some() {
            return Err(PayloadContextError::Disconnected);
        }
        if self.incoming.epoch.phase == horizon_protocol::payload_negotiation::Phase::Configuration
        {
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
        match self.incoming.epoch.phase {
            horizon_protocol::payload_negotiation::Phase::Configuration => {
                PayloadProtocol::Configuration
            }
            horizon_protocol::payload_negotiation::Phase::Play => PayloadProtocol::Play,
        }
    }
    fn server_host(&mut self) -> Result<&mut dyn mod_api::ServerHost, PayloadContextError> {
        if !self.incoming.epoch.active() || self.disconnect.is_some() {
            return Err(PayloadContextError::Disconnected);
        }
        if self.endpoint.side() != Side::Server {
            return Err(PayloadContextError::WrongSide);
        }
        Err(PayloadContextError::WrongThread)
    }
    fn client_host(&mut self) -> Result<&mut dyn mod_api::ClientHost, PayloadContextError> {
        if !self.incoming.epoch.active() || self.disconnect.is_some() {
            return Err(PayloadContextError::Disconnected);
        }
        if self.endpoint.side() != Side::Client {
            return Err(PayloadContextError::WrongSide);
        }
        Err(PayloadContextError::WrongThread)
    }
    fn reply(
        &mut self,
        id: &PayloadId,
        bytes: PayloadBytes,
    ) -> Result<PayloadAccepted, PayloadSendError> {
        if self.disconnect.is_some() {
            return Err(PayloadSendError::Disconnected);
        }
        let reply = self.endpoint.prepare(&self.incoming.epoch, id, bytes)?;
        self.after.push(Some(After::Reply(reply)));
        #[cfg(any(test, feature = "test-payload-probe"))]
        super::staging_probe::prepared(&self.incoming.id.0, false);
        Ok(PayloadAccepted::Queued)
    }
    fn disconnect(&mut self, reason: Text) {
        self.disconnect = Some(reason);
    }
    fn enqueue_work(
        &mut self,
        work: Arc<dyn PayloadWork>,
    ) -> Result<PayloadWorkStatus, PayloadContextError> {
        if self.disconnect.is_some() {
            return Err(PayloadContextError::Disconnected);
        }
        let work = self.incoming.work(self.endpoint.transport(), work)?;
        let prepared = self.main.prepare(work)?;
        self.after.push(Some(After::Work(prepared)));
        #[cfg(any(test, feature = "test-payload-probe"))]
        super::staging_probe::prepared(&self.incoming.id.0, true);
        Ok(PayloadWorkStatus::Queued)
    }
}
