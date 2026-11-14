//! A server's players between dimensions.
//!
//! A server holds a level per dimension (game/world/levels.rs). A player who steps
//! into a Nether or End portal moves to the destination's level, which opens
//! if nobody was there: the portal plans and arrival search pick the exit,
//! and the player hears it as `ServerPlayer.teleport` into
//! another level: a Respawn packet keeping all data, then a teleport to the
//! exit once the destination has loaded around them.

use super::end::GatewayJob;
use super::nether_portal::{PLAYER_DELAY, Plan};
use super::persist::OffLevel;
use super::respawn_anchor::RespawnConfig;
use super::respawn_position::Respawn;
use super::{Dim, Game, Mob, boats, lan::LanHost, minecarts};
use horizon_client::{lan::hub::TickCtx, world_spawn::PlayerSpawnFinder};
use horizon_core::{geometry::Vec3, position::BlockPos};
use horizon_protocol::packets::play::{self, Clientbound as C};

/// Why an entity changes level (`entity_travel_to_dimension`'s `cause`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TravelCause {
    NetherPortal,
    EndPortal,
    EndGateway,
    /// A teleport into another level (`/tp`, a spectator's teleport).
    Teleport,
}

impl TravelCause {
    /// The name a mod is told.
    pub(super) fn id(self) -> &'static str {
        match self {
            Self::NetherPortal => "nether_portal",
            Self::EndPortal => "end_portal",
            Self::EndGateway => "end_gateway",
            Self::Teleport => "teleport",
        }
    }

    /// The portal a plan goes through.
    pub(super) fn of_plan(plan: &Plan) -> Self {
        if plan.end {
            Self::EndPortal
        } else {
            Self::NetherPortal
        }
    }
}

/// A trip under way: the player is in the destination's level, whose
/// chunks are loading around `target`.
pub(super) struct Trip {
    conn: u32,
    dim: Dim,
    /// The level the player came from: the vehicle it rides waits there
    /// until the exit is found.
    source: Dim,
    target: Vec3,
    /// Through an End portal.
    end: bool,
    /// The Nether portal plan, whose exit is found or built on arrival.
    plan: Option<Plan>,
    /// The vehicle the player rides, which travels with it
    /// (`Entity.teleportCrossDimension` carries the passengers).
    vehicle: Option<u32>,
    /// Through an End gateway, within the level: its exit, found or built
    /// on arrival.
    gateway: Option<GatewayJob>,
    /// A respawn at this point outside the Overworld, resolved through its
    /// block on arrival.
    respawn: Option<RespawnConfig>,
    /// A respawn at the world spawn, moved onto a free column within the
    /// respawn radius once the chunks around it have loaded.
    world_spawn: bool,
    /// The respawn point whose bed or anchor was gone, for a world-spawn
    /// respawn that replaces it (`TeleportTransition.missingRespawnBlock`).
    missing: Option<RespawnConfig>,
    /// The search for that column while a candidate's chunk loads.
    finder: Option<PlayerSpawnFinder>,
    /// Sent by a command: the player is already where it goes.
    placed: bool,
    /// A `tp` inside the level to chunks not loaded yet: the client's moves are ignored (it would fall
    /// through the missing terrain) until the player is placed again where the chunks have loaded.
    hold: bool,
    /// The level the player left for another one: the mods are told they
    /// changed dimension when the trip ends (`tell_mods_changed_dimension`).
    pub(super) left: Option<Dim>,
}

/// A vehicle a player rides: an animal, a boat or a minecart. It travels
/// with the player through a portal (`Entity.teleportCrossDimension` carries
/// the passengers, and they ride the vehicle that is recreated in the other
/// dimension).
trait Rideable {
    fn id(&self) -> u32;
    /// The vehicle stands where its rider arrives, with the motion it crossed
    /// with (none for the kinds that keep none).
    fn arrive(&mut self, at: Vec3, yaw: f32, motion: Vec3);
}

impl Rideable for Mob {
    fn id(&self) -> u32 {
        self.id
    }
    fn arrive(&mut self, at: Vec3, yaw: f32, _motion: Vec3) {
        self.position = at;
        self.previous = at;
        self.velocity = Vec3::ZERO;
        self.yaw = yaw;
        self.previous_yaw = yaw;
        self.fall_distance = 0.0;
    }
}

impl Rideable for boats::Boat {
    fn id(&self) -> u32 {
        self.id
    }
    fn arrive(&mut self, at: Vec3, yaw: f32, _motion: Vec3) {
        self.position = at;
        self.previous = at;
        self.velocity = Vec3::ZERO;
        self.yaw = yaw;
        self.previous_yaw = yaw;
        self.delta_rotation = 0.0;
    }
}

impl Rideable for minecarts::Minecart {
    fn id(&self) -> u32 {
        self.id
    }
    fn arrive(&mut self, at: Vec3, yaw: f32, motion: Vec3) {
        self.position = at;
        self.previous = at;
        // `PositionMoveRotation.of(this)` takes the cart's `getKnownMovement`
        // (`OldMinecartBehavior` clamps it to 0.4 either way along x and z):
        // it rolls on at that speed instead of arriving at rest.
        self.velocity = Vec3::new(
            motion.x.clamp(-0.4, 0.4),
            motion.y,
            motion.z.clamp(-0.4, 0.4),
        );
        self.yaw = yaw;
        self.previous_yaw = yaw;
    }
}

/// The hub's view of the server for packets sent outside its own tick.
macro_rules! server_ctx {
    ($s:ident) => {{
        let spawn = $s.world_spawn_position();
        let clock_rate = $s.network_clock_rate();
        TickCtx {
            world: &mut $s.world,
            entities: &[],
            next_entity: &mut $s.next_entity,
            spawn,
            spawn_angle: $s.level.world_spawn_angle,
            sea_level: $s.end.locator().map_or(63, |terrain| terrain.sea_level()),
            difficulty: i32::from($s.ui.difficulty),
            difficulty_locked: $s.level.difficulty_locked,
            clock_rate,
            tick_rate: $s.clock.rate(),
            tick_frozen: $s.clock.frozen(),
            tick_steps: $s.clock.step_state(),
            collision: Some($s.collision),
            border: &$s.border,
            streaming: Some(&$s.streaming),
        }
    }};
}

/// The vehicle `id` leaves the level it is in, at the point its rider
/// arrives. It keeps its entity id, so the ride is never broken.
/// A guest leaving for another level leaves its fishing hook behind in the
/// old one, where vanilla's hook stays until its distance check ends it. A
/// guest's hook lives on the guest here, so it ends now: it must not act on
/// the blocks of the level the guest arrives in.
fn drop_hook(host: &mut LanHost, conn: u32) {
    if let Some(play) = host.play.get_mut(&conn) {
        play.hook.discard();
    }
}

fn take_vehicle<
    T: Rideable
        + horizon_client::join_seq::Joined
        + horizon_mod_runtime::attachments::HasAttachmentLifetime,
    const DIRECT: bool,
>(
    list: &mut horizon_client::join_seq::JoinList<T, DIRECT>,
    id: u32,
    at: Vec3,
    yaw: f32,
    motion: Vec3,
) -> Option<T> {
    let i = list.iter().position(|v| v.id() == id)?;
    let mut v = list.remove_for_transfer(i);
    v.arrive(at, yaw, motion);
    v.set_join_seq(horizon_client::join_seq::JoinSeq::UNJOINED);
    Some(v)
}

/// `TeleportCommand` into another level for a server's player: where, and
/// the rotation it sets (none keeps the player's).
pub(super) struct LevelTeleport {
    pub conn: u32,
    pub dim: Dim,
    pub at: [f64; 3],
    pub rotation: Option<[f32; 2]>,
    /// `TeleportTransition.asPassenger`: the vehicle already moved with its tree.
    pub vehicle: Option<u32>,
}

/// A teleport a command made inside a level: told to the mods once it has
/// reached the hub (`native_player_teleported`).
pub(super) struct TeleportNote {
    pub conn: u32,
    pub uuid: u128,
    pub name: String,
    pub from: [f64; 3],
    pub to: [f64; 3],
    pub dimension: String,
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
    pub(super) fn gateway_source_chunks(&self) -> Vec<(i32, i32)> {
        self.server_trips
            .iter()
            .filter(|t| t.dim == self.dim)
            .filter_map(|t| t.gateway.and_then(GatewayJob::source_chunk))
            .collect()
    }
    /// A server player's `tp` inside their level, where the chunks round `target` have not loaded: vanilla loads
    /// them before the player arrives, so the player is held as a trip holds them (their body is left alone, their
    /// client's moves ignored) until the chunks are lit, then placed there again (#15727).
    pub(super) fn hold_for_chunks(&mut self, conn: u32, target: Vec3) {
        // The arrival the player was waiting for elsewhere is abandoned.
        self.server_trips.retain(|t| !t.is(conn));
        let (cx, cz) = (
            (target.x.floor() as i32) >> 4,
            (target.z.floor() as i32) >> 4,
        );
        if (-2..=2).all(|dz| (-2..=2).all(|dx| self.streaming.is_lit(cx + dx, cz + dz))) {
            return;
        }
        self.server_trips.push(Trip {
            conn,
            dim: self.dim,
            source: self.dim,
            target,
            end: false,
            plan: None,
            vehicle: None,
            gateway: None,
            respawn: None,
            world_spawn: false,
            finder: None,
            placed: false,
            hold: true,
            missing: None,
            left: None,
        });
    }
    /// Before the arrivals: a hold follows the player where a mod sent them (`native_player_teleport` may cancel
    /// a `tp` or send the player elsewhere: the guest stands where the hub put them, not at the hold's target),
    /// and a hold that ended without its arrival (replaced by a `tp` to lit land, a trip to another level, a
    /// respawn, a death) lets their client's moves be heard again. A portal's trip still waiting for the player
    /// lands them itself.
    fn release_held_arrivals(&mut self) {
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let level = self.dim.id();
        let moved: Vec<(u32, Vec3)> = self
            .server_trips
            .iter()
            .filter(|t| t.hold && t.dim == self.dim)
            .filter_map(|t| {
                let g = host.hub.guests.iter().find(|g| g.conn == t.conn)?;
                let at = Vec3::new(g.position[0], g.position[1], g.position[2]);
                (g.dimension == level && at != t.target).then_some((t.conn, at))
            })
            .collect();
        for (conn, at) in moved {
            self.hold_for_chunks(conn, at);
        }
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        for i in 0..host.hub.guests.len() {
            let g = &host.hub.guests[i];
            if !g.tp_held {
                continue;
            }
            let conn = g.conn;
            if self.server_trips.iter().any(|t| t.is(conn) && t.hold) {
                continue;
            }
            let landing = self
                .server_trips
                .iter()
                .any(|t| t.is(conn) && (t.end || t.plan.is_some()));
            host.hub.guests.update(i, |g| {
                g.tp_held = false;
                if !landing {
                    g.landing = false;
                }
            });
        }
    }
    /// Whether `conn` is between two levels: the server leaves its body
    /// alone until it arrives.
    pub(super) fn on_server_trip(&self, conn: u32) -> bool {
        self.server_trips.iter().any(|t| t.conn == conn)
    }

