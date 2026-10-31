//! A spectating guest's camera (`ServerPlayer.camera`): chosen by a click
//! (`handleSpectatorAction`) or `/spectate`, followed every tick, dropped
//! when the entity dies, the player sneaks, is teleported or leaves
//! spectator mode.
use super::{Game, commands::Target, lan::LanHost};
use horizon_protocol::packets::play::{self, Clientbound as C};

/// `ServerPlayer.camera` of one guest other than itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct GuestCamera {
    /// The camera entity's network id.
    pub id: i32,
    /// The guest's teleport counter once the view moved to the camera: a
    /// later teleport is `teleportTo(.., resetCamera = true)`.
    teleports: i32,
}

/// Whether `ServerPlayer.tick` hands the view back to the player: the
/// camera is gone (`!isAlive`), the player wants to stop riding (sneaks),
/// left spectator mode (`setGameMode`) or was teleported (`resetCamera`).
fn camera_ends(alive: bool, sneaking: bool, spectator: bool, teleported: bool) -> bool {
    !alive || sneaking || !spectator || teleported
}

impl Game<'_> {
    /// `ServerPlayer.isAlive` of the camera, with where it is and how it
    /// looks (`absSnapTo`'s arguments): a player, the host or any other
    /// entity of the level, by network id.
    fn camera_state(&self, host: &LanHost, id: i32) -> Option<([f64; 3], [f32; 2])> {
        if let Some(g) = host
            .hub
            .guests
            .iter()
            .find(|g| g.entity_id == id && host.hub.here(g))
        {
            return (!g.dead).then_some((g.position, [g.yaw, g.pitch]));
        }
        if id == host.host_entity {
            let p = self.player.as_ref()?;
            return Some((
                [p.position.x, p.position.y, p.position.z],
                [self.yaw, self.pitch],
            ));
        }
        let t = self.level_entity(id)?;
        if let Target::Mob(m) = t
            && !self.mobs.iter().any(|x| x.id == m && x.alive())
        {
            return None;
        }
        Some((self.target_position(t)?, self.target_rotation(t)))
    }

    /// `Entity.isPickable` of the entity with that id.
    fn camera_pickable(&self, host: &LanHost, id: i32) -> bool {
        // Player.isPickable: not a spectator.
        if let Some(g) = host.hub.guests.iter().find(|g| g.entity_id == id) {
            return !g.spectator() && !g.dead;
        }
        if id == host.host_entity {
            return self.player.is_some() && !self.spectator();
        }
        match self.level_entity(id) {
            // LivingEntity: not removed. ArmorStand: not a marker.
            Some(Target::Mob(_)) => self.camera_state(host, id).is_some(),
            Some(Target::Stand(s)) => self.armor_stands.iter().any(|a| a.id == s && !a.marker),
            // BlockAttachedEntity, minecarts, boats, end crystals.
            Some(Target::Frame(_) | Target::Painting(_) | Target::Boat(_) | Target::Cart(_))
            | Some(Target::Crystal(_)) => true,
            // Interaction.
            Some(t @ Target::Other(_)) => self.target_type(t) == "minecraft:interaction",
            _ => false,
        }
    }

    /// `ServerGamePacketListenerImpl.handleSpectatorAction`.
    pub(super) fn guest_spectator_action(
        &mut self,
        host: &mut LanHost,
        gi: usize,
        target: Option<i32>,
    ) {
        let g = &host.hub.guests[gi];
        if !g.loaded || !g.spectator() {
            return;
        }
        let conn = g.conn;
        let Some(id) = target else {
            return;
        };
        let Some(bounds) = self.guest_target_box(host, gi, id) else {
            return;
        };
        let Some((at, _)) = self.camera_state(host, id) else {
            return;
        };
        let block = at.map(|v| v.floor() as i32);
        if !self
            .border
            .contains(f64::from(block[0]), f64::from(block[2]))
        {
            return;
        }
        if self.as_guest(host, conn, |g| {
            g.within_entity_interaction_range(&bounds, 3.0)
        }) != Some(true)
        {
            return;
        }
        if self.camera_pickable(host, id) {
            self.guest_set_camera(host, gi, Some(id));
        }
    }

    /// `ServerPlayer.setCamera`: the view moves to the new camera, which
    /// the client is told, or back to the player (`None`).
    pub(super) fn guest_set_camera(&mut self, host: &mut LanHost, gi: usize, camera: Option<i32>) {
        let (conn, own) = (host.hub.guests[gi].conn, host.hub.guests[gi].entity_id);
        let camera = camera.filter(|id| *id != own);
        if self.cmds.guest_cameras.get(&conn).map(|c| c.id) == camera {
            return;
        }
        let at = match camera {
            Some(id) => self.camera_state(host, id).map(|s| s.0),
            None => Some(host.hub.guests[gi].position),
        };
        match camera {
            Some(id) => {
                self.cmds
                    .guest_cameras
                    .insert(conn, GuestCamera { id, teleports: 0 });
            }
            None => {
                self.cmds.guest_cameras.remove(&conn);
            }
        }
        // teleportTo(level, camera.x, y, z): same look.
        host.hub.teleport(gi, at);
        host.hub.send_to(
            conn,
            &C::SetCamera(play::SetCamera {
                camera_id: camera.unwrap_or(own),
            }),
        );
        let teleports = host.hub.guests[gi].teleport_counter;
        if let Some(c) = self.cmds.guest_cameras.get_mut(&conn) {
            c.teleports = teleports;
        }
    }

    /// The camera part of `ServerPlayer.tick` for every guest, after the
    /// `/spectate` requests commands queued.
    pub(super) fn tick_guest_cameras(&mut self, host: &mut LanHost) {
        for (conn, id) in std::mem::take(&mut self.cmds.camera_requests) {
            if let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) {
                self.guest_set_camera(host, gi, id);
            }
        }
        let cameras: Vec<(u32, GuestCamera)> = self
            .cmds
            .guest_cameras
            .iter()
            .map(|(c, v)| (*c, *v))
            .collect();
        for (conn, camera) in cameras {
            let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) else {
                self.cmds.guest_cameras.remove(&conn);
                continue;
            };
            let g = &host.hub.guests[gi];
            let state = self.camera_state(host, camera.id);
            if camera_ends(
                state.is_some(),
                g.sneaking,
                g.spectator(),
                g.teleport_counter != camera.teleports,
            ) {
                self.guest_set_camera(host, gi, None);
            } else if let Some((at, [yaw, pitch])) = state {
                // absSnapTo; the level's chunk source follows the player.
                let g = &mut host.hub.guests[gi];
                g.position = at;
                (g.yaw, g.pitch) = (yaw, pitch);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::camera_ends;

    /// A living camera is followed until the player sneaks, leaves
    /// spectator mode or is teleported; a dead one ends it at once.
    #[test]
    fn camera_ends_like_server_player_tick() {
        assert!(!camera_ends(true, false, true, false));
        assert!(camera_ends(false, false, true, false));
        assert!(camera_ends(true, true, true, false));
        assert!(camera_ends(true, false, false, false));
        assert!(camera_ends(true, false, true, true));
    }
}
