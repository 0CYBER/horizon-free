//! `ServerExplosion`: exploded positions (vanilla ray march over a Java
//! HashSet, so iteration and the following shuffle match vanilla's random
//! draws), entity damage with exposure and knockback, block interaction with
//! merged drops and TNT chain priming, and optional fire.
use super::{
    Game,
    entity::{Blast, Entity, EntityHandle, with_entity, with_entity_mut},
};
use horizon_client::health::DamageLevel;
use horizon_client::{
    Result, blocks::Fluid, collisions::SceneCollisions, items::ItemStack, mobs::Who, text::Text,
};
use horizon_core::{
    geometry::{Vec3, VoxelShape},
    position::BlockPos,
    random::{LegacyRandom, RandomSource},
};
use mod_api::ExplosionStart;
#[cfg(test)]
use mod_api::ItemDrop;
use std::{cmp::Ordering, collections::HashSet, io};
use {
    horizon_core::java_hash_map::{JavaHashMap, JavaKey},
    horizon_world::{
        collision_context::{CollisionContext, CollisionEntity, CollisionItem},
        entity_obstruction::EntityIdentity,
        registry::BlockStateId,
        world_ray::{RayBlockMode, RayFluidMode, RayWorld, clip_world},
    },
};

/// Keep each block's contribution while vanilla merges stacks across blocks.
#[derive(Default)]
struct ExplosionDrops {
    stacks: Vec<([i32; 3], ItemStack)>,
    blocks: Vec<ExplosionBlockDrops>,
}

struct ExplosionBlockDrops {
    pos: [i32; 3],
    state: BlockStateId,
    pieces: Vec<ExplosionDropPiece>,
}

struct ExplosionDropPiece {
    group: usize,
    count: u32,
}

/// The causing entity of an explicit explosion damage source. Minecart ignition
/// keeps this in memory only, just like `MinecartTNT.ignitionSource`.
#[derive(Clone, Debug)]
pub(super) struct ExplosionCause {
    pub entity: (u32, Text),
    pub player: bool,
    pub guest: Option<u32>,
    pub species: Option<horizon_client::mobs::Species>,
}

/// `Level.clip` with `ClipContext.Block.COLLIDER`, no fluids and the
/// collision context of the entity whose exposure is measured.
pub(super) struct ClipData<'a, 'r> {
    pub facts: &'a horizon_client::blocks::BlockFacts,
    pub behaviours: &'a horizon_world::level::Behaviours,
    pub tables: &'a horizon_client::mesher::MeshTables,
    pub registry: &'r horizon_world::registry::BlockRegistry,
}
pub(super) struct ColliderRays<'a, 'r> {
    pub scene: SceneCollisions<'a, 'r>,
    pub context: CollisionContext<'static>,
    pub clip_data: Option<ClipData<'a, 'r>>,
}
impl RayWorld for ColliderRays<'_, '_> {
    type Error = io::Error;
    type Block = BlockStateId;
    type Fluid = ();
    fn block_state(&mut self, p: BlockPos) -> io::Result<BlockStateId> {
        let world = self.scene.world;
        if self.clip_data.is_some()
            && (!(world.bottom_y()..world.top_y()).contains(&p.y)
                || !world.chunks.contains_key(&(p.x >> 4, p.z >> 4)))
        {
            return Err(io::Error::other("ray entered an unloaded block"));
        }
        Ok(world.block(p.x, p.y, p.z))
    }
    fn fluid_state(&mut self, _: BlockPos) -> io::Result<()> {
        Ok(())
    }
    fn block_shape(
        &mut self,
        state: BlockStateId,
        p: BlockPos,
        mode: RayBlockMode,
    ) -> io::Result<VoxelShape> {
        if let Some(data) = &self.clip_data {
            if data.behaviours.data(state).dynamic_shape {
                return Err(io::Error::other("dynamic ray shape unsupported"));
            }
            if mode == RayBlockMode::Outline {
                let facts = data.facts.get(state);
                let shape = super::perspective::ray_shape_from_boxes(&facts.outline)?;
                let offset = super::perspective::outline_offset(
                    data.tables,
                    state,
                    p.x,
                    p.z,
                    facts.outline_offset,
                );
                return Ok(shape.moved(Vec3::new(offset[0], offset[1], offset[2])));
            }
        }
        let binding = self.scene.binding;
        binding
            .collision_shape_with_context(&mut self.scene, state, p, &self.context)
            .map_err(|e| io::Error::other(format!("{e:?}")))
    }
    fn interaction_shape(&mut self, state: BlockStateId, _: BlockPos) -> io::Result<VoxelShape> {
        match &self.clip_data {
            Some(data) => super::perspective::ray_interaction_shape(data.registry, state),
            None => Ok(VoxelShape::empty()),
        }
    }
    fn fluid_is_source(&mut self, _: ()) -> io::Result<bool> {
        Ok(false)
    }
    fn fluid_is_empty(&mut self, _: ()) -> io::Result<bool> {
        Ok(true)
    }
    fn fluid_is_water(&mut self, _: ()) -> io::Result<bool> {
        Ok(false)
    }
    fn fluid_shape(&mut self, _: (), _: BlockPos) -> io::Result<VoxelShape> {
        Ok(VoxelShape::empty())
    }
}

/// `BlockPos` with Java `Vec3i.hashCode` / `compareTo`.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Key([i32; 3]);
impl JavaKey for Key {
    fn java_hash(&self) -> i32 {
        let [x, y, z] = self.0;
        y.wrapping_add(z.wrapping_mul(31))
            .wrapping_mul(31)
            .wrapping_add(x)
    }
    fn comparable_compare(&self, other: &Self) -> Option<Ordering> {
        let ([x, y, z], [ox, oy, oz]) = (self.0, other.0);
        Some(y.cmp(&oy).then(z.cmp(&oz)).then(x.cmp(&ox)))
    }
    fn tie_break(&self, _other: &Self) -> Ordering {
        Ordering::Less
    }
}

/// `Explosion.BlockInteraction` for the explosions Horizon produces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BlockInteraction {
    Keep,
    Destroy,
    DestroyWithDecay,
    /// `TRIGGER_BLOCK`: nothing breaks; buttons, doors and the like react.
    Trigger,
}
impl BlockInteraction {
    /// `Explosion.BlockInteraction.shouldAffectBlocklikeEntities`.
    fn affects_blocklike(self) -> bool {
        matches!(self, Self::Destroy | Self::DestroyWithDecay)
    }
}

/// Properties of the direct entity passed to `ServerLevel.explode`.
#[derive(Clone, Copy, Default)]
pub(super) struct ExplosionSource {
    silent: bool,
    pub(super) portal_proof: bool,
    /// The entity that exploded and its kind (`Explosion.getDirectSourceEntity`), when known.
    entity: Option<(u32, &'static str)>,
}
impl ExplosionSource {
    pub(super) fn with_silence(mut self, silent: bool) -> Self {
        self.silent = silent;
        self
    }
    pub(super) fn with_portal_proof(mut self, portal_proof: bool) -> Self {
        self.portal_proof = portal_proof;
        self
    }
    pub(super) fn with_entity(mut self, id: u32, kind: &'static str) -> Self {
        self.entity = Some((id, kind));
        self
    }
}

/// `ServerExplosion.hurtEntities`: an entity is hurt when its position is
/// within the blast's diameter of the centre.
pub(super) fn blast_reaches(center: [f64; 3], diameter: f64, at: [f64; 3]) -> bool {
    let d =
        ((at[0] - center[0]).powi(2) + (at[1] - center[1]).powi(2) + (at[2] - center[2]).powi(2))
            .sqrt();
    d / diameter <= 1.0
}

/// `ItemEntity.ignoreExplosion` through
/// `ServerExplosion.shouldAffectBlocklikeEntities`: an explosion that does not
/// affect blocklike entities is ignored entirely by a dropped item. With
/// `mobGriefing` off (`canEntityGrief` for the exploding entity) that is every
/// blast that keeps blocks; with it on, none (no wind charge goes through
/// `server_explode`).
fn items_ignore_explosion(mob_griefing: bool, interaction: BlockInteraction) -> bool {
    !mob_griefing && !interaction.affects_blocklike()
}

/// Whether a blast leaves item frames, paintings, cushions and armour stands
/// alone. Without `mobGriefing` (`canEntityGrief` for the exploding entity), a
/// blast that keeps blocks is ignored (`ServerExplosion.shouldAffectBlocklikeEntities`
/// through their `ignoreExplosion`; no wind charge goes through `server_explode`).
/// `mob_refused`: a mob causes the blast and may not grief, so it does no harm
/// (`BlockAttachedEntity.hurtServer` and `ArmorStand.hurtServer`:
/// `source.getEntity() instanceof Mob`, with `canEntityGrief` for that mob).
pub(super) fn blast_spares_decorations(
    mob_griefing: bool,
    interaction: BlockInteraction,
    mob_refused: bool,
    source_in_water: bool,
) -> bool {
    // `BlockAttachedEntity.ignoreExplosion`: a source in water first.
    source_in_water || mob_refused || (!mob_griefing && !interaction.affects_blocklike())
}

/// How far the blast's `center` is from `p`.
fn blast_distance(center: [f64; 3], p: [f64; 3]) -> f64 {
    ((p[0] - center[0]).powi(2) + (p[1] - center[1]).powi(2) + (p[2] - center[2]).powi(2)).sqrt()
}

/// The unit vector from `center` to `origin`, 0 when they coincide.
fn blast_direction(center: [f64; 3], origin: [f64; 3]) -> [f64; 3] {
    let d = [
        origin[0] - center[0],
        origin[1] - center[1],
        origin[2] - center[2],
    ];
    let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    if l < 1.0e-4 {
        [0.0; 3]
    } else {
        [d[0] / l, d[1] / l, d[2] / l]
    }
}

/// The blast as `native_explosion_knockback` tells it: its centre and the
/// blocks it is about to break.
struct ExplosionHookContext<'a> {
    center: [f64; 3],
    blocks: &'a [[i32; 3]],
    explicit_cause: Option<Option<ExplosionCause>>,
    direct_name: Option<Text>,
    held_item: Option<Text>,
    credit: bool,
    /// The calculator's `shouldDamageEntity`, and its knockback multiplier for
    /// a player that isn't flying and for one that is.
    hurts: bool,
    knockback: f32,
    flying_knockback: f32,
}

/// An entity in the blast's reach: how far (as a share of the blast's
/// diameter), how much of it the rays see, and the knockback.
struct Hit {
    dist: f64,
    exposure: f32,
    push: Vec3,
}

/// Apply blast damage when the mob's team and wither immunity allow it.
fn hurt_mob(
    level: &mut DamageLevel<'_>,
    m: &mut horizon_client::mobs::Mob,
    hurtable: bool,
    cause: Option<horizon_client::mobs::Species>,
    (amount, mut source): (f32, horizon_client::mobs::Damage),
) -> bool {
    source.attacker_species = cause;
    hurtable && m.damage(level, amount, source)
}

/// A blast's causing entity and its name (`Explosion.getIndirectSourceEntity`):
/// the mob that owns the exploding projectile while it is in the level
/// (`Projectile.getOwner` is null once it is removed), else the exploding
/// mob. `live` names a mob in the level, `exploded` the mobs whose blast
/// went off this tick and that already left it.
fn blast_cause(
    owner: Option<u32>,
    exploding: Option<u32>,
    live: impl Fn(u32) -> Option<Text>,
    exploded: &[(u32, Text)],
) -> Option<(u32, Text)> {
    if let Some(owner) = owner {
        return live(owner).map(|name| (owner, name));
    }
    let id = exploding?;
    let name = live(id).or_else(|| {
        exploded
            .iter()
            .rfind(|(m, _)| *m == id)
            .map(|(_, n)| n.clone())
    })?;
    Some((id, name))
}

thread_local! {
    /// Client-side randomness (explosion sound pitch), separate from the level random.
    static CLIENT_RANDOM: std::cell::RefCell<LegacyRandom> = std::cell::RefCell::new(LegacyRandom::new(0x5eed));
}
pub(super) fn client_float() -> f32 {
    CLIENT_RANDOM.with(|r| r.borrow_mut().next_float())
}

