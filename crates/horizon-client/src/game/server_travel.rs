//! A server's players between dimensions.
//!
//! A server holds a level per dimension (game/levels.rs). A player who steps
//! into a Nether or End portal moves to the destination's level, which opens
//! if nobody was there: the swap is the in-frame path's portal plans and
//! arrival search, and the player hears it as `ServerPlayer.teleport` into
//! another level: a Respawn packet keeping all data, then a teleport to the
//! exit once the destination has loaded around them.

use super::end::GatewayJob;
use super::nether_portal::{PLAYER_DELAY, Plan};
use super::respawn_anchor::RespawnConfig;
use super::{Dim, Game, lan::LanHost};
use horizon_client::{lan::hub::TickCtx, text::Text};
use horizon_core::{geometry::Vec3, position::BlockPos};

/// A trip under way: the player is in the destination's level, whose
/// chunks are loading around `target`.
pub(super) struct Trip {
    conn: u32,
    dim: Dim,
    target: Vec3,
    /// Through an End portal.
    end: bool,
    /// The Nether portal plan, whose exit is found or built on arrival.
    plan: Option<Plan>,
    /// Through an End gateway, within the level: its exit, found or built
    /// on arrival.
    gateway: Option<GatewayJob>,
    /// A respawn at this point outside the Overworld, resolved through its
    /// block on arrival.
    respawn: Option<RespawnConfig>,
    /// Sent by a command: the player is already where it goes.
    placed: bool,
}

/// `TeleportCommand` into another level for a server's player: where, and
/// the rotation it sets (none keeps the player's).
pub(super) struct LevelTeleport {
    pub conn: u32,
    pub dim: Dim,
    pub at: [f64; 3],
    pub rotation: Option<[f32; 2]>,
}

/// A player's box: Player's standing dimensions.
const WIDTH: f64 = 0.6;
const HEIGHT: f64 = 1.8;

impl Trip {
    pub(super) fn is(&self, conn: u32) -> bool {
        self.conn == conn
    }
}

