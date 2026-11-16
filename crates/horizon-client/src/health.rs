//! Stored player health and empty-equipment server damage for the resident scene.
//! World/audio/network consumers are queued explicitly; full death and tick
//! lifecycles (drops, respawn, effects) remain separate work.
use crate::{
    Result,
    motion::{MotionEvent, PlayerMotion},
};
use horizon_world::{
    attributes::live::LiveAttributes,
    block_landing::LandingDamage,
    combat_rules::ArmorWorld,
    damage_immunity::{self, DamageRule, ImmunityTag, ImmunityWorld},
    damage_reduction::Bypass,
    damage_source::{Catalog, DamageSource, DamageTypeId, Scaling},
    food::{Difficulty, FoodData},
    living_damage::DamageStat,
    living_hurt::{HurtState, HurtTag, HurtWorld, PendingHurt},
    player_health::{PlayerAttributeHealth, PlayerFloatIds},
    player_hurt::{CausingEntity, PlayerHurtWorld, ServerPlayerHurtWorld},
    server_player_combat::{CombatEnvironment, ServerPlayerCombat},
    server_player_damage::ServerDamageEnvironment,
    synced_floats::{FloatValue, Observer, SyncedFloats},
};
pub use mod_api::entity::health::{HurtSource, ShieldBlock, Victim};
use std::{fs, io, path::Path};
pub(crate) type Attributes = LiveAttributes<Box<dyn horizon_world::attributes::live::Identities>>;
pub(crate) type State = HurtState<DamageTypeId>;

pub use crate::world::HealthRules;
mod accepted_hit;
mod attack;
mod hit_attempt;
pub use accepted_hit::AcceptedHit;
pub use attack::{damage_type, melee, mob_source, source_damage_type};
/// What `LivingEntity.hurtServer` is about to hurt, for [`DamageHooks::incoming`], or
/// `LivingEntity.knockback` to push, for [`DamageHooks::knockback`]: the entity and the
/// hit's source.
pub enum Hurting<'a> {
    /// A player, by its entity id.
    Player(i32, HurtSource<'a>),
    /// A mob, and what hurts it.
    Mob(&'a crate::mobs::Mob, &'a crate::mobs::Damage),
    /// The ender dragon, by its entity id.
    Dragon(i32, HurtSource<'a>),
    /// An armor stand, by its entity id, for sweep knockback.
    ArmorStand(i32, HurtSource<'a>),
    /// A non-living entity, by its entity id and registry type.
    Entity(i32, &'a str, HurtSource<'a>),
}
pub use horizon_world::living_damage::{DamageContainer, Reduction};
/// The native hooks a hit runs, owned by the game and lent to the level the hit lands in
/// ([`DamageLevel`]), so no hook is a global. A table with no entry (`NONE`) runs none.
#[derive(Clone, Copy)]
pub struct DamageHooks {
    /// Override vanilla's per-source entity immunity decision.
    pub invulnerable: Option<fn(DamageLevel<'_>, Hurting<'_>, bool) -> bool>,
    /// `native_living_incoming_damage`: `LivingEntity.hurtServer` once its guards passed. It
    /// may change the container's new damage and post-attack invulnerability ticks; true
    /// cancels the hit.
    pub incoming: Option<fn(DamageLevel<'_>, Hurting<'_>, Victim, &mut DamageContainer) -> bool>,
    /// `native_living_damage_pre`: `LivingEntity.actuallyHurt` once armor and magic took their
    /// share of a hit, before the absorption hearts (`LivingDamageEvent.Pre`). It may change
    /// the new damage.
    pub pre: Option<fn(DamageLevel<'_>, Hurting<'_>, &mut DamageContainer)>,
    /// `native_living_damage_post`: the end of `hurtServer` for a hit that passed the incoming
    /// check and the cooldown, with the health damage it took, 0 included
    /// (`LivingDamageEvent.Post`).
    pub post: Option<fn(DamageLevel<'_>, Hurting<'_>, &DamageContainer)>,
    /// `native_living_knockback`: `LivingEntity.knockback`, at its start
    /// (`LivingKnockBackEvent`), on the raw strength and before the knockback resistance and
    /// the motion change. It may change the [`Knock`]; true cancels it.
    pub knockback: Option<fn(DamageLevel<'_>, Hurting<'_>, &mut Knock) -> bool>,
    /// `native_living_use_totem`: a `death_protection` item in the hand (true: the off hand)
    /// is about to save the entity from a killing blow (`LivingUseTotemEvent`). True cancels
    /// it: the entity dies.
    pub totem: Option<fn(DamageLevel<'_>, Hurting<'_>, bool) -> bool>,
    /// `native_living_shield_block`: `LivingEntity.applyItemBlocking` of a hit with damage,
    /// for what it hurts, the damage and whether vanilla blocks. It may change the
    /// [`ShieldBlock`].
    pub shield: Option<fn(DamageLevel<'_>, Hurting<'_>, f32, bool, &mut ShieldBlock)>,
    /// Retain an accepted source only when a death/loot continuation needs it.
    pub accepted: Option<fn(Hurting<'_>, &Attack) -> Option<AcceptedHit>>,
}
impl DamageHooks {
    /// No hooks: what tests of the vanilla behaviour hit through.
    pub const NONE: Self = Self {
        invulnerable: None,
        incoming: None,
        pre: None,
        post: None,
        knockback: None,
        totem: None,
        shield: None,
        accepted: None,
    };
    /// The hooks a client asks while it predicts a mob it rides: only the invulnerability
    /// check, which fires on both sides; the others are the server's.
    pub const fn client_side(self) -> Self {
        Self {
            invulnerable: self.invulnerable,
            ..Self::NONE
        }
    }
}
/// The level a hit lands in, as damage code sees it: its blocks, its random
/// (`Level.random`) and its own attachments, lent to the damage hooks as their level host.
/// Built from the level's world, or, in the mob tick, which reads the world while mobs
/// draw, from a [`LentLevel`] taken out of it for the tick.
pub struct DamageLevel<'a> {
    shape: crate::chunk::LevelShape,
    chunks: &'a crate::chunk_map::ChunkMap,
    air: horizon_world::registry::BlockStateId,
    registry: &'a horizon_world::registry::BlockRegistry,
    tags: Option<&'a crate::items::Items>,
    block_registries: Option<&'a crate::command::args::Registries>,
    random: &'a mut horizon_core::random::LevelRandom,
    attachments: &'a mut horizon_mod_runtime::attachments::Attachments,
    attachment_syncs: &'a mut crate::fast_hash::FxHashSet<horizon_mod_runtime::attachments::Holder>,
    /// The game's hooks; on a client predicting its own mount, only the both-side ones
    /// ([`DamageHooks::client_side`]).
    hooks: &'a DamageHooks,
    /// Who attacks in this hit and what they hold: the facts of the site that built the level.
    attack: &'a Attack,
    /// Whether this is a client's level, for a hook asked on both sides ([`Self::on_client`]).
    client_side: bool,
    /// Which entry of `LivingEntity.knockback` a push in progress came through, for
    /// `living_knockback` ([`Self::through_short_knockback`]).
    knock_entry: mod_api::KnockbackEntry,
}
impl<'a> DamageLevel<'a> {
    /// The level of `world`, whose blocks `registry` names.
    pub fn of(
        world: &'a mut crate::world::World,
        registry: &'a horizon_world::registry::BlockRegistry,
        tags: Option<&'a crate::items::Items>,
        block_registries: Option<&'a crate::command::args::Registries>,
        hooks: &'a DamageHooks,
    ) -> Self {
        let crate::world::World {
            chunks,
            shape,
            air,
            random,
            attachments,
            attachment_syncs,
            ..
        } = world;
        Self {
            shape: *shape,
            chunks,
            air: *air,
            registry,
            tags,
            block_registries,
            random,
            attachments,
            attachment_syncs,
            hooks,
            attack: &NO_ATTACK,
            client_side: false,
            knock_entry: mod_api::KnockbackEntry::Full,
        }
    }
    /// The level of `world`, with the random and attachments `lent` took out of it.
    pub fn lent(
        world: &'a crate::world::World,
        registry: &'a horizon_world::registry::BlockRegistry,
        lent: &'a mut LentLevel,
        tags: Option<&'a crate::items::Items>,
        block_registries: Option<&'a crate::command::args::Registries>,
        hooks: &'a DamageHooks,
    ) -> Self {
        Self {
            shape: world.shape,
            chunks: &world.chunks,
            air: world.air,
            registry,
            tags,
            block_registries,
            random: &mut lent.random,
            attachments: &mut lent.attachments,
            attachment_syncs: &mut lent.attachment_syncs,
            hooks,
            attack: &NO_ATTACK,
            client_side: false,
            knock_entry: mod_api::KnockbackEntry::Full,
        }
    }
    /// The same level, for a shorter borrow.
    pub fn reborrow(&mut self) -> DamageLevel<'_> {
        DamageLevel {
            shape: self.shape,
            chunks: self.chunks,
            air: self.air,
            registry: self.registry,
            tags: self.tags,
            block_registries: self.block_registries,
            random: self.random,
            attachments: self.attachments,
            attachment_syncs: self.attachment_syncs,
            hooks: self.hooks,
            attack: self.attack,
            client_side: self.client_side,
            knock_entry: self.knock_entry,
        }
    }
    /// The same level with a task-local random, retaining its typed hook and attachment context.
    pub fn reborrow_with_random<'b>(
        &'b mut self,
        random: &'b mut horizon_core::random::LevelRandom,
    ) -> DamageLevel<'b> {
        DamageLevel {
            shape: self.shape,
            chunks: self.chunks,
            air: self.air,
            registry: self.registry,
            tags: self.tags,
            block_registries: self.block_registries,
            random,
            attachments: self.attachments,
            attachment_syncs: self.attachment_syncs,
            hooks: self.hooks,
            attack: self.attack,
            client_side: self.client_side,
            knock_entry: self.knock_entry,
        }
    }
    /// Runs `push` as the short entry of `LivingEntity.knockback` (power and direction), which
    /// delegates to the full entry: the knockback hook, fired once inside the full entry, tells
    /// its listener the push came in as [`mod_api::KnockbackEntry::Short`]. A direct call of the
    /// full entry stays `Full`.
    pub fn through_short_knockback<R>(&mut self, push: impl FnOnce(&mut Self) -> R) -> R {
        let before = std::mem::replace(&mut self.knock_entry, mod_api::KnockbackEntry::Short);
        let out = push(self);
        self.knock_entry = before;
        out
    }
    /// Which entry of `LivingEntity.knockback` the push in progress came through.
    pub fn knockback_entry(&self) -> mod_api::KnockbackEntry {
        self.knock_entry
    }
    /// The same level, on a client when `client_side`: a hook asked on both sides tells its
    /// listener so.
    pub fn on_client(self, client_side: bool) -> Self {
        Self {
            client_side,
            ..self
        }
    }
    /// Whether this is a client's level ([`Self::on_client`]).
    pub fn client_side(&self) -> bool {
        self.client_side
    }
    /// The same level for the hit `attack` describes.
    pub fn attacked_by(self, attack: &'a Attack) -> Self {
        Self { attack, ..self }
    }
    /// Who attacks in this hit and what they hold, for the damage hooks.
    pub fn attack(&self) -> &'a Attack {
        self.attack
    }
    /// The authoritative world's merged tag tables, if available.
    pub fn tags(&self) -> Option<&crate::items::Items> {
        self.tags
    }
    /// The cached block parser context, if available.
    pub fn block_registries(&self) -> Option<&crate::command::args::Registries> {
        self.block_registries
    }
    /// The build range of the level whose damage hook is running.
    pub fn shape(&self) -> crate::chunk::LevelShape {
        self.shape
    }
    /// The level's random (`level.getRandom()`).
    pub fn random(&mut self) -> &mut horizon_core::random::LevelRandom {
        self.random
    }
    /// The block state at `pos`: air where no chunk is loaded, as `World::block`.
    pub fn block(&self, pos: [i32; 3]) -> horizon_world::registry::BlockStateId {
        self.chunks
            .get(&(pos[0] >> 4, pos[2] >> 4))
            .map_or(self.air, |chunk| {
                chunk.block(pos[0] & 15, pos[1], pos[2] & 15)
            })
    }
    /// The registry naming the level's block states.
    pub fn registry(&self) -> &'a horizon_world::registry::BlockRegistry {
        self.registry
    }
    /// The level's own attachments (`Holder::Level`), marked for the next sync. A chunk,
    /// block entity or entity's are not reachable from a hit: `MissingHolder`.
    pub fn attachments(
        &mut self,
        holder: horizon_mod_runtime::attachments::Holder,
    ) -> std::result::Result<
        &mut horizon_mod_runtime::attachments::Attachments,
        horizon_mod_runtime::attachments::AttachmentError,
    > {
        if holder != horizon_mod_runtime::attachments::Holder::Level {
            return Err(horizon_mod_runtime::attachments::AttachmentError::MissingHolder);
        }
        self.attachment_syncs.insert(holder);
        Ok(self.attachments)
    }
    /// Return the hook's immunity decision, or vanilla's without a listener.
    pub fn invulnerability_check(&mut self, hurting: Hurting<'_>, vanilla: bool) -> bool {
        match self.hooks.invulnerable {
            Some(hook) => hook(self.reborrow(), hurting, vanilla),
            None => vanilla,
        }
    }
    /// Whether `LivingIncomingDamageEvent` canceled the hit on `hurting`, standing as
    /// `victim` says ([`DamageHooks::incoming`]; never without it).
    pub fn incoming_canceled(
        &mut self,
        hurting: Hurting<'_>,
        victim: Victim,
        container: &mut DamageContainer,
    ) -> bool {
        let hook = self.hooks.incoming;
        hook.is_some_and(|hook| hook(self.reborrow(), hurting, victim, container))
    }
    /// `LivingDamageEvent.Pre` of the hit on `hurting` ([`DamageHooks::pre`]; nothing without it).
    pub fn damage_pre(&mut self, hurting: Hurting<'_>, container: &mut DamageContainer) {
        if let Some(hook) = self.hooks.pre {
            hook(self.reborrow(), hurting, container);
        }
    }
    /// `LivingDamageEvent.Post` of the hit on `hurting` ([`DamageHooks::post`]; nothing without it).
    pub fn damage_post(&mut self, hurting: Hurting<'_>, container: &DamageContainer) {
        if let Some(hook) = self.hooks.post {
            hook(self.reborrow(), hurting, container);
        }
    }
}
/// A level's random and attachments, taken out of its world while the world is read
/// elsewhere (the mob tick), and put back after.
pub struct LentLevel {
    random: horizon_core::random::LevelRandom,
    attachments: horizon_mod_runtime::attachments::Attachments,
    attachment_syncs: crate::fast_hash::FxHashSet<horizon_mod_runtime::attachments::Holder>,
}
impl LentLevel {
    /// Take `world`'s random and attachments: until [`Self::give_back`] the world has a
    /// placeholder random and none.
    pub fn take(world: &mut crate::world::World) -> Self {
        Self {
            random: std::mem::replace(
                &mut world.random,
                horizon_core::random::LevelRandom::for_level(
                    horizon_core::random::LegacyRandom::new(0),
                ),
            ),
            attachments: std::mem::take(&mut world.attachments),
            attachment_syncs: std::mem::take(&mut world.attachment_syncs),
        }
    }
    /// Put them back into `world`.
    pub fn give_back(self, world: &mut crate::world::World) {
        world.random = self.random;
        world.attachments = self.attachments;
        world.attachment_syncs = self.attachment_syncs;
    }
}
/// `Entity.isInvulnerableTo` asked outside a hit (a mod goal's or an entity handle's query):
/// the hook's answer over `vanilla`, or `vanilla` without a listener. The world is only read,
/// and the hook's level host reads no random or attachments, so the level lends empty ones.
pub fn query_invulnerability(
    world: &crate::world::World,
    registry: &horizon_world::registry::BlockRegistry,
    tags: Option<&crate::items::Items>,
    block_registries: Option<&crate::command::args::Registries>,
    hooks: &DamageHooks,
    hurting: Hurting<'_>,
    vanilla: bool,
) -> bool {
    let Some(hook) = hooks.invulnerable else {
        return vanilla;
    };
    let mut unread = LentLevel {
        random: horizon_core::random::LevelRandom::for_level(
            horizon_core::random::LegacyRandom::new(0),
        ),
        attachments: Default::default(),
        attachment_syncs: Default::default(),
    };
    hook(
        DamageLevel::lent(world, registry, &mut unread, tags, block_registries, hooks),
        hurting,
        vanilla,
    )
}
/// An empty level for tests and probes of hits: an air-only world whose random is seeded
/// with `seed`.
pub struct TestLevel {
    hooks: DamageHooks,
    world: crate::world::World,
    registry: horizon_world::registry::BlockRegistry,
}
impl TestLevel {
    pub fn new(seed: i64) -> Self {
        Self::with_hooks(seed, DamageHooks::NONE)
    }
    /// [`Self::new`] whose hits run `hooks`.
    pub fn with_hooks(seed: i64, hooks: DamageHooks) -> Self {
        let facts =
            br#"{"defaults":{"minecraft:air":0},"states":[["air",true,false,"","minecraft:air"]]}"#;
        Self {
            hooks,
            world: crate::world::World::new(horizon_world::registry::BlockStateId(0), seed),
            registry: horizon_world::registry::BlockRegistry::from_facts_json(&facts[..]).unwrap(),
        }
    }
    pub fn level(&mut self) -> DamageLevel<'_> {
        DamageLevel::of(&mut self.world, &self.registry, None, None, &self.hooks)
    }
    /// The level's random, for a probe that draws from it outside a hit.
    pub fn random(&mut self) -> &mut horizon_core::random::LevelRandom {
        &mut self.world.random
    }
}
/// A knockback's strength and direction, as `LivingEntity.knockback` has them
/// before knockback resistance is applied. The entity is pushed away from
/// the direction (`x`, `z`), which vanilla points at the source of the hit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Knock {
    pub strength: f64,
    pub x: f64,
    pub z: f64,
}
/// `CombatRules.getDamageAfterAbsorb`'s weapon: the stack
/// `DamageSource.getWeaponItem` names, the registry its `armor_effectiveness`
/// enchantments (Breach) are read from, and the entity that swung it.
pub struct ArmorWeapon {
    pub stack: crate::items::ItemStack,
    pub enchantments: std::sync::Arc<crate::enchant::Enchantments>,
    /// The causing and direct entity's type id: one entity swung.
    pub attacker: String,
    /// `DamageSource.isDirect`: the causing entity swung itself.
    pub is_direct: bool,
}
impl std::fmt::Debug for ArmorWeapon {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ArmorWeapon")
            .field("stack", &self.stack)
            .field("attacker", &self.attacker)
            .field("is_direct", &self.is_direct)
            .finish_non_exhaustive()
    }
}
impl DamageLevel<'_> {
    /// What `applyItemBlocking` stops of `damage` (above 0) once [`DamageHooks::shield`] had
    /// its say on `vanilla`, the block vanilla resolved: none when the hook unblocks it.
    pub fn shield_block(
        &mut self,
        hurting: Hurting<'_>,
        damage: f32,
        vanilla: ShieldBlock,
    ) -> Option<ShieldBlock> {
        let mut block = vanilla;
        if let Some(hook) = self.hooks.shield {
            hook(
                self.reborrow(),
                hurting,
                damage,
                vanilla.blocked,
                &mut block,
            );
        }
        if !block.blocked {
            return None;
        }
        // A value a mod leaves that is not a number stops nothing.
        block.blocked_damage = if block.blocked_damage.is_nan() {
            0.0
        } else {
            block.blocked_damage.clamp(0.0, damage)
        };
        Some(block)
    }
    /// Whether [`DamageHooks::totem`] canceled the totem of `hurting` (never without it).
    pub fn totem_canceled(&mut self, hurting: Hurting<'_>, off_hand: bool) -> bool {
        let Some(hook) = self.hooks.totem else {
            return false;
        };
        hook(self.reborrow(), hurting, off_hand)
    }
}
/// The block an entity with no raised item is asked about: none, and one a mod forces
/// stops the whole hit.
pub fn no_block(damage: f32) -> ShieldBlock {
    ShieldBlock {
        blocked: false,
        blocked_damage: damage,
        shield_damage: 0,
    }
}

