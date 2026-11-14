//! Mobs in the game loop: ticking with entity snapshots, natural and
//! chunk-generation spawning (`NaturalSpawner`), mob events (attacks,
//! breeding, conversions, drops), projectiles, player interactions
//! (feeding, shearing, milking, spawn eggs, throwing) and rendering.
#[cfg(test)]
mod borrowed_panic_tests;
mod borrowed_tick;
mod breeding;
mod combat_adaptations;
mod conversion_identity;
mod death_tail;
mod hoglin_target;
#[cfg(test)]
mod mob_dispatch_panic_tests;
mod player_loot_entity;
mod projectile_targets;
mod projectile_world;
pub(super) use projectile_world::ProjWorld;
mod pufferfish_player;
mod residency;
#[cfg(test)]
mod retained_limits_tests;
mod section_steps;
mod world_view;
use super::api_values::{TryIntoApi, TryIntoEngine};
use super::level_sounds::LevelSound;
use super::piston_push::Lid;
use super::{
    Dim, Game, InteractTarget, entity_culling::EntityCullDecision, native_mob_draw::PlayerRender,
    native_projectiles::Impact, view_vector,
};
use horizon_assets::item_model::DisplayContext;
use horizon_client::actions::Action;
use horizon_client::{
    Result,
    arrows::EntityHit,
    blocks::Fluid,
    collisions::SceneCollisions,
    crafting::Recipes,
    graphics::EntityCullingMode,
    health::{Attack, HealthEvent, HurtSource, Hurting, Knock},
    items::{ItemStack, Items},
    mesher::Vertex,
    mobs::{
        Damage, DamageKind, Mob, MobEvent, MobWorld, PlayerView, Seen, SpawnSetup, Species, Who,
        ai::{eating_sound, eating_sound_is_bound, feed_animal, takes_food},
        entity_sounds::full_id,
        farm::Climate,
        finalize::Group,
        spawning::{self, BiomeFacts},
    },
    particles::{ParticleOptions, ParticleType},
    projectiles::{Hit, Projectile, ProjectileKind, ProjectileWorld, clip_including_border},
};
use horizon_client_render::render_geometry::EntitySpan;
use horizon_core::{
    geometry::{Aabb, Direction, Vec3, traverse_blocks, trig::Mth},
    position::BlockPos,
    random::RandomSource,
};
use mod_api::{EquipmentSlot, LootDrops, SoundSource};
use std::sync::Arc;

/// `AbstractArrow.onHitEntity` when `hurtOrSimulate` returned false.
/// An enderman that teleported (`projectileReceivesSideEffectsOnHit` is
/// false) leaves the arrow in flight. Every other refusal, including
/// `Shulker.hurtServer` while `isClosed()`, is
/// `deflect(ProjectileDeflection.REVERSE, entity, owner, false, 0.2)`.
fn arrow_miss(dodged: bool) -> EntityHit {
    if dodged {
        EntityHit::Dodged
    } else {
        EntityHit::Refused
    }
}
pub(super) type SpawnReason = horizon_client::mobs::spawn_rules::Reason;

/// Whether vanilla calls `finalizeSpawn` for a mob a spawn site makes, and with
/// which reason. The reason is vanilla's `EntitySpawnReason` at that site.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SpawnSite {
    /// Vanilla calls `finalizeSpawn` with this reason: `finalize_spawn` is told.
    Finalizing(SpawnReason),
    /// Vanilla creates the mob without `finalizeSpawn` (a breeding, a block's or
    /// an item's mob): the game finalizes it as before, and no mod is told.
    Plain,
    Raw,
}

/// What a spawn site does with a mob after `finalize_spawn` was heard.
#[derive(Debug, PartialEq)]
pub(super) enum Finalized {
    /// Finalized as vanilla does; the site goes on to the class's follow-ups.
    Done,
    /// A mod skipped finalization: the mob spawns uninitialised.
    Skipped,
    /// A mod stopped the spawn: the mob is not added.
    Cancelled,
}

mod death_source;
mod death_weapon;
mod destroy_rules;
pub(in crate::game) use death_source::death_hurt_source;

/// A dying entity whose items drop (`LivingDropsEvent`).
pub(super) struct DyingDrops<'a> {
    pub(super) entity: u32,
    pub(super) kind: &'a str,
    /// The UUID of a dying player.
    pub(super) player: Option<u128>,
    /// The killing blow; none when no mod listens.
    pub(super) source: Option<HurtSource<'a>>,
    /// Whether a player hurt it within 100 ticks.
    pub(super) recently_hit: bool,
}

/// A sound a native mod queued, sent when the host is free.
pub(super) struct NativeSound {
    pub dimension: &'static str,
    pub sound: String,
    pub at: [f64; 3],
    pub source: i32,
    pub volume: f32,
    pub pitch: f32,
    pub seed: i64,
    /// The connection of a player who doesn't hear it, as
    /// `Level.playSound(except, ...)`.
    pub except: Option<u32>,
    /// The entity the sound follows (`ClientboundSoundEntityPacket`), sent
    /// from `at`, where it was when the mod queued the sound.
    pub entity: Option<i32>,
}

#[derive(Default)]
struct NormalActionFrame {
    events: std::collections::VecDeque<MobEvent>,
    broke: Vec<u32>,
    actor: Option<u32>,
}
#[derive(Default)]
struct NormalMobActions {
    root: NormalActionFrame,
    nested: Vec<NormalActionFrame>,
}
impl NormalMobActions {
    fn new(events: Vec<MobEvent>) -> Self {
        Self {
            root: NormalActionFrame {
                events: events.into(),
                broke: Vec::new(),
                actor: None,
            },
            nested: Vec::new(),
        }
    }
}
struct MobOutputBatch {
    normal: NormalMobActions,
    guests: Vec<(u32, u32, MobEvent)>,
}
// Backpressure applies to producer passes retained behind either producer or
// consumer failure, never to healthy mob counts. Empty delivery resets the quota.
// Sixteen retained passes permit 0.8 seconds of production at the vanilla tick rate.
const MAX_PENDING_MOB_PASSES: usize = 16;
#[derive(Default)]
struct PendingMobBatch {
    batches: std::collections::VecDeque<MobOutputBatch>,
    pushes: horizon_client::mobs::PendingPushes,
    retained_passes: usize,
    refusal_logged: bool,
}

/// Remove the current action before dispatch; retain only the unentered tail on unwind.
pub(super) struct RemainingActions<'a, T> {
    remaining: std::vec::IntoIter<T>,
    target: &'a mut Vec<T>,
}
impl<'a, T> RemainingActions<'a, T> {
    pub(super) fn new(target: &'a mut Vec<T>) -> Self {
        Self {
            remaining: std::mem::take(target).into_iter(),
            target,
        }
    }
}
impl<T> Iterator for RemainingActions<'_, T> {
    type Item = T;
    fn next(&mut self) -> Option<T> {
        self.remaining.next()
    }
}
impl<T> Drop for RemainingActions<'_, T> {
    fn drop(&mut self) {
        let remaining = std::mem::replace(&mut self.remaining, Vec::new().into_iter());
        *self.target = remaining.collect();
    }
}

/// Mob-system state owned by the game.
#[derive(Default)]
pub(super) struct Creatures {
    pub projectiles: horizon_client::join_seq::JoinList<Projectile, false>,
    /// The entities a family tells the mods ticked, kept between ticks
    /// (game/modding/native_ticks.rs).
    pub tick_buffer: Vec<(u32, super::native_ticks::TickKind)>,
    /// Wind bursts from a guest's turn, waiting for it to end (game/interface/wind.rs).
    pub wind_bursts: Vec<super::wind::Burst>,
    /// The wind charge whose burst is being handled, by entity id: which
    /// entities have already ticked in vanilla's spawn-ordered tick list
    /// (game/interface/wind.rs).
    pub wind_source: Option<horizon_client::join_seq::JoinSeq>,
    /// Wind pushes held until the entity's own tick has run, by entity id
    /// (game/interface/wind.rs `apply_late_wind_pushes`).
    pub late_wind_pushes: Vec<(u32, crate::game::Vec3)>,
    /// The players a fishing hook can catch, taken as the hooks tick, and
    /// the pulls on them waiting for the hub (game/items/fishing.rs).
    pub hook_targets: Vec<super::fishing::HookTarget>,
    pub hook_pulls: Vec<(u32, horizon_core::geometry::Vec3)>,
    /// Mace smash knockback on the other players from a guest's turn, waiting
    /// for it to end (game/items/mace.rs).
    pub smash_pushes: Vec<super::mace::SmashPush>,
    /// Sweep attacks from a guest's turn, waiting for it to end
    /// (game/network/lan_pvp.rs).
    pub sweeps: Vec<super::lan_pvp::Sweep>,
    /// Explosion packets from a guest's turn, waiting for it to end
    /// (game/network/lan_particles.rs).
    pub lan_explosions: Vec<super::lan_particles::LanExplosion>,
    /// The snapshot's buffer between ticks, empty: ~400 bytes a mob, so
    /// thousands of mobs would allocate and free megabytes every tick.
    seen_buf: Vec<Seen>,
    /// Per-tick scratch retained between creature ticks.
    event_buf: Vec<MobEvent>,
    pickable_projectile_targets: Vec<(Who, Aabb)>,
    pending: std::collections::HashMap<&'static str, PendingMobBatch>,
    /// The mobs whose own tick ran in the last `tick_creature_entities`:
    /// `ServerLevel.tick` ticks only the entities in entity-ticking range, so
    /// what the passes after the loop do for a mob's `aiStep`, goals or brain
    /// (pickups, item searches) is for these alone.
    pub ticked: horizon_client::fast_hash::FxHashSet<u32>,
    pub deferred_hits: bool,
    /// The wandering trader spawner's clock.
    pub trader: super::wandering_trader::Spawner,
    /// When each player last put a parrot on a shoulder (`timeEntitySatOnShoulder`).
    pub shoulder_time: std::collections::HashMap<[i32; 4], i64>,
    /// The cat spawner's countdown (`CatSpawner.nextTick`).
    pub cat_tick: i32,
    /// Each level's custom spawners, built when its first tick asks (game/mobs/custom_spawners.rs).
    pub custom_spawners: std::collections::HashMap<super::Dim, mod_api::CustomSpawners>,
    /// Native sounds waiting for the server host to be available.
    pub lan_sounds: Vec<NativeSound>,
    /// Particles from a guest's turn, waiting for it to end.
    pub lan_particles: Vec<super::lan_particles::QueuedParticle>,
}
/// `Projectile.isPickable`: a projectile in `#minecraft:redirectable_projectile`
/// (a ghast's fireball and both wind charges) is pickable, so another
/// projectile's `ProjectileUtil.getEntityHitResult` can strike it
/// (`Projectile.onHit` then redirects it). `PICKABLE_PROJECTILE_IDS + j` is
/// `projectiles[j]` during `tick_projectiles`, below the dragon's reserved ids.
const PICKABLE_PROJECTILE_IDS: u32 = super::dragon::SPECIAL - 0x10000;

/// The index of the projectile a `pickable_projectile_hitboxes` id stands for.
pub(super) fn pickable_projectile_hit(id: u32) -> Option<usize> {
    (PICKABLE_PROJECTILE_IDS..super::dragon::SPECIAL)
        .contains(&id)
        .then(|| (id - PICKABLE_PROJECTILE_IDS) as usize)
}

fn pickable_projectile_hitboxes_into(
    projectiles: &[Projectile],
    alive: &[bool],
    skip: usize,
    out: &mut Vec<(Who, Aabb)>,
) {
    out.clear();
    out.extend(
        projectiles
            .iter()
            .enumerate()
            .filter(|(j, q)| *j != skip && alive[*j] && super::deflect::redirectable(q))
            .map(|(j, q)| (Who::Mob(PICKABLE_PROJECTILE_IDS + j as u32), q.aabb())),
    );
}

#[cfg(test)]
fn pickable_projectile_hitboxes(
    projectiles: &[Projectile],
    alive: &[bool],
    skip: usize,
) -> Vec<(Who, Aabb)> {
    let mut out = Vec::new();
    pickable_projectile_hitboxes_into(projectiles, alive, skip, &mut out);
    out
}

/// `FlameFeatureRenderer`: fire quads stacked up a burning entity's box,
/// turned to the camera around Y, at full block light. `at` is the entity's
/// feet relative to the camera.
fn flame_geometry(
    atlas: &horizon_client::hud::HudAtlas,
    at: glam::Vec3,
    width: f32,
    height: f32,
    camera_yaw: f32,
    light: u8,
    out: &mut Vec<Vertex>,
) {
    let (Some(fire0), Some(fire1)) = (
        atlas.sprites.get("block/fire_0").copied(),
        atlas.sprites.get("block/fire_1").copied(),
    ) else {
        return;
    };
    let s = width * 1.4;
    let mut h = height / s;
    // Mth.rotationAroundAxis(Y, camera.orientation): the camera's turn about Y.
    let turn = glam::Mat3::from_rotation_y(std::f32::consts::PI - camera_yaw.to_radians());
    let lift = 0.3 - (h as i32) as f32 * 0.02;
    let packed = (u32::from(light >> 4) * 16) | ((15 * 16) << 8);
    let (mut r, mut yo, mut zo, mut ss) = (0.5f32, 0.0f32, 0.0f32, 0);
    while h > 0.0 {
        let tex = if ss % 2 == 0 { fire0 } else { fire1 };
        let (mut u0, v0, mut u1, v1) = (tex.u0, tex.v0, tex.u1, tex.v1);
        if ss / 2 % 2 == 0 {
            std::mem::swap(&mut u0, &mut u1);
        }
        for (x, y, uv) in [
            (-r, -yo, [u1, v1]),
            (r, -yo, [u0, v1]),
            (r, 1.4 - yo, [u0, v0]),
            (-r, 1.4 - yo, [u1, v0]),
        ] {
            let p = at + turn * glam::Vec3::new(x, y, zo + lift) * s;
            out.push(Vertex {
                normal: [0; 4],
                pos: p.to_array(),
                uv,
                // HDR output: the flames shine like fire (entity.wgsl fs_hdr_mark).
                layer: u32::from(horizon_client::mesher::INCANDESCENT) << 16,
                color: 0xFFFF_FFFF,
                light: packed,

                glint_uv: [0.0, 0.0],
            });
        }
        h -= 0.45;
        yo -= 0.45;
        r *= 0.9;
        zo -= 0.03;
        ss += 1;
    }
}
pub(super) fn seen(m: &Mob, player: Option<[i32; 4]>, items: &Items) -> Seen {
    m.seen(player, items)
}
/// Whether a mob a packet named may take the click: `mob_hit` skips a mob in
/// its death animation, the one the player rides (it shares the player's root
/// vehicle) and the camera entity the pick starts from, so a packet that names
/// one of those does nothing rather than reaching the feed, shear, leash or
/// name paths the pick path refuses.
pub(super) fn named_mob_pickable(
    alive: bool,
    id: u32,
    riding: Option<u32>,
    camera: Option<u32>,
) -> bool {
    alive && Some(id) != riding && Some(id) != camera
}

/// What an entity drew on the GPU path: the skin frame's mark before its draws, and their
/// camera-relative bounds (`cull_skinned`).
pub(super) type Skinned = Option<((usize, usize, u32), Option<(glam::Vec3, glam::Vec3)>)>;

/// What a block's `onProjectileHit` reads of the projectile that struck it:
/// a thrown projectile ([`ProjectileHitter::of`]) or a fishing bobber
/// (`FishingHook.onHitBlock` calls `Projectile.onHitBlock` too).
#[derive(Clone, Copy, Debug)]
pub(super) struct ProjectileHitter {
    /// `Projectile.getOwner`.
    pub(super) owner: Option<Who>,
    /// On a server, the guest (by connection) who owns it.
    pub(super) guest: Option<u32>,
    /// An `AbstractArrow` (a target block resets after 20 ticks, not 8).
    pub(super) arrow: bool,
    /// Its entity type, for `target_hit`'s projectile predicate.
    pub(super) kind: &'static str,
    pub(super) position: Vec3,
}
impl ProjectileHitter {
    pub(super) fn of(p: &Projectile) -> Self {
        Self {
            owner: p.owner,
            guest: p.guest,
            arrow: p.arrow.is_some(),
            kind: p.entity_type(),
            position: p.position,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum OrbOrder {
    BeforeChild,
    AfterChild,
}

impl Game<'_> {
    /// The game's own player's uuid and entity id — the identity behind
    /// [`Self::player_view`], so a tick that lends no guest's view names its
    /// player (#1077). None where the view stands for nobody: no player of
    /// the game's own, a fake one, or a guest's turn acting in its place.
    pub(super) fn player_identity(&self) -> Option<([i32; 4], u32)> {
        if self.lan_acting_fake || self.lan_acting_uuid.is_some() || self.player.is_none() {
            return None;
        }
        let uuid = self.level.player_uuid?;
        let id = u32::try_from(self.player.as_ref()?.entity_id).ok()?;
        Some((uuid, id))
    }
    /// A sulfur cube's attribute map (`LivingEntity.attributes`), from its
    /// type's defaults, before its first tick adopts the body's archetype
    /// modifiers. A map already there (commands, armor) is kept.
    fn init_sulfur_attribute_maps(&mut self) {
        for m in &mut self.mobs {
            if m.species() == horizon_client::mobs::Species::SulfurCube
                && m.extra.armor_attributes.is_none()
            {
                m.extra.armor_attributes = self.motion.attributes.create(&m.kind.id).map(Box::new);
            }
        }
    }
    pub(super) fn player_view(&self) -> Option<PlayerView> {
        if self.lan_acting_fake {
            return None;
        }
        let p = self.player.as_ref()?;
        let w = f64::from(p.dimensions.width) / 2.0;
        let bounds = Aabb::new(
            Vec3::new(p.position.x - w, p.position.y, p.position.z - w),
            Vec3::new(
                p.position.x + w,
                p.position.y + f64::from(p.dimensions.height),
                p.position.z + w,
            ),
        );
        Some(PlayerView {
            position: p.position,
            bounds,
            eye: [
                p.position.x,
                p.position.y + f64::from(p.dimensions.eye_height),
                p.position.z,
            ],
            alive: !p.health.dead_or_dying(),
            invulnerable: p.invulnerable
                || p.health.permanently_invulnerable
                || p.health.invulnerable_time > 0,
            // EntitySelector.NO_CREATIVE_OR_SPECTATOR.
            creative: self.creative || self.spectator(),
            sneaking: self.keys.held(Action::Sneak),
            swimming: p.swimming,
            look: view_vector(self.yaw, self.pitch),
            main_hand: self.held().map(|s| s.item.clone()),
            off_hand: self.offhand.as_ref().map(|s| s.item.clone()),
            head: self.armor[0].as_ref().map(|s| s.item.clone()),
            chest: self.armor[1].as_ref().map(|s| s.item.clone()),
            legs: self.armor[2].as_ref().map(|s| s.item.clone()),
            feet: self.armor[3].as_ref().map(|s| s.item.clone()),
            armor_cover: self.armor.iter().filter(|a| a.is_some()).count() as f32 / 4.0,
            invisible: self.potions.effects.get("invisibility").is_some(),
            spectator: self.spectator(),
            gold_armor: self.wearing_gold(),
            hurt_by: self.player_hurt_by(),
            last_hurt: self.combat.last_hurt_mob.map(|(id, at)| (Who::Mob(id), at)),
            velocity: p.velocity,
            yaw: self.yaw,
            sprinting: p.sprinting,
            riding: self.riding.is_some(),
            hero: self.potions.effects.get("hero_of_the_village").is_some(),
            effects: self.potions.effects.list.iter().map(|e| e.id).collect(),
            uuid: self.acting_uuid(),
            allies: std::sync::Arc::new(self.player_allies()),
            remote: false,
            sleeping: self.sleeping.is_some(),
            sleep_timer: self.sleep_timer,
            passenger: self.riding.is_some(),
            riding_vehicle: self.riding,
            vehicle: self.riding.and_then(|id| {
                if let Some(b) = self.boats.iter().find(|b| b.id == id) {
                    // A boat's box: 1.375 wide and 0.5625 high around its position.
                    let w = 0.6875;
                    return Some(horizon_client::mobs::Vehicle {
                        nautilus: false,
                        bounds: Aabb::new(
                            Vec3::new(b.position.x - w, b.position.y, b.position.z - w),
                            Vec3::new(b.position.x + w, b.position.y + 0.5625, b.position.z + w),
                        ),
                        velocity: b.velocity,
                    });
                }
                let m = self.mobs.iter().find(|m| m.id == id)?;
                matches!(m.species(), Species::Nautilus | Species::ZombieNautilus).then(|| {
                    horizon_client::mobs::Vehicle {
                        nautilus: true,
                        bounds: m.bounds(),
                        velocity: m.velocity,
                    }
                })
            }),
        })
    }
    /// Biome facts at a position for finalizeSpawn and variant rules.
    pub(super) fn biome_facts(&self, x: i32, y: i32, z: i32) -> BiomeFacts {
        let Some(chunk) = self.world.chunks.get(&(x >> 4, z >> 4)) else {
            return BiomeFacts::default();
        };
        let b = world_biomes!(self).get(chunk.biome(x, y, z));
        let tags = self.res.mob_data();
        let has = |t: &str| tags.biome_has(t, &b.name);
        let climate = if has("minecraft:spawns_warm_variant_farm_animals") {
            Climate::Warm
        } else if has("minecraft:spawns_cold_variant_farm_animals") {
            Climate::Cold
        } else {
            Climate::Temperate
        };
        BiomeFacts {
            coral_nautilus: has("minecraft:spawns_coral_variant_zombie_nautilus"),
            climate,
            temperature: b.temperature,
            snowy: has("minecraft:spawns_snow_foxes")
                || b.name.contains("snow")
                || b.name.contains("frozen"),
            snow_fox: has("minecraft:spawns_snow_foxes"),
            swamp: b.name.contains("swamp"),
            ocean: has("minecraft:is_ocean"),
            mushroom: b.name == "minecraft:mushroom_fields",
            desert: b.name == "minecraft:desert",
            jungle: has("minecraft:is_jungle"),
            wolf: horizon_client::mobs::tame::wolf_variant_for(&b.name, &has),
            full_moon: (self.world.day_time / 24000).rem_euclid(8) == 0,
            frog: if has("minecraft:spawns_warm_variant_frogs") {
                1
            } else if has("minecraft:spawns_cold_variant_frogs") {
                2
            } else {
                0
            },
        }
    }
    /// Create a mob of `kind` at a position, finalized like a natural spawn.
    pub(super) fn spawn_mob(
        &mut self,
        kind: &str,
        at: [f64; 3],
        persistent: bool,
        site: SpawnSite,
    ) -> Option<u32> {
        let mob = self.create_mob_reason(kind, at, persistent, site, None)?;
        self.insert_spawned_mob(mob)
    }
    /// Create a mob of `kind` at a position without `finalizeSpawn`'s rolls,
    /// matching vanilla `EntityType.create(level, reason)`.
    pub(super) fn spawn_mob_raw(
        &mut self,
        kind: &str,
        at: [f64; 3],
        persistent: bool,
    ) -> Option<u32> {
        self.spawn_mob(kind, at, persistent, SpawnSite::Raw)
    }
    /// `SummonCommand` and `SpawnEggItem`: vanilla spawns the mob, but it
    /// never calls `Mob.setPersistenceRequired` for it, so the mob is not
    /// persistent: the despawn rule holds it like any other mob, and only
    /// its NBT can make it persistent.
    pub(super) fn spawn_mob_spawned(
        &mut self,
        kind: &str,
        at: [f64; 3],
        site: SpawnSite,
    ) -> Option<u32> {
        // Dragons have a separate entity implementation, but eggs use the same spawn path.
        if matches!(kind, "ender_dragon" | "minecraft:ender_dragon") {
            return self.spawn_dragon_spawned(at, site);
        }
        // The spawn is not a natural one (no jockeys, the patrol leader roll
        // stays), but the mob is not persistent: vanilla's SummonCommand
        // never calls setPersistenceRequired. The flag is false from the
        // start, before the mob joins the level, so a mod's own write in the
        // entity_join hook survives it - clearing it after the join would
        // wipe what the hook set (Mob.setPersistenceRequired from
        // entity_join_level).
        self.spawn_mob(kind, at, false, site)
    }
    fn create_mob_reason(
        &mut self,
        kind: &str,
        at: [f64; 3],
        persistent: bool,
        site: SpawnSite,
        fixed_yaw: Option<f32>,
    ) -> Option<Mob> {
        let kind = if kind.contains(':') {
            kind.to_owned()
        } else {
            format!("minecraft:{kind}")
        };
        let t = self.res.mob_types().get(&kind).cloned()?;
        self.next_entity += 1;
        // Yaw belongs to the vanilla creation path, independently of whether
        // the fork's SpawnSite allows finalizeSpawn (and its hook).
        let yaw = match (fixed_yaw, site) {
            (Some(yaw), _) => yaw,
            (None, SpawnSite::Finalizing(SpawnReason::Command)) => 0.0,
            (None, SpawnSite::Finalizing(SpawnReason::SpawnItemUse | SpawnReason::Dispenser)) => {
                horizon_client::camera::wrap_degrees(self.world.random.next_float() * 360.0)
            }
            _ => self.world.random.next_float() * 360.0,
        };
        // Entity.random first supplies the UUID, then LivingEntity's initial
        // yaw; species constructor draws continue from that same stream.
        let (own, uuid) = horizon_client::mobs::living_random();
        self.persist.uuids.insert(self.next_entity, uuid);
        let mut mob = Mob::with_random(
            self.next_entity,
            t,
            Vec3::new(at[0], at[1], at[2]),
            yaw,
            own,
        );
        // `EntityType.create`: the mob is constructed before `finalizeSpawn`.
        self.initialize_entity(mod_api::Holder::Entity(mob.id), &kind);
        mob.persistent = persistent;
        match site {
            SpawnSite::Raw => return Some(mob),
            SpawnSite::Finalizing(reason) => match self.finalize_with(&mut mob, reason, None) {
                Finalized::Cancelled => return self.discard_unjoined_mob(mob.id),
                Finalized::Skipped => return Some(mob),
                Finalized::Done => {}
            },
            // A plain spawn finalizes as a summoned one (vanilla's /summon
            // and egg path): its few reason-gated arms behave the same.
            SpawnSite::Plain => {
                let difficulty =
                    self.local_difficulty([mob.position.x, mob.position.y, mob.position.z]);
                self.finalize_plain(
                    &mut mob,
                    &difficulty,
                    horizon_client::mobs::spawn_rules::Reason::MobSummoned,
                );
            }
        }
        Some(mob)
    }

    fn insert_spawned_mob(&mut self, mob: Mob) -> Option<u32> {
        let id = mob.id;
        self.mobs.push(mob);
        self.native_mobs_joined(self.mobs.len() - 1, false);
        if !self.mobs.iter().any(|m| m.id == id) {
            return None;
        }
        Some(id)
    }
    /// `Mob.burnUndead` for a mob in daylight with an item on its sun
    /// protection slot (the head, or the body for a zombie horse or
    /// nautilus): `hurtAndBreak(random.nextInt(2))` on a damageable item, and
    /// when it breaks the slot empties and `onEquippedItemBroken` broadcasts
    /// the break event.
    fn wear_sun_protection(&mut self, id: u32) {
        use horizon_core::equipment_slot::EquipmentSlot;
        let Some(m) = self.mobs.iter().find(|m| m.id == id) else {
            return;
        };
        let slot = m.sun_protection_slot();
        let body = slot == EquipmentSlot::Body;
        let Some(stack) = m.item_by_slot(slot) else {
            return;
        };
        if !horizon_client::stations::is_damageable(self.res.items(), &stack) {
            return;
        }
        let at = [m.position.x, m.position.y, m.position.z];
        let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) else {
            return;
        };
        let amount = m.random.next_int_bound(2).unwrap_or(0) as u32;
        if amount == 0 {
            return;
        }
        let (updated, broken) = self.damage_living_stack(&stack, amount, id);
        let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) else {
            return;
        };
        if m.item_by_slot(slot).as_ref() != Some(&stack) {
            return;
        }
        m.set_equipment_stack(slot as usize, updated);
        if broken.is_some() {
            // LivingEntity.entityEventForEquipmentBreak: head 49, body 65.
            self.lan_entity_event(id as i32, if body { 65 } else { 49 }, at);
        }
    }
    /// `LivingEntity.updateFallFlying`'s wear, `ItemStack.hurtAndBreak(1, this,
    /// slot)` on the glider `MobEvent::GliderWear` picked: Unbreaking (from the
    /// level's random, `Enchantment.modifyItemFilteredCount`) may save it, and
    /// the rest goes on the stack where it is worn. Only a part's `break_now`
    /// breaks one (`canGlideUsing` picks none a point would break): emptied, with its event.
    pub(in crate::game) fn wear_mob_glider(
        &mut self,
        id: u32,
        slot: horizon_core::equipment_slot::EquipmentSlot,
    ) {
        let Some(stack) = self
            .mobs
            .iter()
            .find(|m| m.id == id)
            .and_then(|m| horizon_client::mobs::finalize::equipment_stack(m, slot))
        else {
            return;
        };
        if !horizon_client::stations::is_damageable(self.res.items(), &stack) {
            return;
        }
        let (updated, broken) = self.damage_living_stack(&stack, 1, id);
        let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) else {
            return;
        };
        let at = [m.position.x, m.position.y, m.position.z];
        m.set_equipment_stack(slot as usize, updated);
        if broken.is_some() {
            let event = horizon_client::mobs::item_break::equipment_break_event(slot);
            self.lan_entity_event(id as i32, event, at);
        }
    }
    /// `LivingEntity.hurtArmor` -> `LivingEntity.doHurtEquipment`, from the hit
    /// `MobEvent::ArmorHurt` reported. Only `Horse` overrides `hurtArmor` for a
    /// mob, and its override asks for the BODY slot alone, so the body piece is
    /// the only one this wears: it loses `max(1, damage / 4)` durability,
    /// Unbreaking applied, and at its maximum the slot empties and the break
    /// event goes out (`LivingEntity.entityEventForEquipmentBreak`: 65 for the
    /// body — 49 to 52 are the four armour slots, which only a player's own
    /// pieces reach). A fire hit leaves a fire-resistant piece alone
    /// (`ItemStack.canBeHurtBy`).
    fn hurt_mob_armor(&mut self, id: u32, damage: f32, source: &HurtSource<'_>) {
        use horizon_core::equipment_slot::EquipmentSlot;
        if damage <= 0.0 {
            return;
        }
        let Some(m) = self.mobs.iter().find(|m| m.id == id) else {
            return;
        };
        let at = [m.position.x, m.position.y, m.position.z];
        let amount = ((damage / 4.0) as u32).max(1);
        let name = source.damage_type;
        // A piece's maximum, or none when it may not lose durability at all.
        // `doHurtEquipment` starts from the item's `equippable` component
        // (`Equippable equippable = item.get(DataComponents.EQUIPPABLE)`), so a
        // damageable item with no such component — a sword put in a horse's body
        // slot by `/item replace` — never wears; `equip_damage_on_hurt` alone
        // cannot say that, since it defaults to true when the component is absent.
        fn wearable(me: &Game<'_>, stack: &ItemStack, name: &str) -> Option<u32> {
            let info = me.res.items().get(&stack.item)?;
            if info.equip_slot.is_none()
                || !horizon_client::item_sync::damage_on_hurt(stack, info.equip_damage_on_hurt)
                || info.max_damage == 0
                || !horizon_client::stations::is_damageable(me.res.items(), stack)
                || !me.stack_can_be_hurt_by(stack, name).unwrap_or(true)
            {
                return None;
            }
            Some(info.max_damage)
        }
        let Some(body) = self
            .mobs
            .iter()
            .find(|m| m.id == id)
            .and_then(|m| m.item_by_slot(EquipmentSlot::Body))
        else {
            return;
        };
        if wearable(self, &body, name).is_none() {
            return;
        }
        // ArmorHurtEvent for a mob's body armor.
        let mut amount = amount;
        if mod_api::__private::listened(mod_api::HookId::ArmorHurt)
            && let Ok(body_snapshot) = body.try_into_api()
        {
            let kind = self
                .mobs
                .iter()
                .find(|m| m.id == id)
                .map(super::native_hooks::mob_kind)
                .unwrap_or_default();
            let mut entries = vec![mod_api::ArmorHurt {
                slot: EquipmentSlot::Body,
                stack: body_snapshot,
                damage: super::durability_damage(damage),
                new_damage: super::durability_damage(damage),
            }];
            let mut cancel = false;
            self.native_armor_hurt(None, id, &kind, source, &mut entries, &mut cancel);
            if cancel {
                return;
            }
            amount = super::answered_wear(&entries, EquipmentSlot::Body, amount);
            if amount == 0 {
                return;
            }
            // A mod may have swapped the piece: it is not worn by this hit.
            if self
                .mobs
                .iter()
                .find(|m| m.id == id)
                .and_then(|m| m.item_by_slot(EquipmentSlot::Body))
                != Some(body.clone())
            {
                return;
            }
        }
        let (updated, broken) = self.damage_living_stack(&body, amount, id);
        let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) else {
            return;
        };
        if m.item_by_slot(EquipmentSlot::Body).as_ref() != Some(&body) {
            return;
        }
        m.set_equipment_stack(EquipmentSlot::Body as usize, updated);
        if broken.is_some() {
            // LivingEntity.onEquippedItemBroken through the level's broadcast.
            let event =
                horizon_client::mobs::item_break::equipment_break_event(EquipmentSlot::Body);
            self.lan_entity_event(id as i32, event, at);
        }
    }
    /// Finalize a converted frog or witch before it joins the level.
    fn finalize_converted(&mut self, mob: &mut Mob) {
        // A mod that stops a conversion's finalization leaves the mob as converted.
        self.finalize_with(mob, SpawnReason::Conversion, None);
        mob.persistent = true;
    }
    /// `Mob.convertTo(type, ConversionParams.single(...))`: a new mob of
    /// `kind`, made without spawn finalization unless its conversion asks, takes
    /// its place with what `conversion::single` carries over, and the old
    /// mob's riders move onto it. A zombie that comes out of it rolls
    /// `Zombie.handleAttributes(special, CONVERSION)` (the
    /// `postConversionStep` of its `ConversionTracker`).
    pub(super) fn convert_to(
        &mut self,
        from: &Mob,
        kind: &str,
        params: horizon_client::mobs::conversion::Params,
        after: impl FnOnce(&mut Self, &mut Mob),
    ) -> Option<u32> {
        // Tick conversions can have been removed from the tick list before
        // their queued event is handled. Restore them until Post has returned.
        match self.mobs.iter_mut().find(|m| m.id == from.id) {
            Some(old) => old.removed = false,
            None => {
                let mut old = from.clone();
                old.removed = false;
                self.mobs.push(old);
            }
        }
        let t = self.res.mob_types().get(kind).cloned()?;
        let mut cancel = false;
        let from_kind = (mod_api::__private::listened(mod_api::HookId::LivingConversionPre)
            || mod_api::__private::listened(mod_api::HookId::LivingConversionPost))
        .then(|| super::native_hooks::mob_kind(from));
        if let Some(from_kind) = &from_kind {
            self.native_living_conversion_pre(from.id, from_kind, kind, &mut cancel);
        }
        if cancel {
            return None;
        }
        let from = self.mobs.iter().find(|m| m.id == from.id)?.clone();
        self.next_entity += 1;
        let mut mob = Mob::new(self.next_entity, t, from.position, from.yaw);
        horizon_client::mobs::conversion::single(&from, &mut mob, params);
        self.transfer_conversion_identity(from.id, mob.id);
        if matches!(from.species(), Species::Zombie | Species::Husk) {
            let at = [from.position.x, from.position.y, from.position.z];
            let special = self.local_difficulty(at).special();
            horizon_client::mobs::finalize::zombie_attributes(
                &mut mob,
                special,
                horizon_client::mobs::spawn_rules::Reason::Conversion,
            );
        }
        match (from.species(), kind) {
            (Species::Tadpole, "minecraft:frog") | (Species::Villager, "minecraft:witch") => {
                self.finalize_converted(&mut mob);
            }
            (Species::Pig, "minecraft:zombified_piglin") => {
                let old = self.mobs.iter_mut().find(|m| m.id == from.id)?;
                horizon_client::mobs::conversion::pig_thunder_hit(old, &mut mob);
            }
            (Species::Hoglin, "minecraft:zoglin")
            | (Species::Piglin | Species::PiglinBrute, "minecraft:zombified_piglin") => {
                mob.add_effect("minecraft:nausea", 200, 0);
            }
            _ => {}
        }
        after(self, &mut mob);
        let id = mob.id;
        self.mobs.push(mob);
        // ConversionType.SINGLE: `to.startRiding(vehicle, false, false)`.
        self.seat_mob(id);
        self.copy_conversion_attachments(from.id, id, kind);
        self.native_mobs_joined(self.mobs.len() - 1, false);
        let transformed = self.mobs.iter().find(|m| m.id == id)?;
        let transformed_kind = from_kind.as_ref().map(|_| transformed.kind.id.clone());
        // Only move passengers once the new mob's join has succeeded.
        for rider in self.mobs.iter_mut() {
            if rider.extra.mount == Some(from.id) {
                rider.extra.mount = Some(id);
            }
        }
        if let (Some(from_kind), Some(transformed_kind)) = (from_kind, transformed_kind) {
            self.native_living_conversion_post(from.id, &from_kind, id, &transformed_kind);
        }
        if let Some(old) = self.mobs.iter_mut().find(|m| m.id == from.id) {
            old.removed = true;
        }
        Some(id)
    }
    /// Default position for a summon command (3 blocks in front of the player).
    pub(super) fn default_summon_position(&self) -> Result<[f64; 3]> {
        let p = self.player.as_ref().ok_or("no player yet")?;
        let d = view_vector(self.yaw, 0.0);
        Ok([
            p.position.x + d[0] * 3.0,
            p.position.y,
            p.position.z + d[2] * 3.0,
        ])
    }
    /// The `summon` command: spawn a mob in front of the player or at a position.
    pub(super) fn summon_command_mob(
        &mut self,
        kind: &str,
        at: Option<[f64; 3]>,
        baby: bool,
    ) -> Result<u32> {
        let at = match at {
            Some(a) => a,
            None => self.default_summon_position()?,
        };
        let id = self
            .spawn_mob_spawned(kind, at, SpawnSite::Finalizing(SpawnReason::Command))
            .ok_or_else(|| format!("unknown or unsupported mob {kind}"))?;
        if baby && let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
            m.extra.age = if m.species() == Species::Sniffer {
                horizon_client::mobs::sniffer::BABY_START_AGE
            } else {
                -24000
            };
        }
        Ok(id)
    }
    /// The `mobs` command: nearby mobs with position, health and running goals.
    pub(super) fn control_mobs(&self, radius: f64) -> serde_json::Value {
        let center = self.player.as_ref().map_or(Vec3::ZERO, |p| p.position);
        self.control_mobs_at(radius, center)
    }
    pub(super) fn control_mobs_at(&self, radius: f64, center: Vec3) -> serde_json::Value {
        let list: Vec<serde_json::Value> = self
            .mobs
            .iter()
            .filter(|m| m.distance_sqr(center) <= radius * radius)
            .map(|m| {
                let mut state = serde_json::json!({
                    "id": m.id,
                    "kind": m.kind.id,
                    "pos": [m.position.x, m.position.y, m.position.z],
                    "health": m.health,
                    "baby": m.is_baby(),
                    "target": m.ai.target.map(|t| format!("{t:?}")),
                    "goals": m.ai.goals.running_names(),
                    "held": m.extra.main_hand_id(),
                    "mount": m.extra.mount,
                    "rider": m.extra.rider,
                    // A player's input this tick, and whether it steers.
                    "ridden": m.rider.map(|r| r.steer),
                    "controlled": horizon_client::mobs::equine::controlled(m),
                    "saddled": m.extra.saddled(),
                    "enchants": horizon_client::mobs::gear::gear(m).map(|g| g.slots.iter().map(|s| s.iter().map(|(e, l)| format!("{e}:{l}")).collect::<Vec<_>>()).collect::<Vec<_>>()),
                    "ints": m.extra.ints,
                    "pois": [m.extra.home, m.extra.bed, m.extra.meeting],
                    "profession": m.extra.profession.index(),
                    "targets": m.ai.targets.running_names(),
                    "path": m.ai.nav.path.as_ref().map(|p| p.nodes.len() - p.next.min(p.nodes.len())),
                    "on_ground": m.on_ground,
                    "variant": m.extra.variant,
                    "color": m.extra.color,
                    "sheared": m.extra.sheared,
                    "burning": m.fire_ticks > 0,
                    "size": m.extra.size,
                    "leash": m.extra.leash.map(|h| format!("{h:?}")),
                    "tame": m.extra.tame,
                    "sitting": m.extra.sitting,
                    "temper": if horizon_client::mobs::equine::is_equine(m.species()) {
                        horizon_client::mobs::equine::temper(m)
                    } else { i32::from(m.extra.variant3) },
                    "chested": m.extra.chested,
                });
                // Set apart: one json! with every key goes past the default macro recursion
                // limit, and horizon-client can't raise it (#8169).
                state["head"] = m.extra.head_id().into();
                state["body"] = m.extra.body_id().into();
                state["warden"] = serde_json::json!(horizon_client::mobs::warden::describe(m));
                state["villager"] = serde_json::json!(horizon_client::mobs::villager::ext(m).map(|e| serde_json::json!({
                    "food_level": e.food_level,
                    "inventory": e.inventory.iter().map(|s| format!("{}x{}", s.count, s.item)).collect::<Vec<_>>(),
                    "gossips": e.gossips.len(),
                    "last_slept": e.last_slept,
                    "raid": e.raid,
                })));
                state["raider"] = serde_json::json!(horizon_client::mobs::raider::state(m).map(|s| serde_json::json!({
                    "raid": s.raid_id,
                    "wave": s.wave,
                    "leader": s.patrol_leader,
                    "patrolling": s.patrolling,
                    "patrol_target": s.patrol_target,
                    "vehicle": m.extra.mount,
                })));
                // Drawn seated (Entity.isPassenger).
                state["passenger"] = m.extra.passenger.into();
                state["guardian_beam"] = serde_json::json!(horizon_client::mobs::guardian::beam(m, 0.0));
                state
            })
            .collect();
        serde_json::json!({ "count": self.mobs.len(), "near": list, "projectiles": self.creatures.projectiles.len() })
    }
    /// Highest motion-blocking (no leaves) block + 1 (Heightmap MOTION_BLOCKING_NO_LEAVES).
    pub(super) fn motion_top(&self, x: i32, z: i32) -> i32 {
        let top = if self.dim == Dim::Nether { 127 } else { 319 };
        for y in (self.world.bottom_y()..=top).rev() {
            let f = self.res.facts().get(self.world.block(x, y, z));
            if (f.full_collision || f.fluid != Fluid::None) && !f.name.ends_with("leaves") {
                return y + 1;
            }
        }
        self.world.bottom_y()
    }
    /// What the generator finalizes this level's structure mobs with.
    pub(super) fn share_spawn_conditions(&self) {
        if let Some(terrain) = self.end.locator() {
            terrain.set_spawn_conditions(self.difficulty_id() as u8, self.world.day_time);
        }
    }
    /// `Mob.finalizeSpawn` behind `FinalizeSpawnEvent`: the one fire point for
    /// the mobs the game spawns where vanilla calls `finalizeSpawn`. The class's
    /// follow-ups (raider flags and the like) are part of `finalizeSpawn`, so a
    /// site skips them on [`Finalized::Skipped`].
    pub(super) fn finalize_with(
        &mut self,
        mob: &mut Mob,
        reason: SpawnReason,
        spawner: Option<mod_api::SpawnSource>,
    ) -> Finalized {
        let mut group = None;
        let (outcome, difficulty) = self.finalize_spawn_event(mob, reason, spawner, &mut group);
        if outcome == Finalized::Done {
            self.finalize_grouped(mob, &difficulty, reason, group);
        }
        outcome
    }
    /// `Mob.finalizeSpawn`'s event, the half listeners own: they hear the
    /// spawn, cancel it, take it over, set its difficulty, edit the mob
    /// through its lend and change the pack's `group`, which finalization then
    /// reads. The decision and the difficulty the spawn finalizes with come
    /// back together.
    pub(super) fn finalize_spawn_event(
        &self,
        mob: &mut Mob,
        reason: SpawnReason,
        spawner: Option<mod_api::SpawnSource>,
        group: &mut Option<Group>,
    ) -> (Finalized, spawning::LocalDifficulty) {
        let at = [mob.position.x, mob.position.y, mob.position.z];
        let difficulty = self.local_difficulty(at);
        if !mod_api::__private::listened(mod_api::HookId::FinalizeSpawn) {
            return (Finalized::Done, difficulty);
        }
        let kind = mob.kind.id.clone();
        self.finalize_spawn_lent(&kind, at, reason, spawner, difficulty, Some(mob), group)
    }
    /// `Mob.getMaxSpawnClusterSize` as the entity parts of the finalized `mob`'s type answer it
    /// (`EntityPart::spawn_cluster_size`), asked with `mob` lent as it stands before it joins the
    /// level: the first part in dispatch order to answer wins, and none answering leaves vanilla's
    /// `vanilla` to the spawner. A type with no part costs one table lookup. Server-side.
    pub(super) fn spawn_cluster_size(
        &self,
        mob: &mut Mob,
        reason: SpawnReason,
        spawner: Option<mod_api::SpawnSource>,
        vanilla: u32,
    ) -> Option<u32> {
        if !mod_api::__private::has_entity_parts() {
            return None;
        }
        let parts = mod_api::__private::entity_parts();
        let kind = mob.kind.id.clone();
        let list = parts.entity_parts(&kind);
        if list.is_empty() {
            return None;
        }
        let at = [mob.position.x, mob.position.y, mob.position.z];
        self.lend_unspawned(&kind, at, Some(mob), |lent, _| {
            list.iter().find_map(|attached| {
                let _guard =
                    mod_api::content::crash::enter_native_mod(mod_api::content::crash::ModCall {
                        id: attached.mod_id(),
                        hook: "spawn_cluster_size",
                        part: Some(attached.type_name()),
                    });
                attached
                    .part()
                    .spawn_cluster_size(self, lent, reason, spawner, vanilla)
            })
        })
    }
    /// The shared finalization decision for a mob type the game keeps apart
    /// from mobs (the dragon), before the entity joins the level: its lend has
    /// a kind and a position and no mob.
    pub(super) fn finalize_spawn_at(
        &self,
        kind: &str,
        at: [f64; 3],
        reason: SpawnReason,
        spawner: Option<mod_api::SpawnSource>,
    ) -> (Finalized, spawning::LocalDifficulty) {
        let difficulty = self.local_difficulty(at);
        if !mod_api::__private::listened(mod_api::HookId::FinalizeSpawn) {
            return (Finalized::Done, difficulty);
        }
        self.finalize_spawn_lent(kind, at, reason, spawner, difficulty, None, &mut None)
    }
    /// Fires `finalize_spawn` with `mob` lent to it and the group data as the
    /// API's; a group the listener changed comes back as the game's.
    #[allow(clippy::too_many_arguments)]
    fn finalize_spawn_lent(
        &self,
        kind: &str,
        at: [f64; 3],
        reason: SpawnReason,
        spawner: Option<mod_api::SpawnSource>,
        difficulty: spawning::LocalDifficulty,
        mob: Option<&mut Mob>,
        group: &mut Option<Group>,
    ) -> (Finalized, spawning::LocalDifficulty) {
        let mut heard = mod_api::Difficulty::from(difficulty);
        let (mut spawn_cancelled, mut skip) = (false, false);
        let given = group
            .as_ref()
            .and_then(|g| super::native_unspawned_mob::native_spawn_group(g, self.lend_epoch()));
        let mut native_group = given.clone();
        self.lend_unspawned(kind, at, mob, |mob, edits| {
            self.native_finalize_spawn(mod_api::__private::FinalizeSpawnCall {
                dimension: self.dim.id(),
                kind,
                reason,
                pos: at,
                spawner,
                difficulty: &mut heard,
                spawn_cancelled: &mut spawn_cancelled,
                mob,
                edits,
                group: &mut native_group,
                cancel: &mut skip,
            });
        });
        if native_group != given {
            *group = native_group
                .as_ref()
                .and_then(super::native_unspawned_mob::spawn_group);
        }
        if spawn_cancelled {
            return (Finalized::Cancelled, difficulty);
        }
        if skip {
            return (Finalized::Skipped, difficulty);
        }
        (Finalized::Done, heard.into())
    }
    /// `ServerLevel.getCurrentDifficultyAt` in the level in place.
    pub(super) fn local_difficulty(&self, at: [f64; 3]) -> spawning::LocalDifficulty {
        local_difficulty_in(&self.world, at)
    }
    /// Generation's animals for the chunks generated since, once lit: chunk
    /// generation places them whether the level ticks or not.
    pub(super) fn spawn_fresh_chunks(&mut self) {
        if !self.world.chunks.has_fresh() {
            return;
        }
        let fresh: Vec<(i32, i32)> = self
            .world
            .chunks
            .fresh_keys_ordered()
            .into_iter()
            .filter(|k| self.world.chunks.get(k).is_some_and(|c| c.fresh && c.lit))
            .collect();
        for key in fresh {
            if let Some(c) = self.world.chunks.get_mut(&key) {
                Arc::make_mut(c).fresh = false;
            }
            self.world.chunks.remove_fresh(&key);
            self.spawn_generation_creatures(key.0, key.1);
        }
    }
    /// Advance every mob one tick, then apply their events.
    pub(super) fn tick_creatures(&mut self) -> Result<()> {
        // Hurts dealt since the last creature tick (explosions, commands, the
        // systems after it) teleport an enderman before it ticks, not in its AI step.
        self.settle_hurt_teleports()?;
        self.attack_ticks = self.attack_ticks.saturating_add(1);
        self.refresh_gear_knockback_resistance();
        self.tick_leads();
        // Vanilla's spawning and raids run before ChunkMap.tick.
        self.tick_spawners();
        let ticked =
            horizon_client::spans::time("tick_creature_entities", || self.tick_creature_entities());
        self.dismount_underwater_mob_riders();
        ticked
    }
    /// `DistanceManager.inEntityTickingRange` in an entity-ticking chunk
    /// (block coordinates), from every player's tickets (`update_chunk_tickets`).
    pub(super) fn entity_ticking_area(&self) -> impl Fn(i32, i32) -> bool + use<> {
        let levels = self.tickets.levels();
        move |x, z| levels.entity_ticking_at(x, z)
    }
    /// The entity phase of `tick_creatures`: every mob's tick, deaths, and the
    /// projectiles and effects that follow from them.
    pub(super) fn tick_creature_entities(&mut self) -> Result<()> {
        // Its steps as spans for `horizon-report` (nothing is read while spans are off).
        let mut laps = horizon_client::spans::laps();
        let blocked = self.mobs_below_off_level();
        let level_id = self.dim.id();
        if let Some(pending) = self.creatures.pending.get_mut(level_id)
            && (pending.refusal_logged || pending.retained_passes >= MAX_PENDING_MOB_PASSES)
        {
            if !pending.refusal_logged {
                log::warn!(
                    "Refusing mob production in {level_id}: {} retained producer passes (limit {MAX_PENDING_MOB_PASSES}); draining before retry",
                    pending.retained_passes
                );
                pending.refusal_logged = true;
            }
            // No new producer work until the backlog drains. Unrelated projectile
            // phases still advance; mob-specific follow-up must not reuse old ticks.
            self.creatures.ticked.clear();
            let primable = self.sulfur_cubes_adopted_explosive();
            self.deliver_pending_mob_actions(level_id)?;
            return self.finish_creature_entities(&mut laps, &primable);
        }
        // `EntityEvent.Size` for the sizes that changed outside the mob ticks (spawns,
        // commands, a player's interaction), before the ticks move by them.
        self.native_mob_sizes();
        // LivingEntity.die runs when the killing blow lands: mobs killed
        // since the last tick (by the player, explosions...) die first.
        let killed: Vec<Mob> = self
            .mobs
            .iter()
            .filter(|m| !m.alive() && !m.removed && !self.died.contains(&m.id))
            .cloned()
            .collect();
        for m in killed {
            self.died.insert(m.id);
            self.mob_died(&m);
        }
        // Their victims have been looted and mourned: no charged creeper is
        // waiting any more.
        // Resolve saved attacker UUIDs before the mobs run baseTick.
        self.resolve_last_hurt_by_refs();
        self.combat.charged_exploders.clear();
        self.combat.exploded.clear();
        self.combat.exploded_kinds.clear();
        // EntityTickEvent.Pre for the mobs about to tick, before the loop holds them.
        let mut round = self.native_tick_round();
        if round.listening() || mod_api::__private::listened(mod_api::HookId::CanContinueSleeping) {
            let ticking = self.entity_ticking_area();
            let mut i = 0;
            while let Some(m) = self.mobs.get(i) {
                let (x, z) = (m.position.x.floor() as i32, m.position.z.floor() as i32);
                if !blocked.contains(&m.id) && self.world.loaded(x, z) && ticking(x, z) {
                    let (id, kind) = (m.id, super::native_ticks::TickKind::Mob(m.kind.clone()));
                    if self.native_tick_pre_batch(&mut round, id, kind) {
                        self.sync_sleeping_mob(i);
                    }
                }
                i += 1;
            }
        }
        self.init_sulfur_attribute_maps();
        let uuid = self.level.player_uuid;
        // LivingEntity.isPushable reads onClimbable live; a mob's cached one
        // is only refreshed by its own push step, so one that has not ticked
        // yet (the first tick after a load, a fresh spawn) or has moved since
        // would be pushed out of a ladder or scaffolding. Refreshed here, from
        // where each mob stands now, before the snapshot others push it by.
        let climbing: Vec<bool> = {
            let w = self.mob_world_base(&[]);
            self.mobs.iter().map(|m| m.on_climbable(&w)).collect()
        };
        for (m, climbing) in self.mobs.iter_mut().zip(climbing) {
            m.on_climbable = climbing;
        }
        let mut snapshot = std::mem::take(&mut self.creatures.seen_buf);
        snapshot.extend(self.mobs.iter().map(|m| seen(m, uuid, self.items())));
        let mut events = std::mem::take(&mut self.creatures.event_buf);
        events.clear();
        let passenger_objects = self.retained_passenger_snapshot(uuid);
        let mut pending = self.creatures.pending.remove(level_id).unwrap_or_default();
        let primable = self.sulfur_cubes_adopted_explosive();
        let mut mobs = self.mobs.lift();
        #[cfg(test)]
        super::entity::taken_pass_tests::after_take(self);
        let mut dead = Vec::new();
        let mut corpses = Vec::new();
        let mut died = std::mem::take(&mut self.died);
        let mut ticked = std::mem::take(&mut self.creatures.ticked);
        ticked.clear();
        // With guests on, each mob sees its nearest player (game/network/lan_mobs.rs).
        let mut guests = self.lan_guest_views();
        // What stands in a guest's slot while a mob holds that guest's view for
        // its tick (see `lent` below).
        let mut spare = guests.first().map(|g| g.1.clone());
        let mut bindings = match self.lan.as_mut() {
            Some(host) if !guests.is_empty() => std::mem::take(&mut host.mob_players),
            _ => Default::default(),
        };
        // The guests' entity ids by connection, for the shulker bullets fired
        // at the guest a mob sees.
        let mut guest_ids: Vec<(u32, u32)> = match self.lan.as_ref() {
            Some(host) if !guests.is_empty() => host
                .hub
                .guests
                .iter()
                .map(|g| (g.conn, g.entity_id as u32))
                .collect(),
            _ => Vec::new(),
        };
        let mut guest_events = Vec::new();
        let mut player_credit = std::mem::take(&mut self.last_hurt_by_player);
        // The block writes since the last tick: mobs re-path around them.
        let mut path_writes = std::mem::take(self.world.path_writes.get_or_insert_with(Vec::new));
        // The mobs whose bodies go this tick: (id, feet, width, height, split).
        // 🤖 Agent: the last field is the grown slime, magma cube or sulfur
        // cube the body leaves behind (`AbstractCubeMob.remove`), captured as
        // the body goes because it has left the list by the time it splits.
        let mut gone = Vec::new();
        // The level's random and attachments, lent to the mobs (brain behaviours draw from
        // the random, and a hit's damage hooks get both) and taken back after their ticks.
        let lent = horizon_client::health::LentLevel::take(&mut self.world);
        let (lent, dispatch_panic) = self.tick_borrowed_creatures(borrowed_tick::BorrowedMobTick {
            mobs: &mut mobs,
            snapshot: &mut snapshot,
            passenger_objects: &passenger_objects,
            blocked: &blocked,
            events: &mut events,
            pushes: &mut pending.pushes,
            dead: &mut dead,
            corpses: &mut corpses,
            died: &mut died,
            ticked: &mut ticked,
            guests: &mut guests,
            spare: &mut spare,
            bindings: &mut bindings,
            guest_ids: &mut guest_ids,
            guest_events: &mut guest_events,
            player_credit: &mut player_credit,
            path_writes: &mut path_writes,
            gone: &mut gone,
            lent,
            laps: &mut laps,
            uuid,
            level_id,
            round: &round,
        });
        if dispatch_panic.is_some() {
            // The aborted pass never applies retention, including previously
            // finished despawn answers. Do not carry those into the retry.
            for m in &mut mobs {
                m.despawn = mod_api::Despawn::default();
            }
        }
        lent.give_back(&mut self.world);
        self.last_hurt_by_player = player_credit;
        if let Some(host) = self.lan.as_mut()
            && !guests.is_empty()
        {
            // A set: a scan of every mob for each binding is quadratic, and a
            // server's loaded mobs run into the thousands.
            let live: horizon_client::fast_hash::FxHashSet<u32> =
                mobs.iter().map(|m| m.id).collect();
            bindings.retain(|id, _| live.contains(id));
            host.mob_players = bindings;
        }
        // The mobs added during the tick (into the emptied `self.mobs`) come
        // first, as `append` put them; with none, the ticked list goes back
        // as it is instead of being copied into a new one, every mob a tick.
        self.mobs
            .restore(mobs, horizon_client::join_seq::RestoreOrder::SpawnsFirst);
        if dispatch_panic.is_some() {
            // These death callbacks have not run, and the borrowed block writes
            // must still invalidate paths on the next attempted pass.
            for mob in &dead {
                died.remove(&mob.id);
            }
            self.world
                .path_writes
                .get_or_insert_with(Vec::new)
                .extend(path_writes);
        }
        self.died = died;
        snapshot.clear();
        self.creatures.seen_buf = snapshot;
        self.creatures.ticked = ticked;
        // Completed brain actions are never fed back into the producer pass.
        if !events.is_empty() || !guest_events.is_empty() {
            pending.batches.push_back(MobOutputBatch {
                normal: NormalMobActions::new(std::mem::take(&mut events)),
                guests: std::mem::take(&mut guest_events),
            });
        }
        // A successful producer can still be retained behind a failing consumer.
        // Count before any downstream callbacks; fully empty delivery resets it.
        pending.retained_passes += 1;
        if dispatch_panic.is_none() {
            // The successful final sweep consumed all pushes for surviving mobs.
            pending.pushes = Default::default();
        }
        self.creatures.pending.insert(level_id, pending);
        self.creatures.event_buf = events;
        if let Some(payload) = dispatch_panic {
            // Restoration may itself panic and replace this payload; a second
            // destructor panic may abort. panic=abort cannot recover at all.
            std::panic::resume_unwind(payload);
        }
        self.native_tick_post_batch(round);
        // ... and entity event 60 goes out as it is: the poof.
        // 🤖 Agent: the split follows the poof, as `LivingEntity.tickDeath`
        // broadcasts event 60 before it calls `remove`.
        for (id, at, width, height, split) in gone {
            let at = [at.x, at.y, at.z];
            self.lan_entity_event(id as i32, 60, at);
            self.death_poof(at, width, height);
            if let Some(m) = split {
                self.split_dead_cube(m);
            }
        }
        laps.lap("creature_retain");
        self.sync_riders();
        self.tick_skeleton_traps();
        self.guardian_bubbles();
        self.villager_reputation_tick();
        self.villager_item_sense();
        self.villager_pickups();
        laps.lap("creature_villagers");
        for m in dead {
            self.mob_died(&m);
        }
        for mut m in corpses {
            let names: Vec<&'static str> = m
                .extra
                .effects
                .iter()
                .map(|e| e.id.trim_start_matches("minecraft:"))
                .collect();
            let bounds = m.bounds();
            self.effects_on_death(&names, bounds, false, Some(m.id), Some(&mut m.random));
        }
        self.deliver_pending_mob_actions(level_id)?;
        self.finish_creature_entities(&mut laps, &primable)
    }

    fn deliver_pending_mob_actions(&mut self, level_id: &'static str) -> Result<()> {
        let mut pending = self.creatures.pending.remove(level_id).unwrap_or_default();
        let delivery = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<()> {
            while let Some(batch) = pending.batches.front_mut() {
                self.creatures.deferred_hits = true;
                let result = self.handle_mob_events_pending(&mut batch.normal);
                self.creatures.deferred_hits = false;
                result?;
                self.lan_guest_mob_events(&mut batch.guests);
                self.creatures.event_buf = pending
                    .batches
                    .pop_front()
                    .unwrap()
                    .normal
                    .root
                    .events
                    .into();
            }
            if !pending.pushes.is_empty() {
                for mob in &mut self.mobs {
                    horizon_client::mobs::apply_mob_pushes(mob, &mut pending.pushes);
                }
                // As in the producer's successful final sweep, removed targets
                // have no remaining owner to receive a push.
                pending.pushes = Default::default();
            }
            Ok(())
        }));
        if pending.batches.is_empty() && pending.pushes.is_empty() {
            pending.retained_passes = 0;
            pending.refusal_logged = false;
        }
        self.creatures.pending.insert(level_id, pending);
        match delivery {
            Ok(result) => result,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    fn finish_creature_entities(
        &mut self,
        laps: &mut horizon_client::spans::Laps,
        primable: &[(u32, i32)],
    ) -> Result<()> {
        self.drain_entity_inside();
        laps.lap("creature_events");
        self.ordered_before(super::ordered_tick::Batch::Projectiles)?;
        self.tick_projectiles()?;
        self.drain_entity_inside();
        self.tick_fangs();
        laps.lap("creature_projectiles");
        self.tick_allays();
        self.tick_pandas();
        self.tick_foxes();
        self.tick_dolphins();
        self.tick_mob_looting();
        self.tick_breeze_effects();
        self.tick_sniffer_effects();
        self.piglin_pickups();
        self.sulfur_cube_pickups();
        self.sulfur_cube_power_priming(primable);
        laps.lap("creature_systems");
        // Hurt and death sounds of damage dealt outside the mob ticks.
        let mut queued = Vec::new();
        for m in &mut self.mobs {
            queued.append(&mut m.outbox);
        }
        self.handle_mob_events(queued)?;
        // The enderman teleports those hurts asked for (`Enderman.hurtServer`).
        self.settle_hurt_teleports()?;
        laps.lap("creature_outbox");
        // And for the sizes the ticks changed (a pose, a baby grown, a scale).
        self.native_mob_sizes();
        Ok(())
    }
    /// `LivingEntity.makePoofParticles` (entity event 60) about a mob's box.
    pub(super) fn death_poof(&mut self, at: [f64; 3], width: f64, height: f64) {
        let options = horizon_client::particles::ParticleOptions::simple(
            horizon_client::particles::ParticleType::Poof,
        );
        for _ in 0..20 {
            let r = self.particles.level_random();
            let xa = r.next_gaussian() * 0.02;
            let ya = r.next_gaussian() * 0.02;
            let za = r.next_gaussian() * 0.02;
            let x = at[0] + width * (2.0 * r.next_double() - 1.0) - xa * 10.0;
            let y = at[1] + height * r.next_double() - ya * 10.0;
            let z = at[2] + width * (2.0 * r.next_double() - 1.0) - za * 10.0;
            self.particles.add(&options, [x, y, z], [xa, ya, za]);
        }
    }
    /// LivingEntity.die / dropAllDeathLoot.
    pub(super) fn mob_died(&mut self, m: &Mob) {
        // The killing blow for the drop hooks, before the player's hit is forgotten.
        let source = (!mod_api::__private::mods().is_empty()).then(|| self.mob_damage_source(m));
        let killer = self
            .last_hurt_by_player
            .get(&m.id)
            .copied()
            .filter(|_| m.ai.last_hurt_by_player > 0)
            .and_then(|credit| self.resolve_player_credit(credit));
        if self.mob_death_canceled(m) {
            return;
        }
        // LivingEntity.die: a sleeper gets up, and its bed is free again.
        if let Some(bed) = horizon_client::mobs::villager::sleeping_pos(m) {
            self.wake_villager(m.id);
            // The body may be gone already.
            self.set_bed_occupied(bed, false);
        }
        self.last_hurt_by_player.remove(&m.id);
        // TamableAnimal.die, before the rest of LivingEntity.die.
        self.tell_owner_pet_died(m, killer);
        self.love_cause.remove(&m.id);
        let by_player = m.ai.last_hurt_by_player > 0;
        let mut dying = m.clone();
        // LivingEntity.die: the death is heard, and a sculk catalyst nearby
        // takes the experience (before the loot is dropped). It takes what
        // would drop, whoever killed the mob. Experience the catalyst takes is
        // never dropped, so living_experience_drop does not hear of it.
        self.sculk_mob_died(m);
        let mut catalyst_xp = 0;
        let consumed =
            self.catalyst_takes_experience([m.position.x, m.position.y, m.position.z], |_| {
                catalyst_xp = if horizon_client::mobs::drops::drops_experience(&dying) {
                    horizon_client::mobs::drops::base_experience(&mut dying).max(0) as u32
                } else {
                    0
                };
                catalyst_xp
            });
        // CatalystListener: with experience to take, the player who last hurt
        // the mob gets KILL_MOB_NEAR_SCULK_CATALYST.
        if consumed && catalyst_xp > 0 {
            let who = match killer {
                Some(Some(conn)) if self.lan_acting == Some(conn) => {
                    Some(super::commands::Target::Player)
                }
                Some(Some(conn)) => Some(super::commands::Target::Guest(conn)),
                Some(None) if self.lan_acting.is_none() => Some(super::commands::Target::Player),
                _ => None,
            };
            if let Some(who) = who {
                self.for_player(who, |g| {
                    // CatalystListener: trigger(player, killedEntity, damageSource).
                    let view = g.mob_entity(m);
                    let blow = g.last_blow(m);
                    g.trigger_killed_as(
                        "minecraft:kill_mob_near_sculk_catalyst",
                        &view,
                        blow.as_ref(),
                    );
                });
            }
        }
        if let Some(killer) = killer {
            // Only the resolved player receives statistics and advancements.
            match killer {
                Some(conn) if self.lan_acting != Some(conn) => {
                    self.as_guest(conn, |g| g.credit_kill(m));
                }
                _ => self.credit_kill(m),
            }
        }
        // The body keeps its scores until it is removed (`ServerLevel.
        // EntityCallbacks.onDestroyed` → `Scoreboard.entityRemoved`, run by
        // `score_destroyed_entities`): LivingEntity.die does not reset them.
        // Raider.die's hero and Villager.tellWitnessesThatIWasMurdered go to
        // the causing entity, not the credited player: a tame wolf's kill is
        // neither its owner's heroism nor a murder by them.
        let slain_by_player = by_player && !killed_by_mob(m);
        self.raider_died(m, slain_by_player, killer.flatten());
        self.villager_murdered(m, slain_by_player, killer.flatten());
        let pos = [m.position.x, m.position.y + 0.5, m.position.z];
        let killer_entity = self.killer_loot_entity(killer.flatten());
        // LAST_DAMAGE_PLAYER only while the player is still in the world and
        // the hurt memory lasts (LivingEntity.dropFromLootTable). Experience
        // and equipment use the timer alone (dropExperience / playerKilled):
        // a guest who has left still counts.
        let player = killer
            .filter(|_| m.ai.last_hurt_by_player > 0)
            .and_then(|conn| self.killer_uuid(conn));
        if let Some(player) = player.filter(|_| m.extra.leash.is_some()) {
            self.lead_killers.insert(m.id, (player, self.dim));
        }
        self.drop_death_loot(
            &mut dying,
            player,
            m.ai.last_hurt_by_player > 0,
            killer_entity,
            consumed,
            pos,
            source,
        );
        self.finish_mob_death(m, &mut dying, source);
    }
    /// `AbstractCubeMob.remove`: split only as the dead body is removed.
    fn split_dead_cube(&mut self, mut dying: Mob) {
        if matches!(
            dying.species(),
            Species::Slime | Species::MagmaCube | Species::SulfurCube
        ) && dying.extra.size > 1
        {
            // AbstractCubeMob.remove: convert without spawn finalization.
            let drawn = horizon_client::mobs::cube::split_count(&mut dying);
            if drawn > 0 {
                let count = self.mob_split_count(&dying, drawn);
                for i in 0..count {
                    self.next_entity += 1;
                    let mut child =
                        Mob::new(self.next_entity, dying.kind.clone(), dying.position, 0.0);
                    horizon_client::mobs::cube::split_child(&mut dying, &mut child, i);
                    self.mobs.push(child);
                    self.native_mobs_joined(self.mobs.len() - 1, false);
                }
            }
        }
    }
    /// The killer's side of a mob's death: `ServerPlayer.awardKillScore`'s
    /// statistics and scores, and the `player_killed_entity` trigger.
    fn credit_kill(&mut self, m: &Mob) {
        // Player.killedEntity: the killed statistic goes to the causing
        // entity, so a kill by a mob (a tame wolf credited to its owner)
        // leaves it out; ServerPlayer.awardKillScore counts the rest.
        if !killed_by_mob(m) {
            self.award_stat("killed", &m.kind.id, 1);
        }
        self.award_custom_stat("mob_kills", 1);
        self.score_player_killed(super::commands::Target::Mob(m.id));
        let view = self.mob_entity(m);
        let blow = self.last_blow(m);
        self.trigger_killed(&view, blow.as_ref());
    }
    /// The damage source a mob died to (`die`'s), its direct entity included when the mob
    /// recorded one ([`Mob::last_direct`]).
    fn last_blow(&self, m: &Mob) -> Option<horizon_client::advancements::DamageView> {
        let (d, _) = m.last_damage?;
        let mut view = self.damage_view(d);
        if let Some(kind) = m.last_direct {
            view.direct = Some(horizon_client::advancements::EntityView {
                kind: kind.into(),
                position: d.from.unwrap_or_default(),
                ..Default::default()
            });
            view.is_direct = false;
        }
        Some(view)
    }
    /// A mob's damage source as advancement predicates see it: its type, the
    /// causing entity, and the projectile when one dealt it.
    pub(in crate::game) fn damage_view(
        &self,
        d: Damage,
    ) -> horizon_client::advancements::DamageView {
        let source = match d.attacker {
            // Its callers are KilledTrigger's: the source as their killing_blow sees it.
            Some(Who::Player) => Some(self.player_entity_for_any(&[
                "minecraft:player_killed_entity",
                "minecraft:kill_mob_near_sculk_catalyst",
            ])),
            Some(Who::Mob(id)) => self
                .mobs
                .iter()
                .find(|o| o.id == id)
                .map(|o| self.mob_entity(o)),
            None => None,
        };
        let projectile = match d.kind {
            DamageKind::Arrow => Some("minecraft:arrow"),
            DamageKind::Fireball => Some("minecraft:fireball"),
            DamageKind::WitherSkull => Some("minecraft:wither_skull"),
            DamageKind::Fireworks => Some("minecraft:firework_rocket"),
            _ => None,
        };
        let direct = match projectile {
            Some(kind) => Some(horizon_client::advancements::EntityView {
                kind: kind.into(),
                position: d.from.unwrap_or_default(),
                ..Default::default()
            }),
            None => source.clone(),
        };
        horizon_client::advancements::DamageView {
            kind: source_damage_type(d).to_owned(),
            // DamageSource.isDirect: the causing entity is the direct one.
            is_direct: projectile.is_none(),
            direct,
            source,
            ..Default::default()
        }
    }
    /// `dropAllDeathLoot` through `mobs::drops`: the entity loot table from the
    /// world's random sequence, equipment by drop chance, species extras and
    /// the experience orbs, spawned in order.
    /// The UUID of the player `last_hurt_by_player` keeps: the guest on
    /// connection `conn` (None once they left), else the player in place
    /// (None on a server, which has none of its own).
    /// The entity id of the player `killer_uuid` names for `conn`.
    pub(super) fn player_entity_id_by_connection(&self, conn: Option<u32>) -> Option<i32> {
        match conn {
            Some(conn) if self.lan_acting != Some(conn) => {
                Some(self.lan.as_ref()?.hub.guest(conn)?.entity_id)
            }
            Some(_) => Some(self.own_entity_id()),
            None if self.lan_acting.is_none() && self.player.is_some() => {
                Some(self.own_entity_id())
            }
            None => None,
        }
    }
    fn killer_uuid(&mut self, conn: Option<u32>) -> Option<u128> {
        match conn {
            // During that guest's own turn, the player in place has their UUID.
            Some(conn) if self.lan_acting != Some(conn) => {
                Some(self.lan.as_ref()?.hub.guest(conn)?.uuid)
            }
            Some(_) => Some(self.native_acting_id()),
            None if self.lan_acting.is_none() && self.player.is_some() => {
                Some(self.native_acting_id())
            }
            None => None,
        }
    }
    /// `LivingEntity.dropExperience`: the experience a dying mob or player is worth once
    /// the mods have had their say; 0 when they cancel it.
    pub(super) fn living_experience_drop(
        &mut self,
        entity: u32,
        kind: &str,
        attacker: Option<u128>,
        original: i32,
    ) -> i32 {
        let (mut amount, mut cancel) = (original, false);
        self.native_living_experience_drop(
            entity,
            kind,
            attacker,
            original,
            &mut amount,
            &mut cancel,
        );
        if cancel { 0 } else { amount.max(0) }
    }

    /// Adds the items a dying entity dropped to the level, each at its own
    /// position. With mods, they first hear the list (`native_living_drops`):
    /// edited or canceled.
    pub(super) fn drop_death_items(
        &mut self,
        dying: &DyingDrops<'_>,
        stacks: Vec<(ItemStack, [f64; 3], i32)>,
        pickup_delay: u32,
        mut velocity: impl FnMut(&mut Self) -> Option<[f64; 3]>,
    ) {
        let Some(source) = &dying.source else {
            for (stack, position, age) in stacks {
                let mut random = super::spill::prepare_item_random(None);
                random.state.age = age;
                let motion = velocity(self).unwrap_or(random.velocity);
                self.add_item_entity_prepared(
                    stack,
                    position,
                    Some(motion),
                    pickup_delay,
                    Some(random),
                );
            }
            return;
        };
        // Vanilla builds each item entity (and draws its motion) when it
        // collects the drop, before the event sees the list.
        let mut drops: Vec<super::spill::BlockItemDrop> = stacks
            .into_iter()
            .map(|(stack, position, age)| {
                let random = super::spill::prepare_item_random(None);
                super::spill::BlockItemDrop {
                    random: Some(random),
                    stack,
                    position,
                    velocity: velocity(self).unwrap_or(random.velocity),
                    pickup_delay,
                    state: super::spill::DropState {
                        age,
                        ..Default::default()
                    },
                }
            })
            .filter(|drop| drop.stack.count > 0 && &*drop.stack.item != "minecraft:air")
            .collect();
        let mut cancel = false;
        if mod_api::__private::listened(mod_api::HookId::LivingDrops)
            && let Ok(mut snapshots) = drops.try_into_api()
        {
            self.native_living_drops(mod_api::__private::LivingDropsCall {
                entity: dying.entity,
                kind: dying.kind,
                player: dying.player,
                source,
                recently_hit: dying.recently_hit,
                drops: &mut snapshots,
                cancel: &mut cancel,
            });
            if let Ok(mut next) = snapshots.try_into_engine(self.items()) {
                for (drop, original) in next.iter_mut().zip(&drops) {
                    drop.random = original.random;
                }
                drops = next;
            } else {
                cancel = false;
            }
        }
        if cancel {
            return;
        }
        for drop in drops {
            if drop.stack.count > 0 && &*drop.stack.item != "minecraft:air" {
                self.add_item_drop(drop);
            }
        }
    }
    /// Melee or ranged damage to another mob; whether the hit landed.
    pub(in crate::game) fn mob_hits_mob(
        &mut self,
        target: u32,
        amount: f32,
        mut source: Damage,
    ) -> bool {
        if source.attacker_species.is_none()
            && let Some(Who::Mob(id)) = source.attacker
        {
            source.attacker_species = self.mobs.iter().find(|m| m.id == id).map(|m| m.species());
        }
        let mut killed = false;
        let mut hurt = false;
        let mut infested = Vec::new();
        let catch_up = self.catch_up_countdowns(source.attacker, target);
        let aim = self.attacker_aim(source.attacker);
        // So `living_knockback` sees who hit and what they hold.
        let attack = self.attack_of_who(source.attacker);
        if let Some(t) = self.mobs.iter_mut().find(|m| m.id == target) {
            let previous_invulnerable = t.invulnerable;
            let hit = t.damage_aimed(
                crate::game::entity::damage_level_of!(self, &attack),
                amount,
                source,
                aim,
            );
            hurt = hit;
            if hit && catch_up && t.invulnerable > previous_invulnerable {
                t.hurt_time = (t.hurt_time - 1).max(0);
                t.invulnerable = (t.invulnerable - 1).max(0);
            }
            if hit {
                killed = !t.alive();
                // The hurt or death sound the mob raised.
                let center = t.center();
                for event in std::mem::take(&mut t.outbox) {
                    match event {
                        MobEvent::Sound(s, _, volume, pitch) => {
                            self.audio.play(&full_id(s), Some(center), volume, pitch);
                        }
                        MobEvent::Infested { at, velocities } => infested.push((at, velocities)),
                        other => {
                            // Left for the end of the tick.
                            t.outbox.push(other);
                        }
                    }
                }
            }
        }
        if hurt {
            self.resolve_player_responsible(target, source);
        }
        for (at, velocities) in infested {
            self.spawn_infested(at, velocities);
        }
        if hurt
            && let Some(Who::Mob(attacker)) = source.attacker
            && let Some(m) = self
                .mobs
                .iter()
                .find(|m| m.id == attacker && m.species() == Species::Axolotl)
        {
            self.audio.play(
                "minecraft:entity.axolotl.attack",
                Some([m.position.x, m.position.y, m.position.z]),
                1.0,
                1.0,
            );
        }
        if killed && let Some(Who::Mob(killer)) = source.attacker {
            self.zombie_killed(killer, target);
        }
        hurt
    }
    /// `LivingEntity.resolvePlayerResponsibleForDamage` after `source` hurt a
    /// mob: a tame wolf's hit is its owner's (`setLastHurtByPlayer(uuid, 100)`),
    /// so the kill drops experience and player-only loot and the owner, when
    /// in the world, gets the credit (`getKillCredit`); a tame wolf without an
    /// owner clears it. A player's own hits are recorded where they land.
    pub(super) fn resolve_player_responsible(&mut self, victim: u32, source: Damage) {
        let Some(Who::Mob(attacker)) = source.attacker else {
            return;
        };
        let Some(owner) = self
            .mobs
            .iter()
            .find(|m| m.id == attacker && m.species() == Species::Wolf && m.extra.tame)
            .map(|m| m.extra.owner)
        else {
            return;
        };
        let Some(m) = self.mobs.iter_mut().find(|m| m.id == victim) else {
            return;
        };
        m.ai.last_hurt_by_player = if owner.is_some() { 100 } else { 0 };
        match owner {
            Some(uuid) => self.last_hurt_by_player.insert(
                victim,
                super::combat::PlayerCredit(horizon_client::lan::playerdata::ints_uuid(uuid)),
            ),
            None => self.last_hurt_by_player.remove(&victim),
        };
    }
    /// Remember the acting player for a mob's `lastHurtByPlayer`.
    pub(super) fn remember_mob_player_credit(&mut self, victim: u32) {
        if let Some(uuid) = self.player_uuid_of(self.lan_acting) {
            self.last_hurt_by_player.insert(
                victim,
                super::combat::PlayerCredit(horizon_client::lan::playerdata::ints_uuid(uuid)),
            );
        }
    }

    /// Resolve `EntityReference.getEntity` across levels. None inside means
    /// the player in place; Some is a guest connection.
    pub(super) fn resolve_player_credit(
        &self,
        credit: super::combat::PlayerCredit,
    ) -> Option<Option<u32>> {
        let uuid = horizon_client::lan::playerdata::uuid_ints(credit.0);
        if self.player.is_some()
            && horizon_client::mobs::tame::owned_by(Some(uuid), self.acting_uuid())
        {
            return Some(self.lan_acting);
        }
        let uuid = horizon_client::lan::playerdata::ints_uuid(uuid);
        self.lan
            .iter()
            .flat_map(|h| h.hub.guests.iter())
            .find(|g| g.placed && g.uuid == uuid)
            .map(|g| Some(g.conn))
    }
    /// `LivingEntity.dropFromGiftLootTable`: the gift table from its random
    /// sequence with the mob as `this` (a chicken's egg depends on its
    /// variant); the loot data of the game's resources decides, and without it
    /// the items' table engine (no entity) rolls it. Whether anything dropped.
    fn drop_gift_loot(&mut self, mob: u32, table: &'static str, at: [f64; 3]) -> bool {
        use horizon_client::mobs::drops;
        use horizon_world::loot::EntityLootParams;
        let loot = self.server_loot.get();
        let this = self
            .mobs
            .iter()
            .find(|m| m.id == mob)
            .map(|m| drops::loot_entity(m, None));
        let stacks = match (loot, this) {
            (Some(loot), Some(this)) if loot.has_table(table) => {
                let params = EntityLootParams {
                    this,
                    ..EntityLootParams::default()
                };
                let seed = self.level.seed;
                match loot.random_sequence(table) {
                    Some(id) => {
                        let random = self.rand_sequences.get(id, seed);
                        loot.roll_entity(table, random, &params)
                            .iter()
                            .filter(|s| !s.is_empty())
                            .map(from_loot)
                            .collect()
                    }
                    None => Vec::new(),
                }
            }
            _ => self
                .res
                .items()
                .loot_table(table, false, None, &mut self.world.random),
        };
        let dropped = !stacks.is_empty();
        for d in stacks {
            // The ItemEntity constructor's motion, from the entity's own
            // random (not the level's).
            self.spawn_item_own(d, at, 10);
        }
        dropped
    }
    /// Adds a projectile, giving it an id from the entity counter
    /// (`Entity.id`, the same counter as the rest) when it has none. Its id.
    pub(in crate::game) fn add_projectile(
        &mut self,
        mut p: horizon_client::projectiles::Projectile,
    ) -> u32 {
        if p.id == 0 {
            self.next_entity += 1;
            p.id = self.next_entity;
        }
        // The UUID its own random made, unless it already has one (a client
        // copy's from the spawn packet).
        if let Some(uuid) = p.uuid.take() {
            self.persist.uuids.get_or_insert_with(p.id, || uuid);
        }
        // Projectile.setOwner: the acting player's or the mob's UUID, which
        // its save keeps even once the owner is gone.
        if p.owner_uuid.is_none() {
            p.owner_uuid = match p.owner {
                Some(Who::Player) => Some(self.acting_player_uuid()),
                Some(Who::Mob(id)) => Some(self.uuid_for(id)),
                None => None,
            };
        }
        let player = (p.owner == Some(Who::Player))
            .then(|| horizon_client::lan::playerdata::ints_uuid(self.acting_player_uuid()));
        let name = if player.is_some() {
            self.acting_player_name().to_owned()
        } else {
            String::new()
        };
        let (id, kind) = (p.id, p.entity_type());
        if let Some(a) = p.arrow.as_mut()
            && !p.stuck
        {
            a.spawn_pos = Some(p.position);
            a.spawn_velocity = Some(p.velocity);
            a.spawn_rot = Some((p.pitch, p.yaw));
        }
        self.creatures.projectiles.push(p);
        self.native_projectile_launch(player, &name, id, kind);
        id
    }
    /// `MushroomCow.shear`'s `sendParticles(EXPLOSION, x, getY(0.5), z, 1, 0, 0, 0, 0)`:
    /// one puff over a sheared mooshroom, half its `height` up, shown here and to
    /// the players within range.
    pub(super) fn mooshroom_shear_puff(&mut self, at: [f64; 3], height: f64) {
        let at = [at[0], at[1] + 0.5 * height, at[2]];
        let options = ParticleOptions::simple(ParticleType::Explosion);
        self.particles
            .send_particles(&options, at, [0.0; 3], 0.0, 1);
        self.lan_level_particles(&options, at, [0.0; 3], 0.0, 1, None);
    }
    /// `Entity.sendBubbleColumnParticles` at the top of the column block `pos`: two
    /// splashes and two bubbles over it, which the level random places, shown here
    /// and to the players within range.
    pub(super) fn bubble_column_particles(&mut self, pos: BlockPos) {
        for _ in 0..2 {
            let r = &mut self.world.random;
            let splash = [
                f64::from(pos.x) + r.next_double(),
                f64::from(pos.y + 1),
                f64::from(pos.z) + r.next_double(),
            ];
            let bubble = [
                f64::from(pos.x) + r.next_double(),
                f64::from(pos.y + 1),
                f64::from(pos.z) + r.next_double(),
            ];
            let splash_options = ParticleOptions::simple(ParticleType::Splash);
            self.particles
                .send_particles(&splash_options, splash, [0.0; 3], 1.0, 1);
            self.lan_level_particles(&splash_options, splash, [0.0; 3], 1.0, 1, None);
            let bubble_options = ParticleOptions::simple(ParticleType::Bubble);
            self.particles
                .send_particles(&bubble_options, bubble, [0.0, 0.01, 0.0], 0.2, 1);
            self.lan_level_particles(&bubble_options, bubble, [0.0, 0.01, 0.0], 0.2, 1, None);
        }
    }
    /// A mob's projectile enters the level.
    fn launch_mob_projectile(&mut self, mut p: Projectile) {
        self.next_entity += 1;
        p.id = self.next_entity;
        let at = [p.position.x, p.position.y, p.position.z];
        // LlamaSpit.recreateFromPacket: the spit's burst on the client.
        if p.kind == ProjectileKind::LlamaSpit {
            let v = p.velocity;
            self.particles.llama_spit_burst(at, [v.x, v.y, v.z]);
        }
        self.spawn_mob_projectile(p);
    }
    /// `Entity.igniteForSeconds`: `seconds * 20` ticks of fire, scaled by the
    /// target's burning-time attribute.
    fn ignite_entity(&mut self, target: Who, seconds: i32) -> Result<()> {
        match target {
            Who::Player => {
                if !self.creative
                    && let Some(p) = self.player.as_mut()
                {
                    p.ignite_for_ticks(seconds * 20)?;
                }
            }
            Who::Mob(id) => {
                if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
                    m.ignite_for_ticks(seconds * 20);
                }
            }
        }
        Ok(())
    }
    pub(super) fn handle_mob_events(&mut self, mut events: Vec<MobEvent>) -> Result<()> {
        self.handle_mob_events_reusable(&mut events)
    }

    pub(in crate::game) fn handle_mob_events_reusable(
        &mut self,
        events: &mut Vec<MobEvent>,
    ) -> Result<()> {
        let mut actions = NormalMobActions::new(std::mem::take(events));
        let result = self.handle_mob_events_pending(&mut actions);
        *events = actions.root.events.into();
        result
    }

    fn handle_mob_events_pending(&mut self, actions: &mut NormalMobActions) -> Result<()> {
        loop {
            let frame = actions.nested.last_mut().unwrap_or(&mut actions.root);
            let Some(e) = frame.events.pop_front() else {
                if actions.nested.pop().is_some() {
                    continue;
                }
                return Ok(());
            };
            let actor = frame.actor;
            let broke = &mut frame.broke;
            match e {
                MobEvent::ForPlayer { conn, event } => {
                    let mut event = *event;
                    // Projectiles are spawned in the world rather than run
                    // as a guest, but must retain the selected guest target.
                    if let Some(conn) = conn
                        && let MobEvent::Projectile(p) = &mut event
                        && let ProjectileKind::ShulkerBullet { target, .. } = &mut p.kind
                        && *target == Some(Who::Player)
                    {
                        *target = self.guest_identity(conn).map(|(_, id)| Who::Mob(id));
                        self.handle_mob_events(vec![event])?;
                    } else if let Some(conn) = conn {
                        if let Some(mob) = actor {
                            self.lan_guest_mob_events(&mut vec![(conn, mob, event)]);
                        } else {
                            // No nested block-break actor is in scope. A
                            // missing guest must not redirect to another player.
                            if let Some(result) =
                                self.as_guest(conn, |game| game.handle_mob_events(vec![event]))
                            {
                                result?;
                            }
                        }
                    } else {
                        self.handle_mob_events(vec![event])?;
                    }
                }
                MobEvent::BubbleColumnParticles { pos } => {
                    self.bubble_column_particles(BlockPos::new(pos[0], pos[1], pos[2]))
                }
                MobEvent::LandingDust {
                    state,
                    at,
                    count,
                    speed,
                } => self.landing_dust(state, Vec3::new(at[0], at[1], at[2]), count, speed),
                MobEvent::SquidCycle { mob, at } => self.lan_entity_event(mob as i32, 19, at),
                MobEvent::RabbitHop { mob, at } => self.lan_entity_event(mob as i32, 1, at),
                MobEvent::Drown { mob, at, bubbles } => self.mob_drown(mob, at, bubbles),
                MobEvent::PowderSnowParticle { at, velocity } => {
                    self.snowflake_particle(at, velocity);
                }
                MobEvent::SunProtectionWear { mob } => self.wear_sun_protection(mob),
                MobEvent::ArmorHurt {
                    mob,
                    amount,
                    source,
                } => self.hurt_mob_armor(mob, amount, &source),
                MobEvent::GliderWear { mob, slot } => self.wear_mob_glider(mob, slot),
                MobEvent::EntityEvent { mob, event, at } => {
                    // Ravager.roar (event 69 ends it): with the raw mob_griefing rule (not
                    // canEntityGrief), ROAR_TARGET_WITH_GRIEFING also takes the armor stands.
                    if event == 69
                        && self
                            .ui
                            .rules
                            .bool_at(horizon_client::gamerules::vanilla::MOB_GRIEFING)
                        && let Some(area) = self
                            .mobs
                            .iter()
                            .find(|m| m.id == mob && m.species() == Species::Ravager)
                            .map(|m| m.bounds().inflate(Vec3::new(4.0, 4.0, 4.0)))
                    {
                        self.roar_pushes_armor_stands(area, at);
                    }
                    self.lan_entity_event(mob as i32, event, at)
                }
                MobEvent::LevelParticles {
                    kind,
                    at,
                    spread,
                    speed,
                    count,
                } => {
                    if let Some(ty) = horizon_client::particles::ParticleType::from_name(kind) {
                        let options = horizon_client::particles::ParticleOptions::simple(ty);
                        self.particles
                            .send_particles(&options, at, spread, speed, count);
                        self.lan_level_particles(&options, at, spread, speed, count, None);
                    }
                }
                // The LAN packet has no item for a particle yet: the host's window only.
                MobEvent::ItemParticles {
                    item,
                    at,
                    spread,
                    speed,
                    count,
                } => {
                    let options =
                        self.item_particle(&horizon_client::items::ItemStack::new(item, 1));
                    self.particles
                        .send_particles(&options, at, spread, speed, count);
                }
                MobEvent::Sound(name, at, volume, pitch) => {
                    self.audio.play(&full_id(name), Some(at), volume, pitch)
                }
                MobEvent::ProfessionWorkSound(name, at, volume, pitch) => {
                    self.audio.play(&name, Some(at), volume, pitch)
                }
                MobEvent::LocalSound(name, at, volume, pitch) => {
                    self.audio
                        .play_unshared(&full_id(name), Some(at), volume, pitch)
                }
                MobEvent::BoundSound {
                    mob,
                    sound,
                    source,
                    at,
                    volume,
                    pitch,
                } => {
                    let name = full_id(sound);
                    let sound = LevelSound::new(&name, at, volume, pitch).in_source(source);
                    self.play_bound_sound(sound, mob, None);
                }
                MobEvent::LevelEventSound {
                    sound,
                    at,
                    volume,
                    spread,
                } => {
                    // WitherBoss.customServerAiStep: the spawn roar is global
                    // level event 1023 at the block position.
                    if sound == "entity.wither.spawn" {
                        let pos = at.map(|v| v.floor() as i32);
                        self.global_level_event(1023, pos, 0);
                    } else {
                        // The client's own level random: nothing drawn from the server's.
                        let mut r = horizon_core::random::LegacyRandom::new(
                            self.world.game_time.wrapping_mul(341_873_128_712),
                        );
                        let pitch = (r.next_float() - r.next_float()) * spread + 1.0;
                        self.audio.play(&full_id(sound), Some(at), volume, pitch)
                    }
                }
                MobEvent::SoundId(name, at, volume, pitch) => {
                    self.audio.play(&name, Some(at), volume, pitch)
                }
                MobEvent::GameEvent {
                    event, at, source, ..
                } if event == "minecraft:flap"
                    && self.mobs.iter().any(|m| {
                        m.id == source && matches!(m.species(), Species::Chicken | Species::Parrot)
                    }) =>
                {
                    let ctx = self.who_context(Some(Who::Mob(source)), None);
                    self.post_game_event(event, at, ctx);
                }
                // A mob's own `gameEvent(..)` calls: Armadillo.rollUp/rollOut's
                // ENTITY_ACTION, Creeper.tick's PRIME_FUSE, Mob.ate's EAT, a fox's
                // DRINK, the shulker's lid, an evoker's vexes (ENTITY_PLACE),
                // Enderman.teleport's TELEPORT, onEquipItem's EQUIP/UNEQUIP.
                MobEvent::GameEvent {
                    event, at, source, ..
                } if matches!(
                    event,
                    "minecraft:entity_action"
                        | "minecraft:prime_fuse"
                        | "minecraft:eat"
                        | "minecraft:drink"
                        | "minecraft:container_open"
                        | "minecraft:container_close"
                        | "minecraft:entity_place"
                        | "minecraft:teleport"
                        | "minecraft:equip"
                        | "minecraft:unequip"
                ) =>
                {
                    let ctx = self.who_context(Some(Who::Mob(source)), None);
                    self.post_game_event(event, at, ctx);
                }
                MobEvent::GameEvent {
                    event: event @ ("minecraft:block_place" | "minecraft:block_destroy"),
                    at,
                    source,
                    state,
                } => {
                    let ctx = self.who_context(Some(Who::Mob(source)), Some(state));
                    self.post_game_event(event, at, ctx);
                }
                // `FarmlandBlock.turnToBaseBlock` and `TurtleEggBlock.decreaseEggs`:
                // the landing's own block change posts its game event, from the
                // mob that landed (`Context.of(entity, state)`) unless the block
                // destroyed itself under it (`Context.of(state)`).
                MobEvent::LandingGameEvent {
                    event,
                    at,
                    source,
                    state,
                } => {
                    let who = source.map(Who::Mob);
                    self.post_block_game_event(event, at, who, Some(state));
                }
                // The sculk layer tracks steps and bee/bat flaps itself
                // (`entity_motion_events`, `tick_mob_sculk_events`). Birds
                // above use their movement's flyDist/nextFlap threshold.
                MobEvent::GameEvent { .. } => {}
                MobEvent::AttackPlayer(amount, from) => {
                    self.combat.hit_landed =
                        self.mob_hits_player(amount, from, "minecraft:mob_attack");
                }
                // doHurtTarget: the effect only when hurtServer took damage.
                // Bee.doHurtTarget: a sting that hurt leaves the stinger in the
                // body, then the poison.
                MobEvent::StingPlayer {
                    amount,
                    from,
                    poison_ticks,
                } => {
                    if self.mob_hits_player(amount, from, "minecraft:mob_attack") {
                        self.combat.stuck.add_stinger();
                        if poison_ticks > 0 {
                            self.give_player_effect("minecraft:poison", poison_ticks, 0);
                        }
                    }
                }
                MobEvent::AttackPlayerWithEffect {
                    amount,
                    from,
                    effect,
                    ticks,
                    amplifier,
                } => {
                    self.combat.hit_landed =
                        self.mob_hits_player(amount, from, "minecraft:mob_attack");
                    if self.combat.hit_landed {
                        self.give_player_effect(effect, ticks, amplifier);
                    }
                }
                MobEvent::DamagePlayer { amount, from, kind } => {
                    self.mob_hits_player(amount, from, kind);
                }
                MobEvent::ElderCurse { at, silent } => self.elder_curse(at, silent),
                MobEvent::SetVelocity { target, velocity } => {
                    let v = Vec3::new(velocity[0], velocity[1], velocity[2]);
                    match target {
                        Who::Player => {
                            if let Some(p) = self.player.as_mut() {
                                p.velocity = v;
                            }
                        }
                        Who::Mob(id) => {
                            if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
                                m.velocity = v;
                            }
                        }
                    }
                }
                MobEvent::RemoveKilled { target } => {
                    // The bite that killed it ran `LivingEntity.die` (loot, such
                    // as a magma cube's froglight, and experience) before
                    // `eatEntity` removes it: a removed mob never reaches the
                    // next tick's deaths, so it dies here first.
                    if let Some(m) = self
                        .mobs
                        .iter()
                        .find(|m| m.id == target && !m.alive() && !m.removed)
                        .cloned()
                    {
                        if self.died.insert(target) {
                            self.mob_died(&m);
                        }
                        if let Some(m) = self.mobs.iter_mut().find(|m| m.id == target) {
                            m.removed = true;
                        }
                    }
                }
                MobEvent::Push { target, velocity } => {
                    let v = Vec3::new(velocity[0], velocity[1], velocity[2]);
                    match target {
                        Who::Player => {
                            if let Some(p) = self.player.as_mut() {
                                p.velocity = p.velocity.add_vector(v);
                            }
                        }
                        Who::Mob(id) => {
                            if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
                                m.velocity = m.velocity.add_vector(v);
                            }
                        }
                    }
                }
                MobEvent::Knockback {
                    target,
                    power,
                    x,
                    z,
                    source,
                    apply_blocking,
                } => match target {
                    // Ram knockback checks blocking independently of the hurt call.
                    Who::Player => self.mob_pushes_player(power, x, z, source, apply_blocking),
                    Who::Mob(id) => {
                        let attack = self.attack_of_who(source.attacker);
                        if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
                            let level = crate::game::entity::damage_level_of!(self, &attack);
                            m.knockback(level, power, x, z, &source, 0.0);
                        }
                    }
                },
                MobEvent::LidPush {
                    shulker,
                    area,
                    delta,
                } => {
                    let (lid, delta) = (
                        Lid::Shulker(shulker),
                        Vec3::new(delta[0], delta[1], delta[2]),
                    );
                    // A client of a server pushes only its own player, as
                    // the shulker box's lid does.
                    if self.remote.is_some() {
                        self.lid_push_local(lid, area, delta);
                    } else {
                        self.lid_push(lid, area, delta);
                    }
                }
                MobEvent::Wololo { mob } => {
                    if let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob) {
                        // DyeColor.RED.
                        m.extra.color = 14;
                    }
                }
                MobEvent::Fangs {
                    at,
                    yaw,
                    delay,
                    owner,
                } => self.add_fangs(at, yaw, delay, owner),
                MobEvent::Shoot(..) => {}
                MobEvent::ExplosionSource {
                    mob,
                    charged,
                    name,
                    in_water,
                    kind,
                } => {
                    self.combat.explosion_source = Some(mob);
                    self.combat.explosion_in_water = in_water;
                    // The creeper causes the blast's damage, but it is out of
                    // the mob list already: its name comes with the event.
                    self.combat.exploded.push((mob, name));
                    self.combat.exploded_kinds.push((mob, kind));
                    if charged {
                        self.combat.charged_exploders.push(mob);
                    }
                }
                MobEvent::LingeringCloud {
                    at,
                    effects,
                    hidden,
                    ambient,
                    color,
                } => {
                    self.spawn_creeper_cloud(at, effects, hidden, ambient, color);
                }
                MobEvent::Explode(at, radius, species, silent) => {
                    // The exploding mob causes the damage (Creeper.explodeCreeper).
                    let noted = self.combat.explosion_source.is_some();
                    let source_mob = self.combat.explosion_source.or_else(|| {
                        self.mobs
                            .iter()
                            .filter(|m| {
                                (m.position.x - at[0]).powi(2) + (m.position.z - at[2]).powi(2)
                                    < 1.0
                            })
                            .map(|m| m.id)
                            .next()
                    });
                    self.combat.explosion_source = source_mob;
                    self.combat.explosion_source_named = noted;
                    let portal_proof = species == Species::SulfurCube
                        && source_mob.is_some_and(|id| {
                            self.portals
                                .entities
                                .get(&id)
                                .is_some_and(|p| p.cooldown > 0)
                        });
                    self.combat.explosion_cause = match species {
                        Species::Creeper => Some("minecraft:creeper"),
                        Species::Wither => Some("minecraft:wither"),
                        Species::SulfurCube => Some("minecraft:sulfur_cube"),
                        _ => None,
                    }
                    .map(|kind| horizon_client::advancements::EntityView {
                        kind: kind.into(),
                        position: at,
                        ..Default::default()
                    });
                    // Only a creeper or a wither empties the hives it blows up.
                    // The creeper is already gone from the list, so the event
                    // carries its kind. A sulfur cube's blast is TNT's.
                    let mut source =
                        super::explosion::ExplosionSource::default().with_silence(silent);
                    if let Some(id) = source_mob
                        && matches!(species, Species::Creeper | Species::Wither)
                    {
                        let kind = match species {
                            Species::Creeper => "minecraft:creeper",
                            _ => "minecraft:wither",
                        };
                        source = source.with_entity(id, kind);
                    }
                    let result = match species {
                        Species::Creeper | Species::Wither => self.with_hive_emptying_blast(|g| {
                            g.explode(source_mob, at, radius, source)
                        }),
                        Species::SulfurCube => self.explode_sulfur_cube(
                            source_mob,
                            at,
                            radius,
                            source.with_portal_proof(portal_proof),
                        ),
                        _ => self.explode(source_mob, at, radius, source),
                    };
                    self.combat.explosion_source = None;
                    self.combat.explosion_source_named = false;
                    self.combat.explosion_in_water = false;
                    self.combat.explosion_cause = None;
                    result?
                }
                MobEvent::AttackMob {
                    target,
                    amount,
                    source,
                } => {
                    self.combat.hit_landed = self.mob_attacks_mob(target, amount, source);
                }
                MobEvent::DamageMob {
                    target,
                    amount,
                    source,
                } => {
                    self.mob_hits_mob(target, amount, source);
                }
                MobEvent::AttackMobWithEffect {
                    target,
                    amount,
                    source,
                    effect,
                    ticks,
                    amplifier,
                } => {
                    // CaveSpider, Husk and WitherSkeleton.doHurtTarget: the
                    // effect sits inside the branch where the hit landed, and
                    // `LivingEntity.addEffect` takes a mob as well as a player.
                    self.combat.hit_landed = self.mob_attacks_mob(target, amount, source);
                    if self.combat.hit_landed
                        && let Some(t) = self.mobs.iter_mut().find(|m| m.id == target)
                    {
                        t.add_effect(effect, ticks, amplifier);
                    }
                }
                MobEvent::CubeHurtPlayer { cube, amount, from } => {
                    // AbstractCubeMob.dealDamage: the attack sound (and its
                    // two pitch draws, made even for a silent cube, whose
                    // Entity.playSound is then skipped) only follows a hit
                    // that landed.
                    if self.hurt_player_from(
                        amount,
                        Some(from),
                        "minecraft:mob_attack",
                        Some(Some(cube)),
                        false,
                    ) && let Some(m) = self.mobs.iter_mut().find(|m| m.id == cube)
                    {
                        let pitch = (m.random.next_float() - m.random.next_float()) * 0.2 + 1.0;
                        let at = m.center();
                        if !m.silent {
                            self.handle_mob_events(vec![MobEvent::Sound(
                                "entity.slime.attack",
                                at,
                                1.0,
                                pitch,
                            )])?;
                        }
                    }
                }
                MobEvent::CubeHurtMob {
                    cube,
                    target,
                    amount,
                    source,
                } => {
                    // AbstractCubeMob.dealDamage: the attack sound (and its
                    // two pitch draws, made even for a silent cube, whose
                    // Entity.playSound is then skipped) only follows a hit
                    // that landed.
                    if self.mob_hits_mob(target, amount, source)
                        && let Some(m) = self.mobs.iter_mut().find(|m| m.id == cube)
                    {
                        let pitch = (m.random.next_float() - m.random.next_float()) * 0.2 + 1.0;
                        let at = m.center();
                        if !m.silent {
                            self.handle_mob_events(vec![MobEvent::Sound(
                                "entity.slime.attack",
                                at,
                                1.0,
                                pitch,
                            )])?;
                        }
                        self.mob_post_attack(
                            target,
                            source.attacker,
                            None,
                            Some(super::creatures::damage_type(source.kind, false)),
                        );
                    }
                }
                MobEvent::SonicBoom {
                    target,
                    attacker,
                    from,
                    direction,
                } => {
                    // SonicBoom: the push only follows a hit that landed, scaled
                    // by the target's KNOCKBACK_RESISTANCE.
                    let (hurt, resistance) = match target {
                        Who::Player => {
                            let hurt = self.mob_hits_player(10.0, from, "minecraft:sonic_boom");
                            let resistance = self.player.as_mut().map_or(0.0, |p| {
                                p.attribute("minecraft:knockback_resistance").unwrap_or(0.0)
                            });
                            (hurt, resistance)
                        }
                        Who::Mob(id) => {
                            let source = Damage {
                                kind: DamageKind::SonicBoom,
                                attacker: Some(Who::Mob(attacker)),
                                from: Some(from),
                                attacker_species: Some(Species::Warden),
                            };
                            let hurt = self.mob_hits_mob(id, 10.0, source);
                            let resistance = self
                                .mobs
                                .iter()
                                .find(|m| m.id == id)
                                .map_or(0.0, super::lan_attributes::mob_knockback_resistance);
                            (hurt, resistance)
                        }
                    };
                    if hurt {
                        let horizontal = 2.5 * (1.0 - resistance);
                        let vertical = 0.5 * (1.0 - resistance);
                        let velocity = [
                            direction[0] * horizontal,
                            direction[1] * vertical,
                            direction[2] * horizontal,
                        ];
                        self.handle_mob_events(vec![MobEvent::Push { target, velocity }])?;
                    }
                }
                MobEvent::FinishAdmiring { mob } => self.finish_admiring(mob),
                MobEvent::Hiss { mob } => {
                    if let Some(m) = self.mobs.iter().find(|m| m.id == mob) {
                        let at = m.center();
                        let sound = horizon_client::mobs::finalize::variant_sound(m, "hiss")
                            .unwrap_or("entity.cat.hiss");
                        self.audio.play(&full_id(sound), Some(at), 1.0, 1.0);
                    }
                }
                MobEvent::Tamed { .. }
                | MobEvent::Buck { .. }
                | MobEvent::StabDismount { .. }
                | MobEvent::EjectPassengers { .. } => self.mount_event(&e),
                MobEvent::IronGolemHit {
                    target,
                    amount,
                    source,
                } => {
                    // IronGolem.doHurtTarget: the toss only follows a hit that
                    // landed, scaled by the target's KNOCKBACK_RESISTANCE.
                    let hurt = match target {
                        Who::Player => {
                            let from = source.from.unwrap_or_default();
                            self.mob_hits_player(amount, from, "minecraft:mob_attack")
                        }
                        Who::Mob(id) => self.mob_hits_mob(id, amount, source),
                    };
                    if hurt {
                        let resistance = match target {
                            Who::Player => self.player.as_mut().map_or(0.0, |p| {
                                p.attribute("minecraft:knockback_resistance").unwrap_or(0.0)
                            }),
                            Who::Mob(id) => {
                                self.mobs.iter().find(|m| m.id == id).map_or(0.0, |m| {
                                    super::lan_attributes::mob_knockback_resistance(m)
                                })
                            }
                        };
                        let up = f64::from(0.4f32) * (1.0 - resistance).max(0.0);
                        match target {
                            Who::Player => {
                                if let Some(p) = self.player.as_mut() {
                                    p.velocity.y += up;
                                }
                            }
                            Who::Mob(id) => {
                                if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
                                    m.velocity.y += up;
                                }
                            }
                        }
                        if let Who::Mob(id) = target {
                            self.mob_post_attack(
                                id,
                                source.attacker,
                                None,
                                Some("minecraft:mob_attack"),
                            );
                        }
                    }
                }
                MobEvent::Bonemeal { mob, pos } => {
                    self.villager_bonemeal(mob, BlockPos::new(pos[0], pos[1], pos[2]));
                }
                MobEvent::RingBell { bell } => {
                    let pos = BlockPos::new(bell[0], bell[1], bell[2]);
                    let rang = self
                        .with_level(|b, level, ticks| b.bell_attempt_to_ring(level, ticks, pos));
                    if rang {
                        let _ = self.process_level_events();
                    }
                }
                MobEvent::ChestOpen { chest, open, mob } => self.golem_chest_open(chest, open, mob),
                MobEvent::ChestSlot { chest, slot, stack } => {
                    self.golem_chest_slot(chest, slot, stack)
                }
                MobEvent::BecomeStatue { mob } => self.golem_become_statue(mob),
                MobEvent::EnterHive { mob, hive } => self.bee_enter_hive(mob, hive),
                MobEvent::ParrotToShoulder { mob } => self.parrot_to_shoulder(mob),
                MobEvent::Infested { at, velocities } => self.spawn_infested(at, velocities),
                MobEvent::Gift { target, item } => self.receive_gift(target, item),
                MobEvent::HurtSelf { mob, kind, amount } => {
                    if let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob) {
                        m.damage(
                            crate::game::entity::damage_level_of!(self, scoped),
                            amount,
                            horizon_client::mobs::Damage::of(kind),
                        );
                    }
                }
                MobEvent::SetTarget { mob, target } => {
                    let player = self
                        .lan
                        .as_ref()
                        .and_then(|host| host.mob_players.get(&mob).copied())
                        .or(self.lan_acting);
                    self.set_event_ai_target(mob, Some(target), player);
                }
                MobEvent::PiglinAlertTarget(alert) => self.piglin_alert_target(alert),
                MobEvent::HoglinAttackTarget {
                    mob,
                    target,
                    player,
                } => self.hoglin_attack_target(mob, target, player),
                MobEvent::HuntHoglin { mob, target, ticks } => {
                    let mut acquire = false;
                    if let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob) {
                        m.extra.ints[0] = ticks;
                        acquire = m.ai.target.is_none();
                    }
                    if acquire {
                        let player = self
                            .lan
                            .as_ref()
                            .and_then(|host| host.mob_players.get(&mob).copied())
                            .or(self.lan_acting);
                        self.set_event_ai_target(mob, Some(Who::Mob(target)), player);
                    }
                }
                MobEvent::Projectile(p) => self.launch_mob_projectile(p),
                MobEvent::BowShot {
                    mob,
                    weapon,
                    mut arrow,
                } => {
                    self.mob_bow_ammo(mob, &weapon, &mut arrow);
                    if let Some(state) = arrow.arrow.as_mut() {
                        state.fired_from_weapon = Some(weapon);
                        arrow.bonus.damage = 0.0;
                    }
                    self.launch_mob_projectile(arrow);
                }
                MobEvent::CrossbowShot { ammo, mut arrow } => {
                    if &*ammo.item == "minecraft:firework_rocket" {
                        // `CrossbowItem.createProjectile`: a rocket, from 0.15 under the eyes.
                        let mut at = arrow.position;
                        at.y -= f64::from(0.15f32) - 0.1;
                        self.launch_angled_rocket(at, &ammo, arrow.owner, |_| arrow.velocity);
                    } else {
                        Self::arrow_from_ammo(&ammo, &mut arrow);
                        self.launch_mob_projectile(arrow);
                    }
                }
                MobEvent::CrossbowLoad {
                    mob,
                    weapon,
                    at,
                    pitch,
                } => {
                    if self.mob_crossbow_ammo(mob, &weapon) {
                        self.audio.play(
                            "minecraft:item.crossbow.loading_end",
                            Some(at),
                            1.0,
                            pitch,
                        );
                    }
                }
                MobEvent::Spawn(mut m) => {
                    self.next_entity += 1;
                    m.id = self.next_entity;
                    self.mobs.push(*m);
                    self.native_mobs_joined(self.mobs.len() - 1, false);
                }
                MobEvent::SpawnKind { kind, at, setup } => {
                    if let Some(id) = self.spawn_mob(
                        &kind,
                        at,
                        setup.persistent,
                        if setup.skip_finalize {
                            SpawnSite::Raw
                        } else {
                            SpawnSite::Finalizing(
                                horizon_client::mobs::spawn_rules::Reason::MobSummoned,
                            )
                        },
                    ) {
                        self.apply_setup(id, &setup);
                    }
                }
                MobEvent::Convert { from, kind, params } => {
                    self.convert_to(&from, kind, params, |_, _| {});
                }
                MobEvent::ThrowItem {
                    mob,
                    stack,
                    at,
                    motion,
                    pickup_delay,
                } => {
                    // `thrownItem.setThrower(the mob)`.
                    self.spawn_item(stack, at, motion, pickup_delay);
                    let thrower = self.mob_uuid(mob);
                    if let Some(thrown) = self.just_spawned_item() {
                        thrown.thrower = Some(thrower);
                    }
                }
                MobEvent::DropStack { mob, stack, at } => {
                    self.native_entity_drop_item(mob, &stack.item, stack.count as i32, at);
                    self.spawn_item_own(stack, at, 10);
                }
                MobEvent::DropItem {
                    mob,
                    item,
                    count,
                    at,
                } => {
                    self.native_entity_drop_item(mob, &item, count as i32, at);
                    // The ItemEntity constructor's motion, from the entity's
                    // own random (not the level's), so spawnAtLocation drops
                    // (a fox's berries, a panda's held food, a goat's horn)
                    // land apart.
                    self.spawn_item_own(
                        ItemStack {
                            item,
                            count,
                            damage: 0,
                            extra: None,
                        },
                        at,
                        10,
                    );
                }
                MobEvent::DropGoatHorn {
                    mob,
                    screaming,
                    at,
                    motion,
                } => {
                    let instrument =
                        horizon_client::mobs::goat::create_horn(self.mob_uuid(mob), screaming);
                    let mut horn = ItemStack::new("minecraft:goat_horn", 1);
                    horn.data_mut().set_other(
                        "minecraft:instrument",
                        Some(horizon_core::nbt::Tag::String(instrument.into())),
                    );
                    // Goat.dropHorn's ItemEntity constructor leaves pickupDelay at
                    // 0 (no setDefaultPickUpDelay).
                    self.spawn_item(horn, at, motion, 0);
                }
                MobEvent::DropGift { table, at } => {
                    let stacks =
                        self.res
                            .items()
                            .loot_table(table, false, None, &mut self.world.random);
                    for stack in stacks {
                        // The ItemEntity constructor's motion, from the
                        // entity's own random (not the level's).
                        self.spawn_item_own(stack, at, 10);
                    }
                }
                // `Level.setBlockAndUpdate`: neighbour and shape updates, so a crop
                // above trampled farmland breaks and drops.
                MobEvent::SetBlock { pos, state } => self.level_set(pos, state)?,
                MobEvent::EndermanPlaceBlock {
                    mob,
                    pos,
                    carried,
                    occupants,
                } => {
                    self.enderman_place_block(mob, pos, carried, &occupants)?;
                }
                MobEvent::PlaceBlock { mob, pos, state } => {
                    self.mob_place_block(mob, pos, state)?
                }
                MobEvent::CropRaided {
                    pos,
                    state,
                    old,
                    mob,
                } => {
                    self.level_set_with_flags(
                        pos,
                        state,
                        horizon_world::level::flags::UPDATE_CLIENTS,
                    )?;
                    if let Some(old) = old {
                        self.post_block_game_event(
                            "minecraft:block_change",
                            pos,
                            Some(Who::Mob(mob)),
                            None,
                        );
                        self.level_event_at(2001, pos, old.0 as i32);
                    }
                }
                MobEvent::CrackTurtleEgg { pos } => self.crack_turtle_egg(pos)?,
                MobEvent::BedOccupied { pos, occupied } => self.set_bed_occupied(pos, occupied),
                MobEvent::SetDoorOpen { pos, open } => {
                    let pos = BlockPos::new(pos[0], pos[1], pos[2]);
                    self.with_level(|b, level, ticks| b.set_door_open(level, ticks, pos, open));
                    self.process_level_events()?;
                }
                MobEvent::TakeBlock { mob, pos, state } => {
                    // The block must still be the one the enderman picked.
                    if self.world.block(pos[0], pos[1], pos[2]) == state
                        && self.mob_may_destroy(mob, pos)
                    {
                        let air = self.world.air;
                        self.level_set(pos, air)?;
                    } else if let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob) {
                        m.extra.carried = None;
                    }
                }
                MobEvent::IfBroke {
                    mob,
                    effects,
                    otherwise,
                } => {
                    let follow = if broke.contains(&mob) {
                        effects
                    } else {
                        otherwise
                    };
                    actions.nested.push(NormalActionFrame {
                        events: follow.into(),
                        broke: Vec::new(),
                        actor: Some(mob),
                    });
                }
                MobEvent::Jump { mob } => {
                    if let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob) {
                        m.ai.jumping = true;
                    }
                }
                MobEvent::BreakDoor { mob, pos } => {
                    if !self.mob_may_destroy(mob, pos) {
                        continue;
                    }
                    // BreakDoorGoal.tick: removeBlock, then level events 1021
                    // and 2001 with the state that is there now.
                    let kind = self
                        .mobs
                        .iter()
                        .find(|m| m.id == mob)
                        .map(super::native_hooks::mob_kind)
                        .unwrap_or_default();
                    let air = self.world.air;
                    let at = BlockPos::new(pos[0], pos[1], pos[2]);
                    self.with_level(|b, level, ticks| b.set_block(level, ticks, at, air, 3));
                    self.process_level_events()?;
                    self.level_event_at(1021, pos, 0);
                    let left = self.world.block(pos[0], pos[1], pos[2]);
                    self.level_event_at(2001, pos, left.0 as i32);
                    self.native_entity_break_door(mob, &kind, pos);
                }
                MobEvent::DestroyProgress { mob, pos, progress } => {
                    self.server_destroy_block_progress(mob as i32, pos, progress);
                }
                MobEvent::LevelEvent { id, pos, data } => self.level_event_at(id, pos, data),
                MobEvent::SetBlockName {
                    mob,
                    pos,
                    name,
                    effect,
                    flags,
                } => {
                    if effect {
                        // EatBlockGoal passes the block's default state constant
                        // (Blocks.GRASS_BLOCK.defaultBlockState()), not the live state
                        // (which could be snowy).
                        let old = self.world.block(pos[0], pos[1], pos[2]);
                        let state = self
                            .pack
                            .assets
                            .registry
                            .default_state(&self.res.facts().get(old).name)
                            .unwrap_or(old);
                        self.level_event_at(2001, pos, state.0 as i32);
                    }
                    if let Some(state) = self.pack.assets.registry.default_state(name) {
                        let old_name = self
                            .res
                            .facts()
                            .get(self.world.block(pos[0], pos[1], pos[2]))
                            .name
                            .clone();
                        let mut cancel = false;
                        if let Some(mob) = mob
                            && let Some(kind) = self
                                .mobs
                                .iter()
                                .find(|m| m.id == mob)
                                .map(|m| m.kind.id.clone())
                        {
                            let from = self.world.block(pos[0], pos[1], pos[2]);
                            self.native_entity_change_block_pre(
                                crate::game::native_hooks::EntityChangeBlockPreCall {
                                    player: None,
                                    entity: Some(mob),
                                    kind: &kind,
                                    pos,
                                    from,
                                    to: state,
                                    cancel: &mut cancel,
                                },
                            );
                        }
                        if !cancel {
                            self.level_set_with_flags(pos, state, flags)?;
                            if let Some(mob) = mob
                                && let Some(m) = self.mobs.iter().find(|m| m.id == mob)
                            {
                                let kind = m.kind.id.clone();
                                self.native_entity_change_block(mob, &kind, pos, &old_name, name);
                            }
                        }
                    }
                }
                MobEvent::Infest {
                    pos,
                    infested,
                    poof,
                } => {
                    let registry = &self.pack.assets.registry;
                    let old = self.world.block(pos[0], pos[1], pos[2]);
                    if let Some(mut state) = registry.state(old).cloned() {
                        let (from, to) = if infested {
                            ("minecraft:", "minecraft:infested_")
                        } else {
                            ("minecraft:infested_", "minecraft:")
                        };
                        state.name = state.name.replacen(from, to, 1);
                        if let Ok(id) = registry.resolve(&state) {
                            self.level_set(pos, id)?;
                        }
                    }
                    if let Some(mob) = poof {
                        self.mob_poof(mob);
                    }
                }
                MobEvent::BreakBlock {
                    pos,
                    drop,
                    by,
                    breaker,
                } => {
                    if let Some(mob) = by {
                        if !self.mob_may_destroy(mob, pos) {
                            continue;
                        }
                        broke.push(mob);
                    }
                    let old = self.world.block(pos[0], pos[1], pos[2]);
                    // Block.spawnDestroyParticles: fire breaks leave nothing.
                    if !self.res.facts().get(old).has_class("BaseFireBlock") {
                        self.level_event_at(2001, pos, old.0 as i32);
                    }
                    if drop {
                        let entity = by.or(breaker);
                        let mut drops = self.roll_block_loot(pos, old, entity, Default::default());
                        self.native_block_loot(pos, old, None, None, None, &mut drops);
                        let dropped = self.entity_block_dropped(pos, old, entity, drops);
                        let canceled = dropped.canceled;
                        self.spawn_block_drops(pos, dropped);
                        // InfestedBlock.spawnAfterBreak runs after the drops,
                        // unless BlockDropsEvent canceled it.
                        if !canceled
                            && self
                                .res
                                .facts()
                                .get(old)
                                .name
                                .starts_with("minecraft:infested_")
                            && self
                                .ui
                                .rules
                                .bool_at(horizon_client::gamerules::vanilla::BLOCK_DROPS)
                        {
                            let at = [
                                f64::from(pos[0]) + 0.5,
                                f64::from(pos[1]),
                                f64::from(pos[2]) + 0.5,
                            ];
                            if let Some(id) =
                                self.spawn_mob("minecraft:silverfish", at, false, SpawnSite::Plain)
                            {
                                self.mob_poof(id);
                            }
                        }
                    }
                    // Level.destroyBlock: setBlock(fluidState.createLegacyBlock(), 3),
                    // so water stays water and a waterlogged block leaves its water.
                    let b = self.res.behaviours();
                    let left = b.legacy_block(b.fluid(old));
                    if left != old {
                        self.level_set(pos, left)?;
                    }
                }
                MobEvent::Particles { kind, at, count } => {
                    // Mob particle bursts (hearts, smoke, splashes) around a point.
                    if let Some(ty) = horizon_client::particles::ParticleType::from_name(kind) {
                        if kind.starts_with("item_") {
                            self.ensure_item_particle_sprites();
                        }
                        let options = horizon_client::particles::ParticleOptions::simple(ty);
                        if ty == horizon_client::particles::ParticleType::SonicBoom {
                            self.lan_sonic_boom(&options, at);
                        }
                        // Count 0: one particle exactly there, at rest.
                        if count == 0 {
                            self.particles.add(&options, at, [0.0; 3]);
                        }
                        for _ in 0..count {
                            let r = &mut self.world.random;
                            let p = [
                                at[0] + (f64::from(r.next_float()) - 0.5),
                                at[1] + (f64::from(r.next_float()) - 0.5) * 0.5,
                                at[2] + (f64::from(r.next_float()) - 0.5),
                            ];
                            let v = [
                                r.next_gaussian() * 0.02,
                                r.next_gaussian() * 0.02,
                                r.next_gaussian() * 0.02,
                            ];
                            self.particles.add(&options, p, v);
                        }
                    }
                }
                MobEvent::AroundSelf {
                    mob,
                    event,
                    kind,
                    feet,
                    width,
                    height,
                    burst,
                } => {
                    self.lan_entity_event(mob as i32, event, feet);
                    self.particles_around(kind, feet, [width, height], burst)
                }
                MobEvent::ParticleAt { kind, at, velocity } => {
                    if let Some(ty) = horizon_client::particles::ParticleType::from_name(kind) {
                        let options = horizon_client::particles::ParticleOptions::simple(ty);
                        self.particles.add(&options, at, velocity);
                    }
                }
                MobEvent::ParticleColored {
                    at,
                    color: [r, g, b],
                } => {
                    let options = horizon_client::particles::ParticleOptions::colored(
                        horizon_client::particles::ParticleType::EntityEffect,
                        [r, g, b, 1.0],
                    );
                    self.particles.add(&options, at, [0.0; 3]);
                }
                MobEvent::Experience { at, amount } => self.add_orb(at, amount),
                MobEvent::AxolotlSupportPlayer { mob } => {
                    let previous = self
                        .potions
                        .effects
                        .get("regeneration")
                        .map_or(0, |e| e.duration);
                    if (0..=2399).contains(&previous) {
                        self.potions
                            .effects
                            .add(horizon_client::effects::EffectInstance::new(
                                "regeneration",
                                (previous + 100).min(2400),
                                0,
                            ));
                    }
                    self.potions
                        .effects
                        .list
                        .retain(|e| e.id != "mining_fatigue");
                    let source = self
                        .mobs
                        .iter()
                        .find(|m| m.id == mob)
                        .map(|m| self.mob_entity(m));
                    self.trigger_effects_changed_by(source);
                }
                MobEvent::DolphinTreasure { mob } => {
                    // ServerLevel.findNearestMapStructure(#dolphin_located, 50).
                    let Some(at) = self.mobs.iter().find(|m| m.id == mob).map(|m| {
                        [m.position.x, m.position.y, m.position.z].map(|v| v.floor() as i32)
                    }) else {
                        continue;
                    };
                    let names = self.structure_tag("minecraft:dolphin_located");
                    let found = self
                        .end
                        .locator()
                        .filter(|_| !names.is_empty())
                        .and_then(|t| t.locate(&names, at, 50));
                    if let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob) {
                        m.extra.home = found.as_ref().map(|(_, p)| *p);
                    }
                    // DolphinSwimToTreasureGoal.start: entity event 38 once it has a treasure.
                    if found.is_some() {
                        self.mob_event_particles(mob, 38);
                    }
                }
                MobEvent::EffectPlayer {
                    effect,
                    ticks,
                    amplifier,
                    ambient,
                } => {
                    self.give_mob_effect(effect, ticks, amplifier, ambient);
                }
                MobEvent::EffectMob {
                    target,
                    effect,
                    ticks,
                    amplifier,
                } => {
                    if let Some(t) = self.mobs.iter_mut().find(|m| m.id == target) {
                        t.add_effect(effect, ticks, amplifier);
                    }
                }
                MobEvent::Ignite(target, seconds) => self.ignite_entity(target, seconds)?,
                MobEvent::ZombieIgnite(target, seconds) if self.combat.hit_landed => {
                    self.ignite_entity(target, seconds)?
                }
                MobEvent::ZombieIgnite(..) => {}
                MobEvent::Breed {
                    parent,
                    partner,
                    equine,
                    parent_variant,
                    wolf_voice,
                    experience,
                } => {
                    let child = self.breed(
                        parent,
                        partner,
                        equine,
                        parent_variant,
                        wolf_voice,
                        experience,
                    );
                    self.native_entity_breed(parent, partner, child);
                }
                MobEvent::LockGaze { mob, partner } => {
                    if let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob) {
                        horizon_client::mobs::animal_brain::lock_gaze(m, partner);
                    }
                }
                MobEvent::BreedTarget { mob, partner } => {
                    if let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob) {
                        horizon_client::mobs::animal_brain::set_breed_partner(m, partner);
                    }
                }
                MobEvent::Retreat { mob, from, at } => {
                    if let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob) {
                        horizon_client::mobs::piglin::receive_retreat(m, from, at);
                    }
                }
                MobEvent::StartRiding { rider, vehicle } => {
                    if horizon_client::mobs::riders::start_riding(&mut self.mobs, rider, vehicle) {
                        self.seat_mob(rider);
                        // sendEventAndTriggers: the server players already on it.
                        self.note_started_riding(vehicle);
                    }
                }
                // Applied after the fish's tick (`apply_brain_events`).
                MobEvent::StartFollowing { .. }
                | MobEvent::SchoolSize { .. }
                | MobEvent::PufferfishSting { .. } => {}
                MobEvent::LeashBreak { mob, drop, sound } => {
                    self.tick_break_leash(mob, drop, sound)
                }
                MobEvent::NotifyLeashHolder { holder } => {
                    if let Some(m) = self.mobs.iter_mut().find(|m| m.id == holder) {
                        horizon_client::mobs::happy_ghast::notify_leash_holder(m);
                    }
                }
                MobEvent::Vibration {
                    from,
                    to,
                    ticks,
                    source,
                } => self.mob_vibration_particle(source, from, to, ticks),
                MobEvent::DigParticles { at } => self.warden_dig_particles(at),
                MobEvent::BlockBreakEffect { pos, block } => {
                    // Turtle.aiStep broadcasts LevelEvent 2001, not the
                    // particles themselves: the event carries the particles and
                    // the sand's break sound, and reaches the clients the same
                    // way a broken block's does.
                    let state = self.world.block(block[0], block[1], block[2]);
                    self.level_event_at(2001, pos, state.0 as i32);
                }
                MobEvent::Darkness { at, radius } => self.apply_darkness(at, radius),
                MobEvent::CreakingHurt {
                    home,
                    by_player,
                    id,
                } => self.creaking_hurt(home, id, by_player),
                MobEvent::CreakingTearDown { center, spread } => {
                    self.creaking_tear_down(center, spread)
                }
                MobEvent::CureZombieVillager { mob } => self.cure_zombie_villager(mob),
                MobEvent::Reinforcements { mob, target } => self.call_reinforcements(mob, target),
                MobEvent::VillagerMeet { mob, other } => self.villager_meet(mob, other),
                MobEvent::VillagerTrade { mob, player } => self.villager_traded_by(mob, player),
                MobEvent::VillagerOffers { mob } => {
                    if let Some(i) = self.mobs.iter().position(|m| m.id == mob) {
                        self.ensure_offers(i);
                    }
                }
                MobEvent::VillagerBirth {
                    parent,
                    partner,
                    child,
                } => self.villager_birth(parent, partner, child),
                MobEvent::WorkAtComposter { mob, pos } => self.villager_work_at_composter(mob, pos),
                MobEvent::GolemCheck { mob, needed } => self.villager_golem_check(mob, needed),
                MobEvent::Firework { at, color, flight } => {
                    self.villager_firework(at, color, flight)
                }
                MobEvent::ThrowGift { table, from, to } => self.throw_gift(&table, from, to),
                MobEvent::PatrolTarget { mob, target } => {
                    if let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob) {
                        horizon_client::mobs::raider::set_patrol_target(m, target);
                    }
                }
                MobEvent::RaiderTarget {
                    mob,
                    target,
                    aggressive,
                } => {
                    let player = self
                        .lan
                        .as_ref()
                        .and_then(|host| host.mob_players.get(&mob).copied())
                        .or(self.lan_acting);
                    self.set_event_ai_target(mob, Some(target), player);
                    if let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob) {
                        m.ai.aggressive |= aggressive;
                    }
                }
                MobEvent::PandaSneezed { mob } => self.panda_sneezed(mob),
                MobEvent::FallToPassengers {
                    mob,
                    distance,
                    modifier,
                } => self.fall_to_passengers(mob, distance, modifier),
                MobEvent::ShedScute { mob, at, pitch } => {
                    // The mob already checked `shouldDropLoot` (a baby, `mob_drops`).
                    let table = "minecraft:gameplay/armadillo_shed";
                    let drops = self.sequence_loot(table, table, None);
                    for d in drops {
                        self.spawn_at_location(Vec3::new(at[0], at[1], at[2]), 0.0, d);
                    }
                    self.audio.play(
                        "minecraft:entity.armadillo.scute_drop",
                        Some(at),
                        1.0,
                        pitch,
                    );
                    let ctx = self.who_context(Some(Who::Mob(mob)), None);
                    self.post_game_event("minecraft:entity_place", at, ctx);
                }
                MobEvent::LayEgg { mob, at, pitch } => {
                    // Chicken.aiStep: `dropFromGiftLootTable` says whether
                    // anything dropped; only then the sound and the game event.
                    if self.drop_gift_loot(mob, "minecraft:gameplay/chicken_lay", at) {
                        self.audio
                            .play("minecraft:entity.chicken.egg", Some(at), 1.0, pitch);
                        let ctx = self.who_context(Some(Who::Mob(mob)), None);
                        self.post_game_event("minecraft:entity_place", at, ctx);
                    }
                }
                MobEvent::GiftLoot { mob, table, at } => {
                    // Turtle.ageBoundaryReached and Panda.afterSneeze: only
                    // with mob loot on.
                    if matches!(
                        table,
                        "minecraft:gameplay/turtle_grow" | "minecraft:gameplay/panda_sneeze"
                    ) && !self
                        .ui
                        .rules
                        .bool_at(horizon_client::gamerules::vanilla::MOB_DROPS)
                    {
                        continue;
                    }
                    self.drop_gift_loot(mob, table, at);
                }
            }
        }
    }
    /// `TurtleEggBlock.decreaseEggs` after an entity's step or landing won its
    /// roll (`destroyEgg`): the break sound and the remaining eggs (a stack of
    /// one is destroyed after it).
    pub(super) fn crack_turtle_egg(&mut self, pos: [i32; 3]) -> Result<()> {
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let eggs = self
            .pack
            .assets
            .registry
            .state(state)
            .filter(|s| s.name == "minecraft:turtle_egg")
            .and_then(|s| s.properties.get("eggs"))
            .and_then(|e| e.parse::<i32>().ok());
        if let Some(eggs) = eggs {
            let at = BlockPos::new(pos[0], pos[1], pos[2]);
            self.with_level(|b, level, ticks| {
                b.turtle_egg_destroyed_by_player(level, ticks, at, state);
                if eggs <= 1 {
                    b.destroy_block(level, ticks, at, false);
                }
            });
            self.process_level_events()?;
        }
        Ok(())
    }
    /// `TurtleEggBlock.randomTick` at the last hatch stage: one baby turtle
    /// per egg, a day from grown, with the egg as its home (the sound, the
    /// crumbs and the block's removal came from the level).
    pub(super) fn hatch_turtles(
        &mut self,
        pos: [i32; 3],
        state: horizon_world::registry::BlockStateId,
    ) {
        let eggs = self
            .pack
            .assets
            .registry
            .state(state)
            .and_then(|s| s.properties.get("eggs"))
            .and_then(|v| v.parse::<i32>().ok())
            .unwrap_or(1);
        for i in 0..eggs {
            let at = [
                f64::from(pos[0]) + 0.3 + f64::from(i) * 0.2,
                f64::from(pos[1]),
                f64::from(pos[2]) + 0.3,
            ];
            if let Some(id) = self.spawn_mob_raw("minecraft:turtle", at, true)
                && let Some(m) = self.mobs.iter_mut().find(|m| m.id == id)
            {
                m.extra.age = -24000;
                m.extra.home = Some(pos);
                m.yaw = 0.0;
                m.body_yaw = 0.0;
                m.head_yaw = 0.0;
            }
        }
    }
    fn apply_setup(&mut self, id: u32, setup: &SpawnSetup) {
        let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) else {
            return;
        };
        if setup.baby {
            m.extra.age = -24000;
        }
        if setup.nausea {
            m.add_effect("minecraft:nausea", 200, 0);
        }
        if setup.size > 0 {
            m.extra.size = setup.size;
        }
        m.yaw = setup.yaw;
        m.body_yaw = setup.yaw;
        m.head_yaw = setup.yaw;
        m.velocity = Vec3::new(setup.velocity[0], setup.velocity[1], setup.velocity[2]);
        if let Some(h) = setup.health {
            m.health = h;
        }
        if let Some(v) = setup.variant {
            m.extra.variant = v;
        }
        if let Some(owner) = setup.owner_mob {
            m.extra.counter2 = owner as i32;
        }
        if let Some(home) = setup.home {
            m.extra.home = Some(home);
        }
        if let Some(life) = setup.life {
            m.extra.egg_time = life;
        }
    }
    /// `Animal.finalizeSpawnChildFromBreeding`'s single experience orb, drawn
    /// from the breeding parent after the rule check.
    fn award_breeding_experience(&mut self, parent: u32, at: [f64; 3]) {
        let mob_drops = self
            .ui
            .rules
            .bool_at(horizon_client::gamerules::vanilla::MOB_DROPS);
        if let Some(amount) = self
            .mobs
            .iter_mut()
            .find(|m| m.id == parent)
            .and_then(|m| horizon_client::mobs::breeding_experience(m, mob_drops))
        {
            self.add_orb(at, amount);
        }
    }
    fn breeding_orb(
        &mut self,
        parent: u32,
        at: [f64; 3],
        experience: horizon_client::mobs::BreedExperience,
    ) {
        match experience {
            horizon_client::mobs::BreedExperience::Deferred => {
                self.award_breeding_experience(parent, at)
            }
            horizon_client::mobs::BreedExperience::Drawn(Some(amount)) => self.add_orb(at, amount),
            horizon_client::mobs::BreedExperience::Drawn(None) => {}
        }
    }
    /// `ServerPlayer.tellNeutralMobsThatIDied`: the mobs within 32 blocks
    /// across and 10 up and down get `NeutralMob.playerDied`, when the
    /// `forgive_dead_players` rule is on. A mob's `Who::Player` is the guest
    /// it is bound to in `bindings` (game/network/lan_mobs.rs), else the host's
    /// player, so only the mobs whose binding is `conn` (`None` for the
    /// host's player) hear it.
    pub(in crate::game) fn tell_neutral_mobs_player_died(
        &mut self,
        feet: [f64; 3],
        conn: Option<u32>,
    ) {
        // A client of someone else's world has the server's mobs only to show.
        if self.remote.is_some()
            || !self
                .ui
                .rules
                .bool_at(horizon_client::gamerules::vanilla::FORGIVE_DEAD_PLAYERS)
        {
            return;
        }
        let block = feet.map(f64::floor);
        let area = Aabb::new(
            Vec3::new(block[0], block[1], block[2]),
            Vec3::new(block[0] + 1.0, block[1] + 1.0, block[2] + 1.0),
        )
        .inflate(Vec3::new(32.0, 10.0, 32.0));
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let ids: Vec<u32> = self
            .mobs
            .iter()
            .filter(|m| {
                host.mob_players.get(&m.id).copied() == conn
                    && m.bounds().intersects(area)
                    && horizon_client::mobs::ai::forgives_dead_player(m)
            })
            .map(|m| m.id)
            .collect();
        for id in ids {
            let Some(epoch) = self.native_lifecycle.epoch() else {
                if let Some(mob) = self.mobs.iter_mut().find(|m| m.id == id) {
                    horizon_client::mobs::ai::forgive_dead_player(mob);
                }
                continue;
            };
            let mob_ref = mod_api::MobRef::new(mod_api::EntityRef::new(epoch, id));
            let change = self
                .native_mob_target_event(mob_ref, None, false)
                .and_then(|target| self.native_mob_bind_target(id, target))
                .map_or(
                    horizon_client::mobs::TargetChange::Keep,
                    horizon_client::mobs::TargetChange::Set,
                );
            if let Some(mob) = self.mobs.iter_mut().find(|m| m.id == id) {
                horizon_client::mobs::ai::forgive_dead_player_with_change(mob, change);
            }
        }
    }
    /// Animal.spawnChildFromBreeding
    fn breed(
        &mut self,
        parent: u32,
        partner: u32,
        equine: Option<Box<horizon_client::mobs::equine::Offspring>>,
        parent_variant: Option<bool>,
        wolf_voice: Option<u8>,
        experience: horizon_client::mobs::BreedExperience,
    ) -> Option<u32> {
        let p = self.mobs.iter().find(|m| m.id == parent).cloned()?;
        // BabyEntitySpawnEvent: not for an egg (turtle, frog, sniffer).
        if !matches!(
            p.species(),
            Species::Turtle | Species::Frog | Species::Sniffer
        ) && mod_api::__private::listened(mod_api::HookId::BabyEntitySpawn)
        {
            let mule = matches!(
                (
                    p.species(),
                    self.mobs
                        .iter()
                        .find(|m| m.id == partner)
                        .map(|m| m.species())
                ),
                (Species::Horse, Some(Species::Donkey)) | (Species::Donkey, Some(Species::Horse))
            );
            let kind = if mule {
                "minecraft:mule".to_owned()
            } else {
                super::native_hooks::mob_kind(&p)
            };
            let conn = self
                .love_cause
                .get(&parent)
                .or(self.love_cause.get(&partner))
                .copied();
            let player = conn.and_then(|c| self.killer_uuid(c));
            let player_entity_id = conn.and_then(|c| self.player_entity_id_by_connection(c));
            let mut cancel = false;
            self.native_baby_entity_spawn(
                parent,
                partner,
                player,
                player_entity_id,
                &kind,
                &mut cancel,
            );
            if cancel {
                breeding_credit(&mut self.love_cause, parent, partner);
                if let Some(q) = self.mobs.iter_mut().find(|m| m.id == partner) {
                    q.extra.age = 6000;
                    q.extra.in_love = 0;
                }
                return None;
            }
        }
        let q = self.mobs.iter().find(|m| m.id == partner).cloned();
        // Sniffer.spawnChildFromBreeding: an egg instead of a baby.
        if p.species() == Species::Sniffer {
            self.finalize_breeding(&p, partner, None, OrbOrder::BeforeChild);
            self.lay_sniffer_egg(&p);
            return None;
        }
        // TurtleBreedGoal.breed: the goal gave the mother her egg; only the
        // experience is left, and only with mob loot on.
        if matches!(p.species(), Species::Turtle | Species::Frog) {
            self.finalize_breeding(&p, partner, None, OrbOrder::BeforeChild);
            // One `new ExperienceOrb`, not split as `ExperienceOrb.award` does.
            self.award_breeding_experience(parent, [p.position.x, p.position.y, p.position.z]);
            return None;
        }
        let coin = if horizon_client::mobs::draws_parent_variant(p.species()) {
            match parent_variant {
                Some(choice) => choice,
                None => {
                    let parent = self.mobs.iter_mut().find(|m| m.id == parent)?;
                    parent.random.next_boolean()
                }
            }
        } else {
            // Other species either draw their own variants or do not need a coin.
            false
        };
        // Axolotl.getBreedOffspring: useRareVariant (one in 1200) makes it blue,
        // else either parent's variant. The draws come from the mother's random.
        let axolotl = if p.species() == Species::Axolotl {
            let other = q.as_ref().map_or(p.extra.variant, |q| q.extra.variant);
            self.mobs.iter_mut().find(|m| m.id == parent).map(|m| {
                if m.random.next_int_bound(1200).unwrap_or(1) == 0 {
                    // Axolotl.Variant.getRareSpawnVariant: the one blue, still a draw.
                    let _ = m.random.next_int_bound(1);
                    4
                } else if m.random.next_boolean() {
                    p.extra.variant
                } else {
                    other
                }
            })
        } else {
            None
        };
        // MushroomCow.getOffspringVariant, from the mother's random: the other
        // colour 1 time in 1024 when both parents share one, otherwise either
        // parent's colour by a coin flip.
        let mooshroom = if p.species() == Species::Mooshroom {
            let mate = q.as_ref().map_or(p.extra.variant, |q| q.extra.variant);
            let own = p.extra.variant;
            self.mobs.iter_mut().find(|m| m.id == parent).map(|m| {
                if own == mate && m.random.next_int_bound(1024).unwrap_or(1) == 0 {
                    if own == 1 { 0 } else { 1 }
                } else if m.random.next_boolean() {
                    own
                } else {
                    mate
                }
            })
        } else {
            None
        };
        // A horse and a donkey have a mule.
        let kind = match (p.species(), q.as_ref().map(|q| q.species())) {
            (Species::Horse, Some(Species::Donkey)) | (Species::Donkey, Some(Species::Horse)) => {
                "minecraft:mule".to_owned()
            }
            _ => p.kind.id.clone(),
        };
        let mut child = self.create_mob_reason(
            &kind,
            [p.position.x, p.position.y, p.position.z],
            true,
            SpawnSite::Raw,
            None,
        )?;
        'offspring: {
            let child = &mut child;
            child.extra.age = -24000;
            // No Mob.finalizeSpawn either: no follow range bonus, right-handed.
            spawning::bred_offspring(child);
            // getBreedOffspring creates the baby without finalizeSpawn: it keeps
            // the default (`classic`) sound variant.
            child.extra.sound_variant = None;
            // Fox.getBreedOffspring runs no finalizeSpawn: nothing in the
            // baby fox's mouth.
            if p.species() == Species::Fox {
                child.extra.set_main_hand(None);
            }
            // Fox.FoxBreedGoal.breed: the baby trusts the players who fed its
            // parents into love.
            if p.species() == Species::Fox {
                let parent_cause = self
                    .love_cause
                    .get(&parent)
                    .copied()
                    .and_then(|conn| self.killer_uuid(conn));
                let partner_cause = self
                    .love_cause
                    .get(&partner)
                    .copied()
                    .and_then(|conn| self.killer_uuid(conn));
                horizon_client::mobs::fox::trust_breeders(
                    child,
                    parent_cause.map(horizon_client::lan::playerdata::uuid_ints),
                    partner_cause.map(horizon_client::lan::playerdata::uuid_ints),
                );
            }
            // Goat.getBreedOffspring: GoatAi.initMemories, then the baby
            // screams if the parent a coin toss picks does, or on a 2% roll.
            if p.species() == Species::Goat {
                child.extra.lost_horns = [false; 2];
                horizon_client::mobs::finalize::goat_init_memories(child, &mut self.world.random);
                let pick_parent = self.world.random.next_boolean();
                let parent_screams = if pick_parent {
                    p.extra.screaming
                } else {
                    q.as_ref().unwrap_or(&p).extra.screaming
                };
                child.extra.screaming = parent_screams || self.world.random.next_double() < 0.02;
            }
            // Inheritance was drawn synchronously in the breeding goal,
            // before the parent's later same-tick aiStep rolls.
            if let Some(o) = equine {
                horizon_client::mobs::equine::apply_offspring(child, &o);
                break 'offspring;
            }
            // Camel.getBreedOffspring creates a camel with default attributes.
            if horizon_client::mobs::camel::is_camel(p.species()) {
                break 'offspring;
            }
            if p.species() == Species::Cat {
                // Cat.getBreedOffspring.
                let other = q.as_ref().unwrap_or(&p);
                child.extra.variant = if coin {
                    p.extra.variant
                } else {
                    other.extra.variant
                };
                if p.extra.tame {
                    child.extra.tame = true;
                    child.extra.owner = p.extra.owner;
                    let mixed = dye_mix(
                        &self.loop_context,
                        world_recipes!(self),
                        &mut self.world.random,
                        p.extra.color,
                        other.extra.color,
                    );
                    child.extra.color = mixed;
                }
                break 'offspring;
            }
            if p.species() == Species::Wolf {
                // Wolf.getBreedOffspring: a parent's variant; tame parents
                // give a tame baby with their collar colours mixed.
                let other = q.as_ref().unwrap_or(&p);
                child.extra.variant = if coin {
                    p.extra.variant
                } else {
                    other.extra.variant
                };
                if p.extra.tame {
                    horizon_client::mobs::tame::set_tame(child, true);
                    child.extra.owner = p.extra.owner;
                    let mixed = dye_mix(
                        &self.loop_context,
                        world_recipes!(self),
                        &mut self.world.random,
                        p.extra.color,
                        other.extra.color,
                    );
                    child.extra.color = mixed;
                }
                child.extra.sound_variant = wolf_voice.or_else(|| self.bred_wolf_voice(parent));
                break 'offspring;
            }
            // Panda.getBreedOffspring: genes from both parents.
            if p.species() == Species::Panda {
                horizon_client::mobs::panda::inherit(child, &p, q.as_ref().unwrap_or(&p));
                break 'offspring;
            }
            // Axolotl.getBreedOffspring: the drawn variant, and the baby is persistent.
            if let Some(variant) = axolotl {
                child.extra.variant = variant;
                child.persistent = true;
                break 'offspring;
            }
            // Nautilus.getBreedOffspring: a tame parent's baby is tame too.
            if p.species() == Species::Nautilus && p.extra.tame {
                child.extra.tame = true;
                child.extra.owner = p.extra.owner;
                child.persistent = true;
            }
            // Offspring inherit a parent's variant (pigs/cows/chickens) or mixed wool.
            child.extra.variant = p.extra.variant;
            // Fox, Pig, Cow and Chicken.getBreedOffspring: either parent's variant.
            if horizon_client::mobs::draws_parent_variant(p.species())
                && p.species() != Species::Cat
                && !coin
            {
                child.extra.variant = q.as_ref().map_or(p.extra.variant, |q| q.extra.variant);
            }
            if let Some(variant) = mooshroom {
                child.extra.variant = variant;
            }
            // Sheep.getBreedOffspring: the lamb's wool mixes both parents' dyes.
            if p.species() == Species::Sheep {
                let other = q.as_ref().unwrap_or(&p);
                let mixed = dye_mix(
                    &self.loop_context,
                    world_recipes!(self),
                    &mut self.world.random,
                    p.extra.color,
                    other.extra.color,
                );
                child.extra.color = mixed;
            }
        }
        let order = if horizon_client::mobs::orb_after_child(p.species()) {
            OrbOrder::AfterChild
        } else {
            OrbOrder::BeforeChild
        };
        self.finalize_breeding(&p, partner, Some(&child), order);
        let at = [p.position.x, p.position.y, p.position.z];
        match order {
            OrbOrder::AfterChild => {
                let joined = self.insert_spawned_mob(child);
                self.mob_event_particles(parent, 18);
                self.breeding_orb(parent, at, experience);
                joined
            }
            OrbOrder::BeforeChild => {
                self.breeding_orb(parent, at, experience);
                self.insert_spawned_mob(child)
            }
        }
    }
    /// `MobEffectUtil.addEffectToPlayersAround` for an elder guardian:
    /// mining fatigue III for five minutes unless a stronger or longer one
    /// is on, with the curse sound and the elder's apparition.
    fn elder_curse(&mut self, at: [f64; 3], silent: bool) {
        let mut cursed = Vec::new();
        self.for_each_player(|g| {
            if g.elder_curse_player(at, silent) {
                cursed.push(g.lan_acting);
            }
        });
        // A guest's client shows the apparition and plays the curse itself
        // (ClientboundGameEventPacket.GUARDIAN_ELDER_EFFECT), the sound only
        // for 1: `isSilent() ? 0.0F : 1.0F`.
        if let Some(host) = self.lan.as_ref() {
            for conn in cursed.into_iter().flatten() {
                host.hub.send_to(
                    conn,
                    &horizon_protocol::packets::play::Clientbound::GameEvent(
                        horizon_protocol::packets::play::GameEvent {
                            event: horizon_client::lan::hub::game_event::GUARDIAN_ELDER_EFFECT,
                            param: crate::mobs::guardian::elder_curse_param(silent),
                        },
                    ),
                );
            }
        }
    }
    /// The curse on the player in place; whether it took.
    fn elder_curse_player(&mut self, at: [f64; 3], silent: bool) -> bool {
        if self.creative || self.spectator() {
            return false;
        }
        let Some(p) = self.player.as_ref() else {
            return false;
        };
        let d = p.position.subtract(Vec3::new(at[0], at[1], at[2]));
        if d.x * d.x + d.y * d.y + d.z * d.z >= 2500.0 || p.health.dead_or_dying() {
            return false;
        }
        // The player's effects are kept by their bare name. A curse as strong
        // that won't end within the next minute stays (endsWithin(1199)).
        let active = self.potions.effects.get("mining_fatigue");
        if active.is_some_and(|e| e.amplifier >= 2 && !e.ends_within(1199)) {
            return false;
        }
        let at = [p.position.x, p.position.y, p.position.z];
        self.give_player_effect("minecraft:mining_fatigue", 6000, 2);
        if self.lan_acting.is_none() {
            // ClientPacketListener.handleGameEvent: the apparition always, the
            // curse sound unless the elder is silent.
            if crate::mobs::guardian::elder_curse_param(silent) == 1.0 {
                self.audio
                    .play("minecraft:entity.elder_guardian.curse", Some(at), 1.0, 1.0);
            }
            self.particles.elder_guardian();
        }
        true
    }
    /// `hurtServer` on the player. `from` is the source's position (none: no
    /// direction), `known`: the causing mob when it is given (`/damage`),
    /// else the melee attacker is the mob nearest to `from`.
    pub(super) fn hurt_player_from(
        &mut self,
        amount: f32,
        from: Option<[f64; 3]>,
        damage_type: &str,
        known: Option<Option<u32>>,
        by_creative_player: bool,
    ) -> bool {
        let post_attack_if_alive = std::mem::take(&mut self.combat.post_attack_if_alive);
        let rocket = std::mem::take(&mut self.combat.rocket_hit);
        let pierce = std::mem::take(&mut self.combat.piercing_hit);
        let melee = self.combat.melee_mob.take();
        // A projectile entity dealt it: `#is_projectile`, or a firework
        // rocket (its `fireworks` type is not in the tag).
        let projectile = rocket
            || world_stations!(self)
                .enchantments
                .damage_tags
                .get("minecraft:is_projectile")
                .is_some_and(|t| t.contains(damage_type));
        let blocking = self.player_blocking(from, damage_type, pierce);
        let raised = blocking.is_some();
        // Melee: the attacker stands at `from` (another player's hit has no mob behind it).
        let attacker = match known {
            Some(known) => known,
            None if melee.is_some() => melee,
            None => from
                .filter(|_| !projectile && damage_type != "minecraft:player_attack")
                .and_then(|from| {
                    self.mobs
                        .iter()
                        .filter(|m| m.alive())
                        .min_by(|a, b| {
                            let d = |m: &horizon_client::mobs::Mob| {
                                (m.position.x - from[0]).powi(2) + (m.position.z - from[2]).powi(2)
                            };
                            d(a).total_cmp(&d(b))
                        })
                        .map(|m| m.id)
                }),
        };
        // The causing entity for the death message.
        let owner = self.combat.projectile_owner.take();
        // The mob that caused the hit, else the attack the caller landed it in (a projectile's
        // owner, another player's blow): `combat.attack`, read through `current_attack`.
        let mut attack = match attacker.or(owner) {
            Some(id) => self.attack_of(id as i32),
            None => self.current_attack(),
        };
        attack.projectile = self.combat.attack().projectile;
        attack.direct = self.combat.attack().direct;
        attack.position = from;
        attack.projectile_weapon = self.combat.attack().projectile_weapon.clone();
        if let Some(a) = attacker.or(owner) {
            self.note_attacker(damage_type, a);
        }
        if let Some((id, name)) = self.combat.projectile_attacker.take() {
            self.note_named_attacker(damage_type, id, name);
        }
        // The projectile is the direct entity: named when it has no owner.
        if let Some(name) = self.combat.projectile_direct.take() {
            self.note_direct_entity(damage_type, name);
        }
        // DamageSource.scalesWithDifficulty: a mob (or its projectile's
        // owner) caused the hit, not a player.
        let living_cause = attacker.or(owner).is_some();
        // The native damage hook's causing entity: the one given, or the
        // owner of the projectile that hit (`projectile_owner` outlives a
        // projectile's other hits), not the guessed melee attacker.
        let owner = owner.filter(|_| projectile);
        let causing = known.map_or_else(
            || attacker.or(owner).map_or(attack.attacker, |id| id as i32),
            |known| known.map_or(0, |id| id as i32),
        );
        let mut outcome = horizon_client::health::BlockOutcome::default();
        // The hit's own push, held back until after the shield's (`hurtServer`
        // runs `applyItemBlocking` before `dealDefaultKnockback`).
        let mut default_push: Option<([f64; 2], HurtSource<'_>)> = None;
        let events_before = self.player.as_ref().map_or(0, |p| p.events.len());
        let damage_outcome = if let Some(id) = self.health_resources.damage.id(damage_type) {
            self.hurt_player_equipment(
                id,
                amount,
                blocking,
                living_cause,
                causing,
                by_creative_player,
                &attack,
            )
        } else {
            Ok(Default::default())
        };
        if let Ok(o) = damage_outcome
            && let Some(player) = self.player.as_mut()
            && {
                outcome = o;
                o.hurt
            }
        {
            // LivingEntity.hurtServer's dealDefaultKnockback: not inside the
            // hurt cooldown (a stronger hit there only adds its difference).
            if o.knockback {
                // No source position: a random direction.
                let (xd, zd) = from.map_or((0.0, 0.0), |from| {
                    (from[0] - player.position.x, from[2] - player.position.z)
                });
                // dealDefaultKnockback's `indicateDamage`, unless blocked.
                if o.blocked <= 0.0 {
                    player.health.indicate_damage(xd, zd, player.yaw);
                }
                let source = HurtSource {
                    damage_type,
                    causing,
                    direct: attack.projectile_direct(damage_type),
                    position: None,
                };
                default_push = Some(([xd, zd], source));
            }
            // The thorns hurt the entity that caused the hit, if there is one.
            // Thorns' own hit is no attack, so nothing answers it
            // (`doPostAttackEffects` runs from the attacks, not `hurtServer`).
            let killed = self.player.as_ref().is_some_and(|p| {
                super::wither::killed(true, p.health.dead_or_dying(), p.health.totem_used)
            });
            if let Some(a) = attacker.or(owner)
                && !(post_attack_if_alive && killed)
                && damage_type != "minecraft:thorns"
            {
                self.thorns(a);
            }
        }
        // LivingEntity.playSecondaryHurtSound: a thorns hit that landed whole
        // sounds where the player stands, for everyone around.
        if let Some(p) = self.player.as_ref()
            && p.events.get(events_before..).is_some_and(|new| {
                new.iter().any(|e| {
                    matches!(
                        e,
                        horizon_client::motion::MotionEvent::Health(HealthEvent::Phase(
                            "thorns-sound"
                        ))
                    )
                })
            })
        {
            let at = [p.position.x, p.position.y, p.position.z];
            self.audio
                .play("minecraft:enchant.thorns.hit", Some(at), 1.0, 1.0);
        }
        // LivingEntity.hurtServer: ENTITY_HURT_PLAYER, unless it returned
        // early (invulnerable, or inside the hurt cooldown).
        // A hit a native mod canceled records no damage (hurtServer never ran).
        self.combat.forget_noted(outcome.canceled);
        if outcome.hurt || outcome.sound {
            let source = attacker
                .or(owner)
                .and_then(|id| self.mob_index.get(&self.mobs, id))
                .map(|m| self.mob_entity(m));
            self.trigger_entity_hurt_player(&horizon_client::advancements::DamageView {
                kind: damage_type.to_owned(),
                // Arrows and the like name no direct entity here; a
                // rocket's burst names the rocket.
                direct: if rocket {
                    Some(horizon_client::advancements::EntityView {
                        kind: "minecraft:firework_rocket".to_owned(),
                        position: from.unwrap_or_default(),
                        ..Default::default()
                    })
                } else {
                    source.clone().filter(|_| !projectile)
                },
                is_direct: !projectile,
                source,
                original: amount,
                actual: outcome.dealt,
                blocked: outcome.blocked > 0.0,
            });
        }
        if raised {
            let source = HurtSource {
                damage_type,
                causing,
                direct: attack.projectile_direct(damage_type),
                position: None,
            };
            self.after_block(outcome, (attacker, projectile), source, &attack);
        }
        // `LivingEntity.dealDefaultKnockback`: after the item's block push, so
        // a half-blocked hit pushes the defender back the way vanilla does.
        if let Some(([xd, zd], source)) = default_push {
            self.knock_back_player(f64::from(0.4_f32), xd, zd, source, &attack, amount);
            if let Some(p) = self.player.as_mut()
                && let Some(r) = p.hurt_log.last_mut()
            {
                r.knockback = Some([p.velocity.x, p.velocity.y, p.velocity.z]);
            }
        }
        outcome.hurt
    }
    /// A status effect from a mob, arrow or splash potion (`addEffect`).
    pub(in crate::game) fn give_player_effect(
        &mut self,
        effect: &'static str,
        ticks: i32,
        amplifier: i32,
    ) {
        self.give_mob_effect(effect, ticks, amplifier, false);
    }
    /// A mob's effect on the player. Creative and spectator players shrug off
    /// the damaging effects here, but the nautilus's ambient Breath is plain
    /// `addEffect` and lands in every game mode.
    fn give_mob_effect(&mut self, effect: &'static str, ticks: i32, amplifier: i32, ambient: bool) {
        let immune = self.creative || self.spectator();
        if let Some(instance) = mob_effect_instance(effect, ticks, amplifier, ambient, immune) {
            self.potions.effects.add(instance);
        }
    }
    /// `WitherRoseBlock.entityInside` for the player: on a server level that
    /// isn't peaceful, `getBeeInteractionEffect` (Wither, 40 ticks) unless the
    /// player is invulnerable to wither damage (creative, spectator). A
    /// server's player gets it from the server.
    pub(super) fn wither_rose_contact(&mut self) {
        if self.remote.is_some() || self.ui.difficulty == 0 {
            return;
        }
        self.give_player_effect("minecraft:wither", 40, 0);
    }
    pub(in crate::game) fn hurt_player_by(&mut self, damage_type: &str, amount: f32) {
        let mut canceled = false;
        let attack = self.current_attack();
        if self.player.is_some()
            && let Some(id) = self.health_resources.damage.id(damage_type)
            && let Ok(o) = self.hurt_player_equipment(id, amount, None, false, 0, false, &attack)
        {
            canceled = o.canceled;
        }
        self.combat.forget_noted(canceled);
    }
    /// `Player.aiStep` touching arrows in its box grown by (1, 0.5, 1):
    /// `AbstractArrow.playerTouch`, then `Player.take`'s pickup sound at the
    /// arrow (`ClientPacketListener.handleTakeItemEntity`).
    pub(super) fn touch_arrows(&mut self) {
        use horizon_client::arrows::Touch;
        let Some(p) = self.player.as_ref() else {
            return;
        };
        if p.health.dead_or_dying() || self.spectator() {
            return;
        }
        let w = f64::from(p.dimensions.width) / 2.0;
        let reach = Aabb::new(
            Vec3::new(p.position.x - w, p.position.y, p.position.z - w),
            Vec3::new(
                p.position.x + w,
                p.position.y + f64::from(p.dimensions.height),
                p.position.z + w,
            ),
        )
        .inflate(Vec3::new(1.0, 0.5, 1.0));
        // Player.hasInfiniteMaterials.
        let creative = self.creative;
        let mut projectiles = self.creatures.projectiles.lift();
        #[cfg(test)]
        super::entity::taken_pass_tests::after_take(self);
        let mut picked = Vec::new();
        projectiles.retain(|shot| {
            let Some(a) = shot.arrow.as_ref().filter(|a| !a.removed) else {
                return true;
            };
            let at = shot.position;
            let bounds = Aabb::new(
                Vec3::new(at.x - 0.25, at.y, at.z - 0.25),
                Vec3::new(at.x + 0.25, at.y + 0.5, at.z + 0.25),
            );
            if !bounds.intersects(reach) {
                return true;
            }
            let taken = match a.player_touch(creative) {
                Touch::Ignore => false,
                Touch::Take => true,
                // Inventory.add: a creative player takes what does not fit.
                Touch::Give(stack) => self.add_stack_to_inventory(stack).is_none() || creative,
            };
            if taken {
                let r = &mut self.world.random;
                let pitch = (r.next_float() - r.next_float()) * 1.4 + 2.0;
                self.audio.play(
                    "minecraft:entity.item.pickup",
                    Some([at.x, at.y, at.z]),
                    0.2,
                    pitch,
                );
                picked.push(shot.clone());
            }
            !taken
        });
        self.creatures.projectiles.restore(
            projectiles,
            horizon_client::join_seq::RestoreOrder::SurvivorsFirst,
        );
        for shot in picked {
            self.add_pickup(
                super::pickups::Taken::Arrow(Box::new(shot)),
                super::pickups::Collector::Player,
            );
        }
    }

    pub(super) fn tick_projectiles_now(&mut self) -> Result<()> {
        if self.creatures.projectiles.is_empty() {
            self.flush_lan_explosions();
            return self.flush_wind_bursts();
        }
        let mut projectiles = self.creatures.projectiles.lift();
        let ticking = self.entity_ticking_area();
        let mut hits = Vec::new();
        // The breezes that deflected a projectile, and where.
        let mut deflections: Vec<(u32, [f64; 3])> = Vec::new();
        let mut arrow_events = Vec::new();
        let mut shoot_events = Vec::new();
        let mut trails = Vec::new();
        let mut bubble_tops = Vec::new();
        let mut snow_contacts = Vec::new();
        let mut section_steps = section_steps::begin(self);
        let mut keep_flags = vec![true; projectiles.len()];
        let mut winds = std::mem::take(&mut self.creatures.pickable_projectile_targets);
        // The impacts the mods may cancel (`native_projectile_impact`), on a server.
        let hear = self.native_hears_impacts();
        let mut impacts: Vec<Impact> = Vec::new();
        let mut vehicle_winds = self.vehicle_winds();
        let targets = self.projectile_targets();
        let breezes: Vec<u32> = self
            .mobs
            .iter()
            .filter(|m| m.alive() && m.species() == Species::Breeze)
            .map(|m| m.id)
            .collect();
        // Mobs' projectiles also hit the guests (game/network/lan_mobs.rs).
        let guests = self.lan_projectile_targets();
        let has_guests = !guests.is_empty();
        let targets_guests = if has_guests {
            let mut targets_guests = targets.clone();
            targets_guests.extend(guests);
            targets_guests
        } else {
            Vec::new()
        };
        let border = Some(self.border.bounds());
        // Every guest, not only the ones a projectile can hit: a dead or
        // spectating guest's pearl or arrow still resolves to them.
        let guest_entities = self.lan_guest_entities();
        // ThrownEnderpearl.tick: a dead player's pearl is discarded in
        // place of its tick while ender_pearls_vanish_on_death is on.
        let vanish = self
            .ui
            .rules
            .bool_at(horizon_client::gamerules::vanilla::ENDER_PEARLS_VANISH_ON_DEATH);
        let host_dead = self.player_view().is_some_and(|p| !p.alive);
        let dead_guests: Vec<u32> = self.lan.as_ref().map_or_else(Vec::new, |h| {
            h.hub
                .guests
                .iter()
                .filter(|g| g.dead)
                .map(|g| g.conn)
                .collect()
        });
        // `EntityTickList.forEach`: a projectile that joined during this
        // tick's entity pass (a mob's shot) first ticks on the next one; a
        // player's or a dispenser's, which join before the pass, tick now.
        let through = self.ticked_entities_through;
        for i in 0..projectiles.len() {
            if projectiles[i].id > through {
                continue;
            }
            // ServerLevel.tick: losing an entity-ticking ticket must not
            // advance projectile flags or motion while its chunk unloads.
            let at = projectiles[i].position;
            if !ticking(at.x.floor() as i32, at.z.floor() as i32) {
                projectiles.update(i, |p| p.previous = at);
                continue;
            }
            // A ghast's fireball or a wind charge is pickable, so a pearl, an
            // arrow or a trident can strike it (`Projectile.isPickable`,
            // `Projectile.onHit`).
            winds.clear();
            if matches!(projectiles[i].kind, ProjectileKind::EnderPearl)
                || projectiles[i].arrow.is_some()
            {
                pickable_projectile_hitboxes_into(&projectiles, &keep_flags, i, &mut winds);
            }
            let mut redirect = None;
            {
                let w = self.mob_world_base(&[]);
                let rain = self.arrow_rain();
                let scene = self.arrow_scene(&rain);
                let pw = ProjWorld {
                    w: &w,
                    targets: &targets,
                    border,
                };
                let pw_guests = ProjWorld {
                    w: &w,
                    targets: &targets_guests,
                    border,
                };
                projectiles.update(i, |p| {
                    if vanish
                        && matches!(p.kind, ProjectileKind::EnderPearl)
                        && p.owner == Some(Who::Player)
                        && p.guest.map_or(host_dead, |c| dead_guests.contains(&c))
                    {
                        keep_flags[i] = false;
                        return;
                    }
                    // The projectile as it was, to fly on if a mod cancels its impact.
                    let before = (hear && !p.stuck).then(|| p.clone());
                    let wind_before = vehicle_winds.before(p);
                    let mob_shot = !matches!(p.owner, Some(Who::Player));
                    // A player's arrows, snowballs and eggs meet every player; a
                    // guest's flies from that guest (Projectile.getOwner), not
                    // from the host, until it has left them.
                    let pvp = !mob_shot && super::lan_pvp::meets_players(&p.kind);
                    let shooter = p
                        .guest
                        .filter(|_| pvp)
                        .and_then(|c| guest_entities.iter().find(|(g, _)| *g == c))
                        .map(|&(_, id)| Who::Mob(id));
                    if shooter.is_some() {
                        p.owner = shooter;
                    }
                    let use_guests = (mob_shot || pvp) && has_guests;
                    let (keep, hit) = match (p.arrow.is_some(), use_guests, winds.is_empty()) {
                        (true, false, true) => horizon_client::arrows::tick(p, &scene, pw.targets),
                        (true, true, true) => {
                            horizon_client::arrows::tick(p, &scene, pw_guests.targets)
                        }
                        (true, with_guests, false) => {
                            let mut targets = if with_guests {
                                pw_guests.targets.to_vec()
                            } else {
                                pw.targets.to_vec()
                            };
                            targets.extend_from_slice(&winds);
                            horizon_client::arrows::tick(p, &scene, &targets)
                        }
                        (false, false, true) => p.tick(&pw),
                        (false, true, true) => p.tick(&pw_guests),
                        (false, pvp_shot, false) => {
                            let mut targets = if pvp_shot {
                                pw_guests.targets.to_vec()
                            } else {
                                pw.targets.to_vec()
                            };
                            targets.extend_from_slice(&winds);
                            p.tick(&ProjWorld {
                                w: if pvp_shot { pw_guests.w } else { pw.w },
                                targets: &targets,
                                border,
                            })
                        }
                    };
                    if (p.is_thrown() && !matches!(p.kind, ProjectileKind::LlamaSpit))
                        || p.is_arrow() && p.arrow.as_ref().is_none_or(|a| !a.no_physics)
                    {
                        for block in horizon_client::powder_snow::contacts(
                            p.previous,
                            p.position,
                            p.bounds(),
                            false,
                            |at| w.block_name(at.x, at.y, at.z) == "minecraft:powder_snow",
                        ) {
                            snow_contacts.push((block, p.position, p.velocity));
                        }
                    }
                    if shooter.is_some() {
                        p.owner = Some(Who::Player);
                    }
                    if let Some(at) = p.take_shoot_event() {
                        shoot_events.push((p.owner, p.guest, [at.x, at.y, at.z]));
                    }
                    // AbstractHurtingProjectile.tick ends with createParticleTrail.
                    if super::remote_projectiles::has_smoke_trail(&p.kind) {
                        trails.push(p.position);
                    }
                    // ThrownEnderpearl.onAboveBubbleColumn / onInsideBubbleColumn.
                    if keep && hit == Hit::None && matches!(p.kind, ProjectileKind::EnderPearl) {
                        let (at, h) = (p.position, 0.125);
                        horizon_client::motion::bubble_column_push_over(
                            w.world,
                            w.registry,
                            w.facts,
                            w.collision,
                            Aabb::new(
                                Vec3::new(at.x - h, at.y, at.z - h),
                                Vec3::new(at.x + h, at.y + 2.0 * h, at.z + h),
                            ),
                            &mut p.velocity,
                            |pos| bubble_tops.push(pos),
                        );
                    }
                    let events_from = arrow_events.len();
                    if let Some(a) = p.arrow.as_mut() {
                        arrow_events.extend(a.events.drain(..).map(|e| {
                            if e == horizon_client::arrows::ArrowEvent::ShootEvent {
                                shoot_events.push((p.owner, p.guest, p.position.components()));
                            }
                            (p.position, e)
                        }));
                    }
                    let wind = matches!(
                        p.kind,
                        ProjectileKind::WindCharge | ProjectileKind::BreezeWindCharge
                    );
                    // Entity.deflection: a breeze turns every projectile but wind charges away.
                    if let Hit::Entity {
                        who: Who::Mob(id),
                        at,
                    } = hit
                        && !wind
                        && breezes.contains(&id)
                    {
                        super::wind::reverse(p);
                        if p.arrow.is_some() {
                            horizon_client::arrows::finish_entity_hit(
                                p,
                                &scene,
                                EntityHit::Deflected,
                            );
                            let a = p.arrow.as_mut().expect("an arrow");
                            arrow_events.extend(a.events.drain(..).map(|e| {
                                if e == horizon_client::arrows::ArrowEvent::ShootEvent {
                                    shoot_events.push((p.owner, p.guest, p.position.components()));
                                }
                                (p.position, e)
                            }));
                        }
                        deflections.push((id, [at.x, at.y, at.z]));
                        return;
                    }
                    // AbstractWindCharge.tick: out of the world's top it bursts.
                    if wind && keep && p.position.y > 350.0 {
                        hits.push((
                            p.clone(),
                            Hit::Block {
                                pos: [0; 3],
                                at: p.position,
                                face: [0; 3],
                            },
                        ));
                        keep_flags[i] = false;
                        return;
                    }
                    vehicle_winds.met(wind_before, hit);
                    if hit != Hit::None {
                        if let Some(before) = before {
                            impacts.push(Impact {
                                index: i,
                                before,
                                hit,
                                hit_slot: hits.len(),
                                events: events_from..arrow_events.len(),
                            });
                        }
                        hits.push((p.clone(), hit));
                    }
                    keep_flags[i] = keep;
                    // Projectile.onHit: the redirectable projectile it hit (a ghast's
                    // fireball or a wind charge) turns before its own tick.
                    if let Hit::Entity {
                        who: Who::Mob(id), ..
                    } = hit
                        && let Some(j) = pickable_projectile_hit(id)
                    {
                        redirect = Some((j, (p.owner, p.owner_uuid, p.guest)));
                    }
                });
            }
            if let Some((j, (owner, owner_uuid, guest))) = redirect {
                let look = self.owner_look(owner, guest);
                projectiles.update(j, |wind| {
                    super::deflect::aim_deflect(wind, look, owner, owner_uuid, guest);
                });
            }
            keep_flags[i] = keep_flags[i]
                && section_steps::record(
                    self,
                    &mut section_steps,
                    projectiles.get(i),
                    at,
                    keep_flags[i],
                );
        }
        self.creatures.pickable_projectile_targets = winds;
        for (block, position, movement) in snow_contacts {
            self.powder_snow_particle(block, position, movement);
        }
        let last_ticked_join = projectiles.iter().map(|p| p.join).max();
        let listed = keep_flags.clone();
        let mut idx = 0;
        if impacts.is_empty() {
            projectiles.retain(|_| {
                let keep = keep_flags[idx];
                idx += 1;
                keep
            });
        } else {
            // Keep the owners of impacts that may replay alive until the verdict.
            let mut removed = projectiles
                .extract_if(|_| {
                    let take = !keep_flags[idx];
                    idx += 1;
                    take
                })
                .collect();
            self.native_settle_impacts(
                &mut projectiles,
                &mut removed,
                &listed,
                impacts,
                &mut hits,
                &mut arrow_events,
            );
        }
        for (owner, guest, at) in shoot_events {
            self.with_projectile_dispatch(&mut projectiles, |game| {
                game.projectile_shoot_event(owner, guest, at);
            });
        }
        self.smoke_trails(trails);
        self.dragon_breath_trails();
        for pos in bubble_tops {
            self.bubble_column_particles(pos);
        }
        // Breeze.onProjectileDeflect: level.playSound(null, breeze,
        // BREEZE_DEFLECT, HOSTILE, 1, 1).
        for (breeze, at) in deflections {
            let sound = LevelSound::new("minecraft:entity.breeze.deflect", at, 1.0, 1.0)
                .in_source(SoundSource::Hostile);
            self.play_bound_sound(sound, breeze, None);
        }
        for (at, event) in arrow_events {
            self.arrow_event(at, event);
        }
        // `Entity.applyEffectsFromBlocks` sweeps the original cohort by join identity:
        // settlement can remove rows or append hook spawns. Contacts need the lent
        // level here, after the read-only borrow, each still in this batch's tick.
        for i in 0..projectiles.len() {
            if last_ticked_join.is_none_or(|last| projectiles[i].join > last) {
                continue;
            }
            let mut swept = None;
            projectiles.update(i, |p| {
                swept = Some((p.previous, p.position));
            });
            let _ = swept;
            projectiles.update(i, |p| self.sweep_projectile_inside(p));
        }
        self.creatures.projectiles.restore(
            projectiles,
            horizon_client::join_seq::RestoreOrder::SpawnsFirst,
        );
        section_steps::finish(self, section_steps);
        for (p, hit) in hits {
            let Some((p, hit)) = self.broken_vehicle_retick(p, hit, &mut vehicle_winds) else {
                continue;
            };
            // A player's arrow on another player lands as the victim's hurt.
            if let Some(victim) = self.projectile_pvp_victim(&p, hit) {
                self.projectile_hits_player(&p, hit, victim)?;
                continue;
            }
            // A guest's projectile lands as theirs: a pearl takes them, what
            // it hurts answers to them. One hitting the host (`Who::Player`
            // here) stays the host's hit.
            match p.guest {
                Some(conn)
                    if !matches!(
                        hit,
                        Hit::Entity {
                            who: Who::Player,
                            ..
                        }
                    ) =>
                {
                    self.guest_projectile_hit(conn, &p, hit)?;
                }
                _ => self.projectile_hit(&p, hit)?,
            }
        }
        self.flush_lan_explosions();
        self.flush_wind_bursts()
    }
    /// `LivingEntity.hurtServer`'s `setLastHurtByPlayer` for a projectile a
    /// player owns (a fireball they turned back): the kill is theirs, so a
    /// guest's (the host on its server thread too) reaches their statistics
    /// and `player_killed_entity`, as the arrow's hit does.
    fn player_hurt_mob(&mut self, p: &Projectile, id: u32) {
        if p.owner == Some(Who::Player) {
            self.remember_mob_player_credit(id);
        }
    }
    /// `LivingEntity.hurtServer` for a snowball, egg or pearl (`kind`) that hurt the mob `id`,
    /// 0 damage included: the hit is its owner's, and a player's fires PLAYER_HURT_ENTITY.
    fn thrown_hurt_mob(&mut self, p: &Projectile, id: u32, kind: &str, amount: f32) {
        self.player_hurt_mob(p, id);
        if p.owner == Some(Who::Player)
            && let Some(m) = self.mobs.iter().find(|m| m.id == id)
        {
            let victim = self.mob_entity(m);
            let direct = horizon_client::advancements::EntityView {
                kind: kind.into(),
                position: [p.position.x, p.position.y, p.position.z],
                ..Default::default()
            };
            self.player_hit(&victim, "minecraft:thrown", Some(direct), amount);
        }
    }
    /// `projectile_hit` for a projectile the guest `conn` shot, run as them;
    /// as the host's when they have left.
    fn guest_projectile_hit(&mut self, conn: u32, p: &Projectile, hit: Hit) -> Result<()> {
        let result = self.as_guest(conn, |g| g.projectile_hit(p, hit));
        // The mob hit now answers to the guest (LivingEntity.lastHurtByPlayer).
        if result.is_some()
            && self
                .lan
                .as_ref()
                .and_then(|host| host.hub.guest(conn))
                .is_some_and(|guest| guest.participates_online())
            && let Hit::Entity {
                who: Who::Mob(id), ..
            } = hit
            && self.mobs.iter().any(|m| m.id == id)
            && let Some(host) = self.lan.as_mut()
        {
            host.mob_players.insert(id, conn);
        }
        match result {
            Some(r) => r,
            None => self.projectile_hit(p, hit),
        }
    }
    /// `EnchantmentHelper.doPostAttackEffects` after the player's shot `p`
    /// (its `direct` entity, a hit of `damage_type`) hurt the mob `who`: the
    /// victim's own post-attack enchantments answer the player (Thorns).
    pub(super) fn shot_post_attack(
        &mut self,
        p: &Projectile,
        who: Who,
        direct: &str,
        damage_type: &str,
    ) {
        if let Who::Mob(id) = who {
            self.mob_post_attack(id, p.owner, Some(direct), Some(damage_type));
        }
    }
    /// A projectile's hurt of `amount` on the boat or minecart `id`
    /// (`VehicleEntity.hurtServer`): even 0 shakes it (`Snowball`, `ThrownEgg`,
    /// `ThrownEnderpearl` `.onHitEntity`), 5 from a small fireball destroys it.
    pub(super) fn hurt_vehicle(&mut self, p: &Projectile, id: u32, amount: f32) {
        let creative = p.owner == Some(Who::Player) && self.creative;
        let damage_kind = self.projectile_damage_kind(p);
        let damage_type = source_damage_type(Damage {
            kind: damage_kind,
            attacker: p.owner,
            from: None,
            attacker_species: None,
        });
        if let Some(i) = self.boats.iter().position(|b| b.id == id) {
            let kind = self.boats[i].entity_type();
            if !self.vehicle_refuses_projectile(p, id, &kind, damage_type) {
                self.hurt_boat(i, amount, creative);
            }
        } else if let Some(i) = self.carts.iter().position(|c| c.id == id) {
            let kind = self.carts[i].kind.id();
            if !self.vehicle_refuses_projectile(p, id, kind, damage_type) {
                self.hurt_cart(i, amount, creative, false);
            }
        }
    }
    /// Whether the per-source invulnerability hook rejects a projectile hit on a vehicle.
    fn vehicle_refuses_projectile(
        &mut self,
        p: &Projectile,
        id: u32,
        kind: &str,
        damage_type: &str,
    ) -> bool {
        let causing = match p.owner {
            Some(Who::Player) => match p.guest {
                Some(conn) => self.command_guest(conn).map_or(0, |g| g.entity_id),
                None => self.own_entity_id(),
            },
            Some(Who::Mob(id)) => id as i32,
            None => 0,
        };
        let source = horizon_client::health::HurtSource {
            damage_type,
            causing,
            direct: p.id as i32,
            position: Some([p.position.x, p.position.y, p.position.z]),
        };
        let creative = match p.owner {
            Some(Who::Player) => match p.guest {
                Some(conn) => self.command_guest(conn).is_some_and(|g| g.creative()),
                None => self.creative,
            },
            _ => false,
        };
        let vanilla = self.saved_refuses_damage(id, damage_type, creative);
        crate::game::entity::damage_level_of!(self, scoped).invulnerability_check(
            horizon_client::health::Hurting::Entity(id as i32, kind, source),
            vanilla,
        )
    }
    /// `BellBlock.onProjectileHit`: a projectile that strikes the bell proper
    /// rings it, and a player's shot counts as the player's ring.
    pub(super) fn bell_projectile_hit(
        &mut self,
        p: &ProjectileHitter,
        pos: [i32; 3],
        at: Vec3,
        face: [i32; 3],
    ) {
        let Some(face) = Direction::ALL.into_iter().find(|d| d.normal() == face) else {
            return;
        };
        let block = BlockPos::new(pos[0], pos[1], pos[2]);
        let rang = self
            .with_level(|b, level, ticks| b.bell_projectile_hit(level, ticks, block, face, at.y));
        if rang {
            let _ = self.process_level_events();
            if p.owner == Some(Who::Player) {
                self.award_custom_stat("bell_ring", 1);
            }
        }
    }
    /// `AmethystBlock.onProjectileHit`: a projectile that strikes an amethyst
    /// block rings `block.amethyst_block.chime` there, pitched by the level random.
    pub(super) fn amethyst_projectile_hit(&mut self, pos: [i32; 3]) {
        let state = self.world.block(pos[0], pos[1], pos[2]);
        if &*self.res.facts().get(state).name != "minecraft:amethyst_block" {
            return;
        }
        let pitch = 0.5 + self.world.random.next_float() * 1.2;
        self.level_events
            .push(horizon_world::level::LevelEvent::Sound {
                pos: BlockPos::new(pos[0], pos[1], pos[2]),
                sound: "minecraft:block.amethyst_block.chime".into(),
                volume: 1.0,
                pitch,
            });
        let _ = self.process_level_events();
    }
    /// `onProjectileHit` of chorus flowers, decorated pots and speleothems
    /// (pointed dripstone, sulfur spikes): an impact projectile
    /// (`#impact_projectiles`) breaks the block when `Projectile.mayInteract`
    /// and `Projectile.mayBreak` allow it, which takes the
    /// `projectiles_can_break_blocks` rule and a shooter that may break blocks
    /// (`canBreakBlockInAdventureMode`: a resolved mob's arrow needs its
    /// actual projectile predicate and mob_griefing). Other owner/kind paths
    /// retain their existing policy. A pot is cracked first (flags 260).
    fn projectile_breaks_block(&mut self, p: &Projectile, pos: [i32; 3]) {
        use horizon_client::command::args::GameMode;
        let impact = matches!(
            p.kind,
            ProjectileKind::Arrow { .. }
                | ProjectileKind::Trident
                | ProjectileKind::Snowball
                | ProjectileKind::Thrown(_)
                | ProjectileKind::Egg
                | ProjectileKind::SmallFireball
                | ProjectileKind::LargeFireball { .. }
                | ProjectileKind::WitherSkull { .. }
                | ProjectileKind::WindCharge
                | ProjectileKind::BreezeWindCharge
        );
        use horizon_client::arrows::OwnerLookup;
        let shooter_may_break = match p.owner {
            Some(Who::Player) => self.mode != GameMode::Adventure,
            Some(Who::Mob(_)) => {
                matches!(p.kind, ProjectileKind::Arrow { .. })
                    && self
                        .ui
                        .rules
                        .bool_at(horizon_client::gamerules::vanilla::MOB_GRIEFING)
                    && p.arrow
                        .as_ref()
                        .and_then(|a| a.can_break.as_ref())
                        .is_some_and(|predicate| {
                            super::game_mode::projectile_predicate_supported(predicate)
                                && self.adventure_predicate_at(predicate, pos)
                        })
            }
            None => !p.owner_uuid.is_some_and(|u| self.non_player_entity(u)),
        };
        if !impact
            || !shooter_may_break
            || !self
                .ui
                .rules
                .bool_at(horizon_client::gamerules::vanilla::PROJECTILES_CAN_BREAK_BLOCKS)
        {
            return;
        }
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let at = BlockPos::new(pos[0], pos[1], pos[2]);
        // `destroyBlock(pos, true, projectile)`; a speleothem's names no entity.
        let mut entity = Some(p.id);
        match &*self.res.facts().get(state).name {
            "minecraft:chorus_flower"
                if matches!(
                    p.kind,
                    ProjectileKind::Arrow { .. } | ProjectileKind::Snowball
                ) =>
            {
                // `ChorusFlowerBlock.onProjectileHit` supplies the projectile,
                // even when it has no owner, as the direct block's loot entity.
                let _ = self.destroy_block_by(pos, Some(p.id));
                return;
            }
            "minecraft:chorus_flower" => {}
            // SpeleothemBlock.onProjectileHit (pointed dripstone and sulfur spikes): a
            // thrown trident faster than 0.6 breaks it.
            "minecraft:pointed_dripstone" | "minecraft:sulfur_spike"
                if matches!(p.kind, ProjectileKind::Trident)
                    && p.arrow.as_ref().is_some_and(|a| a.hit_speed > 0.6) =>
            {
                entity = None;
            }
            "minecraft:decorated_pot" => {
                let registry = &self.pack.assets.registry;
                let Some(mut pot) = registry.state(state).cloned() else {
                    return;
                };
                pot.properties.insert("cracked".into(), "true".into());
                let Ok(cracked) = registry.resolve(&pot) else {
                    return;
                };
                self.with_level(|b, level, ticks| b.set_block(level, ticks, at, cracked, 260));
            }
            _ => return,
        }
        let _ = self.destroy_block_by(pos, entity);
    }
    /// What an arrow's tick asked for: its particles and sounds.
    pub(in crate::game) fn arrow_event(
        &mut self,
        at: Vec3,
        event: horizon_client::arrows::ArrowEvent,
    ) {
        use horizon_client::arrows::{ArrowEvent, ArrowParticle, ArrowSound, BlockReaction};
        use horizon_client::particles::{ParticleOptions, ParticleType};
        let kind = |k| match k {
            ArrowParticle::Bubble => ParticleType::Bubble,
            ArrowParticle::Crit => ParticleType::Crit,
            ArrowParticle::Splash => ParticleType::Splash,
        };
        match event {
            ArrowEvent::Particle {
                kind: k,
                position,
                velocity,
            } => self.particles.add(
                &ParticleOptions::simple(kind(k)),
                [position.x, position.y, position.z],
                [velocity.x, velocity.y, velocity.z],
            ),
            // Arrow.tick: `broadcastEntityEvent(this, 0)`. The window and the
            // guests burst the particles themselves (`remote_arrow_puff`).
            ArrowEvent::PotionPuff { id, .. } => {
                self.lan_entity_event(id as i32, 0, [at.x, at.y, at.z]);
            }
            ArrowEvent::Sound {
                sound,
                volume,
                pitch,
            } => {
                let name = match sound {
                    ArrowSound::Splash => "minecraft:entity.generic.splash",
                    ArrowSound::HitGround => "minecraft:entity.arrow.hit",
                    ArrowSound::CrossbowHit => "minecraft:item.crossbow.hit",
                    ArrowSound::Extinguish => "minecraft:entity.generic.extinguish_fire",
                    ArrowSound::TridentHitGround => "minecraft:item.trident.hit_ground",
                    ArrowSound::TridentHit => "minecraft:item.trident.hit",
                };
                // Each one is the projectile's own `Entity.playSound`, on `getSoundSource()`:
                // NEUTRAL, which neither AbstractArrow nor ThrownTrident overrides. The
                // name-based guess would put `entity.arrow.*` and `item.*` on PLAYERS.
                self.audio
                    .play_on(6, name, Some([at.x, at.y, at.z]), volume, pitch);
            }
            ArrowEvent::BubbleColumnParticles { pos } => self.bubble_column_particles(pos),
            ArrowEvent::Block {
                pos,
                reaction,
                owner,
            } => match reaction {
                BlockReaction::PrimeTnt => {
                    // `TntBlock.onProjectileHit`: the arrow's owner lights it.
                    if self.tnt_prime_allowed(pos) {
                        self.spawn_primed_tnt(pos, false, owner, None);
                        self.with_level(|b, level, ticks| b.remove_block(level, ticks, pos, false));
                        let _ = self.process_level_events();
                    }
                }
                BlockReaction::Light(lit) => {
                    let _ = self.level_set([pos.x, pos.y, pos.z], lit);
                }
            },
            ArrowEvent::SplashEvent => {
                self.projectile_event("minecraft:splash", None, [at.x, at.y, at.z]);
            }
            // finish_arrow_hit drops the pickup, which it holds.
            ArrowEvent::ShootEvent | ArrowEvent::Drop => {}
        }
    }
    /// `Level.isRainingAt` for arrows.
    /// What a projectile can hit: the mobs, the player, the dragon's parts and
    /// the End crystals (under reserved ids, game/mobs/dragon.rs).
    /// The boats and minecarts a projectile can strike.
    pub(super) fn vehicle_targets(&self) -> Vec<(Who, Aabb)> {
        let mut targets: Vec<(Who, Aabb)> = self
            .boats
            .iter()
            .map(|b| (Who::Mob(b.id), b.bounds()))
            .collect();
        targets.extend(self.carts.iter().map(|c| (Who::Mob(c.id), c.bounds())));
        targets
    }

    pub(super) fn arrow_rain(&self) -> impl Fn(BlockPos) -> bool + '_ {
        move |p: BlockPos| self.raining_at(p)
    }
    /// The level as arrows see it.
    pub(super) fn arrow_scene<'s>(
        &'s self,
        rain: &'s dyn Fn(BlockPos) -> bool,
    ) -> horizon_client::arrows::ArrowScene<'s, 's> {
        horizon_client::arrows::ArrowScene {
            world: &self.world,
            registry: self.pack.assets.registry,
            facts: self.res.facts(),
            collision: self.collision,
            fluids: &self.motion.fluids,
            loop_context: &self.loop_context,
            raining: rain,
            mob_griefing: self
                .ui
                .rules
                .bool_at(horizon_client::gamerules::vanilla::MOB_GRIEFING),
            tnt_explodes: self
                .ui
                .rules
                .bool_at(horizon_client::gamerules::vanilla::TNT_EXPLODES),
            mob_hooks: self
                .grief_hooked()
                .then_some(self as &dyn horizon_client::mobs::MobHooks),
            owners: self,
        }
    }
    /// Dispatch with the projectile's actual owner context, including a guest shooter.
    pub(super) fn projectile_shoot_event(
        &mut self,
        owner: Option<Who>,
        guest: Option<u32>,
        at: [f64; 3],
    ) {
        if let Some(conn) = guest.filter(|conn| self.lan_acting != Some(*conn)) {
            let dispatched = self
                .as_guest(conn, |g| {
                    g.projectile_event("minecraft:projectile_shoot", owner, at);
                })
                .is_some();
            // An owner who left cannot resolve to the host. The event
            // still occurs, with no resolved owner (Projectile.getOwner).
            if !dispatched {
                self.projectile_event("minecraft:projectile_shoot", None, at);
            }
        } else {
            self.projectile_event("minecraft:projectile_shoot", owner, at);
        }
    }
    /// The rest of the tick of an arrow that hit an entity
    /// (`arrows::finish_entity_hit`): its sounds and drop, and back among the
    /// projectiles if it flies on.
    fn finish_arrow_hit(&mut self, mut arrow: Projectile, outcome: EntityHit) {
        let keep = {
            let rain = self.arrow_rain();
            horizon_client::arrows::finish_entity_hit(&mut arrow, &self.arrow_scene(&rain), outcome)
        };
        let Some(a) = arrow.arrow.as_mut() else {
            return;
        };
        let (events, item) = (std::mem::take(&mut a.events), a.pickup_item.clone());
        for event in events {
            if event == horizon_client::arrows::ArrowEvent::Drop {
                self.spawn_at_location(arrow.position, f64::from(0.1f32), item.clone());
            } else if event == horizon_client::arrows::ArrowEvent::ShootEvent {
                self.projectile_shoot_event(arrow.owner, arrow.guest, arrow.position.components());
            } else {
                self.arrow_event(arrow.position, event);
            }
        }
        if keep {
            self.add_projectile(arrow);
        }
    }
    /// `ThrownEnderpearl.onHit` for a living non-player owner: the same hook,
    /// then the normal entity teleport, without a player's endermite roll or damage.
    fn mob_pearl_teleport(&mut self, pearl: &Projectile, id: u32, hit: Hit) {
        let Some(owner) = self.entity(id) else { return };
        let Some(from) = self
            .mobs
            .iter()
            .find(|m| {
                m.id == id
                    && m.alive()
                    && !m.removed
                    && !horizon_client::mobs::villager::sleeping(m)
            })
            .map(|m| m.position)
        else {
            return;
        };
        let dimension = self.dim;
        let _: std::result::Result<(), ()> = mod_api::run_transaction(self, |game, _| {
            let (mut target, _, cancel) =
                game.native_pearl_teleport(owner, pearl.id, from, pearl.previous, 0.0, hit);
            // Listener writes stay in the transaction; never overwrite a changed,
            // dead, sleeping or removed owner using the old decision.
            if cancel
                || game.dim != dimension
                || game.entity(id) != Some(owner)
                || !game.mobs.iter().any(|m| {
                    m.id == id
                        && m.alive()
                        && !m.removed
                        && m.position == from
                        && !horizon_client::mobs::villager::sleeping(m)
                })
            {
                return Ok(());
            }
            if !target.iter().all(|v| v.is_finite()) {
                target = [pearl.previous.x, pearl.previous.y, pearl.previous.z];
            }
            if !super::native_entity_moves::in_bounds(target) {
                return Ok(());
            }
            let _ = game.place_teleport_filed(
                super::commands::Target::Mob(id),
                target,
                None,
                [false; 3],
                true,
            );
            if let Some(m) = game.mobs.iter_mut().find(|m| m.id == id) {
                m.fall_distance = 0.0;
                // Entity.teleport's setOldRot: keep the owner's angles without
                // interpolating from the pre-teleport rotation on the client.
                m.previous_yaw = m.yaw;
                m.previous_head_pitch = m.head_pitch;
            }
            game.audio
                .play("minecraft:entity.player.teleport", Some(target), 1.0, 1.0);
            Ok(())
        });
    }

    /// A projectile's hit, landed in the attack of its owner.
    pub(super) fn projectile_hit(&mut self, p: &Projectile, hit: Hit) -> Result<()> {
        let attack = self.projectile_attack(p);
        let result = self.with_attack(attack, |g| g.projectile_hit_landed(p, hit));
        self.settle_dealt_absorbed(p.owner == Some(Who::Player));
        result?;
        // The hit's hurt teleports an enderman or a shulker now (`hurtServer`).
        self.settle_hurt_teleports()
    }
    fn projectile_hit_landed(&mut self, p: &Projectile, hit: Hit) -> Result<()> {
        // Projectile.onHitBlock: BlockState.onProjectileHit, for every
        // projectile (a wind charge out of the world's top has no face).
        if let Hit::Block { pos, at, face } = hit
            && face != [0; 3]
        {
            self.projectile_block_impact(&ProjectileHitter::of(p), pos, at, face)?;
            self.projectile_breaks_block(p, pos);
        }
        self.combat.projectile_owner = match p.owner {
            Some(Who::Mob(id)) => Some(id),
            _ => None,
        };
        self.combat.projectile_direct = match hit {
            Hit::Entity {
                who: Who::Player, ..
            } => Some(self.target_name(super::commands::Target::Projectile(p.id))),
            _ => None,
        };
        let at = match hit {
            Hit::Entity { at, .. } | Hit::Block { at, .. } => at,
            Hit::None => return Ok(()),
        };
        // Projectile.onHitBlock: BlockState.onProjectileHit, for every
        // projectile (a wind charge out of the world's top has no face).
        if let Hit::Block { pos, face, .. } = hit
            && face != [0; 3]
        {
            self.target_block_hit(&ProjectileHitter::of(p), pos, at, face);
        }
        if let Hit::Entity {
            who: Who::Mob(id),
            at,
        } = hit
            && self.lan_projectile_hit(p, id, at)
        {
            return Ok(());
        }
        if matches!(
            p.kind,
            ProjectileKind::WindCharge | ProjectileKind::BreezeWindCharge
        ) {
            return self.wind_charge_hit(p, hit);
        }
        // A wither skull's hit on a part or crystal goes through
        // `wither_skull_hit` below, as on any entity, and then explodes.
        if let Hit::Entity {
            who: Who::Mob(id), ..
        } = hit
            && id >= super::dragon::SPECIAL
            && !matches!(
                p.kind,
                ProjectileKind::WitherSkull { .. } | ProjectileKind::DragonFireball
            )
        {
            let by_player = p.owner == Some(Who::Player);
            if p.arrow.is_some() {
                // AbstractArrow.onHitEntity: spent if the part or crystal took
                // it, else turned back (a perched dragon sets it alight first).
                let mut arrow = p.clone();
                let amount = self.shot_damage(
                    &mut arrow,
                    Some(if id >= super::dragon::CRYSTAL_IDS {
                        "minecraft:end_crystal"
                    } else {
                        "minecraft:ender_dragon"
                    }),
                    Who::Mob(id),
                );
                let outcome = match self.hit_special(
                    id,
                    amount,
                    by_player,
                    self.projectile_damage_kind(p),
                    Some(p),
                ) {
                    super::dragon::Hurt::Landed { .. } => EntityHit::Hurt,
                    super::dragon::Hurt::Refused => EntityHit::Refused,
                    super::dragon::Hurt::Shrugged => {
                        if let Some(a) = arrow.arrow.as_mut() {
                            a.ignite_for_seconds(1.0);
                        }
                        EntityHit::Refused
                    }
                };
                self.finish_arrow_hit(arrow, outcome);
                return Ok(());
            }
            let amount = match p.kind {
                ProjectileKind::SmallFireball => 5.0,
                ProjectileKind::LargeFireball { .. } => 6.0,
                ProjectileKind::Thrown(t) => t.damage,
                _ => 0.0,
            };
            let kind = match p.kind {
                ProjectileKind::SmallFireball | ProjectileKind::LargeFireball { .. } => {
                    horizon_client::mobs::DamageKind::Fireball
                }
                _ => horizon_client::mobs::DamageKind::Thrown,
            };
            self.hit_special(id, amount, by_player, kind, Some(p));
            // ThrowableProjectile.onHit goes on after onHitEntity: a pearl
            // still takes its owner to where it broke (into the crystal's
            // blast), an egg still hatches, a snowball still breaks. A large
            // fireball still explodes (LargeFireball.onHit), whatever the
            // part or crystal made of its hit (both refuse fire), and the
            // blast takes the crystal.
            if !matches!(
                p.kind,
                ProjectileKind::EnderPearl
                    | ProjectileKind::Egg
                    | ProjectileKind::Snowball
                    | ProjectileKind::Thrown(_)
                    | ProjectileKind::LargeFireball { .. }
            ) {
                return Ok(());
            }
        }
        if p.arrow.is_some()
            && let Hit::Entity {
                who: Who::Mob(id), ..
            } = hit
            && self.item_frames.iter().any(|f| f.id == id)
        {
            // AbstractArrow.onHitEntity on an item frame: spent when the
            // frame took the hit, turned back when a fixed one refused it.
            let by_player = p.owner == Some(Who::Player);
            let hurt = self.projectile_hurts_frame(id, by_player, by_player && self.creative);
            let outcome = if hurt {
                EntityHit::Hurt
            } else {
                EntityHit::Refused
            };
            self.finish_arrow_hit(p.clone(), outcome);
            return Ok(());
        }
        if p.arrow.is_some()
            && let Hit::Entity {
                who: Who::Mob(id), ..
            } = hit
            && let Some(i) = self.carts.iter().position(|c| c.id == id)
        {
            // AbstractArrow.onHitEntity on a minecart.
            let mut arrow = p.clone();
            let victim_type = self.carts[i].kind.id().to_owned();
            let amount = self.shot_damage(&mut arrow, Some(&victim_type), Who::Mob(id));
            let burning = p.arrow.as_ref().is_some_and(|a| a.on_fire());
            let creative = p.owner == Some(Who::Player) && self.creative;
            let kind = self.carts[i].kind.id();
            let damage_type = horizon_client::health::damage_type(
                self.projectile_damage_kind(p),
                p.owner.is_some(),
            );
            let refused = self.vehicle_refuses_projectile(p, id, kind, damage_type);
            if !refused {
                self.arrow_hits_cart(i, amount, p.velocity, burning, creative);
            }
            self.finish_arrow_hit(
                arrow,
                if refused {
                    EntityHit::Refused
                } else {
                    EntityHit::Hurt
                },
            );
            return Ok(());
        }
        if p.arrow.is_some()
            && let Hit::Entity {
                who: Who::Mob(id), ..
            } = hit
            && let Some(i) = self.boats.iter().position(|b| b.id == id)
        {
            // AbstractArrow.onHitEntity on a boat: VehicleEntity.hurtServer
            // takes the arrow's damage, and the arrow is spent.
            let mut arrow = p.clone();
            let victim_type = self.boats[i].entity_type();
            let amount = self.shot_damage(&mut arrow, Some(&victim_type), Who::Mob(id));
            let creative = p.owner == Some(Who::Player) && self.creative;
            let kind = self.boats[i].entity_type();
            let damage_type = horizon_client::health::damage_type(
                self.projectile_damage_kind(p),
                p.owner.is_some(),
            );
            let refused = self.vehicle_refuses_projectile(p, id, &kind, damage_type);
            if !refused {
                self.hurt_boat(i, amount, creative);
            }
            self.finish_arrow_hit(
                arrow,
                if refused {
                    EntityHit::Refused
                } else {
                    EntityHit::Hurt
                },
            );
            return Ok(());
        }
        let atp = [at.x, at.y, at.z];
        // Projectile.onHit: the landing is heard.
        self.projectile_landed_event(p.id, p.owner, atp);
        let source_kind = self.projectile_damage_kind(p);
        let attacker_species = match p.owner {
            Some(Who::Mob(id)) => self.mobs.iter().find(|m| m.id == id).map(|m| m.species()),
            _ => None,
        };
        let source = Damage {
            kind: source_kind,
            attacker: p.owner,
            from: Some([p.previous.x, p.previous.y, p.previous.z]),
            attacker_species,
        };
        let aim = self.attacker_aim(source.attacker);
        match (&p.kind, hit) {
            // ThrownTrident.onHitEntity: 8 damage (Impaling on aquatic mobs),
            // dealt whether or not the hurt lands, then it bounces off.
            (ProjectileKind::Trident, Hit::Entity { who, .. }) => {
                let mut trident = p.clone();
                let victim = match who {
                    Who::Mob(id) => self
                        .mobs
                        .iter()
                        .find(|m| m.id == id)
                        .map(|m| m.kind.id.clone()),
                    Who::Player => Some("minecraft:player".to_owned()),
                };
                let amount = self.shot_damage(&mut trident, victim.as_deref(), who);
                let hurt = match who {
                    Who::Player => self.mob_hits_player(
                        amount,
                        [p.previous.x, p.previous.y, p.previous.z],
                        "minecraft:trident",
                    ),
                    Who::Mob(id) => self.damage_projectile_with_weapon(p, id, amount, source, aim),
                };
                let outcome = if hurt {
                    self.shot_post_attack(p, who, "minecraft:trident", "minecraft:trident");
                    EntityHit::Hurt
                } else {
                    EntityHit::Refused
                };
                self.burning_arrow_primes(p, who);
                self.finish_arrow_hit(trident, outcome);
            }
            // AbstractArrow.onHitBlock plays its sound itself (ArrowEvent).
            (ProjectileKind::Trident, Hit::Block { .. }) => {}
            // AbstractArrow.onHitEntity on a ghast's fireball or a wind charge:
            // `hurtOrSimulate` fails (`AbstractHurtingProjectile.hurtServer` is
            // false; the projectile turned in `Projectile.onHit`, which already
            // ran), so the arrow takes `deflect(REVERSE, 0.2)` instead of vanishing.
            (
                ProjectileKind::Arrow { .. },
                Hit::Entity {
                    who: Who::Mob(id), ..
                },
            ) if pickable_projectile_hit(id).is_some() => {
                let mut arrow = p.clone();
                arrow.arrow_damage();
                self.finish_arrow_hit(arrow, EntityHit::Refused);
            }
            (ProjectileKind::Arrow { .. }, Hit::Entity { who, .. }) => {
                let mut arrow = p.clone();
                let victim = match who {
                    Who::Player => Some("minecraft:player".to_owned()),
                    Who::Mob(id) => self
                        .mobs
                        .iter()
                        .find(|m| m.id == id)
                        .map(|m| m.kind.id.clone()),
                };
                let amount = self.shot_damage(&mut arrow, victim.as_deref(), who);
                let player_fire = self.player.as_ref().map(|pl| pl.fire_ticks);
                // AbstractArrow.onHitEntity saves the fire, ignites, then puts
                // it back when hurtOrSimulate fails and the target still takes
                // a projectile's side effects.
                let mob_fire = match who {
                    Who::Mob(id) => self.mobs.iter().find(|m| m.id == id).map(|m| m.fire_ticks),
                    Who::Player => None,
                };
                let mut dodged = false;
                // AbstractArrow.onHitEntity: a burning arrow sets its target alight.
                if p.arrow.as_ref().is_some_and(|a| a.on_fire()) {
                    self.handle_mob_events(vec![MobEvent::Ignite(who, 5)])?;
                }
                let hurt = match who {
                    Who::Player => {
                        let hurt = self.arrow_hits_player(amount, p);
                        // AbstractArrow.onHitEntity: a hit that hurt leaves a
                        // non-piercing arrow stuck in the body.
                        if hurt && p.arrow.as_ref().is_some_and(|a| (a.pierce as i8) <= 0) {
                            self.combat.stuck.add_arrow();
                        }
                        hurt
                    }
                    Who::Mob(id) => {
                        let hurt = self.damage_projectile_with_weapon(p, id, amount, source, aim);
                        dodged = !hurt
                            && self.mobs.iter().find(|m| m.id == id).is_some_and(|m| {
                                m.species() == Species::Enderman
                                    && horizon_client::mobs::enderman::dodges(m, &source)
                            });
                        if p.owner == Some(Who::Player) {
                            self.remember_mob_player_credit(id);
                            if hurt && let Some(m) = self.mobs.iter().find(|m| m.id == id) {
                                // LivingEntity.hurtServer: PLAYER_HURT_ENTITY.
                                let victim = self.mob_entity(m);
                                let spectral = p.arrow.as_ref().is_some_and(|a| {
                                    &*a.pickup_item.item == "minecraft:spectral_arrow"
                                });
                                let direct = horizon_client::advancements::EntityView {
                                    kind: if spectral {
                                        "minecraft:spectral_arrow"
                                    } else {
                                        "minecraft:arrow"
                                    }
                                    .into(),
                                    position: [p.position.x, p.position.y, p.position.z],
                                    ..Default::default()
                                };
                                self.player_hit(&victim, "minecraft:arrow", Some(direct), amount);
                            }
                            // AbstractArrow.onHitEntity: KILLED_BY_ARROW for a kill
                            // (piercing arrows are not modelled).
                            if hurt
                                && let Some(m) = self.mobs.iter().find(|m| m.id == id)
                                && !m.alive()
                            {
                                let victim = self.mob_entity(m);
                                let weapon =
                                    p.arrow.as_ref().and_then(|a| a.fired_from_weapon.as_ref());
                                self.trigger_killed_by_arrow(&[victim], weapon);
                            }
                        }
                        hurt
                    }
                };
                self.burning_arrow_primes(p, who);
                // doKnockback: Punch pushes along the flight, less the target's
                // knockback resistance.
                let flight = (p.velocity.x * p.velocity.x + p.velocity.z * p.velocity.z).sqrt();
                let resistance = match who {
                    Who::Player => self
                        .player
                        .as_mut()
                        .and_then(|pl| pl.attribute("minecraft:knockback_resistance").ok())
                        .unwrap_or(0.0),
                    Who::Mob(id) => self
                        .mobs
                        .iter()
                        .find(|m| m.id == id)
                        .map_or(0.0, |m| m.knockback_resistance()),
                };
                let knockback = if !hurt {
                    0.0
                } else if let Some(weapon) =
                    p.arrow.as_ref().and_then(|a| a.fired_from_weapon.as_ref())
                {
                    let victim = match who {
                        Who::Player => Some("minecraft:player"),
                        Who::Mob(id) => self
                            .mobs
                            .iter()
                            .find(|m| m.id == id)
                            .map(|m| m.kind.id.as_str()),
                    };
                    let owner = match p.owner {
                        Some(Who::Player) => Some("minecraft:player"),
                        Some(Who::Mob(id)) => self
                            .mobs
                            .iter()
                            .find(|m| m.id == id)
                            .map(|m| m.kind.id.as_str()),
                        None => None,
                    };
                    let arrow_type = if p
                        .arrow
                        .as_ref()
                        .is_some_and(|a| &*a.pickup_item.item == "minecraft:spectral_arrow")
                    {
                        "minecraft:spectral_arrow"
                    } else {
                        "minecraft:arrow"
                    };
                    let ctx = horizon_client::enchant::EffectContext {
                        this: victim,
                        attacker: owner,
                        direct_attacker: Some(arrow_type),
                        damage_type: Some("minecraft:arrow"),
                        items: Some(self.res.items()),
                        ..Default::default()
                    };
                    self.res.stations().enchantments.modify_value(
                        Some(weapon),
                        "minecraft:knockback",
                        &ctx,
                        &mut self.world.random,
                        0.0,
                    )
                } else {
                    p.bonus.punch as f32
                };
                let push = f64::from(knockback) * 0.6 * (1.0 - resistance).max(0.0);
                if hurt && push > 0.0 && flight > 1e-6 {
                    let k = push / flight;
                    self.handle_mob_events(vec![MobEvent::Push {
                        target: who,
                        velocity: [p.velocity.x * k, 0.1, p.velocity.z * k],
                    }])?;
                }
                if hurt {
                    let spectral = p
                        .arrow
                        .as_ref()
                        .is_some_and(|a| &*a.pickup_item.item == "minecraft:spectral_arrow");
                    let direct = if spectral {
                        "minecraft:spectral_arrow"
                    } else {
                        "minecraft:arrow"
                    };
                    self.shot_post_attack(p, who, direct, "minecraft:arrow");
                    self.arrow_post_hurt_effects(p, who);
                }
                match (hurt, who) {
                    (true, _) => self.finish_arrow_hit(arrow, EntityHit::Hurt),
                    // What the player took no damage from (a raised shield)
                    // keeps its fire as it was and turns the arrow back.
                    (false, Who::Player) => {
                        if let (Some(pl), Some(ticks)) = (self.player.as_mut(), player_fire) {
                            pl.set_remaining_fire_ticks(ticks);
                        }
                        self.finish_arrow_hit(arrow, EntityHit::Refused);
                    }
                    // EnderMan.hurtServer teleported away: the arrow flies on.
                    (false, Who::Mob(_)) if dodged => {
                        self.finish_arrow_hit(arrow, arrow_miss(true));
                    }
                    // hurtOrSimulate returned false and
                    // projectileReceivesSideEffectsOnHit(false) is the default
                    // (Shulker.hurtServer on a closed shell, invulnerability).
                    // The fire the arrow just set goes back, then
                    // deflect(REVERSE, entity, owner, false, 0.2). The hit
                    // sound plays only when the hurt lands.
                    (false, Who::Mob(id)) => {
                        if let (Some(m), Some(ticks)) =
                            (self.mobs.iter_mut().find(|m| m.id == id), mob_fire)
                        {
                            m.fire_ticks = ticks;
                        }
                        self.finish_arrow_hit(arrow, arrow_miss(false));
                    }
                }
            }
            // AbstractArrow.onHitBlock plays its sound itself (ArrowEvent).
            (ProjectileKind::Arrow { .. }, Hit::Block { .. }) => {}
            (ProjectileKind::Snowball, hit) => {
                if let Hit::Entity {
                    who: Who::Player, ..
                } = hit
                {
                    self.mob_hits_player(
                        0.0,
                        [p.previous.x, p.previous.y, p.previous.z],
                        "minecraft:thrown",
                    );
                } else if let Hit::Entity {
                    who: Who::Mob(id), ..
                } = hit
                    && let Some(m) = self.mobs.iter_mut().find(|m| m.id == id)
                {
                    // Snowball.onHitEntity: 3 damage to blazes, 0 otherwise.
                    let amount = if m.species() == Species::Blaze {
                        3.0
                    } else {
                        0.0
                    };
                    if m.damage_projectile(
                        crate::game::entity::damage_level_of!(self, scoped),
                        amount,
                        source,
                        aim,
                        [p.velocity.x, p.velocity.z],
                    ) {
                        self.thrown_hurt_mob(p, id, "minecraft:snowball", amount);
                    }
                } else if let Hit::Entity {
                    who: Who::Mob(id), ..
                } = hit
                {
                    // Snowball.onHitEntity: 0 damage, which still shakes a
                    // boat or minecart (VehicleEntity.hurtServer).
                    self.hurt_vehicle(p, id, 0.0);
                }
                self.thrown_break_event(p.id, atp);
                self.item_burst(atp, "minecraft:snowball");
            }
            // a mod's copy of Snowball: its damage to anything it hits.
            (ProjectileKind::Thrown(t), hit) => {
                match hit {
                    Hit::Entity {
                        who: Who::Mob(id), ..
                    } => {
                        let push = [p.velocity.x, p.velocity.z];
                        let hurt = self.mobs.iter_mut().find(|m| m.id == id).is_some_and(|m| {
                            m.damage_projectile(
                                crate::game::entity::damage_level_of!(self, scoped),
                                t.damage,
                                source,
                                aim,
                                push,
                            )
                        });
                        if hurt {
                            self.player_hurt_mob(p, id);
                        }
                        // LivingEntity.hurtServer: PLAYER_HURT_ENTITY.
                        if hurt
                            && p.owner == Some(Who::Player)
                            && let Some(m) = self.mobs.iter().find(|m| m.id == id)
                        {
                            let victim = self.mob_entity(m);
                            let direct = horizon_client::advancements::EntityView {
                                kind: t.id.into(),
                                position: [p.position.x, p.position.y, p.position.z],
                                ..Default::default()
                            };
                            self.player_hit(&victim, "minecraft:thrown", Some(direct), t.damage);
                        }
                    }
                    // Player.hurtServer observes invulnerability before refusing 0 damage.
                    Hit::Entity {
                        who: Who::Player, ..
                    } => {
                        let from = [p.previous.x, p.previous.y, p.previous.z];
                        self.mob_hits_player(t.damage, from, "minecraft:thrown");
                    }
                    _ => {}
                }
                self.thrown_break_event(p.id, atp);
                self.item_burst(atp, t.item);
            }
            (ProjectileKind::Egg, hit) => {
                if let Hit::Entity {
                    who: Who::Player, ..
                } = hit
                {
                    self.mob_hits_player(
                        0.0,
                        [p.previous.x, p.previous.y, p.previous.z],
                        "minecraft:thrown",
                    );
                } else if let Hit::Entity {
                    who: Who::Mob(id), ..
                } = hit
                    && let Some(m) = self.mobs.iter_mut().find(|m| m.id == id)
                {
                    if m.damage_projectile(
                        crate::game::entity::damage_level_of!(self, scoped),
                        0.0,
                        source,
                        aim,
                        [p.velocity.x, p.velocity.z],
                    ) {
                        self.thrown_hurt_mob(p, id, "minecraft:egg", 0.0);
                    }
                } else if let Hit::Entity {
                    who: Who::Mob(id), ..
                } = hit
                {
                    // ThrownEgg.onHitEntity: the same 0 damage.
                    self.hurt_vehicle(p, id, 0.0);
                }
                // ThrownEgg.onHit: 1/8 chance of a chick, 1/32 of four.
                if self.world.random.next_int_bound(8).unwrap_or(1) == 0 {
                    let n = if self.world.random.next_int_bound(32).unwrap_or(1) == 0 {
                        4
                    } else {
                        1
                    };
                    for _ in 0..n {
                        if let Some(id) = self.spawn_mob_raw("minecraft:chicken", atp, true)
                            && let Some(c) = self.mobs.iter_mut().find(|m| m.id == id)
                        {
                            (c.extra.age, c.extra.variant) = (-24000, p.egg_variant.unwrap_or(0));
                        }
                    }
                }
                self.thrown_break_event(p.id, atp);
                self.item_burst(atp, "minecraft:egg");
            }
            (ProjectileKind::EnderPearl, hit) => {
                if let Hit::Entity {
                    who: Who::Player, ..
                } = hit
                {
                    self.mob_hits_player(
                        0.0,
                        [p.previous.x, p.previous.y, p.previous.z],
                        "minecraft:thrown",
                    );
                } else if let Hit::Entity {
                    who: Who::Mob(id), ..
                } = hit
                    && let Some(m) = self.mobs.iter_mut().find(|m| m.id == id)
                {
                    if m.damage_projectile(
                        crate::game::entity::damage_level_of!(self, scoped),
                        0.0,
                        source,
                        aim,
                        [p.velocity.x, p.velocity.z],
                    ) {
                        self.thrown_hurt_mob(p, id, "minecraft:ender_pearl", 0.0);
                    }
                } else if let Hit::Entity {
                    who: Who::Mob(id), ..
                } = hit
                {
                    // ThrownEnderpearl.onHitEntity: the same 0 damage.
                    self.hurt_vehicle(p, id, 0.0);
                }
                // To where the pearl started its last tick (oldPosition).
                let mut to = p.previous;
                let mut damage = 5.0;
                let mut cancel = false;
                // EntityTeleportEvent.EnderPearl: a server player's pearl.
                if p.owner == Some(Who::Player)
                    && let Some(from) = self.player.as_ref().map(|pl| pl.position)
                    && let Some(owner) = self.native_acting_player().map(|player| player.entity())
                {
                    let dimension = self.dim;
                    let target;
                    (target, damage, cancel) =
                        self.native_pearl_teleport(owner, p.id, from, to, damage, hit);
                    cancel |= self.dim != dimension
                        || self.player.as_ref().is_none_or(|pl| pl.position != from)
                        || self.native_acting_player().map(|player| player.entity()) != Some(owner);
                    // A target that is not a number keeps the pearl's.
                    if target.iter().all(|v| v.is_finite()) {
                        to = Vec3::new(target[0], target[1], target[2]);
                    }
                }
                // ThrownEnderpearl.isAllowedToTeleportOwner: an owner who is
                // dead or asleep cannot take the pearl, so it is discarded
                // where it lands (no teleport and no 5 damage).
                let taken = self.player.as_ref().is_some_and(|pl| {
                    horizon_client::projectiles::owner_can_take_pearl(
                        !pl.health.dead_or_dying(),
                        self.sleeping.is_some(),
                    )
                });
                if p.owner == Some(Who::Player) && taken && !cancel {
                    // ThrownEnderpearl: 5% endermite where the player was
                    // (when monsters spawn), teleport, 5 damage.
                    if self.world.random.next_float() < 0.05
                        && self
                            .ui
                            .rules
                            .bool_at(horizon_client::gamerules::vanilla::SPAWN_MONSTERS)
                        && self.ui.difficulty != 0
                        && let Some(pl) = self.player.as_ref()
                    {
                        let from = pl.position;
                        self.spawn_mob_raw("minecraft:endermite", [from.x, from.y, from.z], false);
                    }
                    // ServerPlayer.teleport: removeVehicle before the placement.
                    section_steps::teleport_player(self, to);
                    self.hurt_player_by("minecraft:ender_pearl", damage);
                    self.audio.play(
                        "minecraft:entity.player.teleport",
                        Some([to.x, to.y, to.z]),
                        1.0,
                        1.0,
                    );
                }
                if let Some(Who::Mob(id)) = p.owner {
                    self.mob_pearl_teleport(p, id, hit);
                }
            }
            (ProjectileKind::SmallFireball, hit) => match hit {
                Hit::Entity { who, .. } => match who {
                    Who::Player => {
                        // SmallFireball.onHitEntity: remember the fire, ignite,
                        // and restore it when the hurt is refused (a raised shield).
                        let remaining = self.player.as_ref().map(|p| p.fire_ticks);
                        if let Some(p) = self.player.as_mut() {
                            p.ignite_for_ticks(100)?;
                        }
                        let hurt = self.mob_hits_player(
                            5.0,
                            [p.previous.x, p.previous.y, p.previous.z],
                            "minecraft:fireball",
                        );
                        if !hurt
                            && let Some(remaining) = remaining
                            && let Some(pl) = self.player.as_mut()
                        {
                            pl.set_remaining_fire_ticks(remaining);
                        }
                    }
                    Who::Mob(id) => {
                        let hurt = self.mobs.iter_mut().find(|m| m.id == id).is_some_and(|m| {
                            let remaining = m.fire_ticks;
                            if !m.kind.flags.fire_immune {
                                m.ignite_for_ticks(100);
                            }
                            let hurt = m.damage_projectile(
                                crate::game::entity::damage_level_of!(self, scoped),
                                5.0,
                                source,
                                aim,
                                [p.velocity.x, p.velocity.z],
                            );
                            if !hurt {
                                m.fire_ticks = remaining;
                            }
                            hurt
                        });
                        if hurt {
                            self.player_hurt_mob(p, id);
                            self.shot_post_attack(
                                p,
                                who,
                                "minecraft:small_fireball",
                                "minecraft:fireball",
                            );
                        } else if !self.mobs.iter().any(|m| m.id == id) {
                            // SmallFireball.onHitEntity: 5 damage to a boat or minecart.
                            self.hurt_vehicle(p, id, 5.0);
                        }
                    }
                },
                Hit::Block { pos, face, .. } => {
                    // SmallFireball.onHitBlock: a mob's only with mobGriefing
                    // (NeoForge: `canEntityGrief` for the owner);
                    // BaseFireBlock.getState placed with updates (the fire
                    // ticks, ages and goes out where it can't stay).
                    let fire = [pos[0] + face[0], pos[1] + face[1], pos[2] + face[2]];
                    let may = match p.owner {
                        Some(Who::Mob(owner)) => self.can_entity_grief(Some(owner)),
                        _ => true,
                    };
                    if may
                        && self
                            .res
                            .facts()
                            .is_air(self.world.block(fire[0], fire[1], fire[2]))
                    {
                        let at = BlockPos::new(fire[0], fire[1], fire[2]);
                        let state =
                            self.with_level(|b, level, ticks| b.fire_state_for(level, ticks, at));
                        self.level_set(fire, state)?;
                    }
                }
                Hit::None => {}
            },
            (ProjectileKind::LargeFireball { power }, hit) => {
                // Fireball explosions are caused by the shooter. A direct hit
                // on the player takes `projectile_owner`, so read the owner
                // from the fireball.
                let owner = match p.owner {
                    Some(Who::Mob(id)) => Some(id),
                    _ => None,
                };
                if let Hit::Entity { who, .. } = hit {
                    match who {
                        Who::Player => {
                            self.mob_hits_player(
                                6.0,
                                [p.previous.x, p.previous.y, p.previous.z],
                                "minecraft:fireball",
                            );
                        }
                        Who::Mob(id) => {
                            if self.mobs.iter_mut().find(|m| m.id == id).is_some_and(|m| {
                                m.damage_projectile(
                                    crate::game::entity::damage_level_of!(self, scoped),
                                    6.0,
                                    source,
                                    aim,
                                    [p.velocity.x, p.velocity.z],
                                )
                            }) {
                                self.player_hurt_mob(p, id);
                            }
                            self.shot_post_attack(
                                p,
                                who,
                                "minecraft:fireball",
                                "minecraft:fireball",
                            );
                        }
                    }
                }
                self.combat.explosion_owner = owner;
                self.combat.explosion_in_water = p.touching_water;
                // The blast's source names the owner, so a dragon it kills
                // remembers the player who shot it.
                self.set_blast_by_player(p.owner == Some(Who::Player));
                // LargeFireball.onHit: the fire follows
                // `EventHooks.canEntityGrief(level, getOwner())`, so a mod's
                // answer for whoever owns the fireball decides: the ghast that
                // shot it, or the player who sent it back.
                let fire = self.large_fireball_fire(p.owner, p.guest);
                self.combat.explosion_direct = Some(p.id);
                let result = self.explode_mob(
                    atp,
                    *power as f32,
                    fire,
                    super::explosion::ExplosionSource::default()
                        .with_silence(self.entity_silent(p.id))
                        .with_entity(p.id, "minecraft:fireball"),
                );
                self.combat.explosion_direct = None;
                self.set_blast_by_player(false);
                self.combat.explosion_owner = None;
                self.combat.explosion_in_water = false;
                result?;
            }
            // AbstractThrownPotion.onHit, then onHitAsPotion from the stack it
            // flies as: the splash happens where the hit was, a lingering
            // potion's cloud sits at the mob it hit (ThrownLingeringPotion),
            // and the potion that lands names the hurt it deals.
            (kind @ (ProjectileKind::Potion(..) | ProjectileKind::ExperienceBottle), hit) => {
                // A bottle flies as the bottle item (`getDefaultItem`).
                let bottle;
                let stack = match kind {
                    ProjectileKind::Potion(stack, _) => stack,
                    _ => {
                        bottle = Box::new(ItemStack::new("minecraft:experience_bottle", 1));
                        &bottle
                    }
                };
                let block = match hit {
                    Hit::Block { pos, face, .. } => Some((
                        pos,
                        Direction::approximate_nearest(Vec3::new(
                            f64::from(face[0]),
                            f64::from(face[1]),
                            f64::from(face[2]),
                        )),
                    )),
                    _ => None,
                };
                let mob = match hit {
                    Hit::Entity {
                        who: Who::Player, ..
                    } => self.player.as_ref().map(|pv| pv.position),
                    Hit::Entity {
                        who: Who::Mob(id), ..
                    } => self.mobs.iter().find(|m| m.id == id).map(|m| m.position),
                    _ => None,
                };
                let owner = self.potion_owner(p.owner);
                let item = stack.item.to_string();
                self.splash(
                    stack,
                    at,
                    mob,
                    block,
                    p.age,
                    &owner,
                    self.entity_silent(p.id),
                    p.velocity,
                );
                if stack.item.as_ref() != "minecraft:experience_bottle" {
                    self.native_potion_splash(&item, [at.x, at.y, at.z]);
                }
            }
            // LlamaSpit.onHitEntity hurts only when the owner is a LivingEntity.
            (ProjectileKind::LlamaSpit, Hit::Entity { .. }) if p.owner.is_none() => {}
            (ProjectileKind::LlamaSpit, Hit::Entity { who, .. }) => match who {
                Who::Player => {
                    self.mob_hits_player(
                        1.0,
                        [p.previous.x, p.previous.y, p.previous.z],
                        "minecraft:spit",
                    );
                }
                Who::Mob(id) => {
                    if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id)
                        && m.damage_projectile(
                            crate::game::entity::damage_level_of!(self, scoped),
                            1.0,
                            source,
                            aim,
                            [p.velocity.x, p.velocity.z],
                        )
                    {
                        self.shot_post_attack(p, who, "minecraft:llama_spit", "minecraft:spit");
                    }
                }
            },
            // ShulkerBullet.onHitEntity: 4 damage, then levitation for 10 s.
            (ProjectileKind::ShulkerBullet { .. }, Hit::Entity { who, .. }) => match who {
                Who::Player => {
                    // Only a hit that hurt (`wasHurt`) brings the levitation.
                    if self.mob_hits_player(
                        4.0,
                        [p.previous.x, p.previous.y, p.previous.z],
                        "minecraft:mob_projectile",
                    ) {
                        self.give_player_effect("minecraft:levitation", 200, 0);
                    }
                }
                Who::Mob(id) => {
                    let hurt = if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
                        let hurt = m.damage_projectile(
                            crate::game::entity::damage_level_of!(self, scoped),
                            4.0,
                            source,
                            aim,
                            [p.velocity.x, p.velocity.z],
                        );
                        if hurt {
                            // Shulker.hurtServer: a shulker hit by a bullet.
                            horizon_client::mobs::shulker::hit_by_bullet(m);
                        }
                        hurt
                    } else {
                        false
                    };
                    if hurt {
                        // The hurt's teleport (`teleportSomewhere`,
                        // `hitByShulkerBullet`) comes before the levitation.
                        self.settle_hurt_teleports()?;
                        self.shot_post_attack(
                            p,
                            who,
                            "minecraft:shulker_bullet",
                            "minecraft:mob_projectile",
                        );
                        if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
                            m.add_effect("minecraft:levitation", 200, 0);
                        }
                    }
                }
            },
            // DragonFireball.onHit (its own owner's hit never gets here): the
            // dragon's breath where it burst.
            (ProjectileKind::DragonFireball, _) => self.dragon_fireball_on_hit(at),
            (ProjectileKind::WitherSkull { .. }, hit) => {
                let who = match hit {
                    Hit::Entity { who, .. } => Some(who),
                    _ => None,
                };
                self.wither_skull_hit(p, who, atp)?;
            }
            (ProjectileKind::ShulkerBullet { .. }, Hit::Block { .. }) => {
                self.audio
                    .play("minecraft:entity.shulker_bullet.hit", Some(atp), 1.0, 1.0);
                if let Some(ty) = horizon_client::particles::ParticleType::from_name("explosion") {
                    let options = horizon_client::particles::ParticleOptions::simple(ty);
                    for _ in 0..2 {
                        let r = &mut self.world.random;
                        let at = [
                            atp[0] + r.next_gaussian() * 0.2,
                            atp[1] + r.next_gaussian() * 0.2,
                            atp[2] + r.next_gaussian() * 0.2,
                        ];
                        self.particles.add(&options, at, [0.0; 3]);
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
    /// `Snowball`/`ThrownEgg.onHit`: entity event 3 for the guests, before the
    /// projectile is discarded (the clients lose it when the tracker next runs).
    fn thrown_break_event(&mut self, id: u32, at: [f64; 3]) {
        if id != 0 {
            self.lan_entity_event(id as i32, 3, at);
        }
    }
    /// `Snowball`/`ThrownEgg.handleEntityEvent(3)`: eight crumbs of the
    /// thrown item where it broke, at rest for a snowball, scattered by the
    /// client entity's random (the particle engine's here) for an egg.
    pub(super) fn item_burst(&mut self, at: [f64; 3], item: &str) {
        let options = self.item_particle(&horizon_client::items::ItemStack::new(item, 1));
        let egg = item == "minecraft:egg";
        for _ in 0..8 {
            let v = if egg {
                let r = self.particles.random();
                [
                    (f64::from(r.next_float()) - 0.5) * 0.08,
                    (f64::from(r.next_float()) - 0.5) * 0.08,
                    (f64::from(r.next_float()) - 0.5) * 0.08,
                ]
            } else {
                [0.0; 3]
            };
            self.particles.add(&options, at, v);
        }
    }
    /// `AgeableMob.mobInteract`: whether a click with `item` on `m` gets to
    /// the golden dandelion check (`canUseGoldenDandelion`, which a tadpole
    /// calls with `isBaby` true). A panda that is scared or on its back and an
    /// armadillo in its shell answer in their own `mobInteract` first.
    pub(super) fn age_lock_takes(&self, m: &Mob, item: &str) -> bool {
        let species = m.species();
        let baby = if species == Species::Tadpole {
            true
        } else if horizon_client::mobs::finalize::ageable(species) {
            m.is_baby()
                && !(species == Species::Panda
                    && (horizon_client::mobs::panda::scared(m)
                        || horizon_client::mobs::panda::is_on_back(m)))
                && !(species == Species::Armadillo && horizon_client::mobs::armadillo::scared(m))
        } else {
            false
        };
        let cannot = self
            .res
            .mob_data()
            .entity_has("minecraft:cannot_be_age_locked", &m.kind.id);
        horizon_client::mobs::ai::can_use_golden_dandelion(
            item,
            baby,
            m.extra.age_lock_particle_timer,
            cannot,
        )
    }
    /// `AgeableMob.setAgeLocked`: a golden dandelion on mob `index` flips its
    /// age lock (a tadpole's or an ageable mob's own `setAgeLockedData`) and is
    /// used up; a locked mob stays (`setPersistenceRequired`), and the lock's
    /// use or unuse sound plays at its block for everyone near.
    pub(super) fn set_age_locked(&mut self, index: usize) {
        let m = &mut self.mobs[index];
        if m.species() == Species::Tadpole {
            horizon_client::mobs::aquatic::set_age_locked_data(m);
        } else {
            horizon_client::mobs::ai::set_age_locked_data(m);
        }
        self.consume_held();
        let m = &mut self.mobs[index];
        let locked = m.extra.age_locked;
        if locked {
            m.persistent = true;
        }
        // Level.playSound(null, blockPosition, ..): the middle of the block.
        let at = [
            m.position.x.floor() + 0.5,
            m.position.y.floor() + 0.5,
            m.position.z.floor() + 0.5,
        ];
        let chunk = (
            m.position.x.floor() as i32 >> 4,
            m.position.z.floor() as i32 >> 4,
        );
        self.world.modified.insert(chunk);
        let sound = if locked {
            "minecraft:item.golden_dandelion.use"
        } else {
            "minecraft:item.golden_dandelion.unuse"
        };
        self.audio.play(sound, Some(at), 1.0, 1.0);
    }
    /// The mob a normal interaction can reach: a packet's exact validated ID,
    /// or the crosshair's mob before its blocking shape.
    pub(in crate::game) fn mob_interaction_target(
        &self,
        target: Option<InteractTarget>,
    ) -> Option<usize> {
        let (index, distance, _) = (match target {
            // The id the packet named, under the filters the crosshair pick
            // applies (`mob_hit`): a mob in its death animation is not clicked
            // by either path, so naming one in a packet must not reach the
            // feed, shear, leash or name paths either.
            Some(t) => Self::named_entity(&self.mobs, t.id, |m| m.id)
                .filter(|&i| {
                    let m = &self.mobs[i];
                    named_mob_pickable(m.alive(), m.id, self.riding, self.remote_camera_id())
                })
                .map(|i| (i, 0.0, [0.0; 3])),
            None => self.mob_hit(),
        })?;
        // Only a client's own pick is stopped by a block: the range check passed.
        if target.is_none()
            && let Some(hit) = self.pick_unfiltered()
            && let Some(eye) = self.eye_position()
        {
            let d = ((hit.point[0] - eye[0]).powi(2)
                + (hit.point[1] - eye[1]).powi(2)
                + (hit.point[2] - eye[2]).powi(2))
            .sqrt();
            if d < distance {
                return None;
            }
        }
        Some(index)
    }
    /// Right click on a mob (Mob.interact / mobInteract), on the mob under the
    /// crosshair or on the one a guest's interact packet named: returns true
    /// when consumed.
    pub(super) fn interact_mob(&mut self, target: Option<InteractTarget>) -> Result<bool> {
        let Some(index) = self.mob_interaction_target(target) else {
            return Ok(false);
        };
        // PlayerInteractTrigger: the name tag as it was before it named the mob.
        let used = self.held().cloned();
        if self.name_mob(index) {
            if let Some(stack) = &used {
                let target = self.mob_entity(&self.mobs[index]);
                self.trigger_item_entity("minecraft:player_interacted_with_entity", stack, &target);
            }
            return Ok(true);
        }
        // Shears ask the mob's entity parts first, then fire `player_shear_entity`.
        if self.held_shears()
            && let Some(consumed) = self.player_shear(index)
        {
            return Ok(consumed);
        }
        if self.spawn_egg_offspring(index) {
            self.swing_arm();
            return Ok(true);
        }
        {
            let item = self.held().map(|h| h.item.to_string()).unwrap_or_default();
            let used = self.held().cloned();
            if let Some(context) = self.interact_leash_with_item_context(index, &item) {
                let used = match context {
                    super::leads::LeashInteraction::WithItem => used,
                    super::leads::LeashInteraction::WithoutItem => {
                        Some(ItemStack::new("minecraft:air", 0))
                    }
                };
                if let Some(stack) = &used {
                    let target = self.mob_entity(&self.mobs[index]);
                    self.trigger_item_entity(
                        "minecraft:player_interacted_with_entity",
                        stack,
                        &target,
                    );
                }
                self.swing_arm();
                return Ok(true);
            }
        }
        let held = self.held().cloned();
        let item = held
            .as_ref()
            .map(|h| h.item.to_string())
            .unwrap_or_default();
        let shears = self.held_shears();
        // Bucketable.bucketMobPickup: a water bucket scoops up a fish, a
        // tadpole or an axolotl, with the data its bucket carries.
        if item == "minecraft:water_bucket"
            && self.mobs[index].alive()
            && let Some((bucket, sound)) = bucketable(self.mobs[index].species())
        {
            let mut filled = ItemStack::new(bucket, 1);
            filled.extra = Some(Box::new(horizon_client::entity_nbt::bucket_components(
                &self.mobs[index],
            )));
            self.audio
                .play(sound, Some(self.mobs[index].center()), 1.0, 1.0);
            self.trigger_item("minecraft:filled_bucket", &filled);
            // PlayerInteractTrigger: the water bucket as it was, and the mob picked up.
            if let Some(stack) = &held {
                let target = self.mob_entity(&self.mobs[index]);
                self.trigger_item_entity("minecraft:player_interacted_with_entity", stack, &target);
            }
            let species = bucket.trim_end_matches("_bucket").to_owned();
            let id = self.mobs[index].id as i32;
            // Leashable.dropLeash, then discard.
            self.break_leash(id as u32, true, false);
            self.mobs[index].removed = true;
            if self.creative {
                if let Some(rest) = self.add_stack_to_inventory(filled) {
                    self.spawn_item(rest, self.eye_position().unwrap_or([0.0; 3]), [0.0; 3], 40);
                }
            } else {
                *self.held_mut() = Some(filled);
            }
            if let (Some(player), Some(name)) = (self.lan_acting_uuid, self.lan_acting_name.clone())
            {
                self.native_player_bucket_entity(player, &name, id, &species);
            }
            self.swing_arm();
            return Ok(true);
        }
        // AgeableMob.mobInteract: a golden dandelion on a baby freezes its
        // growth, or lets it grow again.
        if self.mobs[index].alive() && self.age_lock_takes(&self.mobs[index], &item) {
            let entity_id = self.mobs[index].id;
            self.set_age_locked(index);
            if let (Some(player), Some(name)) = (self.lan_acting_uuid, self.lan_acting_name.clone())
            {
                self.native_player_toggle_entity_age_lock(
                    player,
                    &name,
                    entity_id,
                    self.mobs[index].extra.age_locked,
                );
            }
            if let Some(stack) = &held {
                let target = self.mob_entity(&self.mobs[index]);
                self.trigger_item_entity("minecraft:player_interacted_with_entity", stack, &target);
            }
            self.swing_arm();
            return Ok(true);
        }
        let creative = self.creative;
        // The hand's slot (Player.interactOn passes the hand).
        let selected = self.held_slot();
        // Panda.mobInteract: bamboo, or a click on a panda lying on its back.
        if self.mobs[index].species() == Species::Panda && self.mobs[index].alive() {
            let taken = self.interact_panda(index, &item);
            if taken {
                self.swing_arm();
            }
            return Ok(taken);
        }
        if self.interact_zombie_villager(index, &item) {
            return Ok(true);
        }
        if self.interact_trader(index, &item)? {
            self.swing_arm();
            return Ok(true);
        }
        if !self.mobs[index].kind.food.contains(&item)
            && self.interact_sheep_dye(index, self.hand, false)
        {
            // PlayerInteractTrigger: the dye as it was before it coloured the sheep.
            if let Some(stack) = &held {
                let target = self.mob_entity(&self.mobs[index]);
                self.trigger_item_entity("minecraft:player_interacted_with_entity", stack, &target);
            }
            self.swing_arm();
            return Ok(true);
        }
        if self.interact_allay(index, held.as_ref()) {
            self.swing_arm();
            return Ok(true);
        }
        if self.interact_sulfur_cube(index, &item) {
            if let Some(stack) = &held {
                let target = self.mob_entity(&self.mobs[index]);
                self.trigger_item_entity("minecraft:player_interacted_with_entity", stack, &target);
            }
            self.swing_arm();
            return Ok(true);
        }
        if self.interact_armadillo(index, &item) {
            if let Some(stack) = &held {
                let target = self.mob_entity(&self.mobs[index]);
                self.trigger_item_entity("minecraft:player_interacted_with_entity", stack, &target);
            }
            self.swing_arm();
            return Ok(true);
        }
        if self.interact_copper_golem(index, &item) {
            self.swing_arm();
            return Ok(true);
        }
        if self.interact_shear_mob(index, shears) {
            self.swing_arm();
            return Ok(true);
        }
        if let Some(sheared) = self.shear_saddle(index, &item) {
            if sheared {
                self.swing_arm();
            }
            return Ok(sheared);
        }
        // AbstractHorse.mobInteract: feed, equip or ride.
        if self.interact_mount(index, &item) {
            self.swing_arm();
            return Ok(true);
        }
        // PiglinAi.mobInteract: hand over barter currency to admire.
        if self.mobs[index].alive() && held.as_ref().is_some_and(|h| self.give_piglin(index, h)) {
            self.swing_arm();
            if !creative && let Some(stack) = self.stack_in_mut(selected).as_mut() {
                stack.count -= 1;
                if stack.count == 0 {
                    *self.stack_in_mut(selected) = None;
                }
            }
            return Ok(true);
        }
        // Creeper.mobInteract: any item in #creeper_igniters, damaged when it
        // is damageable and used up otherwise.
        let creeper_igniter = self
            .res
            .items()
            .item_tags
            .get("minecraft:creeper_igniters")
            .is_some_and(|t| t.contains(&*item));
        let fish = self
            .res
            .items()
            .item_tags
            .get("minecraft:fishes")
            .is_some_and(|t| t.contains(&*item));
        let damageable = self
            .res
            .items()
            .get(&item)
            .is_some_and(|i| i.max_damage > 0);
        // Wolf.mobInteract: the owner's #wolf_collar_dyes item only ever dyes the
        // collar; one of the colour the collar already has does nothing.
        let wolf_collar_dye = self
            .res
            .items()
            .item_tags
            .get("minecraft:wolf_collar_dyes")
            .is_some_and(|t| t.contains(&*item));
        let parrot_poisonous = self
            .res
            .items()
            .item_tags
            .get("minecraft:parrot_poisonous_food")
            .is_some_and(|t| t.contains(&*item));
        let player_at = self.eye_position();
        let player_feet = self
            .player
            .as_ref()
            .map(|p| [p.position.x, p.position.y, p.position.z]);
        let acting = self.acting_player_uuid();
        let (acting_id, acting_entity_id) = (self.native_acting_id(), self.own_entity_id());
        let sneaking =
            self.keys.held(Action::Sneak) || self.keys.raw(&winit::keyboard::KeyCode::ShiftRight);
        let held_damage = held.as_ref().map_or(0, |h| h.damage as i32);
        let held_extra = held.as_ref().and_then(|h| h.extra.clone());
        let m = &mut self.mobs[index];
        if !m.alive() {
            return Ok(false);
        }
        let species = m.species();
        let center = m.center();
        // Wolf armor the owner takes off with shears or leaves a dropped stack of.
        let mut armor_drop: Option<ItemStack> = None;
        let mut equip_event: Option<bool> = None;
        let mut consume = false;
        let mut give: Option<&str> = None;
        let mut sound: Option<(&str, f32, f32)> = None;
        // Sounds played as vanilla plays them: (event, volume, pitch) at the feet.
        let mut eat: Option<(&'static str, f32, f32)> = None;
        let feet = [m.position.x, m.position.y, m.position.z];
        let mut handled = true;
        let mut ignited_creeper = false;
        let mut clear_wolf_target = false;
        let mut drops: Vec<(String, u32)> = Vec::new();
        // The shearing table rolled, for the mods.
        let mut drops_table = String::new();
        let mob = (m.id, super::native_hooks::mob_kind(m));
        // `Sheep.shear` adds a spread on top of the ItemEntity's own motion.
        let mut wool = false;
        let is_food = m.kind.food.contains(&item);
        let baby_turtle = species == Species::Turtle && m.is_baby();
        // MushroomCow.getEffectsFromItemStack: the item's block supplies them.
        let flower = if species == Species::Mooshroom {
            horizon_client::flowers::stew_effect(&item)
        } else {
            None
        };
        let mut give_stew: Option<Vec<(Arc<str>, i32)>> = None;
        // `Some(true)` for the four spell particles, `Some(false)` for the two
        // smoke puffs of a cow that already holds effects.
        let mut stew_particles: Option<bool> = None;
        // `Ocelot.mobInteract`: whether the fish won its trust.
        let mut ocelot_trust: Option<bool> = None;
        match species {
            Species::Cow | Species::Mooshroom if item == "minecraft:bucket" && !m.is_baby() => {
                give = Some("minecraft:milk_bucket");
                consume = true;
                sound = Some(("entity.cow.milk", 1.0, 1.0));
            }
            // Goat.mobInteract: a bucket on an adult gives milk.
            Species::Goat if item == "minecraft:bucket" && !m.is_baby() => {
                give = Some("minecraft:milk_bucket");
                consume = true;
                let milk = if m.extra.screaming {
                    "entity.goat.screaming.milk"
                } else {
                    "entity.goat.milk"
                };
                sound = Some((milk, 1.0, 1.0));
            }
            // MushroomCow.mobInteract: a bowl gives back what a flower left as
            // a suspicious stew, or plain stew when it holds no effects.
            Species::Mooshroom if item == "minecraft:bowl" && !m.is_baby() => {
                consume = true;
                match m.extra.stew_effects.take() {
                    Some(effects) => {
                        give = Some("minecraft:suspicious_stew");
                        give_stew = Some(effects);
                        // `playSound(.., 1.0F, 1.0F)` at the feet.
                        eat = Some(("entity.mooshroom.suspicious_milk", 1.0, 1.0));
                    }
                    None => {
                        give = Some("minecraft:mushroom_stew");
                        sound = Some(("entity.mooshroom.milk", 1.0, 1.0));
                    }
                }
            }
            Species::Mooshroom if shears && !m.is_baby() => {
                let player = Some(self.native_acting_id());
                self.shear_mooshroom(index, player, held.as_ref().expect("held shears"));
                let ctx = self.who_context(Some(horizon_client::mobs::Who::Player), None);
                self.post_game_event("minecraft:shear", feet, ctx);
                self.damage_held_tool();
            }
            // MushroomCow.mobInteract: a brown adult takes the flower's effects
            // unless it holds some already, when it only puffs smoke.
            Species::Mooshroom if m.extra.variant == 1 && !m.is_baby() && flower.is_some() => {
                if m.extra.stew_effects.is_some() {
                    stew_particles = Some(false);
                } else {
                    let (effect, ticks) = flower.expect("the guard checked");
                    m.extra.stew_effects = Some(vec![(Arc::from(effect), ticks)]);
                    consume = true;
                    eat = Some(("entity.mooshroom.eat", 2.0, 1.0));
                    stew_particles = Some(true);
                }
            }
            Species::Sheep if shears && !m.extra.sheared && !m.is_baby() => {
                self.audio.play_bound(
                    "minecraft:entity.sheep.shear",
                    7,
                    1.0,
                    1.0,
                    horizon_client::sound_engine::SoundEntity::new(m.id, feet, m.silent),
                    None,
                );
                m.extra.sheared = true;
                let color = horizon_client::mobs::render::DYE_NAMES[m.extra.color as usize & 15];
                drops_table = format!("minecraft:shearing/sheep/{color}");
                let stacks = world_items!(self).loot_table(
                    &drops_table,
                    false,
                    None,
                    &mut self.world.random,
                );
                drops.extend(stacks.into_iter().map(|s| (s.item.to_string(), s.count)));
                wool = true;
                let ctx = self.who_context(Some(horizon_client::mobs::Who::Player), None);
                self.post_game_event("minecraft:shear", feet, ctx);
                self.damage_held_tool();
            }
            // Entity.interact: shears take a tamed wolf's armor off for its owner.
            Species::Wolf
                if item == "minecraft:shears"
                    && m.extra.tame
                    && m.extra.owner == Some(acting)
                    && m.extra.body_id().is_some()
                    && !sneaking =>
            {
                armor_drop = m.equipment_stacks()[6].clone();
                m.extra.set_body_stack(None);
                eat = Some(("item.armor.unequip_wolf", 1.0, 1.0));
                equip_event = Some(false);
                self.damage_held_tool();
            }
            // Wolf.mobInteract: heal, dye the collar or sit/stand when tame;
            // a bone tames a wild one a third of the time.
            Species::Wolf if m.extra.tame && is_food && m.health < m.attrs.max_health => {
                let heal = self
                    .res
                    .items()
                    .get(&item)
                    .and_then(|i| i.food.as_ref())
                    .map_or(2.0, |f| f.0 as f32 * 2.0);
                // TamableAnimal.feed(.., 2.0F, 2.0F): heals twice the nutrition,
                // then playEatingSound, which wolves leave empty.
                m.heal(&self.loop_context, heal);
                consume = true;
            }
            // Wolf.mobInteract: the owner puts wolf armor on a tamed adult.
            Species::Wolf
                if item == "minecraft:wolf_armor"
                    && m.extra.tame
                    && m.extra.owner == Some(acting)
                    && m.extra.body_id().is_none()
                    && !m.is_baby() =>
            {
                let mut body = ItemStack::new(item.as_str(), 1);
                body.damage = held_damage.max(0) as u32;
                body.extra = held_extra;
                m.extra.set_body_stack(Some(body));
                consume = true;
                eat = Some(("item.armor.equip_wolf", 1.0, 1.0));
                equip_event = Some(true);
            }
            // Wolf.mobInteract: an armadillo scute repairs a sitting wolf's
            // armor by an eighth of its durability.
            Species::Wolf
                if item == "minecraft:armadillo_scute"
                    && m.extra.tame
                    && m.extra.owner == Some(acting)
                    && m.extra.sitting
                    && m.extra.body_id().is_some()
                    && m.extra.body_damage() > 0 =>
            {
                m.extra.set_body_damage(
                    m.extra.body_damage()
                        - (horizon_client::mobs::WOLF_ARMOR_DURABILITY as f32 * 0.125) as i32,
                );
                consume = true;
                eat = Some(("item.wolf_armor.repair", 1.0, 1.0));
            }
            Species::Wolf
                if m.extra.tame
                    && m.extra.owner == Some(acting)
                    && item.ends_with("_dye")
                    && horizon_client::mobs::render::DYE_NAMES
                        .iter()
                        .position(|d| {
                            item.trim_start_matches("minecraft:")
                                .trim_end_matches("_dye")
                                == *d
                        })
                        .is_some_and(|c| c as u8 != m.extra.color) =>
            {
                let name = item
                    .trim_start_matches("minecraft:")
                    .trim_end_matches("_dye");
                if let Some(c) = horizon_client::mobs::render::DYE_NAMES
                    .iter()
                    .position(|d| *d == name)
                {
                    m.extra.color = c as u8;
                }
                consume = true;
            }
            // Wolf.mobInteract: Animal.mobInteract first breeds an adult or
            // grows a baby with the food; only when it did not does the owner
            // make the pet sit or stand.
            Species::Wolf
                if m.extra.tame
                    && m.extra.owner == Some(acting)
                    && !wolf_collar_dye
                    && !(is_food && takes_food(m)) =>
            {
                m.extra.sitting = !m.extra.sitting;
                m.ai.jumping = false;
                m.nav_stop();
                clear_wolf_target = true;
                self.world.modified.insert((
                    m.position.x.floor() as i32 >> 4,
                    m.position.z.floor() as i32 >> 4,
                ));
            }
            Species::Wolf
                if !m.extra.tame
                    && item == "minecraft:bone"
                    && !horizon_client::mobs::tame::angry(m) =>
            {
                consume = true;
                let rolled = m.random.next_int_bound(3).unwrap_or(1) == 0;
                let tamed =
                    rolled && !self.animal_tame_cancelled(index, acting_id, acting_entity_id);
                let m = &mut self.mobs[index];
                if tamed {
                    horizon_client::mobs::tame::set_tame(m, true);
                    m.extra.owner = Some(acting);
                    m.persistent = true;
                    m.nav_stop();
                    clear_wolf_target = true;
                    m.extra.sitting = true;
                }
                // TamableAnimal.spawnTamingParticles (entity event 7 or 6).
                self.mob_event_particles(self.mobs[index].id, if tamed { 7 } else { 6 });
                if tamed {
                    let view = self.mob_entity(&self.mobs[index]);
                    self.trigger_entity("minecraft:tame_animal", "entity", &view);
                    self.pet_tamed(index);
                }
                let m = &self.mobs[index];
                self.world.modified.insert((
                    m.position.x.floor() as i32 >> 4,
                    m.position.z.floor() as i32 >> 4,
                ));
            }
            // Cat.mobInteract: dye, heal or sit when tame; fish may tame a wild one.
            Species::Cat
                if m.extra.tame
                    && m.extra.owner == Some(acting)
                    && item.ends_with("_dye")
                    && horizon_client::mobs::render::DYE_NAMES
                        .iter()
                        .position(|d| {
                            item.trim_start_matches("minecraft:")
                                .trim_end_matches("_dye")
                                == *d
                        })
                        .is_some_and(|c| c as u8 != m.extra.color) =>
            {
                let name = item
                    .trim_start_matches("minecraft:")
                    .trim_end_matches("_dye");
                if let Some(c) = horizon_client::mobs::render::DYE_NAMES
                    .iter()
                    .position(|d| *d == name)
                {
                    m.extra.color = c as u8;
                }
                m.persistent = true;
                consume = true;
            }
            Species::Cat
                if m.extra.tame
                    && m.extra.owner == Some(acting)
                    && is_food
                    && m.health < m.attrs.max_health =>
            {
                let heal = self
                    .res
                    .items()
                    .get(&item)
                    .and_then(|i| i.food.as_ref())
                    .map_or(1.0, |f| f.0 as f32);
                // TamableAnimal.feed.
                m.heal(&self.loop_context, heal);
                consume = true;
                eat = eating_sound(m, &mut self.world.random);
            }
            // Cat.mobInteract: Animal.mobInteract first breeds an adult or
            // grows a baby with the food; only when it did not does the owner
            // make the pet sit or stand.
            Species::Cat
                if m.extra.tame && m.extra.owner == Some(acting) && !(is_food && takes_food(m)) =>
            {
                m.extra.sitting = !m.extra.sitting;
                self.world.modified.insert((
                    m.position.x.floor() as i32 >> 4,
                    m.position.z.floor() as i32 >> 4,
                ));
            }
            // Parrot.mobInteract: the owner makes a tame parrot on the ground
            // sit or stand (poisonous food is handled apart).
            Species::Parrot
                if m.extra.tame
                    && m.extra.owner == Some(acting)
                    && m.on_ground
                    && !parrot_poisonous =>
            {
                m.extra.sitting = !m.extra.sitting;
                self.world.modified.insert((
                    m.position.x.floor() as i32 >> 4,
                    m.position.z.floor() as i32 >> 4,
                ));
            }
            Species::Cat if !m.extra.tame && is_food => {
                consume = true;
                m.persistent = true;
                let rolled = m.random.next_int_bound(3).unwrap_or(1) == 0;
                let tamed =
                    rolled && !self.animal_tame_cancelled(index, acting_id, acting_entity_id);
                let m = &mut self.mobs[index];
                if tamed {
                    m.extra.tame = true;
                    m.extra.owner = Some(acting);
                    m.extra.sitting = true;
                }
                eat = eating_sound(m, &mut self.world.random);
                // TamableAnimal.spawnTamingParticles (entity event 7 or 6).
                self.mob_event_particles(self.mobs[index].id, if tamed { 7 } else { 6 });
                if tamed {
                    let view = self.mob_entity(&self.mobs[index]);
                    self.trigger_entity("minecraft:tame_animal", "entity", &view);
                    self.pet_tamed(index);
                }
                let m = &self.mobs[index];
                self.world.modified.insert((
                    m.position.x.floor() as i32 >> 4,
                    m.position.z.floor() as i32 >> 4,
                ));
            }
            // IronGolem.mobInteract: an iron ingot repairs 25 health.
            Species::IronGolem
                if item == "minecraft:iron_ingot" && m.health < m.attrs.max_health =>
            {
                m.heal(&self.loop_context, 25.0);
                consume = true;
                eat = Some(("entity.iron_golem.repair", 1.0, m.golem_repair_pitch()));
            }
            Species::Creeper if creeper_igniter => {
                m.extra.flag = true;
                ignited_creeper = true;
                // FIRECHARGE_USE for a fire charge, else FLINTANDSTEEL_USE,
                // at the creeper's feet with a pitch from its own random.
                let event = if item == "minecraft:fire_charge" {
                    "item.firecharge.use"
                } else {
                    "item.flintandsteel.use"
                };
                eat = Some((event, 1.0, m.random.next_float() * 0.4 + 0.8));
                if damageable {
                    self.damage_held_tool();
                } else {
                    consume = true;
                }
            }
            // Parrot.mobInteract: #parrot_poisonous_food (a cookie) poisons the
            // parrot for 45 s and then hurts it with a player attack of
            // Float.MAX_VALUE, which an invulnerable one shrugs off.
            Species::Parrot
                if self
                    .res
                    .items()
                    .item_tags
                    .get("minecraft:parrot_poisonous_food")
                    .is_some_and(|t| t.contains(&*item)) =>
            {
                consume = true;
                m.add_effect("minecraft:poison", 900, 0);
                let id = m.id;
                let hurt = m.damage(
                    crate::game::entity::damage_level_of!(self),
                    f32::MAX,
                    horizon_client::mobs::Damage::player(player_at.unwrap_or(center)),
                );
                // The feeder gets the kill, as for a normal player attack.
                if hurt {
                    self.remember_mob_player_credit(id);
                }
            }
            // Parrot.mobInteract: parrot food may tame a wild one (one time in
            // ten); a parrot never takes it as Animal food (isFood is false).
            Species::Parrot if !m.extra.tame && is_food => {
                consume = true;
                if !m.silent {
                    let pitch = 1.0 + (m.random.next_float() - m.random.next_float()) * 0.2;
                    eat = Some(("entity.parrot.eat", 1.0, pitch));
                }
                let rolled = m.random.next_int_bound(10).unwrap_or(1) == 0;
                let tamed =
                    rolled && !self.animal_tame_cancelled(index, acting_id, acting_entity_id);
                let m = &mut self.mobs[index];
                if tamed {
                    m.extra.tame = true;
                    m.extra.owner = Some(acting);
                }
                // TamableAnimal.spawnTamingParticles (entity event 7 or 6).
                self.mob_event_particles(self.mobs[index].id, if tamed { 7 } else { 6 });
                if tamed {
                    let view = self.mob_entity(&self.mobs[index]);
                    self.trigger_entity("minecraft:tame_animal", "entity", &view);
                    self.pet_tamed(index);
                }
            }
            // Dolphin.mobInteract: a fish makes a baby grow by a tenth and an
            // adult remember it (`gotFish`), to swim to a treasure.
            Species::Dolphin if fish => {
                eat = Some(("entity.dolphin.eat", 1.0, 1.0));
                if horizon_client::mobs::ai::can_age_up(m) {
                    let secs = ((-m.extra.age / 20) as f32 * 0.1) as i32;
                    horizon_client::mobs::ai::age_up(m, secs, true);
                } else {
                    m.extra.flag = true;
                }
                consume = true;
            }
            // Tadpole.feed: frog food ages it by a tenth of the time left to
            // grow into a frog (it grows on its next tick once past 24000).
            Species::Tadpole if is_food && !m.extra.age_locked => {
                consume = true;
                let left = 24_000 - m.extra.age;
                m.extra.age += ((left / 20) as f32 * 0.1) as i32 * 20;
                // One happy villager particle at rest about the body.
                self.particles_around_mob(
                    self.mobs[index].id,
                    "happy_villager",
                    horizon_client::particles::Burst::HORSE_EATING,
                );
            }
            // Bee.mobInteract: a wither rose or an open eyeblossom is eaten for
            // its `getBeeInteractionEffect` instead of breeding the bee.
            Species::Bee
                if is_food
                    && matches!(
                        &*item,
                        "minecraft:wither_rose" | "minecraft:open_eyeblossom"
                    ) =>
            {
                if &*item == "minecraft:wither_rose" {
                    m.add_effect("minecraft:wither", 40, 0);
                } else {
                    m.add_effect("minecraft:poison", 25, 0);
                }
                consume = true;
            }
            // Ocelot.mobInteract: raw fish from a hand within 3 blocks, while it comes for the
            // food (`temptGoal.isRunning`), is used up; one time in three it trusts the player
            // (entity event 41, hearts), otherwise smoke (event 40). Anything else is
            // `Animal.mobInteract`.
            Species::Ocelot
                if is_food
                    && !m.extra.trusting
                    && m.ai.goals.is_running("OcelotTemptGoal")
                    && player_feet.is_some_and(|p| {
                        let d = [
                            p[0] - m.position.x,
                            p[1] - m.position.y,
                            p[2] - m.position.z,
                        ];
                        d[0] * d[0] + d[1] * d[1] + d[2] * d[2] < 9.0
                    }) =>
            {
                consume = true;
                let trusts = m.random.next_int_bound(3).unwrap_or(1) == 0;
                if trusts {
                    m.extra.trusting = true;
                }
                ocelot_trust = Some(trusts);
            }
            // HappyGhast.canFallInLove: never, so only a ghastling takes food.
            _ if is_food
                && m.kind.flags.animal
                && species != Species::Parrot
                && (species != Species::HappyGhast || m.is_baby()) =>
            {
                // Animal.mobInteract: breed adults, speed up babies, then
                // playEatingSound (silent unless the species overrides it).
                match feed_animal(m, &mut self.world.random, Some(acting)) {
                    Some(sound) => {
                        consume = true;
                        eat = sound;
                        // Axolotl.usePlayerItem: a tropical fish bucket is
                        // emptied into a water bucket in its place.
                        if species == Species::Axolotl && &*item == "minecraft:tropical_fish_bucket"
                        {
                            give = Some("minecraft:water_bucket");
                        }
                        // Animal.setInLove: entity event 18 (a baby that
                        // only grew is not in love).
                        if self.mobs[index].extra.in_love > 0 {
                            let id = self.mobs[index].id;
                            self.love_cause.insert(id, self.lan_acting);
                            let kind = self.mobs[index].kind.id.clone();
                            self.native_entity_enter_love_mode(id, &kind, self.lan_acting_uuid);
                            self.mob_event_particles(id, 18);
                        }
                    }
                    // Animal.mobInteract still consumes the client click when
                    // an adult cannot breed, without swinging or taking food.
                    None => return Ok(true),
                }
            }
            _ => handled = false,
        }
        if !handled {
            return Ok(false);
        }
        if clear_wolf_target {
            self.set_event_ai_target(mob.0, None, self.lan_acting);
        }
        if ignited_creeper {
            self.native_creeper_ignite(
                self.mobs[index].id,
                horizon_client::lan::playerdata::ints_uuid(acting),
                &item,
            );
        }
        if let Some(trusts) = ocelot_trust {
            // `spawnTrustingParticles` on the clients: entity event 41 (hearts) or 40 (smoke).
            self.mob_event_particles(self.mobs[index].id, if trusts { 41 } else { 40 });
        }
        if let Some(fresh) = stew_particles {
            // `SpellParticleOption.create(EFFECT, -1, 1)` is the plain white
            // effect particle; the smoke is the already-fed answer.
            let ty = if fresh {
                ParticleType::Effect
            } else {
                ParticleType::Smoke
            };
            let options = ParticleOptions::simple(ty);
            let at = [feet[0], feet[1] + 0.5, feet[2]];
            let (spread, count) = ([0.5, 0.0, 0.5], if fresh { 4 } else { 2 });
            // shortcut: scalar speed models DEFAULT randomization; 26.3 sends
            // ALTERNATIVE_WITH_SPEED with a per-axis maximum speed.
            self.particles
                .send_particles(&options, at, spread, 0.2, count);
            self.lan_send_particles(super::lan_particles::QueuedParticle {
                dimension: self.dim.id(),
                options,
                state: None,
                at,
                spread,
                speed: [0.2; 3],
                count: count as i32,
                randomization: 0,
            });
        }
        // PlayerInteractTrigger: the item used on the mob, once the interaction took.
        if let Some(stack) = &held {
            let target = self.mob_entity(&self.mobs[index]);
            self.trigger_item_entity("minecraft:player_interacted_with_entity", stack, &target);
        }
        // Turtle.ageBoundaryReached: food that finishes a baby's growth.
        if baby_turtle && !self.mobs[index].is_baby() {
            self.handle_mob_events(vec![MobEvent::GiftLoot {
                mob: self.mobs[index].id,
                table: "minecraft:gameplay/turtle_grow",
                at: feet,
            }])?;
        }
        self.swing_arm();
        match sound {
            // Sheep.shear: level.playSound(null, this, SHEEP_SHEAR, PLAYERS, 1, 1).
            // DyeItem.interactLivingEntity: level.playSound(player, sheep,
            // DYE_USE, PLAYERS, 1, 1), whose player is left out because their
            // client played it; Horizon's own client plays it from the packet,
            // so every player gets it.
            Some((s @ ("entity.sheep.shear" | "item.dye.use"), volume, pitch)) => {
                let name = full_id(s);
                let id = self.mobs[index].id;
                let sound =
                    LevelSound::new(&name, feet, volume, pitch).in_source(SoundSource::Players);
                self.play_bound_sound(sound, id, None);
            }
            Some((s, volume, pitch)) => {
                self.audio.play(&full_id(s), Some(feet), volume, pitch);
            }
            None => {}
        }
        if let Some((s, volume, pitch)) = eat {
            let name = full_id(s);
            if eating_sound_is_bound(species) {
                let id = self.mobs[index].id;
                let sound =
                    LevelSound::new(&name, feet, volume, pitch).in_source(SoundSource::Neutral);
                self.play_bound_sound(sound, id, None);
            } else {
                self.audio.play(&name, Some(feet), volume, pitch);
            }
        }
        if consume
            && !creative
            && let Some(stack) = self.stack_in_mut(selected).as_mut()
        {
            stack.count -= 1;
            if stack.count == 0 {
                *self.stack_in_mut(selected) = None;
            }
        }
        if let Some(g) = give {
            let mut stack = ItemStack {
                item: g.into(),
                count: 1,
                damage: 0,
                extra: None,
            };
            if let Some(effects) = give_stew {
                stack.data_mut().stew_effects = effects;
            }
            if self.stack_in(selected).is_none() {
                *self.stack_in_mut(selected) = Some(stack);
            } else if let Some(rest) = self.add_stack_to_inventory(stack) {
                let at = self.eye_position().unwrap_or(center);
                self.spawn_item(rest, at, [0.0, 0.0, 0.0], 40);
            }
        }
        if let Some(stack) = armor_drop {
            // Mob.shearItem: PLAYER_SHEARED_EQUIPMENT with the armor and the wolf.
            let target = self.mob_entity(&self.mobs[index]);
            self.trigger_item_entity("minecraft:player_sheared_equipment", &stack, &target);
            self.spawn_item(stack, [center[0], center[1] + 0.5, center[2]], [0.0; 3], 10);
        }
        if let Some(equip) = equip_event {
            let id = self.mobs[index].id;
            let pos = self.mobs[index].position;
            self.mob_equip_event(id, [pos.x, pos.y, pos.z], equip);
        }
        let mut drops: Vec<ItemStack> = drops
            .into_iter()
            .map(|(item, count)| ItemStack::new(&item, count))
            .collect();
        if !drops.is_empty() {
            let player = Some(self.native_acting_id());
            self.native_use_drops(
                &drops_table,
                feet,
                None,
                Some((mob.0, &mob.1)),
                player,
                held.as_ref(),
                &mut drops,
            );
        }
        for stack in drops {
            for _ in 0..stack.count {
                // `spawnAtLocation(stack, 1.0)`: an ItemEntity a block above
                // the feet that starts with the constructor's own motion, and
                // `setDefaultPickUpDelay`. Sheep.shear adds its spread on top;
                // MushroomCow.shear builds the entity by hand instead, so its
                // mushrooms keep the constructor's motion and no pickup delay.
                let mut spread = [0.0; 3];
                if wool {
                    spread[0] = (self.world.random.next_float() - self.world.random.next_float())
                        as f64
                        * 0.1;
                    spread[1] = self.world.random.next_float() as f64 * 0.05;
                    spread[2] = (self.world.random.next_float() - self.world.random.next_float())
                        as f64
                        * 0.1;
                }
                self.spawn_item_own(
                    ItemStack {
                        count: 1,
                        ..stack.clone()
                    },
                    [feet[0], feet[1] + 1.0, feet[2]],
                    if wool { 10 } else { 0 },
                );
                if let Some(e) = self.just_spawned_item() {
                    e.velocity = e
                        .velocity
                        .add_vector(Vec3::new(spread[0], spread[1], spread[2]));
                }
            }
        }
        Ok(true)
    }
    /// `SnowGolem.mobInteract` and `Bogged.mobInteract` with shears: the
    /// pumpkin or the mushrooms come off (the shearing loot table), the shear
    /// game event is posted and the tool takes one damage.
    fn interact_shear_mob(&mut self, index: usize, shears: bool) -> bool {
        let m = &self.mobs[index];
        if !shears || !m.alive() || m.extra.sheared {
            return false;
        }
        let (sound, table, height) = match m.species() {
            Species::SnowGolem => (
                "minecraft:entity.snow_golem.shear",
                "minecraft:shearing/snow_golem",
                m.eye_height(),
            ),
            Species::Bogged => (
                "minecraft:entity.bogged.shear",
                "minecraft:shearing/bogged",
                f64::from(m.height()),
            ),
            _ => return false,
        };
        let at = m.position;
        let mob = (m.id, super::native_hooks::mob_kind(m));
        let tool = self.held().cloned();
        // shear: level.playSound(null, this, sound, PLAYERS, 1, 1).
        let shear =
            LevelSound::new(sound, [at.x, at.y, at.z], 1.0, 1.0).in_source(SoundSource::Players);
        self.play_bound_sound(shear, mob.0, None);
        self.mobs[index].extra.sheared = true;
        let mut stacks = self.sequence_loot(table, table, tool.as_ref());
        let origin = [at.x, at.y, at.z];
        let player = Some(self.native_acting_id());
        self.native_use_drops(
            table,
            origin,
            None,
            Some((mob.0, &mob.1)),
            player,
            tool.as_ref(),
            &mut stacks,
        );
        for stack in stacks {
            self.spawn_at_location(at, height, stack);
        }
        let ctx = self.who_context(Some(horizon_client::mobs::Who::Player), None);
        self.post_game_event("minecraft:shear", [at.x, at.y, at.z], ctx);
        self.damage_held_tool();
        true
    }
    /// `Armadillo.mobInteract`: a brush brushes a scute off an adult
    /// (`brushOffScute`) and loses 16 durability.
    fn interact_armadillo(&mut self, index: usize, item: &str) -> bool {
        let m = &self.mobs[index];
        if item != "minecraft:brush"
            || m.species() != Species::Armadillo
            || !m.alive()
            || m.is_baby()
        {
            return false;
        }
        let (id, at) = (m.id, m.position);
        let feet = [at.x, at.y, at.z];
        let tool = self.held().cloned();
        let table = "minecraft:brush/armadillo";
        let mut stacks = self.sequence_loot(table, table, tool.as_ref());
        let kind = super::native_hooks::mob_kind(&self.mobs[index]);
        let player = Some(self.native_acting_id());
        self.native_use_drops(
            table,
            feet,
            None,
            Some((id, &kind)),
            player,
            tool.as_ref(),
            &mut stacks,
        );
        for d in stacks {
            self.spawn_at_location(at, 0.0, d);
        }
        self.audio
            .play("minecraft:entity.armadillo.brush", Some(feet), 1.0, 1.0);
        let ctx = self.who_context(Some(Who::Mob(id)), None);
        self.post_game_event("minecraft:entity_interact", feet, ctx);
        // ItemStack.hurtAndBreak(16).
        if !self.creative
            && let Some(stack) = tool
        {
            let max = self
                .res
                .items()
                .get(&stack.item)
                .map_or(0, |i| i.max_damage);
            if max > 0 {
                self.damage_held(16);
            }
        }
        true
    }
    /// `ItemStack.hurtAndBreak(1, ...)` on the held tool: Unbreaking, break sound and stat.
    pub(super) fn damage_held_tool(&mut self) {
        self.damage_held(1);
    }
    /// Throwables and spawn eggs (SnowballItem/EggItem/EnderpearlItem/SpawnEggItem.use).
    pub(super) fn throw_item(&mut self, stack: &ItemStack) -> Result<bool> {
        let item = &*stack.item;
        let kind = match item {
            "minecraft:snowball" => ProjectileKind::Snowball,
            "minecraft:egg" | "minecraft:blue_egg" | "minecraft:brown_egg" => ProjectileKind::Egg,
            "minecraft:ender_pearl" => ProjectileKind::EnderPearl,
            "minecraft:wind_charge" => {
                let thrown = self.throw_wind_charge(stack)?;
                if thrown {
                    self.apply_use_cooldown(stack);
                }
                return Ok(thrown);
            }
            _ if item.ends_with("_spawn_egg") => return self.use_spawn_egg(stack),
            _ => match horizon_client::projectiles::thrown_by_item(item) {
                Some(t) => ProjectileKind::Thrown(t),
                None => return Ok(false),
            },
        };
        let Some(eye) = self.eye_position() else {
            return Ok(false);
        };
        let Some(p) = self.player.as_ref() else {
            return Ok(false);
        };
        // Projectile.shootFromRotation adds getKnownMovement: what the thrower last moved.
        let on_ground = p.ground.on_ground;
        let velocity = self.known_player_movement();
        let start = Vec3::new(eye[0], eye[1] - 0.1, eye[2]);
        let mut proj = Projectile::new(kind.clone(), start, Some(Who::Player));
        proj.set_egg_item(stack);
        proj.shoot_from_rotation(self.pitch, self.yaw, 0.0, 1.5, 1.0, velocity, on_ground);
        self.spawn_projectile(proj, stack);
        // SnowballItem.use / EggItem.use / EnderpearlItem.use: awardStat(ITEM_USED).
        self.award_stat("used", item, 1);
        // ItemStack.use's applyAfterUseComponentSideEffects: the pearl's
        // second is the `use_cooldown` component, as for any thrown item.
        self.apply_use_cooldown(stack);
        let sound = match kind {
            ProjectileKind::Snowball | ProjectileKind::Thrown(_) => {
                "minecraft:entity.snowball.throw"
            }
            ProjectileKind::Egg => "minecraft:entity.egg.throw",
            _ => "minecraft:entity.ender_pearl.throw",
        };
        let pitch = 0.4 / (self.world.random.next_float() * 0.4 + 0.8);
        self.audio.play(sound, Some(eye), 0.5, pitch);
        self.swing_arm();
        if !self.creative
            && let Some(s) = self.held_mut().as_mut()
        {
            s.count -= 1;
            if s.count == 0 {
                *self.held_mut() = None;
            }
        }
        if kind == ProjectileKind::Egg
            && let (Some(player), Some(name)) = (self.lan_acting_uuid, self.lan_acting_name.clone())
        {
            self.native_player_egg_throw(player, &name);
        }
        Ok(true)
    }
    /// `Mob.checkAndHandleImportantInteractions` with a spawn egg of mob
    /// `index`'s own kind (`SpawnEggItem.spawnOffspringFromSpawnEgg`): a baby
    /// at the mob's feet, taking the parent's variant as `getBreedOffspring`
    /// does, and one egg used. False (PASS) when the mob has no baby form.
    fn spawn_egg_offspring(&mut self, index: usize) -> bool {
        let Some(stack) = self.held().cloned() else {
            return false;
        };
        let parent = &self.mobs[index];
        if !parent.alive()
            || !horizon_client::mobs::finalize::spawn_egg_is_for(&stack.item, &parent.kind.id)
            || !horizon_client::mobs::species::can_be_a_baby(parent.species())
            // WanderingTrader.getBreedOffspring returns null: no offspring, the
            // click carries on to trading.
            || parent.species() == Species::WanderingTrader
        {
            return false;
        }
        let (kind, at) = (
            parent.kind.id.clone(),
            [parent.position.x, parent.position.y, parent.position.z],
        );
        let (species, variant, color) =
            (parent.species(), parent.extra.variant, parent.extra.color);
        let target = self.mob_entity(&self.mobs[index]);
        // SpawnEggItem.spawnOffspringFromSpawnEgg: `type.create(level, SPAWN_ITEM_USE)`
        // creates the baby without finalizeSpawn, and `snapTo(pos, 0, 0)` faces
        // it south without a level-random draw.
        let Some(child) = self.create_mob_reason(&kind, at, false, SpawnSite::Raw, Some(0.0))
        else {
            return false;
        };
        let Some(id) = self.insert_spawned_mob(child) else {
            return false;
        };
        if let Some(child) = self.mobs.iter_mut().find(|m| m.id == id) {
            horizon_client::mobs::finalize::spawn_egg_offspring(child, species, variant, color);
        }
        // `Mob.checkAndHandleImportantInteractions` then calls
        // `onOffspringSpawnedFromEgg`: `Fox`'s makes the baby trust the player
        // who used the egg (a fox egg used on a block trusts nobody).
        if species == Species::Fox {
            let spawner = self.acting_player_uuid();
            if let Some(child) = self.mobs.iter_mut().find(|m| m.id == id) {
                horizon_client::mobs::fox::add_trusted_entity(child, spawner);
            }
        }
        self.trigger_item_entity("minecraft:player_interacted_with_entity", &stack, &target);
        self.consume_held();
        true
    }
    /// `SpawnEggItem.use`: the SOURCE_ONLY ray must hit a liquid block.
    pub(super) fn spawn_egg_fluid_hit(&self) -> Option<super::Hit> {
        let hit = self.bucket_hit()?;
        matches!(
            &*self.block_name(hit.pos),
            "minecraft:water" | "minecraft:lava"
        )
        .then_some(hit)
    }
    /// Shared peaceful-mode eligibility for spawn eggs.
    pub(super) fn spawn_egg_allowed(&self, item: &str) -> bool {
        let kind = item.trim_end_matches("_spawn_egg");
        self.ui.difficulty != 0
            || self
                .res
                .mob_types()
                .get(kind)
                .is_none_or(|t| horizon_client::mobs::spawn_rules::allowed_in_peaceful(t.species))
    }
    /// `SpawnEggItem.useOn` for a block, or `use` for a fluid source.
    fn use_spawn_egg(&mut self, stack: &ItemStack) -> Result<bool> {
        let fluid = matches!(self.guest_use, Some(super::GuestUse::Item(_)));
        let Some(hit) = (if fluid {
            self.spawn_egg_fluid_hit()
        } else {
            self.pick()
        }) else {
            return Ok(false);
        };
        let kind = stack.item.trim_end_matches("_spawn_egg").to_owned();
        if !self.spawn_egg_allowed(&stack.item) {
            self.use_failed = true;
            return Ok(true);
        }
        if fluid
            && !self.may_build()
            && !self.can_place_on_in_adventure([
                hit.pos[0] - hit.face[0],
                hit.pos[1] - hit.face[1],
                hit.pos[2] - hit.face[2],
            ])
        {
            self.use_failed = true;
            return Ok(true);
        }
        // SpawnEggItem.useOn on a spawner sets its mob (and an op's egg data).
        let (spawner_id, data) = horizon_client::block_entity_item::spawn_egg_on_spawner(
            stack,
            self.acting_player_is_op(),
        );
        if let Some(done) = self.use_spawn_egg_on_spawner(hit.pos, &spawner_id, data.as_ref()) {
            if done {
                self.swing_arm();
                self.award_stat("used", &stack.item, 1);
                self.consume_held();
            }
            return Ok(true);
        }
        // useOn: a clicked block without a collision shape (vines, grass)
        // takes the mob itself, any other the one on the clicked face.
        let p = if fluid || self.collision_empty(hit.pos) {
            hit.pos
        } else {
            [
                hit.pos[0] + hit.face[0],
                hit.pos[1] + hit.face[1],
                hit.pos[2] + hit.face[2],
            ]
        };
        let at = [
            f64::from(p[0]) + 0.5,
            f64::from(p[1]),
            f64::from(p[2]) + 0.5,
        ];
        let Some(id) =
            self.spawn_mob_spawned(&kind, at, SpawnSite::Finalizing(SpawnReason::SpawnItemUse))
        else {
            return Ok(false);
        };
        // SpawnEggItem.spawnMob: `EntityType.create` finalizes the mob, then
        // `PostSpawnProcessor` puts the egg's components on it.
        if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
            horizon_client::entity_nbt::apply_item_components(m, stack);
        }
        self.apply_spawn_egg_entity_data(id, stack);
        self.post_block_game_event("minecraft:entity_place", p, Some(Who::Player), None);
        self.award_stat("used", &stack.item, 1);
        self.swing_arm();
        self.consume_held();
        Ok(true)
    }
    /// `EntityType.updateCustomEntityTag`: the egg's `minecraft:entity_data` on
    /// the mob it spawned, once the guards let it through, as
    /// `TypedEntityData.loadInto`: the mob's own compound, the data merged over
    /// it, loaded back into the same mob (`/data merge entity`'s loader).
    pub(super) fn apply_spawn_egg_entity_data(&mut self, id: u32, stack: &ItemStack) {
        let Some(i) = self.mobs.iter().position(|m| m.id == id) else {
            return;
        };
        let kind = self.mobs[i].kind.id.clone();
        let op = self.acting_player_is_op();
        let Some(data) = horizon_client::block_entity_item::spawn_egg_entity_data(stack, &kind, op)
        else {
            return;
        };
        let own = self.mob_tree_nbt(i);
        let tag = horizon_client::data_cmd::merged(&own, data);
        // Nothing to report: the mob's own compound always loads back.
        let _ = self.apply_entity_nbt(super::commands::Target::Mob(id), tag);
    }
    /// A beam target's centre (half its height), interpolated.
    fn beam_target(&self, who: Option<Who>, alpha: f64) -> Option<glam::DVec3> {
        match who? {
            Who::Player => {
                let p = self.player.as_ref()?;
                let (a, b) = (self.previous_position, p.position);
                Some(glam::DVec3::new(
                    a.x + (b.x - a.x) * alpha,
                    a.y + (b.y - a.y) * alpha + f64::from(p.dimensions.height) * 0.5,
                    a.z + (b.z - a.z) * alpha,
                ))
            }
            Who::Mob(id) => {
                let m = self.mobs.iter().find(|m| m.id == id)?;
                Some(glam::DVec3::new(
                    m.previous.x + (m.position.x - m.previous.x) * alpha,
                    m.previous.y
                        + (m.position.y - m.previous.y) * alpha
                        + f64::from(m.height()) * 0.5,
                    m.previous.z + (m.position.z - m.previous.z) * alpha,
                ))
            }
        }
    }
    /// `Guardian.aiStep` on a client: two bubbles a tick trailing a guardian
    /// that swims along, and bubbles up the beam to its target. The server
    /// never draws them, so the mob's own random stays untouched.
    pub(super) fn guardian_bubbles(&mut self) {
        use horizon_client::mobs::guardian;
        use horizon_client::particles::{ParticleOptions, ParticleType};
        let mut swimming = Vec::new();
        let mut beams = Vec::new();
        for m in &self.mobs {
            if !m.alive() || !matches!(m.species(), Species::Guardian | Species::ElderGuardian) {
                continue;
            }
            let at = [m.position.x, m.position.y, m.position.z];
            if m.extra.flag2 && m.fluids.in_water {
                let (width, height, _) = m.dims();
                swimming.push((at, width, height, m.view_vector()));
            }
            if let Some((_, scale)) = guardian::beam(m, 0.0)
                && let Some(t) = self.beam_target(m.ai.target, 1.0)
            {
                beams.push(([at[0], m.eye_y(), at[2]], [t.x, t.y, t.z], f64::from(scale)));
            }
        }
        let bubble = ParticleOptions::simple(ParticleType::Bubble);
        for (at, width, height, view) in swimming {
            for _ in 0..2 {
                let (x, y, z) = {
                    let r = self.particles.random();
                    (r.next_double(), r.next_double(), r.next_double())
                };
                let p = [
                    at[0] + width * (2.0 * x - 1.0) * 0.5 - view[0] * 1.5,
                    at[1] + height * y - view[1] * 1.5,
                    at[2] + width * (2.0 * z - 1.0) * 0.5 - view[2] * 1.5,
                ];
                self.particles.add(&bubble, p, [0.0; 3]);
            }
        }
        for (eye, target, scale) in beams {
            let d = [target[0] - eye[0], target[1] - eye[1], target[2] - eye[2]];
            let length = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            if length <= 0.0 {
                continue;
            }
            let mut dist = self.particles.random().next_double();
            while dist < length {
                let r = self.particles.random();
                dist += 1.8 - scale + r.next_double() * (1.7 - scale);
                let f = dist / length;
                let p = [eye[0] + d[0] * f, eye[1] + d[1] * f, eye[2] + d[2] * f];
                self.particles.add(&bubble, p, [0.0; 3]);
            }
        }
    }
    /// Mob models and projectiles for the entity pass.
    /// The renderer's models for the GPU path (`mobs::skin`) this frame, if it
    /// poses entities: a new frame of draws, the store restarted when the
    /// renderer's no longer matches. Once per frame, before any entity draws.
    pub(super) fn begin_skinning(&self) {
        let renderer = self.renderer.as_ref().filter(|r| r.skins_entities());
        match renderer.and_then(|r| r.skinned_models()) {
            Some(stored) => {
                let mut models = self.skin_models.borrow_mut();
                // Reuse helps CPU-bound crowds, but regressed TAA frame tails in
                // windowed measurements. Keep the original arithmetic with TAA on.
                models.set_rotation_cache_enabled(!self.settings.horizon.taa);
                models.begin(&mut self.skin_frame.borrow_mut(), stored);
                models.tagged = renderer.is_some_and(|r| r.skins_tagged());
            }
            None => self.skin_frame.borrow_mut().clear_draws(),
        }
    }

    /// Whether this frame's models can be posed on the GPU (`begin_skinning`).
    pub(super) fn skinning(&self) -> bool {
        self.renderer
            .as_ref()
            .is_some_and(|r| r.skins_entities() && r.skinned_models().is_some())
    }

    /// Culls one entity's vertices from `entity_start`, counting what it drew
    /// on the GPU path by the corners of `skinned`'s bounds; a culled entity's
    /// posed draws since `skinned`'s mark are taken back.
    pub(super) fn cull_skinned(
        &self,
        out: &mut Vec<Vertex>,
        entity_start: usize,
        skinned: Skinned,
    ) {
        let corners = skinned.and_then(|(_, bounds)| bounds);
        if out.len() == entity_start {
            if let Some((lo, hi)) = corners
                && self.cull_entity_camera_bounds(lo, hi)
                && let Some((mark, _)) = skinned
            {
                if self.pack_shadow_casters.0 {
                    let mut frame = self.skin_frame.borrow_mut();
                    let end = frame.vertices as usize;
                    horizon_client::gfx::pack_draw::EntitySpan::hide(
                        &mut frame.spans,
                        mark.2 as usize,
                        end,
                    );
                } else {
                    self.skin_frame.borrow_mut().truncate(mark);
                }
            }
            return;
        }
        if let Some((lo, hi)) = corners {
            out.extend([lo, hi].map(|pos| Vertex {
                pos: pos.to_array(),
                ..bytemuck::Zeroable::zeroed()
            }));
        }
        let len = out.len();
        let hidden = self.cull_entity_vertices(out, entity_start);
        if out.len() < len {
            if let Some((mark, _)) = skinned {
                self.skin_frame.borrow_mut().truncate(mark);
            }
        } else if corners.is_some() {
            if let Some((mark, _)) = skinned {
                // Previous GPU-only entities can leave zero-length CPU spans at
                // this same offset. Use this query's result, not their metadata.
                if hidden {
                    let mut skin = self.skin_frame.borrow_mut();
                    let end = skin.vertices as usize;
                    EntitySpan::hide(&mut skin.spans, mark.2 as usize, end);
                }
            }
            out.truncate(len - 2);
            // The temporary bounds are not real geometry. Keep the culling end marker
            // at the true CPU end before the next entity appends a span there.
            for span in self.entity_spans.borrow_mut().iter_mut().rev() {
                if span.start as usize <= out.len() {
                    break;
                }
                span.start = out.len() as u32;
            }
        }
    }

    pub(in crate::game) fn mob_geometry(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<Vertex>,
    ) {
        // The coarse mob bounds pay off only in dense async scenes; otherwise,
        // keep the existing exact mesh-culling path.
        if self.settings.horizon.entity_culling == EntityCullingMode::Asynchronous
            && self
                .async_occlusion_culler
                .as_ref()
                .is_some_and(|culler| culler.camera_is_stationary())
            && self.settings.horizon.entity_culling_precheck != 0
            && self.mobs.len() >= self.settings.horizon.entity_culling_precheck as usize
            && !self.pack_shadow_casters.0
            && mod_api::__private::mods().is_empty()
        {
            self.mob_geometry_inner::<true>(camera, alpha, out);
        } else {
            self.mob_geometry_inner::<false>(camera, alpha, out);
        }
    }

    fn mob_geometry_inner<const PRECHECK: bool>(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<Vertex>,
    ) {
        let atlas = &self.gui.atlas;
        let lookup = |name: &str| atlas.sprites.get(name).map(|r| [r.u0, r.v0, r.u1, r.v1]);
        let scale = self.view_scale();
        self.rendered_mobs.set(0);
        let (skinning, listened) = (self.skinning(), self.mob_render_listened());
        // What custom entity model animations read of the frame.
        horizon_client::mobs::cem_anim::set_frame(horizon_client::mobs::cem_anim::Frame {
            time: self.world.game_time as f64 + alpha,
            day_time: self.world.day_time as f64 + alpha,
            dimension: match self.dim {
                Dim::Overworld | Dim::Extra(_) => 0.0,
                Dim::Nether => -1.0,
                Dim::End => 1.0,
            },
            player: self.player.as_ref().map_or(camera.to_array(), |p| {
                [p.position.x, p.position.y, p.position.z]
            }),
            player_rot: [self.pitch, self.yaw],
            paused: self.pause,
            first_person: self.camera_type.is_first_person(),
            third_person_reversed: self.camera_type
                == horizon_client::camera::CameraType::ThirdPersonFront,
            ..Default::default()
        });
        for mob in &self.mobs {
            // LevelRenderer: the entity the view looks out of is not drawn
            // in first person.
            if self.hides_as_camera(mob.id) {
                continue;
            }
            let is_player = &*mob.kind.id == "minecraft:player";
            let d = glam::DVec3::new(mob.position.x, mob.position.y, mob.position.z) - camera;
            // Entity.shouldRenderAtSqrDistance: 64 times the bounding box's
            // mean edge, scaled by the view scale. RemotePlayer's box counts
            // ten times.
            let (w, h, _) = mob.dims();
            let edge = (w + h + w) / 3.0 * if is_player { 10.0 } else { 1.0 };
            let range = edge * 64.0 * scale;
            if d.length_squared() >= range * range {
                continue;
            }
            let beam = horizon_client::mobs::guardian::beam(mob, alpha as f32);
            // EntityRenderer.shouldRender: the renderer's interpolated culling box,
            // grown by 0.5 (GuardianRenderer also draws while its beam is out).
            if let Some(view) = &self.view_frustum
                && beam.is_none()
            {
                let (lo, hi) = super::entity_culling::cull_box(mob, alpha);
                let (lo, hi) = (
                    (lo - camera - glam::DVec3::splat(0.5)).as_vec3(),
                    (hi - camera + glam::DVec3::splat(0.5)).as_vec3(),
                );
                if !view.intersects(lo, hi) {
                    continue;
                }
            }
            let outline = self.mob_outline(mob);
            // Entity.DATA_SHARED_FLAGS_ID invisible: only the name tag draws (and
            // the outline of a glowing mob, LivingEntityRenderer.getRenderType).
            // A player keeps its armour, elytra and held items
            // (`LivingEntityRenderer.submit` skips only the body).
            let layers_only = mob.invisible && outline.is_none() && is_player;
            if mob.invisible && outline.is_none() && !layers_only {
                continue;
            }
            let culling = if PRECHECK
                && outline.is_none()
                && !is_player
                && mob.kind.id.starts_with("minecraft:")
                && (0..6).all(|slot| mob.extra.slot_stack(slot).is_none())
                && beam.is_none()
            {
                self.entity_cull_decision(mob, alpha)
            } else {
                EntityCullDecision::CheckVertices
            };
            if culling.hides_entity() {
                continue;
            }
            let entity_start = out.len();
            self.mark_motion(entity_start, (1 << 32) | u64::from(mob.id));
            self.mark_entity(entity_start, &mob.kind.id);
            self.rendered_mobs.set(self.rendered_mobs.get() + 1);
            let packed = self.mob_packed_light(mob, alpha);
            // The mods' pre hooks (a player's `render_player_pre`, then `render_living_pre`),
            // before any of the vanilla model's parts (a sulfur cube's swallowed block) are
            // drawn. A cancel skips the rest of the entity's drawing and its post hooks; the
            // frame's later passes (flames, name tag, shadow, leash) leave it out too.
            let who = is_player
                .then(|| self.remote_player_name(mob.id))
                .flatten()
                .map(|name| PlayerRender { name, local: false });
            let pre = self.mod_render_pre(mob, who, listened, alpha as f32, camera, packed, out);
            let Some(hidden) = pre else {
                // What the mods drew in its place is culled as the entity's own would be.
                self.cull_entity_vertices(out, entity_start);
                continue;
            };
            // A mod's renderer for the type (`register_entity_renderers`) draws in place of its
            // model and vanilla layers; the mods' layers and post hooks still follow.
            let modded = self.mod_living_renderer_geometry(mob, alpha as f32, camera, packed, out);
            if mob.species() == Species::SulfurCube && !mob.is_invisible() && !modded {
                self.sulfur_cube_body_geometry(
                    mob,
                    alpha as f32,
                    horizon_client::mobs::render::base_transform(mob, alpha as f32, camera),
                    packed,
                    out,
                );
            }
            // Other players on a server: PlayerRenderer's HumanoidArmorLayer.
            let armor = if is_player {
                self.worn_armor_layers(mob)
            } else {
                Vec::new()
            };
            let body = out.len();
            // The GPU path: an ordinary mob, no layers the CPU path changes after drawing them
            // (a player's, the HDR tiers below, the sulfur cube's shell). A glowing mob's
            // silhouette is posed too (`SkinFrame::outline_since`), unless it is invisible:
            // then only the outline draws.
            let silhouette_on_gpu = outline.is_none() || !mob.invisible;
            let posable = skinning
                && !(is_player || modded)
                && !layers_only
                && silhouette_on_gpu
                && armor.is_empty()
                && !matches!(
                    mob.species(),
                    Species::Blaze | Species::MagmaCube | Species::GlowSquid | Species::SulfurCube
                );
            let (mut skinned, mut parts, added) = (None, None, self.mod_layers(mob));
            if posable {
                let mut models = self.skin_models.borrow_mut();
                let mut frame = self.skin_frame.borrow_mut();
                let mark = frame.mark();
                let mut skin = horizon_client::mobs::skin::Skin::new(&mut models, &mut frame);
                horizon_client::mobs::render::render_skinned(
                    mob,
                    alpha as f32,
                    camera,
                    &lookup,
                    packed,
                    self.res.mob_data(),
                    hidden,
                    (listened || added.is_some()).then_some(&mut parts),
                    out,
                    &mut skin,
                );
                skinned = Some((mark, skin.bounds));
            } else if !modded {
                if layers_only {
                    horizon_client::mobs::render::render_layers(
                        mob,
                        alpha as f32,
                        camera,
                        &lookup,
                        packed,
                        self.res.mob_data(),
                        &armor,
                        (listened || added.is_some()).then_some(&mut parts),
                        out,
                    );
                } else {
                    horizon_client::mobs::render::render_with_hidden(
                        mob,
                        alpha as f32,
                        camera,
                        &lookup,
                        packed,
                        self.res.mob_data(),
                        &armor,
                        hidden,
                        (listened || added.is_some()).then_some(&mut parts),
                        out,
                    );
                }
            }
            // HDR output: mobs that are light shine by their kind, as blocks do (entity.wgsl
            // fs_hdr_mark); a tier's window lifts only their bright texels.
            if let Some(tier) = match mob.species() {
                Species::Blaze => Some(horizon_client::mesher::BRIGHT),
                Species::MagmaCube | Species::GlowSquid => Some(horizon_client::mesher::LOW),
                _ => None,
            } {
                for v in &mut out[body..] {
                    v.layer |= u32::from(tier) << 16;
                }
            }
            // AvatarRenderer.shouldRenderLayers: a spectator draws no layers.
            let layers = horizon_client::mobs::render::layers_shown(mob);
            if is_player && layers {
                // PlayerRenderer's CapeLayer, WingsLayer, ParrotOnShoulderLayer
                // and SpinAttackEffectLayer. Only the cape hides with the body;
                // the others have no `isInvisible` check.
                let chest = mob.extra.slot_id(horizon_items::equipment::CHEST);
                if !layers_only {
                    self.player_cape_geometry(mob, chest, camera, alpha as f32, packed, out);
                }
                if let Some((_, name)) = self
                    .remote_player_names()
                    .into_iter()
                    .find(|(id, _)| *id == mob.id)
                {
                    self.player_ears_geometry(mob, &name, camera, alpha as f32, packed, out);
                }
                self.player_elytra_geometry(mob, chest, camera, alpha as f32, packed, out);
                self.shoulder_parrots_geometry(mob, camera, alpha as f32, packed, out);
                self.spin_attack_geometry(mob, camera, alpha as f32, packed, out);
                self.stuck_in_body_geometry(mob, mob.id as i32, camera, alpha as f32, packed, out);
            }
            // Cosmetics (game/cosmetics.rs), posed on the GPU with the mob's own jobs: culled
            // with it.
            if !(layers_only || modded)
                && horizon_hooks::engine::any()
                && (is_player || &*mob.kind.id == "minecraft:mannequin")
            {
                let mark = self.skin_frame.borrow().mark();
                let base = horizon_client::mobs::render::base_transform(mob, alpha as f32, camera);
                let gpu = skinning;
                if let Some((lo, hi)) =
                    self.player_layers_geometry(mob, base, camera, alpha as f32, packed, gpu, out)
                {
                    skinned = Some(match skinned {
                        Some((first, Some((l, h)))) => (first, Some((l.min(lo), h.max(hi)))),
                        Some((first, None)) => (first, Some((lo, hi))),
                        None => (mark, Some((lo, hi))),
                    });
                }
            }
            // CustomHeadLayer (a pumpkin, skull or head item) included.
            if layers {
                if !modded {
                    // Held items join the GPU path's draws, their silhouette too.
                    let skin = (skinning && silhouette_on_gpu).then_some(&mut skinned);
                    self.mob_block_geometry(mob, camera, alpha, packed, skin, out);
                }
                self.mod_layer_geometry(mob, who, parts, added, alpha as f32, camera, packed, out);
            }
            self.mod_render_post(mob, who, parts, alpha as f32, camera, packed, out);
            if let Some(rgb) = outline {
                self.mark_outline(&out[entity_start..], rgb);
                if let Some(((first, _, _), _)) = skinned {
                    let color = horizon_client::mobs::render::abgr(rgb);
                    self.skin_frame.borrow_mut().outline_since(first, color);
                }
                if mob.invisible {
                    out.truncate(entity_start);
                    continue;
                }
            }
            // Glowing outlines and a guardian's beam remain visible independently
            // of whether the body is hidden behind terrain.
            if outline.is_none() && culling.checks_vertices() && beam.is_none() {
                // What the GPU draws of the mob counts by its bounds' corners.
                self.cull_skinned(out, entity_start, skinned);
            }
            // GuardianRenderer: the laser to the locked target.
            if let Some((time, scale)) = beam
                && !modded
                && let Some(target) = self.beam_target(mob.ai.target, alpha)
                && let Some(uv) = lookup("entity/guardian/guardian_beam")
            {
                let a = alpha;
                let eye = glam::DVec3::new(
                    mob.previous.x + (mob.position.x - mob.previous.x) * a,
                    mob.previous.y + (mob.position.y - mob.previous.y) * a + mob.eye_height(),
                    mob.previous.z + (mob.position.z - mob.previous.z) * a,
                ) - camera;
                horizon_client::mobs::guardian::beam_geometry(
                    eye.as_vec3(),
                    (target - camera).as_vec3(),
                    time,
                    scale,
                    uv,
                    out,
                );
            }
        }
        // What follows isn't a tracked mob: close the last one's span.
        self.mark_motion(out.len(), 0);
        self.elder_apparition_geometry(alpha, out);
        self.spawner_geometry(camera, alpha, out);
        let live = self.creatures.projectiles.iter().map(|a| {
            let at = [
                a.previous.x + (a.position.x - a.previous.x) * alpha,
                a.previous.y + (a.position.y - a.previous.y) * alpha,
                a.previous.z + (a.position.z - a.previous.z) * alpha,
            ];
            (a, at)
        });
        self.projectile_geometry(camera, alpha, live, out);
    }

    pub(super) fn pickup_arrow_geometry(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<Vertex>,
    ) {
        // ItemPickupParticle: a picked-up arrow flying to its collector.
        let taken = self.pickups.live.iter().filter_map(|p| match &p.taken {
            super::pickups::Taken::Arrow(a) => Some((&**a, super::pickups::position(p, alpha))),
            _ => None,
        });
        self.projectile_geometry(camera, alpha, taken, out);
    }

    fn projectile_geometry<'a>(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        projectiles: impl Iterator<Item = (&'a Projectile, [f64; 3])>,
        out: &mut Vec<Vertex>,
    ) {
        let atlas = &self.gui.atlas;
        let (right, up) = thrown_item_axes(self.camera_type, self.yaw, self.pitch);
        // A glowing projectile's outline: what it drew up to the next one.
        let mut glowing = None;
        for (a, at) in projectiles {
            if let Some((start, rgb)) = glowing.take() {
                self.mark_outline(&out[start..], rgb);
            }
            if a.id != 0
                && let Some(rgb) = self.thing_outline(a.id, a.kind.id())
            {
                glowing = Some((out.len(), rgb));
            }
            let p = glam::DVec3::from_array(at) - camera;
            let c = p.as_vec3();
            // EntityRenderer.getPackedLightCoords: the block its light probe
            // (the eye) is in, at the position this frame draws rather than
            // this tick's, and not the one under its feet.
            let light = self.world.light(
                at[0].floor() as i32,
                a.light_probe_y(alpha).floor() as i32,
                at[2].floor() as i32,
            );
            let mut packed = (u32::from(light >> 4) * 16) | ((u32::from(light & 15) * 16) << 8);
            // EntityRenderer.getBlockLightLevel: a shulker bullet and anything
            // on fire are drawn at 15 block light (the fireball renderers'
            // own override); the sky light stays the level's.
            if let Some(block) = a.block_light_override() {
                packed = (packed & 0xFF) | (u32::from(block) * 16) << 8;
            }
            if a.is_arrow() {
                let Some(model) = self.arrow_model.as_ref() else {
                    continue;
                };
                // SpectralArrowRenderer, and TippableArrowRenderer's `getColor() > 0`.
                let texture = if a.is_spectral_arrow() {
                    "entity/projectiles/arrow_spectral"
                } else if a.arrow.as_ref().is_some_and(|s| s.effect_color > 0) {
                    "entity/projectiles/arrow_tipped"
                } else {
                    "entity/projectiles/arrow"
                };
                let Some(r) = atlas.sprites.get(texture).copied() else {
                    continue;
                };
                // ArrowRenderer: `Mth.lerp(partialTick, state.yRotO, state.yRot)`
                // and the same for the pitch, so a turning arrow is drawn
                // between the two ticks it turned over.
                let alpha32 = alpha as f32;
                let yaw = a.previous_yaw + alpha32 * (a.yaw - a.previous_yaw);
                let pitch = a.previous_pitch + alpha32 * (a.pitch - a.previous_pitch);
                let m = glam::Mat4::from_translation(c)
                    * glam::Mat4::from_rotation_y((yaw - 90.0).to_radians())
                    * glam::Mat4::from_rotation_z(pitch.to_radians());
                // ArrowRenderer: `state.shake = shakeTime - partialTicks`; ArrowModel.setupAnim
                // rolls the root while the shake timer runs.
                let shake = a
                    .arrow
                    .as_ref()
                    .map_or(0.0, |s| s.shake as f32 - alpha as f32);
                let roll = if shake > 0.0 {
                    -(shake * 3.0).msin() * shake * (std::f32::consts::PI / 180.0)
                } else {
                    0.0
                };
                let pose_for = |part: &horizon_client::mobs::Part| {
                    let mut pose = part.pose;
                    if std::ptr::eq(part, model) {
                        pose[5] += roll;
                    }
                    pose
                };
                horizon_client::mobs::render_model(
                    model,
                    m,
                    &pose_for,
                    [r.u0, r.v0, r.u1, r.v1],
                    packed,
                    0,
                    out,
                );
                if a.arrow.as_ref().is_some_and(|s| s.on_fire()) {
                    flame_geometry(atlas, c, 0.5, 0.5, self.yaw, light, out);
                }
                continue;
            }
            // ThrownTridentRenderer.
            if matches!(a.kind, ProjectileKind::Trident) {
                let Some(model) = self.trident_model.as_ref() else {
                    continue;
                };
                let Some(r) = atlas.sprites.get("entity/trident/trident").copied() else {
                    continue;
                };
                let m = glam::Mat4::from_translation(c)
                    * glam::Mat4::from_rotation_y((a.yaw - 90.0).to_radians())
                    * glam::Mat4::from_rotation_z((a.pitch + 90.0).to_radians());
                let poses = horizon_client::mobs::anim::Poses::new(model);
                horizon_client::mobs::render::draw(
                    model,
                    m,
                    &poses,
                    [r.u0, r.v0, r.u1, r.v1],
                    0xFFFF_FFFF,
                    packed,
                    0,
                    out,
                );
                continue;
            }
            // WindChargeRenderer.
            if matches!(
                a.kind,
                ProjectileKind::WindCharge | ProjectileKind::BreezeWindCharge
            ) {
                self.wind_charge_geometry(a, c, packed, alpha as f32, out);
                continue;
            }
            // ShulkerBulletRenderer: the tumbling bullet model, then a larger
            // translucent copy of it (colour 0x26FFFFFF); always 15 block light.
            if matches!(a.kind, ProjectileKind::ShulkerBullet { .. }) {
                let model = &self.entity_models.shulker_bullet;
                if let (Some(model), Some(r)) =
                    (model, atlas.sprites.get("entity/shulker/spark").copied())
                {
                    use horizon_core::geometry::trig;
                    let tc = a.age as f32 + alpha as f32;
                    let mut poses = horizon_client::mobs::anim::Poses::new(model);
                    poses.yrot("main", a.yaw.to_radians());
                    poses.xrot("main", a.pitch.to_radians());
                    let light = (packed & 0xFF) | ((15 * 16) << 8);
                    let base = glam::Mat4::from_translation(c)
                        * glam::Mat4::from_translation(glam::Vec3::new(0.0, 0.15, 0.0))
                        * glam::Mat4::from_rotation_y(
                            (trig::sin(f64::from(tc * 0.1)) * 180.0).to_radians(),
                        )
                        * glam::Mat4::from_rotation_x(
                            (trig::cos(f64::from(tc * 0.1)) * 180.0).to_radians(),
                        )
                        * glam::Mat4::from_rotation_z(
                            (trig::sin(f64::from(tc * 0.15)) * 360.0).to_radians(),
                        )
                        * glam::Mat4::from_scale(glam::Vec3::new(-0.5, -0.5, 0.5));
                    let uv = [r.u0, r.v0, r.u1, r.v1];
                    horizon_client::mobs::render::draw(
                        model,
                        base,
                        &poses,
                        uv,
                        0xFFFF_FFFF,
                        light,
                        0,
                        out,
                    );
                    // The translucent shell blends over the opaque core
                    // (`RenderTypes.entityTranslucent`).
                    let shell = out.len();
                    horizon_client::mobs::render::draw(
                        model,
                        base * glam::Mat4::from_scale(glam::Vec3::splat(1.5)),
                        &poses,
                        uv,
                        0x26FF_FFFF,
                        light,
                        0,
                        out,
                    );
                    horizon_client::gfx::mark_oit(&mut out[shell..]);
                }
                continue;
            }
            // WitherSkullRenderer: the skull model facing its flight.
            if let Some(dangerous) = super::wither::skull_kind(a) {
                let model = &self.entity_models.wither_skull;
                let tex = if dangerous {
                    "entity/wither/wither_invulnerable"
                } else {
                    "entity/wither/wither"
                };
                if let (Some(model), Some(r)) = (model, atlas.sprites.get(tex).copied()) {
                    let mut poses = horizon_client::mobs::anim::Poses::new(model);
                    poses.yrot("head", a.yaw.to_radians());
                    poses.xrot("head", a.pitch.to_radians());
                    let base = glam::Mat4::from_translation(c)
                        * glam::Mat4::from_scale(glam::Vec3::new(-1.0, -1.0, 1.0));
                    horizon_client::mobs::render::draw(
                        model,
                        base,
                        &poses,
                        [r.u0, r.v0, r.u1, r.v1],
                        0xFFFF_FFFF,
                        packed,
                        0,
                        out,
                    );
                }
                continue;
            }
            if self.thrown_item_geometry(a, alpha, at, c, packed, out) {
                continue;
            }
            // `Entity.displayFireAnimation`: a thrown item that flew through a
            // fire block burns (`ThrowableProjectile.tick`'s
            // `applyEffectsFromBlocks`), and ThrownItemRenderer draws the flame
            // over its box as any other entity renderer does.
            if a.on_fire() {
                let size = a.kind.size() as f32;
                flame_geometry(atlas, c, size, size, self.yaw, light, out);
            }
            // `ThrownItemRenderer` for a fireball: the item `Fireball.getItem`
            // returns (a fire charge until `setItem` gave it another), its model
            // at the renderer's scale for the kind. `packed` is already 15, the
            // renderer's full bright.
            if let Some(stack) = a.item.as_ref() {
                let owner = super::item_draw::owner_at(at, 0.0, a.kind.id());
                let state = self.resolve_item(stack, DisplayContext::Ground, Some(&owner), 0);
                let pose = glam::Mat4::from_translation(c)
                    * glam::Mat4::from_quat(self.camera_orientation())
                    * glam::Mat4::from_scale(glam::Vec3::splat(a.sprite_scale()));
                self.submit_item_world(&state, pose, packed, out);
                continue;
            }
            let Some(name) = a.item_sprite() else {
                continue;
            };
            let Some(r) = atlas.sprites.get(name).copied() else {
                continue;
            };
            let lift = is_item_billboard(&a.kind);
            let quad = thrown_item_quad(c, right, up, a.sprite_scale(), lift);
            for (v, uv) in quad
                .iter()
                .zip([[r.u0, r.v1], [r.u1, r.v1], [r.u1, r.v0], [r.u0, r.v0]])
            {
                out.push(Vertex {
                    normal: [0; 4],
                    pos: v.to_array(),
                    uv,
                    layer: 0,
                    color: 0xFFFF_FFFF,
                    light: packed,

                    glint_uv: [0.0, 0.0],
                });
            }
        }
        if let Some((start, rgb)) = glowing {
            self.mark_outline(&out[start..], rgb);
        }
    }
}
/// `ThrownItemRenderer.submit` turns the item by `CameraRenderState.orientation`
/// (`Camera.setRotation`): the billboard's right and up axes are the camera's.
/// They stay continuous at pitch ±90, and in third-person front view they are
/// the turned-round camera's, not the player's.
fn thrown_item_axes(
    camera_type: horizon_client::camera::CameraType,
    yaw: f32,
    pitch: f32,
) -> (glam::Vec3, glam::Vec3) {
    let orientation = super::camera_rotation(camera_type, yaw, pitch);
    (orientation * glam::Vec3::X, orientation * glam::Vec3::Y)
}
/// The four corners of the item billboard, in the order its UVs are laid out.
/// `ThrownItemRenderer` only scales and billboards it; the lift is
/// `item/generated`'s ground display transform, `translation [0, 2, 0]` of 16
/// units = 0.125 blocks, which is `0.125 * scale` == `0.5 * s`.
fn thrown_item_quad(
    at: glam::Vec3,
    right: glam::Vec3,
    up: glam::Vec3,
    scale: f32,
    lift: bool,
) -> [glam::Vec3; 4] {
    let s = 0.25 * scale;
    let rise = if lift { 0.5 * s } else { 0.0 };
    let centre = at + up * rise;
    [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
        .map(|k| centre + right * k.0 * s + up * k.1 * s)
}
/// Whether vanilla draws the kind as `ThrownItemRenderer`'s item billboard, the
/// only ones that take that lift. Llama spit shares this quad but
/// `LlamaSpitRenderer` is a model, with its own offset, and a fireball's item
/// is drawn as the item model that carries the lift itself.
fn is_item_billboard(kind: &ProjectileKind) -> bool {
    matches!(
        kind,
        ProjectileKind::Snowball | ProjectileKind::Egg | ProjectileKind::EnderPearl
    )
}
/// `Bucketable.getBucketItemStack` and `getPickupSound` of a mob that a water
/// bucket picks up.
fn bucketable(species: Species) -> Option<(&'static str, &'static str)> {
    let fish = "minecraft:item.bucket.fill_fish";
    Some(match species {
        Species::Cod => ("minecraft:cod_bucket", fish),
        Species::Salmon => ("minecraft:salmon_bucket", fish),
        Species::Pufferfish => ("minecraft:pufferfish_bucket", fish),
        Species::TropicalFish => ("minecraft:tropical_fish_bucket", fish),
        Species::Tadpole => (
            "minecraft:tadpole_bucket",
            "minecraft:item.bucket.fill_tadpole",
        ),
        Species::Axolotl => (
            "minecraft:axolotl_bucket",
            "minecraft:item.bucket.fill_axolotl",
        ),
        _ => return None,
    })
}
pub(super) use horizon_client::health::{damage_type, melee, source_damage_type};
/// A Horizon item stack as a loot stack (id, count, damage, enchantments,
/// and its custom data as SNBT for `minecraft:custom_data` predicates).
fn to_loot(stack: &ItemStack) -> horizon_world::loot::LootStack {
    let components = stack
        .custom_data()
        .map(|data| {
            (
                "minecraft:custom_data".to_owned(),
                Some(horizon_core::nbt::Tag::Compound(data.clone())),
            )
        })
        .into_iter()
        .collect();
    horizon_world::loot::LootStack {
        item: stack.item.to_string(),
        count: stack.count as i32,
        damage: stack.damage as i32,
        enchantments: stack
            .all_enchantments()
            .iter()
            .map(|(e, l)| (e.to_string(), *l as i32))
            .collect(),
        potion: None,
        map: false,
        exploration: None,
        ominous_amplifier: None,
        stew_effects: Vec::new(),
        names: Vec::new(),
        components,
    }
}
/// A generated loot stack as a Horizon item stack.
pub(super) fn from_loot(stack: &horizon_world::loot::LootStack) -> ItemStack {
    let mut item = ItemStack {
        item: Arc::from(stack.item.as_str()),
        count: stack.count.max(1) as u32,
        damage: stack.damage.max(0) as u32,
        extra: None,
    };
    for (id, level) in &stack.enchantments {
        item.enchant(id, (*level).max(0) as u32);
    }
    if let Some(potion) = &stack.potion {
        item = item.with_potion(potion);
    }
    if let Some(amplifier) = stack.ominous_amplifier.filter(|a| *a > 0) {
        item.data_mut().ominous_amplifier = Some(amplifier as u8);
    }
    for (id, text) in &stack.names {
        item.data_mut().set_loot_name(id, text);
    }
    for (id, value) in &stack.components {
        item.data_mut().set_loot_component(id, value);
    }
    item
}
impl Game<'_> {
    /// The rest of `NetherPortalBlock.randomTick` after a won roll: with a
    /// player close enough for spawning, a zombified piglin spawns on the block
    /// under the portal column when it is a valid spawn.
    pub(super) fn portal_spawn(&mut self, pos: [i32; 3]) {
        // ChunkMap.anyPlayerCloseEnoughForSpawning: a non-spectator within 128
        // blocks of the chunk centre.
        let centre = [
            f64::from((pos[0] >> 4) * 16 + 8),
            f64::from((pos[2] >> 4) * 16 + 8),
        ];
        let close = self.level_players().iter().any(|p| {
            let (dx, dz) = (p.position.x - centre[0], p.position.z - centre[1]);
            !p.spectator && dx * dx + dz * dz < 16384.0
        });
        if !close {
            return;
        }
        let mut y = pos[1];
        while &*self
            .res
            .facts()
            .get(self.world.block(pos[0], y, pos[2]))
            .name
            == "minecraft:nether_portal"
        {
            y -= 1;
        }
        // isValidSpawn: a sturdy top face (portal frames emit no light).
        let below = self.res.facts().get(self.world.block(pos[0], y, pos[2]));
        if below.sturdy_full & (1 << 1) == 0 {
            return;
        }
        let at = [
            f64::from(pos[0]) + 0.5,
            f64::from(y + 1),
            f64::from(pos[2]) + 0.5,
        ];
        // setPortalCooldown: the new piglin does not travel at once.
        if let Some(id) = self.spawn_mob(
            "minecraft:zombified_piglin",
            at,
            false,
            SpawnSite::Finalizing(horizon_client::mobs::spawn_rules::Reason::Structure),
        ) {
            self.set_portal_cooldown(id);
        }
    }
}
/// The instance a mob effect gives the player; `None` for a creative or
/// spectator player (`immune`) unless it is ambient (the nautilus's Breath).
fn mob_effect_instance(
    effect: &'static str,
    ticks: i32,
    amplifier: i32,
    ambient: bool,
    immune: bool,
) -> Option<horizon_client::effects::EffectInstance> {
    if immune && !ambient {
        return None;
    }
    let id = effect.strip_prefix("minecraft:").unwrap_or(effect);
    let mut instance = horizon_client::effects::EffectInstance::new(id, ticks, amplifier);
    instance.ambient = ambient;
    Some(instance)
}

include!("mob_query_hooks.rs");

impl Game<'_> {
    /// `EventHooks.canEntityGrief(level, entity)`: the `mob_griefing` rule,
    /// which a mod may change for `entity` (`EntityMobGriefingEvent`). No
    /// entity: the rule alone, as NeoForge.
    pub(super) fn can_entity_grief(&self, entity: Option<u32>) -> bool {
        let rule = self
            .ui
            .rules
            .bool_at(horizon_client::gamerules::vanilla::MOB_GRIEFING);
        entity.map_or(rule, |entity| self.entity_grief(entity, rule))
    }
    /// Whether a large fireball's blast lights fire (`LargeFireball.onHit`):
    /// [`Self::can_entity_grief`] for its owner, as `getOwner()` gives it. That
    /// is the mob that shot it, or the player who owns it (the guest on `guest`,
    /// else the player in place). An owner that is gone, or none, reads the rule.
    pub(in crate::game) fn large_fireball_fire(
        &self,
        owner: Option<Who>,
        guest: Option<u32>,
    ) -> bool {
        self.can_entity_grief(match owner {
            Some(Who::Mob(id)) => Some(id),
            Some(Who::Player) => self
                .player_entity_id_by_connection(guest)
                .map(|id| id as u32),
            None => None,
        })
    }
    /// Whether a mod may answer `can_entity_grief` differently from the rule:
    /// the sites that skip their whole work while griefing is off skip it only
    /// when no mod listens.
    pub(super) fn grief_hooked(&self) -> bool {
        self.native_authoritative()
            && mod_api::__private::listened(mod_api::HookId::EntityMobGriefing)
    }
    /// [`Self::can_entity_grief`] for `entity`, with the rule already read.
    pub(super) fn entity_grief(&self, entity: u32, rule: bool) -> bool {
        if !self.grief_hooked() {
            return rule;
        }
        let Some(epoch) = self.native_lifecycle.epoch() else {
            return rule;
        };
        let mut can_grief = rule;
        self.native_entity_mob_griefing(mod_api::EntityRef::new(epoch, entity), &mut can_grief);
        can_grief
    }
}

#[cfg(test)]
#[path = "mob_griefing_tests.rs"]
mod mob_griefing_tests;

#[cfg(test)]
mod living_visibility_tests {
    use super::super::native_entity_refs::tests::with_game;
    use super::super::native_item_places::tests::{guest, open_to_lan};
    use std::sync::{Arc, Mutex};

    /// Halves the player's visibility, and logs what it was asked.
    struct Halve(Arc<Mutex<Vec<(u32, Option<u32>, f64)>>>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Halve {
        fn living_visibility(
            &self,
            _: &dyn mod_api::Host,
            event: &mut mod_api::LivingVisibilityEvent,
        ) {
            let looking = event.looking.map(|e| e.id());
            self.0
                .lock()
                .unwrap()
                .push((event.entity.entity().id(), looking, event.visibility));
            event.visibility *= 0.5;
        }
    }

    /// `LivingVisibilityEvent` for a guest a mob targets: the mod gets the
    /// guest, the mob and vanilla's percent, and its answer is the percent.
    #[test]
    #[ignore = "needs prepared resources"]
    fn a_mod_halves_a_guests_visibility() {
        let heard = Arc::default();
        let mods = vec![mod_api::ModEntry::new(
            "halve",
            Box::new(Halve(Arc::clone(&heard))),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                open_to_lan(game);
                let mut seen = guest(5);
                seen.loaded = true;
                game.lan.as_mut().unwrap().hub.guests.push(seen);
                let view = game.lan_guest_views()[0].1.clone();
                let world = game.mob_world_base(&[]);
                assert!(world.mob_hooks.is_some(), "asked only while a mod listens");
                assert_eq!(world.player_visibility(&view, 900, 0.8), 0.4);
                assert_eq!(*heard.lock().unwrap(), [(24, Some(900), 0.8)]);
            });
        });
    }
}

#[cfg(test)]
mod enchanted_entity_loot_tests {
    use super::super::combat::PlayerCredit;
    use super::super::native_entity_refs::tests::with_game;
    use super::super::native_item_places::tests::{guest, open_to_lan};
    use horizon_client::mobs::{Damage, DamageKind, Mob, Who};
    use horizon_client::motion::PlayerMotion;
    use horizon_core::geometry::Vec3;
    use horizon_core::random::{LegacyRandom, RandomSource};
    use mod_api::ServerHost;
    use std::sync::{Arc, Mutex};

    /// What the mod read through its handles, of one question.
    #[derive(Debug, PartialEq)]
    struct Heard {
        entity: Option<String>,
        attacker: Option<String>,
        damage_type: String,
        enchantment: String,
        level: u32,
    }
    /// Answers every question with level `self.1`.
    struct Answer(Arc<Mutex<Vec<Heard>>>, u32);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Answer {
        fn enchanted_entity_loot(
            &self,
            host: &mut dyn ServerHost,
            event: &mut mod_api::EnchantedEntityLootEvent<'_>,
        ) {
            self.0.lock().unwrap().push(Heard {
                entity: event
                    .entity
                    .entity()
                    .kind(&*host)
                    .map(|kind| kind.id(&*host)),
                attacker: event
                    .attacker
                    .and_then(|a| a.kind(&*host))
                    .map(|kind| kind.id(&*host)),
                damage_type: event.source.damage_type.to_owned(),
                enchantment: event.enchantment.to_string(),
                level: event.level,
            });
            event.level = self.1;
        }
    }

    fn listening(seen: &Arc<Mutex<Vec<Heard>>>, level: u32) -> mod_api::Mods {
        let answer = Box::new(Answer(Arc::clone(seen), level));
        vec![mod_api::ModEntry::new("enchanted-loot", answer)].into()
    }
    fn zombie(game: &mut super::Game<'_>, damage: Damage) -> Mob {
        game.next_entity += 1;
        let kind = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        let mut mob = Mob::new(game.next_entity, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
        mob.health = 0.0;
        mob.last_damage = Some((damage, 0));
        game.mobs.push(mob.clone());
        mob
    }
    fn dropped(game: &super::Game<'_>, item: &str) -> u32 {
        let drops = game.dropped.iter().filter(|d| &*d.stack.item == item);
        drops.map(|d| d.stack.count).sum()
    }
    fn heard(entity: &str, attacker: &str, damage_type: &str, level: u32) -> Heard {
        Heard {
            entity: Some(entity.into()),
            attacker: Some(attacker.into()),
            damage_type: damage_type.into(),
            enchantment: "minecraft:looting".into(),
            level,
        }
    }

    /// `EnchantedEntityLootEvent` through a whole kill: the zombie's table
    /// reads Looting for its flesh's `enchanted_count_increase` and its rare
    /// drops' `random_chance_with_enchanted_bonus`. The mod gets the dying
    /// zombie and the attacker as live handles, the attacker's own level, and
    /// the level it answers decides the drops.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_kills_loot_uses_the_level_a_mod_answers() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![mod_api::ModEntry::new(
            "enchanted-loot",
            Box::new(Answer(Arc::clone(&seen), 1000)),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                game.load_server_loot();
                open_to_lan(game);
                game.lan.as_mut().unwrap().hub.guests.push(guest(5));
                // The mobs' ids, clear of the guest's.
                game.next_entity = 1000;
                game.as_guest(5, |g| {
                    let mut sword =
                        horizon_client::items::ItemStack::new("minecraft:iron_sword", 1);
                    sword.enchant("minecraft:looting", 2);
                    g.inventory[g.selected] = Some(sword);
                })
                .unwrap();
                let by_player = Damage {
                    kind: DamageKind::PlayerAttack,
                    attacker: Some(Who::Player),
                    from: Some([0.0; 3]),
                    attacker_species: None,
                };
                let mut victim = zombie(game, by_player);
                victim.ai.last_hurt_by_player = 100;
                game.last_hurt_by_player
                    .insert(victim.id, PlayerCredit(127));
                game.mob_died(&victim);
                let looted = |level| {
                    heard(
                        "minecraft:zombie",
                        "minecraft:player",
                        "minecraft:player_attack",
                        level,
                    )
                };
                assert_eq!(*seen.lock().unwrap(), [looted(2), looted(2)]);
                // A flesh count of at most 2 grew by the level's share, and
                // a chance of 0.035 + 0.01 a level (above 1 at 1000) always
                // holds, so one of the three rare drops came.
                assert!(dropped(game, "minecraft:rotten_flesh") > 2);
                let rare = [
                    "minecraft:iron_ingot",
                    "minecraft:carrot",
                    "minecraft:potato",
                ];
                let rare: u32 = rare.iter().map(|item| dropped(game, item)).sum();
                assert_eq!(rare, 1);

                // In the guest's own turn, their record is out of the hub
                // and the handle still names them.
                seen.lock().unwrap().clear();
                let mut victim = zombie(game, by_player);
                victim.ai.last_hurt_by_player = 100;
                game.last_hurt_by_player
                    .insert(victim.id, PlayerCredit(127));
                game.as_guest(5, |g| g.mob_died(&victim)).unwrap();
                assert_eq!(*seen.lock().unwrap(), [looted(2), looted(2)]);

                // Another mob's blow: no player, so its rare drops read no
                // kill, and the flesh is still asked, from a level of 0.
                seen.lock().unwrap().clear();
                let skeleton = zombie(game, Damage::mob(0, [0.0; 3]));
                // Keep the admitted row's residency and attachment lifetime.
                game.mobs.last_mut().unwrap().health = 20.0;
                let victim = zombie(game, Damage::mob(skeleton.id, [0.0; 3]));
                game.mob_died(&victim);
                assert_eq!(
                    *seen.lock().unwrap(),
                    [heard(
                        "minecraft:zombie",
                        "minecraft:zombie",
                        "minecraft:mob_attack",
                        0
                    )]
                );
            });
        });
    }

    /// The host's own kill names the host: its id, not the 0 of no attacker.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn the_hosts_kill_hands_the_host_as_the_attacker() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![mod_api::ModEntry::new(
            "enchanted-loot",
            Box::new(Answer(Arc::clone(&seen), 0)),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                game.load_server_loot();
                game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
                game.player.as_mut().unwrap().entity_id = 9000;
                // The host's own turn: the uuid is acting, no guest connection.
                crate::game::native_entity_refs::tests::bind_local_player(game, 9000, 99);
                game.lan_acting_uuid = Some(99);
                game.lan_acting_entity = Some(9000);
                let by_player = Damage {
                    kind: DamageKind::PlayerAttack,
                    attacker: Some(Who::Player),
                    from: Some([0.0; 3]),
                    attacker_species: None,
                };
                let mut victim = zombie(game, by_player);
                victim.ai.last_hurt_by_player = 100;
                game.last_hurt_by_player.insert(victim.id, PlayerCredit(99));
                game.mob_died(&victim);
                let looted = || {
                    heard(
                        "minecraft:zombie",
                        "minecraft:player",
                        "minecraft:player_attack",
                        0,
                    )
                };
                assert_eq!(*seen.lock().unwrap(), [looted(), looted()]);
            });
        });
    }

    /// No attacker at all: `getAttacker` is `@Nullable`, and the read is
    /// asked from a level of 0.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_death_with_no_attacker_is_asked_at_level_0() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mods = listening(&seen, 1000);
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                game.load_server_loot();
                let victim = zombie(game, Damage::fall());
                game.mob_died(&victim);
                assert_eq!(
                    *seen.lock().unwrap(),
                    [Heard {
                        attacker: None,
                        ..heard("minecraft:zombie", "", "minecraft:fall", 0)
                    }]
                );
                assert!(dropped(game, "minecraft:rotten_flesh") > 2);
            });
        });
    }

    /// `LootCommand.dropKillLoot`: nothing dies, the damage is `magic()`, whose
    /// source names nobody, and the attacker is the command's source. Both
    /// readers ask, with the source's level, and the answer decides the drops.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn loot_kill_asks_with_the_commands_source_as_the_attacker() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mods = listening(&seen, 1000);
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                game.load_server_loot();
                game.set_allow_commands(true);
                open_to_lan(game);
                game.lan.as_mut().unwrap().hub.guests.push(guest(5));
                game.next_entity = 1000;
                let victim = zombie(game, Damage::mob(0, [0.0; 3]));
                game.mobs.last_mut().unwrap().health = 20.0;
                let looted = |level| {
                    heard(
                        "minecraft:zombie",
                        "minecraft:player",
                        "minecraft:magic",
                        level,
                    )
                };

                game.as_guest(5, |g| {
                    let mut sword =
                        horizon_client::items::ItemStack::new("minecraft:iron_sword", 1);
                    sword.enchant("minecraft:looting", 2);
                    g.inventory[g.selected] = Some(sword);
                    g.ui.registries.permission = 4;
                    assert!(
                        g.run_command("loot spawn 0 64 0 kill @e[type=minecraft:zombie,limit=1]")
                    );
                })
                .unwrap();
                assert_eq!(*seen.lock().unwrap(), [looted(2), looted(2)]);
                assert!(
                    game.mobs
                        .iter()
                        .any(|m| m.id == victim.id && m.health > 0.0)
                );
                assert!(dropped(game, "minecraft:rotten_flesh") > 2);
                let rare = [
                    "minecraft:iron_ingot",
                    "minecraft:carrot",
                    "minecraft:potato",
                ];
                let rare: u32 = rare.iter().map(|item| dropped(game, item)).sum();
                assert_eq!(rare, 1);
            });
        });
    }

    /// A source that isn't living is the attacker all the same, read from a
    /// level of 0; a source with no entity is no attacker. The zombie's rare
    /// drops read no kill then, so only the flesh asks.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn loot_kill_asks_with_a_non_living_source_and_with_none() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mods = listening(&seen, 3);
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                game.load_server_loot();
                game.set_allow_commands(true);
                game.ui.registries.permission = 4;
                game.next_entity = 1000;
                zombie(game, Damage::mob(0, [0.0; 3]));
                game.mobs.last_mut().unwrap().health = 20.0;
                let kill = "loot spawn 0 64 0 kill @e[type=minecraft:zombie,limit=1]";

                let source = game.player_source();
                game.command_source = Some(super::super::commands::execute::Source {
                    entity: None,
                    ..source
                });
                assert!(game.run_command(kill));
                assert_eq!(
                    *seen.lock().unwrap(),
                    [Heard {
                        attacker: None,
                        ..heard("minecraft:zombie", "", "minecraft:magic", 0)
                    }]
                );
                seen.lock().unwrap().clear();

                game.command_source = None;
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
                let stick = horizon_client::items::ItemStack::new("minecraft:stick", 1);
                game.spawn_item(stick, [0.5, 64.0, 0.5], [0.0; 3], 0);
                let item = game.dropped.iter().last().unwrap().id;
                let item = super::super::commands::Target::Item(item);
                game.command_source = Some(super::super::commands::execute::Source {
                    entity: Some(item),
                    ..source
                });
                assert!(game.run_command(kill));
                game.command_source = None;
                assert_eq!(
                    *seen.lock().unwrap(),
                    [heard(
                        "minecraft:zombie",
                        "minecraft:item",
                        "minecraft:magic",
                        0
                    )]
                );
            });
        });
    }

    /// A draw the listener makes from the level's random during `/loot ... kill`
    /// stays made: the roll's sequence is its own, and the level random moves.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_draw_made_during_loot_kill_stays_in_the_level_random() {
        struct Drawing;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Drawing {
            fn enchanted_entity_loot(
                &self,
                host: &mut dyn ServerHost,
                _: &mut mod_api::EnchantedEntityLootEvent<'_>,
            ) {
                let level = host.level("minecraft:overworld").unwrap();
                level.with_random(host, |random| random.next_int()).unwrap();
            }
        }
        let mods = vec![mod_api::ModEntry::new("enchanted-loot", Box::new(Drawing))].into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                game.load_server_loot();
                game.set_allow_commands(true);
                game.ui.registries.permission = 4;
                game.next_entity = 1000;
                zombie(game, Damage::mob(0, [0.0; 3]));
                game.mobs.last_mut().unwrap().health = 20.0;
                game.world.random.set_seed(7);
                let source = game.player_source();
                game.command_source = Some(super::super::commands::execute::Source {
                    entity: None,
                    ..source
                });
                assert!(
                    game.run_command("loot spawn 0 64 0 kill @e[type=minecraft:zombie,limit=1]")
                );
                game.command_source = None;
                let mut expected = LegacyRandom::new(7);
                expected.next_int();
                assert_eq!(game.world.random.next_long(), expected.next_long());
            });
        });
    }

    /// A listener that leaves the level as it came changes nothing: the same
    /// deaths drop the same as with no mod, one question each.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn an_answer_of_the_same_level_leaves_the_drops_alone() {
        let deaths = |mods: mod_api::Mods| {
            let mut counts = Vec::new();
            mod_platform::testing::with_mods(mods, || {
                with_game(|game| {
                    game.load_server_loot();
                    for _ in 0..8 {
                        let victim = zombie(game, Damage::mob(0, [0.0; 3]));
                        game.mob_died(&victim);
                        counts.push(dropped(game, "minecraft:rotten_flesh"));
                    }
                });
            });
            counts
        };
        let seen = Arc::new(Mutex::new(Vec::new()));
        let listening = vec![mod_api::ModEntry::new(
            "enchanted-loot",
            Box::new(Answer(Arc::clone(&seen), 0)),
        )]
        .into();
        assert_eq!(deaths(listening), deaths(Vec::new().into()));
        assert_eq!(seen.lock().unwrap().len(), 8);
    }
}

#[cfg(test)]
mod enderman_anger_tests {
    use super::super::native_entity_refs::tests::with_game;
    use super::super::native_item_places::tests::{guest, open_to_lan};
    use horizon_client::chunk::Chunk;
    use horizon_client::mobs::{Mob, enderman::stared_at};
    use horizon_core::geometry::Vec3;
    use std::sync::{Arc, Mutex};

    /// Keeps enderman `self.0` calm, and logs who stared at whom.
    struct Calm(u32, Arc<Mutex<Vec<(u32, u128)>>>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Calm {
        fn enderman_anger(
            &self,
            _: &dyn mod_api::Host,
            event: &mut mod_api::EndermanAngerEvent,
        ) -> mod_api::Flow {
            let asked = (event.enderman.id(), event.player.uuid());
            self.1.lock().unwrap().push(asked);
            if event.enderman.id() == self.0 {
                mod_api::Flow::Cancel
            } else {
                mod_api::Flow::Continue
            }
        }
    }

    /// `EnderManAngerEvent`: a guest stares straight at two endermen; the
    /// mod keeps one calm, the other still counts the stare.
    #[test]
    #[ignore = "needs prepared resources"]
    fn a_mod_keeps_one_stared_at_enderman_calm() {
        let heard = Arc::default();
        let (calm, angry) = (900, 901);
        let mods = vec![mod_api::ModEntry::new(
            "calm",
            Box::new(Calm(calm, Arc::clone(&heard))),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                open_to_lan(game);
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let mut staring = guest(5);
                staring.loaded = true;
                staring.position = [8.0, 64.0, 2.0];
                game.lan.as_mut().unwrap().hub.guests.push(staring);
                let views = game.lan_guest_views();
                let mut view = views[0].1.clone();
                let kind = game
                    .res
                    .mob_types()
                    .get("minecraft:enderman")
                    .unwrap()
                    .clone();
                let endermen = [calm, angry]
                    .map(|id| Mob::new(id, kind.clone(), Vec3::new(8.0, 64.0, 10.0), 0.0));
                let eye = endermen[0].eye_y();
                view.position = Vec3::new(8.0, 64.0, 2.0);
                view.eye = [8.0, eye, 2.0];
                view.look = [0.0, 0.0, 1.0];
                view.head = None;
                let mut world = game.mob_world_base(&[]);
                assert!(world.mob_hooks.is_some(), "asked only while a mod listens");
                world.player = Some(view);
                assert!(!stared_at(&endermen[0], &world), "the mod keeps it calm");
                assert!(stared_at(&endermen[1], &world));
                assert_eq!(*heard.lock().unwrap(), [(calm, 127), (angry, 127)]);
            });
        });
    }
}

#[cfg(test)]
mod local_sound_event_tests {
    use crate::game::{Audio, Game, Options, Progress, Settings, load_resources};
    use horizon_client::mobs::MobEvent;

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn local_mob_events_play_without_entering_the_shared_capture() {
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
        options.game_dir =
            Some(std::env::temp_dir().join(format!("horizon-h069-unit-{}", std::process::id())));
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
        game.audio.capture = Some(Vec::new());
        game.handle_mob_events(vec![
            MobEvent::Sound("entity.cow.hurt", [1.0, 100.0, 2.0], 0.5, 1.0),
            MobEvent::LocalSound("entity.breeze.whirl", [3.0, 100.0, 4.0], 0.8, 0.7),
            MobEvent::Sound("entity.zombie.hurt", [5.0, 100.0, 6.0], 1.0, 0.9),
        ])
        .unwrap();
        // A positive LocalSound must still take the playback path. An absent
        // producer cannot satisfy this actual Game adapter regression.
        assert_eq!(
            game.audio.heard.get("minecraft:entity.breeze.whirl"),
            Some(&1)
        );
        assert_eq!(game.audio.heard.get("minecraft:entity.cow.hurt"), Some(&1));
        assert_eq!(
            game.audio.heard.get("minecraft:entity.zombie.hurt"),
            Some(&1)
        );
        let shared = game.audio.capture.as_ref().unwrap();
        assert_eq!(shared.len(), 2, "LocalSound must not become a shared sound");
        assert_eq!(shared[0].event, "minecraft:entity.cow.hurt");
        assert_eq!(shared[1].event, "minecraft:entity.zombie.hurt");
        assert_eq!(
            (
                shared[0].position,
                shared[0].volume,
                shared[0].pitch,
                shared[0].source
            ),
            ([1.0, 100.0, 2.0], 0.5, 1.0, 6)
        );
        assert_eq!(
            (
                shared[1].position,
                shared[1].volume,
                shared[1].pitch,
                shared[1].source
            ),
            ([5.0, 100.0, 6.0], 1.0, 0.9, 5)
        );
    }
}

#[cfg(test)]
mod tests {
    use super::mob_effect_instance;
    use super::thrown_item_axes;
    use super::thrown_item_quad;
    use super::{ProjectileKind, is_item_billboard};
    use horizon_client::camera::CameraType;

    #[test]
    #[ignore = "needs prepared resources"]
    fn conversion_transfers_tags_and_team() {
        use super::super::commands::Target;
        use horizon_client::{
            mobs::{Mob, conversion::Params},
            save::uuid_text,
        };
        use horizon_core::geometry::Vec3;

        super::super::native_entity_refs::tests::with_game(|game| {
            let kind = game
                .res
                .mob_types()
                .get("minecraft:villager")
                .unwrap()
                .clone();
            let from = Mob::new(100, kind, Vec3::ZERO, 0.0);
            game.mobs.push(from.clone());
            game.next_entity = 100;
            game.set_entity_tags(Target::Mob(100), &["tracked".into()]);
            let old = uuid_text(game.uuid_for(100));
            game.load_scoreboard();
            game.score.board.add_team("test");
            game.score.board.add_to_team(&old, "test");
            let id = game
                .convert_to(
                    &from,
                    "minecraft:zombie_villager",
                    Params {
                        keep_equipment: false,
                        preserve_can_pick_up_loot: true,
                    },
                    |_, _| {},
                )
                .unwrap();
            assert_eq!(game.tags_of(Target::Mob(id)), ["tracked"]);
            let new = uuid_text(game.uuid_for(id));
            assert_eq!(game.score.board.players_team(&new).unwrap().name, "test");
            assert!(game.score.board.players_team(&old).is_none());
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn wolf_owner_rejoining_receives_the_kill_statistic() {
        use horizon_client::advancements::{Advancement, Criterion, Rewards, Tree};
        use horizon_client::mobs::{Damage, Mob};
        use horizon_core::{geometry::Vec3, position::BlockPos};
        use std::sync::Arc;

        for dedicated in [false, true] {
            super::super::native_guest_turn_tests::with_guests(|game| {
                if dedicated {
                    game.options.dedicated = Some(Vec::new());
                }
                let mut tree = Tree::default();
                tree.list.push(Advancement {
                    id: "test:catalyst".into(),
                    parent: None,
                    display: None,
                    criteria: vec![Criterion {
                        name: "kill".into(),
                        trigger: "minecraft:kill_mob_near_sculk_catalyst".into(),
                        conditions: serde_json::json!({}).into(),
                    }],
                    requirements: vec![vec!["kill".into()]],
                    rewards: Rewards::default(),
                });
                tree.by_id.insert("test:catalyst".into(), 0);
                tree.by_trigger.insert(
                    "minecraft:kill_mob_near_sculk_catalyst".into(),
                    vec![(0, 0)],
                );
                game.adv.tree = Arc::new(tree);
                game.adv.loaded = true;
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
                let catalyst = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:sculk_catalyst")
                    .unwrap();
                game.with_level(|b, level, ticks| {
                    b.set_block(level, ticks, BlockPos::new(1, 0, 0), catalyst, 2 | 16);
                });
                game.sculk_prepare();
                game.sculk_sync_index();
                assert!(game.world.block_entities.contains_key(&(1, 0, 0)));
                let wolf_type = game.res.mob_types().get("minecraft:wolf").unwrap().clone();
                let pig_type = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                let mut wolf = Mob::new(100, wolf_type, Vec3::ZERO, 0.0);
                wolf.extra.tame = true;
                wolf.extra.owner = Some(horizon_client::lan::playerdata::uuid_ints(12));
                game.mobs.push(wolf);
                game.mobs.push(Mob::new(101, pig_type, Vec3::ZERO, 0.0));
                {
                    let value = false;
                    game.lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .guests
                        .update(0, |guest| guest.placed = value);
                }
                game.resolve_player_responsible(101, Damage::mob(100, [0.0; 3]));
                let credit = *game.last_hurt_by_player.get(&101).unwrap();
                assert!(game.resolve_player_credit(credit).is_none());
                {
                    let value = true;
                    game.lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .guests
                        .update(0, |guest| guest.placed = value);
                }
                assert_eq!(game.resolve_player_credit(credit), Some(Some(12)));
                let victim = game.mobs[1].clone();
                game.mob_died(&victim);
                let stats = &game.lan.as_ref().unwrap().play.get(&12).unwrap().stats;
                assert_eq!(stats.get("custom", "minecraft:mob_kills"), 1);
                game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
                    assert!(guest.advancements.has("test:catalyst", "kill"));
                    guest.advancements = Default::default();
                });
                let mut second = victim.clone();
                second.id = 102;
                game.last_hurt_by_player
                    .insert(102, super::super::combat::PlayerCredit(12));
                game.as_guest(12, |game| game.mob_died(&second)).unwrap();
                let host = game.lan.as_ref().unwrap();
                assert_eq!(host.play[&12].stats.get("custom", "minecraft:mob_kills"), 2);
                assert!(host.hub.guests[0].advancements.has("test:catalyst", "kill"));
                {
                    let value = "minecraft:the_nether".into();
                    game.lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .guests
                        .update(0, |guest| guest.dimension = value);
                }
                let mut third = victim.clone();
                third.id = 103;
                game.resolve_player_responsible(101, Damage::mob(100, [0.0; 3]));
                let credit = game.last_hurt_by_player.remove(&101).unwrap();
                game.last_hurt_by_player.insert(103, credit);
                game.mob_died(&third);
                let host = game.lan.as_ref().unwrap();
                assert_eq!(host.play[&12].stats.get("custom", "minecraft:mob_kills"), 3);
                game.mobs[0].extra.owner = None;
                game.resolve_player_responsible(101, Damage::mob(100, [0.0; 3]));
                assert!(!game.last_hurt_by_player.contains_key(&101));
            });
        }
    }

    /// The component-wise lowest and highest corner of the quad.
    fn bounds(corners: &[glam::Vec3; 4]) -> (glam::Vec3, glam::Vec3) {
        (
            corners.iter().copied().reduce(glam::Vec3::min).unwrap(),
            corners.iter().copied().reduce(glam::Vec3::max).unwrap(),
        )
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn ender_pearl_clears_impulse_before_next_fall() {
        use super::{Game, Hit, Projectile, Vec3, Who};
        use crate::game::{Audio, Options, Progress, Settings, load_resources};
        use horizon_client::motion::{MotionWorld, PlayerMotion};

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
        let mut player = PlayerMotion::new(game.motion, Vec3::new(0.0, 64.0, 0.0)).unwrap();
        player
            .impulse
            .set_ignore_fall_damage(true, Some(player.position));
        player.fall_distance = 4.0;
        game.player = Some(player);
        let destination = Vec3::new(0.0, 80.0, 0.0);
        let mut pearl = Projectile::new(ProjectileKind::EnderPearl, destination, Some(Who::Player));
        pearl.previous = destination;
        game.projectile_hit(
            &pearl,
            Hit::Block {
                pos: [0, 80, 0],
                at: destination,
                face: [0; 3],
            },
        )
        .unwrap();
        let player = game.player.as_mut().unwrap();
        assert_eq!(player.position, destination);
        assert_eq!(player.fall_distance, 0.0);
        let damaged = MotionWorld {
            entities: &[],
            player,
            scene: &mut game.world,
            assets: &game.pack.assets,
            collision_binding: game.collision,
            resources: game.motion,
            health: &game.health_resources,
            tag_items: Some(game.res.items()),
            loop_context: &game.loop_context,
            damage_hooks: &horizon_client::health::DamageHooks::NONE,
            block_registries: Some(&game.ui.registries),
        }
        .cause_mount_fall(10.0, 1.0)
        .unwrap();
        assert!(
            damaged,
            "a ten-block fall after the teleport must cause damage"
        );
    }

    #[test]
    fn nautilus_breath_lands_in_creative_as_ambient() {
        // AbstractNautilus.applyEffects: MobEffectInstance(BREATH_OF_THE_NAUTILUS,
        // 60, 0, ambient true, visible true, showIcon true), no game-mode gate.
        let e = mob_effect_instance("minecraft:breath_of_the_nautilus", 60, 0, true, true)
            .expect("applies to a creative player");
        assert_eq!(
            (e.id, e.duration, e.amplifier),
            ("breath_of_the_nautilus", 60, 0)
        );
        assert!(e.ambient && e.visible);
        // Survival gets the same ambient instance.
        let e =
            mob_effect_instance("minecraft:breath_of_the_nautilus", 60, 0, true, false).unwrap();
        assert!(e.ambient);
    }

    #[test]
    fn other_mob_effects_stay_blocked_in_creative() {
        assert!(mob_effect_instance("minecraft:poison", 140, 0, false, true).is_none());
        let e = mob_effect_instance("minecraft:poison", 140, 0, false, false).unwrap();
        assert!(!e.ambient);
    }

    #[test]
    fn thrown_item_billboard_lifts_by_the_generated_ground_transform() {
        // The kinds vanilla draws with `ThrownItemRenderer` as a billboard: the
        // only lifted ones. A fireball hands `getItem` to the item renderer
        // instead, which carries the ground transform itself.
        assert!(is_item_billboard(&ProjectileKind::Snowball));
        assert!(is_item_billboard(&ProjectileKind::Egg));
        assert!(is_item_billboard(&ProjectileKind::EnderPearl));
        assert!(!is_item_billboard(&ProjectileKind::SmallFireball));
        let large = ProjectileKind::LargeFireball { power: 1 };
        assert!(!is_item_billboard(&large));
        // A projectile at y = 100 with right = +X, up = +Y: the sprite sits
        // 0.125 blocks above the entity (2 of `item/generated`'s 16 units) and
        // stays 0.5 blocks wide, so the quad's y runs 99.875 .. 100.375.
        let at = glam::Vec3::Y * 100.0;
        let (right, up) = (glam::Vec3::X, glam::Vec3::Y);
        let lifted = is_item_billboard(&ProjectileKind::Snowball);
        let (lo, hi) = bounds(&thrown_item_quad(at, right, up, 1.0, lifted));
        assert_eq!((lo.y, hi.y), (99.875, 100.375));
        assert_eq!((lo.x, hi.x), (-0.25, 0.25));
        // Llama spit shares the quad here but is a model in vanilla
        // (`LlamaSpitRenderer`): no lift at all.
        let lifted = is_item_billboard(&ProjectileKind::LlamaSpit);
        assert!(!lifted);
        let (lo, hi) = bounds(&thrown_item_quad(at, right, up, 1.0, lifted));
        assert_eq!((lo.y, hi.y), (99.75, 100.25));
        // The rise follows the billboard's own up axis, not world up.
        let (right, up) = (glam::Vec3::Z, glam::Vec3::X);
        let lifted = is_item_billboard(&ProjectileKind::Egg);
        let (lo, hi) = bounds(&thrown_item_quad(at, right, up, 1.0, lifted));
        assert_eq!((lo.x, hi.x), (-0.125, 0.375));
        assert_eq!((lo.y, hi.y), (100.0, 100.0));
    }

    #[test]
    fn thrown_item_billboard_does_not_flip_at_straight_up_or_down() {
        for yaw in [0.0f32, 37.5, 90.0, 180.0, -135.0] {
            for (edge, near) in [(-90.0f32, -89.9f32), (90.0, 89.9)] {
                let (right_edge, up_edge) = thrown_item_axes(CameraType::FirstPerson, yaw, edge);
                let (right_near, up_near) = thrown_item_axes(CameraType::FirstPerson, yaw, near);
                assert!(
                    right_edge.dot(right_near) > 0.99 && up_edge.dot(up_near) > 0.99,
                    "yaw {yaw}, pitch {edge}: {right_edge}/{up_edge} vs {right_near}/{up_near} at {near}"
                );
            }
        }
    }

    #[test]
    fn thrown_item_billboard_faces_the_front_camera() {
        // Camera.setup turns the front camera round (yaw + 180, -pitch): its
        // right is the player's left.
        let (right, up) = thrown_item_axes(CameraType::FirstPerson, 0.0, 0.0);
        assert!(right.abs_diff_eq(glam::Vec3::NEG_X, 1e-5), "{right}");
        assert!(up.abs_diff_eq(glam::Vec3::Y, 1e-5), "{up}");
        let (right, up) = thrown_item_axes(CameraType::ThirdPersonFront, 0.0, 0.0);
        assert!(right.abs_diff_eq(glam::Vec3::X, 1e-5), "{right}");
        assert!(up.abs_diff_eq(glam::Vec3::Y, 1e-5), "{up}");
        let (right, _) = thrown_item_axes(CameraType::ThirdPersonBack, 0.0, 0.0);
        assert!(right.abs_diff_eq(glam::Vec3::NEG_X, 1e-5), "{right}");
    }
}

/// Whether the killing blow's causing entity (`DamageSource.getEntity`) was a
/// mob, such as a tame wolf whose owner gets the kill credit.
fn killed_by_mob(m: &Mob) -> bool {
    m.last_damage
        .is_some_and(|(d, _)| matches!(d.attacker, Some(Who::Mob(_))))
}

/// `Animal.finalizeSpawnChildFromBreeding`'s player: the one who fed the
/// parent, or else the one who fed the partner (`getLoveCause`). `Some(None)`
/// is the host, `Some(Some(conn))` a guest. Both animals are spent.
/// `DyeColor.getMixedColor`: the colour of the crafting recipe that mixes
/// the two dyes, else one of the two picked at random.
fn dye_mix(
    context: &horizon_world::native_load_context::LoopContext,
    recipes: &Recipes,
    random: &mut impl RandomSource,
    a: u8,
    b: u8,
) -> u8 {
    if let Some(mixed) = dye_color_mix_in_recipes(context, recipes, a, b) {
        return mixed;
    }
    if random.next_boolean() { a } else { b }
}

/// `DyeColor.findColorMixInRecipes`: the dye colour a two-dye crafting recipe
/// makes, when its result is a dye. The two dyes are the grid's first row
/// (`CraftingInput.of(2, 1, ...)`) and the recipe manager answers for it; the
/// result's `_dye` item names the colour.
fn dye_color_mix_in_recipes(
    context: &horizon_world::native_load_context::LoopContext,
    recipes: &Recipes,
    a: u8,
    b: u8,
) -> Option<u8> {
    let dye = |c: u8| {
        let name = horizon_client::mobs::render::DYE_NAMES[usize::from(c) & 15];
        Some(ItemStack::new(&format!("minecraft:{name}_dye"), 1))
    };
    let grid = [dye(a), dye(b), None, None];
    let (_, result) = recipes.find(context, &grid, 2)?;
    let name = result
        .item
        .strip_prefix("minecraft:")?
        .strip_suffix("_dye")?;
    let index = horizon_client::mobs::render::DYE_NAMES
        .iter()
        .position(|n| *n == name)?;
    u8::try_from(index).ok()
}

fn breeding_credit(
    love_cause: &mut std::collections::HashMap<u32, Option<u32>>,
    parent: u32,
    partner: u32,
) -> Option<Option<u32>> {
    let from_parent = love_cause.remove(&parent);
    let from_partner = love_cause.remove(&partner);
    from_parent.or(from_partner)
}

#[cfg(test)]
mod mob_subscription_tests {
    #[test]
    #[ignore = "needs prepared resources"]
    fn empty_mod_never_defers_taming_or_block_destruction() {
        use mod_api::{self, Hooks};
        struct Empty;
        #[mod_api::__subscriptions]
        impl Hooks for Empty {}
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new("empty", Box::new(Empty))].into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                let world = game.mob_world_base(&[]);
                assert!(!world.asks_animal_tame && !world.asks_living_destroy_block);
                let mut cancel = false;
                game.native_animal_tame(1, 1, 2, "minecraft:horse", &mut cancel);
                game.native_living_destroy_block(
                    2,
                    "minecraft:ravager",
                    [0; 3],
                    game.world.air,
                    &mut cancel,
                );
                let world = game.mob_world_base(&[]);
                assert!(!world.asks_animal_tame && !world.asks_living_destroy_block);
                assert!(!cancel);
            });
        });
    }
}

#[cfg(test)]
mod shulker_arrow_tests {
    use super::*;
    use horizon_client::arrows::{ArrowEvent, deflect_reverse};
    use horizon_core::random::LegacyRandom;

    /// `AbstractArrow.onHitEntity` after `hurtOrSimulate` fails and the target
    /// still takes a projectile's side effects: `deflect(REVERSE, …, 0.2)`,
    /// and no hit sound. A closed shulker's refusal is that miss
    /// (`shulker::tests`). An enderman's teleport is `Dodged`, not this.
    #[test]
    fn a_refused_arrow_reverses_without_a_hit_sound() {
        assert_eq!(arrow_miss(false), EntityHit::Refused);
        assert_eq!(arrow_miss(true), EntityHit::Dodged);

        let mut arrow = Projectile::new(
            ProjectileKind::Arrow {
                base_damage: 2.0,
                critical: false,
                effect: None,
            },
            Vec3::new(0.0, 1.0, 0.0),
            Some(Who::Player),
        );
        arrow.random.0 = LegacyRandom::new(7);
        arrow.velocity = Vec3::new(3.0, -1.0, 2.0);
        arrow.yaw = 15.0;
        let mut oracle = LegacyRandom::new(7);
        let expect_yaw = 15.0 + 170.0 + oracle.next_float() * 20.0;
        let mut state = arrow.arrow.take().expect("an arrow");
        deflect_reverse(
            &mut arrow.velocity,
            &mut arrow.yaw,
            &mut state,
            &mut arrow.random.0,
        );
        // power 0.2, and REVERSE multiplies by -power * 0.5.
        assert!((arrow.velocity.x - -0.3).abs() < 1e-6);
        assert!((arrow.velocity.y - 0.1).abs() < 1e-6);
        assert!((arrow.velocity.z - -0.2).abs() < 1e-6);
        assert!((arrow.yaw - expect_yaw).abs() < 1e-4);
        assert!(!state.removed, "the reversed arrow stays in the world");
        assert!(
            state
                .events
                .iter()
                .all(|e| !matches!(e, ArrowEvent::Sound { .. })),
            "the deflect branch plays no hit sound"
        );
    }
}

#[cfg(test)]
mod breeding_tests {
    use super::*;
    use mod_api::ServerHost;

    use horizon_client::mobs::{
        BreedExperience,
        finalize::{Attr, set_base},
        goals::{Breed, Cx, Goal},
    };
    use horizon_core::random::{LegacyRandom, RandomSource};

    /// Isolate the goal's stream from earlier tick work, but leave the actual
    /// Breed, subsequent aiStep and queued Game event handling in the route.
    #[derive(Clone, Debug)]
    struct SeededBreed {
        goal: Breed,
        seed: i64,
    }
    impl Goal for SeededBreed {
        fn box_clone(&self) -> Box<dyn Goal> {
            Box::new(self.clone())
        }
        fn flags(&self) -> u8 {
            self.goal.flags()
        }
        fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
            self.goal.can_use(m, cx)
        }
        fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
            self.goal.can_continue(m, cx)
        }
        fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
            m.random = LegacyRandom::new(self.seed);
            self.goal.tick(m, cx);
        }
    }
    struct Bred {
        child: Mob,
        next_random: i32,
        experience: BreedExperience,
    }
    fn tick_breeding(
        game: &mut Game<'_>,
        parent_kind: &str,
        partner_kind: &str,
        seed: i64,
        mob_drops: bool,
    ) -> Bred {
        game.mobs.clear();
        game.ui.rules.set(
            "mob_drops",
            horizon_client::gamerules::Value::Bool(mob_drops),
        );
        let base = game.next_entity;
        game.next_entity += 2;
        let mut parent = Mob::new(
            base + 1,
            game.res.mob_types().get(parent_kind).unwrap().clone(),
            Vec3::new(0.0, 64.0, 0.0),
            0.0,
        );
        let mut partner = Mob::new(
            base + 2,
            game.res.mob_types().get(partner_kind).unwrap().clone(),
            Vec3::new(1.0, 64.0, 0.0),
            0.0,
        );
        parent.persistent = true;
        parent.extra.in_love = 600;
        partner.extra.in_love = 600;
        parent.extra.tame = true;
        partner.extra.tame = true;
        parent.extra.variant = 1;
        partner.extra.variant = 2;
        parent.extra.variant2 = 1;
        partner.extra.variant2 = if horizon_client::mobs::llama::is_llama(partner.species()) {
            4
        } else {
            2
        };
        set_base(&mut parent, Attr::MaxHealth, 20.0);
        set_base(&mut partner, Attr::MaxHealth, 28.0);
        set_base(&mut parent, Attr::MovementSpeed, 0.2);
        set_base(&mut partner, Attr::MovementSpeed, 0.3);
        parent.health = 20.0;
        partner.health = 28.0;
        parent.extra.vector[0] = 0.5;
        partner.extra.vector[0] = 0.9;
        // The camel's client animation delay must not add an unrelated draw.
        parent.extra.ints[0] = 1000;
        parent.ai.targets.clear();
        parent.ai.goals.clear();
        parent.ai.goals.add(
            2,
            SeededBreed {
                goal: Breed::new(1.0),
                seed,
            },
        );
        let others = [partner.seen(None, game.items())];
        let mut events = Vec::new();
        let mut test_level = horizon_client::health::TestLevel::new(321);
        let mut experience = None;
        {
            let world = MobWorld {
                others: &others,
                mob_drops,
                ..game.mob_world_base(&[])
            };
            assert!(!world.asks_baby_spawn);
            for _ in 0..200 {
                // No floor in this fixture: keep the pair in breeding range.
                parent.position = Vec3::new(0.0, 64.0, 0.0);
                parent.tick(&world, &mut test_level.level(), &mut events);
                experience = events.iter().find_map(|event| match event {
                    MobEvent::Breed { experience, .. } => Some(*experience),
                    _ => None,
                });
                if experience.is_some() {
                    break;
                }
                events.clear();
            }
        }
        let experience = experience.expect("the goal must breed");
        let tick_random = parent.random.state();
        game.mobs.extend([parent, partner]);
        game.handle_mob_events(events).unwrap();
        let parent = game.mobs.iter_mut().find(|m| m.id == base + 1).unwrap();
        assert_eq!(
            parent.random.state(),
            tick_random,
            "Game must not redraw inheritance"
        );
        let next_random = parent.random.next_int();
        let child = game.mobs.iter().find(|m| m.extra.age < 0).unwrap().clone();
        Bred {
            child,
            next_random,
            experience,
        }
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn equine_breeding_inheritance_precedes_same_tick_healing_and_grazing() {
        super::super::native_entity_refs::tests::with_game(|game| {
            // java.util.Random(42), Horse/Donkey/Llama.getBreedOffspring,
            // then XP, then AbstractHorse.aiStep's nextInt(900)/nextInt(300)
            // (llamas only heal). The attributes use stocks (20,.5,.2) and (28,.9,.3).
            for (parent, partner) in [
                ("minecraft:horse", "minecraft:horse"),
                ("minecraft:donkey", "minecraft:donkey"),
                ("minecraft:horse", "minecraft:donkey"),
                ("minecraft:donkey", "minecraft:horse"),
                ("minecraft:llama", "minecraft:llama"),
                ("minecraft:trader_llama", "minecraft:llama"),
            ] {
                let result = tick_breeding(game, parent, partner, 42, true);
                let horse = parent == "minecraft:horse" && partner == parent;
                let llama = horizon_client::mobs::llama::is_llama(result.child.species());
                assert_eq!(
                    result.child.extra.variant,
                    if horse {
                        5
                    } else if llama {
                        1
                    } else {
                        0
                    }
                );
                assert_eq!(
                    result.child.extra.variant2,
                    if horse {
                        2
                    } else if llama {
                        4
                    } else {
                        0
                    }
                );
                let expected = if horse {
                    [24.8238468170166, 0.5886834847817017, 0.3030804447968077]
                } else {
                    [24.91461181640625, 0.7668932765979051, 0.22812363785863735]
                };
                let actual = [
                    f64::from(result.child.attrs.max_health),
                    result.child.extra.vector[0],
                    result.child.attrs.speed,
                ];
                for (actual, expected) in actual.into_iter().zip(expected) {
                    assert!(
                        (actual - expected).abs() < 1e-12,
                        "{parent}/{partner}: {actual} != {expected}"
                    );
                }
                assert_eq!(
                    result.experience,
                    BreedExperience::Drawn(Some(if horse || llama { 2 } else { 1 }))
                );
                assert_eq!(
                    result.next_random,
                    if horse {
                        -1_074_144_810
                    } else if llama {
                        1_888_925_827
                    } else {
                        652_655_727
                    },
                    "{parent}/{partner}"
                );
                if parent != partner && !llama {
                    assert_eq!(result.child.species(), Species::Mule);
                }
                if parent == "minecraft:trader_llama" {
                    assert!(result.child.persistent);
                }
            }
        });
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn wolf_breeding_selects_either_coat_from_the_parent_stream() {
        super::super::native_entity_refs::tests::with_game(|game| {
            // Computed with an independent implementation of the documented java.util.Random
            // algorithm, not from the game's output.
            // Java LCG: s0 = (seed ^ 0x5DEECE66D) & (2^48 - 1),
            // s' = (25214903917*s + 11) mod 2^48. Draw 1 selects the coat;
            // draw 2 selects one of 7 voices; draw 3 yields 1 + (s >> 17) % 7 XP;
            // draw 4 is signed(s >> 16). Neither bounded draw rejects these seeds.
            for (seed, coat, xp, next) in [(42, 1, 7, 205_897_768), (4096, 2, 3, 1_387_017_903)] {
                let result = tick_breeding(game, "minecraft:wolf", "minecraft:wolf", seed, true);
                assert_eq!(result.child.extra.variant, coat);
                assert_eq!(result.experience, BreedExperience::Drawn(Some(xp)));
                assert_eq!(result.next_random, next);
            }
        });
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn camel_breeding_leaves_the_parent_stream_unchanged() {
        super::super::native_entity_refs::tests::with_game(|game| {
            let kind = "minecraft:camel";
            let result = tick_breeding(game, kind, kind, 42, false);
            assert_eq!(result.next_random, -1_170_105_035); // Random(42).nextInt(), no inheritance.
            assert_eq!(result.child.attrs.max_health, result.child.kind.max_health);
            assert_eq!(result.child.attrs.speed, result.child.kind.speed);
        });
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn breeding_announces_the_orb_before_the_lower_id_child() {
        use mod_api::{self, BabyEntitySpawnEvent, EntityJoinLevelEvent, Flow, Hooks};
        use std::sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
        };

        struct Observer {
            joins: Arc<Mutex<Vec<(u32, String)>>>,
            cancel: Arc<AtomicBool>,
        }
        #[mod_api::__subscriptions]
        impl Hooks for Observer {
            fn baby_entity_spawn(
                &self,
                _: &mut dyn ServerHost,
                _: &mut BabyEntitySpawnEvent<'_>,
            ) -> Flow {
                if self.cancel.load(Ordering::SeqCst) {
                    Flow::Cancel
                } else {
                    Flow::Continue
                }
            }
            fn entity_join_level(
                &self,
                _: &mut dyn ServerHost,
                event: &mut EntityJoinLevelEvent<'_>,
            ) -> Flow {
                self.joins
                    .lock()
                    .unwrap()
                    .push((event.entity, event.kind.into()));
                Flow::Continue
            }
        }
        let joins = Arc::new(Mutex::new(Vec::new()));
        let cancel = Arc::new(AtomicBool::new(false));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "breeding-observer",
            Box::new(Observer {
                joins: joins.clone(),
                cancel: cancel.clone(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                assert!(game.mob_world_base(&[]).asks_baby_spawn);
                let cow = game.res.mob_types().get("minecraft:cow").unwrap().clone();
                let base = game.next_entity;
                game.next_entity += 2;
                game.mobs.push(Mob::new(
                    base + 1,
                    cow.clone(),
                    Vec3::new(0.0, 64.0, 0.0),
                    0.0,
                ));
                game.mobs
                    .push(Mob::new(base + 2, cow, Vec3::new(1.0, 64.0, 0.0), 0.0));
                game.native_joins_skip();
                let parent = game.mobs.iter_mut().find(|mob| mob.id == base + 1).unwrap();
                parent.random = horizon_core::random::LegacyRandom::new(0);
                parent.extra.variant = 1;
                parent.extra.age = 6000;
                let child = game.breed(
                    base + 1,
                    base + 2,
                    None,
                    None,
                    None,
                    horizon_client::mobs::BreedExperience::Deferred,
                );
                assert_eq!(child, Some(base + 3));
                assert_eq!(
                    game.mobs
                        .iter()
                        .find(|mob| mob.id == base + 3)
                        .unwrap()
                        .extra
                        .age,
                    -24000
                );
                assert_eq!(
                    game.mobs
                        .iter()
                        .find(|mob| mob.id == base + 1)
                        .unwrap()
                        .extra
                        .age,
                    6000
                );
                assert_eq!(
                    game.mobs
                        .iter()
                        .find(|mob| mob.id == base + 2)
                        .unwrap()
                        .extra
                        .age,
                    6000
                );
                assert_eq!(
                    *joins.lock().unwrap(),
                    vec![
                        (base + 4, "minecraft:experience_orb".into()),
                        (base + 3, "minecraft:cow".into())
                    ]
                );
                assert_eq!(
                    game.mobs
                        .iter()
                        .find(|mob| mob.id == base + 3)
                        .unwrap()
                        .extra
                        .variant,
                    1
                );
                assert_eq!(
                    game.mobs
                        .iter()
                        .find(|mob| mob.id == base + 1)
                        .unwrap()
                        .random
                        .state(),
                    234_029_835_660_143
                );
                joins.lock().unwrap().clear();
                let fox = game.res.mob_types().get("minecraft:fox").unwrap().clone();
                let fox_base = game.next_entity;
                game.next_entity += 2;
                game.mobs.push(Mob::new(
                    fox_base + 1,
                    fox.clone(),
                    Vec3::new(0.0, 64.0, 0.0),
                    0.0,
                ));
                game.mobs
                    .push(Mob::new(fox_base + 2, fox, Vec3::new(1.0, 64.0, 0.0), 0.0));
                game.native_joins_skip();
                game.player = Some(
                    horizon_client::motion::PlayerMotion::new(game.motion, Vec3::ZERO).unwrap(),
                );
                let feeder = horizon_client::lan::playerdata::uuid_ints(game.native_acting_id());
                game.love_cause.insert(fox_base + 1, None);
                let fox_child = game.breed(
                    fox_base + 1,
                    fox_base + 2,
                    None,
                    None,
                    None,
                    horizon_client::mobs::BreedExperience::Deferred,
                );
                assert_eq!(fox_child, Some(fox_base + 3));
                let fox_child = game.mobs.iter().find(|mob| mob.id == fox_base + 3).unwrap();
                assert!(horizon_client::mobs::fox::trusts(fox_child, feeder));
                assert_eq!(
                    *joins.lock().unwrap(),
                    vec![
                        (fox_base + 3, "minecraft:fox".into()),
                        (fox_base + 4, "minecraft:experience_orb".into())
                    ]
                );
                joins.lock().unwrap().clear();
                let next = game.next_entity;
                let parent = game.mobs.iter_mut().find(|mob| mob.id == base + 1).unwrap();
                let mut expected = parent.random.clone();
                let choice = parent.random.next_boolean();
                assert_eq!(choice, expected.next_boolean());
                let random = expected.state();
                cancel.store(true, Ordering::SeqCst);
                let child = game.breed(
                    base + 1,
                    base + 2,
                    None,
                    Some(choice),
                    None,
                    horizon_client::mobs::BreedExperience::Deferred,
                );
                assert_eq!(child, None);
                assert_eq!(game.next_entity, next);
                assert_eq!(
                    game.mobs
                        .iter()
                        .find(|mob| mob.id == base + 1)
                        .unwrap()
                        .random
                        .state(),
                    random
                );
                assert!(joins.lock().unwrap().is_empty());
            });
        });
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn first_chicken_breed_matches_without_mods_and_with_an_empty_mod() {
        use horizon_client::mobs::{BreedExperience, MobEvent, goals::Breed};
        use horizon_core::random::LegacyRandom;
        use mod_api::{self, Hooks};
        struct Empty;
        #[mod_api::__subscriptions]
        impl Hooks for Empty {}
        super::super::native_entity_refs::tests::with_game(|game| {
            let kind = game
                .res
                .mob_types()
                .get("minecraft:chicken")
                .unwrap()
                .clone();
            let mut mate = Mob::new(2, kind.clone(), Vec3::new(0.5, 64.0, 0.0), 0.0);
            mate.extra.in_love = 600;
            let others = [mate.seen(None, game.items())];
            let run = || {
                let world = MobWorld {
                    others: &others,
                    ..game.mob_world_base(&[])
                };
                assert!(!world.asks_baby_spawn);
                let mut parent = Mob::new(1, kind.clone(), Vec3::new(0.0, 64.0, 0.0), 0.0);
                parent.random = LegacyRandom::new(123);
                parent.persistent = true;
                parent.extra.in_love = 600;
                parent.extra.egg_time = 10_000;
                parent.ai.targets.clear();
                parent.ai.goals.clear();
                parent.ai.goals.add(2, Breed::new(1.0));
                let mut events = Vec::new();
                let mut test_level = horizon_client::health::TestLevel::new(321);
                let level = &mut test_level.level();
                for _ in 0..200 {
                    // The fixture has no floor; keep the pair within breeding range.
                    parent.position = Vec3::new(0.0, 64.0, 0.0);
                    parent.tick(&world, level, &mut events);
                    if let Some(xp) = events.iter().find_map(|event| match event {
                        MobEvent::Breed {
                            experience: BreedExperience::Drawn(Some(xp)),
                            ..
                        } => Some(*xp),
                        _ => None,
                    }) {
                        return (parent.random.state(), xp);
                    }
                    events.clear();
                }
                panic!("chickens did not breed");
            };
            let no_mods = mod_platform::testing::with_mods(Vec::new().into(), run);
            let empty = vec![mod_api::ModEntry::new("empty", Box::new(Empty))].into();
            let unrelated = mod_platform::testing::with_mods(empty, run);
            assert_eq!(no_mods, unrelated);
        });
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn empty_mod_never_defers_breeding_xp() {
        use mod_api::{self, Hooks};
        struct Empty;
        #[mod_api::__subscriptions]
        impl Hooks for Empty {}
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new("empty", Box::new(Empty))].into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                assert!(!game.mob_world_base(&[]).asks_baby_spawn);
                let mut cancel = false;
                game.native_baby_entity_spawn(1, 2, None, None, "minecraft:chicken", &mut cancel);
                assert!(!cancel);
                assert!(!game.mob_world_base(&[]).asks_baby_spawn);
            });
        });
    }

    #[test]
    fn the_player_who_fed_either_animal_is_credited() {
        let mut love = std::collections::HashMap::from([(1, Some(7)), (2, Some(9))]);
        // The parent's feeder first, and both animals are spent.
        assert_eq!(breeding_credit(&mut love, 1, 2), Some(Some(7)));
        assert!(love.is_empty());
        // Only the partner was fed: still credited.
        let mut love = std::collections::HashMap::from([(2, None)]);
        assert_eq!(breeding_credit(&mut love, 1, 2), Some(None));
        // Neither was fed by a player (a bred pair loaded from a save).
        assert_eq!(breeding_credit(&mut love, 1, 2), None);
    }
}

#[cfg(test)]
mod projectile_batch_tests {
    use super::*;
    use horizon_client::{chunk::Chunk, join_seq::JoinSeq};
    use horizon_mod_runtime::attachments::{
        AttachmentType, Attachments, HasAttachmentLifetime, Holder, index::AttachmentIndex,
    };
    use mod_api::{Flow, HearsProjectileImpactsEvent, Hooks, ProjectileImpactEvent, ServerHost};
    use std::sync::{Arc, Mutex};
    static VALUE: AttachmentType<i32> = AttachmentType {
        namespace: "test",
        name: "projectile_lifetime",
        default: || 0,
        codec: None,
        copy_on_death: false,
        sync: false,
    };

    struct BoundProjectiles {
        _values: [Attachments; 2],
        index: Arc<Mutex<AttachmentIndex<mod_api::EntityRef>>>,
        registrations: [horizon_mod_runtime::attachments::AttachmentRegistration; 2],
    }

    impl BoundProjectiles {
        fn bind_two(game: &mut Game<'_>) -> Self {
            let index = Arc::new(Mutex::new(AttachmentIndex::default()));
            let mut values = [Attachments::default(), Attachments::default()];
            let registrations = std::array::from_fn(|i| {
                let id = game.creatures.projectiles[i].id;
                let holder = mod_api::EntityRef::new(game.native_lifecycle.epoch().unwrap(), id);
                VALUE
                    .set(&mut values[i], Holder::Entity(id), 10 + i as i32)
                    .unwrap();
                game.creatures.projectiles.update(i, |p| {
                    p.attachment_lifetime().bind(&mut values[i], &index, holder);
                    p.attachment_lifetime().registration().unwrap()
                })
            });
            Self {
                _values: values,
                index,
                registrations,
            }
        }
    }

    #[derive(Clone, Default)]
    struct Observed {
        impacts: Vec<u32>,
        spawned: bool,
        buffered: bool,
        remove: Option<u32>,
    }

    struct CancelImpacts(Arc<Mutex<Observed>>);

    #[mod_api::__subscriptions]
    impl Hooks for CancelImpacts {
        fn vanilla_game_event(
            &self,
            host: &mut dyn ServerHost,
            event: &mut mod_api::VanillaGameEventEvent<'_>,
        ) -> Flow {
            if event.event == "minecraft:projectile_shoot" {
                let mut observed = self.0.lock().unwrap();
                if !observed.buffered {
                    // Shoot events fire after settlement while the tick batch is taken.
                    let _ = host.summon(
                        event.dimension,
                        "minecraft:snowball",
                        [9.5, 70.0, 0.5],
                        Default::default(),
                    );
                    observed.buffered = true;
                }
            }
            Flow::Continue
        }

        fn hears_projectile_impacts(
            &self,
            _: &dyn mod_api::Host,
            event: &mut HearsProjectileImpactsEvent,
        ) {
            event.hears = true;
        }

        fn projectile_impact(
            &self,
            host: &mut dyn ServerHost,
            event: &mut ProjectileImpactEvent<'_>,
        ) -> Flow {
            let mut observed = self.0.lock().unwrap();
            observed.impacts.push(event.projectile);
            if !observed.spawned {
                // The projectile is installed even though summon has no projectile return target.
                let _ = host.summon(
                    event.dimension,
                    "minecraft:snowball",
                    [8.5, 70.0, 0.5],
                    Default::default(),
                );
                observed.spawned = true;
            }
            if observed.remove == Some(event.projectile) {
                let entity = host.entity(event.projectile).unwrap();
                assert!(entity.discard(host));
            }
            Flow::Cancel
        }
    }

    fn with_batch(
        remove: bool,
        test: impl FnOnce(&mut Game<'_>, [u32; 3], [JoinSeq; 3], u32, u32),
    ) {
        let observed = Arc::new(Mutex::new(Observed::default()));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "cancel-projectile-batch",
            Box::new(CancelImpacts(observed.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.world.set(2, 64, 0, stone).unwrap();
                let ids = std::array::from_fn(|i| game.next_entity + i as u32 + 1);
                game.next_entity += 3;
                // Snowballs leave on impact; arrows stay in their slot, stuck in the wall.
                // The third projectile misses, so its survivor order must be preserved.
                for (i, kind) in [
                    ProjectileKind::Snowball,
                    ProjectileKind::Arrow {
                        base_damage: 2.0,
                        critical: false,
                        effect: None,
                    },
                    ProjectileKind::Snowball,
                ]
                .into_iter()
                .enumerate()
                {
                    let x = if i == 2 { 5.5 } else { 0.5 };
                    let mut p = Projectile::new(kind, Vec3::new(x, 64.5, 0.5), None);
                    p.id = ids[i];
                    p.velocity = Vec3::new(3.0, 0.0, 0.0);
                    game.creatures.projectiles.push(p);
                }
                let joins = std::array::from_fn(|i| game.creatures.projectiles[i].join);
                let bound = BoundProjectiles::bind_two(game);
                let index = &bound.index;
                let key = ("test".into(), "projectile_lifetime".into());
                let before = index.lock().unwrap().snapshot(&key);
                assert_eq!(before.len(), 2);
                observed.lock().unwrap().remove = remove.then_some(ids[1]);
                game.tick_projectiles().unwrap();
                assert_eq!(index.lock().unwrap().snapshot(&key).len(), 2);
                for (id, original) in ids[..2].iter().zip(&bound.registrations) {
                    let i = game
                        .creatures
                        .projectiles
                        .iter()
                        .position(|p| p.id == *id)
                        .unwrap();
                    let current = game
                        .creatures
                        .projectiles
                        .update(i, |p| p.attachment_lifetime().registration().unwrap());
                    assert!(original.same_registration(&current));
                }
                let observed = observed.lock().unwrap().clone();
                assert_eq!(observed.impacts, [ids[0], ids[1]]);
                assert!(observed.spawned && observed.buffered);
                let spawned = game
                    .creatures
                    .projectiles
                    .iter()
                    .find(|p| p.position.x == 8.5)
                    .unwrap()
                    .id;
                let buffered = game
                    .creatures
                    .projectiles
                    .iter()
                    .find(|p| p.position.x == 9.5)
                    .unwrap();
                assert_eq!(buffered.age, 0);
                let buffered = buffered.id;
                test(game, ids, joins, spawned, buffered);
                assert_eq!(
                    index.lock().unwrap().snapshot(&key).len(),
                    if remove { 1 } else { 2 }
                );
                game.creatures.projectiles.clear();
                assert!(index.lock().unwrap().snapshot(&key).is_empty());
                assert!(
                    before
                        .iter()
                        .all(|row| matches!(row.entry.read(&VALUE), Ok(10 | 11)))
                );
            });
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn no_impact_expiration_retires_only_the_expired_owner() {
        super::super::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            for (id, age, x) in [(71, 5999, 0.5), (72, 0, 5.5)] {
                let mut p =
                    Projectile::new(ProjectileKind::Snowball, Vec3::new(x, 70.0, 0.5), None);
                p.id = id;
                p.age = age;
                p.velocity = Vec3::new(0.25, 0.0, 0.0);
                game.creatures.projectiles.push(p);
            }
            let bound = BoundProjectiles::bind_two(game);
            let key = ("test".into(), "projectile_lifetime".into());
            let before = bound.index.lock().unwrap().snapshot(&key);
            assert_eq!(before.len(), 2);
            game.tick_projectiles().unwrap();
            assert_eq!(
                game.creatures
                    .projectiles
                    .iter()
                    .map(|p| p.id)
                    .collect::<Vec<_>>(),
                [72]
            );
            assert_eq!(game.creatures.projectiles[0].age, 1);
            let current = game
                .creatures
                .projectiles
                .update(0, |p| p.attachment_lifetime().registration().unwrap());
            assert!(bound.registrations[1].same_registration(&current));
            let after = bound.index.lock().unwrap().snapshot(&key);
            assert_eq!(after.len(), 1);
            assert_eq!(after[0].entry.read(&VALUE), Ok(&11));
            game.creatures.projectiles.clear();
            assert!(bound.index.lock().unwrap().snapshot(&key).is_empty());
            assert!(
                before
                    .iter()
                    .all(|row| matches!(row.entry.read(&VALUE), Ok(10 | 11)))
            );
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn canceled_impacts_keep_projectile_batch_joins_and_survivor_order() {
        with_batch(false, |game, ids, joins, spawned, buffered| {
            let list = &mut game.creatures.projectiles;
            // The replayed snowball is appended; the arrow keeps its original slot.
            assert_eq!(
                list.iter().map(|p| p.id).collect::<Vec<_>>(),
                [buffered, ids[1], ids[2], spawned, ids[0]]
            );
            for (id, join) in ids.into_iter().zip(joins) {
                let p = list.iter().find(|p| p.id == id).unwrap();
                assert_eq!(p.join, join);
                assert_eq!(p.age, 1);
                assert!(p.position.x > 2.0);
                assert!(!p.stuck);
            }
            let snowball = list.iter().position(|p| p.id == ids[0]).unwrap();
            list.update(snowball, |p| {
                assert!(p.has_been_shot);
                assert!(p.take_shoot_event().is_none());
            });
            let fresh = list.iter().find(|p| p.id == spawned).unwrap();
            assert!(joins.into_iter().all(|join| join < fresh.join));
            assert_eq!(fresh.age, 0);
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_projectile_removed_in_its_impact_hook_stays_removed_after_replay() {
        with_batch(true, |game, ids, joins, spawned, buffered| {
            // Pending removal refuses the handle already, even before the physical sweep.
            assert!(game.entity(ids[1]).is_none());
            // Refusal must happen before replay, not only at the later removal sweep.
            let arrow = game
                .creatures
                .projectiles
                .iter()
                .find(|p| p.id == ids[1])
                .unwrap();
            assert!(arrow.stuck);
            assert!(arrow.position.x < 2.0);
            game.native_removals_flush();
            let list = &game.creatures.projectiles;
            assert_eq!(
                list.iter().map(|p| p.id).collect::<Vec<_>>(),
                [buffered, ids[2], spawned, ids[0]]
            );
            assert_eq!(list[1].join, joins[2]);
            assert_eq!(list[3].join, joins[0]);
            game.tick_projectiles().unwrap();
            assert!(!game.creatures.projectiles.iter().any(|p| p.id == ids[1]));
        });
    }
}

#[cfg(test)]
mod breeze_random_tests {
    use super::*;

    #[test]
    #[ignore = "needs prepared resources"]
    fn projectile_handle_pierce_level_changes_real_hits() {
        super::super::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            for (id, x) in [(11, 1.0), (12, 2.0), (13, 3.0)] {
                game.mobs
                    .push(Mob::new(id, zombie.clone(), Vec3::new(x, 64.0, 0.0), 0.0));
            }
            let mut shot = Projectile::new(
                ProjectileKind::Arrow {
                    base_damage: 2.0,
                    critical: false,
                    effect: None,
                },
                Vec3::new(0.0, 64.5, 0.0),
                None,
            );
            shot.velocity = Vec3::new(4.0, 0.0, 0.0);
            game.add_projectile(shot);
            let id = game.creatures.projectiles[0].id;
            let handle = game.projectile(game.entity(id).unwrap()).unwrap();
            assert_eq!(handle.pierce_level(game), Ok(0));
            assert_eq!(handle.set_pierce_level(game, 1), Ok(()));
            let health: Vec<_> = game.mobs.iter().map(|m| m.health).collect();
            game.tick_projectiles().unwrap();
            assert!(game.mobs[0].health < health[0]);
            game.tick_projectiles().unwrap();
            assert!(game.mobs[1].health < health[1]);
            assert_eq!(game.mobs[2].health, health[2]);
        });
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn breeze_deflection_does_not_draw_from_level_random() {
        super::super::native_entity_refs::tests::with_game(|game| {
            let breeze = game
                .res
                .mob_types()
                .get("minecraft:breeze")
                .unwrap()
                .clone();
            game.mobs
                .push(Mob::new(1, breeze, Vec3::new(0.0, 64.0, 0.0), 0.0));
            let mut projectile =
                Projectile::new(ProjectileKind::Snowball, Vec3::new(-2.0, 64.5, 0.0), None);
            projectile.velocity = Vec3::new(3.0, 0.0, 0.0);
            game.creatures.projectiles.push(projectile);
            let before = game.world.random.state();
            game.tick_projectiles().unwrap();
            assert!(game.creatures.projectiles[0].velocity.x < 0.0);
            assert_eq!(game.world.random.state(), before);
        });
    }
}

#[cfg(test)]
mod damage_type_tests {
    use super::*;

    #[test]
    fn named_damage_keeps_its_registry_id_with_or_without_an_attacker() {
        let named = horizon_client::mobs::NamedDamage::new(
            "example:fan_fire",
            horizon_client::mobs::NamedDamage::IS_FIRE,
        );
        for attacker in [None, Some(Who::Mob(7)), Some(Who::Player)] {
            assert_eq!(
                source_damage_type(Damage {
                    kind: DamageKind::Named(named),
                    attacker,
                    from: None,
                    attacker_species: None,
                }),
                "example:fan_fire"
            );
        }
    }

    /// `DamageSources.explosion`: a blast with a causing entity is
    /// `player_explosion` whoever caused it, so an exploding creeper's too.
    #[test]
    fn a_creeper_blast_is_a_player_explosion() {
        let blast = |attacker| Damage {
            kind: DamageKind::Explosion,
            attacker,
            from: Some([0.0; 3]),
            attacker_species: None,
        };
        let creeper = blast(Some(Who::Mob(7)));
        assert_eq!(source_damage_type(creeper), "minecraft:player_explosion");
        // No causing entity.
        assert_eq!(source_damage_type(blast(None)), "minecraft:explosion");
        // The other kinds do not ask.
        let bite = Damage {
            kind: DamageKind::MobAttack,
            ..creeper
        };
        assert_eq!(source_damage_type(bite), "minecraft:mob_attack");
    }
}

#[cfg(test)]
mod pearl_tests {
    use super::*;

    /// A pearl's water test is `Entity.isInWater`: `wasTouchingWater` from the
    /// last `updateFluidInteraction`, water whose surface reaches its 0.25
    /// box (#3072), not the block at its feet.
    #[test]
    fn a_pearl_feels_the_water_its_box_touches() {
        // The throwable air drag is the float 0.99 widened to a double.
        let air = f64::from(0.99f32);
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("entity-types.json").exists()
            || !root.join("movement-properties.nbt").exists()
        {
            return;
        }
        let assets = horizon_client::assets::Assets::load(&root).unwrap();
        let render = assets
            .reload(&horizon_assets::resource_source::ResourceRoot::directory(
                root.to_path_buf(),
            ))
            .unwrap();
        let registry = assets.registry;
        let facts = horizon_client::blocks::BlockFacts::load(&root, registry).unwrap();
        let motion = horizon_client::motion::MotionResources::load(&root, &assets).unwrap();
        let data = horizon_client::mobs::MobData::load(
            &horizon_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
            &facts,
        );
        let biomes = horizon_client::biome::Biomes::load(
            &horizon_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
        )
        .unwrap();
        let defaults = horizon_world::item_defaults::read(&root).unwrap();
        let items = horizon_client::items::Items::load(
            &root,
            registry,
            &facts,
            &horizon_world::loot::PackOverlay::NONE,
            &defaults,
        )
        .unwrap();
        let collision =
            std::sync::Arc::new(horizon_client::collisions::load(&root, registry).unwrap());
        let suffocation =
            horizon_client::mobs::suffocation::bind(collision.clone(), &motion.causes_suffocation)
                .unwrap();
        let mut world = horizon_client::world::World::test_scene(&assets, &render, 3).unwrap();
        // A pool of still water one block deep at y = 4, from x = 20 on (clear of the
        // scene's pyramid): its
        // surface is at 4 + 8/9.
        let water = registry.default_state("minecraft:water").unwrap();
        for x in 20..=40 {
            for z in 0..=2 {
                world.set(x, 4, z, water).unwrap();
            }
        }
        let lid_openers = std::collections::HashMap::new();
        let w = MobWorld {
            mob_maps: horizon_client::neoforge_mob_maps::NeoForgeMobMaps::vanilla(),
            goal_host: None,
            live_mobs: None,
            teams: Default::default(),
            professions: horizon_client::mobs::villager::profession_table(),
            loop_context: &crate::game::test_server_load()
                .loop_context(horizon_world::native_load_context::LoopSide::Server, []),
            world: &world,
            registry,
            facts: &facts,
            fluid_tables: &motion.fluids,
            attributes: &motion.attributes,
            movement: &motion.blocks,
            collision: &collision,
            suffocation: &suffocation,
            data: &data,
            player: None,
            player_identity: None,
            player_selection: None,
            controlling_player_movement: None,
            nearest_player: None,
            others: &[],
            dragon: None,
            leash_boats: &[],
            who_lookup: None,
            foreign: None,
            others_grid: None,
            others_index: None,
            passenger_objects: &[],
            others_villagers: None,
            others_solid: None,
            daylight: true,
            monsters_burn: false,
            game_time: 1000,
            day_time: 1000,
            raining: false,
            thundering: false,
            level: horizon_core::LevelKey::OVERWORLD,
            block_tags: &items.block_tags,
            landing: &motion.landing,
            biomes: &biomes,
            mob_griefing: true,
            universal_anger: false,
            mob_drops: true,
            max_entity_cramming: 24,
            difficulty: spawning::LocalDifficulty::new(2, 1000, 0, 0.0),
            dimension_type: horizon_world::dimension_type::DimensionFacts::overworld(),
            world_seed: 0,
            sea_level: 63,
            items: Some(&items),
            lid_openers: &lid_openers,
            mod_effects: None,
            mob_hooks: None,
            asks_animal_tame: false,
            asks_living_destroy_block: false,
            asks_baby_spawn: false,
            border: None,
            section_writer: None,
        };
        let pw = ProjWorld {
            w: &w,
            targets: &[],
            border: None,
        };
        let pearl = |x: f64, y: f64, velocity: Vec3| {
            let mut p = Projectile::new(ProjectileKind::EnderPearl, Vec3::new(x, y, 1.5), None);
            p.velocity = velocity;
            p
        };
        // Skimming the pool: its feet are in the water block, its box above
        // the surface. ThrowableProjectile.applyInertia keeps the air drag.
        let mut p = pearl(25.5, 4.95, Vec3::new(0.3, 0.0, 0.0));
        p.tick(&pw);
        assert!(p.position.y > 4.9, "the pearl dipped: {p:?}");
        p.tick(&pw);
        assert!(
            (p.velocity.x - 0.3 * air * air).abs() < 1e-9,
            "a pearl above the surface slowed as in water: {p:?}"
        );
        // At the pool's edge: its feet over the dry block at x = 19, a side
        // of its box in the water at x = 20. The water drag after the first
        // tick, which still reads `wasTouchingWater` as false.
        let mut p = pearl(19.95, 4.3, Vec3::ZERO);
        p.tick(&pw);
        assert!(
            (p.velocity.y - (-0.03 * air)).abs() < 1e-9,
            "the first tick has the air drag: {p:?}"
        );
        p.tick(&pw);
        assert!(
            (p.velocity.y - (-0.03 * air - 0.03) * 0.8).abs() < 1e-9,
            "a pearl whose box is in the water kept the air drag: {p:?}"
        );
        // Thrown under water, a pearl moves with the air drag on its first
        // tick and the water drag after it.
        let mut p = pearl(30.5, 4.2, Vec3::ZERO);
        p.tick(&pw);
        assert!(
            (p.velocity.y - (-0.03 * air)).abs() < 1e-9,
            "the first tick under water has the air drag: {p:?}"
        );
        p.tick(&pw);
        assert!(
            (p.velocity.y - (-0.03 * air - 0.03) * 0.8).abs() < 1e-9,
            "the second tick under water has the water drag: {p:?}"
        );
    }
}

#[cfg(test)]
mod knockback_tests {
    use super::*;
    use horizon_client::health::{DamageHooks, DamageLevel};

    /// What the hook of these tests saw: the player, its source (as damage
    /// type, causing, direct) and the push.
    type Pushes = std::sync::Mutex<Vec<(i32, (String, i32, i32), Knock)>>;
    static PUSHES: Pushes = std::sync::Mutex::new(Vec::new());

    /// Sets the native knockback hook of these tests for players 9201 to 9203: it records the push, cancels
    /// 9201's and doubles 9202's strength and turns its direction around.
    fn hook(_: DamageLevel<'_>, pushed: Hurting<'_>, knock: &mut Knock) -> bool {
        let Hurting::Player(entity @ 9201..=9203, s) = pushed else {
            return false;
        };
        let source = (s.damage_type.to_owned(), s.causing, s.direct);
        PUSHES.lock().unwrap().push((entity, source, *knock));
        match entity {
            9201 => return true,
            9202 => {
                knock.strength *= 2.0;
                knock.z = -knock.z;
            }
            _ => {}
        }
        false
    }

    /// `LivingEntity.knockback` on a player: the hook runs before the
    /// knockback resistance and the motion changes; a mod's strength or
    /// direction pushes, a cancel pushes nothing, and without a mod in the way
    /// the push is vanilla's. Needs the prepared data in
    /// `.local/horizon-client/26.3`; skipped without it.
    #[test]
    fn the_native_knockback_hook_changes_or_cancels_a_players_push() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = horizon_client::assets::Assets::load(&root).unwrap();
        let resources = horizon_client::motion::MotionResources::load(&root, &assets).unwrap();
        let hooks = DamageHooks {
            knockback: Some(hook),
            ..DamageHooks::NONE
        };
        let mut test_level = horizon_client::health::TestLevel::with_hooks(1, hooks);
        let hit = HurtSource {
            damage_type: "minecraft:player_attack",
            causing: 5,
            direct: 5,
            position: None,
        };
        let mut push = |entity, resistance: f64| {
            let mut p = horizon_client::motion::PlayerMotion::new(&resources, Vec3::ZERO).unwrap();
            p.entity_id = entity;
            p.ground.on_ground = true;
            if resistance != 0.0 {
                let attribute = "minecraft:knockback_resistance";
                let set = p.set_modifier(attribute, "horizon:test", resistance, "add_value", true);
                set.unwrap();
            }
            knockback::apply_player_knockback(
                &mut p,
                &mut test_level.level(),
                0.4,
                (0.0, 3.0),
                hit,
                false,
            );
            (p.velocity.x, p.velocity.y, p.velocity.z)
        };
        // No mod in the way: vanilla's push, away from z = 3.
        assert_eq!(push(9200, 0.0), (0.0, 0.4, -0.4));
        // Canceled: no motion. The hook saw the push and the hit.
        assert_eq!(push(9201, 0.0), (0.0, 0.0, 0.0));
        let made = Knock {
            strength: 0.4,
            x: 0.0,
            z: 3.0,
        };
        let named = ("minecraft:player_attack".to_owned(), 5, 5);
        let seen = |entity| -> Vec<_> {
            let seen = PUSHES.lock().unwrap();
            seen.iter().filter(|p| p.0 == entity).cloned().collect()
        };
        assert_eq!(seen(9201), [(9201, named, made)]);
        // Twice the strength, direction turned around.
        assert_eq!(push(9202, 0.0), (0.0, 0.4, 0.8));
        // Before the resistance (`LivingKnockBackEvent`): a 0.5 resistance still hands the hook
        // the raw strength, and the push that follows is halved.
        assert_eq!(push(9203, 0.5), (0.0, 0.2, -0.2));
        assert_eq!(seen(9203)[0].2.strength, 0.4);
    }
}

#[cfg(test)]
mod dye_mix_tests {
    use super::*;

    /// `DyeColor.getMixedColor` for the offspring a sheep, wolf or cat breed
    /// gets: the colour of the crafting recipe that mixes its parents' two
    /// dyes (vanilla's `crafting_shapeless` rows: red + yellow makes orange,
    /// blue + green cyan, blue + red purple), and a pair no recipe mixes
    /// keeps one of the two.
    #[test]
    fn two_dyes_mix_by_their_crafting_recipe() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("data/minecraft/recipe").is_dir() {
            return;
        }
        let registry = horizon_world::level::data::load_registry(&root).unwrap();
        let facts = crate::blocks::BlockFacts::load(&root, &registry).unwrap();
        let overlay = horizon_world::loot::PackOverlay::NONE;
        let defaults = horizon_world::item_defaults::read(&root).unwrap();
        let items =
            crate::items::Items::load(&root, &registry, &facts, &overlay, &defaults).unwrap();
        let recipes =
            Recipes::load(&root, &items, &overlay, &crate::game::test_server_load()).unwrap();
        let dye = |name: &str| {
            u8::try_from(
                horizon_client::mobs::render::DYE_NAMES
                    .iter()
                    .position(|n| *n == name)
                    .unwrap_or_else(|| panic!("{name} is not a dye colour")),
            )
            .unwrap()
        };
        let lc = crate::game::test_server_load()
            .loop_context(horizon_world::native_load_context::LoopSide::Server, []);
        let mut random = horizon_core::random::LegacyRandom::new(1);
        for (first, second, mixed) in [
            ("red", "yellow", "orange"),
            ("blue", "green", "cyan"),
            ("blue", "red", "purple"),
            ("green", "white", "lime"),
        ] {
            let (a, b, made) = (dye(first), dye(second), dye(mixed));
            assert_eq!(
                dye_color_mix_in_recipes(&lc, &recipes, a, b),
                Some(made),
                "{first} + {second} makes {mixed}"
            );
            // The recipe answers, so the coin never has to.
            assert_eq!(dye_mix(&lc, &recipes, &mut random, a, b), made);
        }
        // Two dyes that no recipe mixes keep one of the two colours.
        let (orange, cyan) = (dye("orange"), dye("cyan"));
        assert_eq!(dye_color_mix_in_recipes(&lc, &recipes, orange, cyan), None);
        let kept = dye_mix(&lc, &recipes, &mut random, orange, cyan);
        assert!(kept == orange || kept == cyan, "kept {kept}");
    }
}

impl horizon_client::arrows::OwnerLookup for Game<'_> {
    fn non_player_entity(&self, uuid: [i32; 4]) -> bool {
        let found = self.target_by_uuid(horizon_client::lan::playerdata::ints_uuid(uuid));
        !matches!(
            found,
            None | Some(super::commands::Target::Player | super::commands::Target::Guest(_))
        )
    }
}

#[cfg(test)]
mod conversion_subscriber_tests {
    use super::*;
    use mod_api::{self, Hooks, LivingConversionPostEvent, ModEntry, ServerHost};
    use std::sync::Mutex;

    #[test]
    #[ignore = "needs prepared resources"]
    fn post_only_listener_is_subscribed_without_a_pre_listener() {
        struct DefaultMod;
        #[mod_api::__subscriptions]
        impl Hooks for DefaultMod {}
        struct PostOnly(Arc<Mutex<Vec<(u32, u32)>>>);
        #[mod_api::__subscriptions]
        impl Hooks for PostOnly {
            fn living_conversion_post(
                &self,
                _: &mut dyn ServerHost,
                event: &mut LivingConversionPostEvent<'_>,
            ) {
                assert_eq!(event.kind, "minecraft:zombie");
                assert_eq!(event.converted_kind, "minecraft:drowned");
                self.0.lock().unwrap().push((event.entity, event.converted));
            }
        }
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![
            ModEntry::new("default", Box::new(DefaultMod)),
            ModEntry::new("post", Box::new(PostOnly(seen.clone()))),
        ]
        .into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                let zombie = game
                    .res
                    .mob_types()
                    .get("minecraft:zombie")
                    .unwrap()
                    .clone();
                for _ in 0..2 {
                    game.next_entity += 1;
                    let from = Mob::new(
                        game.next_entity,
                        zombie.clone(),
                        Vec3::new(0.5, 64.0, 0.5),
                        0.0,
                    );
                    let converted = game
                        .convert_to(
                            &from,
                            "minecraft:drowned",
                            horizon_client::mobs::conversion::Params::KEEP_ALL,
                            |_, _| {},
                        )
                        .unwrap();
                    let events = seen.lock().unwrap();
                    assert_eq!(events.last(), Some(&(from.id, converted)));
                }
                assert!(!mod_api::__private::listened(
                    mod_api::HookId::LivingConversionPre
                ));
                assert_eq!(seen.lock().unwrap().len(), 2);
            });
        });
    }
}

#[cfg(test)]
mod living_drops_tests {
    use super::*;
    use mod_api::ServerHost;

    #[test]
    fn a_projectile_blow_has_no_known_direct_entity_and_a_melee_one_is_its_causer() {
        let arrow = death_hurt_source("minecraft:arrow", 7);
        assert_eq!((arrow.causing, arrow.direct), (7, 0));
        let melee = death_hurt_source("minecraft:mob_attack", 7);
        assert_eq!((melee.causing, melee.direct), (7, 7));
        let none = death_hurt_source("minecraft:generic", 0);
        assert_eq!((none.causing, none.direct), (0, 0));
    }

    /// `LivingDropsEvent` hears a copper golem's preserved equipment with the
    /// rest of its drops and can cancel it, and an armor stand's break
    /// fires it with an empty list.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn the_golems_preserved_equipment_and_a_broken_stand_reach_living_drops() {
        use mod_api::{self, Flow, Hooks, LivingDropsEvent};
        use std::sync::{
            Mutex,
            atomic::{AtomicBool, Ordering},
        };
        struct Observer(Arc<Mutex<Vec<String>>>, Arc<AtomicBool>);
        #[mod_api::__subscriptions]
        impl Hooks for Observer {
            fn living_drops(
                &self,
                _: &mut dyn ServerHost,
                event: &mut LivingDropsEvent<'_>,
            ) -> Flow {
                let items: Vec<_> = event
                    .drops
                    .iter()
                    .map(|d| {
                        mod_api::__private::item_key_id(d.stack.item())
                            .unwrap()
                            .to_string()
                    })
                    .collect();
                self.0.lock().unwrap().push(format!(
                    "{} {:?} {} {}",
                    event.kind, items, event.source.damage_type, event.recently_hit
                ));
                if self.1.load(Ordering::SeqCst) {
                    Flow::Cancel
                } else {
                    Flow::Continue
                }
            }
        }
        let seen = Arc::new(Mutex::new(Vec::new()));
        let cancel = Arc::new(AtomicBool::new(false));
        let mods: mod_api::Mods = Arc::from(vec![mod_api::ModEntry::new(
            "living-drops-observer",
            Box::new(Observer(seen.clone(), cancel.clone())),
        )]);
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                game.load_server_loot();
                let golem_type = game
                    .res
                    .mob_types()
                    .get("minecraft:copper_golem")
                    .unwrap()
                    .clone();
                let spawn_golem = |game: &mut Game<'_>| {
                    game.next_entity += 1;
                    let mut golem = Mob::new(
                        game.next_entity,
                        golem_type.clone(),
                        Vec3::new(0.5, 64.0, 0.5),
                        0.0,
                    );
                    horizon_client::mobs::transport_items::set_held(
                        &mut golem,
                        Some(ItemStack::new("minecraft:diamond", 1)),
                    );
                    golem.health = 0.0;
                    golem
                };
                let golem = spawn_golem(game);
                game.mobs.push(golem.clone());
                game.mob_died(&golem);
                // The preserved stack comes last in the list the mods hear.
                assert!(
                    seen.lock().unwrap()[0].starts_with("minecraft:copper_golem [")
                        && seen.lock().unwrap()[0]
                            .ends_with("\"minecraft:diamond\"] minecraft:generic false")
                );
                let dropped = game.dropped.len();
                assert!(dropped >= 1);
                // A canceled list drops nothing, the preserved stack included.
                cancel.store(true, Ordering::SeqCst);
                let golem = spawn_golem(game);
                game.mob_died(&golem);
                assert_eq!(game.dropped.len(), dropped);
                // An armor stand that breaks fires it with nothing in the list.
                game.armor_stands
                    .push(horizon_client::entities::ArmorStand::new(
                        9000,
                        Vec3::new(4.5, 64.0, 4.5),
                        0.0,
                    ));
                game.break_armor_stand_by_anything(0, "minecraft:player_explosion", 12);
                let seen = seen.lock().unwrap();
                assert_eq!(
                    seen.last().unwrap(),
                    "minecraft:armor_stand [] minecraft:player_explosion false"
                );
            });
        });
    }
}

#[cfg(test)]
mod armor_destroy_tests {
    use super::MobEvent;
    use horizon_client::items::ItemStack;

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn body_armor_guards_and_break_event_are_preserved() {
        use horizon_client::mobs::Mob;
        use horizon_core::equipment_slot::EquipmentSlot;
        use horizon_core::geometry::Vec3;
        with_game(|game| {
            let kind = game.res.mob_types().get("minecraft:horse").unwrap().clone();
            game.mobs.push(Mob::new(21, kind, Vec3::ZERO, 0.0));
            game.audio.capture = Some(Vec::new());
            let sword = ItemStack::new("minecraft:diamond_sword", 1);
            game.mobs[0].set_equipment_stack(EquipmentSlot::Body as usize, Some(sword.clone()));
            game.hurt_mob_armor(
                21,
                4.0,
                &horizon_client::health::NO_ATTACK.player_source("minecraft:generic", 0),
            );
            assert_eq!(game.mobs[0].item_by_slot(EquipmentSlot::Body), Some(sword));
            let resistant = ItemStack::new("minecraft:netherite_chestplate", 1);
            assert!(
                !game
                    .stack_can_be_hurt_by(&resistant, "minecraft:lava")
                    .unwrap()
            );
            game.mobs[0].set_equipment_stack(EquipmentSlot::Body as usize, Some(resistant.clone()));
            game.hurt_mob_armor(
                21,
                4.0,
                &horizon_client::health::NO_ATTACK.player_source("minecraft:lava", 0),
            );
            assert_eq!(
                game.mobs[0].item_by_slot(EquipmentSlot::Body),
                Some(resistant)
            );
            let source = mod_api::ItemDamageSource {
                damage_type: mod_api::Identifier::parse("minecraft:lava").unwrap(),
                direct: None,
                causing: None,
                position: None,
            };
            let mut vulnerable = ItemStack::new("minecraft:netherite_chestplate", 1);
            vulnerable.data_mut().set_other(
                "!minecraft:damage_resistant",
                Some(horizon_core::nbt::Tag::compound(
                    [] as [(&str, horizon_core::nbt::Tag); 0]
                )),
            );
            assert_eq!(
                game.item_resource_can_be_hurt_by(&vulnerable, &source),
                Some(true)
            );
            game.mobs[0].set_equipment_stack(EquipmentSlot::Body as usize, Some(vulnerable));
            game.hurt_mob_armor(
                21,
                4.0,
                &horizon_client::health::NO_ATTACK.player_source("minecraft:lava", 0),
            );
            assert_eq!(
                game.mobs[0]
                    .item_by_slot(EquipmentSlot::Body)
                    .unwrap()
                    .damage,
                1
            );
            let mut resistant = ItemStack::new("minecraft:netherite_chestplate", 1);
            resistant.data_mut().set_other(
                "minecraft:damage_resistant",
                Some(horizon_core::nbt::Tag::compound([(
                    "types",
                    horizon_core::nbt::Tag::String("#minecraft:is_fire".into()),
                )])),
            );
            assert_eq!(
                game.item_resource_can_be_hurt_by(&resistant, &source),
                Some(false)
            );
            game.mobs[0].set_equipment_stack(EquipmentSlot::Body as usize, Some(resistant));
            game.hurt_mob_armor(
                21,
                4.0,
                &horizon_client::health::NO_ATTACK.player_source("minecraft:lava", 0),
            );
            assert_eq!(
                game.mobs[0]
                    .item_by_slot(EquipmentSlot::Body)
                    .unwrap()
                    .damage,
                0
            );
            let mut armor = ItemStack::new("minecraft:wolf_armor", 1);
            armor.damage = game.items().get(&armor.item).unwrap().max_damage - 1;
            game.mobs[0].set_equipment_stack(EquipmentSlot::Body as usize, Some(armor));
            game.hurt_mob_armor(
                21,
                4.0,
                &horizon_client::health::NO_ATTACK.player_source("minecraft:generic", 0),
            );
            assert!(game.mobs[0].item_by_slot(EquipmentSlot::Body).is_none());
            assert_eq!(game.lan_entity_events.last().map(|event| event.1), Some(65));
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn sun_protection_uses_mob_draw_and_broadcasts_break() {
        use horizon_client::mobs::Mob;
        use horizon_core::equipment_slot::EquipmentSlot;
        use horizon_core::{geometry::Vec3, random::RandomSource};
        with_game(|game| {
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(21, kind, Vec3::ZERO, 0.0));
            let mut helmet = ItemStack::new("minecraft:diamond_helmet", 1);
            helmet.damage = game.items().get(&helmet.item).unwrap().max_damage - 1;
            game.mobs[0].set_equipment_stack(EquipmentSlot::Head as usize, Some(helmet));
            game.audio.capture = Some(Vec::new());
            while game.mobs[0].random.clone().next_int_bound(2) != Ok(1) {
                let _ = game.mobs[0].random.next_int_bound(2);
            }
            game.wear_sun_protection(21);
            assert!(game.mobs[0].item_by_slot(EquipmentSlot::Head).is_none());
            assert_eq!(game.lan_entity_events.last().map(|event| event.1), Some(49));
        });
    }
    use crate::game::{answered_wear, durability_damage, native_entity_refs::tests::with_game};
    use mod_api::EquipmentSlot;

    fn entry(slot: EquipmentSlot, new_damage: f32) -> mod_api::ArmorHurt {
        mod_api::ArmorHurt {
            slot,
            stack: mod_api::ItemStack::new(
                mod_api::__private::item_key("minecraft:diamond_helmet").unwrap(),
                1,
            )
            .unwrap(),
            damage: 2.0,
            new_damage,
        }
    }

    /// A mod that clears the entries, or removes one, leaves vanilla's wear (no panic).
    #[test]
    fn a_cleared_armor_hurt_entry_keeps_vanilla_wear() {
        mod_platform::testing::with_mods(Vec::<mod_api::ModEntry>::new().into(), || {
            mod_api::__private::resolve_items(["minecraft:diamond_helmet"]).unwrap();
            assert_eq!(answered_wear(&[], EquipmentSlot::Body, 3), 3);
            let entries = [entry(EquipmentSlot::Head, 5.0)];
            assert_eq!(answered_wear(&entries, EquipmentSlot::Chest, 2), 2);
            assert_eq!(answered_wear(&entries, EquipmentSlot::Head, 2), 5);
            assert_eq!(
                answered_wear(&[entry(EquipmentSlot::Head, -1.0)], EquipmentSlot::Head, 2),
                0
            );
        });
    }

    /// `(int) Math.max(1.0F, damage / 4.0F)`.
    #[test]
    fn durability_damage_is_vanillas() {
        assert_eq!(durability_damage(0.5) as u32, 1);
        assert_eq!(durability_damage(8.0) as u32, 2);
        assert_eq!(durability_damage(11.9) as u32, 2);
    }

    /// Without a canceled break the effects play; with no break applied the
    /// `otherwise` events do (a canceled break plays no sound and the ravager jumps).
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_break_effect_follows_only_an_applied_break() {
        with_game(|game| {
            let id = game
                .spawn_mob(
                    "minecraft:ravager",
                    [0.5, 70.0, 0.5],
                    false,
                    super::SpawnSite::Plain,
                )
                .unwrap();
            let jump = |game: &mut super::Game<'_>| {
                game.mobs.iter().find(|m| m.id == id).unwrap().ai.jumping
            };
            let ifbroke = || MobEvent::IfBroke {
                mob: id,
                effects: Vec::new(),
                otherwise: vec![MobEvent::Jump { mob: id }],
            };
            // No break of this mob in the batch: the otherwise events run.
            game.mobs
                .iter_mut()
                .find(|m| m.id == id)
                .unwrap()
                .ai
                .jumping = false;
            game.handle_mob_events(vec![ifbroke()]).unwrap();
            assert!(jump(game));
            // An applied break (no mod cancels): the effects run, the jump does not.
            game.mobs
                .iter_mut()
                .find(|m| m.id == id)
                .unwrap()
                .ai
                .jumping = false;
            game.handle_mob_events(vec![
                MobEvent::BreakBlock {
                    pos: [0, 200, 0],
                    drop: false,
                    by: Some(id),
                    breaker: None,
                },
                ifbroke(),
            ])
            .unwrap();
            assert!(!jump(game));
        });
    }

    /// An enderman's pick-up is refused when the block is no longer the one decided on.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn an_enderman_does_not_take_a_changed_block() {
        with_game(|game| {
            let id = game
                .spawn_mob(
                    "minecraft:enderman",
                    [0.5, 70.0, 0.5],
                    false,
                    super::SpawnSite::Plain,
                )
                .unwrap();
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            let dirt = game
                .pack
                .assets
                .registry
                .default_state("minecraft:dirt")
                .unwrap();
            let pos = [0, 64, 0];
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
            );
            game.world.set(0, 64, 0, dirt).unwrap();
            game.mobs
                .iter_mut()
                .find(|m| m.id == id)
                .unwrap()
                .extra
                .carried = Some(stone);
            game.handle_mob_events(vec![MobEvent::TakeBlock {
                mob: id,
                pos,
                state: stone,
            }])
            .unwrap();
            assert_eq!(game.world.block(0, 64, 0), dirt);
            assert!(
                game.mobs
                    .iter()
                    .find(|m| m.id == id)
                    .unwrap()
                    .extra
                    .carried
                    .is_none()
            );
        });
    }
}

#[cfg(test)]
mod conversion_tag_tests {
    use super::*;
    use crate::game::{Audio, Options, Progress, Settings, commands::Target, load_resources};
    use horizon_core::nbt::Tag;

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn converted_mobs_keep_command_tags_at_the_new_id_and_on_save() {
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
        game.build_registries();
        for (number, (from, to)) in [
            ("husk", "zombie"),
            ("zombie", "drowned"),
            ("skeleton", "stray"),
        ]
        .into_iter()
        .enumerate()
        {
            let uuid = [10374, 1, 0, number as i32];
            let tags = Tag::List(vec![
                Tag::String("subject".into()),
                Tag::String(from.into()),
            ]);
            let raw = Tag::compound([
                (
                    "id",
                    Tag::String(format!("minecraft:{from}").as_str().into()),
                ),
                ("UUID", Tag::IntArray(uuid.to_vec())),
                (
                    "Pos",
                    Tag::List(vec![
                        Tag::Double(0.5_f64.to_bits()),
                        Tag::Double(70.0_f64.to_bits()),
                        Tag::Double(0.5_f64.to_bits()),
                    ]),
                ),
                ("NoAI", Tag::Byte(1)),
                ("NoGravity", Tag::Byte(1)),
                ("Tags", tags.clone()),
                ("horizon:old_entity_only", Tag::Byte(1)),
            ]);
            let old_id = game.load_mob(raw, None).unwrap();
            assert_eq!(game.tags_of(Target::Mob(old_id)), ["subject", from]);
            let source = game.mobs.iter().find(|m| m.id == old_id).unwrap().clone();
            let new_id = game
                .convert_to(
                    &source,
                    &format!("minecraft:{to}"),
                    horizon_client::mobs::conversion::Params::KEEP_ALL,
                    |_, _| {},
                )
                .unwrap();
            assert_ne!(old_id, new_id);
            assert_eq!(
                game.tags_of(Target::Mob(new_id)),
                ["subject", from],
                "converted ID must keep tags"
            );
            let index = game.mobs.iter().position(|m| m.id == new_id).unwrap();
            let saved = game.mob_tree_nbt(index);
            assert_eq!(saved.get("Tags"), Some(&tags));
            assert_eq!(
                horizon_client::entity_nbt::kind(&saved).as_deref(),
                Some(format!("minecraft:{to}").as_str())
            );
            assert_ne!(horizon_client::entity_nbt::uuid(&saved), Some(uuid));
            assert!(saved.get("horizon:old_entity_only").is_none());
            game.mobs.remove(index);
            let reloaded = game.load_mob(saved, None).unwrap();
            assert_eq!(game.tags_of(Target::Mob(reloaded)), ["subject", from]);
        }
    }
}

#[cfg(test)]
mod pickable_projectile_tests {
    use super::{pickable_projectile_hit, pickable_projectile_hitboxes};
    use horizon_client::mobs::Who;
    use horizon_client::projectiles::{Hit, Projectile, ProjectileKind, ProjectileWorld};
    use horizon_core::geometry::{Aabb, Vec3};

    /// Open air with nothing in it but `.0`.
    struct Open(Vec<(Who, Aabb)>);
    impl ProjectileWorld for Open {
        fn clip(&self, _: Vec3, _: Vec3) -> Option<(Vec3, [i32; 3], [i32; 3])> {
            None
        }
        fn in_water(&self, _: Aabb) -> bool {
            false
        }
        fn targets(&self) -> &[(Who, Aabb)] {
            &self.0
        }
    }

    /// Which of `list` another projectile (the one at 0) can strike.
    fn struck(list: &[Projectile]) -> Vec<usize> {
        let alive = vec![true; list.len()];
        pickable_projectile_hitboxes(list, &alive, 0)
            .iter()
            .map(|(who, _)| match who {
                Who::Mob(id) => pickable_projectile_hit(*id).expect("a pickable projectile id"),
                Who::Player => panic!("not a projectile"),
            })
            .collect()
    }

    /// `Projectile.isPickable` is `#minecraft:redirectable_projectile`: a ghast's
    /// fireball is in it, as both wind charges are, so another projectile's hit
    /// test can strike it. A blaze's small fireball is not, and a projectile
    /// that is stuck is no target.
    #[test]
    fn a_ghast_fireball_is_pickable_like_a_wind_charge() {
        let at = Vec3::new(0.0, 100.0, 0.0);
        let fired = |kind| Projectile::fireball(kind, at, None, [1.0, 0.0, 0.0]);
        let mut stuck = fired(ProjectileKind::LargeFireball { power: 1 });
        stuck.stuck = true;
        let list = vec![
            Projectile::new(ProjectileKind::EnderPearl, at, Some(Who::Player)),
            fired(ProjectileKind::LargeFireball { power: 1 }),
            fired(ProjectileKind::WindCharge),
            fired(ProjectileKind::BreezeWindCharge),
            fired(ProjectileKind::SmallFireball),
            stuck,
        ];
        assert_eq!(struck(&list), vec![1, 2, 3]);
    }

    /// A pearl thrown through a ghast's fireball strikes it (`Projectile.onHit`
    /// then turns the fireball), where it used to fly on through.
    #[test]
    fn a_pearl_thrown_through_a_ghast_fireball_strikes_it() {
        let mut pearl = Projectile::new(
            ProjectileKind::EnderPearl,
            Vec3::new(0.0, 100.0, 0.0),
            Some(Who::Player),
        );
        pearl.velocity = Vec3::new(1.5, 0.0, 0.0);
        let fireball = Projectile::fireball(
            ProjectileKind::LargeFireball { power: 1 },
            Vec3::new(1.0, 99.5, 0.0),
            None,
            [0.0, 0.0, 1.0],
        );
        let list = vec![pearl, fireball];
        let alive = vec![true; list.len()];
        let world = Open(pickable_projectile_hitboxes(&list, &alive, 0));
        let mut pearl = list[0].clone();
        let (keep, hit) = pearl.tick(&world);
        assert!(!keep, "the pearl is spent on the hit");
        match hit {
            Hit::Entity {
                who: Who::Mob(id), ..
            } => assert_eq!(
                pickable_projectile_hit(id),
                Some(1),
                "it struck the fireball"
            ),
            other => panic!("the pearl flew through the fireball: {other:?}"),
        }
    }
}

#[cfg(test)]
mod mob_pearl_hook_tests {
    use super::*;
    use crate::game::native_entity_refs::tests::with_game;
    use mod_api::{
        EnderPearlTeleportEvent, EntityTeleportEvent, Flow, Hooks, ModEntry, ServerHost,
    };
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicU8, Ordering},
    };

    struct PearlProbe {
        mode: Arc<AtomicU8>,
        owners: Arc<Mutex<Vec<u32>>>,
        command_teleports: Arc<Mutex<Vec<u32>>>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for PearlProbe {
        fn entity_teleport(&self, _: &mut dyn ServerHost, event: &mut EntityTeleportEvent<'_>) {
            self.command_teleports.lock().unwrap().push(event.entity);
        }

        fn ender_pearl_teleport(
            &self,
            host: &mut dyn ServerHost,
            event: &mut EnderPearlTeleportEvent<'_>,
        ) -> Flow {
            self.owners.lock().unwrap().push(event.owner.id());
            assert_eq!(event.owner.position(host), Some(event.from));
            assert_eq!(
                event.damage, 0.0,
                "non-player owners take no vanilla fall damage"
            );
            match self.mode.load(Ordering::SeqCst) {
                0 => Flow::Cancel,
                1 => {
                    *event.target = [12.0, 80.0, -4.0];
                    Flow::Continue
                }
                2 => {
                    assert!(event.owner.teleport_to(host, [3.0, 70.0, 3.0], None));
                    *event.target = [12.0, 80.0, -4.0];
                    Flow::Continue
                }
                3 => {
                    assert!(event.owner.discard(host));
                    Flow::Continue
                }
                _ => unreachable!(),
            }
        }
    }

    struct PlayerPearlProbe(Arc<AtomicU8>);
    #[mod_api::__subscriptions]
    impl Hooks for PlayerPearlProbe {
        fn ender_pearl_teleport(
            &self,
            host: &mut dyn ServerHost,
            event: &mut EnderPearlTeleportEvent<'_>,
        ) -> Flow {
            assert_eq!(
                host.players().acting_player().unwrap().entity(),
                event.owner
            );
            assert_eq!(event.damage, 5.0);
            // What the pearl hit: the block and its face, no entity.
            assert_eq!(event.hit_entity, None);
            assert_eq!(
                (event.hit_block, event.hit_face, event.hit_at),
                ([6, 75, 6], [0, 1, 0], [6.0, 75.0, 6.0])
            );
            match self.0.load(Ordering::SeqCst) {
                0 => return Flow::Cancel,
                2 => assert!(event.owner.teleport_to(host, [3.0, 70.0, 3.0], None)),
                _ => {}
            }
            *event.target = [12.0, 80.0, -4.0];
            event.damage = 0.0;
            Flow::Continue
        }
    }

    #[test]
    #[ignore = "needs prepared resources and an isolated native-mod process"]
    fn player_pearl_keeps_acting_context_cancellation_and_mutable_damage() {
        use horizon_client::motion::PlayerMotion;
        let mode = Arc::new(AtomicU8::new(0));
        let mods: mod_api::Mods = vec![ModEntry::new(
            "player-pearl-proof",
            Box::new(PlayerPearlProbe(Arc::clone(&mode))),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                let from = Vec3::new(0.0, 65.0, 0.0);
                game.player = Some(PlayerMotion::new(game.motion, from).unwrap());
                game.lan_acting = Some(12);
                game.lan_acting_uuid = Some(7);
                crate::game::native_entity_refs::tests::bind_local_player(game, 2, 7);
                game.lan_acting_name = Some("PearlTest".into());
                game.lan_acting_permission = Some(4);
                game.lan_acting_entity = Some(2);
                crate::game::native_entity_refs::tests::bind_acting_guest(game);
                game.ui.difficulty = 0;
                let pearl = Projectile::new(
                    ProjectileKind::EnderPearl,
                    Vec3::new(6.0, 75.0, 6.0),
                    Some(Who::Player),
                );
                let hit = || Hit::Block {
                    pos: [6, 75, 6],
                    at: pearl.position,
                    face: [0, 1, 0],
                };
                let mut expected_random = game.world.random.clone();
                let health = game.player.as_ref().unwrap().health.health();
                game.projectile_hit(&pearl, hit()).unwrap();
                assert_eq!(game.player.as_ref().unwrap().position, from);
                assert_eq!(game.player.as_ref().unwrap().health.health(), health);
                assert_eq!(game.world.random.next_long(), expected_random.next_long());
                mode.store(1, Ordering::SeqCst);
                expected_random = game.world.random.clone();
                expected_random.next_float();
                game.projectile_hit(&pearl, hit()).unwrap();
                let player = game.player.as_ref().unwrap();
                assert_eq!(player.position, Vec3::new(12.0, 80.0, -4.0));
                assert_eq!(
                    player.health.health(),
                    health,
                    "listener suppressed player fall damage"
                );
                assert_eq!(player.fall_distance, 0.0);
                assert_eq!(game.world.random.next_long(), expected_random.next_long());
                mode.store(2, Ordering::SeqCst);
                expected_random = game.world.random.clone();
                game.projectile_hit(&pearl, hit()).unwrap();
                let player = game.player.as_ref().unwrap();
                assert_eq!(
                    player.position,
                    Vec3::new(3.0, 70.0, 3.0),
                    "a listener's direct move is not overwritten by the pearl target"
                );
                assert_eq!(player.health.health(), health);
                assert_eq!(
                    game.world.random.next_long(),
                    expected_random.next_long(),
                    "a stale player move makes no endermite draw"
                );
            })
        });
    }

    #[test]
    #[ignore = "needs prepared resources and an isolated native-mod process"]
    fn mob_pearls_use_the_hook_and_recheck_listener_writes_without_player_side_effects() {
        let mode = Arc::new(AtomicU8::new(0));
        let owners = Arc::new(Mutex::new(Vec::new()));
        let command_teleports = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![ModEntry::new(
            "mob-pearl-proof",
            Box::new(PearlProbe {
                mode: Arc::clone(&mode),
                owners: Arc::clone(&owners),
                command_teleports: Arc::clone(&command_teleports),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                for action in 0..4 {
                    mode.store(action, Ordering::SeqCst);
                    let id = 40 + u32::from(action);
                    let from = Vec3::new(0.0, 65.0, 0.0);
                    let ty = game.res.mob_types().get("minecraft:cow").unwrap().clone();
                    let mut mob = Mob::new(id, ty, from, 37.0);
                    mob.head_pitch = -12.0;
                    mob.previous_yaw = -10.0;
                    mob.previous_head_pitch = 20.0;
                    mob.velocity = Vec3::new(0.5, -0.25, 0.75);
                    mob.fall_distance = 9.0;
                    let health = mob.health;
                    game.mobs.push(mob);
                    let mut pearl = Projectile::new(
                        ProjectileKind::EnderPearl,
                        Vec3::new(6.0, 75.0, 6.0),
                        Some(Who::Mob(id)),
                    );
                    pearl.id = 100 + id;
                    let mut expected_random = game.world.random.clone();
                    game.projectile_hit(
                        &pearl,
                        Hit::Block {
                            pos: [6, 75, 6],
                            at: pearl.position,
                            face: [0; 3],
                        },
                    )
                    .unwrap();
                    assert_eq!(
                        game.world.random.next_long(),
                        expected_random.next_long(),
                        "mob pearl handling makes no player endermite draw"
                    );
                    let mob = game.mobs.iter().find(|m| m.id == id && !m.removed);
                    match action {
                        0 => {
                            let m = mob.unwrap();
                            assert_eq!(m.position, from);
                            assert_eq!(m.fall_distance, 9.0);
                        }
                        1 => {
                            let m = mob.unwrap();
                            assert_eq!(m.position, Vec3::new(12.0, 80.0, -4.0));
                            assert_eq!(m.velocity, Vec3::ZERO);
                            assert_eq!(m.fall_distance, 0.0);
                            assert_eq!((m.yaw, m.head_pitch), (37.0, -12.0));
                            assert_eq!((m.previous_yaw, m.previous_head_pitch), (37.0, -12.0));
                            assert_eq!(m.health, health);
                        }
                        2 => assert_eq!(mob.unwrap().position, Vec3::new(3.0, 70.0, 3.0)),
                        3 => assert!(mob.is_none()),
                        _ => unreachable!(),
                    }
                    assert!(!game.mobs.iter().any(|m| m.species() == Species::Endermite));
                    assert_eq!(
                        *command_teleports.lock().unwrap(),
                        if action < 2 { vec![] } else { vec![42] },
                        "only the listener's explicit teleport emits the command notification"
                    );
                }
                assert_eq!(*owners.lock().unwrap(), [40, 41, 42, 43]);
            })
        });
    }
}

#[cfg(test)]
mod breeding_orb_tests {
    use super::*;

    /// `Animal.finalizeSpawnChildFromBreeding`: one fresh orb of the parent's
    /// 1-to-7 roll, never split (4 is not 3 + 1) or merged (#11388).
    #[test]
    #[ignore = "needs prepared resources (HORIZON_TEST_RESOURCES)"]
    fn breeding_drops_one_orb_of_the_whole_roll() {
        super::super::native_entity_refs::tests::with_game(|game| {
            let kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
            game.mobs
                .push(Mob::new(7, kind, Vec3::new(8.5, 64.0, 8.5), 0.0));
            let mut values = std::collections::BTreeSet::new();
            for _ in 0..40 {
                let roll = 1 + game.mobs[0].random.clone().next_int_bound(7).unwrap() as u32;
                let orbs = game.orbs.len();
                game.award_breeding_experience(7, [8.5, 64.0, 8.5]);
                assert_eq!(game.orbs.len(), orbs + 1, "a fresh orb, not merged");
                assert_eq!(
                    game.orbs.last().map(|o| (o.points(), o.count)),
                    Some((roll, 1))
                );
                values.insert(roll);
            }
            assert!(values.iter().any(|v| matches!(v, 4..=6)), "{values:?}");
        });
    }
}

#[cfg(test)]
mod enchanted_block_loot_breaker_tests {
    use super::super::native_entity_refs::tests::with_game;
    use super::{MobEvent, Projectile, ProjectileKind};
    use horizon_core::geometry::Vec3;
    use mod_api::{EnchantedBlockLootEvent, Hooks, LevelHost, ModEntry};
    use mod_platform::testing::with_mods;
    use std::sync::{Arc, Mutex};

    const POS: [i32; 3] = [1, 64, 1];

    /// Records the entity of every read.
    struct Heard(Arc<Mutex<Vec<Option<u32>>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Heard {
        fn enchanted_block_loot(&self, _: &dyn LevelHost, event: &mut EnchantedBlockLootEvent<'_>) {
            self.0.lock().unwrap().push(event.entity.map(|e| e.id()));
        }
    }

    fn place(game: &mut super::Game<'_>, block: &str) {
        let state = game.pack.assets.registry.default_state(block).unwrap();
        game.world.set(POS[0], POS[1], POS[2], state).unwrap();
    }
    fn projectile(kind: ProjectileKind, id: u32) -> Projectile {
        let mut projectile = Projectile::new(kind, Vec3::ZERO, None);
        projectile.id = id;
        projectile
    }

    /// `destroyBlock(pos, true, entity)` reaches the listener as `getEntity`:
    /// the projectile on a chorus flower or a decorated pot, the villager or
    /// ravager on a crop. A speleothem's `destroyBlock(pos, true)` names
    /// nobody. Vanilla's tables for those three read no level, so a data pack
    /// makes them read one.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES and an isolated process for native mods"]
    fn the_entity_destroy_block_names_reaches_the_listener() {
        let root = std::env::temp_dir().join(format!("block-loot-{}", std::process::id()));
        let dir = root.join("data/minecraft/loot_table/blocks");
        std::fs::create_dir_all(&dir).unwrap();
        let table = r#"{"pools":[{"rolls":1,"conditions":[{"type":"minecraft:table_bonus",
            "enchantment":"minecraft:fortune","chances":[1.0]}],
            "entries":[{"type":"minecraft:item","name":"minecraft:stick"}]}]}"#;
        for block in [
            "chorus_flower",
            "decorated_pot",
            "pointed_dripstone",
            "wheat",
        ] {
            std::fs::write(dir.join(format!("{block}.json")), table).unwrap();
        }
        let tables =
            horizon_world::loot::shared_tables(&root, &horizon_world::loot::PackOverlay::NONE)
                .unwrap();
        let _ = std::fs::remove_dir_all(&root);
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
            let mut items = game.items().clone();
            items.set_loot_tables(tables);
            game.server_items = Some(Arc::new(items));
            let heard = Arc::new(Mutex::new(Vec::new()));
            let mods = vec![ModEntry::new("breaker", Box::new(Heard(heard.clone())))].into();
            with_mods(mods, || {
                let take = || std::mem::take(&mut *heard.lock().unwrap());

                place(game, "minecraft:chorus_flower");
                game.projectile_breaks_block(&projectile(ProjectileKind::Snowball, 77), POS);
                assert_eq!(take(), [Some(77)]);

                place(game, "minecraft:decorated_pot");
                game.projectile_breaks_block(&projectile(ProjectileKind::Egg, 78), POS);
                assert_eq!(take(), [Some(78)]);

                place(game, "minecraft:pointed_dripstone");
                let mut trident = projectile(ProjectileKind::Trident, 79);
                trident.arrow.as_mut().unwrap().hit_speed = 1.0;
                game.projectile_breaks_block(&trident, POS);
                assert_eq!(take(), [None]);

                place(game, "minecraft:wheat");
                let harvest = MobEvent::BreakBlock {
                    pos: POS,
                    drop: true,
                    by: None,
                    breaker: Some(80),
                };
                game.handle_mob_events(vec![harvest]).unwrap();
                assert_eq!(take(), [Some(80)]);
            });
        });
    }
}

#[cfg(test)]
mod chorus_projectile_drop_tests {
    use super::*;
    use crate::game::{Audio, Options, Progress, Settings, load_resources};
    use horizon_client::{chunk::Chunk, motion::PlayerMotion};
    use horizon_core::position::ChunkPos;
    use std::sync::Arc;
    fn with_game(test: impl FnOnce(&mut Game<'_>)) {
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
        options.game_dir = Some(
            std::path::PathBuf::from(std::env::var_os("HORIZON_HOME").unwrap())
                .join(format!("h116-{:?}", std::thread::current().id())),
        );
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
        game.native_lifecycle
            .begin_open(mod_api::Authority::IntegratedServer);
        game.build_registries();
        game.set_allow_commands(true);

        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.block_ticks
            .add_empty_chunk(ChunkPos::new(0, 0))
            .unwrap();
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(2.5, 200.0, 2.5)).unwrap());
        game.creative = true;
        assert!(game.run_command("forceload add 0 0"));
        assert!(game.run_command("tick freeze"));
        assert!(game.run_command("fill 0 198 0 15 199 15 stone"));
        test(&mut game);
    }

    fn source(game: &mut Game<'_>, x: i32, z: i32, block: &str) {
        assert!(game.world.chunks.contains_key(&(0, 0)));
        assert!(game.run_command(&format!("setblock {x} 199 {z} end_stone strict")));
        assert!(game.run_command(&format!("setblock {x} 200 {z} {block} strict")));
        assert!(game.run_command(&format!("execute if block {x} 199 {z} end_stone")));
    }
    fn launch(game: &mut Game<'_>, kind: &str, n: i32, x: i32, z: i32, owner: bool) -> u32 {
        let own = if owner { ",Owner:[I;116,0,0,99]" } else { "" };
        assert!(game.run_command(&format!("summon {kind} {x}.5 202.5 {z}.5 {{UUID:[I;116,0,0,{n}],NoGravity:1b,Motion:[0d,-2d,0d]{own}}}")));
        let p = game
            .creatures
            .projectiles
            .iter()
            .find(|p| game.persist.uuid_of(p.id) == Some([116, 0, 0, n]))
            .unwrap();
        assert_eq!(p.kind.id(), format!("minecraft:{kind}"));
        assert_eq!(p.position, Vec3::new(x as f64 + 0.5, 202.5, z as f64 + 0.5));
        assert_eq!(p.velocity, Vec3::new(0.0, -2.0, 0.0));
        assert!(p.no_gravity);
        assert_eq!(p.owner_uuid, owner.then_some([116, 0, 0, 99]));
        if owner {
            assert!(matches!(p.owner, Some(Who::Mob(_))));
        }
        p.id
    }
    fn dropped(game: &Game<'_>, item: &str) {
        assert_eq!(game.dropped.len(), 1, "actual unfiltered item collection");
        assert_eq!(&*game.dropped[0].stack.item, item);
        assert_eq!(game.dropped[0].stack.count, 1);
    }
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_registered_arrow_collision_keeps_the_actual_projectile_for_chorus_loot() {
        with_game(|game| {
            source(game, 4, 4, "chorus_flower");
            assert!(game.dropped.is_empty());
            let id = launch(game, "arrow", 1, 4, 4, false);
            game.tick_projectiles().unwrap();
            assert!(
                game.run_command("execute if block 4 200 4 air"),
                "actual collision broke the source"
            );
            assert!(
                game.creatures
                    .projectiles
                    .iter()
                    .find(|p| p.id == id)
                    .unwrap()
                    .arrow
                    .as_ref()
                    .unwrap()
                    .in_ground
            );
            dropped(game, "minecraft:chorus_flower");
        });
    }
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_registered_snowball_collision_keeps_the_actual_projectile_for_chorus_loot() {
        with_game(|game| {
            source(game, 4, 4, "chorus_flower");
            let id = launch(game, "snowball", 2, 4, 4, false);
            game.tick_projectiles().unwrap();
            assert!(game.run_command("execute if block 4 200 4 air"));
            assert!(!game.creatures.projectiles.iter().any(|p| p.id == id));
            dropped(game, "minecraft:chorus_flower");
        });
    }
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn denied_hits_other_blocks_and_direct_destroy_keep_their_drop_contracts() {
        with_game(|game| {
            source(game, 4, 4, "chorus_flower");
            assert!(game.run_command("gamerule projectiles_can_break_blocks false"));
            launch(game, "arrow", 3, 4, 4, false);
            game.tick_projectiles().unwrap();
            assert!(game.run_command("execute if block 4 200 4 chorus_flower"));
            assert!(game.dropped.is_empty());
            assert!(game.run_command("gamerule projectiles_can_break_blocks true"));
            assert!(
                game.ui
                    .rules
                    .bool_at(horizon_client::gamerules::vanilla::MOB_GRIEFING)
            );
            assert!(game.run_command("gamerule mob_griefing"));
            assert!(game.run_command(
                "summon pig 12.5 200 12.5 {UUID:[I;116,0,0,99],NoAI:1b,NoGravity:1b,Health:10f}"
            ));
            source(game, 8, 4, "chorus_flower");
            launch(game, "arrow", 4, 8, 4, true);
            game.tick_projectiles().unwrap();
            assert!(game.run_command("execute if block 8 200 4 chorus_flower"));
            assert!(game.dropped.is_empty());
            source(game, 12, 4, "chorus_flower");
            launch(game, "ender_pearl", 5, 12, 4, false);
            game.tick_projectiles().unwrap();
            assert!(game.run_command("execute if block 12 200 4 chorus_flower"));
            assert!(game.dropped.is_empty());
            source(game, 4, 8, "stone");
            launch(game, "arrow", 6, 4, 8, false);
            game.tick_projectiles().unwrap();
            assert!(game.run_command("execute if block 4 200 8 stone"));
            assert!(game.dropped.is_empty());
            assert!(game.run_command("gamerule block_drops false"));
            source(game, 8, 8, "chorus_flower");
            launch(game, "snowball", 7, 8, 8, false);
            game.tick_projectiles().unwrap();
            assert!(game.run_command("execute if block 8 200 8 air"));
            assert!(game.dropped.is_empty());
            assert!(game.run_command("gamerule block_drops true"));
            source(game, 4, 12, "chorus_flower");
            assert!(game.run_command("setblock 4 200 12 air destroy"));
            assert!(game.dropped.is_empty());
            source(game, 8, 12, "dirt");
            assert!(game.run_command("setblock 8 200 12 air destroy"));
            dropped(game, "minecraft:dirt");
        });
    }
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn default_drop_rng_continues_identically_with_table_sequence_or_level_fallback() {
        for named in [false, true] {
            with_game(|game| {
                let root =
                    std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
                let mut table: serde_json::Value = serde_json::from_slice(
                    &std::fs::read(root.join("data/minecraft/loot_table/blocks/gravel.json"))
                        .unwrap(),
                )
                .unwrap();
                if !named {
                    table.as_object_mut().unwrap().remove("random_sequence");
                }
                let files = vec![(
                    "loot_table/minecraft:blocks/gravel".to_owned(),
                    vec![serde_json::to_vec(&table).unwrap()],
                )];
                let registry = game.pack.assets.registry;
                // The level callback requires its catalog for the world lifetime.
                // These two fixture catalogs live only for this test process.
                let items: &'static horizon_client::items::Items = Box::leak(Box::new(
                    horizon_client::items::Items::load(
                        &root,
                        registry,
                        game.res.facts(),
                        &horizon_world::loot::PackOverlay {
                            files: &files,
                            conditions: &horizon_world::conditions::LoadConditions::default(),
                        },
                        game.res.item_defaults(),
                    )
                    .unwrap(),
                ));
                let state = registry.default_state("minecraft:gravel").unwrap();
                assert_eq!(
                    items
                        .block_loot_table(registry, state)
                        .and_then(|id| items.loot_sequence(&id))
                        .is_some(),
                    named
                );
                source(game, 4, 4, "gravel");
                game.level.seed = 116;
                game.world.random.set_seed(116);
                game.rand_sequences = Default::default();
                game.with_level(|b, level, ticks| {
                    level.items = items;
                    assert!(b.destroy_block(level, ticks, BlockPos::new(4, 200, 4), true));
                });
                game.process_level_events().unwrap();
                assert_eq!(game.dropped.len(), 1);
                let pool = game.rand_sequences.to_nbt();
                let sequence_words = match pool
                    .get("sequences")
                    .and_then(|v| v.get("minecraft:blocks/gravel"))
                    .and_then(|v| v.get("source"))
                {
                    Some(horizon_core::nbt::Tag::LongArray(v)) => v.clone(),
                    _ => vec![],
                };
                assert_eq!(
                    !sequence_words.is_empty(),
                    named,
                    "actual selected random source"
                );
                let mut random = game.world.random.clone();
                let continuation: Vec<i64> = (0..4).map(|_| random.next_long()).collect();
                let drops: Vec<_> = game
                    .dropped
                    .iter()
                    .map(|d| (d.stack.item.to_string(), d.stack.count))
                    .collect();
                println!(
                    "h116-continuation {}",
                    serde_json::json!({"named":named,"level_continuation":continuation,"sequence_words":sequence_words,"drops":drops})
                );
            });
        }
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn breaker_context_is_target_only_and_does_not_escape_its_level_call() {
        with_game(|game| {
            source(game, 4, 4, "chorus_flower");
            source(game, 8, 4, "chorus_flower");
            source(game, 12, 4, "chorus_flower");
            let target = BlockPos::new(4, 200, 4);
            let neighbor = BlockPos::new(8, 200, 4);
            let state = game.world.block(4, 200, 4);
            // Actual loaded states and callback/evaluator; this is an isolation
            // unit, beside the registered projectile collision regressions.
            game.with_level(|_, level, _| {
                assert!(level.drop_breaker_at.is_none());
                level.drop_breaker_at = Some(target);
                level.drop_resources_now(neighbor, state);
                level.drop_resources_now(target, state);
            });
            dropped(game, "minecraft:chorus_flower");
            game.with_level(|_, level, _| assert!(level.drop_breaker_at.is_none()));
            for fail in [false, true] {
                let outcome: std::result::Result<(), ()> = game.with_level(|_, level, _| {
                    level.drop_breaker_at = Some(target);
                    if fail {
                        return Err(());
                    }
                    Ok(())
                });
                assert_eq!(outcome.is_err(), fail);
                game.with_level(|_, level, _| assert!(level.drop_breaker_at.is_none()));
            }
            // The same position on a later no-entity destruction cannot reuse
            // the former callback's presence. Nor can its neighbor.
            game.destroy_block_dropping([4, 200, 4]).unwrap();
            game.destroy_block_dropping([8, 200, 4]).unwrap();
            dropped(game, "minecraft:chorus_flower");
            let kept = game.with_level(|b, level, ticks| {
                assert!(level.drop_breaker_at.is_none());
                level.drop_tool = Some(((12, 200, 4), None));
                level.drop_into = Some(Vec::new());
                assert!(b.destroy_block(level, ticks, BlockPos::new(12, 200, 4), true));
                level.drop_into.take().unwrap()
            });
            assert!(kept.is_empty(), "machine's no-entity loot stays empty");
            dropped(game, "minecraft:chorus_flower");
        });
    }
}

#[cfg(test)]
mod loaded_arrow_shot_tests {
    use crate::game::{Audio, Game, Options, Progress, Settings, load_resources};
    use horizon_client::{
        chunk::Chunk,
        mobs::Who,
        motion::PlayerMotion,
        projectiles::{Projectile, ProjectileKind},
    };
    use horizon_core::{geometry::Vec3, position::ChunkPos, random::RandomSource};
    use std::sync::Arc;
    fn with_game(test: impl FnOnce(&mut Game<'_>)) {
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
        options.game_dir = Some(
            std::path::PathBuf::from(std::env::var_os("HORIZON_HOME").unwrap())
                .join(format!("h114-{:?}", std::thread::current().id())),
        );
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
        game.build_registries();
        game.set_allow_commands(true);

        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.block_ticks
            .add_empty_chunk(ChunkPos::new(0, 0))
            .unwrap();
        assert!(game.run_command("difficulty normal"));
        assert!(game.run_command("forceload add 0 0"));
        for (x, z) in [(0, 1), (1, 0), (1, 1)] {
            game.world
                .chunks
                .insert((x, z), Arc::new(Chunk::empty(x, z)));
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(x, z))
                .unwrap();
        }
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(2.5, 200.0, 2.5)).unwrap());
        game.creative = true;
        assert!(game.run_command("fill 0 199 0 31 199 31 stone"));
        assert!(
            game.run_command("summon pig 13.5 200 2.5 {UUID:[I;114,0,0,99],NoAI:1b,NoGravity:1b}")
        );
        game.world.random.set_seed(114);
        test(&mut game);
    }
    fn start_events() {
        crate::game::sculk::TEST_POSTED_EVENTS.with(|v| *v.borrow_mut() = Some(Vec::new()));
    }
    fn take_shots() -> Vec<(&'static str, [f64; 3], Option<Who>)> {
        crate::game::sculk::TEST_POSTED_EVENTS.with(|v| {
            std::mem::take(v.borrow_mut().as_mut().unwrap())
                .into_iter()
                .filter(|e| e.0 == "minecraft:projectile_shoot")
                .collect()
        })
    }
    fn stop_events() {
        crate::game::sculk::TEST_POSTED_EVENTS.with(|v| *v.borrow_mut() = None);
    }
    fn owner(game: &Game<'_>) -> u32 {
        game.mobs
            .iter()
            .find(|m| game.persist.uuid_of(m.id) == Some([114, 0, 0, 99]))
            .unwrap()
            .id
    }
    /// `projectile_shoot` on its first tick, once; one loaded shot posts none.
    /// `FireworkRocketEntity.tick`'s launch sound skips a silent one.
    #[test]
    #[ignore = "prepared resources; actual registered load, Game tick and posted events"]
    fn a_rocket_posts_its_shot_once_and_a_silent_one_launches_quietly() {
        with_game(|game| {
            let launch = "minecraft:entity.firework_rocket.launch";
            assert!(game.run_command(
                "summon firework_rocket 6.5 201.5 10.5 {LifeTime:100,Silent:1b,NoGravity:1b}"
            ));
            assert!(game.run_command(
                "summon firework_rocket 9.5 201.5 10.5 {LifeTime:100,HasBeenShot:1b}"
            ));
            assert!(
                game.rockets
                    .iter()
                    .all(|r| r.has_been_shot == (r.position.x == 9.5))
            );
            game.audio.heard.clear();
            start_events();
            game.tick_rockets();
            let first = take_shots();
            game.tick_rockets();
            let second = take_shots();
            stop_events();
            assert_eq!(
                first,
                vec![("minecraft:projectile_shoot", [6.5, 201.5, 10.5], None)]
            );
            assert!(second.is_empty());
            assert!(game.rockets.iter().all(|r| r.has_been_shot));
            // Only the rocket that isn't silent launches with a sound.
            assert_eq!(game.audio.heard.get(launch), Some(&1));
        });
    }
    /// #14440: an orb's move adds its drop to the fall outside water; landing
    /// after a fall posts HIT_GROUND and forgets the fall (`Entity.checkFallDamage`);
    /// in water the drop adds nothing, so a landing posts nothing.
    #[test]
    #[ignore = "prepared resources; actual registered load, Game tick and posted events"]
    fn a_landing_orb_posts_hit_ground_after_a_fall() {
        use horizon_client::entities::OrbMove;
        with_game(|game| {
            assert!(
                game.run_command("summon experience_orb 6.5 200 10.5 {Value:1,fall_distance:2d}")
            );
            let i = game.orbs.len() - 1;
            let from = Vec3::new(6.5, 200.5, 10.5);
            let down = Vec3::new(0.0, -0.5, 0.0);
            start_events();
            // In the air: the drop adds to the fall.
            game.orbs[i].on_ground = false;
            game.orb_moved(
                i,
                OrbMove {
                    from,
                    moved: down,
                    in_water: false,
                },
            );
            assert_eq!(game.orbs[i].fall_distance, 2.5);
            // On the ground: HIT_GROUND, then the fall is gone.
            game.orbs[i].on_ground = true;
            game.orb_moved(
                i,
                OrbMove {
                    from,
                    moved: down,
                    in_water: false,
                },
            );
            assert_eq!(game.orbs[i].fall_distance, 0.0);
            // In water with no fall: no drop, so no landing event.
            game.orb_moved(
                i,
                OrbMove {
                    from,
                    moved: down,
                    in_water: true,
                },
            );
            let events: Vec<_> = crate::game::sculk::TEST_POSTED_EVENTS.with(|v| {
                std::mem::take(v.borrow_mut().as_mut().unwrap())
                    .into_iter()
                    .filter(|e| e.0 == "minecraft:hit_ground")
                    .collect()
            });
            stop_events();
            assert_eq!(events.len(), 1);
            assert_eq!(game.orbs[i].fall_distance, 0.0);
        });
    }
    /// #14318: an angled rocket falling onto stone builds up its fall and posts
    /// HIT_GROUND when it lands (`Entity.checkFallDamage`); one falling into water
    /// splashes once (`Entity.doWaterSplashEffect`: the sound and SPLASH) and lands
    /// with no fall.
    #[test]
    #[ignore = "prepared resources; actual registered load, Game tick and posted events"]
    fn a_falling_rocket_hits_the_ground_and_one_in_water_splashes() {
        with_game(|game| {
            assert!(game.run_command("setblock 20 200 10 water"));
            for x in [6, 20] {
                assert!(game.run_command(&format!(
                "summon firework_rocket {x}.5 202 10.5 {{LifeTime:100,HasBeenShot:1b,ShotAtAngle:1b,Motion:[0d,-0.5d,0d]}}"
            )));
            }
            game.audio.heard.clear();
            start_events();
            let mut falls = Vec::new();
            for _ in 0..6 {
                game.tick_rockets();
                falls.push(game.rockets[0].fall_distance);
            }
            let events: Vec<_> = crate::game::sculk::TEST_POSTED_EVENTS.with(|v| {
                std::mem::take(v.borrow_mut().as_mut().unwrap())
                    .into_iter()
                    .map(|e| (e.0, e.1[0].floor() as i32))
                    .filter(|e| matches!(e.0, "minecraft:hit_ground" | "minecraft:splash"))
                    .collect()
            });
            stop_events();
            // The fourth move ends on the stone without a collision; the fifth lands.
            assert_eq!(falls, [0.5, 1.0, 1.5, 2.0, 0.0, 0.0]);
            assert_eq!(game.rockets[1].fall_distance, 0.0);
            assert_eq!(
                events,
                [("minecraft:splash", 20), ("minecraft:hit_ground", 6)]
            );
            assert_eq!(
                game.audio.heard.get("minecraft:entity.generic.splash"),
                Some(&1)
            );
        });
    }

    #[test]
    #[ignore = "prepared resources; actual registered load, Game tick and posted events"]
    fn loaded_arrow_first_tick_dispatches_once_with_its_real_owner() {
        with_game(|game| {
            let owner = owner(game);
            // Already-shot is an independent silent control before the old failure.
            for (n, flag) in [(1, Some(1)), (2, None), (3, Some(0))] {
                let extra = flag
                    .map(|f| format!(",HasBeenShot:{f}b"))
                    .unwrap_or_default();
                assert!(game.run_command(&format!("summon arrow 6.5 201.5 10.5 {{UUID:[I;114,0,0,{n}],NoGravity:1b,Motion:[0d,0d,0d],Owner:[I;114,0,0,99]{extra}}}")));
                let id = game
                    .creatures
                    .projectiles
                    .iter()
                    .find(|p| game.persist.uuid_of(p.id) == Some([114, 0, 0, n]))
                    .unwrap()
                    .id;
                let p = game
                    .creatures
                    .projectiles
                    .iter()
                    .find(|p| p.id == id)
                    .unwrap();
                assert_eq!(p.owner_uuid, Some([114, 0, 0, 99]));
                assert_eq!(p.arrow.as_ref().unwrap().has_been_shot, flag == Some(1));
                assert!(p.no_gravity);
                assert_eq!(p.velocity, Vec3::ZERO);
                start_events();
                game.tick_projectiles().unwrap();
                let p = game
                    .creatures
                    .projectiles
                    .iter()
                    .find(|p| p.id == id)
                    .unwrap();
                assert!(p.arrow.as_ref().unwrap().has_been_shot);
                assert_eq!(p.position, Vec3::new(6.5, 201.5, 10.5));
                assert_eq!(p.velocity, Vec3::ZERO);
                let first = take_shots();
                game.tick_projectiles().unwrap();
                let second = take_shots();
                stop_events();
                assert!(second.is_empty(), "no repeated shot on tick2");
                let expected = if flag == Some(1) {
                    Vec::new()
                } else {
                    vec![(
                        "minecraft:projectile_shoot",
                        [6.5, 201.5, 10.5],
                        Some(Who::Mob(owner)),
                    )]
                };
                assert_eq!(first, expected, "actual first tick, input={flag:?}");
                assert!(game.run_command(&format!(
                    "kill {}",
                    horizon_protocol::crypto::uuid_string(
                        horizon_client::lan::playerdata::ints_uuid([114, 0, 0, n])
                    )
                )));
            }
        });
    }
    fn natural(kind: &str) {
        with_game(|game| {
            let shooter = owner(game);
            match kind {
                "bow"=>{
                    assert!(game.run_command("item replace entity @s weapon.mainhand with bow"));
                    assert!(game.run_command("item replace entity @s hotbar.1 with arrow 4"));
                },
                "crossbow"=>assert!(game.run_command("item replace entity @s weapon.mainhand with crossbow[charged_projectiles=[{id:\"minecraft:arrow\",count:1}]]")),
                _=>{}
            }
            let clock = game.world.game_time;
            start_events();
            match kind {
                "bow" => {
                    let bow = game.held().unwrap().clone();
                    game.release_bow(&bow, 20);
                }
                "crossbow" => {
                    let bow = game.held().unwrap().clone();
                    assert!(game.use_crossbow(&bow));
                }
                "mob" => {
                    let mut p = Projectile::new(
                        ProjectileKind::Arrow {
                            base_damage: 2.0,
                            critical: false,
                            effect: None,
                        },
                        Vec3::new(6.5, 201.5, 6.5),
                        Some(Who::Mob(shooter)),
                    );
                    p.velocity = Vec3::new(0.0, 0.0, 1.0);
                    game.launch_mob_projectile(p);
                }
                _ => unreachable!(),
            }
            assert_eq!(game.creatures.projectiles.len(), 1, "real launch {kind}");
            let p = &game.creatures.projectiles[0];
            let id = p.id;
            assert!(id > 0);
            assert!(
                !p.arrow.as_ref().unwrap().has_been_shot,
                "saved pre-tick state"
            );
            let launch = take_shots();
            game.tick_projectiles().unwrap();
            let p = game
                .creatures
                .projectiles
                .iter()
                .find(|p| p.id == id)
                .unwrap();
            assert!(p.arrow.as_ref().unwrap().has_been_shot);
            let at = [p.position.x, p.position.y, p.position.z];
            let first = take_shots();
            game.tick_projectiles().unwrap();
            let second = take_shots();
            stop_events();
            assert_eq!(
                game.world.game_time, clock,
                "fixture directly invokes actual projectile ticks; runtime clocks independently admitted"
            );
            println!("H114_RNG {kind} {}", game.world.random.next_long());
            assert_eq!(
                [launch.len(), first.len(), second.len()],
                [0, 1, 0],
                "actual launch/tick1/tick2 counts {kind}"
            );
            assert_eq!(
                first[0],
                (
                    "minecraft:projectile_shoot",
                    at,
                    Some(if kind == "mob" {
                        Who::Mob(shooter)
                    } else {
                        Who::Player
                    })
                )
            );
        });
    }
    #[test]
    #[ignore = "prepared resources; actual bow release and Game ticks"]
    fn bow_launch_uses_only_the_first_tick_event() {
        natural("bow")
    }
    #[test]
    #[ignore = "prepared resources; actual charged crossbow use and Game ticks"]
    fn crossbow_launch_uses_only_the_first_tick_event() {
        natural("crossbow")
    }
    #[test]
    #[ignore = "prepared resources; actual Mob launch adapter and Game ticks"]
    fn mob_launch_uses_only_the_first_tick_event() {
        natural("mob")
    }
    #[test]
    #[ignore = "prepared resources; actual rocket and mob trident ticks"]
    fn rocket_and_mob_trident_post_only_on_their_first_tick() {
        with_game(|game| {
            assert!(game.run_command("item replace entity @s weapon.mainhand with crossbow[charged_projectiles=[{id:\"minecraft:firework_rocket\",count:1}]]"));
            start_events();
            let bow = game.held().unwrap().clone();
            assert!(game.use_crossbow(&bow));
            assert_eq!(game.rockets.len(), 1);
            assert!(take_shots().is_empty(), "no rocket shot before its tick");
            game.tick_rockets();
            assert_eq!(take_shots().len(), 1, "rocket first tick");
            game.tick_rockets();
            assert!(take_shots().is_empty(), "no repeated rocket shot");
            stop_events();
            for route in ["flight", "finish_hit", "breeze"] {
                if route == "breeze" {
                    assert!(game.run_command("summon breeze 6.5 201.5 7.5 {NoAI:1b,NoGravity:1b}"));
                }
                let mut p = Projectile::new(
                    ProjectileKind::Trident,
                    Vec3::new(6.5, 201.5, 6.5),
                    Some(Who::Mob(owner(game))),
                );
                p.velocity = Vec3::new(0.0, 0.0, 1.0);
                start_events();
                game.launch_mob_projectile(p);
                let id = game.creatures.projectiles.last().unwrap().id;
                assert!(
                    take_shots().is_empty(),
                    "no trident shot before its tick: {route}"
                );
                if route == "finish_hit" {
                    let p = game.creatures.projectiles.pop().unwrap();
                    game.finish_arrow_hit(p, super::EntityHit::Deflected);
                } else {
                    game.tick_projectiles().unwrap();
                }
                assert_eq!(take_shots().len(), 1, "trident first tick: {route}");
                if route == "breeze" {
                    let p = game
                        .creatures
                        .projectiles
                        .iter()
                        .find(|p| p.id == id)
                        .unwrap();
                    assert!(
                        p.velocity.z < 0.0,
                        "the actual breeze deflected the trident"
                    );
                }
                game.tick_projectiles().unwrap();
                assert!(take_shots().is_empty(), "no repeated trident shot: {route}");
                stop_events();
            }
        });
    }
    #[test]
    #[ignore = "prepared resources; real loaded owner and finish-hit source"]
    fn finish_hit_dispatches_once_and_the_next_tick_does_not_repeat_it() {
        with_game(|game| {
            assert!(game.run_command("summon arrow 6.5 201.5 10.5 {UUID:[I;114,0,0,20],NoGravity:1b,Motion:[0d,0d,0d],Owner:[I;114,0,0,99]}"));
            let mut arrow = game.creatures.projectiles.pop().unwrap();
            let owner = owner(game);
            assert_eq!(arrow.owner_uuid, Some([114, 0, 0, 99]));
            // Resolve through the actual registered owner, as tick_projectiles does.
            arrow.owner = game.who_by_uuid(arrow.owner_uuid.unwrap());
            assert_eq!(arrow.owner, Some(Who::Mob(owner)));
            assert!(!arrow.arrow.as_ref().unwrap().has_been_shot);
            let id = arrow.id;
            start_events();
            game.finish_arrow_hit(arrow, super::EntityHit::Deflected);
            let first = take_shots();
            assert!(
                game.creatures
                    .projectiles
                    .iter()
                    .find(|p| p.id == id)
                    .unwrap()
                    .arrow
                    .as_ref()
                    .unwrap()
                    .has_been_shot
            );
            game.tick_projectiles().unwrap();
            assert!(take_shots().is_empty());
            stop_events();
            assert_eq!(
                first,
                vec![(
                    "minecraft:projectile_shoot",
                    [6.5, 201.5, 10.5],
                    Some(Who::Mob(owner))
                )]
            );
        });
    }
    #[test]
    #[ignore = "prepared resources; registered Guest context and finish-hit source"]
    fn finish_hit_uses_registered_guest_context_and_restores_the_host() {
        use horizon_client::lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig};
        use horizon_protocol::packets::configuration::ClientInformation;
        use std::sync::{atomic::AtomicI32, mpsc};
        with_game(|game| {
            game.lan_host(
                crate::game::lan::LanSettings {
                    port: 0,
                    guest_commands: true,
                    force_game_mode: false,
                },
                NetConfig {
                    announce: false,
                    ..NetConfig::loopback(0, "arrow-test".into(), 2)
                },
                HubConfig {
                    default_game_mode: 1,
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
                    uuid: 114,
                    name: "ArrowGuest".into(),
                    properties: Vec::new(),
                    info: ClientInformation::new(2),
                    brand: "test".into(),
                    sender: PlayerSender::new(tx, Arc::default()),
                    latency: Arc::new(AtomicI32::new(0)),
                    address: "127.0.0.1:25565".parse().unwrap(),
                    expected: false,
                },
                2,
                [6.5, 200.0, 6.5],
                2,
            );
            guest.dimension = "minecraft:overworld".into();
            guest.placed = true;
            guest.loaded = true;
            guest.permission = 4;
            game.lan.as_mut().unwrap().hub.guests.push(guest);
            let host_uuid = game.acting_player_uuid();
            game.as_guest(2,|g| {
                assert_eq!(g.lan_acting,Some(2));
                assert_eq!(g.acting_player_uuid(),[0,0,0,114]);
                assert!(g.run_command("item replace entity @s weapon.mainhand with crossbow[charged_projectiles=[{id:\"minecraft:arrow\",count:1}]]"));
                let bow=g.held().unwrap().clone();assert!(g.use_crossbow(&bow));
            }).unwrap();
            assert_eq!(game.acting_player_uuid(), host_uuid);
            assert_eq!(game.lan_acting, None);
            let arrow = game.creatures.projectiles.pop().unwrap();
            assert_eq!(arrow.guest, Some(2));
            assert_eq!(arrow.owner, Some(Who::Player));
            assert_eq!(arrow.owner_uuid, Some([0, 0, 0, 114]));
            assert!(!arrow.arrow.as_ref().unwrap().has_been_shot);
            let at = [arrow.position.x, arrow.position.y, arrow.position.z];
            let id = arrow.id;
            start_events();
            game.finish_arrow_hit(arrow, super::EntityHit::Deflected);
            let first = take_shots();
            assert_eq!(game.acting_player_uuid(), host_uuid);
            assert_eq!(game.lan_acting, None);
            assert_eq!(game.lan.as_ref().unwrap().hub.guests[0].uuid, 114);
            assert!(
                game.creatures
                    .projectiles
                    .iter()
                    .find(|p| p.id == id)
                    .unwrap()
                    .arrow
                    .as_ref()
                    .unwrap()
                    .has_been_shot
            );
            game.tick_projectiles().unwrap();
            assert!(take_shots().is_empty());
            stop_events();
            assert_eq!(
                first,
                vec![("minecraft:projectile_shoot", at, Some(Who::Player))]
            );
        });
    }
}

#[cfg(test)]
mod loaded_mob_arrow_permission_tests {
    use super::*;
    use crate::game::native_entity_refs::tests::with_game;
    use horizon_client::{entity_nbt, save};
    use horizon_core::nbt::Tag;
    use horizon_core::position::BlockPos;

    fn parse(text: &str) -> Tag {
        let (tag, used) = horizon_core::nbt::parse_snbt(text).unwrap();
        assert_eq!(used, text.len());
        tag
    }

    #[test]
    #[ignore = "prepared resources; actual registered summon, collision, permission and drops"]
    fn loaded_mob_arrow_predicate_survives_model_and_controls_actual_block_hit() {
        with_game(|game| {
            game.build_registries();
            game.set_allow_commands(true);
            game.block_ticks
                .add_empty_chunk(BlockPos::new(0, 0, 0).chunk())
                .unwrap();
            // Supply the synthetic chunk before forceload's synchronous load barrier.
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
            );
            assert!(game.run_command("forceload add 0 0"));
            assert!(game.run_command("fill 0 199 0 15 199 15 stone"));
            assert!(game.world.loaded(3, 1));
            assert!(
                game.ui
                    .rules
                    .bool_at(horizon_client::gamerules::vanilla::PROJECTILES_CAN_BREAK_BLOCKS)
            );
            assert!(
                game.ui
                    .rules
                    .bool_at(horizon_client::gamerules::vanilla::BLOCK_DROPS)
            );
            assert!(game.run_command(
                "summon pig 14.5 200 14.5 {UUID:[I;197,0,0,1],NoAI:1b,NoGravity:1b,Health:10f}"
            ));
            let owner = game
                .mobs
                .iter()
                .find(|m| game.persist.uuid_of(m.id) == Some([197, 0, 0, 1]))
                .unwrap()
                .id;
            assert_eq!(
                game.mobs.iter().find(|m| m.id == owner).unwrap().health,
                10.0
            );
            let clock = game.world.game_time;
            let expected = parse("{blocks:\"minecraft:chorus_flower\"}");
            let stale = parse("{can_break:{blocks:\"minecraft:stone\"},marker:37}");
            let mut missing = Vec::new();
            for (n, predicate, griefing, breaks) in [
                (1, Some("chorus_flower"), true, true),
                (2, Some("stone"), true, false),
                (3, None, true, false),
                (4, Some("chorus_flower"), false, false),
                (5, Some("stone"), false, false),
                (6, None, false, false),
            ] {
                let x = if n <= 3 { 3 } else { 11 };
                let z = 1 + 4 * ((n - 1) % 3);
                if game
                    .ui
                    .rules
                    .bool_at(horizon_client::gamerules::vanilla::MOB_GRIEFING)
                    != griefing
                {
                    assert!(game.run_command(&format!("gamerule mob_griefing {griefing}")));
                }
                assert_eq!(
                    game.ui
                        .rules
                        .bool_at(horizon_client::gamerules::vanilla::MOB_GRIEFING),
                    griefing
                );
                assert!(game.run_command(&format!("setblock {x} 199 {z} end_stone")));
                assert!(game.run_command(&format!("setblock {x} 200 {z} chorus_flower[age=0]")));
                let field = predicate.map_or(String::new(), |p| {
                    format!(",can_break:{{blocks:\"minecraft:{p}\"}}")
                });
                assert!(game.run_command(&format!("summon arrow {} 200.5 {} {{UUID:[I;197,0,{n},2],NoGravity:1b,HasBeenShot:1b,Motion:[2d,0d,0d],Owner:[I;197,0,0,1],item:{{id:\"minecraft:arrow\",count:1}}{field}}}", f64::from(x)-1.5, f64::from(z)+0.5)));
                let p = game
                    .creatures
                    .projectiles
                    .iter()
                    .find(|p| game.persist.uuid_of(p.id) == Some([197, 0, n, 2]))
                    .unwrap()
                    .clone();
                assert_eq!(p.owner_uuid, Some([197, 0, 0, 1]));
                assert_eq!(
                    p.owner,
                    Some(Who::Mob(owner)),
                    "actual resolved loaded Mob, not raw Owner alone"
                );
                assert!(matches!(p.kind, ProjectileKind::Arrow { .. }));
                assert!(p.no_gravity);
                assert_eq!(p.velocity, Vec3::new(2.0, 0.0, 0.0));
                assert!(!p.arrow.as_ref().unwrap().in_ground);
                game.persist.raw.remove(&p.id);
                for (route, raw) in [("no-raw", None), ("stale-raw", Some(&stale))] {
                    let saved =
                        entity_nbt::arrow_to_nbt(&p, [197, 0, n, 2], p.owner_uuid, None, raw)
                            .unwrap();
                    let want = predicate.map(|id| parse(&format!("{{blocks:\"minecraft:{id}\"}}")));
                    if saved.get("can_break") != want.as_ref() {
                        missing.push(format!(
                            "{n} cloned loaded model {route}: {:?} != {want:?}",
                            saved.get("can_break")
                        ));
                    }
                    if raw.is_some() {
                        assert_eq!(save::number(saved.get("marker")), Some(37));
                    }
                }
                if n == 1 {
                    assert_eq!(game.persist.uuid_of(p.id), Some([197, 0, n, 2]));
                    // The expected value is the actual admitted input, never an injected model field.
                    assert_eq!(expected, parse("{blocks:\"minecraft:chorus_flower\"}"));
                }
                let drops_before = game.dropped.len();
                game.tick_projectiles().unwrap();
                let after = game
                    .creatures
                    .projectiles
                    .iter()
                    .find(|a| a.id == p.id)
                    .unwrap();
                assert!(
                    after.arrow.as_ref().unwrap().in_ground,
                    "actual air-to-ground block collision"
                );
                let block = &*game.res.facts().get(game.world.block(x, 200, z)).name;
                let want = if breaks {
                    "minecraft:air"
                } else {
                    "minecraft:chorus_flower"
                };
                if block != want {
                    missing.push(format!("{n} actual hit block {block} != {want}"));
                }
                let dropped: Vec<_> = game
                    .dropped
                    .iter()
                    .skip(drops_before)
                    .map(|e| (&*e.stack.item, e.stack.count, e.position, e.velocity))
                    .collect();
                println!("actual collision {n}: block={block}, unfiltered new drops={dropped:?}");
                if !breaks {
                    assert!(dropped.is_empty(), "denied hits cannot create drops");
                }
            }
            assert!(game.run_command("setblock 13 200 12 dirt"));
            let before = game.dropped.len();
            game.destroy_block_dropping([13, 200, 12]).unwrap();
            assert_eq!(
                &*game.res.facts().get(game.world.block(13, 200, 12)).name,
                "minecraft:air"
            );
            let direct: Vec<_> = game.dropped.iter().skip(before).collect();
            assert_eq!(
                direct.len(),
                1,
                "independent actual destruction/drop callback"
            );
            assert_eq!(&*direct[0].stack.item, "minecraft:dirt");
            assert_eq!(direct[0].stack.count, 1);
            assert_eq!(
                game.world.game_time, clock,
                "one projectile step per scene; no fabricated world ticks"
            );
            assert!(
                missing.is_empty(),
                "actual loaded Mob arrow model/permission mismatches: {missing:?}"
            );
        });
    }
}

#[cfg(test)]
mod h169_glide_travel_tests {
    use super::*;
    use crate::game::{Audio, Options, Progress, Settings, load_resources};
    use horizon_client::{chunk::Chunk, motion::PlayerMotion};
    use horizon_core::{geometry::Vec3, position::ChunkPos, random::RandomSource};
    use std::sync::Arc;
    fn with_game(test: impl FnOnce(&mut Game<'_>)) {
        with_role_game(
            crate::game::execution_role::ExecutionRole::IntegratedServer,
            test,
        );
    }
    fn with_role_game(
        role: crate::game::execution_role::ExecutionRole,
        test: impl FnOnce(&mut Game<'_>),
    ) {
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
        options.game_dir = Some(
            std::path::PathBuf::from(std::env::var_os("HORIZON_HOME").unwrap())
                .join(format!("h169-{:?}", std::thread::current().id())),
        );
        options.execution_role = role;
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
        game.build_registries();
        game.set_allow_commands(true);

        let mut center = Chunk::empty(0, 0);
        center.lit = true;
        game.world.chunks.insert((0, 0), Arc::new(center));
        game.block_ticks
            .add_empty_chunk(ChunkPos::new(0, 0))
            .unwrap();
        assert!(game.run_command("difficulty normal"));
        assert!(game.run_command("forceload add 0 0"));
        for x in -1..=1 {
            for z in -1..=1 {
                if x == 0 && z == 0 {
                    continue;
                }
                let mut chunk = Chunk::empty(x, z);
                chunk.lit = true;
                game.world.chunks.insert((x, z), Arc::new(chunk));
                game.block_ticks
                    .add_empty_chunk(ChunkPos::new(x, z))
                    .unwrap();
            }
        }
        game.load_ticking_neighbourhood(0, 0);
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(2.5, 200.0, 2.5)).unwrap());
        game.creative = true;
        assert!(game.run_command("fill 0 199 0 31 199 31 stone"));
        game.world.random.set_seed(169);
        game.update_chunk_tickets();
        assert!(crate::game::ticking_chunk(&game.world, 0, 0));
        assert!(game.tickets.levels().block_ticking(0, 0));
        test(&mut game);
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn h169_registered_eligible_husk_actually_travels_as_a_glider() {
        with_game(|game| {
            game.world.game_time = 0;
            for (n, no_ai, glide, x) in [(1, 0, 1, 4.5), (2, 1, 1, 8.5), (3, 0, 0, 12.5)] {
                assert!(game.run_command(&format!(
                    "summon husk {x} 300 8.5 {{UUID:[I;169,0,0,{n}],NoAI:{no_ai}b,PersistenceRequired:1b,FallFlying:{glide}b,Rotation:[0f,0f],Motion:[0d,0d,0d],equipment:{{chest:{{id:\"minecraft:elytra\",count:1}}}}}}"
                )));
            }
            let indexes: Vec<_> = (1..=3)
                .map(|n| {
                    let id = game
                        .persist
                        .uuids
                        .iter()
                        .find_map(|(id, u)| (*u == [169, 0, 0, n]).then_some(*id))
                        .unwrap();
                    game.mobs.iter().position(|m| m.id == id).unwrap()
                })
                .collect();
            let main = indexes[0];
            let id = game.mobs[main].id;
            assert!(game.mobs[main].extra.fall_flying);
            assert!(!game.mobs[main].no_ai && !game.mobs[main].on_ground);
            let stack = horizon_client::mobs::finalize::equipment_stack(
                &game.mobs[main],
                horizon_core::equipment_slot::EquipmentSlot::Chest,
            )
            .unwrap();
            assert!(horizon_client::item_sync::can_glide_using(
                &stack,
                "chest",
                game.items()
            ));
            game.world.game_time += 1;
            game.update_chunk_tickets();
            assert!(game.entity_ticking_area()(4, 8));
            game.tick_creature_entities().unwrap();
            assert_eq!(game.mobs[main].id, id);
            assert_eq!(game.uuid_for(id), [169, 0, 0, 1]);
            assert!(
                game.mobs[main].extra.fall_flying,
                "real modeled flag remains eligible after the tick"
            );
            assert_eq!(
                game.mobs[indexes[1]].position,
                Vec3::new(8.5, 300.0, 8.5),
                "NoAI is a stationary control"
            );
            assert!(!game.mobs[indexes[2]].extra.fall_flying);
            assert_eq!(game.world.game_time, 1);
            // Independent fresh Java first-tick values, not native helper output.
            assert_eq!(
                game.mobs[main].position,
                Vec3::new(4.5, 299.9823599996567, 8.501782000017165)
            );
            assert_eq!(
                game.mobs[main].velocity,
                Vec3::new(0.0, -0.017640000343322755, 0.0017820000171661376)
            );
        });
    }

    // Added source/unit control, not part of the frozen dry-air Java runtime pair.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn h169_added_actual_wall_collision_uses_mob_damage_source() {
        with_game(|game| {
            assert!(game.run_command("fill 6 298 7 6 303 10 stone"));
            assert!(game.run_command(r#"summon husk 4.5 300 8.5 {UUID:[I;169,0,0,4],PersistenceRequired:1b,FallFlying:1b,Rotation:[-90f,0f],Motion:[2d,0d,0d],equipment:{chest:{id:"minecraft:elytra",count:1}}}"#));
            let id = game
                .persist
                .uuids
                .iter()
                .find_map(|(id, u)| (*u == [169, 0, 0, 4]).then_some(*id))
                .unwrap();
            let index = game.mobs.iter().position(|m| m.id == id).unwrap();
            assert!(
                game.mobs[index].kind.armor > 0.0,
                "real Husk armor is the bypass control"
            );
            assert_eq!(game.mobs[index].health, 20.0);
            game.world.game_time += 1;
            game.update_chunk_tickets();
            game.tick_creature_entities().unwrap();
            let m = &game.mobs[index];
            assert!(m.horizontal_collision, "actual wall collision");
            assert!(m.position.x > 4.5 && m.position.x < 6.0);
            assert_eq!(m.velocity.x, 0.0, "resolved collision, no damage knockback");
            assert_eq!(m.health, 3.0, "(2 - 0) * 10 - 3, bypassing the real armor");
            let damage = m.last_damage.unwrap().0;
            assert_eq!(damage.kind, horizon_client::mobs::DamageKind::FlyIntoWall);
            assert!(damage.bypasses_armor() && damage.no_knockback());
            assert!(!damage.is_fire() && !damage.is_projectile());
            assert_eq!(source_damage_type(damage), "minecraft:fly_into_wall");
            assert_eq!(game.uuid_for(id), [169, 0, 0, 4]);
        });
    }
}

/// `ServerLevel.getCurrentDifficultyAt` in `world`: the difficulty, the level's clock, and
/// the inhabited time and moon of the chunk at `at` (both 0 when it isn't loaded).
pub(in crate::game) fn local_difficulty_in(
    world: &horizon_client::world::World,
    at: [f64; 3],
) -> spawning::LocalDifficulty {
    use horizon_world::food::Difficulty;
    let base = match world.health_rules.difficulty {
        Difficulty::Peaceful => 0,
        Difficulty::Easy => 1,
        Difficulty::Normal => 2,
        Difficulty::Hard => 3,
    };
    let key = ((at[0].floor() as i32) >> 4, (at[2].floor() as i32) >> 4);
    let (inhabited, moon) = if world.chunks.contains_key(&key) {
        let phase = world.day_time.div_euclid(24000).rem_euclid(8) as usize;
        (
            world.inhabited.get(&key).copied().unwrap_or(0),
            spawning::MOON_BRIGHTNESS[phase],
        )
    } else {
        (0, 0.0)
    };
    spawning::LocalDifficulty::new(base, world.day_time, inhabited, moon)
}
#[cfg(test)]
#[path = "../../entities/tests/leash_without_item_criterion_tests.rs"]
mod leash_without_item_criterion_tests;

#[cfg(test)]
#[path = "../../entities/tests/leash_without_item_postbaseline_controls.rs"]
mod leash_without_item_postbaseline_controls;

#[cfg(test)]
#[path = "../tests/revenge_home_tests.rs"]
mod revenge_home_tests;

mod knockback;

#[cfg(test)]
mod death_loot_context_tests;

#[cfg(test)]
mod breeding_heart_tests {
    use super::*;
    /// `Turtle.TurtleBreedGoal.breed` overrides `Animal.breed`, so a bred turtle
    /// never runs `finalizeSpawnChildFromBreeding`'s heart burst (entity event
    /// 18); `Frog.spawnChildFromBreeding` still does.
    #[test]
    #[ignore = "needs prepared resources (HORIZON_TEST_RESOURCES)"]
    fn a_bred_turtle_posts_no_heart_burst_but_a_bred_frog_does() {
        let mut resources = None;
        let mut game = crate::game::game_tests::tests::server_game(&mut resources);
        game.audio.capture = Some(Vec::new());
        for (kind_id, hearts) in [("minecraft:turtle", 0), ("minecraft:frog", 1)] {
            let kind = game.res.mob_types().get(kind_id).unwrap().clone();
            game.mobs
                .push(Mob::new(1, kind.clone(), Vec3::new(8.5, 64.0, 8.5), 0.0));
            game.mobs
                .push(Mob::new(2, kind, Vec3::new(9.5, 64.0, 8.5), 0.0));
            game.lan_entity_events.clear();
            game.breed(
                1,
                2,
                None,
                None,
                None,
                horizon_client::mobs::BreedExperience::Deferred,
            );
            let burst = game
                .lan_entity_events
                .iter()
                .filter(|(id, event)| *id == 1 && *event == 18)
                .count();
            assert_eq!(
                burst, hearts,
                "{kind_id} heartbeat events: {:?}",
                game.lan_entity_events
            );
            game.mobs.clear();
            game.orbs.clear();
        }
    }
}

#[cfg(feature = "upstream-parallel-mobs")]
#[path = "../../creatures/mob_group.rs"]
mod mob_group;

#[cfg(feature = "upstream-parallel-mobs")]
#[path = "../../creatures/mob_islands.rs"]
mod mob_islands;

#[cfg(feature = "upstream-parallel-mobs")]
#[path = "../../creatures/prep.rs"]
mod prep;

#[cfg(test)]
#[path = "frame_projectile_tests.rs"]
mod frame_projectile_tests;