impl Game<'_> {
    pub(super) fn explosion_entity_name(&self, id: u32, kind: &str) -> Text {
        self.persist_raw(id)
            .and_then(|r| r.get("CustomName"))
            .map_or_else(
                || Text::translate(&format!("entity.{}", kind.replace(':', ".")), &[]),
                |name| {
                    horizon_client::scoreboard::comp_text(&horizon_client::scoreboard::tag_to_json(
                        name,
                    ))
                },
            )
    }

    fn explosion_cause_resolves(&self, cause: &ExplosionCause) -> bool {
        if !cause.player {
            return self
                .mobs
                .iter()
                .any(|m| m.id == cause.entity.0 && !m.removed);
        }
        let acting = self
            .lan_acting_entity
            .or_else(|| self.player.as_ref().map(|p| p.entity_id as u32));
        acting == Some(cause.entity.0)
            || self.lan.as_ref().is_some_and(|host| {
                host.hub
                    .guests
                    .iter()
                    .any(|g| g.entity_id as u32 == cause.entity.0)
            })
    }

    /// An explicit DamageSource must not inherit an enclosing blast's cause.
    pub(super) fn with_explosion_cause<R>(
        &mut self,
        cause: Option<ExplosionCause>,
        direct_source: (u32, Text),
        run: impl FnOnce(&mut Self) -> R,
    ) -> R {
        // The DamageSource retains the removed mob's display name. This is
        // deliberately not added to exploded_kinds: its EntityReference no
        // longer resolves for kill credit, unlike Horizon's early exploders.
        if let Some(cause) = &cause
            && cause.species.is_some()
            && !self.mobs.iter().any(|m| m.id == cause.entity.0)
            && !self
                .combat
                .exploded
                .iter()
                .any(|(id, _)| *id == cause.entity.0)
        {
            self.combat.exploded.push(cause.entity.clone());
        }
        let source = self.combat.explosion_source.take();
        let owner = self.combat.explosion_owner.take();
        let player = self.combat.blast_owner.take();
        let guest = self.combat.blast_guest.take();
        let direct = self.combat.explosion_cause.take();
        self.combat.explosion_source = Some(direct_source.0);
        let outer_direct = self
            .combat
            .explicit_explosion_direct
            .replace(direct_source.1);
        if let Some(cause) = &cause {
            if cause.player {
                self.combat.blast_owner = Some(cause.entity.clone());
                self.combat.blast_guest = cause.guest;
            } else {
                self.combat.explosion_owner = Some(cause.entity.0);
            }
        }
        let outer = self.combat.explicit_explosion_cause.replace(cause);
        let result = run(self);
        self.combat.explicit_explosion_cause = outer;
        self.combat.explosion_source = source;
        self.combat.explosion_owner = owner;
        self.combat.blast_owner = player;
        self.combat.blast_guest = guest;
        self.combat.explosion_cause = direct;
        self.combat.explicit_explosion_direct = outer_direct;
        result
    }

    /// The blast damage's cause. An explicit crystal/cart DamageSource may
    /// differ from the explosion's own indirect source.
    pub(super) fn blast_attacker(&self) -> Option<(u32, Text)> {
        if let Some(cause) = &self.combat.explicit_explosion_cause {
            return cause.as_ref().map(|c| {
                let name = (!c.player)
                    .then(|| self.mob_display_name(c.entity.0))
                    .flatten()
                    .unwrap_or_else(|| c.entity.1.clone());
                (c.entity.0, name)
            });
        }
        // `Explosion.getIndirectSourceEntity`: a primed TNT's owner names the
        // blast before any mob does, which makes the damage type
        // `minecraft:player_explosion` (`server_explode` asks this).
        self.combat.blast_owner.clone().or_else(|| {
            blast_cause(
                self.combat.explosion_owner,
                self.combat.explosion_source,
                |id| self.mob_display_name(id),
                &self.combat.exploded,
            )
        })
    }
    /// `ServerExplosion.getSeenPercent` for an entity box.
    pub(in crate::game) fn seen_percent(
        &self,
        center: [f64; 3],
        min: [f64; 3],
        max: [f64; 3],
    ) -> f32 {
        let xs = 1.0 / ((max[0] - min[0]) * 2.0 + 1.0);
        let ys = 1.0 / ((max[1] - min[1]) * 2.0 + 1.0);
        let zs = 1.0 / ((max[2] - min[2]) * 2.0 + 1.0);
        let x_offset = (1.0 - (1.0 / xs).floor() * xs) / 2.0;
        let z_offset = (1.0 - (1.0 / zs).floor() * zs) / 2.0;
        if xs < 0.0 || ys < 0.0 || zs < 0.0 {
            return 0.0;
        }
        // ClipContext(..., entity): the entity standing at the box's bottom.
        let mut rays = ColliderRays {
            clip_data: None,
            scene: SceneCollisions {
                world: &self.world,
                binding: self.collision,
                subject: None,
            },
            context: CollisionContext::for_entity(
                CollisionEntity {
                    identity: EntityIdentity(0),
                    kind: horizon_world::collision_context::EntityContextKind::None,
                    y: min[1],
                    descending: false,
                    main_hand: Some(CollisionItem::EMPTY),
                },
                false,
            ),
        };
        let (mut hits, mut count) = (0, 0);
        let mut xx = 0.0;
        while xx <= 1.0 {
            let mut yy = 0.0;
            while yy <= 1.0 {
                let mut zz = 0.0;
                while zz <= 1.0 {
                    let x = min[0] + (max[0] - min[0]) * xx;
                    let y = min[1] + (max[1] - min[1]) * yy;
                    let z = min[2] + (max[2] - min[2]) * zz;
                    let from = Vec3::new(x + x_offset, y, z + z_offset);
                    let to = Vec3::new(center[0], center[1], center[2]);
                    let missed = clip_world(
                        &mut rays,
                        from,
                        to,
                        RayBlockMode::Collider,
                        RayFluidMode::None,
                        4096,
                    )
                    .is_ok_and(|r| r.miss);
                    if missed {
                        hits += 1;
                    }
                    count += 1;
                    zz += zs;
                }
                yy += ys;
            }
            xx += xs;
        }
        hits as f32 / count as f32
    }
    /// What the cell at `pos` resists the blast `explosion` with
    /// (`ExplosionDamageCalculator.getBlockExplosionResistance` and the
    /// calculator overrides `rules` holds): None for no
    /// resistance at all, which costs a ray only its step. Also whether the
    /// block is spared. With `answers`, a state whose parts declare their
    /// answer pure is asked once and answered from there after. The level is
    /// not changed.
    pub(super) fn cell_explosion_resistance(
        &mut self,
        pos: [i32; 3],
        rules: &BlastRules,
        explosion: &ExplosionStart<'_>,
        answers: Option<&mut PartAnswers>,
    ) -> (Option<f32>, bool) {
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let f = self.res.facts().get(state);
        if let Some(step) = self.blast_calculator_step(&f.name) {
            return step;
        }
        // USED_PORTAL_DAMAGE_CALCULATOR returns no resistance and never explodes a portal.
        if rules.portal_proof && &*f.name == "minecraft:nether_portal" {
            return (None, true);
        }
        let fluid = if f.fluid != Fluid::None {
            100.0f32
        } else {
            0.0
        };
        // A primed TNT minecart's rails and the blocks they
        // stand on: no resistance, and never blown up.
        let spared = rules.rail_proof
            && (self.rail(None, pos[0], pos[1], pos[2]).is_some()
                || self.rail(None, pos[0], pos[1] + 1, pos[2]).is_some());
        let resistance = match rules.resists {
            Some((at, resistance)) if at == pos => Some(resistance),
            _ if !self.res.facts().is_air(state) || f.fluid != Fluid::None => {
                let immune = rules
                    .wither_immune
                    .as_ref()
                    .is_some_and(|t| t.contains(&*f.name));
                let own = f.explosion_resistance;
                // WitherBoss.canDestroy for the skull's cap, or what the
                // block's parts answer for the skull.
                let kept = match (rules.cap, rules.direct) {
                    (Some(_), Some(skull)) => !self.entity_may_destroy(skull, pos, !immune),
                    _ => immune,
                };
                // Block.getExplosionResistance, or what the block's parts answer.
                let base = if spared {
                    0.0
                } else {
                    let asked = match answers {
                        Some(answers)
                            if crate::game::api_values::block_state::explosion_resistance_pure(
                                state,
                            ) =>
                        {
                            *answers.entry(state).or_insert_with(|| {
                                self.part_explosion_resistance(pos, state, explosion)
                            })
                        }
                        _ => self.part_explosion_resistance(pos, state, explosion),
                    };
                    asked.unwrap_or(own).max(fluid)
                };
                Some(blast_resistance(base, rules.cap, kept))
            }
            _ => None,
        };
        (resistance, spared)
    }
    /// The rules of the blast going off now, `resists` overriding one block's
    /// resistance and `portal_proof` as its source has it.
    pub(super) fn blast_rules(
        &self,
        resists: Option<([i32; 3], f32)>,
        portal_proof: bool,
    ) -> BlastRules {
        let cap = self.combat.blast_resistance_cap;
        BlastRules {
            resists,
            cap,
            direct: self.combat.explosion_direct,
            // WitherSkull.getBlockExplosionResistance: a dangerous skull's blast
            // caps a block the wither may destroy at 0.8.
            wither_immune: cap.and_then(|_| self.block_tag_blocks("minecraft:wither_immune")),
            rail_proof: self.combat.rail_proof_blast,
            portal_proof,
        }
    }
    /// The blocks of tag `tag`, if the game has it.
    fn block_tag_blocks(&self, tag: &str) -> Option<HashSet<String>> {
        self.res.items().block_tags.get(tag).cloned()
    }
    /// `ServerExplosion.calculateExplodedPositions`; `resists` overrides one
    /// block's resistance (`ExplosionDamageCalculator.getBlockExplosionResistance`).
    fn exploded_positions(
        &mut self,
        center: [f64; 3],
        radius: f32,
        resists: Option<([i32; 3], f32)>,
        portal_proof: bool,
        explosion: &ExplosionStart<'_>,
    ) -> Vec<[i32; 3]> {
        let (min_y, max_y) = self.with_level(|_, level, _| {
            use horizon_world::level::LevelAccess;
            (level.min_y(), level.max_y())
        });
        let rules = self.blast_rules(resists, portal_proof);
        let mut set: JavaHashMap<Key, ()> = JavaHashMap::default();
        for xx in 0..16 {
            for yy in 0..16 {
                for zz in 0..16 {
                    if !(xx == 0 || xx == 15 || yy == 0 || yy == 15 || zz == 0 || zz == 15) {
                        continue;
                    }
                    let mut xd = f64::from(xx as f32 / 15.0 * 2.0 - 1.0);
                    let mut yd = f64::from(yy as f32 / 15.0 * 2.0 - 1.0);
                    let mut zd = f64::from(zz as f32 / 15.0 * 2.0 - 1.0);
                    let d = (xd * xd + yd * yd + zd * zd).sqrt();
                    xd /= d;
                    yd /= d;
                    zd /= d;
                    let mut power = radius * (0.7 + self.world.random.next_float() * 0.6);
                    let (mut xp, mut yp, mut zp) = (center[0], center[1], center[2]);
                    // A ray's steps are 0.3 long, about three to a block: a
                    // step in the block the last one was in takes what that
                    // one took (the level does not change while rays cast),
                    // and that block is in `set` already if it is to be.
                    let mut last: Option<([i32; 3], f32, bool)> = None;
                    while power > 0.0 {
                        let pos = [xp.floor() as i32, yp.floor() as i32, zp.floor() as i32];
                        if pos[1] < min_y
                            || pos[1] >= max_y
                            || !(-30_000_000..30_000_000).contains(&pos[0])
                            || !(-30_000_000..30_000_000).contains(&pos[2])
                        {
                            break;
                        }
                        let (taken, spared, again) = match last {
                            Some((at, taken, spared)) if at == pos => (taken, spared, true),
                            _ => {
                                let (resistance, spared) =
                                    self.cell_explosion_resistance(pos, &rules, explosion, None);
                                let taken = resistance.map_or(0.0, |r| (r + 0.3) * 0.3);
                                last = Some((pos, taken, spared));
                                (taken, spared, false)
                            }
                        };
                        power -= taken;
                        if power > 0.0 && !spared && !again {
                            set.put(Key(pos), ());
                        }
                        xp += xd * f64::from(0.3f32);
                        yp += yd * f64::from(0.3f32);
                        zp += zd * f64::from(0.3f32);
                        power -= 0.225_000_01;
                    }
                }
            }
        }
        set.entries().map(|(k, _)| k.0).collect()
    }
    /// `ServerExplosion.hurtEntities` for the player: damage scaled by
    /// distance and exposure, through a raised shield (`hurtServer` blocks
    /// explosions like any attack from where the blast is), then the push
    /// (less with explosion knockback resistance; none for spectators or a
    /// creative player flying).
    ///
    /// Returns the push their client adds from the explosion packet
    /// (`hitPlayers`), unless a hurt marked their velocity: that goes to them
    /// whole, this push in it, once their server copy has travelled
    /// (Entity.markHurt, then ServerEntity.sendChanges), and replaces what the
    /// packet would add.
    ///
    /// `cause` is the blast's source as predicates see it, read before the
    /// guests' turns swap their own combat state in.
    fn explosion_hurt_player(
        &mut self,
        context: &ExplosionHookContext<'_>,
        radius: f32,
        damage_type: &str,
        exploding: Option<u32>,
        attacker: Option<&(u32, Text)>,
        cause: Option<&horizon_client::advancements::EntityView>,
    ) -> Option<[f64; 3]> {
        let ExplosionHookContext { center, blocks, .. } = *context;
        let flying = self.player.as_ref().is_some_and(|p| p.flying);
        let knockback = if flying {
            context.flying_knockback
        } else {
            context.knockback
        };
        let diameter = f64::from(radius * 2.0);
        // Blast protection: `EXPLOSION_KNOCKBACK_RESISTANCE` (0.15 per level and
        // piece, so two Blast Protection IV pieces cancel it).
        let resistance = self
            .player
            .as_mut()
            .and_then(|p| p.attribute("minecraft:explosion_knockback_resistance").ok())
            .unwrap_or(0.0);
        let p = self.player.as_ref()?;
        let feet = [p.position.x, p.position.y, p.position.z];
        let dist = ((feet[0] - center[0]).powi(2)
            + (feet[1] - center[1]).powi(2)
            + (feet[2] - center[2]).powi(2))
        .sqrt()
            / diameter;
        if dist > 1.0 {
            return None;
        }
        // `getEntities(this.source, ...)`: a player that is the blast's source is spared.
        let acting = self.lan_acting_entity.or(Some(p.entity_id as u32));
        if self.combat.explosion_source_named && exploding == acting {
            return None;
        }
        // ServerPlayer.onExplosionHit: the blast that hit the player is theirs.
        self.adv.marks.blast = Some((feet, cause.cloned()));
        let w = f64::from(p.dimensions.width) / 2.0;
        let h = f64::from(p.dimensions.height);
        let eye = [
            feet[0],
            feet[1] + f64::from(p.dimensions.eye_height),
            feet[2],
        ];
        // Neither hurt nor pushed: not checked for exposure.
        let exposure = if context.hurts || knockback != 0.0 {
            self.seen_percent(
                center,
                [feet[0] - w, feet[1], feet[2] - w],
                [feet[0] + w, feet[1] + h, feet[2] + w],
            )
        } else {
            0.0
        };
        let pow = (1.0 - dist) * f64::from(exposure);
        let amount = ((pow * pow + pow) / 2.0 * 7.0 * diameter + 1.0) as f32;
        let d = [eye[0] - center[0], eye[1] - center[1], eye[2] - center[2]];
        let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        let dir = if l < 1.0e-4 {
            [0.0; 3]
        } else {
            [d[0] / l, d[1] / l, d[2] / l]
        };
        let power = pow * f64::from(knockback) * (1.0 - resistance);
        // `shouldDamageEntity`, then `source.doTeamsAllowDamage(entity)`.
        let hurtable = context.hurts
            && exploding.is_none_or(|id| {
                horizon_client::scoreboard::teams_allow_damage(
                    self.mob_team(id),
                    self.player_team(),
                )
            });
        let blocking = self
            .player_blocking(Some(center), damage_type, false)
            .filter(|_| context.hurts);
        let raised = blocking.is_some();
        let (creative, spectator) = (self.creative, self.spectator());
        let mut outcome = horizon_client::health::BlockOutcome::default();
        let mut pushed = None;
        let explicit = context.explicit_cause.as_ref().and_then(|c| c.as_ref());
        let living_attacker = explicit.is_none_or(|c| c.player || c.species.is_some());
        let living_non_player = context
            .explicit_cause
            .as_ref()
            .map_or(attacker.is_some(), |c| {
                c.as_ref().is_some_and(|c| !c.player && c.species.is_some())
            });
        // The death message names the blast's causing entity. It goes to this
        // player's tracker, the host's or a guest's.
        if let Some((id, name)) = attacker.filter(|_| !creative && !spectator && hurtable) {
            if living_attacker && context.credit {
                self.note_player_attacker(
                    damage_type,
                    *id,
                    name.clone(),
                    context.held_item.clone(),
                );
            } else {
                self.note_uncredited_attacker(
                    damage_type,
                    *id,
                    name.clone(),
                    context.held_item.clone(),
                    living_attacker,
                );
            }
        } else if !creative
            && !spectator
            && hurtable
            && let Some(direct) = context.direct_name.clone()
        {
            self.note_direct_entity(damage_type, direct);
        }
        let rules = self.world.health_rules;
        if let Some(p) = self.player.as_mut()
            && let Some(id) = self.health_resources.damage.id(damage_type)
        {
            // Creative damage is attempted and refused by Player.hurtServer, not here.
            if !spectator
                && hurtable
                && let Ok(o) = p.hurt_caused_by(
                    crate::game::entity::damage_level_of!(self, scoped),
                    &self.motion.health,
                    rules,
                    id,
                    amount,
                    blocking,
                    living_non_player,
                    // The blast's causing entity.
                    attacker.map_or(0, |(id, _)| *id as i32),
                    false,
                )
            {
                outcome = o;
                // dealDefaultKnockback's `indicateDamage` points at the blast
                // (the source position), not the no-direction fallback.
                if o.knockback && o.blocked <= 0.0 {
                    p.health
                        .indicate_damage(center[0] - feet[0], center[2] - feet[2], p.yaw);
                }
            }
        }
        if !spectator {
            let mut push = [dir[0] * power, dir[1] * power, dir[2] * power];
            self.native_explosion_knockback(self.dim.id(), center, blocks, None, &mut push);
            if let Some(p) = self.player.as_mut() {
                p.velocity = Vec3::new(
                    p.velocity.x + push[0],
                    p.velocity.y + push[1],
                    p.velocity.z + push[2],
                );
                if !(creative && flying) {
                    // ServerExplosion.hitPlayers: the packet carries the push even when the hurt
                    // also marks the velocity (its motion packet follows).
                    pushed = Some(push);
                }
            }
        }
        self.combat.forget_noted(outcome.canceled);
        if raised {
            let source = mod_api::entity::health::HurtSource {
                damage_type,
                causing: attacker.map_or(0, |(id, _)| *id as i32),
                direct: 0,
                position: None,
            };
            let attack = self.current_attack();
            self.after_block(outcome, (None, false), source, &attack);
        }
        pushed
    }

    /// Where a blast at `center` reaches `entity`, unless it ignores the blast
    /// or is out of range (`getEntities` and `ServerExplosion.hurtEntities`).
    /// Generic over the kind, so every walked list gets its own copy.
    fn blast_hit<E: Entity>(
        &self,
        entity: &E,
        center: [f64; 3],
        diameter: f64,
        blast: &Blast,
        seen: bool,
    ) -> Option<Hit> {
        if entity.ignores_explosion(blast) || !entity.pushed_by_explosion() {
            return None;
        }
        let feet = entity.position();
        let dist = blast_distance(center, [feet.x, feet.y, feet.z]) / diameter;
        if dist > 1.0 {
            return None;
        }
        let b = entity.explosion_box();
        let (lo, hi) = (b.min_position(), b.max_position());
        let exposure = if seen {
            self.seen_percent(center, [lo.x, lo.y, lo.z], [hi.x, hi.y, hi.z])
        } else {
            0.0
        };
        let dir = blast_direction(
            center,
            [feet.x, feet.y + entity.explosion_eye_height(), feet.z],
        );
        let power = (1.0 - dist) * f64::from(exposure);
        Some(Hit {
            dist,
            exposure,
            push: Vec3::new(dir[0] * power, dir[1] * power, dir[2] * power),
        })
    }
    /// `ServerExplosion.hurtEntities` (player, mobs, items, primed TNT, minecarts);
    /// the player takes `damage_type`. Returns the players whose clients add a
    /// push from the explosion packet, and how much (`explosion_hurt_player`).
    fn explosion_hurt_entities(
        &mut self,
        center: [f64; 3],
        radius: f32,
        damage_type: &str,
        interaction: BlockInteraction,
        blocks: &[[i32; 3]],
    ) -> Vec<(u32, [f64; 3])> {
        if radius < 1.0e-5 {
            return Vec::new();
        }
        // The calculator's answers, read before the guests' turns swap their
        // own combat state in.
        let hurts = self.blast_damages_entities();
        let (knockback, flying_knockback) =
            (self.blast_knockback(false), self.blast_knockback(true));
        let kind = self.blast_damage_kind();
        if hurts {
            self.explosion_hits_special(center, radius, blocks, damage_type);
        }
        let diameter = f64::from(radius * 2.0);
        let damage = |dist: f64, exposure: f32| {
            let pow = (1.0 - dist) * f64::from(exposure);
            ((pow * pow + pow) / 2.0 * 7.0 * diameter + 1.0) as f32
        };
        // `source.doTeamsAllowDamage(entity)`: the exploding mob spares its
        // team mates when its team has no friendly fire (they are still pushed).
        let exploding = self.combat.explosion_source;
        let teams_allow = |g: &Self, victim: Option<&horizon_client::scoreboard::Team>| {
            exploding.is_none_or(|id| {
                horizon_client::scoreboard::teams_allow_damage(g.mob_team(id), victim)
            })
        };
        // The player, then every other player in the level (LAN guests),
        // each swapped in as the player. In a guest's turn (their hit on an
        // end crystal) the player is that guest. The blast's source is the
        // same for all of them (`onExplosionHit(this.source)`).
        let attacker = self.blast_attacker();
        let blast_source = self.combat.explosion_cause.clone();
        let context = ExplosionHookContext {
            center,
            blocks,
            explicit_cause: self.combat.explicit_explosion_cause.clone(),
            direct_name: self.combat.explicit_explosion_direct.clone(),
            credit: self
                .combat
                .explicit_explosion_cause
                .as_ref()
                .is_none_or(|cause| {
                    cause
                        .as_ref()
                        .is_some_and(|cause| self.explosion_cause_resolves(cause))
                }),
            held_item: self
                .combat
                .explicit_explosion_cause
                .as_ref()
                .and_then(|c| c.as_ref())
                .and_then(|c| {
                    if c.player {
                        self.player_main_hand_custom_name(c.guest)
                    } else {
                        self.mob_main_hand_name(c.entity.0)
                    }
                }),
            hurts,
            knockback,
            flying_knockback,
        };
        let own = if self
            .lan_acting_entity
            .is_some_and(|id| !self.combat.visits(id))
        {
            None
        } else {
            self.explosion_hurt_player(
                &context,
                radius,
                damage_type,
                exploding,
                attacker.as_ref(),
                blast_source.as_ref(),
            )
        };
        let mut pushed: Vec<_> = self.lan_acting.zip(own).into_iter().collect();
        if let Some(host) = self.lan.as_ref() {
            let combat = &self.combat;
            let guests: Vec<u32> = host
                .hub
                .guests
                .iter()
                .filter(|g| host.hub.here(g))
                .filter(|g| u32::try_from(g.entity_id).map_or(true, |id| combat.visits(id)))
                .map(|g| g.conn)
                .collect();
            for conn in guests {
                let push = self.as_guest(conn, |g| {
                    g.explosion_hurt_player(
                        &context,
                        radius,
                        damage_type,
                        exploding,
                        attacker.as_ref(),
                        blast_source.as_ref(),
                    )
                });
                if let Some(Some(push)) = push {
                    pushed.push((conn, push));
                }
            }
        }
        // Mobs. `WitherBoss.hurtServer`: a wither shrugs off an undead cause.
        let cause = self
            .combat
            .explicit_explosion_cause
            .as_ref()
            .and_then(|c| c.as_ref())
            .and_then(|c| c.species)
            .or_else(|| {
                self.combat
                    .explosion_owner
                    .or(exploding)
                    .and_then(|id| self.mobs.iter().find(|m| m.id == id))
                    .map(|m| m.species())
            });
        // `ServerExplosion.hurtEntities`: every entity in reach, on the walk. Each
        // answers `ignoreExplosion` itself (`Entity::ignores_explosion`); the rest
        // is the same for all, and only what a blast does to a kind differs. The
        // walk covers the entities there are now: what the blast drops or spawns
        // is not hit by it. Destroyed items and orbs are squeezed out afterwards
        // (as `retain` does), and a destroyed boat or cart shifts its list, so
        // `gone` counts how far.
        let blast = Blast {
            vehicles_ignore: self.combat.explicit_explosion_cause.is_none()
                && self.combat.blast_owner.is_none()
                && self.blast_attacker().is_some()
                && !self
                    .ui
                    .rules
                    .bool_at(horizon_client::gamerules::vanilla::MOB_GRIEFING),
            items_ignore: items_ignore_explosion(self.blast_griefs(), interaction),
            spared: exploding.filter(|_| self.combat.explosion_source_named),
        };
        let mut destroyed_contents = Vec::new();
        macro_rules! blocked_damage {
            ($id:expr, $kind:expr) => {{
                let source = horizon_client::health::HurtSource {
                    damage_type,
                    causing: attacker.as_ref().map_or(0, |(id, _)| *id as i32),
                    direct: exploding.map_or(0, |id| id as i32),
                    position: Some(center),
                };
                let creative = self.creative_damage_source_id(source.causing);
                let vanilla = self.saved_refuses_damage($id as u32, damage_type, creative);
                crate::game::entity::damage_level_of!(self, scoped).invulnerability_check(
                    horizon_client::health::Hurting::Entity($id, $kind, source),
                    vanilla,
                )
            }};
        }
        // The walk visits the storages in storage order, unless a listening
        // `explosion_detonate` left the list, and then it visits exactly what
        // the list names, in its order (`explosion_detonate.rs`). The kinds
        // outside the storages (players, frames, paintings, cushions,
        // crystals, the dragon, hooks) answer no handle here: the player walk
        // and their own passes read the list for them.
        let mut cursor = self.entity_id_cursor();
        let mut listed_at = 0;
        loop {
            let next = if let Some(list) = self.combat.blast_hit_list.as_ref() {
                let Some(&id) = list.get(listed_at) else {
                    break;
                };
                listed_at += 1;
                self.blast_handle(id).map(|handle| (id, handle))
            } else {
                let Some(next) = self.next_entity_id(&mut cursor) else {
                    break;
                };
                Some(next)
            };
            let Some((entity, handle)) = next else {
                continue;
            };
            // An entity the blast's list does not name is neither hurt nor
            // pushed; without a listener the storages are the walk.
            let hit = if with_entity!(self, handle, e => self.combat.visits(e.id())) {
                let seen = hurts || knockback != 0.0;
                with_entity!(self, handle, e => self.blast_hit(e, center, diameter, &blast, seen))
            } else {
                None
            };
            let mut destroyed = false;
            if let Some(Hit {
                dist,
                exposure,
                push,
            }) = hit
            {
                let amount = damage(dist, exposure);
                match handle {
                    _ if !hurts => {}
                    EntityHandle::Mob(i) => {
                        let hurtable = teams_allow(self, self.mob_team(self.mobs[i].id));
                        let m = &mut self.mobs[i];
                        // The exploding mob is the damage source's entity, so
                        // victims it kills drop its charged-creeper loot
                        // (`Creeper.killedEntity`).
                        let source = horizon_client::mobs::Damage {
                            kind,
                            // A projectile's owner (`getIndirectSourceEntity`),
                            // else the exploding mob itself.
                            attacker: self
                                .combat
                                .explicit_explosion_cause
                                .as_ref()
                                .and_then(|c| c.as_ref())
                                .filter(|c| c.player)
                                .map(|_| Who::Player)
                                .or_else(|| {
                                    let player = self.combat.blast_owner_player
                                        && self.combat.explicit_explosion_cause.is_none();
                                    player.then_some(Who::Player)
                                })
                                .or_else(|| {
                                    attacker
                                        .as_ref()
                                        .map(|&(id, _)| id)
                                        .filter(|&o| o != m.id)
                                        .filter(|_| {
                                            self.combat
                                                .explicit_explosion_cause
                                                .as_ref()
                                                .is_none_or(|c| {
                                                    c.as_ref().is_some_and(|c| c.species.is_some())
                                                })
                                        })
                                        .map(horizon_client::mobs::Who::Mob)
                                }),
                            from: Some(center),
                            attacker_species: cause,
                        };
                        // `EXPLOSION_KNOCKBACK_RESISTANCE`, not `KNOCKBACK_RESISTANCE`:
                        // its base is 0 for every mob type, so iron golems and
                        // ravagers are pushed.
                        let victim = m.id;
                        let hurt = hurt_mob(
                            crate::game::entity::damage_level_of!(self, scoped),
                            m,
                            hurtable,
                            cause,
                            (amount, source),
                        );
                        if hurt {
                            if source.attacker == Some(Who::Player) {
                                let guest = self.combat.blast_guest;
                                if let Some(uuid) = self.player_uuid_of(guest) {
                                    self.last_hurt_by_player.insert(
                                        victim,
                                        crate::game::combat::PlayerCredit(
                                            horizon_client::lan::playerdata::ints_uuid(uuid),
                                        ),
                                    );
                                }
                                if let Some(host) = self.lan.as_mut() {
                                    match guest {
                                        Some(conn) => {
                                            host.mob_players.insert(victim, conn);
                                        }
                                        None => {
                                            host.mob_players.remove(&victim);
                                        }
                                    }
                                }
                            }
                            self.resolve_player_responsible(victim, source);
                        }
                        // Enderman.hurtServer teleports inside the hurt, before
                        // the push (the mob order stays as it was).
                        if let Err(e) = self.settle_hurt_teleports() {
                            log::warn!("Horizon: explosion: enderman teleport: {e}");
                        }
                    }
                    // `ItemEntity.hurtServer`: the damage comes off `Health`
                    // (5 to start) in f32 and the item is destroyed at zero, so
                    // a survivor keeps the health the blast left it. Stack damage
                    // resistance is resolved against this blast's damage type. A
                    // blast with a source entity under `mobGriefing` off never
                    // reaches here (`items_ignore`).
                    EntityHandle::Item(i) => {
                        let id = self.dropped[i].id as i32;
                        let blocked = blocked_damage!(id, "minecraft:item");
                        let Some(EntityHandle::Item(i)) =
                            self.refresh_entity_handle(entity, handle)
                        else {
                            continue;
                        };
                        let can_hurt = self
                            .stack_can_be_hurt_by(&self.dropped[i].stack, damage_type)
                            .unwrap_or(true);
                        let item = &mut self.dropped[i];
                        if !blocked && can_hurt {
                            item.health = (item.health as f32 - amount) as i32;
                            destroyed = item.health <= 0;
                            if destroyed {
                                let at = item.position;
                                // Health was already reduced above. Run the shared
                                // onDestroyed path without applying damage twice.
                                destroyed_contents.extend(
                                    item.hurt(0)
                                        .unwrap_or_default()
                                        .into_iter()
                                        .map(|stack| (stack, at)),
                                );
                            }
                        }
                    }
                    // `ExperienceOrb.hurtServer` takes the damage off their 5 health.
                    EntityHandle::Orb(i) => {
                        let id = self.orbs[i].id as i32;
                        if !blocked_damage!(id, "minecraft:experience_orb") {
                            let Some(EntityHandle::Orb(i)) =
                                self.refresh_entity_handle(entity, handle)
                            else {
                                continue;
                            };
                            destroyed = self.orbs[i].hurt(amount);
                        }
                    }
                    // `VehicleEntity.hurtServer` (explosions ignite TNT carts);
                    // a destroyed one is gone before the push.
                    EntityHandle::Cart(i) => {
                        let kind = self.carts[i].kind.id();
                        if !blocked_damage!(entity as i32, kind) {
                            let Some(EntityHandle::Cart(i)) =
                                self.refresh_entity_handle(entity, handle)
                            else {
                                continue;
                            };
                            self.hurt_cart(i, amount, false, true);
                        }
                    }
                    // `VehicleEntity.hurtServer`: damage * 10, broken past 40.
                    EntityHandle::Boat(i) => {
                        let kind = self.boats[i].entity_type();
                        if !blocked_damage!(entity as i32, &kind) {
                            let Some(EntityHandle::Boat(i)) =
                                self.refresh_entity_handle(entity, handle)
                            else {
                                continue;
                            };
                            self.hurt_boat(i, amount, false);
                        }
                    }
                    // `hurtServer` does nothing to the rest.
                    _ => {}
                }
                if destroyed {
                    match self.refresh_entity_handle(entity, handle) {
                        Some(EntityHandle::Item(i)) => {
                            self.dropped.remove(i);
                        }
                        Some(EntityHandle::Orb(i)) => {
                            self.orbs.remove(i);
                        }
                        _ => {}
                    }
                }
                let k = f64::from(knockback);
                let mut velocity = [push.x * k, push.y * k, push.z * k];
                self.native_explosion_knockback(
                    self.dim.id(),
                    center,
                    blocks,
                    Some(entity),
                    &mut velocity,
                );
                let push = Vec3::new(velocity[0], velocity[1], velocity[2]);
                let Some(handle) = self.refresh_entity_handle(entity, handle) else {
                    continue;
                };
                match handle {
                    EntityHandle::Item(_) | EntityHandle::Orb(_) if destroyed => {}
                    EntityHandle::Cart(i) => {
                        self.carts.update(i, |cart| cart.push_from_explosion(push))
                    }
                    EntityHandle::Boat(i) => {
                        self.boats.update(i, |boat| boat.push_from_explosion(push))
                    }
                    other => with_entity_mut!(self, other, e => e.push_from_explosion(push)),
                }
            }
        }
        // The original target walk is finished: released contents cannot be
        // hurt or pushed by the explosion that destroyed their container.
        self.spawn_destroyed_contents(destroyed_contents);
        self.native_leaves_flush();
        // Frames, paintings, cushions and armour stands last: vanilla lists
        // the entities in reach before hurting any, so the items they drop
        // here are neither destroyed nor pushed by the same blast.
        // BlockAttachedEntity.kill / ArmorStand.kill credit DamageSource.getEntity.
        // VehicleEntity.destroy above uses Entity.kill and keeps the vehicle as source.
        if hurts {
            self.explosion_breaks_frames(center, diameter, interaction);
            self.explosion_breaks_decorations(center, diameter, interaction);
        }
        pushed
    }

    /// Decoration kills credit `DamageSource.getEntity`: the explicit blast
    /// cause, including a TNT igniter or the player who broke an end crystal.
    pub(super) fn explosion_kill_credit(&self) -> Option<super::commands::Target> {
        use super::commands::Target;
        // A primed TNT's owner (a player) names the blast before any mob does.
        let player =
            self.combat.blast_owner_player || self.combat.explicit_explosion_cause.is_some();
        if self.combat.blast_owner.is_some() && player {
            return Some(
                self.combat
                    .blast_guest
                    .map_or(Target::Player, Target::Guest),
            );
        }
        if let Some((id, _)) = self.blast_attacker() {
            return Some(Target::Mob(id));
        }
        self.crystal_blast_player()
    }
    /// Decoration hurt checks use the damage source's cause (`instanceof Mob`),
    /// unlike vehicles, which use the explosion's indirect source.
    pub(super) fn blast_spares_decorations(&self, interaction: BlockInteraction) -> bool {
        blast_spares_decorations(
            self.blast_griefs(),
            interaction,
            self.blast_mob_refused(),
            self.combat.explosion_in_water,
        )
    }
    /// `ServerExplosion.shouldAffectBlocklikeEntities`'s griefing read:
    /// `canEntityGrief` for the exploding entity (`this.source`), the rule without one.
    fn blast_griefs(&self) -> bool {
        self.can_entity_grief(self.combat.explosion_source)
    }
    /// A mob causes the blast (`source.getEntity() instanceof Mob`) and
    /// `canEntityGrief` refuses it: decorations take no harm.
    fn blast_mob_refused(&self) -> bool {
        // No listener and the rule on: no mob is refused, and no cause is looked up.
        if !Game::grief_hooked(self)
            && self
                .ui
                .rules
                .bool_at(horizon_client::gamerules::vanilla::MOB_GRIEFING)
        {
            return false;
        }
        // The mob the damage names (an explicit cause only when it is a mob), found once.
        let mob = match &self.combat.explicit_explosion_cause {
            Some(cause) => cause
                .as_ref()
                .filter(|c| c.species.is_some())
                .map(|c| c.entity.0),
            None => self.blast_attacker().map(|(id, _)| id),
        };
        mob.is_some_and(|id| !self.can_entity_grief(Some(id)))
    }
    /// `Entity.isInWater` for an entity box: `updateFluidInteraction` over the box.
    pub(super) fn bounds_in_water(&self, bounds: horizon_core::geometry::Aabb) -> bool {
        self.mob_world_base(&[])
            .fluids(bounds, bounds.max_position().y)
            .in_water
    }
    /// `ServerExplosion.hurtEntities` for paintings, cushions and armour
    /// stands (item frames: `explosion_breaks_frames`). A painting or cushion
    /// in reach is killed and dropped (`BlockAttachedEntity.hurtServer`,
    /// `Painting.dropItem`, `Cushion.dropItem`). An armour stand that is
    /// neither invisible nor a marker breaks (`ArmorStand.hurtServer` with
    /// `IS_EXPLOSION`: `brokenByAnything`, then `kill`): its equipment drops,
    /// the stand itself does not.
    fn explosion_breaks_decorations(
        &mut self,
        center: [f64; 3],
        diameter: f64,
        interaction: BlockInteraction,
    ) {
        // An armour stand is no `BlockAttachedEntity`: a source in water does not spare it.
        let spares_stands = blast_spares_decorations(
            self.blast_griefs(),
            interaction,
            self.blast_mob_refused(),
            false,
        );
        if !self.blast_spares_decorations(interaction) {
            self.explosion_breaks_hanging(center, diameter);
        }
        if spares_stands {
            return;
        }
        // `DamageSources.explosion`: a caused blast is `player_explosion`.
        let cause = self.blast_attacker().map(|(id, _)| id);
        let damage_type = self.blast_damage_type();
        let causing = cause.unwrap_or(0);
        let mut i = 0;
        while i < self.armor_stands.len() {
            let a = &self.armor_stands[i];
            let p = a.position;
            if !super::armor_stands::spared_by_blast(a)
                && self.combat.visits(a.id)
                && blast_reaches(center, diameter, [p.x, p.y, p.z])
            {
                let id = a.id;
                let source = horizon_client::health::HurtSource {
                    damage_type,
                    causing: causing as i32,
                    direct: causing as i32,
                    position: Some(center),
                };
                let creative = self.creative_damage_source_id(source.causing);
                let vanilla = self.saved_refuses_damage(id, damage_type, creative);
                if crate::game::entity::damage_level_of!(self, scoped).invulnerability_check(
                    horizon_client::health::Hurting::ArmorStand(id as i32, source),
                    vanilla,
                ) {
                    i += 1;
                    continue;
                }
                self.break_armor_stand_by_anything(i, damage_type, causing);
            } else {
                i += 1;
            }
        }
    }
    /// `ExplosionEvent.Start` for the explosion `start`: true when a mod
    /// cancels it, and nothing of it happens.
    pub(in crate::game) fn explosion_start_cancelled(
        &mut self,
        start: &ExplosionStart<'_>,
    ) -> bool {
        if !mod_api::__private::listened(mod_api::HookId::ExplosionStart) {
            return false;
        }
        let mut cancel = false;
        self.native_explosion_start(start, &mut cancel);
        cancel
    }
    /// Paintings and cushions in reach (`BlockAttachedEntity.hurtServer`).
    fn explosion_breaks_hanging(&mut self, center: [f64; 3], diameter: f64) {
        let mut i = 0;
        while i < self.paintings.len() {
            // HangingEntity.recalculateBoundingBox: a painting stands at the
            // centre of its box.
            let (lo, hi) = self.paintings[i].bounds();
            let at = [0, 1, 2].map(|k| (lo[k] + hi[k]) / 2.0);
            // A painting a mod keeps stays at `i`.
            if !(self.combat.visits(self.paintings[i].id)
                && blast_reaches(center, diameter, at)
                && self.break_painting_by(i, false, self.explosion_kill_credit()))
            {
                i += 1;
            }
        }
        let mut i = 0;
        while i < self.cushions.len() {
            let p = self.cushions[i].position;
            if self.combat.visits(self.cushions[i].id)
                && blast_reaches(center, diameter, [p.x, p.y, p.z])
            {
                self.break_cushion_by(i, false, self.explosion_kill_credit());
            } else {
                i += 1;
            }
        }
    }
    /// `ServerLevel.getDestroyType`: a drop decay rule picks between decaying
    /// and plain destruction.
    pub(super) fn destroy_type(
        &self,
        rule: horizon_client::gamerules::RuleIndex,
    ) -> BlockInteraction {
        if self.ui.rules.bool_at(rule) {
            BlockInteraction::DestroyWithDecay
        } else {
            BlockInteraction::Destroy
        }
    }
    /// Runs an explosion whose source empties the hives it destroys
    /// (`BeehiveBlock.getDrops`: primed TNT, creeper, wither skull, wither,
    /// TNT minecart).
    pub(in crate::game) fn with_hive_emptying_blast<R>(
        &mut self,
        blast: impl FnOnce(&mut Self) -> R,
    ) -> R {
        self.combat.hive_emptying_blast = true;
        let result = blast(self);
        self.combat.hive_emptying_blast = false;
        result
    }
    /// `Level.explode` on the server: positions, entities, blocks, fire; then
    /// the client's explosion sound.
    pub(super) fn server_explode(
        &mut self,
        center: [f64; 3],
        radius: f32,
        interaction: BlockInteraction,
        fire: bool,
    ) -> Result<()> {
        self.server_explode_from(
            center,
            radius,
            interaction,
            fire,
            ExplosionSource::default(),
        )
    }
    /// `ServerLevel.explode`: only the direct source's `isSilent` suppresses
    /// the sound; terrain, entity damage and the visual packet are unchanged.
    pub(super) fn server_explode_from(
        &mut self,
        center: [f64; 3],
        radius: f32,
        interaction: BlockInteraction,
        fire: bool,
        source: ExplosionSource,
    ) -> Result<()> {
        self.server_explode_from_cause(center, radius, interaction, fire, source, None)
    }
    /// An explicit `DamageSource` is independent of the blast's indirect owner.
    /// `Some(None)` is explicitly ownerless; `None` keeps the ordinary source.
    pub(super) fn server_explode_from_cause(
        &mut self,
        center: [f64; 3],
        radius: f32,
        interaction: BlockInteraction,
        fire: bool,
        source: ExplosionSource,
        cause: Option<Option<ExplosionCause>>,
    ) -> Result<()> {
        let blast = |g: &mut Self| {
            let damage_type = g.blast_damage_type();
            g.server_explode_as_source(center, radius, interaction, fire, damage_type, None, source)
        };
        match cause {
            // A player causes it (a lit TNT's igniter, a mod blast's source):
            // their hits land as that player's.
            None if self.combat.blast_owner_player => {
                let attack = self.blast_owner_attack();
                self.with_attack(attack, blast)
            }
            None => blast(self),
            Some(cause) => {
                let (id, kind) = source.entity.ok_or_else(|| {
                    io::Error::other("an explicit explosion cause needs its direct entity")
                })?;
                let direct = (id, self.explosion_entity_name(id, kind));
                self.with_explosion_cause(cause, direct, blast)
            }
        }
    }

    /// The blast going off now as the hooks hear it, with the source and
    /// causing entity the combat state holds.
    pub(super) fn explosion_start<'a>(
        &self,
        center: [f64; 3],
        radius: f32,
        interaction: BlockInteraction,
        fire: bool,
        damage_type: &'a str,
    ) -> ExplosionStart<'a> {
        ExplosionStart::from(mod_api::__private::built::ExplosionStart {
            dimension: self.dim.id(),
            center,
            radius,
            fire,
            interaction: match interaction {
                BlockInteraction::Keep => mod_api::BlockInteraction::Keep,
                BlockInteraction::Destroy => mod_api::BlockInteraction::Destroy,
                BlockInteraction::DestroyWithDecay => mod_api::BlockInteraction::DestroyWithDecay,
                BlockInteraction::Trigger => mod_api::BlockInteraction::TriggerBlock,
            },
            damage_type,
            source: self.combat.explosion_source,
            // `getIndirectSourceEntity`, as the blast's attacker is.
            owner: self.blast_attacker().map(|(id, _)| id),
        })
    }
    /// `server_explode` with the damage type the player takes and a block
    /// whose explosion resistance the damage calculator overrides.
    pub(super) fn server_explode_as(
        &mut self,
        center: [f64; 3],
        radius: f32,
        interaction: BlockInteraction,
        fire: bool,
        damage_type: &str,
        resists: Option<([i32; 3], f32)>,
    ) -> Result<()> {
        self.server_explode_as_source(
            center,
            radius,
            interaction,
            fire,
            damage_type,
            resists,
            ExplosionSource::default(),
        )
    }
    fn server_explode_as_source(
        &mut self,
        center: [f64; 3],
        radius: f32,
        interaction: BlockInteraction,
        fire: bool,
        damage_type: &str,
        resists: Option<([i32; 3], f32)>,
        source: ExplosionSource,
    ) -> Result<()> {
        let start = self.explosion_start(center, radius, interaction, fire, damage_type);
        if self.serving.is_some() || self.options.dedicated.is_some() {
            if self.explosion_start_cancelled(&start) {
                return Ok(());
            }
            self.native_explosion_prime(center, radius);
        }
        if let Some(p) = &self.player {
            let d = [
                p.position.x - center[0],
                p.position.y - center[1],
                p.position.z - center[2],
            ];
            let distance = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            self.input_feedback(horizon_client::input_source::Feedback::Explosion {
                distance,
                radius,
            });
        }
        // Explosion.explode: the blast is heard.
        let ctx = horizon_client::sculk::Context::default();
        self.post_game_event("minecraft:explode", center, ctx);
        let mut targets =
            self.exploded_positions(center, radius, resists, source.portal_proof, &start);
        // `shouldBlockExplode`: a calculator that explodes no blocks keeps none.
        if !self.blast_explodes_blocks() {
            targets.clear();
        }
        // The walks that hurt entities follow the list a listening
        // `explosion_detonate` leaves: a blast set off inside one of them
        // (its own detonate, a nested `level_explode`) keeps its own list.
        let outer = std::mem::take(&mut self.combat.blast_hit_list);
        let outer_ids = std::mem::take(&mut self.combat.blast_hit_ids);
        self.explosion_detonate(center, radius, &mut targets);
        let pushed =
            self.explosion_hurt_entities(center, radius, damage_type, interaction, &targets);
        self.combat.blast_hit_list = outer;
        self.combat.blast_hit_ids = outer_ids;
        self.explosion_block_step(&mut targets, radius, interaction, fire, &start)?;
        let effects = self.blast_effects();
        if !source.silent {
            let pitch = (1.0 + (client_float() - client_float()) * 0.2) * 0.7;
            self.audio.play(&effects.sound, Some(center), 4.0, pitch);
        }
        // ServerLevel.explode sends toBlow.size() whatever the interaction, and
        // `ServerExplosion.isSmall` also covers an explosion that keeps blocks.
        let block_count = targets.len();
        let small = radius < 2.0 || interaction == BlockInteraction::Keep;
        let emitter = effects.emitter(small);
        self.particles.explosion(
            center,
            emitter,
            block_count,
            radius,
            &effects.block_particles,
        );
        self.lan_explode(center, small, radius, block_count, pushed, effects);
        Ok(())
    }
    /// `InfestedBlock.spawnAfterBreak` without a tool that prevents it: while
    /// blocks drop, its silverfish (`spawnInfestation`) comes out at the
    /// bottom centre of the block, with its spawn animation.
    pub(super) fn spawn_infestation(&mut self, pos: [i32; 3]) {
        if !self
            .ui
            .rules
            .bool_at(horizon_client::gamerules::vanilla::BLOCK_DROPS)
        {
            return;
        }
        let at = [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]),
            f64::from(pos[2]) + 0.5,
        ];
        if let Some(id) = self.spawn_mob(
            "minecraft:silverfish",
            at,
            false,
            crate::game::creatures::SpawnSite::Plain,
        ) {
            self.mob_poof(id);
        }
    }
    /// `BlockBehaviour.onExplosionHit` (TNT: `wasExploded` primes a short fuse).
    /// The blast's block step on `targets` (`ServerExplosion.explode` from
    /// `interactWithBlocks` to `createFire`): a trigger blast works the
    /// blocks it may; one that breaks blocks shuffles them with the level's
    /// random and hits each, its drops merged across all of them and popped
    /// after the last; then, with `fire`, a third of the cells left air above
    /// a solid block catch fire, whatever the interaction. The number of
    /// blocks hit: the ones that were not air when their turn came.
    pub(super) fn explosion_block_step(
        &mut self,
        targets: &mut [[i32; 3]],
        radius: f32,
        interaction: BlockInteraction,
        fire: bool,
        start: &ExplosionStart<'_>,
    ) -> Result<usize> {
        let mut hit = 0;
        if interaction == BlockInteraction::Trigger {
            self.explosion_trigger_blocks(targets)?;
        } else if interaction != BlockInteraction::Keep {
            // Util.shuffle(list, level.random)
            for i in (2..=targets.len()).rev() {
                let swap = self.world.random.next_int_bound(i as i32).unwrap_or(0) as usize;
                targets.swap(i - 1, swap);
            }
            let mut drops = ExplosionDrops::default();
            for &pos in targets.iter() {
                if self.explosion_hit_block(pos, radius, interaction, start, &mut drops)? {
                    hit += 1;
                }
            }
            self.spawn_explosion_drops(drops);
        }
        if fire {
            for &pos in targets.iter() {
                if self.world.random.next_int_bound(3).unwrap_or(1) == 0
                    && self
                        .res
                        .facts()
                        .is_air(self.world.block(pos[0], pos[1], pos[2]))
                    && self
                        .res
                        .facts()
                        .get(self.world.block(pos[0], pos[1] - 1, pos[2]))
                        .solid_render
                {
                    // BaseFireBlock.getState: soul fire on its base blocks,
                    // fire shaped by its neighbours elsewhere.
                    let at = BlockPos::new(pos[0], pos[1], pos[2]);
                    let fire =
                        self.with_level(|b, level, ticks| b.fire_state_for(level, ticks, at));
                    self.level_set(pos, fire)?;
                }
            }
        }
        Ok(hit)
    }
    fn explosion_hit_block(
        &mut self,
        pos: [i32; 3],
        radius: f32,
        interaction: BlockInteraction,
        explosion: &ExplosionStart<'_>,
        dropped: &mut ExplosionDrops,
    ) -> Result<bool> {
        let state = self.world.block(pos[0], pos[1], pos[2]);
        if self.res.facts().is_air(state) {
            return Ok(false);
        }
        let name = self.res.facts().get(state).name.clone();
        let is_tnt = &*name == "minecraft:tnt";
        // CreakingHeartBlock.onExplosionHit: its creaking dies, and a blast
        // whose indirect source is a player (a primed TNT's owner) awards
        // the heart's experience (`tryAwardExperience`).
        if &*name == "minecraft:creaking_heart" {
            // tryAwardExperience checks the TNT owner's own abilities.
            let award = match self.combat.blast_guest {
                Some(conn) => self
                    .command_guest(conn)
                    .is_some_and(|g| !g.creative() && !g.spectator()),
                None => self.combat.blast_owner_player && !self.creative && !self.spectator(),
            };
            self.creaking_heart_broken_by(pos, award);
        }
        // Block.onExplosionHit: the drops copy from the block entity, which
        // then spills its contents as the block turns to air.
        let key = (pos[0], pos[1], pos[2]);
        let is_hive = matches!(&*name, "minecraft:beehive" | "minecraft:bee_nest");
        // BeehiveBlock.getDrops: the bees of a hive blown up by TNT, a
        // creeper, a wither skull, a wither or a TNT minecart come out.
        if is_hive && self.combat.hive_emptying_blast {
            let facing = self
                .pack
                .assets
                .registry
                .state(state)
                .and_then(|s| s.properties.get("facing").cloned());
            self.empty_hive(
                pos,
                horizon_client::beehive::facing_step(facing.as_deref()),
                horizon_client::beehive::Release::Emergency,
            );
        }
        if let Some(entity) = self.world.block_entities.get(&key).cloned() {
            let custom = self.world.block_names.get(&key).cloned();
            self.pending
                .record(pos, &entity, &name, custom, self.world.block_lock(key));
        } else if let Some(tag) = super::spill::saved_entity(&self.world, pos) {
            self.pending.record_saved(pos, &tag);
        }
        // Block.dropFromExplosion (only TntBlock refuses), or what the
        // block's parts answer: it gates spawnAfterBreak and the loot.
        let drops = self
            .part_drops_from_explosion(pos, state, explosion)
            .unwrap_or(!is_tnt);
        // Block.onExplosionHit runs spawnAfterBreak before the drops.
        if drops && name.starts_with("minecraft:infested_") {
            self.spawn_infestation(pos);
        }
        // Its dropExperience is whether the blast's indirect source is a
        // player (a primed TNT's owner, not a mob's): ores, sculk and spawners
        // sample their experience then, and pop it while blocks drop.
        if drops && self.combat.blast_owner_player {
            let xp = horizon_world::level::removal::break_experience(&mut self.world.random, &name)
                .unwrap_or(0);
            if xp > 0
                && self
                    .ui
                    .rules
                    .bool_at(horizon_client::gamerules::vanilla::BLOCK_DROPS)
            {
                self.spawn_orbs(pos.map(|v| f64::from(v) + 0.5), xp);
            }
        }
        if drops {
            let dynamic = self.pending.dynamic(pos);
            // LootContextParams.EXPLOSION_RADIUS under DESTROY_WITH_DECAY: the
            // tables' survives_explosion and explosion_decay draw from their
            // random sequence.
            let decay = (interaction == BlockInteraction::DestroyWithDecay).then_some(radius);
            let place = self.loot_place_at(
                pos.map(|v| f64::from(v) + 0.5),
                self.loot_block_entity(pos, &name),
            );
            // `Explosion.getDirectSourceEntity`.
            let entity = self
                .combat
                .explosion_direct
                .or(self.combat.explosion_source);
            let mut drops = horizon_world::loot::with_native_loot_place(place, || {
                let roll = horizon_client::items::BlockLootRoll {
                    dynamic: &dynamic,
                    explosion: decay,
                    ..Default::default()
                };
                self.roll_block_loot(pos, state, entity, roll)
            });
            self.native_block_loot(pos, state, None, None, Some(radius), &mut drops);
            let mut pieces: Vec<ExplosionDropPiece> = Vec::new();
            let listened = mod_api::__private::listened(mod_api::HookId::BlockDrops);
            for mut stack in drops {
                if &*stack.item == "minecraft:air" || stack.count == 0 {
                    continue;
                }
                self.pending.attach(pos, &mut stack);
                let items = self.res.items();
                add_or_append_stack(
                    &mut dropped.stacks,
                    stack,
                    pos,
                    &|s| items.get(&s.item).map_or(64, |i| i.max_stack),
                    |group, count| {
                        if !listened {
                            return;
                        }
                        if let Some(piece) = pieces.iter_mut().find(|p| p.group == group) {
                            piece.count += count;
                        } else {
                            pieces.push(ExplosionDropPiece { group, count });
                        }
                    },
                );
            }
            if listened {
                dropped
                    .blocks
                    .push(ExplosionBlockDrops { pos, state, pieces });
            }
        }
        self.pending.clear(pos);
        self.notify_block_exploded(pos, state, explosion);
        self.level_set(pos, self.world.air)?;
        // BeehiveBlock.onExplosionHit: the bees about turn on the player.
        if is_hive {
            self.anger_nearby_bees(pos);
        }
        // TntBlock.wasExploded: new PrimedTnt (its push drawn first), then
        // getRandomShortFuse(80); no game event.
        if is_tnt
            && self
                .ui
                .rules
                .bool_at(horizon_client::gamerules::vanilla::TNT_EXPLODES)
        {
            let r = &mut self.world.random;
            let rot = r.next_double() * f64::from(std::f32::consts::TAU);
            let fuse = r.next_int_bound(20).unwrap_or(0) + 10;
            // TntBlock.wasExploded hands the new TNT the blast's indirect
            // source, so a chain reaction keeps a primed TNT's owner, player or mob.
            let (owner, guest) = match &self.combat.explicit_explosion_cause {
                // A crystal/cart's explicit DamageSource does not change
                // Explosion.getIndirectSourceEntity, which is null for them.
                Some(_) => (None, None),
                None => match &self.combat.blast_owner {
                    Some(_) if self.combat.blast_owner_player => {
                        (Some(Who::Player), self.combat.blast_guest)
                    }
                    Some((id, _)) => (Some(Who::Mob(*id)), None),
                    // The mob that causes it (a creeper, a mod blast's source).
                    None => (self.blast_attacker().map(|(id, _)| Who::Mob(id)), None),
                },
            };
            self.add_primed_tnt(
                BlockPos::new(pos[0], pos[1], pos[2]),
                rot,
                fuse,
                owner,
                guest,
            );
        }
        Ok(true)
    }
    /// Build the merged entities before the events, without extra level-random
    /// draws. Each block owns only its contribution to a merged stack: canceling
    /// it cannot take another block's loot. Unedited contributions rejoin their
    /// original entity; edited or added entities spawn exactly as returned.
    fn spawn_explosion_drops(&mut self, drops: ExplosionDrops) {
        if !mod_api::__private::listened(mod_api::HookId::BlockDrops) {
            for (pos, stack) in drops.stacks {
                self.pop_resource(pos, stack);
            }
            return;
        }
        let items: Vec<_> = self.with_level(|_, level, _| {
            drops
                .stacks
                .into_iter()
                .map(|(pos, stack)| level.block_drop_item(pos, stack))
                .collect()
        });
        let mut merged: Vec<Option<super::spill::BlockItemDrop>> = vec![None; items.len()];
        let mut edited = Vec::new();
        for block in drops.blocks {
            let mut originals: Vec<_> = block
                .pieces
                .into_iter()
                .map(|piece| {
                    let mut item = items[piece.group].clone();
                    item.stack.count = piece.count;
                    (piece, item)
                })
                .collect();
            let dropped = self.with_level(|_, level, _| {
                level.block_drops_ready(
                    BlockPos::new(block.pos[0], block.pos[1], block.pos[2]),
                    block.state,
                    None,
                    None,
                    super::spill::BlockDrops {
                        items: originals.iter().map(|(_, item)| item.clone()).collect(),
                        experience: 0,
                        canceled: false,
                    },
                )
            });
            for item in dropped.items {
                if let Some(index) = originals.iter().position(|(_, original)| *original == item) {
                    let (piece, _) = originals.swap_remove(index);
                    match &mut merged[piece.group] {
                        Some(existing) => existing.stack.count += item.stack.count,
                        slot @ None => *slot = Some(item),
                    }
                } else {
                    edited.push(item);
                }
            }
            // Keep the blast path's zero base XP; the event may award some.
            if dropped.experience > 0 {
                self.spawn_orbs(block.pos.map(|v| f64::from(v) + 0.5), dropped.experience);
            }
        }
        for item in merged.into_iter().flatten().chain(edited) {
            // A fresh constructor random, as `spawn_item` draws, with the state a hook set.
            let random = None;
            self.add_item_drop(super::spill::BlockItemDrop { random, ..item });
        }
    }

    /// ServerExplosion: rays from a 16^3 grid lose (resistance + 0.3) * 0.3 per step.
    /// A mob explosion (`ServerLevel.explode` with `ExplosionInteraction.MOB`:
    /// blocks are kept without mobGriefing, else the mob_explosion_drop_decay
    /// rule picks decay).
    pub(super) fn explode(
        &mut self,
        source: Option<u32>,
        center: [f64; 3],
        radius: f32,
        blast: ExplosionSource,
    ) -> Result<()> {
        self.explode_with(source, center, radius, false, blast)
    }
    /// `ServerLevel.explode` with `ExplosionInteraction.MOB` and the caller's fire flag.
    pub(super) fn explode_mob(
        &mut self,
        center: [f64; 3],
        radius: f32,
        fire: bool,
        blast: ExplosionSource,
    ) -> Result<()> {
        self.explode_with(None, center, radius, fire, blast)
    }
    /// `SulfurCube.tickFuse`: `ExplosionInteraction.TNT` (the
    /// tnt_explosion_drop_decay rule) with mobGriefing, `NONE` without.
    pub(super) fn explode_sulfur_cube(
        &mut self,
        source: Option<u32>,
        center: [f64; 3],
        radius: f32,
        blast: ExplosionSource,
    ) -> Result<()> {
        let interaction = if !self.can_entity_grief(source) {
            BlockInteraction::Keep
        } else {
            self.destroy_type(horizon_client::gamerules::vanilla::TNT_EXPLOSION_DROP_DECAY)
        };
        self.server_explode_from(center, radius, interaction, false, blast)
    }
}

