//! Whether one player's client is shown another, by the rules the entity tracker follows, and a
//! block-change acknowledgement and a block shown to one player.
use super::{Game, native_server::PlayerRead};
use horizon_client::lan::{guest::Guest, player_shown::player_shown};
use horizon_protocol::packets::play::{self, Clientbound as C};
use horizon_world::registry::BlockStateId;
use mod_api::{PlayerRef, PlayerWorldRefusal as Refusal};

/// `BlockPos.asLong` packs x and z in 26 signed bits and y in 12: a value outside wraps.
const HORIZONTAL: std::ops::Range<i32> = -(1 << 25)..(1 << 25);
const VERTICAL: std::ops::Range<i32> = -(1 << 11)..(1 << 11);

impl Game<'_> {
    /// The server's record of a player in its list, by handle.
    fn listed_guest(&self, player: PlayerRef) -> Result<&Guest, Refusal> {
        if self.native_lifecycle.epoch() != Some(player.entity().epoch()) {
            return Err(Refusal::Stale);
        }
        let guests = &self.lan.as_ref().ok_or(Refusal::Stale)?.hub.guests;
        let index = guests
            .identity_index(player.uuid(), player.entity().id())
            .ok_or(Refusal::Stale)?;
        let guest = &guests[index];
        if guest.is_fake() {
            Err(Refusal::NoConnection)
        } else if guest.placed {
            Ok(guest)
        } else {
            Err(Refusal::Stale)
        }
    }
    /// The connection of a player a client stands behind, in their turn or out of it.
    pub(super) fn player_connection(&self, player: PlayerRef) -> Result<u32, Refusal> {
        if !self.is_logical_server() {
            return Err(Refusal::Client);
        }
        let conn = match self.player_handle_read(player).ok_or(Refusal::Stale)? {
            PlayerRead::Acting { .. } => self.lan_acting.ok_or(Refusal::NoConnection)?,
            PlayerRead::Guest(g) if !g.is_fake() => g.conn,
            PlayerRead::Guest(_) => return Err(Refusal::NoConnection),
        };
        let guest = self.lan.as_ref().and_then(|lan| lan.hub.guest(conn));
        match guest {
            Some(guest) if !guest.is_fake() => Ok(conn),
            Some(_) => Err(Refusal::NoConnection),
            None => Err(Refusal::Stale),
        }
    }
    /// `ackBlockChangesUpTo` for a connected player: the hub's pending ack, sent after the
    /// next tick's block changes.
    pub(super) fn player_ack_block_changes(
        &mut self,
        player: PlayerRef,
        sequence: i32,
    ) -> Result<(), Refusal> {
        let conn = self.player_connection(player)?;
        if sequence < 0 {
            return Err(Refusal::NegativeSequence { sequence });
        }
        let host = self.lan.as_ref().ok_or(Refusal::Stale)?;
        host.hub.ack(conn, sequence);
        Ok(())
    }
    /// `Player.sendBlockChange`: one `BlockUpdate` to that player's client, now.
    pub(super) fn player_show_block(
        &mut self,
        player: PlayerRef,
        pos: [i32; 3],
        state: BlockStateId,
    ) -> Result<(), Refusal> {
        let conn = self.player_connection(player)?;
        let [x, y, z] = pos;
        if !(HORIZONTAL.contains(&x) && VERTICAL.contains(&y) && HORIZONTAL.contains(&z)) {
            return Err(Refusal::PositionNotSendable { pos });
        }
        if self.pack.assets.registry.state(state).is_none() {
            return Err(Refusal::UnknownState);
        }
        let guest = self
            .lan
            .as_ref()
            .and_then(|lan| lan.hub.guest(conn))
            .ok_or(Refusal::Stale)?;
        guest.send(&C::BlockUpdate(play::BlockUpdate {
            pos,
            state: state.0 as i32,
        }));
        Ok(())
    }
    /// `ServerPlayer.broadcastToPlayer` of `target` for `viewer` (`lan::player_shown`).
    pub(super) fn player_sees(
        &self,
        viewer: PlayerRef,
        target: PlayerRef,
    ) -> Result<bool, Refusal> {
        if !self.is_logical_server() {
            return Err(Refusal::Client);
        }
        let (v, t) = (self.listed_guest(viewer)?, self.listed_guest(target)?);
        if v.uuid == t.uuid {
            return Ok(true);
        }
        // The tracker's own level (`Hub::tracker_hidden`): also everyone's while the target is being sent
        // again, as the player list (`Hub::sees`) hides it then.
        let hidden = self
            .lan
            .as_ref()
            .and_then(|lan| lan.hub.tracker_hidden(t.uuid));
        Ok(player_shown(
            v.spectator(),
            v.permission,
            t.spectator(),
            t.watching,
            hidden,
        ))
    }
}
