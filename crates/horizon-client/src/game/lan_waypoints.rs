//! The locator bar of the players on a served world (lan/waypoints.rs): what
//! each player looks like to the waypoint manager this tick.
use super::{Game, lan::LanHost};
use horizon_client::{
    items::ItemStack,
    lan::{
        hub::HostState,
        waypoints::{self, HIDING_HEADS, Icon, PLAYER_RANGE},
    },
};

/// `waypoint_transmit_range`: hidden by crouching, invisibility or a head worn.
fn transmit_range(crouching: bool, invisible: bool, head: Option<&ItemStack>) -> f64 {
    let hiding_head = head.is_some_and(|s| HIDING_HEADS.contains(&&*s.item));
    if crouching || invisible || hiding_head {
        0.0
    } else {
        PLAYER_RANGE
    }
}

impl Game<'_> {
    /// The player's team and its colour, as the locator bar shows it.
    fn waypoint_team(&self, name: &str) -> Option<(String, Option<i32>)> {
        let team = self.score.board.players_team(name)?;
        let color = team.color.as_deref().and_then(waypoints::team_rgb);
        Some((team.name.clone(), color))
    }

    pub(super) fn lan_update_waypoints(&mut self, host: &mut LanHost, state: Option<&HostState>) {
        let mut players = Vec::new();
        if let Some(h) = state {
            let icon = self
                .cmds
                .waypoint_icons
                .get(&super::commands::Target::Player)
                .map(|i| Icon {
                    style: i.style.clone(),
                    color: i.color,
                })
                .unwrap_or_default();
            players.push(waypoints::Player {
                uuid: h.uuid,
                conn: None,
                position: h.position,
                spectator: self.spectator(),
                transmit_range: transmit_range(
                    h.sneaking,
                    self.potions.effects.get("invisibility").is_some(),
                    h.equipment[5].as_ref(),
                ),
                receive_range: PLAYER_RANGE,
                first_tick: false,
                view_distance: host.hub.config.view_distance,
                team: self.waypoint_team(&h.name),
                icon,
            });
        }
        let mut respawned = Vec::new();
        for g in host.hub.guests.iter().filter(|g| g.placed) {
            // PlayerList.respawn makes a new ServerPlayer.
            if g.dead {
                host.waypoint_dead.insert(g.uuid);
            } else if host.waypoint_dead.remove(&g.uuid) {
                respawned.push(g.uuid);
            }
            // Player.updatePlayerPose: the shift key crouches unless flying.
            let crouching = g.sneaking && !g.flying && !g.fall_flying && !g.sleeping;
            players.push(waypoints::Player {
                uuid: g.uuid,
                conn: Some(g.conn),
                position: g.position,
                spectator: g.spectator(),
                transmit_range: transmit_range(
                    crouching,
                    g.effects.get("invisibility").is_some(),
                    g.armor[0].as_ref(),
                ),
                receive_range: PLAYER_RANGE,
                first_tick: g.ticks == 0,
                view_distance: g.view_distance,
                team: self.waypoint_team(&g.name),
                icon: Icon::default(),
            });
        }
        host.waypoint_dead
            .retain(|uuid| players.iter().any(|p| p.uuid == *uuid));
        let locator_bar = self.ui.rules.bool("locator_bar");
        for (conn, packet) in host.waypoints.tick(&players, &respawned, locator_bar) {
            if let Some(g) = host.hub.guest(conn) {
                g.send(&packet);
            }
        }
    }
}