#[cfg(test)]
impl Game<'_> {
    /// Exercise the real explosion entity walk on an empty, headless world.
    pub(super) fn run_explosion_items_fixture(
        &mut self,
        directory: &std::path::Path,
    ) -> Result<()> {
        #[derive(serde::Deserialize)]
        struct Case {
            name: String,
            stack: String,
            health: i32,
            distances: Vec<f64>,
            #[serde(default)]
            sibling: bool,
        }
        let cases: Vec<Case> =
            serde_json::from_str(&std::fs::read_to_string(directory.join("inputs.json"))?)?;
        let mut output = Vec::new();
        for case in cases {
            self.dropped.clear();
            let stack = horizon_client::item_data::parse_stack(&case.stack)?
                .ok_or("empty fixture stack")?;
            self.spawn_item(stack, [0.0, 100.0, 0.0], [0.0; 3], 0);
            self.dropped[0].health = case.health;
            let original = self.dropped[0].id;
            if case.sibling {
                self.spawn_item(
                    ItemStack::new("minecraft:dirt", 1),
                    [0.0, 100.0, 0.0],
                    [0.0; 3],
                    0,
                );
            }
            let mut steps = Vec::new();
            for distance in case.distances {
                for item in &mut self.dropped {
                    item.position = Vec3::new(distance, 100.0, 0.0);
                }
                self.explosion_hurt_entities(
                    [0.0, 100.0, 0.0],
                    1.0,
                    "minecraft:explosion",
                    BlockInteraction::Destroy,
                    &[],
                );
                let mut remaining: Vec<_> = self
                    .dropped
                    .iter()
                    .map(|item| (item.stack.item.to_string(), item.stack.count, item.health))
                    .collect();
                remaining.sort();
                steps.push(serde_json::json!(remaining));
                if !self.dropped.iter().any(|item| item.id == original) {
                    break;
                }
            }
            output.push(serde_json::json!({"name": case.name, "steps": steps}));
        }
        std::fs::write(directory.join("rust.json"), serde_json::to_vec(&output)?)?;
        Ok(())
    }
}