    /// Whether a vehicle waits in `dim` for its rider to arrive elsewhere:
    /// that level stays open until it has moved.
    pub(super) fn vehicle_waits_in(&self, dim: Dim) -> bool {
        self.server_trips
            .iter()
            .any(|t| t.vehicle.is_some() && t.source == dim)
    }

    /// Per server tick in each level, after the guests' own: the arrivals of
    /// the trips into this level, then `Entity.handlePortal` for each player
    /// in it (Nether portals count up their transition time, End portals
    /// take them at once).
    pub(super) fn tick_server_travel(&mut self) {
        if self.lan.is_none() {
            return;
        }
        // Outside a server only the respawns at the world spawn wait for chunks.
        self.arrive_server_trips();
        if self.options.dedicated.is_none() {
            return;
        }
        self.carry_riders_through_portals();
        let Some(host) = self.lan.as_ref() else {
            return;
        };
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
            let Some(host) = self.lan.as_mut() else {
                return;
            };
            let mut portal = host.play.entry(conn).or_default().portal;
            std::mem::swap(&mut self.portals.player, &mut portal);
            self.handle_server_portal(conn);
            std::mem::swap(&mut self.portals.player, &mut portal);
            if let Some(host) = self.lan.as_mut() {
                host.play.entry(conn).or_default().portal = portal;
            }
        }
    }

    /// `ServerPlayerGameMode.useItemOn` for a spectator: a `Portal` block
    /// (Nether portal, End portal, End gateway) takes the player to its
    /// destination (`getPortalDestination`, `Entity.teleportToPortalDestination`)
    /// after the camera goes back to the player, or says `spectator.cannot_teleport`
    /// when it has none. Returns whether the block was a portal (`CONSUME`).
    pub(super) fn spectator_use_portal(&mut self, conn: u32, gi: usize, pos: [i32; 3]) -> bool {
        let name = self.block_name(pos);
        let name = name.trim_start_matches("minecraft:");
        if !matches!(name, "nether_portal" | "end_portal" | "end_gateway") {
            return false;
        }
        if self.on_server_trip(conn) {
            return true;
        }
        let Some(g) = self.lan.as_ref().map(|host| &host.hub.guests[gi]) else {
            return false;
        };
        let at = Vec3::new(g.position[0], g.position[1], g.position[2]);
        let seen_credits = g.seen_credits;
        // EndGatewayBlock.getPortalDestination: none without its block entity.
        let gateway_ok = name != "end_gateway"
            || self
                .world
                .block_entities
                .get(&(pos[0], pos[1], pos[2]))
                .is_some_and(|e| {
                    matches!(e, horizon_client::containers::BlockEntity::EndGateway(_))
                });
        if !gateway_ok {
            self.as_guest(conn, |g| {
                g.send_overlay_message(
                    horizon_client::text::Text::translate("spectator.cannot_teleport", &[])
                        .with_color(horizon_client::text::RED),
                );
            });
            return true;
        }
        if self.cmds.guest_cameras.contains_key(&conn) {
            self.guest_set_camera(gi, None);
        }
        match name {
            "nether_portal" => {
                let plan = self.portal_plan(
                    BlockPos::new(pos[0], pos[1], pos[2]),
                    at,
                    WIDTH as f32,
                    HEIGHT as f32,
                    true,
                );
                // Entity.teleportToPortalDestination: isAllowedToEnterPortal.
                if self.may_enter(plan.dim) {
                    // NetherPortalBlock.getExitPortal: a spectator never
                    // builds an exit portal, it gets no transition without one.
                    if !self.spectator_exit_exists(&plan) {
                        self.as_guest(conn, |g| {
                            g.send_overlay_message(
                                horizon_client::text::Text::translate(
                                    "spectator.cannot_teleport",
                                    &[],
                                )
                                .with_color(horizon_client::text::RED),
                            );
                        });
                        return true;
                    }
                    if self.server_player_travel_allowed(conn, plan.dim, TravelCause::NetherPortal)
                    {
                        self.start_server_trip(conn, plan.dim, Some(plan), false);
                    }
                }
            }
            "end_portal" => {
                let from_end = self.dim == Dim::End;
                // Entity.canTeleport: out of the End only after the credits.
                if !from_end || seen_credits {
                    let target = if from_end { Dim::Overworld } else { Dim::End };
                    if self.server_player_travel_allowed(conn, target, TravelCause::EndPortal) {
                        self.start_server_trip(conn, target, None, false);
                    }
                }
            }
            _ => {
                if self.server_player_travel_allowed(conn, self.dim, TravelCause::EndGateway) {
                    self.start_gateway_trip(conn, pos);
                }
            }
        }
        true
    }

    /// Whether an exit portal already stands in the plan's dimension
    /// (`PortalForcer.findClosestPortalPosition`), searched without building.
    fn spectator_exit_exists(&mut self, plan: &Plan) -> bool {
        let source = self.dim;
        if self.enter_level(plan.dim).is_err() {
            return false;
        }
        let exists = self.find_exit_portal(plan).is_some();
        if let Err(e) = self.enter_level(source) {
            log::error!("could not reopen {}: {e}", source.id());
        }
        exists
    }

    /// Whether this guest's dimension change may start
    /// (`entity_may_travel`).
    pub(super) fn server_player_travel_allowed(
        &mut self,
        conn: u32,
        target: Dim,
        cause: TravelCause,
    ) -> bool {
        let Some(g) = self.lan.as_ref().and_then(|host| host.hub.guest(conn)) else {
            return false;
        };
        let entity = g.entity_id;
        self.entity_may_travel(entity as u32, target, cause)
    }

    /// `EntityTravelToDimensionEvent`: whether the entity `id` (the network
    /// id its handle names) may travel to the level `to`, asked before
    /// anything of it moves. Only a server asks, and only when a mod listens.
    pub(super) fn entity_may_travel(&mut self, id: u32, to: Dim, cause: TravelCause) -> bool {
        self.entity_tree_may_travel(id, to, cause, &mut Vec::new())
    }

    fn entity_tree_may_travel(
        &mut self,
        id: u32,
        to: Dim,
        cause: TravelCause,
        visited: &mut Vec<u32>,
    ) -> bool {
        if !self.native_authoritative()
            || !mod_api::__private::listened(mod_api::HookId::EntityTravelToDimension)
        {
            return true;
        }
        let Some(epoch) = self.native_lifecycle.epoch() else {
            return true;
        };
        if visited.contains(&id)
            || (!self.held_out.contains(&id)
                && self
                    .native_entity_target(mod_api::EntityRef::new(epoch, id))
                    .is_none())
        {
            return false;
        }
        let entity = mod_api::EntityRef::new(epoch, id);
        // A callback can reparent a participant into a later subtree.
        // Keep the whole transition's visited set, not only its active path.
        visited.push(id);
        let riders = self.entity_passengers(entity).unwrap_or_default();
        let mut cancel = false;
        self.native_entity_travel_to_dimension(
            mod_api::EntityRef::new(epoch, id),
            to.id(),
            cause.id(),
            &mut cancel,
        );
        // A discard hides the handle immediately, even when physical removal
        // waits for the entity sweep. Never serialize that deferred copy.
        // Falling blocks tick held out of the level and cannot be addressed
        // through a handle during this callback.
        if cancel {
            return false;
        }
        if self.held_out.contains(&id) {
            return true;
        }
        if self
            .native_entity_target(mod_api::EntityRef::new(epoch, id))
            .is_none()
        {
            return false;
        }
        // One snapshot in the shared player/mob boarding order. Ejection can
        // itself call mods; refuse changed edges instead of overriding them.
        for &rider in riders.iter().rev() {
            if self.entity_vehicle(rider) != Some(entity) {
                continue;
            }
            if !self.entity_stop_riding(rider) && self.entity_vehicle(rider) == Some(entity) {
                return false;
            }
        }
        let accepted: Vec<_> = riders
            .into_iter()
            .filter(|rider| {
                self.entity_is_passenger(*rider) == Some(false)
                    && self.entity_tree_may_travel(rider.id(), to, cause, visited)
            })
            .collect();
        for rider in accepted {
            if self.entity_is_passenger(rider) == Some(false)
                && !self.mobs.iter().any(|m| m.id == rider.id() && !m.alive())
            {
                // Reject callback topology changes: the shared mount path
                // checks live handles, dimension, cycles and vehicle capacity.
                self.entity_start_riding(rider, entity, true);
            }
        }
        self.native_entity_target(mod_api::EntityRef::new(epoch, id))
            .is_some()
    }

    /// `Entity.handlePortal` for one player, whose portal state is in place.
    pub(super) fn handle_server_portal(&mut self, conn: u32) {
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let Some(g) = host.hub.guest(conn) else {
            return;
        };
        let (at, invulnerable, sleeping, riding) = (
            Vec3::new(g.position[0], g.position[1], g.position[2]),
            g.creative(),
            g.sleeping,
            g.vehicle.is_some(),
        );
        // Entity.checkInsideBlocks: the Nether portal blocks the box is in.
        // `LivingEntity.canUsePortal` (#943): not a passenger — its vehicle
        // goes through and carries it (`carry_riders_through_portals`) — and
        // not asleep.
        if g.motion
            .as_deref()
            .is_some_and(|p| self.can_use_portal_of(p, riding, sleeping))
            && let Some(pos) = self.nether_portal_inside(at)
        {
            self.portals.player.set_inside(pos, PLAYER_DELAY);
        }
        // `ServerPlayer.showEndCredits`: the player stays out of the world while the
        // poem rolls, then PERFORM_RESPAWN takes it to its respawn point.
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        if host.play.get(&conn).is_some_and(|play| play.won_game) {
            if host
                .play
                .get(&conn)
                .is_some_and(|play| play.respawn_requested)
            {
                if !self.server_player_travel_allowed(conn, Dim::Overworld, TravelCause::EndPortal)
                {
                    return;
                }
                // The first exit from the End keeps only `PlayerPersisted` of a mod's saved
                // compound, as a death respawn does; later exits keep all of it.
                self.keep_only_player_persisted(conn);
                let Some(host) = self.lan.as_mut() else {
                    return;
                };
                if let Some(play) = host.play.get_mut(&conn) {
                    play.respawn_requested = false;
                    play.won_game = false;
                }
                // PlayerList.respawn(keepAllPlayerData = true), CHANGED_DIMENSION:
                // PlayerEvent.Clone with `wasDeath` false.
                if let Some(g) = host.hub.guest(conn) {
                    let (uuid, name) = (g.uuid, g.name.clone());
                    if let Some(original) = self.capture_original_player(conn) {
                        self.with_original_player(original, |game, original| {
                            game.native_player_clone(uuid, &name, false, original);
                        });
                    }
                }
                self.start_server_trip(conn, Dim::Overworld, None, true);
                self.portals.player.cooldown = 10;
            }
            return;
        }
        let (end_portal, gateway) = self.inside_end_portals(at, WIDTH, HEIGHT);
        if end_portal && !riding {
            // EndPortalBlock.entityInside: the first exit from the End shows the credits.
            let Some(host) = self.lan.as_mut() else {
                return;
            };
            if self.dim == Dim::End && !host.hub.guest(conn).is_some_and(|g| g.seen_credits) {
                let _ = host
                    .hub
                    .guests
                    .update_connection(conn, |g| g.seen_credits = true);
                host.play.entry(conn).or_default().won_game = true;
                // ClientboundGameEventPacket.WIN_GAME.
                host.hub.send_to(
                    conn,
                    &C::GameEvent(play::GameEvent {
                        event: 4,
                        param: 0.0,
                    }),
                );
                return;
            }
            if self.portals.player.cooldown == 0 {
                let target = if self.dim == Dim::End {
                    Dim::Overworld
                } else {
                    Dim::End
                };
                if !self.server_player_travel_allowed(conn, target, TravelCause::EndPortal) {
                    return;
                }
                self.start_server_trip(conn, target, None, false);
            }
            // Entity.setAsInsidePortal while cooling down restarts the cooldown.
            self.portals.player.cooldown = 10;
            return;
        }
        // EndGatewayBlock.entityInside: a gateway that is not cooling down
        // takes the player at once (no transition time) and cools down
        // (triggerCooldown), even for a player still on its own cooldown.
        if !riding
            && let Some(g) = gateway
            && self.gateway_available(g)
        {
            if self.portals.player.cooldown == 0 {
                let Some(player) = self.lan.as_ref().and_then(|host| host.hub.guest(conn)) else {
                    return;
                };
                let (id, name) = (player.uuid, player.name.clone());
                let mut cancel = false;
                self.native_player_teleport_end_gateway(id, &name, g, &mut cancel);
                if cancel {
                    return;
                }
                if !self.server_player_travel_allowed(conn, self.dim, TravelCause::EndGateway) {
                    return;
                }
            }
            if self.gateway_enter(g) {
                if self.portals.player.cooldown == 0 {
                    self.start_gateway_trip(conn, g);
                }
                // setPortalCooldown: after the trip, or again while cooling down.
                self.portals.player.cooldown = 10;
                return;
            }
        }
        let rule = if invulnerable {
            horizon_client::gamerules::vanilla::PLAYERS_NETHER_PORTAL_CREATIVE_DELAY
        } else {
            horizon_client::gamerules::vanilla::PLAYERS_NETHER_PORTAL_DEFAULT_DELAY
        };
        let transition = self.ui.rules.int_at(rule).max(0);
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
        if !self.server_player_travel_allowed(conn, plan.dim, TravelCause::NetherPortal) {
            // The processor already reached its threshold. Forget this attempt
            // so a canceled trip must complete a fresh portal wait before retrying.
            self.portals.player.process = None;
            return;
        }
        self.start_server_trip(conn, plan.dim, Some(plan), false);
    }

    /// `Entity.handlePortal` of a vehicle server players ride, once through
    /// a portal (a vehicle's transition time is 0): the riders travel with
    /// it and ride it again on arrival (`Entity.teleportCrossDimension`), so
    /// a rider never waits for its own portal delay.
    fn carry_riders_through_portals(&mut self) {
        for (vehicle, entry) in self.take_carrying(self.dim) {
            let Some(plan) = self.vehicle_plan(vehicle, entry) else {
                continue;
            };
            if !self.may_enter(plan.dim) {
                continue;
            }
            let Some(host) = self.lan.as_ref() else {
                return;
            };
            let riders: Vec<u32> = host
                .hub
                .guests
                .iter()
                .filter(|g| {
                    g.vehicle
                        .is_some_and(|mount| self.vehicle_tree_contains_mount(vehicle, mount))
                        && !self.on_server_trip(g.conn)
                })
                .map(|g| g.conn)
                .collect();
            // Entity.canTeleport: out of the End, every rider has seen the
            // credits.
            if plan.end
                && self.dim == Dim::End
                && riders
                    .iter()
                    .any(|c| !host.hub.guest(*c).is_some_and(|g| g.seen_credits))
            {
                continue;
            }
            // Refused passengers are detached and stay in the source level.
            let cause = TravelCause::of_plan(&plan);
            if !self.entity_may_travel(vehicle, plan.dim, cause) {
                continue;
            }
            // An End portal's exit is found as a player's (its own respawn
            // point out of the End).
            let (target, plan) = (plan.dim, (!plan.end).then_some(plan));
            for conn in riders {
                if !self
                    .lan
                    .as_ref()
                    .and_then(|h| h.hub.guest(conn))
                    .is_some_and(|g| {
                        g.vehicle
                            .is_some_and(|mount| self.vehicle_tree_contains_mount(vehicle, mount))
                    })
                {
                    continue;
                }
                self.start_server_trip(conn, target, plan, false);
            }
        }
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
    pub(super) fn start_server_trip(
        &mut self,
        conn: u32,
        target: Dim,
        plan: Option<Plan>,
        credits_respawn: bool,
    ) {
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        let Some(i) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return;
        };
        if target != self.dim {
            drop_hook(host, conn);
        }
        let g = &host.hub.guests[i];
        let from = Vec3::new(g.position[0], g.position[1], g.position[2]);
        let yaw = g.yaw;
        // The vehicle the player rides travels with it: the player is its
        // passenger, and the vehicle is recreated in the other dimension,
        // where they ride it again (`Entity.teleportCrossDimension`). It
        // keeps its entity id, so the ride is never broken.
        let direct_vehicle = g.vehicle;
        let vehicle = direct_vehicle.filter(|id| {
            self.mob_object(*id).is_some_and(|mob| !mob.removed)
                || self.boats.iter().any(|b| b.id == *id)
                || self.carts.iter().any(|c| c.id == *id)
        });
        let source = self.dim;
        let end = plan.is_none();
        // Coordinates scale by the levels' coordinate scales (8 between the Overworld
        // and the Nether); an End portal has fixed destinations.
        let scale = if end {
            1.0
        } else {
            horizon_world::dimension_type::DimensionFacts::teleportation_scale(
                self.dimension_type_of(source),
                self.dimension_type_of(target),
            )
        };
        self.end_portal_travel = end;
        // Leaving the End: the player's own respawn point.
        let own = if end {
            self.as_guest(conn, |g| g.end_destination(target)).flatten()
        } else {
            None
        };
        let scaled = own
            .or_else(|| self.end_destination(target))
            .unwrap_or(Vec3::new(from.x * scale, from.y, from.z * scale));
        self.end_portal_travel = false;
        if let Err(e) = self.enter_level(target) {
            log::error!("could not open {}: {e}", target.id());
            return;
        }
        let destination = match plan {
            Some(plan) => self.arrival_centre(&plan),
            None => scaled,
        };
        let at = [destination.x, destination.y, destination.z];
        let ctx = server_ctx!(self);
        // Through a portal: the client waits on its loading screen until
        // the exit is found (`arrive_server_trips`). The cooldown is the one
        // Entity.handlePortal set before the teleport.
        if let Some(host) = self.lan.as_mut() {
            host.hub
                .change_level(i, at, yaw, None, true, PLAYER_DELAY, &ctx);
        }
        if credits_respawn && let Some(host) = self.lan.as_mut() {
            host.hub.guests.update(i, |g| {
                g.load_timeout = horizon_client::lan::guest::CLIENT_LOADED_TIMEOUT;
                g.loading = true;
            });
        }
        self.bind_player_attachments(conn, false);
        self.as_guest(conn, |g| {
            g.trigger_dimension(source, target, [from.x, from.y, from.z]);
            if source != target && !credits_respawn {
                g.stop_active_item_use();
            }
        });
        self.server_trips.push(Trip {
            conn,
            dim: target,
            source,
            target: destination,
            end,
            plan,
            vehicle,
            gateway: None,
            respawn: None,
            world_spawn: false,
            missing: None,
            finder: None,
            placed: false,
            hold: false,
            left: Some(source),
        });
        if let Err(e) = self.enter_level(source) {
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
            source: self.dim,
            target: Vec3::new(f64::from(x) + 0.5, f64::from(y), f64::from(z) + 0.5),
            end: false,
            plan: None,
            vehicle: None,
            gateway: None,
            respawn: Some(config),
            world_spawn: false,
            missing: None,
            finder: None,
            placed: false,
            hold: false,
            left: None,
        });
    }

    /// `findRespawnAndUseSpawnBlock` for the player in place, at its point
    /// in the level in place: where it stands up, facing the anchor (with
    /// the deplete sound) or the bed; else the point is forgotten, and the
    /// caller tells the player (`Hub::missing_respawn_block`).
    pub(super) fn resolve_server_respawn(
        &mut self,
        config: RespawnConfig,
    ) -> Option<(Vec3, f32, f32)> {
        let [x, y, z] = config.pos;
        let anchor = self.is_anchor(self.world.block(x, y, z));
        match self.find_respawn_and_use(config, true) {
            Ok(Some((at, yaw, pitch))) => {
                if !config.forced && anchor {
                    let [x, y, z] = config.pos.map(f64::from);
                    self.audio.play(
                        "minecraft:block.respawn_anchor.deplete",
                        Some([x, y, z]),
                        1.0,
                        1.0,
                    );
                }
                // PlayerList.respawn: the block was found, so the new player
                // copies the respawn point (`copyRespawnPosition`). A cancel
                // leaves it with none.
                if self.spawn_point_change(config.dim, Some(config.pos), config.forced) {
                    self.respawn_config = None;
                }
                Some((at, yaw, pitch))
            }
            Ok(None) | Err(_) => {
                // The block is gone: `PlayerList.respawn` skips
                // `copyRespawnPosition`, so the new player has no point and
                // no `setRespawnPosition` call is made.
                self.respawn_config = None;
                None
            }
        }
    }

    /// TeleportTransition.createDefault and missingRespawnBlock: the player
    /// waits at the world spawn for its chunks, then PlayerSpawnFinder puts
    /// it on a free column within the respawn radius.
    pub(super) fn world_spawn_trip(&mut self, conn: u32, missing: Option<RespawnConfig>) {
        let Some([x, y, z]) = self.level.world_spawn else {
            return;
        };
        self.server_trips.retain(|t| !t.is(conn));
        self.server_trips.push(Trip {
            conn,
            dim: Dim::Overworld,
            source: self.dim,
            target: Vec3::new(f64::from(x) + 0.5, f64::from(y), f64::from(z) + 0.5),
            end: false,
            plan: None,
            vehicle: None,
            gateway: None,
            respawn: None,
            world_spawn: true,
            missing,
            finder: None,
            placed: false,
            hold: false,
            left: None,
        });
    }

    /// TeleportTransition.missingRespawnBlock: the world spawn, in the
    /// Overworld.
    fn respawn_at_world_spawn(&mut self, conn: u32, missing: RespawnConfig) {
        let at = self.level.world_spawn.map_or(
            self.spawn.map_or([0.5, 80.0, 0.5], |s| [s.x, s.y, s.z]),
            |[x, y, z]| [f64::from(x) + 0.5, f64::from(y), f64::from(z) + 0.5],
        );
        self.level_teleports.push(LevelTeleport {
            conn,
            dim: Dim::Overworld,
            at,
            rotation: None,
            vehicle: None,
        });
        self.run_level_teleports();
        self.world_spawn_trip(conn, Some(missing));
    }

    /// `TheEndGatewayBlockEntity.getPortalPosition` and the teleport within
    /// the End (keeping the player's motion and look): straight to an exact
    /// exit, else once the destination has loaded around the player, who
    /// waits there (its exit is found among those blocks, or built).
    fn start_gateway_trip(&mut self, conn: u32, gateway: [i32; 3]) {
        let Some(i) = self
            .lan
            .as_ref()
            .and_then(|host| host.hub.guests.iter().position(|g| g.conn == conn))
        else {
            return;
        };
        let (job, destination) = self.gateway_job(gateway);
        let at = [destination.x, destination.y, destination.z];
        // Relative.union(DELTA, ROTATION).
        if let Some(host) = self.lan.as_mut() {
            host.hub.teleport_keeping_motion(i, at);
        }
        if let GatewayJob::Known { exact: true, .. } = job {
            return;
        }
        self.server_trips.push(Trip {
            conn,
            dim: self.dim,
            source: self.dim,
            target: destination,
            end: false,
            plan: None,
            vehicle: None,
            gateway: Some(job),
            respawn: None,
            world_spawn: false,
            missing: None,
            finder: None,
            placed: false,
            hold: false,
            left: None,
        });
    }

    /// The trips into the level in place whose destination has loaded
    /// around their target: the exit (the Nether portal found or built, the
    /// End platform, the Overworld's spawn column) and the teleport there.
    fn arrive_server_trips(&mut self) {
        self.release_held_arrivals();
        let mut k = 0;
        while k < self.server_trips.len() {
            let trip = &self.server_trips[k];
            let Some(host) = self.lan.as_ref() else {
                return;
            };
            let Some(i) = host.hub.guests.iter().position(|g| g.conn == trip.conn) else {
                // They left on the way.
                self.server_trips.remove(k);
                continue;
            };
            let (cx, cz) = (
                (trip.target.x.floor() as i32) >> 4,
                (trip.target.z.floor() as i32) >> 4,
            );
            if trip.hold {
                self.lan.as_mut().unwrap().hub.guests.update(i, |g| {
                    g.landing = true;
                    g.tp_held = true;
                });
            }
            if trip.dim != self.dim
                || !(-2..=2).all(|dz| (-2..=2).all(|dx| self.streaming.is_lit(cx + dx, cz + dz)))
                || trip
                    .gateway
                    .and_then(GatewayJob::source_chunk)
                    .is_some_and(|c| !self.world.chunks.contains_key(&c))
            {
                k += 1;
                continue;
            }
            let mut trip = self.server_trips.remove(k);
            if trip.hold {
                self.lan.as_mut().unwrap().hub.guests.update(i, |g| {
                    g.landing = false;
                    g.tp_held = false;
                });
            }
            if trip.placed {
                self.tell_mods_changed_dimension(trip.conn, trip.left, trip.dim);
                continue;
            }
            // A portal's trip (`start_server_trip`): its client waits to land.
            let portal = trip.end || trip.plan.is_some();
            self.resolve_arrivals();
            let Some(host) = self.lan.as_ref() else {
                return;
            };
            let yaw = host.hub.guests[i].yaw;
            let (position, yaw) = if let Some(config) = trip.respawn {
                let found = self.as_guest(trip.conn, |g| g.resolve_server_respawn(config));
                match found {
                    Some(Some((at, yaw, pitch))) => {
                        if let Some(host) = self.lan.as_mut() {
                            host.hub.guests.update(i, |guest| guest.pitch = pitch);
                        }
                        (at, yaw)
                    }
                    _ => {
                        if let Some(host) = self.lan.as_mut() {
                            host.hub.missing_respawn_block(trip.conn);
                        }
                        self.respawn_at_world_spawn(trip.conn, config);
                        // The move to the world spawn ends the player's move
                        // from the level they left.
                        if let Some(left) = trip.left
                            && let Some(t) = self.server_trips.iter_mut().find(|t| t.is(trip.conn))
                        {
                            t.left = Some(left);
                        }
                        continue;
                    }
                }
            } else if trip.world_spawn {
                // PlayerSpawnFinder.findSpawn around the world spawn.
                let suggestion =
                    [trip.target.x, trip.target.y, trip.target.z].map(|v| v.floor() as i32);
                match self.search_spawn(suggestion, &mut trip.finder) {
                    // The respawn already put the player on this column:
                    // vanilla teleports once, so no second move.
                    Ok(at)
                        if self.lan.as_ref().is_some_and(|host| {
                            host.hub.guests[i].position == [at.x, at.y, at.z]
                        }) =>
                    {
                        self.tell_mods_changed_dimension(trip.conn, trip.left, trip.dim);
                        // PlayerRespawnPositionEvent: the column is the point.
                        let pitch = self.lan.as_ref().map_or(0.0, |h| h.hub.guests[i].pitch);
                        let found = Respawn {
                            dim: trip.dim,
                            at,
                            yaw,
                            pitch,
                        };
                        if let Some(d) = self.respawn_position(trip.conn, found, trip.missing) {
                            self.apply_respawn_copy(trip.conn, &d, trip.missing);
                            if d.moved && d.to.dim != trip.dim {
                                self.respawn_into_other_level(trip.conn, d.to);
                            } else if d.moved
                                && let Some(host) = self.lan.as_mut()
                            {
                                host.hub.guests.update(i, |g| {
                                    (g.yaw, g.pitch) = (d.to.yaw, d.to.pitch);
                                    g.new_body(true);
                                });
                                host.hub
                                    .teleport(i, Some([d.to.at.x, d.to.at.y, d.to.at.z]));
                            }
                        }
                        continue;
                    }
                    Ok(at) => (at, yaw),
                    Err(chunk) => {
                        self.spawn_wait.push(chunk);
                        self.server_trips.insert(k, trip);
                        k += 1;
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
                let guest_arrival =
                    self.as_guest(trip.conn, |g| g.end_arrival(trip.target, &mut trip.finder));
                let arrival = match guest_arrival {
                    Some(arrival) => arrival,
                    None => self.end_arrival(trip.target, &mut trip.finder),
                };
                match arrival {
                    Ok(Some((at, turned))) => {
                        // EndPortalBlock.getPortalDestination: PLACE_PORTAL_TICKET.
                        self.place_portal_ticket(Self::chunk_of(at));
                        (at, turned.unwrap_or(yaw))
                    }
                    Ok(None) => {
                        self.server_trips.insert(k, trip);
                        k += 1;
                        continue;
                    }
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
            // PlayerRespawnPositionEvent: a deferred respawn's point is found.
            let mut pitch = None;
            let (position, yaw) = if trip.respawn.is_some() || trip.world_spawn {
                let pitch_now = self.lan.as_ref().map_or(0.0, |h| h.hub.guests[i].pitch);
                let found = Respawn {
                    dim: trip.dim,
                    at: position,
                    yaw,
                    pitch: pitch_now,
                };
                match self.respawn_position(trip.conn, found, trip.missing) {
                    Some(d) => {
                        self.apply_respawn_copy(trip.conn, &d, trip.missing);
                        if d.to.dim != trip.dim {
                            self.tell_mods_changed_dimension(trip.conn, trip.left, trip.dim);
                            self.respawn_into_other_level(trip.conn, d.to);
                            continue;
                        }
                        pitch = Some(d.to.pitch);
                        (d.to.at, d.to.yaw)
                    }
                    None => (position, yaw),
                }
            } else {
                (position, yaw)
            };
            // The vehicle the player rode comes with it, and waits for it
            // where it arrives (`Entity.teleportCrossDimension` carries the
            // passengers).
            if let Some(vehicle) = trip.vehicle {
                self.arrive_vehicle(trip.source, vehicle, position, yaw);
            }
            // Player.getDimensionChangingDelay.
            let Some(host) = self.lan.as_mut() else {
                return;
            };
            if !trip.hold {
                host.play.entry(trip.conn).or_default().portal.cooldown = 10;
            }
            host.hub.guests.update(i, |g| {
                g.yaw = yaw;
                if let Some(pitch) = pitch {
                    g.pitch = pitch;
                }
                // A new body at the exit: no fall distance or motion carried over,
                // but the player's attributes (ServerPlayer.teleport keeps the entity).
                if !trip.hold {
                    g.new_body(true);
                }
            });
            let at = [position.x, position.y, position.z];
            if portal {
                host.hub.land(i, at);
            } else {
                host.hub.teleport(i, Some(at));
            }
            self.tell_mods_changed_dimension(trip.conn, trip.left, trip.dim);
        }
    }

    /// The vehicle the player rode arrives with it, at the point the player
    /// arrives: it keeps its entity id, so the ride is never broken
    /// (`Entity.teleportCrossDimension` recreates the vehicle in the other
    /// dimension, and its passengers ride it again).
    fn arrive_vehicle(&mut self, source: Dim, id: u32, at: Vec3, yaw: f32) {
        let Some(id) = self.parked_vehicle_root(source, id) else {
            return;
        };
        let motion = self
            .portals
            .carried_motion
            .remove(&id)
            .unwrap_or(Vec3::ZERO);
        let carried = self.portals.carried_cooldown.remove(&id);
        let Some(parked) = self.levels.parked_mut(source) else {
            return;
        };
        let before = parked
            .boats
            .iter()
            .find(|body| body.id == id)
            .map(|body| (body.position, [body.yaw, 0.0]))
            .or_else(|| {
                parked
                    .carts
                    .iter()
                    .find(|body| body.id == id)
                    .map(|body| (body.position, [body.yaw, 0.0]))
            })
            .or_else(|| {
                parked
                    .mobs
                    .iter()
                    .find(|body| body.id == id)
                    .map(|body| (body.position, [body.yaw, body.head_pitch]))
            });
        let Some((from, rotation)) = before else {
            return;
        };
        // Keep the root out of both levels until its passengers have been
        // admitted: teleportCrossDimension copies passengers before the root.
        let vehicle = if let Some(body) = take_vehicle(&mut parked.boats, id, at, yaw, motion) {
            OffLevel::Boat(body)
        } else if let Some(body) = take_vehicle(&mut parked.carts, id, at, yaw, motion) {
            OffLevel::Cart(body)
        } else if let Some(body) = take_vehicle(&mut parked.mobs, id, at, yaw, motion) {
            OffLevel::Mob(body)
        } else {
            return;
        };
        let left = parked
            .portal_states
            .remove(&id)
            .map_or(0, |state| state.cooldown);
        // All object-owned metadata moves, including rejected intermediate bodies;
        // ordinary destination queries still expose only admitted objects.
        for mut mob in self.move_entity_memory(source, id) {
            mob.position = super::vehicle_teleport::passenger_destination(from, at, mob.position);
            mob.previous = mob.position;
            let [turned_yaw, pitch] = super::vehicle_teleport::passenger_rotation(
                rotation,
                [yaw, rotation[1]],
                [mob.yaw, mob.head_pitch],
            );
            mob.yaw = turned_yaw;
            mob.previous_yaw = turned_yaw;
            mob.body_yaw = turned_yaw;
            mob.head_yaw = turned_yaw;
            mob.head_pitch = pitch;
            mob.previous_head_pitch = pitch;
            self.admit_transferred_object(OffLevel::Mob(mob));
        }
        self.admit_transferred_object(vehicle);
        // The source may have ticked while the player's destination was loading.
        self.portals.entities.entry(id).or_default().cooldown = carried.unwrap_or(left);
    }

    /// Tells the mods of the teleports a command made inside a level, with
    /// each player's state swapped in: where the move has reached the hub.
    pub(super) fn flush_teleport_notes(&mut self) {
        for note in std::mem::take(&mut self.teleport_notes) {
            let TeleportNote {
                conn,
                uuid,
                name,
                from,
                to,
                dimension,
            } = note;
            // In place when it is their turn already (as_guest finds them swapped in).
            if self.lan_acting == Some(conn) {
                self.native_player_teleported(uuid, &name, from, to, &dimension);
            } else {
                self.as_guest(conn, |g| {
                    g.native_player_teleported(uuid, &name, from, to, &dimension);
                });
            }
        }
    }

    /// The players the command just run sends to another level
    /// (`ServerPlayer.teleport` from TeleportCommand): a Respawn packet
    /// keeping all data and the teleport, with no portal sound. The server
    /// leaves the body alone until the level has loaded around it.
    pub(super) fn run_level_teleports(&mut self) {
        if self.lan.is_none() {
            return;
        }
        for t in std::mem::take(&mut self.level_teleports) {
            let Some(host) = self.lan.as_mut() else {
                return;
            };
            let Some(i) = host.hub.guests.iter().position(|g| g.conn == t.conn) else {
                continue;
            };
            let source = self.dim;
            if t.dim != source {
                drop_hook(host, t.conn);
            }
            if let Err(e) = self.enter_level(t.dim) {
                log::error!("could not open {}: {e}", t.dim.id());
                continue;
            }
            let Some(host) = self.lan.as_mut() else {
                return;
            };
            let (uuid, name) = (host.hub.guests[i].uuid, host.hub.guests[i].name.clone());
            let (yaw, pitch, origin, passenger_body) = host.hub.guests.update(i, |g| {
                let [yaw, pitch] = t.rotation.unwrap_or([g.yaw, g.pitch]);
                let origin = g.position;
                // A new body there: no fall distance or motion carried over, but
                // the player's attributes (ServerPlayer.teleport keeps the entity).
                let passenger_body = if t.vehicle.is_some() {
                    g.motion.take()
                } else {
                    g.new_body(true);
                    None
                };
                (yaw, pitch, origin, passenger_body)
            });
            let cooldown = host.play.get(&t.conn).map_or(0, |p| p.portal.cooldown);
            let ctx = server_ctx!(self);
            if let Some(host) = self.lan.as_mut() {
                host.hub
                    .change_level(i, t.at, yaw, Some(pitch), false, cooldown, &ctx);
                host.hub.guests.update(i, |g| {
                    g.vehicle = t.vehicle;
                    if let Some(mut body) = passenger_body {
                        body.position = Vec3::new(t.at[0], t.at[1], t.at[2]);
                        g.motion = Some(body);
                    }
                });
            }
            self.bind_player_attachments(t.conn, false);
            self.as_guest(t.conn, |g| {
                g.trigger_dimension(source, t.dim, origin);
                // The mods hear the teleport into another level, with the
                // player's state swapped in.
                g.native_player_teleported(uuid, &name, origin, t.at, t.dim.id());
            });
            self.server_trips.retain(|trip| !trip.is(t.conn));
            self.server_trips.push(Trip {
                conn: t.conn,
                dim: t.dim,
                source,
                target: Vec3::new(t.at[0], t.at[1], t.at[2]),
                end: false,
                plan: None,
                // A command teleports the player alone: it is not a
                // passenger any more (`ServerPlayer.teleport`).
                vehicle: None,
                gateway: None,
                respawn: None,
                world_spawn: false,
                missing: None,
                finder: None,
                placed: true,
                hold: false,
                left: Some(source),
            });
            if let Err(e) = self.enter_level(source) {
                log::error!("could not reopen {}: {e}", source.id());
            }
        }
    }

    /// `native_player_changed_dimension` for a guest who has ended a move
    /// from the level `left` to the one in place, `to`, with their state
    /// swapped in so that the helpers reach them. Nothing is heard when they
    /// did not leave a level, or ended where they started.
    pub(super) fn tell_mods_changed_dimension(&mut self, conn: u32, left: Option<Dim>, to: Dim) {
        let Some(from) = left.filter(|from| *from != to) else {
            return;
        };
        let Some((uuid, name)) = self
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
            .map(|g| (g.uuid, g.name.clone()))
        else {
            return;
        };
        self.as_guest(conn, |g| {
            g.native_player_changed_dimension(uuid, &name, from.id(), to.id());
        });
    }

    /// The hub's view of the server for packets sent outside its own tick.
    #[cfg(test)]
    pub(super) fn server_ctx(&mut self) -> TickCtx<'_> {
        let spawn = self.world_spawn_position();
        let clock_rate = self.network_clock_rate();
        TickCtx {
            sea_level: self.end.locator().map_or(63, |terrain| terrain.sea_level()),
            world: &mut self.world,
            entities: &[],
            next_entity: &mut self.next_entity,
            spawn,
            spawn_angle: self.level.world_spawn_angle,
            difficulty: i32::from(self.ui.difficulty),
            difficulty_locked: self.level.difficulty_locked,
            clock_rate,
            tick_rate: self.clock.rate(),
            tick_frozen: self.clock.frozen(),
            tick_steps: self.clock.step_state(),
            collision: Some(self.collision),
            border: &self.border,
            streaming: Some(&self.streaming),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::ServerHost;
    use std::sync::Arc;

    #[test]
    #[ignore = "needs prepared resources"]
    fn guest_portal_entry_uses_body_sleep_and_vehicle() {
        super::super::native_guest_turn_tests::with_guests(|game| {
            let mut chunk = horizon_client::chunk::Chunk::empty(0, 0);
            chunk.lit = true;
            game.world.chunks.insert((0, 0), Arc::new(chunk));
            let portal = game
                .pack
                .assets
                .registry
                .default_state("minecraft:nether_portal")
                .unwrap();
            game.world.set(3, 64, 5, portal).unwrap();
            for (body, dead, sleeping, riding, expected) in [
                (true, false, false, false, 9),
                (true, false, true, false, 4),
                (true, false, false, true, 4),
                (true, true, false, false, 4),
                (false, false, false, false, 4),
            ] {
                game.lan.as_mut().unwrap().hub.update_guest(12, |g| {
                    g.position = [3.5, 64.0, 5.5];
                    g.sleeping = sleeping;
                    g.vehicle = riding.then_some(99);
                    g.motion = body.then(|| {
                        Box::new(
                            horizon_client::motion::PlayerMotion::new(
                                game.motion,
                                Vec3::new(3.5, 64.0, 5.5),
                            )
                            .unwrap(),
                        )
                    });
                    if dead {
                        g.motion.as_mut().unwrap().set_health(0.0, false).unwrap();
                    }
                });
                game.portals.player.process = Some(horizon_client::nether_portal::Process {
                    entry: BlockPos::new(3, 64, 5),
                    time: 8,
                    inside: false,
                });
                game.handle_server_portal(12);
                assert_eq!(
                    game.portals.player.process.unwrap().time,
                    expected,
                    "body={body} dead={dead} sleeping={sleeping} riding={riding}"
                );
            }
        });
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn arriving_vehicle_is_announced_once_after_destination_entities() {
        use mod_api::{self, EntityJoinLevelEvent, Flow, Hooks};
        use std::sync::Mutex;
        struct Observer(Arc<Mutex<Vec<u32>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Observer {
            fn entity_join_level(
                &self,
                _: &mut dyn ServerHost,
                event: &mut EntityJoinLevelEvent<'_>,
            ) -> Flow {
                self.0.lock().unwrap().push(event.entity);
                Flow::Continue
            }
        }
        let joins = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "arrival-observer",
            Box::new(Observer(joins.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                game.native_joins_skip();
                let base = game.next_entity;
                game.next_entity += 2;
                game.carts.push(minecarts::Minecart::new(
                    base + 1,
                    minecarts::CartKind::Rideable,
                    Vec3::ZERO,
                ));
                let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                let mut child = Mob::new(base + 10, kind, Vec3::ZERO, 0.0);
                child.extra.mount = Some(base + 1);
                game.persist
                    .off_level
                    .insert(base + 10, super::super::persist::OffLevel::Mob(child));
                // It left with 7; the source level has run it down to 3 since.
                game.portals.entities.entry(base + 1).or_default().cooldown = 3;
                game.portals.carried_cooldown.insert(base + 1, 7);
                let source = game.dim;
                game.enter_level(Dim::Nether).unwrap();
                game.carts.push(minecarts::Minecart::new(
                    base + 2,
                    minecarts::CartKind::Rideable,
                    Vec3::ZERO,
                ));
                game.native_joins_flush(false, 0);
                game.arrive_vehicle(source, base + 1, Vec3::ZERO, 0.0);
                assert!(
                    game.mob_object(base + 10).is_some(),
                    "natural carrier arrival must transfer retained passenger"
                );
                assert_eq!(game.portals.entities[&(base + 1)].cooldown, 7);
                assert!(
                    !game
                        .levels
                        .parked_mut(source)
                        .unwrap()
                        .portal_states
                        .contains_key(&(base + 1))
                );
                game.native_joins_flush(false, 0);
                game.native_joins_flush(false, 0);
                assert_eq!(*joins.lock().unwrap(), vec![base + 2, base + 10, base + 1]);
            });
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn natural_carrier_moves_full_objects_and_readmits_the_tree_in_order() {
        use mod_api::{AttachmentType, EntityConstructingEvent, Holder, Hooks, HostSide};
        use std::sync::Mutex;
        const STATE: AttachmentType<i32> = AttachmentType {
            namespace: "natural_tree",
            name: "state",
            default: || 0,
            codec: None,
            copy_on_death: false,
            sync: false,
        };
        struct Constructed(Arc<Mutex<Vec<Holder>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Constructed {
            fn entity_constructing(&self, host: HostSide<'_>, event: &mut EntityConstructingEvent) {
                self.0.lock().unwrap().push(event.holder);
                if let HostSide::Server(host) = host {
                    STATE.set(host, event.holder, 41).unwrap();
                }
            }
        }
        for conflict in [false, true] {
            let calls = Arc::new(Mutex::new(Vec::new()));
            let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
                "natural_tree",
                Box::new(Constructed(calls.clone())),
            )]
            .into();
            mod_platform::testing::with_mods(mods, || {
                super::super::native_entity_refs::tests::with_game(|game| {
                    game.options.dedicated = Some(Vec::new());
                    let tag = horizon_core::nbt::parse_snbt(r#"{
                    id:"minecraft:pig",UUID:[I;703,0,0,1],Pos:[1.0d,80.0d,2.0d],NoAI:1b,
                    Passengers:[
                        {id:"minecraft:cow",UUID:[I;703,0,0,1],Pos:[1.0d,81.0d,2.0d],NoAI:1b,
                         Passengers:[{id:"minecraft:chicken",UUID:[I;703,0,0,2],Pos:[1.0d,82.0d,2.0d],NoAI:1b}]},
                        {id:"minecraft:sheep",UUID:[I;703,0,0,2],Pos:[1.0d,81.0d,2.0d],NoAI:1b}
                    ]
                }"#).unwrap().0;
                    let root = game.load_mob(tag, None).unwrap();
                    let children = game.mob_passenger_ids(root);
                    let (held, sibling) = (children[0], children[1]);
                    let leaf = game.mob_passenger_ids(held)[0];
                    let ids = [root, held, leaf, sibling];
                    assert!(game.persist.off_level.contains_key(&held));
                    assert!(game.persist.off_level.contains_key(&sibling));
                    STATE.set(game, Holder::Entity(leaf), 77).unwrap();
                    game.persist.forced_meta.insert(leaf, [5, 6, 7, 8]);
                    game.portals.entities.entry(leaf).or_default().cooldown = 19;
                    let source = game.dim;
                    game.enter_level(Dim::Nether).unwrap();
                    let owner = conflict.then(|| {
                    let tag = horizon_core::nbt::parse_snbt(r#"{id:"minecraft:zombie",UUID:[I;703,0,0,1],Pos:[0.0d,70.0d,0.0d],NoAI:1b}"#).unwrap().0;
                    game.load_mob(tag, None).unwrap()
                });
                    let before = calls.lock().unwrap().clone();
                    // A player's direct mount may be nested below a retained object;
                    // arrival must still carry the actual source root and every sibling.
                    let destination = Vec3::new(32.0, 90.0, 48.0);
                    game.arrive_vehicle(source, leaf, destination, 90.0);
                    assert!(ids.iter().all(|id| game.mob_object(*id).is_some()));
                    assert!(game.persist.off_level.contains_key(&root));
                    assert!(game.persist.off_level.contains_key(&sibling));
                    assert_eq!(game.persist.off_level.contains_key(&held), conflict);
                    assert!(game.entity(root).is_none());
                    assert!(game.entity(sibling).is_none());
                    assert!(game.entity(leaf).is_some());
                    assert_eq!(game.entity(held).is_some(), !conflict);
                    if let Some(owner) = owner {
                        assert!(game.entity(owner).is_some());
                    }
                    assert_eq!(game.mob_object(held).unwrap().extra.mount, Some(root));
                    assert_eq!(game.mob_object(leaf).unwrap().extra.mount, Some(held));
                    assert_eq!(game.mob_object(sibling).unwrap().extra.mount, Some(root));
                    assert_eq!(game.mob_object(root).unwrap().position, destination);
                    assert_eq!(
                        game.mob_object(held).unwrap().position,
                        Vec3::new(32.0, 91.0, 48.0)
                    );
                    assert_eq!(
                        game.mob_object(leaf).unwrap().position,
                        Vec3::new(32.0, 92.0, 48.0)
                    );
                    assert_eq!(
                        game.mob_object(sibling).unwrap().position,
                        Vec3::new(32.0, 91.0, 48.0)
                    );
                    assert_eq!(game.persist.forced_meta.get(&leaf), Some(&[5, 6, 7, 8]));
                    assert_eq!(game.portals.entities[&leaf].cooldown, 19);
                    assert_eq!(STATE.get(game, Holder::Entity(leaf)), Ok(77));
                    if !conflict {
                        assert_eq!(STATE.get(game, Holder::Entity(held)), Ok(41));
                    }
                    assert!(game.mobs_below_off_level().contains(&leaf));
                    game.native_joins_flush(false, 0);
                    assert_eq!(
                        *calls.lock().unwrap(),
                        before,
                        "arrival must not construct again"
                    );
                    // Another player from the same stack arrives without duplicating it.
                    game.arrive_vehicle(source, leaf, destination, 90.0);
                    assert_eq!(
                        game.mob_objects().filter(|m| ids.contains(&m.id)).count(),
                        4
                    );
                    game.enter_level(source).unwrap();
                    for id in ids {
                        assert!(game.mob_object(id).is_none());
                        assert!(!game.persist.uuids.contains_key(&id));
                        assert!(!game.persist.raw.contains_key(&id));
                        assert!(!game.persist.forced_meta.contains_key(&id));
                    }
                    game.enter_level(Dim::Nether).unwrap();
                    assert_eq!(STATE.get(game, Holder::Entity(leaf)), Ok(77));
                    game.native_joins_flush(false, 0);
                    assert_eq!(*calls.lock().unwrap(), before);
                })
            });
        }
    }

    /// `Entity.teleportCrossDimension` admits the passengers before the copied
    /// vehicle, so a pig that shares its boat's or minecart's UUID takes it in
    /// the destination and the vehicle is retained off-level below it.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn natural_carrier_retains_a_vehicle_root_refused_by_its_passenger() {
        for (kind, uuid) in [("minecraft:oak_boat", 705), ("minecraft:minecart", 706)] {
            super::super::native_entity_refs::tests::with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                let tag = horizon_core::nbt::parse_snbt(&format!(
                    r#"{{id:"{kind}",UUID:[I;{uuid},0,0,1],Pos:[1.0d,80.0d,2.0d],
                    Passengers:[{{id:"minecraft:pig",UUID:[I;{uuid},0,0,1],Pos:[1.0d,80.0d,2.0d],NoAI:1b,
                     Passengers:[{{id:"minecraft:chicken",UUID:[I;{uuid},0,0,2],Pos:[1.0d,80.0d,2.0d],NoAI:1b}}]}}]}}"#
                ))
                .unwrap()
                .0;
                let root = if kind.ends_with("boat") {
                    game.load_boat_transferred(tag, Some([uuid, 0, 0, 1]), kind, None)
                } else {
                    game.load_cart_transferred(tag, Some([uuid, 0, 0, 1]), kind, None)
                }
                .unwrap();
                let pig = game.mob_passenger_ids(root)[0];
                let chicken = game.mob_passenger_ids(pig)[0];
                assert!(game.persist.off_level.contains_key(&pig));
                let source = game.dim;
                game.enter_level(Dim::Nether).unwrap();
                game.arrive_vehicle(source, chicken, Vec3::new(32.0, 90.0, 48.0), 0.0);
                assert!(game.entity(pig).is_some());
                assert!(game.entity(chicken).is_some());
                assert!(game.entity(root).is_none());
                assert!(game.boats.iter().all(|b| b.id != root));
                assert!(game.carts.iter().all(|c| c.id != root));
                assert!(game.boat_object(root).is_some() || game.cart_object(root).is_some());
                assert_eq!(game.mob_object(pig).unwrap().extra.mount, Some(root));
                assert_eq!(game.mob_object(chicken).unwrap().extra.mount, Some(pig));
                assert!(game.mobs_below_off_level().contains(&pig));
                assert!(game.mobs_below_off_level().contains(&chicken));
                game.enter_level(source).unwrap();
                for id in [root, pig, chicken] {
                    assert!(!game.persist.off_level.contains_key(&id));
                    assert!(game.mob_object(id).is_none());
                }
                assert!(game.boats.iter().all(|b| b.id != root));
                assert!(game.carts.iter().all(|c| c.id != root));
            });
        }
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn natural_portal_schedules_a_player_on_a_retained_indirect_vehicle() {
        super::super::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            super::super::native_player_death::tests::add_guest(game);
            let source = game.dim;
            let tag = horizon_core::nbt::parse_snbt(
                r#"{
                id:"minecraft:pig",UUID:[I;704,0,0,1],Pos:[1.0d,80.0d,2.0d],NoAI:1b,
                Passengers:[{id:"minecraft:cow",UUID:[I;704,0,0,1],Pos:[1.0d,81.0d,2.0d],NoAI:1b}]
            }"#,
            )
            .unwrap()
            .0;
            let root = game.load_mob(tag, None).unwrap();
            let held = game.mob_passenger_ids(root)[0];
            assert!(game.persist.off_level.contains_key(&held));
            game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
                guest.vehicle = Some(held);
            });
            super::super::native_entity_refs::tests::prepare_entity_ticking_neighborhood(
                game,
                horizon_core::position::ChunkPos::new(0, 0),
            );
            game.tickets.add_portal((0, 0));
            game.update_chunk_tickets();
            assert!(game.entity_ticking_area()(1, 2));
            // Supply the block's real inside notification, then exercise the actual
            // entity portal processor, carrier routing and server trip scheduler.
            let delay = horizon_client::nether_portal::ENTITY_DELAY;
            game.portals
                .entities
                .entry(root)
                .or_default()
                .set_inside(BlockPos::new(1, 80, 2), delay);
            game.handle_entity_portals();
            assert!(
                game.mob_object(root).is_some(),
                "route through the player carrier path"
            );
            game.carry_riders_through_portals();
            assert_eq!(game.server_trips.len(), 1);
            let trip = &game.server_trips[0];
            assert_eq!(trip.source, source);
            assert_eq!(trip.dim, Dim::Nether);
            assert_eq!(
                trip.vehicle,
                Some(held),
                "keep the direct mount in the ticket"
            );
            assert_eq!(game.dim, source);
            game.enter_level(Dim::Nether).unwrap();
            game.arrive_vehicle(source, held, Vec3::new(32.0, 90.0, 48.0), 0.0);
            assert!(game.mob_object(root).is_some());
            assert!(game.mob_object(held).is_some());
            assert_eq!(game.mob_object(held).unwrap().extra.mount, Some(root));
            assert_eq!(game.lan.as_ref().unwrap().hub.guests[0].vehicle, Some(held));
        });
    }

    /// A minecart a player rides through a portal rolls on at the motion it
    /// crossed with, clamped to 0.4 along x and z (`PositionMoveRotation.of`
    /// takes `OldMinecartBehavior.getKnownMovement`), not at rest. Vanilla
    /// 26.3 sent the client a cart arriving with motion (0.4, 0, 0).
    #[test]
    fn a_minecart_a_player_rides_keeps_its_clamped_motion_through_a_portal() {
        let mut carts: horizon_client::join_seq::JoinList<_> = [minecarts::Minecart::new(
            3,
            minecarts::CartKind::Rideable,
            Vec3::new(9.9, 100.0, 4.5),
        )]
        .into_iter()
        .collect();
        let at = Vec3::new(1.5, 100.0, 0.5);
        let cart = take_vehicle(&mut carts, 3, at, 0.0, Vec3::new(1.6, -0.1, -0.7))
            .expect("the cart travels");
        assert_eq!(cart.position, at);
        assert_eq!(cart.velocity, Vec3::new(0.4, -0.1, -0.4));
    }

    /// The vehicle a player rides travels with it through a portal: it
    /// leaves the level the player came from, keeps its entity id (so the
    /// ride is never broken) and stands where its rider arrives
    /// (`Entity.teleportCrossDimension` carries the passengers, and they
    /// ride the vehicle that is recreated in the other dimension).
    #[test]
    fn the_vehicle_a_player_rides_travels_with_it() {
        let mut boats: horizon_client::join_seq::JoinList<_> = [boats::Boat::new(
            7,
            Arc::from("oak"),
            Vec3::new(0.0, 64.0, 0.0),
            0.0,
        )]
        .into_iter()
        .collect();
        let mut carts = horizon_client::join_seq::JoinList::<minecarts::Minecart>::default();
        let mut mobs = horizon_client::join_seq::JoinList::<Mob>::default();
        let at = Vec3::new(1.5, 65.0, -2.5);
        let boat = take_vehicle(&mut boats, 7, at, 90.0, Vec3::ZERO)
            .expect("the vehicle the player rides travels with it");
        assert!(
            boats.is_empty(),
            "the vehicle left the level the player came from"
        );
        assert_eq!(boat.id, 7, "the vehicle keeps its entity id");
        assert_eq!(boat.position, at, "it stands where its rider arrives");
        assert_eq!(boat.yaw, 90.0, "it looks where its rider looks");
        assert_eq!(boat.velocity, Vec3::ZERO, "it comes with no motion");
        // A vehicle that is not in the level does not travel.
        assert!(take_vehicle(&mut carts, 9, at, 0.0, Vec3::ZERO).is_none());
        assert!(carts.is_empty());
        assert!(take_vehicle(&mut mobs, 9, at, 0.0, Vec3::ZERO).is_none());
        assert!(mobs.is_empty());
    }
}

