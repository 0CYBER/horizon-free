//! The locator bar of the players on a served world (lan/waypoints.rs): what
//! each player looks like to their level's waypoint manager this tick.
use super::{Game, commands::Target};
use horizon_client::{
    items::ItemStack,
    lan::waypoints::{self, HIDING_HEADS, PLAYER_RANGE},
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
    /// Each level's `ServerWaypointManager` ticked with the players in that
    /// level: a player who changes level leaves the old one's manager
    /// (`ServerLevel.removePlayerImmediately`: untracked for everyone there)
    /// and joins the new one's (`addDuringTeleport`).
    pub(super) fn lan_update_waypoints(&mut self) {
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        let mut players: Vec<(&str, waypoints::Player)> = Vec::new();
        let mut respawned = Vec::new();
        for g in host
            .hub
            .guests
            .iter()
            .filter(|g| g.placed && g.participates_online())
        {
            // PlayerList.respawn makes a new ServerPlayer.
            if g.dead {
                host.waypoint_dead.insert(g.uuid);
            } else if host.waypoint_dead.remove(&g.uuid) {
                respawned.push(g.uuid);
            }
            // Player.updatePlayerPose: the shift key crouches unless flying.
            let crouching = g.sneaking && !g.abilities.flying && !g.fall_flying && !g.sleeping;
            players.push((
                &g.dimension,
                waypoints::Player {
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
                    team: self.score.board.players_team(&g.name).map(|team| {
                        (
                            team.name.clone(),
                            team.color.as_deref().and_then(waypoints::team_rgb),
                        )
                    }),
                    // `/waypoint modify` and a mod's writes, kept by connection.
                    icon: self
                        .cmds
                        .waypoint_icons
                        .get(&Target::Guest(g.conn))
                        .cloned()
                        .unwrap_or_default(),
                },
            ));
        }
        host.waypoint_dead
            .retain(|uuid| players.iter().any(|(_, p)| p.uuid == *uuid));
        let locator_bar = self
            .ui
            .rules
            .bool_at(horizon_client::gamerules::vanilla::LOCATOR_BAR);
        let mut levels: Vec<String> = host.waypoints.keys().cloned().collect();
        for (level, _) in &players {
            if !levels.iter().any(|l| l == level) {
                levels.push((*level).to_owned());
            }
        }
        levels.sort();
        let mut out = Vec::new();
        for level in levels {
            let here: Vec<waypoints::Player> = players
                .iter()
                .filter(|(l, _)| *l == level)
                .map(|(_, p)| p.clone())
                .collect();
            let away: Vec<(u128, u32)> = players
                .iter()
                .filter(|(l, _)| *l != level)
                .filter_map(|(_, p)| Some((p.uuid, p.conn?)))
                .collect();
            let manager = host.waypoints.entry(level.clone()).or_default();
            out.extend(manager.tick_level(&here, &away, &respawned, locator_bar));
            // Nobody left in the level: nothing to remember there.
            if here.is_empty() {
                host.waypoints.remove(&level);
            }
        }
        for (conn, packet) in out {
            if let Some(g) = host.hub.guest(conn) {
                g.send(&packet);
            }
        }
    }
}