/// `ServerExplosion.addOrAppendStack`: `ItemEntity.areMergable` (same item and
/// components, combined count within the item's max stack size), then
/// `ItemEntity.merge(.., 16)`.
fn add_or_append_stack(
    stacks: &mut Vec<([i32; 3], ItemStack)>,
    mut stack: ItemStack,
    pos: [i32; 3],
    max_stack: &dyn Fn(&ItemStack) -> u32,
    mut piece: impl FnMut(usize, u32),
) {
    let max = i64::from(max_stack(&stack));
    for (index, (_, existing)) in stacks.iter_mut().enumerate() {
        if existing.same_item_same_components(&stack)
            && i64::from(existing.count) + i64::from(stack.count) <= max
        {
            let moved = (max.min(16) - i64::from(existing.count))
                .min(i64::from(stack.count))
                .max(0) as u32;
            existing.count += moved;
            stack.count -= moved;
            if moved > 0 {
                piece(index, moved);
            }
            if stack.count == 0 {
                return;
            }
        }
    }
    piece(stacks.len(), stack.count);
    stacks.push((pos, stack));
}

/// The answers of the parts that declare their explosion resistance pure, by
/// state, for one read.
pub(super) type PartAnswers = std::collections::HashMap<BlockStateId, Option<f32>>;