#[cfg(test)]
mod transfer_lifetime_tests {
    use super::*;
    use horizon_client::join_seq::Joined;
    use horizon_mod_runtime::attachments::{
        AttachmentLifetime, AttachmentType, Attachments, HasAttachmentLifetime, Holder,
        index::AttachmentIndex,
    };
    use std::sync::{Arc, Mutex};

    static VALUE: AttachmentType<i32> = AttachmentType {
        namespace: "test",
        name: "transfer",
        default: || 0,
        codec: None,
        copy_on_death: false,
        sync: false,
    };

    fn removal<T: Rideable + horizon_client::join_seq::Joined + HasAttachmentLifetime>(
        mut owner: T,
    ) {
        let index = Arc::new(Mutex::new(AttachmentIndex::default()));
        let mut map = Attachments::default();
        VALUE.set(&mut map, Holder::Entity(owner.id()), 17).unwrap();
        owner.attachment_lifetime().bind(&mut map, &index, 42u32);
        let mut list: horizon_client::join_seq::JoinList<T> = [owner].into_iter().collect();
        let id = list[0].id();
        let key = ("test".into(), "transfer".into());
        let retained = index.lock().unwrap().snapshot(&key);
        let moved = take_vehicle(&mut list, id, Vec3::ZERO, 0.0, Vec3::ZERO).unwrap();
        assert!(list.is_empty());
        assert!(index.lock().unwrap().snapshot(&key).is_empty());
        VALUE.set(&mut map, Holder::Entity(id), 18).unwrap();
        assert!(index.lock().unwrap().snapshot(&key).is_empty());
        assert_eq!(retained[0].entry.read(&VALUE), Ok(&17));
        drop(moved);
    }

