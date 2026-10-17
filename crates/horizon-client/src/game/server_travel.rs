//! A server's player between dimensions.
//!
//! The server holds one level at a time, as the in-frame game does. While a
//! single player is online (single player on the server thread, or a world
//! with one player in it) that level follows the player through Nether and
//! End portals, and back to the Overworld when the player respawns. The swap
//! is the in-frame path's (`switch_level`, the portal plans, the arrival
//! search); the player hears it as `ServerPlayer.teleport` into another
//! level: a Respawn packet keeping all data, then a teleport to the exit once
//! the destination has loaded. With several players online the portals do
//! not take anyone yet: that needs the server to hold every level.

use super::nether_portal::PLAYER_DELAY;
use super::{Dim, Game, lan::LanHost};
use horizon_client::lan::hub::TickCtx;
use horizon_core::{geometry::Vec3, position::BlockPos};

/// A trip under way: the level has changed and its chunks are loading
/// around `target`.
pub(super) struct Trip {
    conn: u32,
    target: Vec3,
    /// Through an End portal (or out of the End by respawning).
    end: bool,
}

/// The lone player's box: Player's standing dimensions.
const WIDTH: f64 = 0.6;
const HEIGHT: f64 = 1.8;

impl Game<'_> {
    /// Whether `conn` is between two levels: the server leaves its body
    /// alone until it arrives.
    pub(super) fn on_server_trip(&self, conn: u32) -> bool {
        self.server_trip.as_ref().is_some_and(|t| t.conn == conn)
    }

    /// Per server tick, after the guests' own: `Entity.handlePortal` for the
    /// lone player (Nether portals count up their transition time, End
    /// portals take it at once), or the arrival of the trip under way.
    pub(super) fn tick_server_travel(&mut self, host: &mut LanHost) {
        if self.options.dedicated.is_none() {
            return;
        }
        if self.server_trip.is_some() {
            self.arrive_server_trip(host);
            return;
        }
        let [g] = host.hub.guests.as_slice() else {
            return;
        };
        if !g.placed || !g.loaded || g.dead || g.spectator() {
            return;
        }
        let (conn, at, invulnerable, sleeping) = (
            g.conn,
            Vec3::new(g.position[0], g.position[1], g.position[2]),
            g.creative(),
            g.sleeping,
        );
        // Entity.checkInsideBlocks: the Nether portal blocks the box is in.
        if let Some(pos) = self.nether_portal_inside(at) {
            self.portals.player.set_inside(pos, PLAYER_DELAY);
        }
        let (end_portal, _) = self.inside_end_portals(at, WIDTH, HEIGHT);
        if end_portal {
            if self.portals.player.cooldown == 0 {
                let target = if self.dim == Dim::End {
                    Dim::Overworld
                } else {
                    Dim::End
                };
                self.start_server_trip(host, conn, target, true);
            }
            // Entity.setAsInsidePortal while cooling down restarts the cooldown.
            self.portals.player.cooldown = 10;
            return;
        }
        let rule = if invulnerable {
            "players_nether_portal_creative_delay"
        } else {
            "players_nether_portal_default_delay"
        };
        let transition = self.ui.rules.int(rule).max(0);
        let Some(entry) = self
            .portals
            .player
            .handle(!sleeping, transition, PLAYER_DELAY)
        else {
            return;
        };
        let plan = self.portal_plan(entry, at, WIDTH as f32, HEIGHT as f32, true);
        if !self.may_enter(plan.dim) {
            return;
        }
        self.portals.travel = Some(plan);
        self.start_server_trip(host, conn, plan.dim, false);
    }

    /// A Nether portal block the lone player's box is in.
    fn nether_portal_inside(&self, at: Vec3) -> Option<BlockPos> {
        let w = WIDTH / 2.0 - 1.0e-5;
        let lo = [at.x - w, at.y + 1.0e-5, at.z - w];
        let hi = [at.x + w, at.y + HEIGHT - 1.0e-5, at.z + w];
        for x in lo[0].floor() as i32..=hi[0].floor() as i32 {
            for y in lo[1].floor() as i32..=hi[1].floor() as i32 {
                for z in lo[2].floor() as i32..=hi[2].floor() as i32 {
                    if &*self.block_name([x, y, z]) == "minecraft:nether_portal" {
                        return Some(BlockPos::new(x, y, z));
                    }
                }
            }
        }
        None
    }

    /// The level swap, and the player told it is in the new level at the
    /// approximate exit while it loads.
    fn start_server_trip(&mut self, host: &mut LanHost, conn: u32, target: Dim, end: bool) {
        let Some(i) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return;
        };
        let g = &host.hub.guests[i];
        let from = Vec3::new(g.position[0], g.position[1], g.position[2]);
        let yaw = g.yaw;
        let source = self.dim;
        self.end_portal_travel = end;
        let switched = self.switch_level(target, from);
        self.end_portal_travel = false;
        let destination = match switched {
            Ok(d) => d,
            Err(e) => {
                log::error!("could not open {}: {e}", target.id());
                return;
            }
        };
        let destination = match self.portals.travel {
            Some(plan) if !end => self.arrival_centre(&plan),
            _ => destination,
        };
        host.hub.set_dimension(target.id());
        let at = [destination.x, destination.y, destination.z];
        let ctx = self.server_ctx();
        host.hub.change_level(i, at, yaw, &ctx);
        self.as_guest(host, conn, |g| g.trigger_dimension(source, target));
        self.server_trip = Some(Trip {
            conn,
            target: destination,
            end,
        });
    }

    /// Once the destination has loaded around the trip's target: the exit
    /// (the Nether portal found or built, the End platform, the Overworld's
    /// spawn column) and the teleport there.
    fn arrive_server_trip(&mut self, host: &mut LanHost) {
        let Some(trip) = self.server_trip.as_ref() else {
            return;
        };
        let Some(i) = host.hub.guests.iter().position(|g| g.conn == trip.conn) else {
            // They left on the way.
            self.server_trip = None;
            self.portals.travel = None;
            return;
        };
        let (cx, cz) = (
            (trip.target.x.floor() as i32) >> 4,
            (trip.target.z.floor() as i32) >> 4,
        );
        if !(-2..=2).all(|dz| (-2..=2).all(|dx| self.streaming.is_lit(cx + dx, cz + dz))) {
            return;
        }
        let Some(trip) = self.server_trip.take() else {
            return;
        };
        self.resolve_arrivals();
        let (position, yaw) = if trip.end {
            match self.end_arrival(trip.target) {
                Ok((at, yaw)) => (at, yaw.unwrap_or(host.hub.guests[i].yaw)),
                Err(e) => {
                    log::error!("End arrival: {e}");
                    (trip.target, host.hub.guests[i].yaw)
                }
            }
        } else {
            let plan = self.portals.travel.take();
            let exit = plan.as_ref().and_then(|plan| self.nether_exit_portal(plan));
            match (plan, exit) {
                (Some(plan), Some(exit)) => {
                    let (at, rotation) = self.arrival(&plan, exit);
                    (at, host.hub.guests[i].yaw + rotation)
                }
                _ => (trip.target, host.hub.guests[i].yaw),
            }
        };
        // Player.getDimensionChangingDelay.
        self.portals.player.cooldown = 10;
        let g = &mut host.hub.guests[i];
        g.yaw = yaw;
        // A new body at the exit: no fall distance or motion carried over.
        g.motion = None;
        host.hub
            .teleport(i, Some([position.x, position.y, position.z]));
    }

    /// `PlayerList.respawn` away from the Overworld: the lone player's level
    /// goes back to the Overworld first, where the world spawn is.
    pub(super) fn respawn_level(&mut self, host: &mut LanHost) {
        if self.options.dedicated.is_none() || self.dim == Dim::Overworld {
            return;
        }
        if host.hub.guests.len() != 1 {
            return;
        }
        self.server_trip = None;
        self.portals.travel = None;
        self.end_portal_travel = true;
        let from = self.spawn.unwrap_or(Vec3::ZERO);
        let switched = self.switch_level(Dim::Overworld, from);
        self.end_portal_travel = false;
        if let Err(e) = switched {
            log::error!("could not open the Overworld: {e}");
            return;
        }
        host.hub.set_dimension(Dim::Overworld.id());
    }

    /// The hub's view of the server for packets sent outside its own tick.
    pub(super) fn server_ctx(&mut self) -> TickCtx<'_> {
        let spawn = self.spawn.map_or([0.5, 80.0, 0.5], |s| [s.x, s.y, s.z]);
        TickCtx {
            world: &mut self.world,
            host: None,
            entities: &[],
            next_entity: &mut self.next_entity,
            spawn,
            difficulty: i32::from(self.ui.difficulty),
            difficulty_locked: self.level.difficulty_locked,
            clock_rate: if self.ui.rules.bool("advance_time") {
                self.ui.clock.rate
            } else {
                0.0
            },
            tick_rate: self.clock.rate(),
            tick_frozen: self.clock.frozen(),
            collision: Some(self.collision),
        }
    }
}