/// What decides a blast's resistance in a cell besides the cell itself: the
/// damage calculator's overrides (`ExplosionDamageCalculator` and the ones
/// that replace it).
pub(super) struct BlastRules {
    /// One block that resists this much (a respawn anchor's water cell).
    resists: Option<([i32; 3], f32)>,
    /// A dangerous wither skull's cap on what the wither may destroy.
    cap: Option<f32>,
    /// The blast's direct source, which `cap` asks about the block.
    direct: Option<u32>,
    /// `#wither_immune`, read while `cap` is set.
    wither_immune: Option<HashSet<String>>,
    /// A primed TNT minecart's: its rails and the blocks under them neither
    /// resist nor break.
    rail_proof: bool,
    /// `USED_PORTAL_DAMAGE_CALCULATOR`: a nether portal neither resists nor breaks.
    portal_proof: bool,
}

/// `WitherSkull.getBlockExplosionResistance`: `isDangerous() &&
/// WitherBoss.canDestroy(state) ? min(0.8, resistance) : resistance`. Air has
/// no resistance to begin with, and a block the skull may not destroy
/// (`kept`: `#wither_immune`, or as its parts answer) keeps its own.
fn blast_resistance(base: f32, cap: Option<f32>, kept: bool) -> f32 {
    match cap {
        Some(cap) if !kept => base.min(cap),
        _ => base,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DropProbe {
        edit: bool,
        seen: std::sync::Arc<std::sync::Mutex<Vec<[i32; 3]>>>,
    }

    #[mod_api::__subscriptions]
    impl mod_api::Hooks for DropProbe {
        fn loot_drops(
            &self,
            _: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::LootDropsEvent<'_>,
        ) {
            if self.edit {
                *event.drops = vec![
                    mod_api::ItemStack::new(
                        mod_api::__private::item_key("minecraft:diamond").unwrap(),
                        1,
                    )
                    .unwrap(),
                ];
            }
        }
        fn block_drops(
            &self,
            _: &dyn mod_api::LevelHost,
            event: &mut mod_api::BlockDropsEvent<'_>,
        ) -> mod_api::Flow {
            let pos = [event.pos.x, event.pos.y, event.pos.z];
            self.seen.lock().unwrap().push(pos);
            assert!(event.breaker.is_none() && event.tool.is_none());
            assert_eq!(event.drops.len(), 1);
            assert_eq!(
                event.drops[0].stack.count(),
                1,
                "only this block's contribution"
            );
            if self.edit {
                assert_eq!(
                    mod_api::__private::item_key_id(event.drops[0].stack.item()).unwrap(),
                    "minecraft:diamond",
                    "loot modifiers ran first"
                );
                event.experience = 5;
                if matches!(event.pos.x, 1 | 4) {
                    return mod_api::Flow::Cancel;
                }
                *event.drops = vec![ItemDrop::new(
                    mod_api::ItemStack::new(
                        mod_api::__private::item_key("minecraft:emerald").unwrap(),
                        3,
                    )
                    .unwrap(),
                    [7.0, 80.0, 7.0],
                    [0.1, 0.2, 0.3],
                    42,
                )];
            }
            mod_api::Flow::Continue
        }
    }

    /// A no-op listener preserves cross-block merging and level-random draws.
    /// Edits/cancel affect only the originating block, after loot modifiers, on
    /// both real caller paths; cancel also suppresses a mob break's silverfish.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES and an isolated process for native mods"]
    fn explosion_and_mob_breaks_apply_block_drops() {
        use crate::game::native_entity_refs::tests::with_game;
        use horizon_client::mobs::MobEvent;
        use std::sync::{Arc, Mutex};
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
            let dirt = game
                .pack
                .assets
                .registry
                .default_state("minecraft:dirt")
                .unwrap();
            let infested = game
                .pack
                .assets
                .registry
                .default_state("minecraft:infested_stone")
                .unwrap();
            let blast = |game: &mut Game<'_>| {
                game.dropped.clear();
                game.orbs.clear();
                game.world.random.set_seed(7);
                let mut drops = ExplosionDrops::default();
                for pos in [[1, 64, 1], [2, 64, 1]] {
                    game.world.set(pos[0], pos[1], pos[2], dirt).unwrap();
                    game.explosion_hit_block(
                        pos,
                        4.0,
                        BlockInteraction::Destroy,
                        &ExplosionStart::from(mod_api::__private::built::ExplosionStart {
                            dimension: "minecraft:overworld",
                            center: [0.0; 3],
                            radius: 4.0,
                            fire: false,
                            interaction: mod_api::BlockInteraction::Destroy,
                            damage_type: "minecraft:explosion",
                            source: None,
                            owner: None,
                        }),
                        &mut drops,
                    )
                    .unwrap();
                }
                game.spawn_explosion_drops(drops);
                game.world.random.next_int()
            };
            let baseline = blast(game);
            assert_eq!(game.dropped.len(), 1);
            assert_eq!(game.dropped[0].stack.count, 2);
            let position = game.dropped[0].position;
            let seen = Arc::new(Mutex::new(Vec::new()));
            let mods = |edit| {
                vec![mod_api::ModEntry::new(
                    "drop-probe",
                    Box::new(DropProbe {
                        edit,
                        seen: seen.clone(),
                    }),
                )]
                .into()
            };
            mod_platform::testing::with_mods(mods(false), || {
                assert_eq!(blast(game), baseline);
                assert_eq!(game.dropped.len(), 1);
                assert_eq!(game.dropped[0].stack.count, 2);
                assert_eq!(game.dropped[0].position, position);
                assert!(game.orbs.is_empty(), "explosions default to no XP");
            });
            assert_eq!(*seen.lock().unwrap(), [[1, 64, 1], [2, 64, 1]]);
            seen.lock().unwrap().clear();
            mod_platform::testing::with_mods(mods(true), || {
                blast(game);
                let check = |game: &Game<'_>| {
                    assert_eq!(game.dropped.len(), 1);
                    let item = &game.dropped[0];
                    assert_eq!(&*item.stack.item, "minecraft:emerald");
                    assert_eq!(item.stack.count, 3);
                    assert_eq!(item.position, Vec3::new(7.0, 80.0, 7.0));
                    assert_eq!(item.velocity, Vec3::new(0.1, 0.2, 0.3));
                    assert_eq!(item.pickup_delay, 42);
                    assert_eq!(
                        game.orbs
                            .iter()
                            .map(|orb| orb.points() * orb.count)
                            .sum::<u32>(),
                        5
                    );
                };
                check(game);
                for (x, state) in [(3, dirt), (4, infested)] {
                    game.dropped.clear();
                    game.orbs.clear();
                    game.world.set(x, 64, 1, state).unwrap();
                    game.handle_mob_events(vec![MobEvent::BreakBlock {
                        pos: [x, 64, 1],
                        drop: true,
                        by: None,
                        breaker: None,
                    }])
                    .unwrap();
                    if x == 3 {
                        check(game);
                    } else {
                        assert!(game.dropped.is_empty() && game.orbs.is_empty());
                        assert!(game.mobs.is_empty(), "cancel suppresses spawnAfterBreak");
                    }
                    assert_eq!(game.world.block(x, 64, 1), game.world.air);
                }
                game.ui
                    .rules
                    .set("block_drops", horizon_client::gamerules::Value::Bool(false));
                game.world.set(5, 64, 1, dirt).unwrap();
                game.handle_mob_events(vec![MobEvent::BreakBlock {
                    pos: [5, 64, 1],
                    drop: true,
                    by: None,
                    breaker: None,
                }])
                .unwrap();
                assert!(game.dropped.is_empty() && game.orbs.is_empty());
            });
            assert_eq!(
                *seen.lock().unwrap(),
                [[1, 64, 1], [2, 64, 1], [3, 64, 1], [4, 64, 1], [5, 64, 1]]
            );
            game.ui
                .rules
                .set("block_drops", horizon_client::gamerules::Value::Bool(true));
            game.world.set(6, 64, 1, infested).unwrap();
            game.handle_mob_events(vec![MobEvent::BreakBlock {
                pos: [6, 64, 1],
                drop: true,
                by: None,
                breaker: None,
            }])
            .unwrap();
            assert_eq!(game.world.block(6, 64, 1), game.world.air);
            assert_eq!(
                game.mobs.len(),
                1,
                "allowed spawnAfterBreak runs exactly once"
            );
            assert_eq!(
                game.mobs[0].species(),
                horizon_client::mobs::Species::Silverfish
            );
        });
    }

    /// `exploded_positions` before rays reused their last block's lookup:
    /// every step looked its block up.
    fn exploded_positions_stepwise(
        game: &mut Game<'_>,
        center: [f64; 3],
        radius: f32,
        resists: Option<([i32; 3], f32)>,
    ) -> Vec<[i32; 3]> {
        let (min_y, max_y) = game.with_level(|_, level, _| {
            use horizon_world::level::LevelAccess;
            (level.min_y(), level.max_y())
        });
        let mut set: JavaHashMap<Key, ()> = JavaHashMap::default();
        for xx in 0..16 {
            for yy in 0..16 {
                for zz in 0..16 {
                    if !(xx == 0 || xx == 15 || yy == 0 || yy == 15 || zz == 0 || zz == 15) {
                        continue;
                    }
                    let mut xd = f64::from(xx as f32 / 15.0 * 2.0 - 1.0);
                    let mut yd = f64::from(yy as f32 / 15.0 * 2.0 - 1.0);
                    let mut zd = f64::from(zz as f32 / 15.0 * 2.0 - 1.0);
                    let d = (xd * xd + yd * yd + zd * zd).sqrt();
                    (xd, yd, zd) = (xd / d, yd / d, zd / d);
                    let mut power = radius * (0.7 + game.world.random.next_float() * 0.6);
                    let (mut xp, mut yp, mut zp) = (center[0], center[1], center[2]);
                    while power > 0.0 {
                        let pos = [xp.floor() as i32, yp.floor() as i32, zp.floor() as i32];
                        if pos[1] < min_y || pos[1] >= max_y {
                            break;
                        }
                        let state = game.world.block(pos[0], pos[1], pos[2]);
                        let f = game.res.facts().get(state);
                        let fluid = if f.fluid != Fluid::None {
                            100.0f32
                        } else {
                            0.0
                        };
                        match resists {
                            Some((at, resistance)) if at == pos => {
                                power -= (resistance + 0.3) * 0.3;
                            }
                            _ if !game.res.facts().is_air(state) || f.fluid != Fluid::None => {
                                let base = f.explosion_resistance.max(fluid);
                                power -= (blast_resistance(base, None, false) + 0.3) * 0.3;
                            }
                            _ => {}
                        }
                        if power > 0.0 {
                            set.put(Key(pos), ());
                        }
                        xp += xd * f64::from(0.3f32);
                        yp += yd * f64::from(0.3f32);
                        zp += zd * f64::from(0.3f32);
                        power -= 0.225_000_01;
                    }
                }
            }
        }
        set.entries().map(|(k, _)| k.0).collect()
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn rays_reusing_their_last_block_explode_what_each_step_did() {
        let mut resources = None;
        let mut game = crate::game::game_tests::tests::server_game(&mut resources);
        let registry = &game.pack.assets.registry;
        let states: Vec<BlockStateId> = [
            "minecraft:stone",
            "minecraft:air",
            "minecraft:water",
            "minecraft:dirt",
            "minecraft:obsidian",
            "minecraft:air",
            "minecraft:oak_planks",
            "minecraft:glass",
        ]
        .iter()
        .map(|n| registry.default_state(n).unwrap())
        .collect();
        // A jumble of blocks of every resistance around the blast, in the
        // loaded chunk at the origin.
        for x in 0..16 {
            for z in 0..16 {
                for y in 56..74 {
                    let i = ((x * 7 + y * 3 + z * 5) as usize) % states.len();
                    game.world.set(x, y, z, states[i]).unwrap();
                }
            }
        }
        let center = [8.3, 64.6, 7.9];
        for (radius, resists) in [(4.0, None), (6.0, None), (4.0, Some(([8, 64, 8], 1200.0)))] {
            game.world.random.set_seed(7);
            let expected = exploded_positions_stepwise(&mut game, center, radius, resists);
            game.world.random.set_seed(7);
            let start = ExplosionStart::from(mod_api::__private::built::ExplosionStart {
                dimension: "minecraft:overworld",
                center,
                radius,
                fire: false,
                interaction: mod_api::BlockInteraction::Destroy,
                damage_type: "minecraft:explosion",
                source: None,
                owner: None,
            });
            let got = game.exploded_positions(center, radius, resists, false, &start);
            assert!(expected.len() > 20, "{radius}: {}", expected.len());
            assert_eq!(got, expected, "radius {radius}, {resists:?}");
        }
    }

    #[test]
    #[ignore = "requires prepared resources; tools/test_explosion_items_parity.py"]
    fn explosion_items_parity_fixture() {
        let directory = std::env::var_os("HORIZON_EXPLOSION_ITEMS_FIXTURE")
            .expect("set HORIZON_EXPLOSION_ITEMS_FIXTURE to the fixture directory");
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
            );
            game.run_explosion_items_fixture(std::path::Path::new(&directory))
                .unwrap();
        });
    }

    /// A dangerous wither skull's blast treats a block as at most 0.8 harder to
    /// break, but a `#wither_immune` block keeps its own resistance and any
    /// other blast is unchanged.
    #[test]
    fn a_dangerous_wither_skull_caps_block_resistance() {
        assert_eq!(blast_resistance(4.0, Some(0.8), false), 0.8);
        assert_eq!(blast_resistance(0.3, Some(0.8), false), 0.3, "already low");
        assert_eq!(blast_resistance(4.0, Some(0.8), true), 4.0, "wither immune");
        assert_eq!(blast_resistance(4.0, None, false), 4.0, "another blast");
    }

    /// A dropped item survives a mob's blast untouched while `mobGriefing` is
    /// off, and is thrown and hurt as usual by one that destroys blocks or by
    /// any blast with the rule back on.
    #[test]
    fn items_ignore_a_blast_that_keeps_blocks_with_griefing_off() {
        assert!(items_ignore_explosion(false, BlockInteraction::Keep));
        assert!(!items_ignore_explosion(false, BlockInteraction::Destroy));
        assert!(!items_ignore_explosion(
            false,
            BlockInteraction::DestroyWithDecay
        ));
        assert!(!items_ignore_explosion(true, BlockInteraction::Keep));
    }

    #[test]
    fn blasts_reach_entities_within_their_diameter() {
        // A TNT blast: radius 4, so the entity pass reaches 8 blocks out.
        let center = [0.5, 64.0, 0.5];
        assert!(blast_reaches(center, 8.0, [8.5, 64.0, 0.5]));
        assert!(blast_reaches(center, 8.0, [-3.5, 68.0, 4.5]));
        assert!(!blast_reaches(center, 8.0, [8.6, 64.0, 0.5]));
    }

    /// A zombie's type with only what a hurt and a push read.
    fn zombie_type() -> std::sync::Arc<horizon_client::mobs::MobType> {
        use horizon_client::mobs::{Anim, Brain, MobType, Part, Species};
        std::sync::Arc::new(MobType {
            shadow: None,
            id: "minecraft:zombie".into(),
            species: Species::Zombie,
            width: 0.6,
            height: 1.95,
            eye_height: 1.74,
            category: "monster".into(),
            max_health: 20.0,
            speed: 0.23,
            flying_speed: 0.02,
            attack_damage: 3.0,
            attack_knockback: 0.0,
            follow_range: 35.0,
            step_height: 0.6,
            armor: 2.0,
            armor_toughness: 0.0,
            knockback_resistance: 0.0,
            jump_strength: 0.42,
            attributes: Default::default(),
            item_stack_limits: Default::default(),
            attribute_bounds: Default::default(),
            gravity: 0.08,
            safe_fall_distance: 3.0,
            fall_damage_multiplier: 1.0,
            water_efficiency: 0.0,
            water_slowdown: 0.8,
            air_speed: 0.02,
            baby_scale: 0.5,
            head_rot_speed: 10.0,
            max_head_x_rot: 40.0,
            max_head_y_rot: 75.0,
            ambient_interval: 80,
            flags: Default::default(),
            model: std::sync::Arc::new(Part {
                name: "root".into(),
                pose: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
                polygons: Vec::new(),
                children: Vec::new(),
            }),
            layers: Default::default(),
            texture: String::new(),
            brain: Brain::Passive,
            anim: Anim::Humanoid { zombie_arms: true },
            sounds: ["", "", ""],
            sound_overrides: Default::default(),
            food: Default::default(),
            tempt: Default::default(),
            passenger_attachments: Vec::new(),
        })
    }

    #[test]
    fn a_mob_that_refuses_the_hit_is_still_pushed() {
        let mut test_level = horizon_client::health::TestLevel::new(0);
        let level = &mut test_level.level();
        use horizon_client::mobs::{Damage, DamageKind, Mob, Species};
        let hit = (10.0, Damage::of(DamageKind::Explosion));
        let push = Vec3::new(0.5, 0.25, -0.5);
        let mob = |id, species| {
            let mut kind = (*zombie_type()).clone();
            kind.species = species;
            Mob::new(id, std::sync::Arc::new(kind), Vec3::ZERO, 0.0)
        };
        let pushed = |m: &Mob| {
            assert_eq!(m.velocity, Vec3::new(0.5, 0.25, -0.5));
            assert!(m.needs_sync);
        };
        // An ordinary mob is hurt and pushed.
        let mut m = mob(1, Species::Zombie);
        let health = m.health;
        hurt_mob(level, &mut m, true, None, hit);
        m.push_from_explosion(push);
        assert!(m.health < health);
        pushed(&m);
        // An invulnerable one refuses the hit, and is pushed all the same.
        let mut m = mob(2, Species::Zombie);
        m.invulnerable_flag = true;
        let health = m.health;
        hurt_mob(level, &mut m, true, None, hit);
        m.push_from_explosion(push);
        assert_eq!(m.health, health);
        pushed(&m);
        // A team mate of the exploding mob is not hurt, but pushed.
        let mut m = mob(3, Species::Zombie);
        let health = m.health;
        hurt_mob(level, &mut m, false, None, hit);
        m.push_from_explosion(push);
        assert_eq!(m.health, health);
        pushed(&m);
        // A wither shrugs off a blast an undead caused, and is pushed.
        let mut m = mob(4, Species::Wither);
        let health = m.health;
        hurt_mob(level, &mut m, true, Some(Species::Skeleton), hit);
        m.push_from_explosion(push);
        assert_eq!(m.health, health);
        pushed(&m);
    }

    #[test]
    fn mob_griefing_off_spares_decorations_from_kept_and_mob_blasts() {
        use BlockInteraction::{Destroy, DestroyWithDecay, Keep};
        // mobGriefing on: every blast breaks them, a creeper's included.
        assert!(!blast_spares_decorations(
            true,
            DestroyWithDecay,
            false,
            false
        ));
        assert!(!blast_spares_decorations(true, Keep, false, false));
        // Off: TNT still breaks them (ExplosionInteraction.TNT destroys blocks).
        assert!(!blast_spares_decorations(false, Destroy, false, false));
        assert!(!blast_spares_decorations(
            false,
            DestroyWithDecay,
            false,
            false
        ));
        // Off: a blast that keeps blocks, or one a refused mob causes, does not.
        assert!(blast_spares_decorations(false, Keep, false, false));
        assert!(blast_spares_decorations(false, Destroy, true, false));
        // A source in water spares them whatever the rules (`BlockAttachedEntity.ignoreExplosion`).
        assert!(blast_spares_decorations(
            true,
            DestroyWithDecay,
            false,
            true
        ));
        assert!(blast_spares_decorations(true, Destroy, true, true));
    }

    /// The type a blast hurts with and the attacker its death message names
    /// come from one cause (`DamageSources.explosion`): `player_explosion`
    /// "blown up by" that mob, or `explosion` with no one when it has none.
    fn blast_hurt(
        owner: Option<u32>,
        exploding: Option<u32>,
        live: &[(u32, Text)],
        exploded: &[(u32, Text)],
    ) -> (&'static str, Option<(u32, Text)>) {
        let find = |id| live.iter().find(|(m, _)| *m == id).map(|(_, n)| n.clone());
        let cause = blast_cause(owner, exploding, find, exploded);
        let kind = horizon_client::mobs::DamageKind::Explosion;
        (
            super::super::creatures::damage_type(kind, cause.is_some()),
            cause,
        )
    }

    #[test]
    fn a_caused_blast_names_its_cause_under_player_explosion() {
        let creeper = Text::translate("entity.minecraft.creeper", &[]);
        let ghast = Text::translate("entity.minecraft.ghast", &[]);
        // A creeper's blast: it left the mob list, its event named it.
        assert_eq!(
            blast_hurt(None, Some(7), &[], &[(7, creeper.clone())]),
            ("minecraft:player_explosion", Some((7, creeper)))
        );
        // A ghast's fireball: its owner is still in the level.
        assert_eq!(
            blast_hurt(Some(3), None, &[(3, ghast.clone())], &[]),
            ("minecraft:player_explosion", Some((3, ghast.clone())))
        );
        // The ghast was removed before its fireball landed: no owner.
        assert_eq!(
            blast_hurt(Some(3), None, &[], &[]),
            ("minecraft:explosion", None)
        );
        // Nothing caused it.
        assert_eq!(
            blast_hurt(None, None, &[], &[]),
            ("minecraft:explosion", None)
        );
        // A projectile's owner wins over the exploding mob, and a removed
        // owner gives no cause even when a mob is exploding.
        let zombie = Text::translate("entity.minecraft.zombie", &[]);
        let live = [(3, ghast.clone()), (5, zombie.clone())];
        assert_eq!(
            blast_hurt(Some(3), Some(5), &live, &[]),
            ("minecraft:player_explosion", Some((3, ghast)))
        );
        assert_eq!(
            blast_hurt(Some(4), Some(5), &live, &[]),
            ("minecraft:explosion", None)
        );
        // A mob still in the level is named as it is now, before its name
        // in the exploded list.
        let named = Text::literal("Bob".to_owned());
        assert_eq!(
            blast_hurt(None, Some(5), &live, &[(5, named)]),
            ("minecraft:player_explosion", Some((5, zombie)))
        );
    }

    #[test]
    fn block_pos_hash_matches_vec3i() {
        // Vec3i.hashCode: (y + z * 31) * 31 + x
        assert_eq!(Key([1, 2, 3]).java_hash(), (2 + 3 * 31) * 31 + 1);
        assert_eq!(Key([-5, 64, -7]).java_hash(), (64 - 7 * 31) * 31 - 5);
        assert_eq!(
            Key([0, 1, 0]).comparable_compare(&Key([5, 0, 9])),
            Some(Ordering::Greater)
        );
        assert_eq!(
            Key([1, 0, 2]).comparable_compare(&Key([0, 0, 3])),
            Some(Ordering::Less)
        );
    }

    #[test]
    fn drops_merge_up_to_sixteen() {
        let stack = |item: &str| ItemStack {
            item: item.into(),
            count: 1,
            damage: 0,
            extra: None,
        };
        let max = |s: &ItemStack| {
            if &*s.item == "minecraft:diamond_sword" {
                1
            } else {
                64
            }
        };
        let mut stacks = Vec::new();
        for _ in 0..20 {
            add_or_append_stack(
                &mut stacks,
                stack("minecraft:dirt"),
                [0, 0, 0],
                &max,
                |_, _| {},
            );
        }
        add_or_append_stack(
            &mut stacks,
            stack("minecraft:stone"),
            [1, 0, 0],
            &max,
            |_, _| {},
        );
        let counts: Vec<u32> = stacks.iter().map(|(_, s)| s.count).collect();
        assert_eq!(counts, vec![16, 4, 1]);
    }

    #[test]
    fn unstackable_drops_stay_separate() {
        let sword = ItemStack {
            item: "minecraft:diamond_sword".into(),
            count: 1,
            damage: 0,
            extra: None,
        };
        let max = |_: &ItemStack| 1;
        let mut stacks = Vec::new();
        add_or_append_stack(&mut stacks, sword.clone(), [0, 0, 0], &max, |_, _| {});
        add_or_append_stack(&mut stacks, sword, [0, 0, 0], &max, |_, _| {});
        let counts: Vec<u32> = stacks.iter().map(|(_, s)| s.count).collect();
        assert_eq!(counts, vec![1, 1]);
    }

    #[test]
    fn drops_with_different_damage_stay_separate() {
        let pick = |damage| ItemStack {
            item: "minecraft:iron_pickaxe".into(),
            count: 1,
            damage,
            extra: None,
        };
        let max = |_: &ItemStack| 64;
        let mut stacks = Vec::new();
        add_or_append_stack(&mut stacks, pick(0), [0, 0, 0], &max, |_, _| {});
        add_or_append_stack(&mut stacks, pick(5), [0, 0, 0], &max, |_, _| {});
        assert_eq!(stacks.len(), 2);
    }

    /// A blast and a burst of wind cast their rays at, and push from the eyes of,
    /// the mob as it stands: a baby's and a size-4 slime's own box and eye
    /// height, not the adult kind's (`Entity.getBoundingBox`, `getEyeHeight`).
    #[test]
    fn a_scaled_mob_is_blasted_by_its_own_box_and_eye_height() {
        use crate::game::entity::Entity;
        use horizon_client::mobs::{Mob, Species};
        let mob = |species| {
            let mut kind = (*zombie_type()).clone();
            kind.species = species;
            Mob::new(1, std::sync::Arc::new(kind), Vec3::ZERO, 0.0)
        };
        let adult = mob(Species::Zombie);
        assert_eq!(adult.explosion_box(), adult.bounds());
        let mut baby = mob(Species::Zombie);
        baby.extra.age = -24000;
        let mut slime = mob(Species::Slime);
        slime.extra.size = 4;
        for m in [&baby, &slime] {
            let b = Entity::explosion_box(m);
            assert_eq!(b, m.bounds());
            assert_eq!(Entity::explosion_eye_height(m), m.eye_height());
            assert_ne!(
                f64::from(m.height()),
                f64::from(adult.height()),
                "the case must scale the box"
            );
            assert_eq!(
                b.max_position().y - b.min_position().y,
                f64::from(m.height())
            );
        }
    }
}

