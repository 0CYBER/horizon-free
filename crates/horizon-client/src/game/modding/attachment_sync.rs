//! Attachment sync (docs/modding/src/how-to/native-mods.md, "Server attachments"): the server
//! sends the synced values of the holders a level pass lent to mods, and whole
//! snapshots to the players who started seeing a holder; a client keeps them.
//!
//! The channel is an engine declaration in the negotiated payload table
//! (`raw_payloads::engine`): a message goes only to a placed player whose connection
//! offered it, is prepared against that connection's current epoch and is published
//! at the root commit, in order with the player's other packets.
use super::Game;
use crate::lan::{
    conn::PlayerSender,
    guest::Guest,
    hub::{AttachmentSnapshot, Hub},
};
use horizon_client::raw_payloads::Endpoint;
use horizon_mod_runtime::attachments::{Attachments, Holder};
use horizon_protocol::attachment_sync::{self, Message};
use mod_api::{PayloadBytes, PayloadId, PayloadSendError};

/// Whether `guest` reads attachment sync: placed, and its connection's play epoch
/// has the channel negotiated. A vanilla client never offers it.
fn reads_attachment_sync(guest: &Guest) -> bool {
    guest.placed && guest.negotiated_attachment_sync()
}

/// The guests `holder`'s changes go to: those in the level in place who see it,
/// and a player's own wherever they are.
fn recipients(hub: &Hub, holder: Holder) -> impl Iterator<Item = &Guest> {
    let owner = match holder {
        Holder::Player(uuid) => hub.holder_guest(uuid, &hub.config.dimension),
        _ => None,
    };
    let sees = move |g: &Guest| match holder {
        Holder::Level => true,
        Holder::Chunk([x, z]) => g.sent_chunks.contains(&(x, z)),
        Holder::BlockEntity([x, _, z]) => g.sent_chunks.contains(&(x >> 4, z >> 4)),
        Holder::Entity(id) => g.tracker.is_tracking(id as i32),
        Holder::Player(_) => owner.is_some_and(|o| g.tracker.is_tracking(o.entity_id)),
        _ => false,
    };
    hub.guests.iter().filter(move |g| {
        (owner.is_some_and(|o| g.conn == o.conn && g.participates_online())
            || (hub.here(g) && sees(g)))
            && reads_attachment_sync(g)
    })
}

/// The senders of `holder`'s recipients, taken out of the hub so a send may journal.
fn recipient_senders(hub: &Hub, holder: Holder) -> Vec<PlayerSender> {
    recipients(hub, holder).map(|g| g.sender.clone()).collect()
}

impl Game<'_> {
    /// The messages that give a player `holder`'s synced attachments whole: its
    /// own, and for a chunk those of its block entities. A player holder with
    /// none still gives one, to drop what the client holds.
    fn attachment_snapshot(&mut self, holder: Holder) -> Vec<Message> {
        let changes = match holder {
            // Read in place: a chunk lent to a mod is copied and saved.
            Holder::Chunk([x, z]) => self
                .world
                .chunks
                .get(&(x, z))
                .and_then(|chunk| chunk.saved.as_ref())
                .map(|extras| extras.attachments.synced()),
            // Read in place: lending the map would give the entity one.
            Holder::Entity(id) => self.entity_attachments.get(&id).map(|map| map.synced()),
            _ => self
                .attachment_map_quiet(holder, mod_api::AttachmentAccess::Existing)
                .ok()
                .map(|map| map.synced()),
        }
        .unwrap_or_default();
        let mut holders = vec![(holder, changes)];
        if let Holder::Chunk([x, z]) = holder {
            holders.extend(
                self.world
                    .block_entities
                    .chunk_attachments(x, z)
                    .into_iter()
                    .map(|((x, y, z), map)| (Holder::BlockEntity([x, y, z]), map.synced())),
            );
        }
        holders
            .into_iter()
            .filter(|(holder, changes)| !changes.is_empty() || matches!(holder, Holder::Player(_)))
            .map(|(holder, changes)| Message {
                holder,
                replace: true,
                changes,
            })
            .collect()
    }

    /// The holders players started seeing are sent whole to them, in the pass
    /// of the level they are in (another's stay queued), then the block
    /// entities a piston landed whole to the players who see them, then the
    /// synced changes of the holders lent to mods since the last pass to the
    /// players who see them.
    pub(super) fn flush_attachment_sync(&mut self) {
        let landings = std::mem::take(&mut self.world.attachment_landings);
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        let snapshots = std::mem::take(&mut host.hub.attachment_snapshots);
        for snapshot in snapshots {
            let AttachmentSnapshot { player, holder } = snapshot;
            let Some(host) = self.lan.as_mut() else {
                return;
            };
            let hub = &mut host.hub;
            let Some(guest) = hub.online_players().find(|g| g.uuid == player) else {
                continue;
            };
            if !reads_attachment_sync(guest) {
                continue;
            }
            if guest.dimension != hub.config.dimension {
                hub.attachment_snapshots.push(snapshot);
                continue;
            }
            // A guest is a player holder, whose entity another player tracks.
            let holder = match holder {
                Holder::Entity(id) => hub
                    .guests
                    .iter()
                    .find(|g| g.entity_id == id as i32)
                    .map_or(holder, |g| Holder::Player(g.uuid)),
                _ => holder,
            };
            let messages = self.attachment_snapshot(holder);
            let sender = self
                .lan
                .as_ref()
                .and_then(|host| host.hub.guests.iter().find(|g| g.uuid == player))
                .map(|guest| guest.sender.clone());
            if let Some(sender) = sender {
                for message in &messages {
                    self.send_attachment_sync(std::slice::from_ref(&sender), message);
                }
            }
        }
        // A landed block entity is a new holder: what its clients held of it
        // went with their moving piston's copy.
        for holder in landings {
            let messages = self.attachment_snapshot(holder);
            let senders = self
                .lan
                .as_ref()
                .map_or_else(Vec::new, |host| recipient_senders(&host.hub, holder));
            for message in &messages {
                self.send_attachment_sync(&senders, message);
            }
        }
        for holder in std::mem::take(&mut self.world.attachment_syncs) {
            let Ok(map) = self.attachment_map_quiet(holder, mod_api::AttachmentAccess::Existing)
            else {
                continue;
            };
            let changes = map.take_sync_changes();
            if changes.is_empty() {
                continue;
            }
            let message = Message {
                holder,
                replace: false,
                changes,
            };
            let senders = self
                .lan
                .as_ref()
                .map_or_else(Vec::new, |host| recipient_senders(&host.hub, holder));
            self.send_attachment_sync(&senders, &message);
        }
    }

    /// Queue `message` for each of `senders` through the negotiated transport. A peer
    /// that has the channel absent, or is between epochs, is sent nothing and that is no
    /// error. Any other refusal is named in the log once per connection epoch: a message
    /// past the connection's frame limits or one that cannot be written is not sent and the
    /// player stays joined; a full outgoing queue closes the connection, as a full queue
    /// did before, since the replica would miss the update.
    fn send_attachment_sync(&mut self, senders: &[PlayerSender], message: &Message) {
        if senders.is_empty() {
            return;
        }
        let body = attachment_sync::encode(message).map_err(|e| e.to_string());
        for sender in senders {
            let Err(error) = self.queue_attachment_sync(sender, &body) else {
                continue;
            };
            if !refused(&error) {
                continue;
            }
            if sender
                .payload_transport()
                .epoch()
                .is_none_or(|epoch| epoch.first_engine_refusal())
            {
                log::warn!(
                    "Horizon: attachment sync for {:?} not sent: {error:?}",
                    message.holder
                );
            }
            if matches!(error, PayloadSendError::Backpressure) {
                sender.close();
            }
        }
    }

    /// Queue one encoded message (or its encoding failure) for one connection: the
    /// transport's typed answer, `FrameTooLarge` past the connection's frame limits.
    fn queue_attachment_sync(
        &mut self,
        sender: &PlayerSender,
        body: &Result<Vec<u8>, String>,
    ) -> Result<(), PayloadSendError> {
        let body = body
            .as_ref()
            .map_err(|reason| PayloadSendError::EncodeFailed(reason.clone()))?;
        self.queue_current_payload(
            Endpoint::server(sender.clone()),
            &PayloadId(attachment_sync::CHANNEL.into()),
            PayloadBytes(body.clone()),
        )
        .map(|_| ())
    }
}