/// What an attacker holds: the main hand and the off hand.
pub type Held = [Option<crate::items::ItemStack>; 2];

/// Who attacks in a hit and what they hold, as `living_knockback` and the weapon of
/// `living_incoming_damage` name them. The site that starts the attack builds it (the game's
/// `attack_of`) and lends it with the level the hit lands in ([`DamageLevel::attacked_by`]), so
/// a nested attack has its own and nothing is left behind when it returns.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Attack {
    /// The attacking entity's id, 0 for none.
    pub attacker: i32,
    /// The projectile delivering this hit, if any (`DamageSource.getDirectEntity`).
    pub projectile: Option<i32>,
    /// Whether that actual projectile extends `AbstractArrow` (including thrown tridents).
    pub projectile_is_arrow: bool,
    /// An explicitly supplied direct entity, including 0 for no direct entity.
    /// None uses the normal melee/projectile factory rules.
    pub direct: Option<i32>,
    /// `DamageSource.getSourcePosition`, as supplied by the hit's factory.
    pub position: Option<[f64; 3]>,
    /// An arrow's fired-from weapon or a thrown trident's complete stack.
    pub projectile_weapon: Option<crate::items::ItemStack>,
    /// The verified causing player's UUID, when the attacker is a player.
    pub player_uuid: Option<[i32; 4]>,
    /// What it holds, or none when it is not known (no native mod listens: the game only
    /// records it then).
    pub held: Held,
}
/// The attack of a hit nobody started: environment damage, falls, a level built outside a hit.
pub static NO_ATTACK: Attack = Attack {
    attacker: 0,
    projectile: None,
    projectile_is_arrow: false,
    direct: None,
    position: None,
    projectile_weapon: None,
    player_uuid: None,
    held: [None, None],
};
/// Whether vanilla's factory for `damage_type` passes the projectile as
/// `DamageSource.getDirectEntity` (`arrow`, `trident`, `thrown`, `spit`,
/// `fireball`, `unattributed_fireball`, `wither_skull`, `wind_charge`,
/// `fireworks`, `mob_projectile`, `indirect_magic`). `magic()` does not, and
/// neither does a blast: those stay entity-free even during a projectile's impact.
pub fn names_its_projectile(damage_type: &str) -> bool {
    matches!(
        damage_type,
        "minecraft:arrow"
            | "minecraft:trident"
            | "minecraft:fireball"
            | "minecraft:unattributed_fireball"
            | "minecraft:wither_skull"
            | "minecraft:mob_projectile"
            | "minecraft:thrown"
            | "minecraft:wind_charge"
            | "minecraft:fireworks"
            | "minecraft:spit"
            | "minecraft:indirect_magic"
    )
}
impl DamageLevel<'_> {
    /// Whether [`DamageHooks::knockback`] canceled the knockback (never without it). A
    /// value it leaves that is not a finite number is put back as it was.
    pub fn knockback_canceled(&mut self, pushed: Hurting<'_>, knock: &mut Knock) -> bool {
        let Some(hook) = self.hooks.knockback else {
            return false;
        };
        let before = *knock;
        let canceled = hook(self.reborrow(), pushed, knock);
        for (now, was) in [
            (&mut knock.strength, before.strength),
            (&mut knock.x, before.x),
            (&mut knock.z, before.z),
        ] {
            if !now.is_finite() {
                *now = was;
            }
        }
        canceled
    }
}
#[derive(Clone)]
pub struct HealthResources {
    pub damage: Catalog,
    pub ids: PlayerFloatIds,
}
impl HealthResources {
    pub fn load(root: &Path) -> Result<Self> {
        Self::load_with_conditions(root, &horizon_world::conditions::LoadConditions::vanilla())
    }
    /// Startup damage resources use the same engine-owned server inventory
    /// as the other tables, before any world pack stack exists.
    pub fn load_with_conditions(
        root: &Path,
        conditions: &horizon_world::conditions::LoadConditions,
    ) -> Result<Self> {
        let read = |name: &str| -> Result<serde_json::Value> {
            Ok(serde_json::from_slice(&fs::read(root.join(name))?)?)
        };
        let ids = read("player-float-ids.json")?;
        let id = |name: &str| -> Result<u8> {
            Ok(u8::try_from(
                ids[name].as_u64().ok_or("missing player float ID")?,
            )?)
        };
        let damage = Catalog::load(
            root,
            &horizon_world::loot::PackOverlay {
                conditions,
                ..horizon_world::loot::PackOverlay::NONE
            },
        )?;
        for name in ["minecraft:fall", "minecraft:stalagmite", "minecraft:starve"] {
            if damage.id(name).is_none() {
                return Err(format!("missing damage type {name}").into());
            }
        }
        Ok(Self {
            damage,
            ids: PlayerFloatIds {
                health: id("health")?,
                absorption: id("absorption")?,
            },
        })
    }
    /// A world's frozen damage registry at open, or reloaded tags against
    /// that same registry. Player metadata IDs stay the prepared version's.
    pub fn with_packs(
        &self,
        root: &Path,
        overlay: &horizon_world::loot::PackOverlay<'_>,
        reload: bool,
    ) -> Result<Self> {
        Ok(Self {
            damage: if reload {
                self.damage.reload_tags(root, overlay)?
            } else {
                Catalog::load(root, overlay)?
            },
            ids: self.ids,
        })
    }
    pub fn landing_source(&self, source: LandingDamage) -> DamageTypeId {
        self.damage
            .id(match source {
                LandingDamage::Fall => "minecraft:fall",
                LandingDamage::Stalagmite => "minecraft:stalagmite",
            })
            .expect("validated landing source")
    }
}
/// `DamageEffects.sound()` by its serialized id: what `Player.getHurtSound`
/// plays for a damage type's `effects` (unknown ids fall back to `hurt`, the codec default).
pub fn hurt_sound(effects: &str) -> &'static str {
    match effects {
        "drowning" => "minecraft:entity.player.hurt_drown",
        "burning" => "minecraft:entity.player.hurt_on_fire",
        "poking" => "minecraft:entity.player.hurt_sweet_berry_bush",
        "freezing" => "minecraft:entity.player.hurt_freeze",
        // HURT and THORNS (both SoundEvents.PLAYER_HURT).
        _ => "minecraft:entity.player.hurt",
    }
}
#[derive(Clone, Copy, Debug)]
pub enum HealthEvent {
    /// `FoodData.tick` calling `hurtServer` with starvation. Only a hit that
    /// lands is recorded (`RecordedDamage`).
    StarvationDamage(f32),
    /// `LivingEntity.aiStep` hurting a fully frozen entity.
    FreezeDamage(f32),
    /// `SweetBerryBushBlock.entityInside` pricking a moving entity.
    SweetBerryBushDamage(f32),
    /// `LivingEntity.baseTick` drowning an entity out of air.
    DrownDamage(f32),
    Updated {
        id: u8,
        value: f32,
        dirty: bool,
    },
    Statistic {
        name: &'static str,
        amount: i32,
    },
    RecordedDamage {
        source: DamageTypeId,
        amount: f32,
    },
    /// `LivingEntity.hurtServer`'s `for (MobEffectInstance effect :
    /// getActiveEffects()) effect.onMobHurt(level, this, source, damage)`: an
    /// accepted hit's effects answer it, with the damage before armor and
    /// absorption, so a hit they take whole still reaches them. The game runs
    /// them (`InfestedMobEffect`).
    MobHurt,
    Phase(&'static str),
    /// `Player.hurtArmor`: a hit that doesn't bypass armor wears the worn
    /// pieces (`LivingEntity.getDamageAfterArmorAbsorb`); the game applies it.
    ArmorHurt {
        source: DamageTypeId,
        causing: i32,
        direct: i32,
        position: Option<[f64; 3]>,
        amount: f32,
    },
    /// `ServerLevel.broadcastDamageEvent`: the client's
    /// `LivingEntity.handleDamageEvent` plays the source's hurt sound.
    DamageBroadcast {
        source: DamageTypeId,
    },
    Criterion {
        /// The hit's damage type, for the responsible player of an accepted hit.
        source: DamageTypeId,
        original: f32,
        amount: f32,
        blocked: bool,
    },
}
impl HealthEvent {
    pub fn trace(self) -> String {
        match self {
            Self::StarvationDamage(amount) => format!("damage starve {:08x}", amount.to_bits()),
            Self::FreezeDamage(amount) => format!("damage freeze {:08x}", amount.to_bits()),
            Self::SweetBerryBushDamage(amount) => {
                format!("damage sweet_berry_bush {:08x}", amount.to_bits())
            }
            Self::DrownDamage(amount) => format!("damage drown {:08x}", amount.to_bits()),
            Self::Updated { id, value, dirty } => {
                format!("updated {id} {:08x} {dirty}", value.to_bits())
            }
            Self::Statistic { name, amount } => format!("health-stat {name} {amount}"),
            Self::RecordedDamage { amount, .. } => {
                format!("record-damage {:08x}", amount.to_bits())
            }
            Self::MobHurt => "mob-hurt".into(),
            Self::Phase(name) => name.into(),
            Self::ArmorHurt { amount, .. } => format!("hurt-armor {:08x}", amount.to_bits()),
            Self::DamageBroadcast { .. } => "broadcast-damage".into(),
            Self::Criterion { .. } => "hurt-criterion".into(),
        }
    }
}
pub(crate) struct Updates<'a>(pub(crate) &'a mut Vec<MotionEvent>);
impl Observer for Updates<'_> {
    type Error = io::Error;
    fn updated(&mut self, data: &mut SyncedFloats, id: u8) -> io::Result<()> {
        self.0.push(MotionEvent::Health(HealthEvent::Updated {
            id,
            value: data.get(id).expect("existing updated field"),
            dirty: data.is_dirty(),
        }));
        Ok(())
    }
}
#[derive(Clone)]
pub struct PlayerHealth {
    pub data: SyncedFloats,
    pub ids: PlayerFloatIds,
    pub food: FoodData,
    pub hurt: State,
    pub accepted_hit: Option<std::sync::Arc<AcceptedHit>>,
    /// `Entity.permanentlyInvulnerable` (arismoko/horizon#916): the NBT `Invulnerable` flag,
    /// what `Entity.setInvulnerable` writes — a field apart from
    /// `Abilities.invulnerable` (the game mode the client is told about),
    /// which vanilla keeps separately too. Not saved: Horizon's player save
    /// keeps no such field yet.
    pub permanently_invulnerable: bool,
    /// `Entity.invulnerableTime` (arismoko/horizon#916): the entity-level invulnerability
    /// window, its own field — never the damage cooldown (`hurt.cooldown`).
    pub invulnerable_time: i32,
    /// Client hurt animation; fully blocked hits only refresh the server's hurt state.
    pub hurt_animation_time: i32,
    /// `Player.hurtDir` on a client: the yaw `ClientboundHurtAnimationPacket`
    /// gave, which `GameRenderer.bobHurt` tilts the camera around.
    pub hurt_dir: f32,
    /// `ServerPlayer.hurtDir` of a hit the player's client hasn't been told
    /// of yet (`indicateDamage`).
    pub indicated_dir: Option<f32>,
    pub dead: bool,
    pub sync_velocity: bool,
    pub game_time: i64,
    pub tick_count: i32,
    /// A death-protection item (totem) is in either hand.
    pub totem_ready: bool,
    /// The totem saved the player this tick; the game applies its effects.
    pub totem_used: bool,
    /// The totem that would save the player is in the off hand.
    pub totem_off_hand: bool,
}
impl PlayerHealth {
    pub fn new(resources: &HealthResources, attributes: &mut Attributes) -> Result<Self> {
        let ids = resources.ids;
        let mut data = SyncedFloats::new(&[
            FloatValue {
                id: ids.health,
                value: 1.0,
            },
            FloatValue {
                id: ids.absorption,
                value: 0.0,
            },
        ])?;
        let mut ignored = Vec::new();
        let mut observer = Updates(&mut ignored);
        let maximum = attributes
            .key("minecraft:max_health")
            .ok_or("missing max health")?;
        let maximum = attributes
            .value(
                Some(maximum),
                horizon_world::attribute_map::ValueKind::Computed,
            )
            .map_err(|e| io::Error::other(format!("health attribute: {e:?}")))?
            as f32;
        PlayerAttributeHealth::new(attributes, &mut data, ids, false, &mut observer)?
            .set_health(maximum)?;
        Ok(Self {
            data,
            ids,
            food: FoodData::default(),
            hurt: HurtState {
                no_action_time: 0,
                cooldown: 0,
                last_hurt: 0.0,
                hurt_duration: 0,
                hurt_time: 0,
                last_source: None,
                last_stamp: 0,
            },
            accepted_hit: None,
            permanently_invulnerable: false,
            invulnerable_time: 0,
            hurt_animation_time: 0,
            hurt_dir: 0.0,
            indicated_dir: None,
            dead: false,
            sync_velocity: false,
            game_time: 0,
            tick_count: 0,
            totem_ready: false,
            totem_used: false,
            totem_off_hand: false,
        })
    }
    pub fn health(&self) -> f32 {
        self.data
            .get(self.ids.health)
            .expect("validated health field")
    }
    pub fn absorption(&self) -> f32 {
        self.data
            .get(self.ids.absorption)
            .expect("validated absorption field")
    }
    /// The game time the damage clock has counted.
    pub fn game_time(&self) -> i64 {
        self.game_time
    }
    pub fn dead_or_dying(&self) -> bool {
        self.health() <= 0.0 || self.dead
    }
    /// The damage-clock phases of ServerPlayer.tick and LivingEntity.baseTick.
    /// The complete enclosing ticks, including death removal, are not yet bound.
    pub fn advance_clock(&mut self) {
        self.game_time = self.game_time.wrapping_add(1);
        self.tick_count = self.tick_count.wrapping_add(1);
        if self.hurt.cooldown > 0 {
            self.hurt.cooldown -= 1;
        }
        // Entity.baseTick: the entity-level window counts down too, its own
        // timer (arismoko/horizon#916), never the damage cooldown above.
        if self.invulnerable_time > 0 {
            self.invulnerable_time -= 1;
        }
        if self.hurt.hurt_time > 0 {
            self.hurt.hurt_time -= 1;
        }
        if self.hurt_animation_time > 0 {
            self.hurt_animation_time -= 1;
        }
    }
    /// `LivingEntity.animateHurt` / `handleDamageEvent` on a client: the hurt
    /// tilt and red flash restart.
    pub fn animate_hurt(&mut self) {
        self.hurt.hurt_duration = 10;
        self.hurt.hurt_time = 10;
        self.hurt_animation_time = 10;
    }
    /// `LivingEntity.animateHurt(yaw)` on a player: the tilt restarts around
    /// `yaw` (`Player.animateHurt` keeps it as `hurtDir`).
    pub fn animate_hurt_dir(&mut self, yaw: f32) {
        self.animate_hurt();
        self.hurt_dir = yaw;
    }
    /// `ServerPlayer.indicateDamage`: the hit's direction relative to where
    /// the player looks, kept for the packet to its client.
    pub fn indicate_damage(&mut self, xd: f64, zd: f64, yaw: f32) {
        use horizon_core::geometry::trig::{RAD_TO_DEG, atan2};
        self.indicated_dir = Some((atan2(zd, xd) * f64::from(RAD_TO_DEG) - f64::from(yaw)) as f32);
    }
    /// `LocalPlayer.hurtTo` with `flashOnSetHealth`: a drop below the current
    /// health starts the hurt animation; `damageCooldownTime` (20 on a drop,
    /// 10 on a gain) is what lets `Hud.extractPlayerHealth` blink the hearts.
    pub fn flash_health_change(&mut self, health: f32) {
        let damage = self.health() - health;
        if damage <= 0.0 {
            if damage < 0.0 {
                self.hurt.cooldown = 10;
            }
        } else {
            self.hurt.last_hurt = damage;
            self.hurt.cooldown = 20;
            self.animate_hurt();
        }
    }
}
impl PlayerMotion {
    /// Where the player stands and looks (`getViewVector`), as a hit's [`Victim`].
    pub(crate) fn victim(&self) -> Victim {
        let view = horizon_world::view_direction::view_vector(self.pitch, self.yaw);
        Victim {
            position: [self.position.x, self.position.y, self.position.z],
            view: [view.x, view.y, view.z],
        }
    }
    /// `hurtServer` of `amount` from `source` in `level`.
    pub fn hurt(
        &mut self,
        level: &mut DamageLevel<'_>,
        resources: &HealthResources,
        rules: HealthRules,
        source: DamageTypeId,
        amount: f32,
    ) -> Result<bool> {
        Ok(self
            .hurt_blocking(level, resources, rules, source, amount, None)?
            .hurt)
    }
    pub fn hurt_blocking(
        &mut self,
        level: &mut DamageLevel<'_>,
        resources: &HealthResources,
        rules: HealthRules,
        source: DamageTypeId,
        amount: f32,
        blocking: Option<Blocking>,
    ) -> Result<BlockOutcome> {
        self.hurt_caused_by(
            level, resources, rules, source, amount, blocking, false, 0, false,
        )
    }
    /// [`hurt_blocking`](Self::hurt_blocking) from a source whose hit was a
    /// creative player's (`DamageSource.isCreativePlayer`): it passes the
    /// invulnerability the victim has (`Entity.isInvulnerableToBase`'s one
    /// exception, arismoko/horizon#916).
    pub fn hurt_blocking_creative(
        &mut self,
        level: &mut DamageLevel<'_>,
        resources: &HealthResources,
        rules: HealthRules,
        source: DamageTypeId,
        amount: f32,
        blocking: Option<Blocking>,
    ) -> Result<BlockOutcome> {
        self.hurt_caused_by(
            level, resources, rules, source, amount, blocking, false, 0, true,
        )
    }
    /// `hurt_blocking` from a source whose causing entity is a living
    /// non-player (`living_cause`), which the `when_caused_by_living_non_player`
    /// damage types scale with the difficulty. `causing` is that entity's id
    /// when it is known (0 otherwise), for the native damage hook.
    pub fn hurt_caused_by(
        &mut self,
        level: &mut DamageLevel<'_>,
        resources: &HealthResources,
        rules: HealthRules,
        source: DamageTypeId,
        amount: f32,
        blocking: Option<Blocking>,
        living_cause: bool,
        causing: i32,
        creative_source: bool,
    ) -> Result<BlockOutcome> {
        let hit = self.start_hurt(
            level,
            resources,
            rules,
            source,
            amount,
            blocking,
            living_cause,
            causing,
            creative_source,
            false,
            true,
            None,
        )?;
        self.finish_hurt(level, resources, rules, hit)
    }
    /// `hurtServer` up to `hurtHelmet`, for a player who wears a `helmet`; the arguments are
    /// `hurt_caused_by`'s. The caller has the player and the level back to wear the helmet
    /// with ([`PausedHurt::helmet_damage`]), and ends the hit with [`Self::finish_hurt`].
    /// `can_harm_player` is the server PvP decision, applied after the invulnerability
    /// observation. Non-PvP damage passes true. The base observation is retained across
    /// all continuations belonging to this hit.
    pub fn start_hurt(
        &mut self,
        level: &mut DamageLevel<'_>,
        resources: &HealthResources,
        rules: HealthRules,
        source: DamageTypeId,
        amount: f32,
        blocking: Option<Blocking>,
        living_cause: bool,
        causing: i32,
        creative_source: bool,
        helmet: bool,
        can_harm_player: bool,
        armor_weapon: Option<ArmorWeapon>,
    ) -> Result<PausedHurt> {
        let mut hit = PausedHurt {
            pending: None,
            invulnerability_checked: None,
            can_harm_player,
            selected: None,
            inner: None,
            source,
            source_name: String::new(),
            amount,
            before: self.health.health() + self.health.absorption(),
            fire_ticks: self.fire_ticks,
            helmet,
            living_cause,
            causing,
            armor_weapon,
            // The invulnerability refusal's inputs (arismoko/horizon#916), kept as the hit
            // was, for the Environment the pipeline's own rule reads.
            invulnerable: self.invulnerable,
            permanently_invulnerable: self.health.permanently_invulnerable,
            invulnerable_time: self.health.invulnerable_time > 0,
            creative_source,
            blocking,
            blocked: 0.0,
            shield_damage: 0,
            blocking_item_used: false,
            fully_blocked: false,
            block_sound: false,
            knocked: false,
            totem_fired: false,
            canceled: false,
            dealt: 0.0,
        };
        if self.client_side {
            return Ok(hit);
        }
        hit.source_name = resources
            .damage
            .definition(source)
            .ok_or("unknown resident damage source")?
            .name
            .clone();
        hit.pending = self.in_combat(level, resources, rules, &mut hit, |combat, state| {
            Ok(combat.hurt_until_helmet(state, amount)?)
        })?;
        Ok(hit)
    }
    /// Select the inner hit before synchronous ordinary equipment wear.
    pub fn select_equipment_hurt(
        &mut self,
        level: &mut DamageLevel<'_>,
        resources: &HealthResources,
        rules: HealthRules,
        hit: &mut PausedHurt,
    ) -> Result<Option<f32>> {
        let Some(pending) = hit.pending.take() else {
            return Ok(None);
        };
        hit.selected = self.in_combat(level, resources, rules, hit, |combat, state| {
            Ok(horizon_world::living_hurt::select_equipment_hurt(
                state, combat, pending,
            )?)
        })?;
        if let Some(selected) = &hit.selected {
            let damage = selected.damage;
            hit.inner = self.in_combat(level, resources, rules, hit, |combat, _| {
                Ok(combat.prepare_equipment_inner()?)
            })?;
            if hit.inner == Some(false) {
                return Ok(Some(damage));
            }
        }
        Ok(None)
    }
    /// The rest of a hit from `hurtHelmet` on, against the player as the helmet's wear left
    /// them: its armor and protection, and whatever a listener changed.
    pub fn finish_hurt(
        &mut self,
        level: &mut DamageLevel<'_>,
        resources: &HealthResources,
        rules: HealthRules,
        mut hit: PausedHurt,
    ) -> Result<BlockOutcome> {
        if self.client_side {
            return Ok(BlockOutcome::default());
        }
        let hurt = if let Some(selected) = hit.selected.take() {
            let inner = hit.inner;
            self.in_combat(level, resources, rules, &mut hit, |combat, state| {
                Ok(combat.finish_selected_equipment(state, selected, inner)?)
            })?
        } else {
            match hit.pending.take() {
                Some(pending) => {
                    self.in_combat(level, resources, rules, &mut hit, |combat, state| {
                        Ok(combat.hurt_after_helmet(state, pending)?)
                    })?
                }
                None => false,
            }
        };
        let h = &mut self.health;
        let outcome = BlockOutcome {
            hurt,
            blocked: hit.blocked,
            shield_damage: hit.shield_damage,
            blocking_item_used: hit.blocking_item_used,
            fully_blocked: hit.fully_blocked,
            sound: hit.block_sound,
            knockback: hit.knocked,
            canceled: hit.canceled,
            dealt: hit.dealt,
        };
        if hit.totem_fired {
            h.totem_used = true;
            h.totem_ready = false;
        }
        if outcome.hurt {
            h.accepted_hit = if h.health() <= 0.0 {
                let source = level.attack().player_source(&hit.source_name, hit.causing);
                level.accepted_hit(Hurting::Player(self.entity_id, source))
            } else {
                None
            };
        }
        if outcome.hurt {
            h.hurt_animation_time = h.hurt.hurt_time;
        }
        // `dealDefaultKnockback` ends in `indicateDamage` unless blocked: a
        // hit with no direction of its own (`xd` and `zd` zero) is sent as
        // the opposite of the player's yaw. A hit that has a direction
        // replaces this (`mob_hits_player`).
        if outcome.knockback && outcome.blocked <= 0.0 {
            h.indicate_damage(0.0, 0.0, self.yaw);
        }
        let after = h.health() + h.absorption();
        // `LivingEntity.getDamageAfterArmorAbsorb` is the hit's only reader of
        // these two instances, and a source in `#minecraft:bypasses_armor`
        // makes it return before `getArmorValue` and
        // `getAttributeValue(ARMOR_TOUGHNESS)`. Native `getValue` is what
        // clears an instance's dirty flag, so the record takes the caches the
        // hit left instead of recomputing them: one that does meet the armour
        // has already recomputed both in `after_armor`, and one that bypassed
        // it leaves creation's zero cache untouched.
        let armor = self
            .attributes
            .cached_value_of("minecraft:armor")
            .unwrap_or(0.0) as f32;
        let toughness = self
            .attributes
            .cached_value_of("minecraft:armor_toughness")
            .unwrap_or(0.0) as f32;
        if self.hurt_log.len() >= HURT_LOG {
            self.hurt_log.remove(0);
        }
        self.hurt_log.push(HurtRecord {
            game_time: h.game_time,
            source: hit.source_name,
            amount: hit.amount,
            blocked: outcome.blocked,
            taken: hit.before - after,
            hurt: outcome.hurt,
            armor,
            toughness,
            health: after,
            fire_ticks: hit.fire_ticks,
            knockback: None,
        });
        Ok(outcome)
    }
    /// `run` on the player's combat, the hit's own state lent to it and kept for its next half.
    fn in_combat<R>(
        &mut self,
        level: &mut DamageLevel<'_>,
        resources: &HealthResources,
        rules: HealthRules,
        hit: &mut PausedHurt,
        run: impl FnOnce(
            &mut ServerPlayerCombat<'_, Environment<'_>, Attributes>,
            &mut State,
        ) -> Result<R>,
    ) -> Result<R> {
        let source = resources
            .damage
            .source::<()>(hit.source, None, None, None)
            .ok_or("unknown resident damage source")?;
        let victim = self.victim();
        let h = &mut self.health;
        let mut environment = Environment {
            level: level.reborrow(),
            invulnerability_checked: hit.invulnerability_checked,
            can_harm_player: hit.can_harm_player,
            entity: self.entity_id,
            victim,
            causing: hit.causing,
            invulnerable: hit.invulnerable,
            permanently_invulnerable: hit.permanently_invulnerable,
            invulnerable_time: hit.invulnerable_time,
            creative_source: hit.creative_source,
            client_loaded: self.client_loaded,
            rules,
            source,
            weapon: hit.armor_weapon.as_ref(),
            random: &mut self.random,
            living_cause: hit.living_cause,
            events: &mut self.events,
            dead: &mut h.dead,
            sync_velocity: &mut h.sync_velocity,
            game_time: h.game_time,
            status: self.status.clone(),
            helmet: hit.helmet,
            blocking: hit.blocking.take(),
            blocked: hit.blocked,
            shield_damage: hit.shield_damage,
            blocking_item_used: hit.blocking_item_used,
            fully_blocked: hit.fully_blocked,
            block_sound: hit.block_sound,
            knocked: hit.knocked,
            totem: h.totem_ready,
            totem_off_hand: h.totem_off_hand,
            totem_fired: hit.totem_fired,
            canceled: hit.canceled,
            dealt: hit.dealt,
        };
        let result = run(
            &mut ServerPlayerCombat::new(
                &mut self.attributes,
                &mut h.data,
                &mut h.food,
                h.ids,
                &mut environment,
            )?,
            &mut h.hurt,
        )?;
        hit.invulnerability_checked = environment.invulnerability_checked;
        hit.blocking = environment.blocking;
        hit.blocked = environment.blocked;
        hit.shield_damage = environment.shield_damage;
        hit.blocking_item_used = environment.blocking_item_used;
        hit.fully_blocked = environment.fully_blocked;
        hit.block_sound = environment.block_sound;
        hit.knocked = environment.knocked;
        hit.totem_fired = environment.totem_fired;
        hit.canceled = environment.canceled;
        hit.dealt = environment.dealt;
        Ok(result)
    }
    pub fn set_health(&mut self, value: f32, absorption: bool) -> Result<()> {
        let h = &mut self.health;
        let mut observer = Updates(&mut self.events);
        let mut binding = PlayerAttributeHealth::new(
            &mut self.attributes,
            &mut h.data,
            h.ids,
            h.dead,
            &mut observer,
        )?;
        if absorption {
            binding.set_absorption(value)?;
        } else {
            binding.set_health(value)?;
        }
        Ok(())
    }
}
pub(crate) struct Environment<'a> {
    pub(crate) invulnerability_checked: Option<bool>,
    pub(crate) can_harm_player: bool,
    /// The level the hit lands in, for the damage hooks.
    pub(crate) level: DamageLevel<'a>,
    /// The player's entity id, and the causing entity's (0: none or not known).
    pub(crate) entity: i32,
    /// Where the player stands and looks, for the incoming-damage hook.
    pub(crate) victim: Victim,
    pub(crate) causing: i32,
    /// `Abilities.invulnerable`, the game mode (`Player.hurtServer`'s own
    /// gate answers it, which no creative hit passes).
    pub(crate) invulnerable: bool,
    /// `Entity.permanentlyInvulnerable` (arismoko/horizon#916): the NBT flag.
    pub(crate) permanently_invulnerable: bool,
    /// `Entity.invulnerableTime > 0` (arismoko/horizon#916): the entity-level window.
    pub(crate) invulnerable_time: bool,
    /// The hit is a creative player's (`DamageSource.isCreativePlayer`,
    /// arismoko/horizon#916): it passes the invulnerability below.
    pub(crate) creative_source: bool,
    /// The player's client has loaded the world since its last respawn or
    /// level change (`ServerGamePacketListenerImpl.hasClientLoaded`).
    pub(crate) client_loaded: bool,
    pub(crate) rules: HealthRules,
    pub(crate) source: DamageSource<'a, ()>,
    /// `DamageSource.getWeaponItem` with the registry of its
    /// `armor_effectiveness` enchantments, for `getDamageAfterArmorAbsorb`.
    pub(crate) weapon: Option<&'a ArmorWeapon>,
    /// The victim's own random. `Enchantment.modifyDamageFilteredValue` gives
    /// its value effects `victim.getRandom()`; `Enchantments::modify_value`
    /// takes one random for those and for the requirements, which vanilla
    /// draws from the level's.
    pub(crate) random: &'a mut horizon_core::random::EntityRandom,
    /// The causing entity is a living non-player (a mob, or its projectile's owner).
    pub(crate) living_cause: bool,
    pub(crate) events: &'a mut Vec<MotionEvent>,
    pub(crate) dead: &'a mut bool,
    pub(crate) sync_velocity: &'a mut bool,
    pub(crate) game_time: i64,
    /// Resistance, fire resistance and armor protection (effects.rs).
    pub(crate) status: crate::effects::StatusFlags,
    /// The head slot holds something, for a hit that stops at `hurtHelmet`.
    pub(crate) helmet: bool,
    /// The raised blocking item, if any, and what it blocked.
    pub(crate) blocking: Option<Blocking>,
    pub(crate) blocked: f32,
    /// The durability the blocking item loses.
    pub(crate) shield_damage: i32,
    pub(crate) blocking_item_used: bool,
    /// What it blocked covers the whole hit, as difficulty scaled it.
    pub(crate) fully_blocked: bool,
    pub(crate) block_sound: bool,
    /// `dealDefaultKnockback` ran: a fresh hit, not fully blocked.
    pub(crate) knocked: bool,
    /// LivingEntity.checkTotemDeathProtection: a totem is held / was used.
    pub(crate) totem: bool,
    /// The totem is in the off hand.
    pub(crate) totem_off_hand: bool,
    pub(crate) totem_fired: bool,
    /// A native mod canceled the hit (`living_incoming_damage`).
    pub(crate) canceled: bool,
    /// The damage `ENTITY_HURT_PLAYER` is told of.
    pub(crate) dealt: f32,
}
/// A hit on a player stopped at `hurtHelmet` ([`PlayerMotion::start_hurt`]): the hit's own
/// state, which borrows nothing, so the game can run the helmet's wear on its real host.
/// Everything the player and level hold is read again when it resumes.
#[derive(Debug)]
pub struct PausedHurt {
    invulnerability_checked: Option<bool>,
    can_harm_player: bool,
    /// None for a hit refused before `hurtHelmet`.
    selected: Option<horizon_world::living_hurt::SelectedHurt<()>>,
    inner: Option<bool>,
    pending: Option<PendingHurt<()>>,
    source: DamageTypeId,
    source_name: String,
    amount: f32,
    /// Health and absorption, and the fire ticks, before the hit.
    before: f32,
    fire_ticks: i32,
    helmet: bool,
    living_cause: bool,
    causing: i32,
    /// The attacker's weapon, for the armour fraction's `armor_effectiveness`.
    armor_weapon: Option<ArmorWeapon>,
    /// `Abilities.invulnerable` at the hit's start (arismoko/horizon#916).
    invulnerable: bool,
    /// `Entity.permanentlyInvulnerable` at the hit's start (arismoko/horizon#916).
    permanently_invulnerable: bool,
    /// `Entity.invulnerableTime > 0` at the hit's start (arismoko/horizon#916).
    invulnerable_time: bool,
    /// The hit is a creative player's (`isCreativePlayer`, arismoko/horizon#916).
    creative_source: bool,
    blocking: Option<Blocking>,
    blocked: f32,
    shield_damage: i32,
    blocking_item_used: bool,
    fully_blocked: bool,
    block_sound: bool,
    knocked: bool,
    totem_fired: bool,
    canceled: bool,
    dealt: f32,
}
impl PausedHurt {
    /// What `hurtHelmet` is given, if this hit wears the helmet.
    pub fn helmet_damage(&self) -> Option<f32> {
        self.pending.as_ref()?.helmet_damage()
    }
}
/// `LivingEntity.applyItemBlocking` inputs for a raised `BlocksAttacks` item:
/// reductions whose damage-type sets are already matched against the source.
#[derive(Clone, Debug)]
pub struct Blocking {
    pub rules: horizon_world::blocking_rules::BlockingRules<bool>,
    /// `bypassed_by` contains the source's damage type.
    pub bypassed: bool,
    /// The direct entity is an arrow with piercing.
    pub pierce: bool,
    /// Angle between the head's view and the source position (pi without one).
    pub angle: f64,
}
impl Blocking {
    /// `BlocksAttacks.resolveBlockedDamage`: what the item stops of `damage`,
    /// nothing for a source it bypasses or an arrow that pierces it.
    pub fn blocked(&self, damage: f32) -> f32 {
        if damage <= 0.0 || self.bypassed || self.pierce {
            return 0.0;
        }
        self.rules
            .resolve(&mut MatchedSets, damage, self.angle)
            .unwrap_or(0.0)
    }
}
/// How many hurts `PlayerMotion::hurt_log` keeps.
const HURT_LOG: usize = 64;
/// One hurt as it was taken: what came in, what a raised blocking item
/// stopped, the armour it met and what health it left.
#[derive(Clone, Debug, PartialEq)]
pub struct HurtRecord {
    pub game_time: i64,
    /// The damage type.
    pub source: String,
    /// The damage before blocking, armour and effects.
    pub amount: f32,
    pub blocked: f32,
    /// Health and absorption lost.
    pub taken: f32,
    pub hurt: bool,
    /// `minecraft:armor` and `minecraft:armor_toughness` as the hit left their
    /// instances' caches. `getDamageAfterArmorAbsorb` reads them only for a
    /// source outside `#minecraft:bypasses_armor`, so a hit that bypassed the
    /// armour reports the cache that source never recomputed.
    pub armor: f32,
    pub toughness: f32,
    pub health: f32,
    /// The fire ticks before the hurt.
    pub fire_ticks: i32,
    /// The push the hurt gave, if the caller applied one.
    pub knockback: Option<[f64; 3]>,
}
/// One melee attack on a mob: the damage after the attack strength, the
/// strength (`getAttackStrengthScale`), and the health it took.
#[derive(Clone, Debug, PartialEq)]
pub struct AttackRecord {
    pub game_time: i64,
    pub target: String,
    /// The mob's entity id, and whether the blow killed it.
    pub id: u32,
    pub killed: bool,
    pub amount: f32,
    pub scale: f32,
    pub crit: bool,
    pub landed: bool,
    pub dealt: f32,
}
/// The outcome of a hurt with a raised blocking item.
#[derive(Clone, Copy, Debug, Default)]
pub struct BlockOutcome {
    /// `hurtBlockingItem` ran, including when reductions resolve to zero.
    pub blocking_item_used: bool,
    pub hurt: bool,
    pub blocked: f32,
    /// The durability the blocking item loses (`ItemDamageFunction`, or what a
    /// native mod set).
    pub shield_damage: i32,
    /// `applyItemBlocking`'s `damageBlocked >= damage`, against the damage
    /// `Player.hurtServer` scaled with the difficulty.
    pub fully_blocked: bool,
    /// `BlocksAttacks.onBlocked` ran (the block sound plays).
    pub sound: bool,
    /// The hit knocks the player back (`dealDefaultKnockback`): not one in
    /// the hurt cooldown, nor one the shield took whole.
    pub knockback: bool,
    /// A native mod canceled the hit: its attacker must not be credited.
    pub canceled: bool,
    /// The damage `ENTITY_HURT_PLAYER` is told of: past the block and the helmet.
    pub dealt: f32,
}
struct MatchedSets;
impl horizon_world::blocking_rules::TypeMatchWorld for MatchedSets {
    type Error = io::Error;
    type Set = bool;
    fn source_in_set(&mut self, set: bool) -> io::Result<bool> {
        Ok(set)
    }
}
impl Environment<'_> {
    fn event(&mut self, event: HealthEvent) {
        self.events.push(MotionEvent::Health(event));
    }
    fn phase(&mut self, name: &'static str) -> io::Result<()> {
        self.event(HealthEvent::Phase(name));
        Ok(())
    }
    fn stat(&mut self, name: &'static str, amount: i32) -> io::Result<()> {
        self.event(HealthEvent::Statistic { name, amount });
        Ok(())
    }
}
impl Observer for Environment<'_> {
    type Error = io::Error;
    fn updated(&mut self, data: &mut SyncedFloats, id: u8) -> io::Result<()> {
        Updates(self.events).updated(data, id)
    }
}
impl<'a> ArmorWorld for Environment<'a> {
    type Error = io::Error;
    type Weapon = &'a ArmorWeapon;
    type ServerLevel = ();
    fn weapon(&mut self) -> io::Result<Option<&'a ArmorWeapon>> {
        Ok(self.weapon)
    }
    fn server_level(&mut self) -> io::Result<Option<()>> {
        Ok(Some(()))
    }
    /// `EnchantmentHelper.modifyArmorEffectiveness`: the weapon's
    /// `armor_effectiveness` effects over the fraction the armour, toughness
    /// and damage left.
    fn modify_armor_effectiveness(
        &mut self,
        _: (),
        weapon: &'a ArmorWeapon,
        value: f32,
    ) -> io::Result<f32> {
        let ctx = crate::enchant::EffectContext {
            // `Enchantment.damageContext` carries no tool, so `match_tool`
            // matches nothing and never reads the item tables.
            items: None,
            tool: None,
            damage_type: Some(&self.source.definition().name),
            is_direct: weapon.is_direct,
            this: Some("minecraft:player"),
            attacker: Some(&weapon.attacker),
            direct_attacker: Some(&weapon.attacker),
            direct_motion: None,
        };
        Ok(weapon.enchantments.modify_value(
            Some(&weapon.stack),
            "minecraft:armor_effectiveness",
            &ctx,
            &mut *self.random,
            value,
        ))
    }
}
impl ServerDamageEnvironment<Attributes> for Environment<'_> {
    type Attacker = ();
    fn invulnerable(&mut self) -> io::Result<bool> {
        damage_immunity::server_player_immunity(self)
    }
    fn bypasses(&mut self, kind: Bypass) -> io::Result<bool> {
        Ok(self.source.bypasses(kind))
    }
    fn hurt_armor(&mut self, _: &mut Attributes, damage: f32) -> io::Result<()> {
        // The worn armor lives in the game, which wears it from this event.
        let hit = self
            .level
            .attack()
            .player_source(&self.source.definition().name, self.causing);
        self.event(HealthEvent::ArmorHurt {
            source: self.source.type_id(),
            causing: hit.causing,
            direct: hit.direct,
            position: hit.position,
            amount: damage,
        });
        Ok(())
    }
    fn has_resistance(&mut self) -> io::Result<bool> {
        Ok(self.status.resistance.is_some())
    }
    fn resistance_amplifier(&mut self) -> io::Result<i32> {
        self.status
            .resistance
            .ok_or_else(|| io::Error::other("no resistance effect"))
    }
    fn award_resisted(&mut self, value: i32) -> io::Result<()> {
        self.stat("resisted", value)
    }
    fn attacker(&mut self) -> io::Result<Option<()>> {
        Ok(None)
    }
    fn award_dealt_resisted(&mut self, _: i32) -> io::Result<()> {
        Err(io::Error::other("no damage attacker"))
    }
    fn damage_protection(&mut self, _: ()) -> io::Result<f32> {
        Ok(self.status.protection(&self.source.definition().name))
    }
    fn award_self(&mut self, stat: DamageStat, value: i32) -> io::Result<()> {
        self.stat(
            match stat {
                DamageStat::Absorbed => "absorbed",
                DamageStat::Taken => "taken",
                DamageStat::DealtAbsorbed => return Err(io::Error::other("attacker statistic")),
            },
            value,
        )
    }
    fn award_attacker(&mut self, _: (), _: DamageStat, _: i32) -> io::Result<()> {
        Err(io::Error::other("no damage attacker"))
    }
    fn source_food_exhaustion(&mut self) -> io::Result<f32> {
        Ok(self.source.definition().exhaustion)
    }
    fn ability_invulnerable(&self) -> bool {
        self.invulnerable
    }
    fn level_is_client(&mut self) -> io::Result<bool> {
        Ok(false)
    }
    fn record_damage(&mut self, amount: f32) -> io::Result<()> {
        self.event(HealthEvent::RecordedDamage {
            source: self.source.type_id(),
            amount,
        });
        Ok(())
    }
    fn damage_event(&mut self) -> io::Result<()> {
        self.phase("entity-damage")
    }
    fn damage_pre(&mut self, container: &mut DamageContainer) -> io::Result<()> {
        container.record_shield_damage(self.shield_damage as f32);
        let source = self
            .level
            .attack()
            .player_source(&self.source.definition().name, self.causing);
        self.level
            .damage_pre(Hurting::Player(self.entity, source), container);
        Ok(())
    }
}
impl CombatEnvironment<Attributes> for Environment<'_> {
    fn dead_flag(&self) -> bool {
        *self.dead
    }
}
impl ImmunityWorld for Environment<'_> {
    type Error = io::Error;
    fn removed(&self) -> bool {
        false
    }
    fn permanently_invulnerable(&mut self) -> io::Result<bool> {
        Ok(self.permanently_invulnerable)
    }
    fn temporarily_invulnerable(&mut self) -> io::Result<bool> {
        Ok(self.invulnerable_time)
    }
    fn tag(&mut self, tag: ImmunityTag) -> io::Result<bool> {
        Ok(self.source.immunity_tag(tag))
    }
    fn creative_source(&mut self) -> io::Result<bool> {
        Ok(self.creative_source)
    }
    fn fire_immune(&mut self) -> io::Result<bool> {
        Ok(false)
    }
    fn fall_immune(&mut self) -> io::Result<bool> {
        Ok(false)
    }
    fn enchantment_immunity(&mut self) -> io::Result<bool> {
        Ok(false)
    }
    fn damage_rule(&mut self, rule: DamageRule) -> io::Result<bool> {
        Ok(match rule {
            DamageRule::Drowning => self.rules.drowning_damage,
            DamageRule::Fall => self.rules.fall_damage,
            DamageRule::Fire => self.rules.fire_damage,
            DamageRule::Freeze => self.rules.freeze_damage,
        })
    }
    fn changing_dimension(&mut self) -> io::Result<bool> {
        Ok(false)
    }
    fn ender_pearl(&mut self) -> io::Result<bool> {
        Ok(self.source.is_type("minecraft:ender_pearl"))
    }
    fn client_loaded(&mut self) -> io::Result<bool> {
        Ok(self.client_loaded)
    }
    fn invulnerability_check(&mut self, vanilla: bool) -> io::Result<bool> {
        Ok(self.observe_hit_invulnerability(vanilla))
    }
}
impl HurtWorld for Environment<'_> {
    type Error = io::Error;
    type Source = DamageTypeId;
    type UseItem = ();
    type Blocking = ();
    type Sound = ();
    fn source(&self) -> DamageTypeId {
        self.source.type_id()
    }
    fn is_server_player(&self) -> bool {
        true
    }
    fn invulnerable(&mut self, _: &mut State) -> io::Result<bool> {
        damage_immunity::server_player_immunity(self)
    }
    fn dead_or_dying(&mut self, _: &mut State) -> io::Result<bool> {
        Err(io::Error::other("combat binding supplies health predicate"))
    }
    fn tag(&mut self, _: &mut State, tag: HurtTag) -> io::Result<bool> {
        Ok(self.source.hurt_tag(tag))
    }
    fn fire_resistance(&mut self, _: &mut State) -> io::Result<bool> {
        Ok(self.status.fire_resistance)
    }
    fn hurt_start(&mut self, _: &mut State, container: &mut DamageContainer) -> io::Result<bool> {
        let source = self
            .level
            .attack()
            .player_source(&self.source.definition().name, self.causing);
        let canceled = self.level.incoming_canceled(
            Hurting::Player(self.entity, source),
            self.victim,
            container,
        );
        self.canceled |= canceled;
        Ok(canceled)
    }
    fn sleeping(&mut self, _: &mut State) -> io::Result<bool> {
        Ok(false)
    }
    fn stop_sleeping(&mut self, _: &mut State) -> io::Result<()> {
        Ok(())
    }
    fn use_item(&mut self, _: &mut State) -> io::Result<()> {
        Ok(())
    }
    fn apply_blocking(&mut self, _: &mut State, damage: f32) -> io::Result<f32> {
        if damage <= 0.0 {
            return Ok(0.0);
        }
        // Nothing is stopped without a raised item, or when its `bypassed_by` or a
        // piercing arrow skips it; a native mod may still force a block.
        let vanilla = match &self.blocking {
            Some(b) if !b.bypassed && !b.pierce => {
                b.rules.resolve(&mut MatchedSets, damage, b.angle)?
            }
            _ => 0.0,
        };
        let shield_damage = self
            .blocking
            .as_ref()
            .map_or(0, |b| b.rules.item_damage.apply(vanilla));
        let block = if vanilla > 0.0 {
            ShieldBlock {
                blocked: true,
                blocked_damage: vanilla,
                shield_damage,
            }
        } else {
            ShieldBlock {
                shield_damage,
                ..no_block(damage)
            }
        };
        let source = self
            .level
            .attack()
            .player_source(&self.source.definition().name, self.causing);
        let Some(block) =
            self.level
                .shield_block(Hurting::Player(self.entity, source), damage, block)
        else {
            return Ok(0.0);
        };
        self.blocking_item_used = true;
        self.blocked = block.blocked_damage;
        // `setShieldDamage` is what the item loses if `blocked` holds, whatever
        // `blocked_damage` says; below 0, the item's own `itemDamage` of what is stopped.
        self.shield_damage = if !block.blocked {
            0
        } else if block.shield_damage < 0 {
            self.blocking
                .as_ref()
                .map_or(0, |b| b.rules.item_damage.apply(block.blocked_damage))
        } else {
            block.shield_damage
        };
        self.fully_blocked = self.blocked >= damage;
        Ok(self.blocked)
    }
    fn freeze_hurts_extra(&mut self, _: &mut State) -> io::Result<bool> {
        Ok(false)
    }
    fn helmet_empty(&mut self, _: &mut State) -> io::Result<bool> {
        Ok(!self.helmet)
    }
    /// The game wears it between the hit's halves ([`PlayerMotion::start_hurt`]).
    fn hurt_helmet(&mut self, _: &mut State, _: f32) -> io::Result<()> {
        Ok(())
    }
    fn actually_hurt(&mut self, _: &mut State, _: &mut DamageContainer, _: f32) -> io::Result<()> {
        Err(io::Error::other("combat binding supplies inner damage"))
    }
    fn resolve_mob(&mut self, _: &mut State) -> io::Result<()> {
        Ok(())
    }
    fn resolve_player(&mut self, _: &mut State) -> io::Result<()> {
        Ok(())
    }
    fn blocking_component(&mut self, _: ()) -> Option<()> {
        self.blocking.as_ref().map(|_| ())
    }
    fn on_blocked(&mut self, _: &mut State, _: ()) -> io::Result<()> {
        self.block_sound = true;
        Ok(())
    }
    fn broadcast_damage(&mut self, _: &mut State) -> io::Result<()> {
        self.event(HealthEvent::DamageBroadcast {
            source: self.source.type_id(),
        });
        Ok(())
    }
    fn mark_hurt(&mut self, _: &mut State) -> io::Result<()> {
        *self.sync_velocity = true;
        Ok(())
    }
    /// The caller applies it: these sources carry no entity or position.
    fn knockback(&mut self, _: &mut State, _: f32, _: bool) -> io::Result<()> {
        self.knocked = true;
        Ok(())
    }
    fn death_protection(&mut self, _: &mut State) -> io::Result<bool> {
        if !self.totem
            || self
                .source
                .immunity_tag(ImmunityTag::BypassesInvulnerability)
        {
            return Ok(false);
        }
        let source = self
            .level
            .attack()
            .player_source(&self.source.definition().name, self.causing);
        // A native mod may cancel the totem: the player dies.
        if self
            .level
            .totem_canceled(Hurting::Player(self.entity, source), self.totem_off_hand)
        {
            return Ok(false);
        }
        self.totem_fired = true;
        Ok(true)
    }
    fn death_sound(&mut self, _: &mut State) -> io::Result<()> {
        Ok(())
    }
    fn make_sound(&mut self, _: &mut State, _: ()) -> io::Result<()> {
        self.phase("death-sound")
    }
    fn secondary_sound(&mut self, _: &mut State) -> io::Result<()> {
        if self.source.definition().effects == "thorns" {
            self.phase("thorns-sound")?;
        }
        Ok(())
    }
    fn die(&mut self, _: &mut State) -> io::Result<()> {
        *self.dead = true;
        self.phase("death-request")
    }
    /// `Player.playSound` leaves the player itself out: others hear this one.
    fn hurt_sound(&mut self, _: &mut State) -> io::Result<()> {
        if !self.block_sound {
            let sound = hurt_sound(&self.source.definition().effects);
            self.events.push(MotionEvent::HurtSound(sound));
        }
        self.phase("hurt-sound")
    }
    fn game_time(&mut self, _: &mut State) -> io::Result<i64> {
        Ok(self.game_time)
    }
    fn effects_on_hurt(&mut self, _: &mut State, _: f32) -> io::Result<()> {
        // `for (MobEffectInstance effect : getActiveEffects()) effect.onMobHurt(..)`:
        // Infested is the only effect that answers a hit, and its roll draws
        // from the level's random, so the game runs it.
        if self.status.infested {
            self.event(HealthEvent::MobHurt);
        }
        Ok(())
    }
    fn player_hurt_criterion(
        &mut self,
        _: &mut State,
        original: f32,
        amount: f32,
        blocked: bool,
    ) -> io::Result<()> {
        self.dealt = amount;
        self.event(HealthEvent::Criterion {
            source: self.source.type_id(),
            original,
            amount,
            blocked,
        });
        Ok(())
    }
    fn award_blocked(&mut self, _: &mut State, amount: i32) -> io::Result<()> {
        self.stat("blocked", amount)
    }
    fn attacker_criterion(&mut self, _: &mut State, _: f32, _: f32, _: bool) -> io::Result<()> {
        Ok(())
    }
    fn damage_post(&mut self, _: &mut State, container: &DamageContainer) -> io::Result<()> {
        let source = HurtSource {
            damage_type: &self.source.definition().name,
            causing: self.causing,
            direct: self
                .level
                .attack()
                .projectile_direct(&self.source.definition().name),
            position: None,
        };
        self.level
            .damage_post(Hurting::Player(self.entity, source), container);
        Ok(())
    }
}
impl PlayerHurtWorld for Environment<'_> {
    fn ability_invulnerable(&self) -> bool {
        self.invulnerable
    }
    fn bypasses_invulnerability(&mut self, _: &mut State) -> io::Result<bool> {
        Ok(self.source.in_tag("minecraft:bypasses_invulnerability"))
    }
    fn remove_shoulders(&mut self, _: &mut State) -> io::Result<()> {
        Ok(())
    }
    fn scales_with_difficulty(&mut self, _: &mut State) -> io::Result<bool> {
        // DamageSource.scalesWithDifficulty.
        Ok(match self.source.definition().scaling {
            Scaling::Never => false,
            Scaling::Always => true,
            Scaling::LivingNonPlayer => self.living_cause,
        })
    }
    fn difficulty(&mut self, _: &mut State) -> io::Result<Difficulty> {
        Ok(self.rules.difficulty)
    }
}
impl ServerPlayerHurtWorld for Environment<'_> {
    type Player = ();
    type Arrow = ();
    fn causing_entity(&mut self, _: &mut State) -> io::Result<CausingEntity<(), ()>> {
        Ok(if self.can_harm_player {
            CausingEntity::Other
        } else {
            CausingEntity::Player(())
        })
    }
    fn arrow_player_owner(&mut self, _: &mut State, _: ()) -> io::Result<Option<()>> {
        Ok(None)
    }
    fn can_harm_player(&mut self, _: &mut State, _: ()) -> io::Result<bool> {
        Ok(self.can_harm_player)
    }
}