#[cfg(test)]
#[path = "explosion/explosion_part_tests.rs"]
mod explosion_part_tests;

#[cfg(test)]
mod shulker_lock_drop_tests {
    use super::super::game_tests::tests::server_game;
    use super::*;
    use horizon_client::{containers::BlockEntity, save};
    use horizon_core::nbt::Tag;

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES and scratch homes/cwd"]
    fn destruction_captures_the_external_shulker_lock_before_removal() {
        let mut resources = None;
        let mut game = server_game(&mut resources);
        game.build_registries();
        game.set_allow_commands(true);
        game.block_ticks
            .add_empty_chunk(BlockPos::new(0, 0, 0).chunk())
            .unwrap();
        assert!(game.run_command("fill 0 199 0 15 199 15 stone"));
        assert!(
            game.ui
                .rules
                .bool_at(horizon_client::gamerules::vanilla::BLOCK_DROPS),
            "actual drop rule must be enabled"
        );
        game.creative = false;
        game.selected = 0;
        game.inventory[0] = Some(ItemStack::new("minecraft:diamond_pickaxe", 1));
        let clock = game.world.game_time;
        let mut missing = Vec::new();
        for (x, z, kind, locked, route) in [
            (3, 3, "shulker_box", true, "command"),
            (11, 3, "shulker_box", true, "player"),
            (3, 11, "shulker_box", true, "explosion"),
            (11, 11, "red_shulker_box", false, "command"),
            (7, 3, "chest", true, "command"),
            (7, 11, "crafter", true, "command"),
        ] {
            let shulker = kind.ends_with("shulker_box");
            let mut fields = Vec::new();
            if shulker {
                fields.push("CustomName:\"Box\"");
                fields.push("Items:[{Slot:0b,id:\"minecraft:diamond\",count:3,components:{\"minecraft:custom_data\":{h211:1}}},{Slot:26b,id:\"minecraft:apple\",count:5}]");
            }
            if locked {
                fields.push("lock:{items:\"minecraft:diamond\"}");
            }
            assert!(game.run_command(&format!(
                "setblock {x} 200 {z} minecraft:{kind}{{{}}}",
                fields.join(",")
            )));
            let pos = [x, 200, z];
            let key = (x, 200, z);
            assert_eq!(
                &*game.res.facts().get(game.world.block(x, 199, z)).name,
                "minecraft:stone",
                "positive floor"
            );
            let actual_lock = game.world.block_lock(key).cloned();
            assert_eq!(actual_lock.is_some(), locked, "actual external source lock");
            if locked {
                assert!(matches!(&actual_lock,Some(Tag::Compound(c)) if !c.is_empty()));
            }
            let entity = game
                .world
                .block_entities
                .get(&key)
                .expect("actual command-created source");
            assert!(
                save::block_entity_encode::encode(key, entity, &format!("minecraft:{kind}"))
                    .get("lock")
                    .is_none(),
                "external lock must not be invented in modeled serializer"
            );
            if shulker {
                let BlockEntity::Chest(slots) = entity else {
                    panic!("modeled shulker inventory missing");
                };
                assert_eq!(slots[0].as_ref().unwrap().count, 3);
                assert_eq!(slots[26].as_ref().unwrap().count, 5);
                assert_eq!(
                    game.world.block_names.get(&key),
                    Some(&Tag::String("Box".into()))
                );
            }
            let start = game.dropped.len();
            match route {
                "command" => {
                    assert!(game.run_command(&format!("setblock {x} 200 {z} air destroy")))
                }
                "player" => game.break_block(pos).unwrap(),
                "explosion" => game
                    .server_explode(
                        pos.map(|v| f64::from(v) + 0.5),
                        2.0,
                        BlockInteraction::Destroy,
                        false,
                    )
                    .unwrap(),
                _ => unreachable!(),
            }
            game.flush_spill();
            assert!(
                game.res.facts().is_air(game.world.block(x, 200, z)),
                "actual destruction occurred: {route}"
            );
            assert!(!game.world.block_entities.contains_key(&key));
            assert!(
                game.world.block_lock(key).is_none(),
                "actual external source was removed"
            );
            let item_id = format!("minecraft:{kind}");
            let drops: Vec<_> = game.dropped[start..]
                .iter()
                .filter(|d| *d.stack.item == item_id)
                .collect();
            assert_eq!(drops.len(), 1, "actual one-item drop: {kind}/{route}");
            let stack = &drops[0].stack;
            assert_eq!(stack.count, 1);
            let data = stack.data();
            let dropped_lock = data.and_then(|d| d.other("minecraft:lock"));
            if shulker {
                let data = data.expect("actual shulker components");
                assert_eq!(data.custom_name, Some(Tag::String("Box".into())));
                let slots = &data.container;
                assert_eq!(slots.len(), 2);
                let diamond = &slots.iter().find(|(slot, _)| *slot == 0).unwrap().1;
                assert_eq!((&*diamond.item, diamond.count), ("minecraft:diamond", 3));
                assert_eq!(
                    diamond
                        .data()
                        .and_then(|d| d.other("minecraft:custom_data"))
                        .and_then(|t| t.get("h211")),
                    Some(&Tag::Int(1))
                );
                assert_eq!(
                    slots.iter().find(|(slot, _)| *slot == 26).unwrap().1.count,
                    5
                );
            }
            if shulker && locked {
                if dropped_lock != actual_lock.as_ref() {
                    missing.push((route, dropped_lock.cloned(), actual_lock));
                }
            } else {
                assert!(
                    dropped_lock.is_none(),
                    "non-shulker/unlocked policy unchanged"
                );
            }
            assert_eq!(game.world.game_time, clock, "zero observation ticks");
        }
        assert!(
            missing.is_empty(),
            "actual external lock lost at destruction capture sites: {missing:?}"
        );
    }
}