impl Game<'_> {
    /// Whether `conn` is between two levels: the server leaves its body
    /// alone until it arrives.
    pub(super) fn on_server_trip(&self, conn: u32) -> bool {
        self.server_trips.iter().any(|t| t.conn == conn)
    }

    /// Per server tick in each level, after the guests' own: the arrivals of
    /// the trips into this level, then `Entity.handlePortal` for each player
    /// in it (Nether portals count up their transition time, End portals
    /// take them at once).
    pub(super) fn tick_server_travel(&mut self, host: &mut LanHost) {
        if self.options.dedicated.is_none() {
            return;
        }
        self.arrive_server_trips(host);
        let conns: Vec<u32> = host
            .hub
            .guests
            .iter()
            .filter(|g| host.hub.here(g) && g.loaded && !g.dead && !g.spectator())
            .map(|g| g.conn)
            .collect();
        for conn in conns {
            if self.on_server_trip(conn) {
                continue;
            }
            // `Entity.portalProcess` and `portalCooldown` are the player's own.
            let mut portal = host.play.entry(conn).or_default().portal;
            std::mem::swap(&mut self.portals.player, &mut portal);
            self.handle_server_portal(host, conn);
            std::mem::swap(&mut self.portals.player, &mut portal);
            host.play.entry(conn).or_default().portal = portal;
        }
    }

    /// `Entity.handlePortal` for one player, whose portal state is in place.
    fn handle_server_portal(&mut self, host: &mut LanHost, conn: u32) {
        let Some(g) = host.hub.guest(conn) else {
            return;
        };
        let (at, invulnerable, sleeping) = (
            Vec3::new(g.position[0], g.position[1], g.position[2]),
            g.creative(),
            g.sleeping,
        );
        // Entity.checkInsideBlocks: the Nether portal blocks the box is in.
        if let Some(pos) = self.nether_portal_inside(at) {
            self.portals.player.set_inside(pos, PLAYER_DELAY);
        }
        let (end_portal, gateway) = self.inside_end_portals(at, WIDTH, HEIGHT);
        if end_portal {
            if self.portals.player.cooldown == 0 {
                let target = if self.dim == Dim::End {
                    Dim::Overworld
                } else {
                    Dim::End
                };
                self.start_server_trip(host, conn, target, None);
            }
            // Entity.setAsInsidePortal while cooling down restarts the cooldown.
            self.portals.player.cooldown = 10;
            return;
        }
        // EndGatewayBlock.entityInside: a gateway that is not cooling down
        // takes the player at once (no transition time) and cools down
        // (triggerCooldown), even for a player still on its own cooldown.
        if let Some(g) = gateway
            && self.gateway_enter(g)
        {
            if self.portals.player.cooldown == 0 {
                self.start_gateway_trip(host, conn, g);
            }
            // setPortalCooldown: after the trip, or again while cooling down.
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
        self.start_server_trip(host, conn, plan.dim, Some(plan));
    }

    /// A Nether portal block a player's box is in.
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

    /// The player moves to `target`'s level (opened if nobody was there)
    /// and is told it is there, at the approximate exit, while it loads. The
    /// level whose tick is running is put back in place afterwards.
    fn start_server_trip(
        &mut self,
        host: &mut LanHost,
        conn: u32,
        target: Dim,
        plan: Option<Plan>,
    ) {
        let Some(i) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return;
        };
        let g = &host.hub.guests[i];
        let from = Vec3::new(g.position[0], g.position[1], g.position[2]);
        let yaw = g.yaw;
        let source = self.dim;
        let end = plan.is_none();
        // Coordinates scale by 8 between the Overworld and the Nether; an
        // End portal has fixed destinations.
        let scale = match (source, target) {
            (Dim::Overworld, Dim::Nether) | (Dim::End, Dim::Nether) => 1.0 / 8.0,
            (Dim::Nether, Dim::Overworld) => 8.0,
            _ => 1.0,
        };
        self.end_portal_travel = end;
        // Leaving the End: the player's own respawn point.
        let own = if end {
            self.as_guest(host, conn, |g| g.end_destination(target))
                .flatten()
        } else {
            None
        };
        let scaled = own
            .or_else(|| self.end_destination(target))
            .unwrap_or(Vec3::new(from.x * scale, from.y, from.z * scale));
        self.end_portal_travel = false;
        if let Err(e) = self.enter_level(Some(host), target) {
            log::error!("could not open {}: {e}", target.id());
            return;
        }
        let destination = match plan {
            Some(plan) => self.arrival_centre(&plan),
            None => scaled,
        };
        let at = [destination.x, destination.y, destination.z];
        let ctx = self.server_ctx();
        // TeleportTransition.PLAY_PORTAL_SOUND.
        host.hub.change_level(i, at, yaw, None, true, &ctx);
        self.as_guest(host, conn, |g| g.trigger_dimension(source, target));
        self.server_trips.push(Trip {
            conn,
            dim: target,
            target: destination,
            end,
            plan,
            gateway: None,
            respawn: None,
            placed: false,
        });
        if let Err(e) = self.enter_level(Some(host), source) {
            log::error!("could not reopen {}: {e}", source.id());
        }
    }

    /// A respawned player waits at its point outside the Overworld for the
    /// level to load around it.
    pub(super) fn respawn_trip(&mut self, conn: u32, config: RespawnConfig) {
        let [x, y, z] = config.pos;
        self.server_trips.push(Trip {
            conn,
            dim: config.dim,
            target: Vec3::new(f64::from(x) + 0.5, f64::from(y), f64::from(z) + 0.5),
            end: false,
            plan: None,
            gateway: None,
            respawn: Some(config),
            placed: false,
        });
    }

    /// `findRespawnAndUseSpawnBlock` for the player in place, at its point
    /// in the level in place: where it stands up, facing the anchor (with
    /// the deplete sound) or the bed; else the point is forgotten
    /// (`block.minecraft.spawn.not_valid`).
    pub(super) fn resolve_server_respawn(&mut self, config: RespawnConfig) -> Option<(Vec3, f32)> {
        let [x, y, z] = config.pos;
        let anchor = self.is_anchor(self.world.block(x, y, z));
        match self.find_respawn_and_use(config, true) {
            Ok(Some((at, yaw))) => {
                if !config.forced && anchor {
                    let [x, y, z] = config.pos.map(f64::from);
                    self.audio.play(
                        "minecraft:block.respawn_anchor.deplete",
                        Some([x, y, z]),
                        1.0,
                        1.0,
                    );
                }
                Some((at, yaw))
            }
            Ok(None) | Err(_) => {
                self.respawn_config = None;
                self.system_message(Text::translate("block.minecraft.spawn.not_valid", &[]));
                None
            }
        }
    }

    /// TeleportTransition.missingRespawnBlock: the world spawn, in the
    /// Overworld.
    fn respawn_at_world_spawn(&mut self, host: &mut LanHost, conn: u32) {
        let at = self.level.world_spawn.map_or(
            self.spawn.map_or([0.5, 80.0, 0.5], |s| [s.x, s.y, s.z]),
            |[x, y, z]| [f64::from(x) + 0.5, f64::from(y), f64::from(z) + 0.5],
        );
        self.level_teleports.push(LevelTeleport {
            conn,
            dim: Dim::Overworld,
            at,
            rotation: None,
        });
        self.run_level_teleports(host);
    }

    /// `TheEndGatewayBlockEntity.getPortalPosition` and the teleport within
    /// the End (keeping the player's motion and look): straight to an exact
    /// exit, else once the destination has loaded around the player, who
    /// waits there (its exit is found among those blocks, or built).
    fn start_gateway_trip(&mut self, host: &mut LanHost, conn: u32, gateway: [i32; 3]) {
        let Some(i) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return;
        };
        let (job, destination) = self.gateway_job(gateway);
        let at = [destination.x, destination.y, destination.z];
        // Relative.union(DELTA, ROTATION).
        host.hub.teleport_keeping_motion(i, at);
        if let GatewayJob::Known { exact: true, .. } = job {
            return;
        }
        self.server_trips.push(Trip {
            conn,
            dim: self.dim,
            target: destination,
            end: false,
            plan: None,
            gateway: Some(job),
            respawn: None,
            placed: false,
        });
    }

    /// The trips into the level in place whose destination has loaded
    /// around their target: the exit (the Nether portal found or built, the
    /// End platform, the Overworld's spawn column) and the teleport there.
    fn arrive_server_trips(&mut self, host: &mut LanHost) {
        let mut k = 0;
        while k < self.server_trips.len() {
            let trip = &self.server_trips[k];
            let Some(i) = host.hub.guests.iter().position(|g| g.conn == trip.conn) else {
                // They left on the way.
                self.server_trips.remove(k);
                continue;
            };
            let (cx, cz) = (
                (trip.target.x.floor() as i32) >> 4,
                (trip.target.z.floor() as i32) >> 4,
            );
            if trip.dim != self.dim
                || !(-2..=2).all(|dz| (-2..=2).all(|dx| self.streaming.is_lit(cx + dx, cz + dz)))
            {
                k += 1;
                continue;
            }
            let trip = self.server_trips.remove(k);
            if trip.placed {
                continue;
            }
            self.resolve_arrivals();
            let yaw = host.hub.guests[i].yaw;
            let (position, yaw) = if let Some(config) = trip.respawn {
                match self.as_guest(host, trip.conn, |g| g.resolve_server_respawn(config)) {
                    Some(Some(found)) => found,
                    _ => {
                        self.respawn_at_world_spawn(host, trip.conn);
                        continue;
                    }
                }
            } else if let Some(job) = trip.gateway {
                match self.resolve_gateway(job, trip.target) {
                    Ok(at) => (at, yaw),
                    Err(e) => {
                        log::error!("End gateway exit: {e}");
                        (trip.target, yaw)
                    }
                }
            } else if trip.end {
                let arrival = match self.as_guest(host, trip.conn, |g| g.end_arrival(trip.target)) {
                    Some(arrival) => arrival,
                    None => self.end_arrival(trip.target),
                };
                match arrival {
                    Ok((at, turned)) => (at, turned.unwrap_or(yaw)),
                    Err(e) => {
                        log::error!("End arrival: {e}");
                        (trip.target, yaw)
                    }
                }
            } else {
                let exit = trip
                    .plan
                    .as_ref()
                    .and_then(|plan| self.nether_exit_portal(plan));
                match (trip.plan, exit) {
                    (Some(plan), Some(exit)) => {
                        let (at, rotation) = self.arrival(&plan, exit);
                        (at, yaw + rotation)
                    }
                    _ => (trip.target, yaw),
                }
            };
            // Player.getDimensionChangingDelay.
            host.play.entry(trip.conn).or_default().portal.cooldown = 10;
            let g = &mut host.hub.guests[i];
            g.yaw = yaw;
            // A new body at the exit: no fall distance or motion carried over.
            g.motion = None;
            host.hub
                .teleport(i, Some([position.x, position.y, position.z]));
        }
    }

    /// The players the command just run sends to another level
    /// (`ServerPlayer.teleport` from TeleportCommand): a Respawn packet
    /// keeping all data and the teleport, with no portal sound. The server
    /// leaves the body alone until the level has loaded around it.
    pub(super) fn run_level_teleports(&mut self, host: &mut LanHost) {
        for t in std::mem::take(&mut self.level_teleports) {
            let Some(i) = host.hub.guests.iter().position(|g| g.conn == t.conn) else {
                continue;
            };
            let source = self.dim;
            if let Err(e) = self.enter_level(Some(host), t.dim) {
                log::error!("could not open {}: {e}", t.dim.id());
                continue;
            }
            let g = &mut host.hub.guests[i];
            let [yaw, pitch] = t.rotation.unwrap_or([g.yaw, g.pitch]);
            // A new body there: no fall distance or motion carried over.
            g.motion = None;
            let ctx = self.server_ctx();
            host.hub
                .change_level(i, t.at, yaw, Some(pitch), false, &ctx);
            self.as_guest(host, t.conn, |g| g.trigger_dimension(source, t.dim));
            self.server_trips.retain(|trip| !trip.is(t.conn));
            self.server_trips.push(Trip {
                conn: t.conn,
                dim: t.dim,
                target: Vec3::new(t.at[0], t.at[1], t.at[2]),
                end: false,
                plan: None,
                gateway: None,
                respawn: None,
                placed: true,
            });
            if let Err(e) = self.enter_level(Some(host), source) {
                log::error!("could not reopen {}: {e}", source.id());
            }
        }
    }

    /// The hub's view of the server for packets sent outside its own tick.
    pub(super) fn server_ctx(&mut self) -> TickCtx<'_> {
        let spawn = self.spawn.map_or([0.5, 80.0, 0.5], |s| [s.x, s.y, s.z]);
        let clock_rate = self.network_clock_rate();
        TickCtx {
            world: &mut self.world,
            host: None,
            entities: &[],
            next_entity: &mut self.next_entity,
            spawn,
            difficulty: i32::from(self.ui.difficulty),
            difficulty_locked: self.level.difficulty_locked,
            clock_rate,
            tick_rate: self.clock.rate(),
            tick_frozen: self.clock.frozen(),
            tick_steps: self.clock.step_state(),
            collision: Some(self.collision),
            border: &self.border,
        }
    }
}