#[cfg(test)]
mod datapack_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_hook_can_refuse_one_damage_type_and_allow_another() {
        use crate::mobs::{Damage, DamageKind, Mob};
        fn immune_to_generic(_: DamageLevel<'_>, hurting: Hurting<'_>, vanilla: bool) -> bool {
            match hurting {
                Hurting::Mob(_, hit) if matches!(hit.kind, DamageKind::Generic) => true,
                _ => vanilla,
            }
        }
        let hooks = DamageHooks {
            invulnerable: Some(immune_to_generic),
            ..DamageHooks::NONE
        };
        let mut test_level = TestLevel::with_hooks(0, hooks);
        let level = &mut test_level.level();
        let mut mob = Mob::new(
            9201,
            crate::mobs::sensor_scan_tests::kind(),
            Default::default(),
            0.0,
        );
        assert!(!mob.damage(level, 2.0, Damage::of(DamageKind::Generic)));
        assert_eq!(mob.health, 10.0);
        assert!(mob.damage(level, 2.0, Damage::of(DamageKind::Fall)));
        assert_eq!(mob.health, 8.0);
    }

    #[test]
    fn the_weapon_is_the_attackers_main_hand_only_for_its_own_hit() {
        let sword = crate::items::ItemStack::new("minecraft:iron_sword", 1);
        let attack = Attack {
            attacker: 7,
            projectile: None,
            direct: None,
            player_uuid: None,
            held: [Some(sword.clone()), None],
            ..Default::default()
        };
        assert_eq!(attack.weapon_of(7), Some(&sword));
        // An arrow the attacker shot is the direct entity, not the attacker: no weapon.
        assert_eq!(attack.weapon_of(9), None);
        assert_eq!(attack.weapon_of(0), None);
        assert_eq!(NO_ATTACK.weapon_of(0), None);
    }
    #[test]
    fn magic_during_a_projectile_impact_names_no_direct_entity() {
        // DamageSources.magic() is entity-free. An ownerless wither skull and a
        // tipped arrow's instant harm both deal it while the projectile's attack
        // is still in scope; arrow, wither_skull and indirect_magic name it.
        let attack = Attack {
            attacker: 0,
            projectile: Some(12),
            direct: None,
            player_uuid: None,
            held: [None, None],
            ..Default::default()
        };
        assert_eq!(attack.projectile_direct("minecraft:arrow"), 12);
        assert_eq!(attack.projectile_direct("minecraft:wither_skull"), 12);
        assert_eq!(attack.projectile_direct("minecraft:spit"), 12);
        assert_eq!(attack.projectile_direct("minecraft:indirect_magic"), 12);
        assert_eq!(attack.projectile_direct("minecraft:magic"), 0);
        assert_eq!(attack.projectile_direct("minecraft:explosion"), 0);
        assert_eq!(NO_ATTACK.projectile_direct("minecraft:arrow"), 0);
    }
    #[test]
    fn a_knockback_hook_cannot_make_the_push_nonfinite() {
        fn hook(_: DamageLevel<'_>, _: Hurting<'_>, knock: &mut Knock) -> bool {
            knock.strength = f64::NAN;
            knock.x *= 2.0;
            false
        }
        let mut level = crate::health::TestLevel::with_hooks(
            0,
            DamageHooks {
                knockback: Some(hook),
                ..DamageHooks::NONE
            },
        );
        let source = HurtSource {
            damage_type: "minecraft:generic",
            causing: 0,
            direct: 0,
            position: None,
        };
        let mut knock = Knock {
            strength: 0.4,
            x: 1.0,
            z: 0.5,
        };
        assert!(
            !level
                .level()
                .knockback_canceled(Hurting::Player(1, source), &mut knock)
        );
        // A value left that is not a number is put back; one that is finite stays changed.
        assert_eq!(
            knock,
            Knock {
                strength: 0.4,
                x: 2.0,
                z: 0.5
            }
        );
    }
    #[test]
    fn a_hit_in_the_hurt_cooldown_knocks_nobody_back() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = crate::assets::Assets::load(&root).unwrap();
        let resources = crate::motion::MotionResources::load(&root, &assets).unwrap();
        let mut player =
            crate::motion::PlayerMotion::new(&resources, horizon_core::geometry::Vec3::ZERO)
                .unwrap();
        let (health, rules) = (&resources.health, super::HealthRules::default());
        let id = health.damage.id("minecraft:mob_attack").unwrap();
        let mut hit = |amount| {
            player
                .hurt_blocking(level, health, rules, id, amount, None)
                .unwrap()
        };
        let first = hit(1.0);
        assert!(first.hurt && first.knockback);
        // A stronger hit inside the cooldown hurts by the difference only
        // (LivingEntity.hurtServer), with no dealDefaultKnockback.
        let second = hit(2.0);
        assert!(second.hurt && !second.knockback);
        // A weaker one doesn't hurt at all.
        let third = hit(1.5);
        assert!(!third.hurt && !third.knockback);
    }

    #[test]
    fn player_post_records_fresh_hit_decision() {
        use horizon_world::living_damage::DamageValues;
        static SEEN: std::sync::Mutex<Vec<super::DamageContainer>> =
            std::sync::Mutex::new(Vec::new());
        fn post(
            _: super::DamageLevel<'_>,
            _: super::Hurting<'_>,
            container: &super::DamageContainer,
        ) {
            // The plain values used by wasm must preserve the decision in both directions.
            let values = DamageValues::from(container);
            assert_eq!(
                values.should_cause_side_effects,
                container.should_cause_side_effects()
            );
            assert_eq!(super::DamageContainer::from(values), *container);
            SEEN.lock().unwrap().push(container.clone());
        }
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = crate::assets::Assets::load(&root).unwrap();
        let resources = crate::motion::MotionResources::load(&root, &assets).unwrap();
        let mut player = crate::motion::PlayerMotion::new(&resources, Default::default()).unwrap();
        let hooks = super::DamageHooks {
            post: Some(post),
            ..super::DamageHooks::NONE
        };
        let mut level = super::TestLevel::with_hooks(0, hooks);
        let health = &resources.health;
        let rules = super::HealthRules::default();
        let attack = health.damage.id("minecraft:mob_attack").unwrap();
        assert!(
            player
                .hurt(&mut level.level(), health, rules, attack, 2.0)
                .unwrap()
        );
        assert!(
            player
                .hurt(&mut level.level(), health, rules, attack, 4.0)
                .unwrap()
        );
        assert!(
            !player
                .hurt(&mut level.level(), health, rules, attack, 3.0)
                .unwrap()
        );
        let seen = SEEN.lock().unwrap();
        assert_eq!(seen.len(), 2, "the refused hit must not fire Post");
        let flags: Vec<_> = seen
            .iter()
            .map(|hit| hit.should_cause_side_effects())
            .collect();
        assert_eq!(flags, [true, false]);
        // Both first hits deal 2 and leave 20 ticks: neither value determines freshness.
        for hit in &seen[..2] {
            assert_eq!(hit.health_damage(), 2.0);
            assert_eq!(hit.post_attack_invulnerability_ticks(), 20);
        }
    }

    /// #2800: on Easy a mob's 4 is scaled to 3 before `applyItemBlocking`,
    /// so a shield that stops all 3 blocks fully (`damageBlocked >= damage`)
    /// and `blockedByItem` must not pull the player toward the mob.
    #[test]
    fn a_shield_fully_blocks_a_hit_scaled_down_by_easy() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        use horizon_world::blocking_rules::{BlockingRules, DamageReduction, ItemDamageFunction};
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = crate::assets::Assets::load(&root).unwrap();
        let resources = crate::motion::MotionResources::load(&root, &assets).unwrap();
        let mut player =
            crate::motion::PlayerMotion::new(&resources, horizon_core::geometry::Vec3::ZERO)
                .unwrap();
        let health = &resources.health;
        let rules = super::HealthRules {
            difficulty: horizon_world::food::Difficulty::Easy,
            ..Default::default()
        };
        let shield = super::Blocking {
            rules: BlockingRules {
                delay_seconds: 0.25,
                disable_scale: 1.0,
                reductions: vec![DamageReduction {
                    horizontal_angle: 90.0,
                    types: None,
                    base: 0.0,
                    factor: 1.0,
                }],
                item_damage: ItemDamageFunction {
                    threshold: 3.0,
                    base: 1.0,
                    factor: 1.0,
                },
            },
            bypassed: false,
            pierce: false,
            angle: 0.0,
        };
        let id = health.damage.id("minecraft:mob_attack").unwrap();
        let o = player
            .hurt_caused_by(level, health, rules, id, 4.0, Some(shield), true, 0, false)
            .unwrap();
        assert_eq!(o.blocked, 3.0, "Easy scales 4 to 3 before the shield");
        assert!(o.fully_blocked && !o.knockback);
    }

    /// #7309: on Hard a mob's 4 is scaled to 6 (`Player.hurtServer`), on
    /// Normal it stays 4, and a non-mob source is never scaled.
    #[test]
    fn hard_scales_a_mob_hit_by_one_and_a_half() {
        use horizon_world::food::Difficulty;
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = crate::assets::Assets::load(&root).unwrap();
        let resources = crate::motion::MotionResources::load(&root, &assets).unwrap();
        let health = &resources.health;
        let id = health.damage.id("minecraft:mob_attack").unwrap();
        let lost = |difficulty, living| {
            let mut player =
                crate::motion::PlayerMotion::new(&resources, horizon_core::geometry::Vec3::ZERO)
                    .unwrap();
            let rules = super::HealthRules {
                difficulty,
                ..Default::default()
            };
            let before = player.health.health();
            player
                .hurt_caused_by(
                    &mut super::TestLevel::new(0).level(),
                    health,
                    rules,
                    id,
                    4.0,
                    None,
                    living,
                    0,
                    false,
                )
                .unwrap();
            before - player.health.health()
        };
        assert_eq!(lost(Difficulty::Hard, true), 6.0);
        assert_eq!(lost(Difficulty::Normal, true), 4.0);
        assert_eq!(lost(Difficulty::Easy, true), 3.0);
        assert_eq!(lost(Difficulty::Hard, false), 4.0);
    }

    /// #2141: an arrow that takes the last of the player's health must kill
    /// them: `LivingEntity.hurt` leaves the health at zero and `Player.die`
    /// follows.
    #[test]
    fn an_arrow_kills_a_player_at_one_health() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = crate::assets::Assets::load(&root).unwrap();
        let resources = crate::motion::MotionResources::load(&root, &assets).unwrap();
        let mut player =
            crate::motion::PlayerMotion::new(&resources, horizon_core::geometry::Vec3::ZERO)
                .unwrap();
        player.set_health(1.0, false).unwrap();
        let (health, rules) = (&resources.health, super::HealthRules::default());
        let id = health.damage.id("minecraft:arrow").unwrap();
        let hurt = player.hurt(level, health, rules, id, 5.0).unwrap();
        assert!(hurt, "the arrow must hurt");
        assert_eq!(player.health.health(), 0.0, "the health must be spent");
        assert!(player.health.dead_or_dying(), "the player must be dead");
    }

    /// What the hook of these tests saw: entity, damage type (a mob's kind)
    /// and causing entity, amount.
    static SEEN: std::sync::Mutex<Vec<(i32, String, f32)>> = std::sync::Mutex::new(Vec::new());
    /// Sets the native damage hook of these tests (global: it stays set for
    /// the process): for entities 9001 to 9004 and 9011 to 9013 it records the
    /// hit, cancels x1's, doubles x2's, makes 9003's -5 and 9004's NaN, and gives 9005's hit an
    /// armor and an absorption reduction modifier.
    fn set_hook() -> (super::DamageHooks, fn(i32) -> Vec<(String, f32)>) {
        use super::{DamageContainer, DamageLevel, Hurting, Victim};
        fn hook(
            _: DamageLevel<'_>,
            hurting: Hurting<'_>,
            _: Victim,
            container: &mut DamageContainer,
        ) -> bool {
            let (entity, seen) = match hurting {
                Hurting::Player(e, s)
                | Hurting::Dragon(e, s)
                | Hurting::ArmorStand(e, s)
                | Hurting::Entity(e, _, s) => (e, format!("{} {}", s.damage_type, s.causing)),
                Hurting::Mob(mob, source) => (mob.id as i32, format!("{:?}", source.kind)),
            };
            if !matches!(entity, 9001..=9005 | 9011..=9013) {
                return false;
            }
            let amount = container.new_damage();
            SEEN.lock().unwrap().push((entity, seen, amount));
            match entity {
                9002 | 9012 => container.set_new_damage(amount * 2.0),
                9003 => container.set_new_damage(-5.0),
                9004 => container.set_new_damage(f32::NAN),
                9005 => {
                    use horizon_world::living_damage::{Reduction, ReductionChange};
                    container.add_reduction_modifier(Reduction::Armor, ReductionChange::Add(1.0));
                    container.add_reduction_modifier(Reduction::Armor, ReductionChange::Scale(2.0));
                    container
                        .add_reduction_modifier(Reduction::Absorption, ReductionChange::Scale(0.5));
                }
                _ => {}
            }
            entity % 10 == 1
        }
        let hooks = super::DamageHooks {
            incoming: Some(hook),
            ..super::DamageHooks::NONE
        };
        (hooks, |entity| {
            let seen = SEEN.lock().unwrap();
            let hits = seen.iter().filter(|s| s.0 == entity);
            hits.map(|s| (s.1.clone(), s.2)).collect()
        })
    }

    #[test]
    fn the_native_damage_hook_changes_or_cancels_a_mobs_hit() {
        let (hooks, seen) = set_hook();
        let mut test_level = crate::health::TestLevel::with_hooks(0, hooks);
        let level = &mut test_level.level();
        use crate::mobs::{Damage, DamageKind, Mob};
        let mob = |id| {
            Mob::new(
                id,
                crate::mobs::sensor_scan_tests::kind(),
                Default::default(),
                0.0,
            )
        };
        let hit = Damage {
            kind: DamageKind::PlayerAttack,
            attacker: None,
            from: Some([3.0, 0.0, 0.0]),
            attacker_species: None,
        };
        // Canceled: no damage, cooldown, sound or knockback.
        let mut kept = mob(9001);
        assert!(!kept.damage(level, 2.0, hit));
        assert_eq!(
            (kept.health, kept.invulnerable, kept.hurt_time),
            (10.0, 0, 0)
        );
        assert!(kept.outbox.is_empty() && kept.velocity == Default::default());
        assert_eq!(seen(9001), [("PlayerAttack".to_owned(), 2.0)]);
        let mut doubled = mob(9002);
        assert!(doubled.damage(level, 2.0, hit));
        assert_eq!(doubled.health, 6.0);
        // LivingEntity.hurtServer: a negative amount hurts by 0, a NaN by
        // Float.MAX_VALUE.
        let (mut negative, mut nan) = (mob(9003), mob(9004));
        assert!(negative.damage(level, 2.0, hit) && nan.damage(level, 2.0, hit));
        assert!(negative.health == 10.0 && nan.health <= 0.0);
        // 8 damage on a mob with 1.5 absorption hearts. Armor took none: +1 then x2 is 2, so 6
        // goes on. The hearts would take their 1.5; half of it is 0.75, so 5.25 reaches the
        // health (10 - 5.25) and 0.75 hearts are left.
        let mut modified = mob(9005);
        modified.add_effect("absorption", 600, 0);
        modified.absorption = 1.5;
        assert!(modified.damage(level, 8.0, hit));
        assert_eq!((modified.health, modified.absorption), (4.75, 0.75));
        // The same hit with no modifier: 8 less the 1.5 hearts.
        let mut plain = mob(9006);
        plain.add_effect("absorption", 600, 0);
        plain.absorption = 1.5;
        assert!(plain.damage(level, 8.0, hit));
        assert_eq!((plain.health, plain.absorption), (3.5, 0.0));
    }

    /// Needs the prepared data in `.local/horizon-client/26.3`; skipped
    /// without it.
    #[test]
    fn the_native_damage_hook_changes_or_cancels_a_players_hit() {
        let (hooks, seen) = set_hook();
        let mut test_level = crate::health::TestLevel::with_hooks(0, hooks);
        let level = &mut test_level.level();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = crate::assets::Assets::load(&root).unwrap();
        let resources = crate::motion::MotionResources::load(&root, &assets).unwrap();
        let (health, rules) = (&resources.health, super::HealthRules::default());
        let id = health.damage.id("minecraft:arrow").unwrap();
        let player = |entity| {
            let mut p = crate::motion::PlayerMotion::new(&resources, Default::default()).unwrap();
            p.entity_id = entity;
            p
        };
        let mut kept = player(9011);
        assert!(!kept.hurt(level, health, rules, id, 5.0).unwrap());
        assert_eq!((kept.health.health(), kept.health.hurt.cooldown), (20.0, 0));
        let mut doubled = player(9012);
        assert!(doubled.hurt(level, health, rules, id, 5.0).unwrap());
        assert_eq!(doubled.health.health(), 10.0);
        // The causing mob when it is known, else 0.
        let hurt =
            player(9013).hurt_caused_by(level, health, rules, id, 5.0, None, false, 44, false);
        assert!(hurt.unwrap().hurt);
        assert_eq!(seen(9011), [("minecraft:arrow 0".to_owned(), 5.0)]);
        assert_eq!(seen(9013), [("minecraft:arrow 44".to_owned(), 5.0)]);
    }

    /// What the knockback hook of these tests saw: the entity, the hit that
    /// causes it (a mob's damage kind) and the push it was handed.
    static PUSHES: std::sync::Mutex<Vec<(i32, String, super::Knock)>> =
        std::sync::Mutex::new(Vec::new());
    /// Sets the native knockback hook of these tests for mobs 9101 to 9106 only: it records the push, cancels
    /// 9101's, doubles 9102's strength and turns its direction around, leaves
    /// 9103 non-finite numbers, and gives 9105 (whose resistance leaves
    /// nothing) a strength of 0.4.
    fn set_knockback_hook() -> (super::DamageHooks, fn(i32) -> Vec<(String, super::Knock)>) {
        use super::{DamageLevel, Hurting, Knock};
        fn hook(_: DamageLevel<'_>, pushed: Hurting<'_>, knock: &mut Knock) -> bool {
            let Hurting::Mob(mob, source) = pushed else {
                return false;
            };
            let entity = mob.id as i32;
            if !(9101..=9106).contains(&entity) {
                return false;
            }
            let hit = format!("{:?}", source.kind);
            PUSHES.lock().unwrap().push((entity, hit, *knock));
            match entity {
                9101 => return true,
                9102 => {
                    knock.strength *= 2.0;
                    knock.x = -knock.x;
                }
                9103 => {
                    knock.strength = f64::NAN;
                    knock.x = f64::INFINITY;
                    knock.z = f64::NEG_INFINITY;
                }
                9105 => knock.strength = 0.4,
                _ => {}
            }
            false
        }
        let hooks = super::DamageHooks {
            knockback: Some(hook),
            ..super::DamageHooks::NONE
        };
        (hooks, |entity| {
            let seen = PUSHES.lock().unwrap();
            let pushes = seen.iter().filter(|p| p.0 == entity);
            pushes.map(|p| (p.1.clone(), p.2)).collect()
        })
    }

    /// A zombie in full netherite (0.1 a piece) is pushed 60 % as far as one
    /// with no armour: `knockback` scales by the attribute's value with the
    /// gear's modifiers, which the game hands the mob as `gear_knockback_resistance`.
    #[test]
    fn a_mobs_gear_resists_knockback() {
        use crate::mobs::{Damage, Mob};
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let hit = Damage::player([3.0, 0.0, 0.0]);
        let mut pushed = |gear: Option<f64>, own: f64| {
            let mut m = Mob::new(
                9200,
                crate::mobs::sensor_scan_tests::kind(),
                Default::default(),
                0.0,
            );
            m.on_ground = true;
            m.attrs.knockback_resistance = own;
            m.gear_knockback_resistance = gear;
            m.knockback(level, 0.4, 3.0, 0.0, &hit, 0.0);
            m.velocity.x
        };
        assert_eq!(pushed(None, 0.0), -0.4);
        assert!((pushed(Some(0.4), 0.0) - -0.4 * 0.6).abs() < 1e-12);
        // The gear's value is the whole attribute: it replaces the mob's own.
        assert!((pushed(Some(0.4), 0.5) - -0.4 * 0.6).abs() < 1e-12);
    }
    /// `LivingEntity.knockback`: the hook runs before knockback resistance
    /// takes its share and before motion changes; a mod's strength or
    /// direction is what pushes, a cancel pushes nothing, and with no mod in
    /// the way the push is vanilla's. Expected values are worked out by hand
    /// from vanilla's formula: the horizontal speed halves and the push is
    /// subtracted along the normalized direction; on the ground the vertical
    /// speed becomes `min(0.4, vy / 2 + strength)`.
    #[test]
    fn the_native_knockback_hook_changes_or_cancels_a_mobs_push() {
        let (hooks, seen) = set_knockback_hook();
        let mut test_level = crate::health::TestLevel::with_hooks(0, hooks);
        let level = &mut test_level.level();
        use super::Knock;
        use crate::mobs::{Damage, Mob};
        let mob = |id, resistance| {
            let mut m = Mob::new(
                id,
                crate::mobs::sensor_scan_tests::kind(),
                Default::default(),
                0.0,
            );
            m.on_ground = true;
            m.attrs.knockback_resistance = resistance;
            m
        };
        let velocity = |m: &Mob| (m.velocity.x, m.velocity.y, m.velocity.z);
        let hit = Damage::player([3.0, 0.0, 0.0]);
        let mut push = |id, resistance| {
            let mut m = mob(id, resistance);
            m.knockback(level, 0.4, 3.0, 0.0, &hit, 0.0);
            m
        };
        // No mod in the way (the hook leaves 9100 alone): vanilla's push, away from x = 3.
        let plain = push(9100, 0.0);
        assert_eq!(velocity(&plain), (-0.4, 0.4, 0.0));
        assert!(plain.needs_sync);
        // Canceled: no motion, no resync. The hook saw the push the game made.
        let kept = push(9101, 0.0);
        assert_eq!(velocity(&kept), (0.0, 0.0, 0.0));
        assert!(!kept.needs_sync);
        let made = Knock {
            strength: 0.4,
            x: 3.0,
            z: 0.0,
        };
        assert_eq!(seen(9101), [("PlayerAttack".to_owned(), made)]);
        // A stronger push the other way: twice the strength, x turned around.
        let turned = push(9102, 0.0);
        assert_eq!(velocity(&turned), (0.8, 0.4, 0.0));
        assert_eq!(velocity(&push(9102, 0.5)), (0.4, 0.4, 0.0));
        // Numbers that are not finite are put back as the game made them.
        assert_eq!(velocity(&push(9103, 0.0)), (-0.4, 0.4, 0.0));
        // Before the resistance (`LivingKnockBackEvent`): the hook is handed the raw 0.4, and
        // the push that follows is halved.
        let resisted = push(9104, 0.5);
        assert_eq!(velocity(&resisted), (-0.2, 0.2, 0.0));
        assert_eq!(seen(9104)[0].1.strength, 0.4);
        // A full resistance takes all of what the hook leaves: no push.
        let stopped = push(9105, 1.0);
        assert_eq!(seen(9105)[0].1.strength, 0.4);
        assert_eq!(velocity(&stopped), (0.0, 0.0, 0.0));
        // The game's own hit on a mob reaches the hook with its damage.
        let mut hurt = mob(9106, 0.0);
        assert!(hurt.damage(level, 2.0, hit));
        // dealDefaultKnockback pushes with the float 0.4F.
        let strength = f64::from(0.4_f32);
        assert_eq!(velocity(&hurt), (-strength, 0.4, 0.0));
        let made = Knock { strength, ..made };
        assert_eq!(seen(9106), [("PlayerAttack".to_owned(), made)]);
    }

    /// #4301: `ServerPlayer.isInvulnerableTo`'s `!connection.hasClientLoaded`
    /// term spares a respawned player the void until the client has loaded the
    /// world; once it has, the void hurts again.
    #[test]
    fn the_void_spares_a_player_whose_client_has_not_loaded() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = crate::assets::Assets::load(&root).unwrap();
        let resources = crate::motion::MotionResources::load(&root, &assets).unwrap();
        let mut player =
            crate::motion::PlayerMotion::new(&resources, horizon_core::geometry::Vec3::ZERO)
                .unwrap();
        let (health, rules) = (&resources.health, super::HealthRules::default());
        let id = health.damage.id("minecraft:out_of_world").unwrap();
        player.client_loaded = false;
        assert!(
            !player.hurt(level, health, rules, id, 4.0).unwrap(),
            "an unloaded client must not take void damage"
        );
        assert_eq!(player.health.health(), 20.0);
        player.client_loaded = true;
        assert!(
            player.hurt(level, health, rules, id, 4.0).unwrap(),
            "a loaded client takes the void's 4.0"
        );
        assert_eq!(player.health.health(), 16.0);
    }

    #[test]
    fn hurt_sounds_follow_damage_effects() {
        // DamageEffects: HURT, THORNS, DROWNING, BURNING, POKING, FREEZING.
        for (effects, sound) in [
            ("hurt", "minecraft:entity.player.hurt"),
            ("thorns", "minecraft:entity.player.hurt"),
            ("drowning", "minecraft:entity.player.hurt_drown"),
            ("burning", "minecraft:entity.player.hurt_on_fire"),
            ("poking", "minecraft:entity.player.hurt_sweet_berry_bush"),
            ("freezing", "minecraft:entity.player.hurt_freeze"),
        ] {
            assert_eq!(super::hurt_sound(effects), sound, "{effects}");
        }
    }

    /// `LivingEntity.getDamageAfterArmorAbsorb` returns before
    /// `getArmorValue` and `getAttributeValue(ARMOR_TOUGHNESS)` for a source in
    /// `#minecraft:bypasses_armor`, and native `AttributeInstance.getValue` is
    /// what clears an instance's dirty flag. A hit must therefore leave both
    /// instances dirty with creation's zero cache when it bypasses the armour,
    /// and recompute them when it does not.
    #[test]
    fn a_bypassing_hit_leaves_the_armor_instances_dirty() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = crate::assets::Assets::load(&root).unwrap();
        let resources = crate::motion::MotionResources::load(&root, &assets).unwrap();
        let (health, rules) = (&resources.health, super::HealthRules::default());
        // `setBaseValue` is what a command or a spawn uses; a base other than
        // the attribute's default is what makes the fresh instance's cache
        // stale, and it is the value a recompute would produce.
        let armoured = |resources: &crate::motion::MotionResources| {
            let mut player =
                crate::motion::PlayerMotion::new(resources, horizon_core::geometry::Vec3::ZERO)
                    .unwrap();
            let mut handles = Vec::new();
            for name in ["minecraft:armor", "minecraft:armor_toughness"] {
                let key = player.attributes.key(name).unwrap();
                let handle = player
                    .attributes
                    .get_instance(Some(key))
                    .unwrap()
                    .expect("the player supplier has this attribute");
                player
                    .attributes
                    .edit(&handle, |a, o| a.set_base_value(7.0, o))
                    .unwrap();
                handles.push(handle);
            }
            (player, handles)
        };
        let states = |player: &crate::motion::PlayerMotion,
                      handles: &[horizon_world::attributes::live::Handle]| {
            handles
                .iter()
                .map(|handle| {
                    let a = player.attributes.instance(handle).unwrap();
                    (a.base_value(), a.is_dirty(), a.cached_value())
                })
                .collect::<Vec<_>>()
        };
        // `minecraft:fall` is in the tag: neither instance is read.
        let (mut bypassing, bypassing_handles) = armoured(&resources);
        let fall = health.damage.id("minecraft:fall").unwrap();
        assert!(bypassing.hurt(level, health, rules, fall, 5.0).unwrap());
        assert_eq!(
            states(&bypassing, &bypassing_handles),
            [(7.0, true, 0.0), (7.0, true, 0.0)],
            "a bypassing hit must leave both instances dirty and uncached"
        );
        let record = bypassing.hurt_log.last().unwrap();
        assert_eq!(
            (record.armor, record.toughness),
            (0.0, 0.0),
            "the hit reports the cache it left"
        );
        // `minecraft:mob_attack` is not: `getDamageAfterAbsorb` asks for both
        // values, so the hit recomputes them and the record reads them back.
        let (mut meeting, meeting_handles) = armoured(&resources);
        let attack = health.damage.id("minecraft:mob_attack").unwrap();
        assert!(meeting.hurt(level, health, rules, attack, 5.0).unwrap());
        assert_eq!(
            states(&meeting, &meeting_handles),
            [(7.0, false, 7.0), (7.0, false, 7.0)],
            "a hit the armour meets must recompute both instances"
        );
        let record = meeting.hurt_log.last().unwrap();
        assert_eq!((record.armor, record.toughness), (7.0, 7.0));
    }
}