#[cfg(test)]
mod explicit_source_tests {
    use super::*;
    use horizon_client::{mobs::Mob, motion::PlayerMotion};

    #[test]
    #[ignore = "requires prepared resources"]
    fn explicit_sources_keep_names_without_inventing_kill_credit() {
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
            );
            game.options.dedicated = None;
            game.creative = false;
            game.ui.difficulty = 2;
            game.build_registries();
            for mode in 0..5 {
                let nested = mode == 4;
                game.combat = Default::default();
                game.mobs.clear();
                game.orbs.clear();
                game.dropped.clear();
                game.player =
                    Some(PlayerMotion::new(game.motion, Vec3::new(10.5, 64.0, 8.5)).unwrap());
                let kind = game.res.mob_types().get("minecraft:husk").unwrap().clone();
                let species = kind.species;
                if mode == 2 || nested {
                    game.mobs
                        .push(Mob::new(43, kind, Vec3::new(40.0, 64.0, 40.0), 0.0));
                }
                // An enclosing blast must not leak its owner into an explicit source.
                game.combat.explosion_owner = Some(99);
                game.combat.exploded.push((99, Text::literal("Outer")));
                let cause = (mode != 0).then(|| ExplosionCause {
                    entity: (43, Text::literal("Igniter")),
                    player: false,
                    guest: None,
                    species: (mode != 1).then_some(species),
                });
                if nested {
                    game.options.dedicated = Some(Vec::new());
                    game.combat.explicit_explosion_cause = Some(cause);
                    game.combat.explicit_explosion_direct = Some(Text::literal("Outer"));
                    game.combat.blast_owner = Some((99, Text::literal("Outer")));
                    game.combat.blast_guest = Some(17);
                    let center = [8.5, 64.0, 8.5];
                    let level = game.level("minecraft:overworld").unwrap();
                    assert_eq!(
                        game.level_explode(
                            level,
                            None,
                            center,
                            2.0,
                            false,
                            mod_api::ExplosionInteraction::None,
                            mod_api::ExplosionSettings::default()
                        ),
                        Ok(())
                    );
                    assert!(matches!(
                        game.combat.explicit_explosion_cause,
                        Some(Some(_))
                    ));
                    assert!(game.combat.explicit_explosion_direct.is_some());
                    assert_eq!(
                        game.combat.blast_owner.as_ref().map(|owner| owner.0),
                        Some(99)
                    );
                    assert_eq!(game.combat.blast_guest, Some(17));
                } else {
                    game.server_explode_from_cause(
                        [8.5, 64.0, 8.5],
                        2.0,
                        BlockInteraction::Keep,
                        false,
                        ExplosionSource::default()
                            .with_entity(7, "minecraft:end_crystal")
                            .with_silence(true),
                        Some(cause),
                    )
                    .unwrap();
                }
                let events = std::mem::take(&mut game.player.as_mut().unwrap().events);
                game.record_combat(&events);
                assert_eq!(
                    game.combat
                        .last_blow()
                        .map(|(kind, id)| (kind.to_owned(), id)),
                    Some((
                        if mode == 0 || nested {
                            "minecraft:explosion"
                        } else {
                            "minecraft:player_explosion"
                        }
                        .into(),
                        if mode == 0 || nested { 0 } else { 43 }
                    ))
                );
                assert_eq!(
                    game.combat.kill_credit(game.world.game_time),
                    (mode == 2).then_some(43)
                );
                if mode == 1 || mode == 3 || nested {
                    let death = game
                        .combat_death_message_for(Text::literal("Victim"))
                        .component();
                    if nested {
                        assert_eq!(death["with"].as_array().unwrap().len(), 1);
                    } else {
                        assert_eq!(death["with"][1]["text"].as_str(), Some("Igniter"));
                    }
                }
                assert_eq!(game.combat.explosion_owner, Some(99));
                assert_eq!(game.combat.explicit_explosion_cause.is_some(), nested);
            }
        });
    }
}