/// Whether a send error is a refusal to name, not a peer without the channel or a
/// connection between epochs.
fn refused(error: &PayloadSendError) -> bool {
    !matches!(
        error,
        PayloadSendError::UnsupportedPeer(_)
            | PayloadSendError::NotNegotiated
            | PayloadSendError::WrongProtocol(_)
            | PayloadSendError::StaleEpoch
            | PayloadSendError::Disconnected
    )
}

impl Game<'_> {
    /// A `horizon:attachment_sync` payload the server sent the window.
    pub(super) fn remote_attachment_sync(&mut self, data: &[u8]) {
        let message = match attachment_sync::decode(data) {
            Ok(message) => message,
            Err(e) => return log::debug!("Horizon: a bad attachment sync: {e}"),
        };
        let Some(remote) = self.remote.as_mut() else {
            return;
        };
        let map = remote.attachments.entry(message.holder).or_default();
        if message.replace {
            map.replace_contents(Attachments::default());
        }
        for change in message.changes {
            map.apply_sync(change);
        }
        self.bind_remote_attachment(message.holder);
    }

    /// Publish a synced copy once its scoped client holder can be named.
    pub(super) fn bind_remote_attachment(&mut self, holder: Holder) {
        use mod_api::{AttachmentHolder as H, ChunkRef, EntityRef};
        let Some(remote) = &self.remote else { return };
        let Some(level) = self.level(&remote.level) else {
            return;
        };
        let scoped = match holder {
            Holder::Entity(id) => H::Entity(EntityRef::new(level.epoch(), id)),
            Holder::Player(uuid) => {
                let id = if uuid == remote.profile() {
                    u32::try_from(remote.entity_id).ok()
                } else {
                    remote.player_entity(uuid)
                };
                let Some(id) = id else { return };
                H::Player(mod_api::__private::game::player_ref(
                    EntityRef::new(level.epoch(), id),
                    uuid,
                ))
            }
            Holder::Chunk(pos) => H::Chunk(ChunkRef::new(level, pos)),
            Holder::BlockEntity(pos) => {
                let Some(entity) = self.block_entity(level, pos) else {
                    return;
                };
                H::BlockEntity(entity)
            }
            Holder::Level => H::Level(level),
            _ => return,
        };
        if let Some(map) = self
            .remote
            .as_mut()
            .and_then(|r| r.attachments.get_mut(&holder))
            && !map.has_index_binding()
        {
            map.bind_index(self.attachment_indexes.entry(self.dim).or_default(), scoped);
        }
    }
}

#[cfg(test)]
#[path = "attachment_sync/route_cases_tests.rs"]
mod route_cases_tests;
#[cfg(test)]
#[path = "attachment_sync/transport_tests.rs"]
mod transport_tests;