    #[test]
    fn transfer_removal_retires_before_returning_the_vehicle() {
        removal(minecarts::Minecart::new(
            71,
            minecarts::CartKind::Rideable,
            Vec3::ZERO,
        ));
        removal(boats::Boat::new(72, Arc::from("oak"), Vec3::ZERO, 0.0));
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn transfer_arrival_rebinds_only_bound_owners_with_the_same_id() {
        for kind in ["minecraft:minecart", "minecraft:oak_boat", "minecraft:pig"] {
            for bound in [true, false] {
                super::super::native_entity_refs::tests::with_game(|game| {
                    game.tickets.update(None);
                    let target = game
                        .summon_with_nbt(kind, [0.5, 70.0, 0.5], "{}")
                        .unwrap()
                        .unwrap();
                    let id = match target {
                        super::super::commands::Target::Cart(id)
                        | super::super::commands::Target::Boat(id)
                        | super::super::commands::Target::Mob(id) => id,
                        _ => panic!("unexpected vehicle target"),
                    };
                    let old_join = match kind {
                        "minecraft:minecart" => game.carts[0].join_seq(),
                        "minecraft:oak_boat" => game.boats[0].join_seq(),
                        _ => game.mobs[0].join_seq(),
                    };
                    assert!(old_join.is_joined());
                    let uuid = game.uuid_for(id);
                    let source = game.dim;
                    let source_ref = game.entity(id).unwrap();
                    let source_index = game.attachment_indexes.entry(source).or_default().clone();
                    let mut map = Attachments::default();
                    VALUE.set(&mut map, Holder::Entity(id), 17).unwrap();
                    let old = if bound {
                        Some(map.bind_index(
                            &source_index,
                            mod_api::AttachmentHolder::Entity(source_ref),
                        ))
                    } else {
                        None
                    };
                    macro_rules! slot {
                        ($list:ident) => {
                            game.$list.update(0, |entity| {
                                if let Some(old) = &old {
                                    *entity.attachment_lifetime() =
                                        AttachmentLifetime::new(old.clone());
                                }
                            })
                        };
                    }
                    match kind {
                        "minecraft:minecart" => slot!(carts),
                        "minecraft:oak_boat" => slot!(boats),
                        _ => slot!(mobs),
                    }
                    let key = ("test".into(), "transfer".into());
                    let retained = source_index.lock().unwrap().snapshot(&key);
                    let mut stale_map = map.clone();
                    game.entity_attachments.insert(id, map);
                    game.enter_level(Dim::Nether).unwrap();
                    let destination_ref =
                        mod_api::EntityRef::new(game.level(game.dim.id()).unwrap().epoch(), id);
                    assert_eq!(source_ref, destination_ref);
                    game.arrive_vehicle(source, id, Vec3::ZERO, 0.0);
                    macro_rules! transferred {
                        ($list:ident) => {{
                            let parked = game.levels.parked_mut(source).unwrap();
                            assert!(
                                !parked
                                    .$list
                                    .iter()
                                    .any(|e| e.id == id || e.join_seq() == old_join)
                            );
                            let arrived = game.$list.iter().find(|e| e.id == id).unwrap();
                            assert!(arrived.join_seq().is_joined());
                            assert_ne!(arrived.join_seq(), old_join);
                            assert!(!game.$list.iter().any(|e| e.join_seq() == old_join));
                        }};
                    }
                    match kind {
                        "minecraft:minecart" => transferred!(carts),
                        "minecraft:oak_boat" => transferred!(boats),
                        _ => transferred!(mobs),
                    }
                    assert!(source_index.lock().unwrap().snapshot(&key).is_empty());
                    let destination_index =
                        game.attachment_indexes.entry(game.dim).or_default().clone();
                    let rows = destination_index.lock().unwrap().snapshot(&key);
                    assert_eq!(rows.len(), usize::from(bound));
                    if let Some(old) = old {
                        let current = match kind {
                            "minecraft:minecart" => game
                                .carts
                                .update(0, |e| e.attachment_lifetime().registration().unwrap()),
                            "minecraft:oak_boat" => game
                                .boats
                                .update(0, |e| e.attachment_lifetime().registration().unwrap()),
                            _ => game.mobs[0].attachment_lifetime.registration().unwrap(),
                        };
                        assert!(!old.same_registration(&current));
                        assert_eq!(
                            rows[0].holder,
                            mod_api::AttachmentHolder::Entity(destination_ref)
                        );
                        assert_eq!(rows[0].entry.read(&VALUE), Ok(&17));
                        assert!(!old.unregister());
                        drop(AttachmentLifetime::new(old));
                        VALUE.set(&mut stale_map, Holder::Entity(id), 99).unwrap();
                        assert_eq!(
                            destination_index.lock().unwrap().snapshot(&key)[0]
                                .entry
                                .read(&VALUE),
                            Ok(&17)
                        );
                        assert_eq!(retained[0].entry.read(&VALUE), Ok(&17));
                        for index in [&source_index, &destination_index] {
                            assert_eq!(index.lock().unwrap().validate_row(&retained[0]),
                                Err(horizon_mod_runtime::attachments::AttachmentRowError::StaleHolder));
                        }
                        assert_eq!(
                            destination_index.lock().unwrap().validate_row(&rows[0]),
                            Ok(())
                        );
                    }
                    assert_eq!(game.persist.uuids.get(&id), Some(&uuid));
                });
            }
        }
    }
}

#[cfg(test)]
mod credits_respawn_tests {
    use crate::game::{Audio, Game, Options, Progress, Settings, load_resources};
    use horizon_client::lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig};
    use horizon_protocol::packets::configuration::ClientInformation;
    use std::sync::{Arc, atomic::AtomicI32, mpsc};

