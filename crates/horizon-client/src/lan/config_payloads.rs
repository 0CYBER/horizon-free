//! Configuration transport has no player. Its replies share the engine writer queue.
use crate::{
    net::{PreparedPayload, queued_writer::QueuedWriter},
    raw_payloads::{Incoming, Transport},
};
use horizon_protocol::{
    connection::Connection, packets::configuration as cfg, payload_negotiation::Phase,
};
use mod_api::{PayloadId, PayloadSendError};
use std::sync::Arc;

#[derive(Clone)]
pub struct ConfigSender {
    writer: QueuedWriter,
    transport: Arc<Transport>,
}
impl ConfigSender {
    pub fn framing(&self) -> horizon_protocol::connection::Framing {
        self.writer.framing()
    }
    pub(crate) fn payload_capacity(&self) -> usize {
        self.writer.payload_capacity()
    }
    pub fn payload_transport(&self) -> &Arc<Transport> {
        &self.transport
    }
    pub fn prepare_payload(
        &self,
        body: Vec<u8>,
        id: &PayloadId,
    ) -> Result<PreparedPayload, PayloadSendError> {
        self.writer.prepare_payload(body, id)
    }
    pub(super) fn send_packet(&self, body: &[u8]) -> horizon_protocol::Result<()> {
        self.writer.send(body)
    }
    pub fn disconnect(&self, reason: horizon_core::nbt::Tag) {
        self.transport.close();
        self.writer
            .disconnect(cfg::Clientbound::Disconnect(cfg::Disconnect { reason }).encode());
    }
}

/// The engine's channels that every configuration loop reads itself, kept into play.
#[derive(Default)]
pub(super) struct EngineChannels {
    /// What the client announced in `minecraft:register`. Allocated on the first
    /// `minecraft:register` / `minecraft:unregister`: most connections never send one.
    pub announced: Option<Arc<super::announced_channels::AnnouncedChannels>>,
    /// Its `horizon:levels` answer.
    pub viewed_levels: super::viewed_levels::capability::Capability,
    /// Its `horizon:menu_extra` revision answer.
    pub menu_extra_rev: super::menu_extra_rev::Capability,
}
impl EngineChannels {
    /// Whether `payload` was one of these channels' and is taken, or the reason that closes the
    /// connection.
    pub fn receive(&mut self, payload: &cfg::CustomPayload) -> Result<bool, String> {
        // ServerCommonPacketListenerImpl hands the lists to the plugin messenger, in
        // configuration as in play: they stay on the connection.
        if super::announced_channels::is_list_channel(&payload.channel) {
            self.announced
                .get_or_insert_with(Arc::default)
                .apply(&payload.channel, &payload.data);
            return Ok(true);
        }
        if payload.channel == horizon_protocol::viewed_levels::CHANNEL {
            self.viewed_levels
                .receive(&payload.data)
                .map_err(|error| error.to_string())?;
            return Ok(true);
        }
        if payload.channel == horizon_protocol::menu_extra::CHANNEL {
            self.menu_extra_rev
                .receive(&payload.data)
                .map_err(|error| error.to_string())?;
            return Ok(true);
        }
        Ok(false)
    }
}

pub(super) struct ConfigIo<'a> {
    connection: &'a mut Connection,
    pub sender: Option<ConfigSender>,
    events: crate::net::ingress::Producer<super::NetEvent>,
    handed_off: bool,
    pub channels: EngineChannels,
}
impl<'a> ConfigIo<'a> {
    pub fn new(
        connection: &'a mut Connection,
        transport: Arc<Transport>,
        events: crate::net::ingress::Producer<super::NetEvent>,
    ) -> horizon_protocol::Result<Self> {
        let sender = if transport.local.manifest.entries().is_empty() {
            None
        } else {
            Some(ConfigSender {
                writer: QueuedWriter::new(connection.writer.take())?,
                transport,
            })
        };
        Ok(Self {
            connection,
            sender,
            events,
            handed_off: false,
            channels: EngineChannels::default(),
        })
    }
    pub fn send(&mut self, body: &[u8]) -> horizon_protocol::Result<()> {
        match &self.sender {
            Some(sender) => sender.writer.send(body),
            None => self.connection.send(body),
        }
    }
    /// Asks a Horizon-branded client how many levels besides its own it can view.
    pub fn ask_viewed_levels(&mut self) -> horizon_protocol::Result<()> {
        let query = self.channels.viewed_levels.query();
        self.send(&cfg::Clientbound::CustomPayload(query).encode())
    }
    /// Asks a Horizon-branded client which `horizon:menu_extra` layout it reads.
    pub fn ask_menu_extra_rev(&mut self) -> horizon_protocol::Result<()> {
        let query = self.channels.menu_extra_rev.query();
        self.send(&cfg::Clientbound::CustomPayload(query).encode())
    }
    pub fn application(
        &mut self,
        payload: cfg::CustomPayload,
    ) -> Result<Option<cfg::CustomPayload>, String> {
        match self.channels.receive(&payload) {
            Ok(true) => return Ok(None),
            Ok(false) => {}
            Err(error) => {
                self.disconnect(horizon_core::nbt::Tag::String(error.as_str().into()));
                return Err(error);
            }
        }
        let Some(sender) = &self.sender else {
            return Ok(Some(payload));
        };
        match Incoming::classify(&sender.transport, Phase::Configuration, payload)
            .map_err(|error| error.to_string())?
        {
            crate::raw_payloads::Classified::Other(payload) => Ok(Some(payload)),
            // Raw channels are play-only: configuration never classifies one.
            crate::raw_payloads::Classified::Raw(_) => Ok(None),
            crate::raw_payloads::Classified::Application(incoming) => {
                let endpoint = crate::raw_payloads::Endpoint::configuration(sender.clone());
                crate::raw_payloads::dispatch(
                    incoming,
                    endpoint.clone(),
                    crate::raw_payloads::MainQueue::Server {
                        events: self.events.clone(),
                        endpoint,
                        conn: None,
                    },
                );
                Ok(None)
            }
        }
    }
    pub fn disconnect(&mut self, reason: horizon_core::nbt::Tag) {
        match &self.sender {
            Some(sender) => sender.disconnect(reason),
            None => {
                let _ = self
                    .connection
                    .send(&cfg::Clientbound::Disconnect(cfg::Disconnect { reason }).encode());
            }
        }
    }
    pub fn finish(&mut self) -> Result<(), String> {
        if let Some(sender) = &self.sender {
            self.connection.writer = sender.writer.drain_and_take().map_err(|e| e.to_string())?;
        }
        self.handed_off = true;
        Ok(())
    }
}
impl std::ops::Deref for ConfigIo<'_> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        self.connection
    }
}
impl std::ops::DerefMut for ConfigIo<'_> {
    fn deref_mut(&mut self) -> &mut Connection {
        self.connection
    }
}
impl Drop for ConfigIo<'_> {
    fn drop(&mut self) {
        if !self.handed_off
            && let Some(sender) = &self.sender
        {
            sender.transport.close();
            if !sender.writer.is_terminal() {
                sender.writer.shutdown();
            }
        }
    }
}
