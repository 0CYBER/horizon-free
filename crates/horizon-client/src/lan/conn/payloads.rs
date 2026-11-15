//! Retry the head payload from the physical receiver's existing maintenance turn.
use super::*;

impl PlayConn {
    pub(super) fn payload_tick(&mut self, now: Instant) -> Option<Vec<u8>> {
        if self
            .payload_transport
            .connection
            .as_ref()
            .is_some_and(|connection| connection.payloads_pending())
        {
            let endpoint = crate::raw_payloads::Endpoint::server(self.payload_sender.clone());
            crate::raw_payloads::dispatch_pending(
                &endpoint,
                &crate::raw_payloads::MainQueue::Server {
                    events: self.shared.events.clone(),
                    endpoint: endpoint.clone(),
                    conn: Some(self.conn),
                },
            );
        }
        if self.payload_transport.closed() || self.sent_at.is_some() || now < self.next_keep_alive {
            return None;
        }
        let id = i64::try_from(self.watch.start.elapsed().as_millis()).unwrap_or(0);
        self.sent_at = Some((id, now));
        self.next_keep_alive = now + KEEP_ALIVE_EVERY;
        Some(play::Clientbound::KeepAlive(cfg::KeepAlive { id }).encode())
    }
}