    /// `ServerGamePacketListenerImpl.handleClientCommand(PERFORM_RESPAWN)` for a
    /// player who won the game: `restartClientLoadTimerAfterRespawn`, so the
    /// respawned player is not loaded (its actions wait, `isInvulnerableTo`
    /// spares it) until the client says so or 60 ticks pass.
    #[test]
    #[ignore = "daily"]
    fn the_credits_respawn_waits_for_the_client_to_load() {
        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            crate::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
                root.clone(),
            )),
            None,
        )
        .unwrap();
        let scratch = std::env::temp_dir().join(format!(
            "horizon-credits-respawn-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&scratch).unwrap();
        game.world_root = scratch.clone();
        game.level.world_spawn = Some([0, 100, 0]);
        game.lan_host(
            super::super::lan::LanSettings {
                port: 0,
                guest_commands: true,
                force_game_mode: false,
            },
            NetConfig {
                announce: false,
                ..NetConfig::loopback(0, "credits-respawn-test".into(), 2)
            },
            HubConfig {
                default_game_mode: 0,
                guest_commands: true,
                force_game_mode: false,
                hardcore: false,
                view_distance: 2,
                simulation_distance: 2,
                dimension: "minecraft:overworld".into(),
                allow_flight: false,
                seed: 0,
                max_players: 2,
                integrated: false,
                levels: Default::default(),
            },
        )
        .unwrap();
        let (tx, _rx) = mpsc::channel();
        let mut guest = Guest::new(
            Joined {
                conn: 2,
                uuid: 330,
                name: "CreditsGuest".into(),
                properties: Vec::new(),
                info: ClientInformation::new(2),
                brand: "test".into(),
                sender: PlayerSender::new(tx, Arc::default()),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:25565".parse().unwrap(),
                expected: false,
            },
            2,
            [0.5, 100.0, 0.5],
            2,
        );
        guest.dimension = "minecraft:overworld".into();
        guest.saved = Some(horizon_core::nbt::Tag::compound([(
            "NeoForgeData",
            horizon_core::nbt::Tag::compound([
                ("outside", horizon_core::nbt::Tag::Int(1)),
                (
                    "PlayerPersisted",
                    horizon_core::nbt::Tag::compound([("inside", horizon_core::nbt::Tag::Int(2))]),
                ),
            ]),
        )]));
        guest.placed = true;
        guest.loaded = true;
        guest.loading = false;
        guest.load_timeout = 0;
        let mut host = game.lan.take().unwrap();
        host.hub.guests.push(guest);
        let play = host.play.entry(2).or_default();
        play.won_game = true;
        play.respawn_requested = true;
        assert!(host.hub.guests[0].client_loaded());

        game.lan = Some(host);
        game.handle_server_portal(2);
        let host = game.lan.take().unwrap();
        assert!(!host.play[&2].won_game, "the credits are over");
        let guest = &host.hub.guests[0];
        assert_eq!(guest.load_timeout, 60);
        assert!(guest.loading);
        assert!(!guest.client_loaded());
        assert_eq!(
            guest.saved.as_ref().and_then(|tag| tag.get("NeoForgeData")),
            Some(&horizon_core::nbt::Tag::compound([(
                "PlayerPersisted",
                horizon_core::nbt::Tag::compound([("inside", horizon_core::nbt::Tag::Int(2))]),
            )])),
            "the real first-End-exit path drops only data outside PlayerPersisted"
        );
        drop(host);
        drop(game);
        std::fs::remove_dir_all(scratch).unwrap();
    }
}

#[cfg(test)]
#[path = "server_travel/hold_tests.rs"]
mod hold_tests;

#[cfg(test)]
#[path = "server_travel/changed_dimension_tests.rs"]
mod changed_dimension_tests;
