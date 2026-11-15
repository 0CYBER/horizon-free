//! Connection-local negotiation state; an empty installation shares immutable storage only.
use super::{
    Local, Side,
    connection::{Connection, Epoch, IngressLease},
};
use horizon_protocol::payload_negotiation::Entry;
use mod_api::{PayloadSendError, PlayerRef};
use std::sync::{Arc, OnceLock};

pub struct Transport {
    pub local: Arc<Local>,
    pub connection: Option<Arc<Connection>>,
}
impl Transport {
    pub fn capture(side: Side) -> Result<Arc<Self>, String> {
        Ok(Self::new(Local::capture(side)?, side))
    }
    /// A listener shares Local, never mutable connection epochs, across guests.
    pub fn new(local: Arc<Local>, side: Side) -> Arc<Self> {
        if !local.has_declarations() {
            return Self::empty();
        }
        Arc::new(Self {
            connection: Some(Connection::new(local.clone(), side)),
            local,
        })
    }
    pub(crate) fn empty() -> Arc<Self> {
        static EMPTY: OnceLock<Arc<Transport>> = OnceLock::new();
        EMPTY
            .get_or_init(|| {
                Arc::new(Self {
                    local: Local::empty(),
                    connection: None,
                })
            })
            .clone()
    }
    /// Reserve physical-connection ingress storage, independent of protocol epoch.
    pub fn reserve_ingress(&self, owned_bytes: usize) -> Result<IngressLease, PayloadSendError> {
        self.connection
            .as_ref()
            .ok_or(PayloadSendError::Disconnected)?
            .reserve_ingress(owned_bytes)
    }
    pub fn bind_player(&self, player: PlayerRef) {
        if let Some(connection) = &self.connection {
            connection.bind_player(player);
        }
    }
    pub fn closed(&self) -> bool {
        self.connection
            .as_ref()
            .is_some_and(|connection| connection.closed())
    }
    pub fn player(&self) -> Option<PlayerRef> {
        self.connection
            .as_ref()
            .and_then(|connection| connection.player())
    }
    pub fn epoch(&self) -> Option<Arc<Epoch>> {
        self.connection
            .as_ref()
            .and_then(|connection| connection.epoch())
    }
    pub fn begin_configuration(&self) {
        if let Some(connection) = &self.connection {
            connection.begin_configuration();
        }
    }
    pub fn accept(&self, entries: &[Entry]) -> Result<(), String> {
        if let Some(connection) = &self.connection {
            connection.accept(entries)?;
        }
        Ok(())
    }
    pub fn finish_configuration(&self) -> Result<(), String> {
        if let Some(connection) = &self.connection {
            connection.finish_configuration()?;
        }
        Ok(())
    }
    pub fn finish_configuration_with(
        &self,
        publish: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        match &self.connection {
            Some(connection) => connection.finish_configuration_with(publish),
            None => publish(),
        }
    }
    pub fn close(&self) {
        if let Some(connection) = &self.connection {
            connection.close();
        }
    }
}
