//! Potions and status effects in the game: drinking (`PotionItem` via
//! `Consumable`), throwing splash/lingering potions (`ThrownSplashPotion`),
//! filling glass bottles (`BottleItem`), the per-tick effect application,
//! effect attribute modifiers and the HUD effect icons (`Hud.extractEffects`).
#[path = "potions/cloud_particle.rs"]
mod cloud_particle;
use super::{
    Game, Screen,
    dragon::{Breath, FightPlayer, FightWho},
    item_draw::owner_at,
    remote_projectiles::cloud_client_tick,
    splash_water::thrown_block_hit,
};
use horizon_assets::item_model::DisplayContext;
use horizon_client::health::DamageLevel;
use horizon_client::{
    block_entity_render::vertex_light,
    brewing,
    effects::{EffectAction, EffectInstance, OmenTick, PlayerEffects},
    hud::HudBatch,
    hud_layout, hud_overlays, inventory_effects,
    items::ItemStack,
    mesher::Vertex,
    mobs::{Damage, DamageKind, Mob, NamedDamage, Who},
    particles::{ParticleOptions, ParticleType},
    projectiles::{Projectile, ProjectileKind},
    text::Text,
};
use horizon_core::nbt::Tag;
use horizon_core::{
    geometry::{Aabb, Axis, Direction, Vec3},
    random::{LegacyRandom, RandomSource},
};
use mod_api::{EffectApplicable, EffectTickChange, EntityRef, LivingRef, PlayerFood};

/// Per-cloud settings; natural lingering clouds keep their creation defaults.
pub(super) struct CloudSettings {
    pub(super) wait_time: i32,
    pub(super) reapplication_delay: i32,
    pub(super) radius_on_use: f32,
    pub(super) duration_on_use: i32,
    pub(super) duration_scale: f32,
}
impl CloudSettings {
    fn lingering() -> Self {
        Self {
            wait_time: LINGERING_WAIT,
            reapplication_delay: REAPPLICATION_DELAY,
            radius_on_use: -0.5,
            duration_on_use: 0,
            duration_scale: 0.25,
        }
    }
}

/// A thrown splash or lingering potion in flight.
#[derive(Clone, Debug)]
pub(super) struct ThrownPotion {
    /// Its entity id once a server's clients track it (0 until then).
    id: u32,
    stack: ItemStack,
    /// The position before the last tick, for drawing between ticks.
    previous: Vec3,
    position: Vec3,
    velocity: Vec3,
    age: u32,
    /// `Projectile.getOwner`'s display name: the player who threw it (none
    /// for a dispenser's).
    owner: Option<Text>,
    /// `Projectile.getOwner` as a target: the thrower (`Who::Player`, a
    /// guest's with `guest` set), none for a dispenser's or a server's.
    thrower: Option<Who>,
    /// On a server, the guest (by connection) who threw it.
    guest: Option<u32>,
    /// `Projectile.leftOwner`: until then it can't hit its thrower.
    left_owner: bool,
    /// `Projectile.lastDeflectedBy`: a mob deflects it once in a row.
    last_deflected_by: Option<u32>,
    /// `Entity.random`: `ProjectileDeflection.REVERSE` draws from the
    /// projectile's own source, not the level's.
    random: LegacyRandom,
}
/// The player (not one of `skip`, the thrower and who rides with them before
/// it has left them) whose box, grown by `margin` (`ProjectileUtil.computeMargin`),
/// holds `p` (`Projectile.canHitEntity`).
fn thrown_player_at<'a>(
    players: &'a [(Who, Aabb)],
    skip: &[Who],
    p: Vec3,
    margin: f64,
) -> Option<&'a (Who, Aabb)> {
    players.iter().find(|(who, b)| {
        !skip.contains(who) && b.inflate(Vec3::new(margin, margin, margin)).contains(p)
    })
}
/// `ProjectileUtil.computeMargin`: how far the pick box is grown at `age`.
fn thrown_margin(age: u32) -> f64 {
    ((f64::from(age) - 2.0) / 20.0).clamp(0.0, 0.3)
}
/// `Projectile.isOutsideOwnerCollisionRange` (`checkLeftOwner`): none of the
/// boxes of the thrower's vehicle tree (`owner.getRootVehicle().getSelfAndPassengers()`,
/// here `ridden`) is within a block of the potion's (0.25 wide) grown by its
/// motion; true too when there is no thrower (`ridden` empty).
fn thrown_left_owner(ridden: &[Aabb], position: Vec3, velocity: Vec3) -> bool {
    let h = 0.125;
    let reach = Aabb::new(
        Vec3::new(position.x - h, position.y, position.z - h),
        Vec3::new(position.x + h, position.y + 2.0 * h, position.z + h),
    )
    .expand_towards(velocity)
    .inflate(Vec3::new(1.0, 1.0, 1.0));
    !ridden.iter().any(|b| b.intersects(reach))
}
/// `AreaEffectCloud` wait time set by `ThrownLingeringPotion.onHitAsPotion`.
pub(super) const LINGERING_WAIT: i32 = 10;
/// `AreaEffectCloud.DEFAULT_REAPPLICATION_DELAY`.
const REAPPLICATION_DELAY: i32 = 20;
/// `AreaEffectCloud.readAdditionalSaveData` for the fields a client cloud keeps:
/// its position, lifetime, radius and use settings, and the potion it carries.
/// The custom particle and the owner reference are not modelled on a client
/// cloud, and the entity id is the caller's (`load_potion_cloud`).
pub(super) fn potion_cloud_from_nbt(tag: &Tag) -> Option<(Vec3, Breath)> {
    use horizon_client::save::{float, number, text};
    let at = horizon_client::entity_nbt::position(tag)?;
    let motion = horizon_client::entity_nbt::loaded_motion(tag);
    let int = |key, default| number(tag.get(key)).map_or(default, |v| v as i32);
    let real = |key, default| float(tag.get(key)).map_or(default, |v| v as f32);
    let contents = tag.get("potion_contents");
    let mut effects = contents
        .and_then(|c| text(c.get("potion")))
        .and_then(|id| brewing::potion(&id))
        .map_or_else(Vec::new, |p| p.effects.to_vec());
    let (mut hidden, mut ambient) = (Vec::new(), Vec::new());
    if let Some(Tag::List(custom)) = contents.and_then(|c| c.get("custom_effects")) {
        for effect in custom {
            let Some(def) = text(effect.get("id")).and_then(|id| brewing::effect(&id)) else {
                continue;
            };
            let duration = number(effect.get("duration")).unwrap_or(0) as i32;
            let amp = number(effect.get("amplifier")).unwrap_or(0).clamp(0, 255) as i32;
            effects.push((def.id, duration, amp));
            if number(effect.get("show_particles")) == Some(0) {
                hidden.push(def.id);
            }
            if number(effect.get("ambient")).unwrap_or(0) != 0 {
                ambient.push(def.id);
            }
        }
    }
    let color = contents
        .and_then(|c| number(c.get("custom_color")))
        .map_or_else(
            || {
                horizon_client_core::potions::effects_color(
                    effects
                        .iter()
                        .filter(|(id, ..)| !hidden.contains(id))
                        .map(|(id, _, amp)| (*id, *amp)),
                )
            },
            |color| color as u32,
        );
    Some((
        at,
        Breath {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id: 0,
            at: [at.x, at.y, at.z],
            motion: [motion.x, motion.y, motion.z],
            radius: real("Radius", 3.0).clamp(0.0, 32.0),
            age: int("Age", 0),
            duration: int("Duration", -1),
            from_dragon: false,
            victims: Vec::new(),
            potion: Some(PotionCloud {
                settings: CloudSettings {
                    wait_time: int("WaitTime", 20),
                    reapplication_delay: int("ReapplicationDelay", 20),
                    radius_on_use: real("RadiusOnUse", 0.0),
                    duration_on_use: int("DurationOnUse", 0),
                    duration_scale: real("potion_duration_scale", 1.0),
                },
                effects,
                hidden,
                ambient,
                radius_per_tick: real("RadiusPerTick", 0.0),
                color,
                custom_particle: tag.get("custom_particle").cloned(),
                victims: Vec::new(),
                source: Text::translate("entity.minecraft.area_effect_cloud", &[]),
                owner: None,
                guest: None,
            }),
        },
    ))
}

/// `AreaEffectCloud.addAdditionalSaveData`: the lifetime, radius and use
/// settings with the effects the cloud applies, as `custom_effects` with the
/// colour they were drawn in. The client keeps no potion id, so a cloud that
/// came from a named potion is written effect by effect; the custom particle
/// and the owner have no counterpart here.
pub(super) fn potion_cloud_to_nbt(c: &Breath, uuid: [i32; 4], raw: Option<&Tag>) -> Tag {
    use horizon_core::nbt::Compound;
    let Some(potion) = &c.potion else {
        return Tag::End;
    };
    let at = Vec3::new(c.at[0], c.at[1], c.at[2]);
    let motion = Vec3::new(c.motion[0], c.motion[1], c.motion[2]);
    let mut tag = horizon_client::entity_nbt::entity_base_tag(
        "minecraft:area_effect_cloud",
        at,
        motion,
        [0.0, 0.0],
        uuid,
        raw,
    );
    let custom: Vec<Tag> = potion
        .effects
        .iter()
        .map(|(id, duration, amplifier)| {
            let mut effect = Compound::new();
            effect.insert("id".into(), Tag::String(brewing::namespaced(id).into()));
            effect.insert("duration".into(), Tag::Int(*duration));
            effect.insert("amplifier".into(), Tag::Int(*amplifier));
            if potion.hidden.contains(id) {
                effect.insert("show_particles".into(), Tag::Byte(0));
            }
            if potion.ambient.contains(id) {
                effect.insert("ambient".into(), Tag::Byte(1));
            }
            Tag::Compound(effect)
        })
        .collect();
    let mut contents = Compound::new();
    contents.insert("custom_effects".into(), Tag::List(custom));
    contents.insert("custom_color".into(), Tag::Int(potion.color as i32));
    let Tag::Compound(map) = &mut tag else {
        return tag;
    };
    map.insert("Age".into(), Tag::Int(c.age));
    map.insert("Duration".into(), Tag::Int(c.duration));
    map.insert("Radius".into(), Tag::Float(c.radius.to_bits()));
    map.insert(
        "RadiusPerTick".into(),
        Tag::Float(potion.radius_per_tick.to_bits()),
    );
    if let Some(particle) = &potion.custom_particle {
        map.insert("custom_particle".into(), particle.clone());
    }
    map.insert("WaitTime".into(), Tag::Int(potion.settings.wait_time));
    map.insert(
        "ReapplicationDelay".into(),
        Tag::Int(potion.settings.reapplication_delay),
    );
    map.insert(
        "RadiusOnUse".into(),
        Tag::Float(potion.settings.radius_on_use.to_bits()),
    );
    map.insert(
        "DurationOnUse".into(),
        Tag::Int(potion.settings.duration_on_use),
    );
    map.insert(
        "potion_duration_scale".into(),
        Tag::Float(potion.settings.duration_scale.to_bits()),
    );
    // Vanilla writes the key only while the contents differ from
    // PotionContents.EMPTY: no effects and the default water colour.
    if !(potion.effects.is_empty()
        && potion.color == horizon_client_core::potions::BASE_POTION_COLOR)
    {
        map.insert("potion_contents".into(), Tag::Compound(contents));
    }
    tag
}

/// An entity a cloud has affected (`AreaEffectCloud.victims`).
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Victim {
    Mob(u32),
    Player(FightWho),
}
/// A potion-bearing `AreaEffectCloud`: its effects, settings and recent victims.
pub(super) struct PotionCloud {
    pub(super) settings: CloudSettings,
    effects: Vec<(&'static str, i32, i32)>,
    /// The effects above that were added hidden (`MobEffectInstance.isVisible` false: no
    /// particles); the instances the cloud applies are copies and keep that.
    hidden: Vec<&'static str>,
    /// The effects above that were ambient (`MobEffectInstance.isAmbient`).
    ambient: Vec<&'static str>,
    /// `AreaEffectCloud.radiusPerTick`: natural clouds use `-radius / duration`.
    pub(super) radius_per_tick: f32,
    /// The potion's colour (`PotionContents.getColor`).
    color: u32,
    /// `AreaEffectCloud.customParticle`: the `custom_particle` compound as saved.
    custom_particle: Option<Tag>,
    /// Each victim and the tick it can be affected again.
    victims: Vec<(Victim, i32)>,
    /// Who its instant damage names (`indirectMagic(cloud, owner)`): the
    /// thrower, else the cloud.
    source: Text,
    /// `AreaEffectCloud.getOwner`: the thrower (`ThrownLingeringPotion.onHitAsPotion`), whose
    /// kill a mob's death to its instant damage is.
    owner: Option<Who>,
    /// On a server, the guest (by connection) who threw it.
    guest: Option<u32>,
}
impl PotionCloud {
    /// `AreaEffectCloud.updateParticle`: the custom particle when one is set,
    /// else an entity effect in the potion's colour.
    pub(super) fn particle(&self) -> ParticleOptions {
        self.custom_particle
            .as_ref()
            .and_then(cloud_particle::custom_particle_options)
            .unwrap_or_else(|| {
                let c = |shift: u32| (self.color >> shift & 255) as f32 / 255.0;
                ParticleOptions::colored(ParticleType::EntityEffect, [c(16), c(8), c(0), 1.0])
            })
    }
}
/// `Projectile.getOwner` of a thrown potion: what its `indirectMagic` damage
/// is named after and who it counts against. Nothing for a dispenser's or a
/// summoned potion nobody owns, the potion itself then naming the death
/// message.
#[derive(Clone, Debug, Default)]
pub(super) struct PotionOwner {
    /// The thrower's name, when it has one.
    name: Option<Text>,
    /// The mob that threw it (a witch): the hurt is its kill credit, as
    /// `indirectMagic(potion, owner)` gives it. A player's throw is the
    /// potion's own and only `name`s the message.
    attacker: Option<u32>,
    /// A player threw it: the water damage `indirectMagic` deals is theirs.
    player: bool,
    /// On a server, the guest (by connection) who threw it: whose kill the mobs it hurts are
    /// (`LivingEntity.lastHurtByPlayer`).
    guest: Option<u32>,
}
impl PotionOwner {
    /// The owner as a damage source's causing entity.
    fn who(&self) -> Option<Who> {
        match self.attacker {
            Some(id) => Some(Who::Mob(id)),
            None => self.player.then_some(Who::Player),
        }
    }
}
/// A mob's effect ids carry the `minecraft:` namespace; a mod's has its own.
pub(super) fn mob_effect_id(id: &str) -> Option<&'static str> {
    brewing::effect(id).map(|e| brewing::namespaced(e.id))
}
/// `LivingEntity.canBeAffected` with the mob overrides (`WitherBoss` and
/// `WitherSkeleton` shrug off wither, `Spider` and `AbstractNautilus` poison,
/// `Parched` weakness): the immune-to-infested and immune-to-oozing tags
/// decide first, then the tag of mobs that ignore poison and regeneration.
/// `WitherBoss.addEffect` refusing everything is `Mob::add_effect`'s: a cloud
/// still takes the wither as a victim (`AreaEffectCloud` asks `canBeAffected`),
/// so its instant effects land and it shrinks.
pub(super) fn mob_can_be_affected(
    tags: &std::collections::HashMap<String, std::collections::HashSet<String>>,
    kind: &str,
    effect: &str,
) -> bool {
    let effect = effect.trim_start_matches("minecraft:");
    let kind = kind.trim_start_matches("minecraft:");
    let overridden = match kind {
        "wither" | "wither_skeleton" => effect == "wither",
        "spider" | "cave_spider" | "nautilus" | "zombie_nautilus" => effect == "poison",
        "parched" => effect == "weakness",
        _ => false,
    };
    if overridden {
        return false;
    }
    let in_tag = |tag: &str| {
        tags.get(tag)
            .is_some_and(|t| t.contains(&format!("minecraft:{kind}")))
    };
    if in_tag("minecraft:immune_to_infested") {
        effect != "infested"
    } else if in_tag("minecraft:immune_to_oozing") {
        effect != "oozing"
    } else {
        !in_tag("minecraft:ignores_poison_and_regen")
            || (effect != "regeneration" && effect != "poison")
    }
}
impl Game<'_> {
    /// The dragon's breath (`AreaEffectCloud` with instant damage at
    /// `amplifier`) on a mob: what a cloud puts on it, `indirectMagic(cloud,
    /// dragon)` with no owner to credit (the dragon is not a mob here).
    pub(super) fn breath_affect_mob(&mut self, mob: u32, amplifier: i32, at: [f64; 3]) {
        let source = Text::translate("entity.minecraft.area_effect_cloud", &[]);
        let effect = [("instant_damage", 1, amplifier)];
        // DragonFireball.onHit and the flame phase: `setPotionDurationScale(0.25)`.
        self.cloud_affect(
            Victim::Mob(mob),
            &effect,
            (&[], &[], 0.25),
            at,
            &source,
            (None, None),
        );
    }
}
/// `ThrownSplashPotion.onHitAsPotion`: the squared distance from the potion's
/// box at the hit point to an entity's box grown by the hit margin
/// (`ProjectileUtil.computeMargin`), for entities that meet the potion's box
/// grown by (4, 2, 4); none when it is 16 or more.
fn splash_distance_sqr(at: Vec3, age: u32, bounds: Aabb) -> Option<f64> {
    let potion = Aabb::new(
        Vec3::new(at.x - 0.125, at.y, at.z - 0.125),
        Vec3::new(at.x + 0.125, at.y + 0.25, at.z + 0.125),
    );
    if !potion.inflate(Vec3::new(4.0, 2.0, 4.0)).intersects(bounds) {
        return None;
    }
    let margin = ((f64::from(age) - 2.0) / 20.0).clamp(0.0, 0.3);
    let d2 = potion.distance_squared_to_box(bounds.inflate(Vec3::new(margin, margin, margin)));
    (d2 < 16.0).then_some(d2)
}
impl Game<'_> {
    /// `LivingEntity.triggerOnDeathMobEffects` (`onMobRemoved` with the reason
    /// KILLED) for what Oozing, Weaving and Wind Charged do when their bearer
    /// dies. `bearer` is the dying mob's id, `player` whether it is a player,
    /// `random` the dying mob's own (`mob.getRandom()`; the world's for the
    /// player).
    pub(super) fn effects_on_death(
        &mut self,
        effects: &[&str],
        bounds: Aabb,
        player: bool,
        bearer: Option<u32>,
        mut random: Option<&mut LegacyRandom>,
    ) {
        let feet = bounds.bottom_center();
        if effects.contains(&"oozing") {
            // OozingMobEffect.numberOfSlimesToSpawn: two, fewer when the
            // slimes close by already fill the cramming limit.
            let limit = self
                .ui
                .rules
                .int_at(horizon_client::gamerules::vanilla::MAX_ENTITY_CRAMMING);
            let mut count = 2;
            if limit >= 1 {
                let near = bounds.inflate(Vec3::new(2.0, 2.0, 2.0));
                let nearby = self
                    .mobs
                    .iter()
                    .filter(|m| {
                        m.species() == horizon_client::mobs::Species::Slime
                            && Some(m.id) != bearer
                            && m.bounds().intersects(near)
                    })
                    .count()
                    .min(limit as usize) as i32;
                count = (limit - nearby).clamp(0, count);
            }
            for _ in 0..count {
                // spawnSlimeOffspring: a size 2 slime half a block up.
                if let Some(id) = self.spawn_mob(
                    "minecraft:slime",
                    [feet.x, feet.y + 0.5, feet.z],
                    false,
                    super::creatures::SpawnSite::Plain,
                ) && let Some(m) = self.mobs.iter_mut().find(|m| m.id == id)
                {
                    horizon_client::mobs::cube::set_size(m, 2);
                }
            }
        }
        // `WeavingMobEffect.onMobRemoved`: `canEntityGrief` for the dying mob.
        if effects.contains(&"weaving") && (player || self.can_entity_grief(bearer)) {
            self.spawn_cobwebs_around(
                [
                    feet.x.floor() as i32,
                    feet.y.floor() as i32,
                    feet.z.floor() as i32,
                ],
                random.as_deref_mut(),
            );
        }
        if effects.contains(&"wind_charged") {
            // A burst centred at half the height, radius 3 to 5.
            let center = [feet.x, feet.y + bounds.size().y / 2.0, feet.z];
            let r = random.unwrap_or(&mut self.world.random);
            let radius = 3.0 + r.next_float() * 2.0;
            // WindChargedMobEffect: the bearer bursts, a living source.
            let bearer = if player {
                Some(self.own_entity_id() as u32)
            } else {
                bearer
            };
            let cause = super::wind::BurstCause {
                source: bearer,
                owner: bearer,
            };
            let _ = self.wind_burst(center, radius, 1.0, false, false, cause);
        }
    }
    /// `WeavingMobEffect.spawnCobwebsRandomlyAround`: two or three cobwebs in
    /// free spaces with a solid floor, picked among 15 random blocks within one
    /// block of `pos`.
    fn spawn_cobwebs_around(&mut self, pos: [i32; 3], mut random: Option<&mut LegacyRandom>) {
        let r = random.as_deref_mut().unwrap_or(&mut self.world.random);
        let wanted = 2 + r.next_int_bound(2).unwrap_or(0) as usize;
        let mut chosen: Vec<[i32; 3]> = Vec::new();
        for _ in 0..15 {
            let r = random.as_deref_mut().unwrap_or(&mut self.world.random);
            let at = [
                pos[0] - 1 + r.next_int_bound(3).unwrap_or(0),
                pos[1] - 1 + r.next_int_bound(3).unwrap_or(0),
                pos[2] - 1 + r.next_int_bound(3).unwrap_or(0),
            ];
            let facts = self.res.facts();
            let here = facts.get(self.world.block(at[0], at[1], at[2]));
            let below = facts.get(self.world.block(at[0], at[1] - 1, at[2]));
            if !chosen.contains(&at)
                && here.replaceable
                && below.sturdy_full & (1 << Direction::Up as u8) != 0
            {
                chosen.push(at);
                if chosen.len() >= wanted {
                    break;
                }
            }
        }
        for at in chosen {
            if self.set_state_named(at, "minecraft:cobweb", &[]).is_ok() {
                self.level_event_at(3018, at, 0);
            }
        }
    }
    /// `InfestedMobEffect.onMobHurt` for the player: from half its height up,
    /// along where it looks.
    pub(super) fn player_infested(&mut self) {
        if let Some(p) = self.player.as_ref() {
            let at = [
                p.position.x,
                p.position.y + f64::from(p.dimensions.height) / 2.0,
                p.position.z,
            ];
            let look = super::view_vector(self.yaw, self.pitch);
            // The player's draws come from the world's random.
            let velocities =
                horizon_client::mobs::infested_silverfish(&mut self.world.random, look);
            self.spawn_infested(at, velocities);
        }
    }
    /// `InfestedMobEffect.spawnSilverfish`: a silverfish at `at` (half the
    /// bearer's height up) for each of the velocities the bearer's random drew
    /// (`horizon_client::mobs::infested_silverfish`).
    pub(super) fn spawn_infested(&mut self, at: [f64; 3], velocities: Vec<Vec3>) {
        for velocity in velocities {
            if let Some(id) = self.spawn_mob(
                "minecraft:silverfish",
                at,
                false,
                super::creatures::SpawnSite::Plain,
            ) && let Some(m) = self.mobs.iter_mut().find(|m| m.id == id)
            {
                m.velocity = velocity;
            }
            self.audio
                .play("minecraft:entity.silverfish.hurt", Some(at), 1.0, 1.0);
        }
    }
}
/// A splash potion, lingering potion or experience bottle about to fly (the
/// player's throws and dispensers both make one): `ThrownSplashPotion`,
/// `ThrownLingeringPotion` or `ThrownExperienceBottle`, which
/// `level.addFreshEntity` makes a level entity like a summoned one, so
/// selectors and `/data` find it (#13201).
pub(super) fn thrown_projectile(stack: ItemStack, position: Vec3, by_player: bool) -> Projectile {
    let kind = match &*stack.item {
        "minecraft:experience_bottle" => ProjectileKind::ExperienceBottle,
        item => {
            let lingering = item == "minecraft:lingering_potion";
            ProjectileKind::Potion(Box::new(stack), lingering)
        }
    };
    Projectile::new(kind, position, by_player.then_some(Who::Player))
}
/// What a player's body was last given for its effects and enchantments:
/// a guest keeps their own while the host's are in `PotionState`.
#[derive(Default)]
pub(super) struct BodyPotions {
    pub(super) applied: Vec<(&'static str, &'static str, f64, &'static str)>,
    protection_for: Option<(u64, Vec<Option<ItemStack>>)>,
    enchant_modifiers: Vec<(String, String, f64, String)>,
}
/// Effects, thrown potions and the attribute modifiers last applied.
#[derive(Default)]
pub(super) struct PotionState {
    pub(super) effects: PlayerEffects,
    pub(super) thrown: Vec<ThrownPotion>,
    /// The area effect clouds a server tracks (game/network/remote_projectiles.rs).
    pub(super) clouds: Vec<super::remote_projectiles::RemoteCloud>,
    pub(super) applied: Vec<(&'static str, &'static str, f64, &'static str)>,
    /// Armor the protection table was computed for.
    protection_for: Option<(u64, Vec<Option<ItemStack>>)>,

    /// Enchantment attribute modifiers on the player: (attribute, id, amount, operation).
    pub(super) enchant_modifiers: Vec<(String, String, f64, String)>,
}

impl PotionState {
    /// Trade the player's own state (effects and what their body was given)
    /// with a guest's, around `as_guest`. Potions in flight stay.
    pub(super) fn swap_player(&mut self, effects: &mut PlayerEffects, body: &mut BodyPotions) {
        std::mem::swap(&mut self.effects, effects);
        std::mem::swap(&mut self.applied, &mut body.applied);
        std::mem::swap(&mut self.protection_for, &mut body.protection_for);
        std::mem::swap(&mut self.enchant_modifiers, &mut body.enchant_modifiers);
    }
    /// Thrown potions in flight as (entity type, position, motion), for the
    /// dispenser parity fixture.
    pub(super) fn thrown_entities(&self) -> Vec<(&'static str, Vec3, Vec3)> {
        self.thrown
            .iter()
            .map(|t| (thrown_kind(&t.stack), t.position, t.velocity))
            .collect()
    }
    pub(super) fn clear_thrown(&mut self) {
        self.thrown.clear();
    }
    /// Their entity ids, for a server to number them.
    pub(super) fn thrown_ids(&mut self) -> impl Iterator<Item = &mut u32> {
        self.thrown.iter_mut().map(|t| &mut t.id)
    }
    /// The numbered ones as a server's clients track them: (id, entity
    /// type, item, position, motion).
    pub(super) fn thrown_tracked(
        &self,
    ) -> impl Iterator<Item = (u32, &'static str, &ItemStack, Vec3, Vec3)> {
        self.thrown.iter().filter(|t| t.id != 0).map(|t| {
            (
                t.id,
                thrown_kind(&t.stack),
                &t.stack,
                t.position,
                t.velocity,
            )
        })
    }
    /// `handleAddEntity` for a thrown potion or bottle on a server's client:
    /// drawn where the server says (its item comes in its entity data).
    pub(super) fn add_remote_thrown(&mut self, id: u32, kind: &str, at: Vec3, velocity: Vec3) {
        self.thrown.retain(|t| t.id != id);
        let item = match kind {
            "minecraft:lingering_potion" | "minecraft:experience_bottle" => kind,
            _ => "minecraft:splash_potion",
        };
        self.thrown.push(ThrownPotion {
            id,
            stack: ItemStack::new(item, 1),
            previous: at,
            position: at,
            velocity,
            age: 0,
            owner: None,
            thrower: None,
            guest: None,
            left_owner: false,
            last_deflected_by: None,
            random: LegacyRandom::new(i64::from(id)),
        });
    }
    /// ThrowableItemProjectile.DATA_ITEM_STACK: the potion it shows.
    pub(super) fn remote_thrown_item(&mut self, id: u32, stack: ItemStack) {
        if let Some(t) = self.thrown.iter_mut().find(|t| t.id == id) {
            t.stack = stack;
        }
    }
    /// A server's move: thrown potions have no interpolation handler, so
    /// `moveOrInterpolateTo` puts them there at once. False when `id` isn't one.
    pub(super) fn move_remote_thrown(&mut self, id: u32, at: Vec3) -> bool {
        let Some(t) = self.thrown.iter_mut().find(|t| t.id == id) else {
            return false;
        };
        t.position = at;
        true
    }
    /// `handleSetEntityMotion` for a thrown potion. False when `id` isn't one.
    pub(super) fn remote_thrown_motion(&mut self, id: u32, velocity: Vec3) -> bool {
        let Some(t) = self.thrown.iter_mut().find(|t| t.id == id) else {
            return false;
        };
        t.velocity = velocity;
        true
    }
    pub(super) fn remove_remote_thrown(&mut self, id: u32) {
        self.thrown.retain(|t| t.id != id);
    }
    pub(super) fn has_thrown(&self, id: u32) -> bool {
        self.thrown.iter().any(|t| t.id == id)
    }
}

/// The entity type of a thrown potion or bottle.
fn thrown_kind(stack: &ItemStack) -> &'static str {
    match &*stack.item {
        "minecraft:lingering_potion" => "minecraft:lingering_potion",
        "minecraft:experience_bottle" => "minecraft:experience_bottle",
        _ => "minecraft:splash_potion",
    }
}

/// `BottleItem.use`: find a living `AreaEffectCloud` owned by an `EnderDragon`
/// within the player's box grown by 2, and shrink its radius by 0.5. Gives
/// the cloud's position when one was found.
pub(super) fn collect_dragon_breath(
    player_box: Aabb,
    clouds: &mut horizon_client::join_seq::ResidentList<super::dragon::Breath>,
) -> Option<[f64; 3]> {
    let search_box = player_box.inflate(Vec3::new(2.0, 2.0, 2.0));
    if let Some(i) = clouds.iter().position(|c| {
        if !c.from_dragon || c.potion.is_some() || super::dragon::breath_expired(c.age, c.duration)
        {
            return false;
        }
        let r = f64::from(c.radius);
        let cloud_box = Aabb::new(
            Vec3::new(c.at[0] - r, c.at[1], c.at[2] - r),
            Vec3::new(c.at[0] + r, c.at[1] + 0.5, c.at[2] + r),
        );
        search_box.intersects(cloud_box)
    }) {
        let at = clouds[i].at;
        clouds.update(i, |c| c.radius = (c.radius - 0.5).max(0.0));
        Some(at)
    } else {
        None
    }
}

impl Game<'_> {
    /// Right-click with a potion, splash/lingering potion or glass bottle.
    pub(super) fn use_potion_item(&mut self, stack: &ItemStack) -> horizon_client::Result<bool> {
        match &*stack.item {
            "minecraft:potion" | "minecraft:ominous_bottle" => {
                self.start_using_item();
                Ok(true)
            }
            "minecraft:splash_potion"
            | "minecraft:lingering_potion"
            | "minecraft:experience_bottle" => {
                let Some(eye) = self.eye_position() else {
                    return Ok(true);
                };
                let pitch = 0.4 / (self.world.random.next_float() * 0.4 + 0.8);
                let (sound, power) = match &*stack.item {
                    "minecraft:splash_potion" => ("minecraft:entity.splash_potion.throw", 0.5),
                    "minecraft:experience_bottle" => {
                        ("minecraft:entity.experience_bottle.throw", 0.7)
                    }
                    _ => ("minecraft:entity.lingering_potion.throw", 0.5),
                };
                self.audio.play(sound, Some(eye), 0.5, pitch);
                // Projectile.shootFromRotation(xRot, yRot, -20, power, 1.0), with
                // what the thrower last moved (getKnownMovement).
                let velocity = self.known_player_movement();
                let on_ground = self.player.as_ref().is_some_and(|p| p.ground.on_ground);
                let one = ItemStack {
                    count: 1,
                    ..stack.clone()
                };
                let mut shot =
                    thrown_projectile(one, Vec3::new(eye[0], eye[1] - 0.1, eye[2]), true);
                shot.shoot_from_rotation(
                    self.pitch, self.yaw, -20.0, power, 1.0, velocity, on_ground,
                );
                shot.guest = self.lan_acting;
                self.add_projectile(shot);
                // ThrowablePotionItem.use / ExperienceBottleItem.use: awardStat(ITEM_USED).
                self.award_stat("used", &stack.item, 1);
                self.swing_arm();
                self.consume_held();
                Ok(true)
            }
            "minecraft:glass_bottle" => {
                // `BottleItem.use`: before the water check, it looks for a living
                // AreaEffectCloud owned by an EnderDragon within the player's box
                // grown by 2. If one is found, it shrinks the cloud's radius by 0.5,
                // plays item.bottle.fill_dragonbreath at the player, emits
                // GameEvent.FLUID_PICKUP, fires PLAYER_INTERACTED_WITH_ENTITY on the
                // cloud, and turns one bottle into dragon_breath.
                if let Some(player) = self.player.as_ref()
                    && let Some(cloud_at) =
                        collect_dragon_breath(player.bounds(), &mut self.breath_clouds)
                {
                    let player_pos = [player.position.x, player.position.y, player.position.z];
                    self.audio.play(
                        "minecraft:item.bottle.fill_dragonbreath",
                        Some(player_pos),
                        1.0,
                        1.0,
                    );
                    self.post_entity_event("minecraft:fluid_pickup", Who::Player);
                    let cloud = horizon_client::advancements::EntityView {
                        kind: "minecraft:area_effect_cloud".into(),
                        position: cloud_at,
                        ..Default::default()
                    };
                    self.trigger_item_entity(
                        "minecraft:player_interacted_with_entity",
                        stack,
                        &cloud,
                    );
                    // BottleItem.turnBottleIntoItem.
                    self.award_stat("used", &stack.item, 1);
                    self.fill_held(ItemStack::new("minecraft:dragon_breath", 1));
                    return Ok(true);
                }
                let Some(hit) = self.pick_fluid() else {
                    return Ok(false);
                };
                let state = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
                if self.res.facts().get(state).fluid != horizon_client::blocks::Fluid::Water {
                    return Ok(false);
                }
                // The sound at the player, the game event at the water block.
                let Some(at) = self
                    .player
                    .as_ref()
                    .map(|p| [p.position.x, p.position.y, p.position.z])
                else {
                    return Ok(false);
                };
                self.audio
                    .play("minecraft:item.bottle.fill", Some(at), 1.0, 1.0);
                self.post_block_game_event(
                    "minecraft:fluid_pickup",
                    hit.pos,
                    Some(Who::Player),
                    None,
                );
                // BottleItem.turnBottleIntoItem.
                self.award_stat("used", &stack.item, 1);
                let water = ItemStack::new("minecraft:potion", 1).with_potion("minecraft:water");
                self.fill_held(water);
                Ok(true)
            }
            _ => Ok(false),
        }
    }
    /// `PotionItem.useOn`: a water bottle used on a block that turns to mud
    /// (`#convertible_to_mud`), not on its underside, is poured out and the
    /// block becomes mud.
    pub(super) fn use_water_bottle_on_block(
        &mut self,
        pos: [i32; 3],
        face: [i32; 3],
        stack: &ItemStack,
    ) -> horizon_client::Result<bool> {
        if &*stack.item != "minecraft:potion"
            || stack.potion() != Some("minecraft:water")
            || face == [0, -1, 0]
            || !self.named_block_in_tag("minecraft:convertible_to_mud", &self.block_name(pos))
        {
            return Ok(false);
        }
        let centre = pos.map(|v| f64::from(v) + 0.5);
        self.audio
            .play("minecraft:entity.generic.splash", Some(centre), 1.0, 1.0);
        self.fill_held(ItemStack::new("minecraft:glass_bottle", 1));
        // PotionItem.useOn: five ServerLevel.sendParticles(SPLASH). The level
        // random places each one (count 1, no spread, speed 1). The packet is
        // what the window draws; a window that runs this itself still adds it.
        for _ in 0..5 {
            let r = &mut self.world.random;
            let at = [
                f64::from(pos[0]) + r.next_double(),
                f64::from(pos[1] + 1),
                f64::from(pos[2]) + r.next_double(),
            ];
            let options = ParticleOptions::simple(ParticleType::Splash);
            self.particles.add(&options, at, [0.0; 3]);
            self.lan_send_particles(super::lan_particles::QueuedParticle {
                dimension: self.dim.id(),
                options,
                state: None,
                at,
                spread: [0.0; 3],
                speed: [1.0; 3],
                count: 1,
                randomization: 0,
            });
        }
        self.audio
            .play("minecraft:item.bottle.empty", Some(centre), 1.0, 1.0);
        self.post_block_game_event("minecraft:fluid_place", pos, None, None);
        if let Some(mud) = self.pack.assets.registry.default_state("minecraft:mud") {
            self.level_set(pos, mud)?;
        }
        self.award_stat("used", &stack.item, 1);
        self.swing_arm();
        Ok(true)
    }
    /// `ItemUtils.createFilledResult`: replace one of the held stack.
    fn fill_held(&mut self, filled: ItemStack) {
        if self.creative {
            let _ = self.add_stack_to_inventory(filled);
            return;
        }
        self.consume_held();
        if self.held().is_none() {
            *self.held_mut() = Some(filled);
        } else if let Some(rest) = self.add_stack_to_inventory(filled)
            && let Some(eye) = self.eye_position()
        {
            // `player.drop(stack, false)`.
            self.throw_from_eyes(rest, eye, false, false);
        }
    }
    /// Drinking a potion while right-click is held; true when handled.
    pub(super) fn tick_drinking(&mut self, slot: usize, ticks: u32, stack: &ItemStack) -> bool {
        let ominous = &*stack.item == "minecraft:ominous_bottle";
        if &*stack.item != "minecraft:potion" && !ominous {
            return false;
        }
        // the duration the use started with (32, as a mod's Start or Tick left it).
        let duration = self.use_duration;
        let ticks = ticks + 1;
        let remaining = duration.saturating_sub_unsigned(ticks).max(0);
        let eye = self.eye_position().unwrap_or_default();
        // Consumable.emitParticlesAndSounds: the drink sound every 4 ticks near the end.
        if remaining <= 25 && remaining % 4 == 0 {
            let pitch = self.world.random.next_float() * 0.1 + 0.9;
            self.audio
                .play("minecraft:entity.generic.drink", Some(eye), 0.5, pitch);
        }
        // ItemStack.onUseTick: the item's own tick after the consumable's.
        if !self.item_on_use_tick(stack, ticks - 1) {
            return true;
        }
        if i64::from(ticks) < i64::from(duration) {
            self.using = Some((slot, ticks));
            return true;
        }
        self.finish_drinking(slot, stack);
        self.use_item_finished(stack);
        true
    }
    /// `PotionItem.finishUsingItem` (and `OminousBottleItem`'s) of `stack` in
    /// `slot`, the use complete: false for anything else.
    pub(super) fn finish_drinking(&mut self, slot: usize, stack: &ItemStack) -> bool {
        let ominous = &*stack.item == "minecraft:ominous_bottle";
        if &*stack.item != "minecraft:potion" && !ominous {
            return false;
        }
        let eye = self.eye_position().unwrap_or_default();
        self.using = None;
        self.send_use_item_complete();
        // Consumable.onConsume: the used statistic and the consume_item
        // trigger, with the stack as drunk (before it shrinks).
        self.award_stat("used", &stack.item, 1);
        self.trigger_item("minecraft:consume_item", stack);
        self.post_entity_event("minecraft:drink", horizon_client::mobs::Who::Player);
        if ominous {
            self.drink_ominous_bottle(slot, stack);
            return true;
        }
        // PotionContents.onConsume: the potion's and the custom effects, at the
        // stack's POTION_DURATION_SCALE.
        let (lasting, now) = PlayerEffects::contents_effects(
            &brewing::contents_effects(stack),
            stack.potion_duration_scale(),
        );
        for e in lasting {
            self.add_player_effect(e);
        }
        for a in now {
            self.apply_effect_action(a);
        }
        if !self.creative {
            if let Some(s) = self.stack_in_mut(slot).as_mut() {
                s.count -= 1;
                if s.count == 0 {
                    *self.stack_in_mut(slot) = None;
                }
            }
            let bottle = ItemStack::new("minecraft:glass_bottle", 1);
            if self.stack_in(slot).is_none() {
                *self.stack_in_mut(slot) = Some(bottle);
            } else if let Some(rest) = self.add_stack_to_inventory(bottle) {
                self.spawn_item(rest, eye, [0.0, 0.1, 0.0], 40);
            }
        }
        true
    }
    /// `Arrow.doPostHurtEffects` and `SpectralArrow.doPostHurtEffects`: the
    /// arrow's item gives its target Glowing (spectral) or its potion contents
    /// at the stack's duration scale (a tipped arrow's default is an eighth);
    /// a mob's arrow without such an item (a stray's) carries its own effect.
    /// Instant effects act at once: harming hurts and healing heals, the
    /// other way round for the undead.
    pub(super) fn arrow_post_hurt_effects(&mut self, p: &Projectile, who: Who) {
        let from_item = p.arrow.as_ref().map(|a| &a.pickup_item);
        let mut effects: Vec<(&'static str, i32, i32)> = match from_item {
            // SpectralArrow.doPostHurtEffects: Glowing for the arrow's own
            // (saved) Duration.
            Some(s) if &*s.item == "minecraft:spectral_arrow" => {
                let ticks = p.arrow.as_ref().map_or(200, |a| a.spectral_duration);
                vec![("glowing", ticks, 0)]
            }
            Some(s) => {
                let scale = s.potion_duration_scale();
                brewing::contents_effects(s)
                    .into_iter()
                    .map(|(id, d, amp)| {
                        (id, horizon_client::effects::scaled_duration(d, scale), amp)
                    })
                    .collect()
            }
            None => Vec::new(),
        };
        if effects.is_empty()
            && let ProjectileKind::Arrow {
                effect: Some((id, ticks)),
                ..
            } = p.kind
            && let Some(e) = brewing::effect(id)
        {
            effects.push((e.id, ticks, 0));
        }
        if effects.is_empty() {
            return;
        }
        match who {
            Who::Player => {
                for (id, duration, amp) in effects {
                    if brewing::effect(id).is_some_and(|e| e.instant) {
                        if let Some(a) = horizon_client::effects::instant(id, amp, 1.0) {
                            self.apply_effect_action(a);
                        }
                    } else {
                        self.add_player_effect(EffectInstance::new(id, duration, amp));
                    }
                }
            }
            Who::Mob(id) => {
                let Some(i) = self.mobs.iter().position(|m| m.id == id && m.alive()) else {
                    return;
                };
                let inverted = self
                    .res
                    .stations()
                    .enchantments
                    .entity_tags
                    .get("minecraft:inverted_healing_and_harm")
                    .is_some_and(|t| t.contains(&self.mobs[i].kind.id));
                for (effect, duration, amp) in effects {
                    // A mod's `onEffectAdded` may have changed the mobs.
                    let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) else {
                        return;
                    };
                    match effect {
                        "instant_damage" | "instant_health" => {
                            // `LivingEntity.addEffect` refuses nothing here; the
                            // tick hurts with magic (6 << amp) or heals (4 << amp).
                            if (effect == "instant_damage") != inverted {
                                m.damage(
                                    crate::game::entity::damage_level_of!(self, scoped),
                                    (6 << amp.min(30)) as f32,
                                    Damage::of(DamageKind::Magic),
                                );
                            } else {
                                m.heal(&self.loop_context, (4 << amp.min(30)) as f32);
                            }
                        }
                        _ => {
                            // `LivingEntity.addEffect` refuses what the mob
                            // cannot be affected by: the borrow ends before
                            // the hook asks, and the mob is found again.
                            let kind = m.kind.id.clone();
                            if self.can_be_affected(Some(id as i32), &kind, effect)
                                && let Some(name) = mob_effect_id(effect)
                            {
                                let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) else {
                                    return;
                                };
                                let new = !m.has_effect(name);
                                m.add_effect(name, duration, amp);
                                self.mod_effect_added(name, Some(id as i32), amp, new);
                            }
                        }
                    }
                }
            }
        }
    }
    pub(super) fn apply_effect_action(&mut self, action: EffectAction) {
        let action = match action {
            EffectAction::Heal(mut amount) => {
                if let Some(player) = self.player.as_ref() {
                    let mut cancel = false;
                    horizon_client::hunger::native_living_heal(
                        &self.loop_context,
                        player.entity_id as u32,
                        &mut amount,
                        &mut cancel,
                    );
                    if cancel {
                        return;
                    }
                }
                EffectAction::Heal(amount)
            }
            action => action,
        };
        let rules = self.world.health_rules;
        let Some(p) = self.player.as_mut() else {
            self.combat.forget_noted(false);
            return;
        };
        match action {
            EffectAction::Heal(amount) => {
                let max = p.attribute("minecraft:max_health").unwrap_or(20.0) as f32;
                let h = p.health.health();
                if h > 0.0 {
                    let _ = p.set_health((h + amount).min(max), false);
                }
            }
            EffectAction::Hurt(source, amount) => {
                let mut canceled = false;
                if let Some(id) = self.motion.health.damage.id(source)
                    && let Ok(o) = p.hurt_blocking(
                        crate::game::entity::damage_level_of!(self, scoped),
                        &self.motion.health,
                        rules,
                        id,
                        amount,
                        None,
                    )
                {
                    canceled = o.canceled;
                }

                self.combat.forget_noted(canceled);
            }
            EffectAction::Poison => {
                if p.health.health() > 1.0
                    && let Some(id) = self.health_resources.damage.id("minecraft:magic")
                {
                    let _ = p.hurt(
                        crate::game::entity::damage_level_of!(self, scoped),
                        &self.motion.health,
                        rules,
                        id,
                        1.0,
                    );
                }
            }
            EffectAction::Exhaust(amount) => {
                p.cause_food_exhaustion(amount);
            }
            EffectAction::Eat(nutrition, saturation) => {
                let food = &mut p.health.food;
                food.food = (food.food + nutrition).clamp(0, 20);
                food.saturation = (food.saturation + nutrition as f32 * saturation * 2.0)
                    .clamp(0.0, food.food as f32);
            }
        }
    }
    /// The local player's effects (`tick_mob_effects`), then the potions
    /// in flight, which a server without a local player still has. A frozen
    /// tick (`/tick freeze`) leaves the potions where they are
    /// (`TickRateManager.isEntityFrozen`); players' effects still run.
    pub(super) fn tick_effects(&mut self, running: bool) {
        self.tick_mob_effects();
        if running {
            self.tick_thrown_potions();
            self.tick_potion_clouds();
        }
    }
    /// `LivingEntity.tickEffects` (server side) for `self.player`, the local
    /// player or a guest acting (`as_guest`), then its attribute modifiers
    /// and status flags.
    pub(super) fn tick_mob_effects(&mut self) {
        let Some(player) = self.player.as_ref() else {
            return;
        };
        if player.health.dead_or_dying() {
            // LivingEntity.die → removeAllEffects. Vanilla runs the effects
            // that act on death (`onMobRemoved`) when the dead player is
            // removed, up to 20 ticks later; here it is as the effects go.
            let w = f64::from(player.dimensions.width) / 2.0;
            let bounds = Aabb::new(
                Vec3::new(
                    player.position.x - w,
                    player.position.y,
                    player.position.z - w,
                ),
                Vec3::new(
                    player.position.x + w,
                    player.position.y + f64::from(player.dimensions.height),
                    player.position.z + w,
                ),
            );
            let ids: Vec<&'static str> = self.potions.effects.list.iter().map(|e| e.id).collect();
            self.potions.effects.clear();
            if !ids.is_empty() {
                self.effects_on_death(&ids, bounds, true, None, None);
            }
        }
        // The omens act from inside the effect tick (BadOmenMobEffect,
        // RaidOmenMobEffect), as a content mod's effects do.
        let absorption_empty = self
            .player
            .as_ref()
            .is_some_and(|p| p.health.absorption() <= 0.0);
        let mut effects = std::mem::take(&mut self.potions.effects);
        // `MobEffectEvent.Expired`: ask before the pass, for the effects at
        // zero ticks and the ones reaching it in this pass (the last tick of
        // their countdown); a canceled expiry keeps the effect at no ticks,
        // asking again next tick. Only on the server, where the effects tick.
        let keep_expired: Vec<&'static str> = if self.remote.is_none() {
            let player = self.effect_living_ref(None);
            effects
                .list
                .iter()
                .filter(|e| matches!(e.duration, 0 | 1))
                .filter(|e| {
                    let Some(player) = player else {
                        return false;
                    };
                    let mut cancel = false;
                    self.native_mob_effect_expired(
                        player,
                        mod_api::__private::effect_key_by_index(
                            e.registry_index.expect("active effect registry index"),
                        )
                        .expect("effects resolved at load"),
                        e.amplifier,
                        &mut cancel,
                    );
                    cancel
                })
                .map(|e| e.id)
                .collect()
        } else {
            Vec::new()
        };
        let (actions, raid_omen_added) = effects.tick_with(
            &mut |e, t, actions, dispatch| {
                for action in actions.drain(..) {
                    self.apply_effect_action(action);
                }
                if !dispatch {
                    return OmenTick::Continue;
                }
                if !brewing::is_mod_effect(e.id) {
                    self.omen_tick(e)
                } else {
                    let Some(player) = self.player.as_mut() else {
                        return OmenTick::Remove;
                    };
                    let Some(epoch) = self.native_lifecycle.epoch() else {
                        return OmenTick::Continue;
                    };
                    let holder = LivingRef::new(EntityRef::new(epoch, player.entity_id as u32));
                    let health = player.health.health();
                    let max_health =
                        player.attribute("minecraft:max_health").unwrap_or(20.0) as f32;
                    let food = &player.health.food;
                    let food = Some(PlayerFood::from(mod_api::__private::built::PlayerFood {
                        level: food.food,
                        saturation: food.saturation,
                        exhaustion: food.exhaustion,
                    }));
                    let (keep, changes) = self.mod_effect_tick(
                        mod_api::__private::effect_key_by_index(
                            e.registry_index.expect("active effect registry index"),
                        )
                        .expect("effects resolved at load"),
                        None,
                        t,
                        e.amplifier,
                        holder,
                        health,
                        max_health,
                        food,
                    );
                    let registry = &self.motion.health.damage;
                    actions.extend(changes.into_iter().filter_map(|change| {
                        match change {
                            EffectTickChange::Heal(amount) => Some(EffectAction::Heal(amount)),
                            // An unregistered damage type hurts nothing.
                            EffectTickChange::Hurt(source, amount) => {
                                NamedDamage::registered(registry, &source)
                                    .map(|damage| EffectAction::Hurt(damage.name, amount))
                            }
                            EffectTickChange::AddExhaustion(amount) => {
                                Some(EffectAction::Exhaust(amount))
                            }
                            EffectTickChange::Eat(nutrition, saturation) => {
                                Some(EffectAction::Eat(nutrition, saturation))
                            }
                            _ => unreachable!("EffectTickChange: unhandled variant"),
                        }
                    }));
                    if keep {
                        OmenTick::Continue
                    } else {
                        OmenTick::Remove
                    }
                }
            },
            absorption_empty,
            &keep_expired,
        );
        self.potions.effects = effects;
        if raid_omen_added {
            self.raid_omen_added();
        }
        for action in actions {
            self.apply_effect_action(action);
        }
        self.turtle_helmet_tick();
        self.apply_effect_status();
    }
    /// `Player.tick` -> `turtleHelmetTick`: with a turtle helmet on the head
    /// and the eyes out of the water, Water Breathing for 200 ticks (no
    /// particles, an icon).
    fn turtle_helmet_tick(&mut self) {
        let Some(p) = self.player.as_ref() else {
            return;
        };
        let eye_y = p.position.y + f64::from(p.dimensions.eye_height);
        if !p.health.dead_or_dying()
            && self.armor[0]
                .as_ref()
                .is_some_and(|s| &*s.item == "minecraft:turtle_helmet")
            && !self.eye_in_water(p.position.x, eye_y, p.position.z)
        {
            let mut effect = EffectInstance::new("water_breathing", 200, 0);
            effect.visible = false;
            self.potions.effects.add(effect);
        }
    }
    /// A client's own effects (`LivingEntity.tickEffects` on the client,
    /// `MobEffectInstance.tickClient`): they count down and blend, and the
    /// server removes them. Their modifiers and flags move the predicted
    /// player.
    pub(super) fn tick_client_effects(&mut self) {
        client_tick_effects(&mut self.potions.effects, &mut self.sculk.darkness);
        self.apply_effect_status();
    }
    /// Add an active effect to the player and update attributes / absorption.
    /// `LivingEntity.canBeAffected` with the `mob_effect_applicable` hook:
    /// the event fires first, for every effect, and `Allow` or `Deny`
    /// decides; unanswered, the vanilla tag check runs. `entity` is a mob's
    /// `Entity.getId`, None for the player.
    pub(super) fn can_be_affected(
        &mut self,
        entity: Option<i32>,
        kind: &str,
        effect: &str,
    ) -> bool {
        let key = brewing::effect(effect)
            .and_then(|def| mod_api::__private::effect_key_by_index(def.index));
        self.can_be_affected_resolved(entity, kind, effect, key)
    }

    /// The validated registry path, also used by native operations without parsing a name again.
    pub(super) fn can_be_affected_resolved(
        &mut self,
        entity: Option<i32>,
        kind: &str,
        effect: &str,
        key: Option<mod_api::EffectKey>,
    ) -> bool {
        let mut answered = None;
        if let Some(living) = self.effect_living_ref(entity)
            && let Some(key) = key
        {
            self.native_mob_effect_applicable(living, key, &mut answered);
        }
        match answered {
            Some(EffectApplicable::Allow) => true,
            Some(EffectApplicable::Deny) => false,
            // An unanswered hook, or a future variant, defers to the
            // vanilla tag check.
            _ => mob_can_be_affected(&self.res.stations().enchantments.entity_tags, kind, effect),
        }
    }
    pub(super) fn add_player_effect(&mut self, effect: EffectInstance) -> bool {
        // `LivingEntity.addEffect` refuses what cannot affect the entity;
        // players pass the tag check, but the `mob_effect_applicable` hook
        // is asked for every add.
        let key = effect
            .registry_index
            .and_then(mod_api::__private::effect_key_by_index);
        if !self.can_be_affected_resolved(None, "minecraft:player", effect.id, key) {
            return false;
        }
        let id = effect.id;
        let amp = effect.amplifier;
        let new = self.potions.effects.get(id).is_none();
        let changed = self.potions.effects.add(effect);
        self.apply_effect_status();
        // MobEffect.onEffectAdded: an omen's sound when it is new, at the
        // player's feet (#11373).
        if new
            && changed
            && let Some(sound) = sound_on_added(id)
            && let Some(p) = self.player.as_ref()
        {
            let at = [p.position.x, p.position.y, p.position.z];
            self.audio.play(sound, Some(at), 1.0, 1.0);
        }
        if id == "absorption"
            && let Some(p) = self.player.as_mut()
        {
            let target = 4.0 * (amp + 1) as f32;
            let current = p.health.absorption();
            let _ = p.set_health(current.max(target), true);
        }
        if let Some(key) = key {
            self.mod_effect_added_resolved(key, None, amp, new);
        }
        changed
    }
    /// Removes one effect, or all effects, and refreshes the player's derived state.
    pub(super) fn remove_player_effects(&mut self, effect: Option<usize>) -> bool {
        let before = self.potions.effects.list.len();
        let mut index = 0;
        while index < self.potions.effects.list.len() {
            let current = &self.potions.effects.list[index];
            // The one being removed, or all of them when `effect` is None.
            if effect.is_some_and(|index| current.registry_index != Some(index)) {
                index += 1;
                continue;
            }
            let amplifier = current.amplifier;
            // `MobEffectEvent.Remove`: refusing the removal keeps it, and the
            // next one is asked with this one still in place, as in vanilla.
            let mut cancel = false;
            if let Some(living) = self.effect_living_ref(None) {
                self.native_mob_effect_remove(
                    living,
                    mod_api::__private::effect_key_by_index(
                        current
                            .registry_index
                            .expect("active effect registry index"),
                    )
                    .expect("effects resolved at load"),
                    amplifier,
                    &mut cancel,
                );
            }
            if cancel {
                index += 1;
            } else {
                self.potions.effects.list.remove(index);
            }
        }
        let changed = self.potions.effects.list.len() != before;
        if changed {
            self.apply_effect_status();
        }
        changed
    }
    /// `LivingEntity.addEffect` for a content mod's effect: its
    /// `onEffectAdded` when it is new, then its `onEffectStarted`, on a
    /// server. `entity` is a mob's id, None for the player.
    pub(super) fn mod_effect_added(
        &mut self,
        effect: &str,
        entity: Option<i32>,
        amplifier: i32,
        new: bool,
    ) {
        if let Some(effect) = brewing::effect(effect)
            .and_then(|def| mod_api::__private::effect_key_by_index(def.index))
        {
            self.mod_effect_added_resolved(effect, entity, amplifier, new);
        }
    }
    /// Dispatch an already resolved effect's added notices, without a registry name lookup.
    pub(super) fn mod_effect_added_resolved(
        &mut self,
        effect: mod_api::EffectKey,
        entity: Option<i32>,
        amplifier: i32,
        new: bool,
    ) {
        if self.remote.is_none()
            && mod_api::__private::effect_index(effect)
                .and_then(horizon_client_core::potions::effect_by_index)
                .is_some_and(|def| def.index >= brewing::EFFECTS.len())
        {
            if new {
                self.native_on_effect_added(effect, entity, amplifier);
            }
            self.native_mob_effect_added(effect, entity, amplifier);
        }
    }
    /// `MobEffectInstance.tickServer` for a content mod's effect: its
    /// `shouldApplyEffectTickThisTick`, then its `applyEffectTick`, which
    /// may end it. Whether it stays.
    fn mod_effect_tick(
        &self,
        effect: mod_api::EffectKey,
        entity: Option<i32>,
        tick_count: i32,
        amplifier: i32,
        holder: LivingRef,
        health: f32,
        max_health: f32,
        food: Option<PlayerFood>,
    ) -> (bool, Vec<EffectTickChange>) {
        let mut apply = false;
        self.native_should_apply_effect_tick_this_tick(effect, tick_count, amplifier, &mut apply);
        let mut keep = true;
        let mut changes = Vec::new();
        if apply {
            self.native_apply_effect_tick(mod_api::__private::ApplyEffectTickCall {
                effect,
                dimension: self.dim.id(),
                entity,
                amplifier,
                holder,
                health,
                max_health,
                food,
                changes: &mut changes,
                keep: &mut keep,
            });
        }
        (keep, changes)
    }
    /// The effects' attribute modifiers, armor protection and status flags
    /// on `self.player`.
    pub(super) fn apply_effect_status(&mut self) {
        // LivingEntity.onEffectAdded, onEffectUpdated and onEffectsRemoved
        // change the attributes on a server only. A server's client has the
        // modifiers its ClientboundUpdateAttributesPacket sends, which comes a
        // tick after the effect's own packet when the effect starts or ends in
        // the player's tick (ServerPlayer.doTick runs after the tick's
        // sendDirtyEntityData). Its own would slow it a tick before the
        // server's player when Speed runs out (#12040).
        if self.remote.is_some() {
            self.potions.applied.clear();
        }
        let modifiers = self.potions.effects.modifiers();
        if self.remote.is_none()
            && modifiers != self.potions.applied
            && let Some(p) = self.player.as_mut()
        {
            for (attr, id, _, op) in &self.potions.applied {
                let _ = p.set_modifier_unclamped(attr, id, 0.0, op, false);
            }
            for (attr, id, amount, op) in &modifiers {
                let _ = p.set_modifier_unclamped(attr, id, *amount, op, true);
            }
            // onAttributeUpdated, once the batch settled: a lost health_boost.
            let _ = p.clamp_health_to_max();
            self.potions.applied = modifiers;
        }
        if let Some(p) = self.player.as_mut()
            && self.potions.effects.amplifier("absorption").is_none()
            && p.health.absorption() > 0.0
        {
            let _ = p.set_health(0.0, true);
        }
        self.refresh_armor_protection();
        let mut status = self.potions.effects.status();
        if let Some(p) = self.player.as_mut() {
            status.protection = std::mem::take(&mut p.status.protection);
            p.status = status;
        }
    }
    /// `EnchantmentHelper.getDamageProtection` after equipment changes. This
    /// does not tick effects or clear absorption while a hit is in progress.
    pub(in crate::game) fn refresh_armor_protection(&mut self) {
        // EnchantmentHelper.getDamageProtection per damage type, recomputed on armor change.
        let armor: Vec<Option<ItemStack>> = self.armor.to_vec();
        if !self
            .potions
            .protection_for
            .as_ref()
            .is_some_and(|(generation, cached)| {
                *generation == self.damage_tag_generation && cached == &armor
            })
        {
            let mut table = Vec::new();
            let e = &self.stations().enchantments;
            let mut random = horizon_core::random::LegacyRandom::new(0);
            for (_, def) in self.health_resources.damage.entries() {
                let ctx = horizon_client::enchant::EffectContext {
                    items: Some(self.items()),
                    damage_type: Some(&def.name),
                    this: Some("minecraft:player"),
                    ..Default::default()
                };
                let total: f32 = armor
                    .iter()
                    .flatten()
                    .map(|s| {
                        e.modify_value(
                            Some(s),
                            "minecraft:damage_protection",
                            &ctx,
                            &mut random,
                            0.0,
                        )
                    })
                    .sum();
                if total > 0.0 {
                    table.push((def.name.clone(), total));
                }
            }
            self.potions.protection_for = Some((self.damage_tag_generation, armor));
            if let Some(p) = self.player.as_mut() {
                p.status.protection = table;
            }
        }
    }
    /// The players a thrown potion can hit (`Projectile.canHitEntity`): the
    /// host and the guests, but not a spectator or a dead one.
    fn thrown_player_targets(&self) -> Vec<(Who, Aabb)> {
        let mut targets = Vec::new();
        if let Some(p) = self.player_view()
            && p.alive
            && !self.spectator()
        {
            targets.push((Who::Player, p.bounds));
        }
        targets.extend(self.lan_projectile_targets());
        targets
    }
    /// The thrower's vehicle tree as `Projectile` sees it
    /// (`owner.getRootVehicle().getSelfAndPassengers()`): the players in it (the
    /// thrower and who shares their vehicle), the mob that is the vehicle, and
    /// the boxes of all of them. Until the potion has left it, none can be hit
    /// (`canHitEntity`: `isPassengerOfSameVehicle`) and any one near keeps it
    /// from leaving (`isOutsideOwnerCollisionRange`).
    pub(super) fn thrown_ridden(
        &self,
        owner: Option<Who>,
        players: &[(Who, Aabb)],
    ) -> (Vec<Who>, Option<u32>, Vec<Aabb>) {
        let Some(owner) = owner else {
            return (Vec::new(), None, Vec::new());
        };
        let vehicle_of = |who: Who| match who {
            Who::Player => self.riding,
            Who::Mob(id) => self
                .lan
                .as_ref()
                .and_then(|h| h.hub.guests.iter().find(|g| g.entity_id as u32 == id))
                .and_then(|g| g.vehicle),
        };
        let vehicle = vehicle_of(owner);
        let mut who = vec![owner];
        let mut boxes = Vec::new();
        for (w, b) in players {
            if *w == owner || (vehicle.is_some() && vehicle_of(*w) == vehicle) {
                if *w != owner {
                    who.push(*w);
                }
                boxes.push(*b);
            }
        }
        if let Some(v) = vehicle {
            if let Some(m) = self.mobs.iter().find(|m| m.id == v) {
                boxes.push(m.bounds());
            }
            if let Some(b) = self.boats.iter().find(|b| b.id == v) {
                boxes.push(b.bounds());
            }
        }
        (who, vehicle, boxes)
    }
    /// `ThrowableProjectile.tick` for the potions in flight: the motion, then
    /// the first block, mob or player the step meets
    /// (`ProjectileUtil.getHitResultOnMoveVector`), where the potion splashes
    /// (`Projectile.onHit`). Its thrower is not met until it has left them
    /// (`Projectile.leftOwner`, checked before the move like `Projectile.tick`),
    /// nor anything else in their vehicle tree.
    fn tick_thrown_potions(&mut self) {
        let mut thrown = std::mem::take(&mut self.potions.thrown);
        let players = if thrown.is_empty() {
            Vec::new()
        } else {
            self.thrown_player_targets()
        };
        let mut landed = Vec::new();
        let mut deflected: Vec<u32> = Vec::new();
        // ServerLevel.tick: only entities in entity-ticking chunks tick.
        let ticking = self.entity_ticking_area();
        thrown.retain_mut(|t| {
            if !ticking(t.position.x.floor() as i32, t.position.z.floor() as i32) {
                t.previous = t.position;
                return true;
            }
            t.age += 1;
            // Projectile.tick: whether it has left its thrower's vehicle tree,
            // before it moves.
            let owner = self.shooter_who(t.thrower, t.guest);
            let (ridden, vehicle, boxes) = self.thrown_ridden(owner, &players);
            if !t.left_owner {
                t.left_owner = thrown_left_owner(&boxes, t.position, t.velocity);
            }
            let skip: &[Who] = if t.left_owner { &[] } else { &ridden };
            t.previous = t.position;
            self.thrown_potion_motion(t);
            let next = Vec3::new(
                t.position.x + t.velocity.x,
                t.position.y + t.velocity.y,
                t.position.z + t.velocity.z,
            );
            // Hit a block along the step (sampled), a mob, a player, or fell out
            // of the world.
            let margin = thrown_margin(t.age);
            let steps = 4;
            let mut hit = None;
            let mut deflect_stop = false;
            for i in 1..=steps {
                let f = f64::from(i) / f64::from(steps);
                let p = Vec3::new(
                    t.position.x + t.velocity.x * f,
                    t.position.y + t.velocity.y * f,
                    t.position.z + t.velocity.z * f,
                );
                let state =
                    self.world
                        .block(p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
                let mob = self.mobs.iter().find(|m| {
                    m.alive()
                        && (t.left_owner || Some(m.id) != vehicle)
                        && (m.position.x - p.x).abs() < m.kind.width / 2.0 + 0.125
                        && (m.position.z - p.z).abs() < m.kind.width / 2.0 + 0.125
                        && p.y >= m.position.y
                        && p.y <= m.position.y + m.kind.height
                });
                // A player (`canHitEntity`: not the thrower before it has left
                // them), the box grown by the 0.3 pick margin.
                let player = thrown_player_at(&players, skip, p, margin);
                if self.res.facts().get(state).full_collision || mob.is_some() || player.is_some() {
                    // Projectile.hitTargetOrDeflectSelf: a breeze's deflection
                    // (`Breeze.deflection`) turns the potion back at half
                    // speed instead of `onHit`, and it flies on.
                    if let Some(m) = mob
                        && m.species() == horizon_client::mobs::Species::Breeze
                    {
                        if t.last_deflected_by != Some(m.id) {
                            // ProjectileDeflection.REVERSE.
                            let _ = t.random.next_float();
                            t.velocity = Vec3::new(
                                t.velocity.x * -0.5,
                                t.velocity.y * -0.5,
                                t.velocity.z * -0.5,
                            );
                            t.last_deflected_by = Some(m.id);
                            deflected.push(m.id);
                        }
                        // ThrowableProjectile.tick: `setPos(hitResult.getLocation())`
                        // before the deflection, so it does not step on past the
                        // breeze this tick.
                        t.position = p;
                        deflect_stop = true;
                        break;
                    }
                    // A block hit has the face it came in by
                    // (`onHitBlock`); an entity hit has none.
                    let block = (mob.is_none() && player.is_none())
                        .then(|| thrown_block_hit(t.position, p, t.velocity));
                    let at = mob.map(|m| m.position).or_else(|| {
                        player.map(|(_, b)| {
                            let c = b.center();
                            Vec3::new(c.x, b.min(Axis::Y), c.z)
                        })
                    });
                    hit = Some((p, at, block));
                    break;
                }
            }
            if let Some((p, mob, block)) = hit {
                landed.push((
                    t.id,
                    t.stack.clone(),
                    p,
                    mob,
                    block,
                    t.age,
                    t.owner.clone(),
                    t.guest,
                    self.entity_silent(t.id),
                    t.velocity,
                ));
                return false;
            }
            if deflect_stop {
                return t.age < 1200 && t.position.y >= self.world.below_world_y();
            }
            t.position = next;
            let bounds = Aabb::new(
                next.add_vector(Vec3::new(-0.125, 0.0, -0.125)),
                next.add_vector(Vec3::new(0.125, 0.25, 0.125)),
            );
            self.entity_powder_snow_particles(t.previous, next, bounds, t.velocity, false);
            t.age < 1200 && t.position.y >= self.world.below_world_y()
        });
        self.potions.thrown = thrown;
        for id in deflected {
            if let Some(index) = self.mobs.iter().position(|m| m.id == id) {
                self.play_mob_bound_sound(
                    index,
                    "minecraft:entity.breeze.deflect",
                    5,
                    1.0,
                    1.0,
                    false,
                );
            }
        }
        for (id, stack, at, mob, block, age, owner, guest, silent, motion) in landed {
            // Projectile.onHit (super.onHit of AbstractThrownPotion): a block
            // is heard at its centre, an entity at the hit.
            let heard = if mob.is_some() {
                [at.x, at.y, at.z]
            } else {
                [at.x.floor() + 0.5, at.y.floor() + 0.5, at.z.floor() + 0.5]
            };
            let thrower = owner.is_some().then_some(Who::Player);
            self.projectile_landed_event(id, thrower, heard);
            let item = stack.item.to_string();
            // A potion a dispenser threw has nobody to name.
            let owner = PotionOwner {
                player: owner.is_some(),
                name: owner,
                attacker: None,
                guest,
            };
            self.splash(&stack, at, mob, block, age, &owner, silent, motion);
            if item != "minecraft:experience_bottle" {
                self.native_potion_splash(&item, [at.x, at.y, at.z]);
            }
        }
    }
    /// `AreaEffectCloud.readAdditionalSaveData`: loaded clouds join the same
    /// simulation as lingering potions, with their own timing and potion data.
    /// The caller retains their UUID and original compound for command queries.
    pub(super) fn load_potion_cloud(&mut self, tag: &Tag) -> Option<u32> {
        let (_, mut cloud) = potion_cloud_from_nbt(tag)?;
        let id = self.take_entity_id();
        cloud.id = id;
        self.breath_clouds.push(cloud);
        Some(id)
    }
    /// `AreaEffectCloud.tick` for the lingering potions' clouds: the client
    /// tick's particles, then the server tick.
    fn tick_potion_clouds(&mut self) {
        if self.breath_clouds.iter().all(|c| c.potion.is_none()) {
            return;
        }
        let players: Vec<FightPlayer> = self
            .fight_players()
            .into_iter()
            .filter(|p| p.alive)
            .collect();
        let mut clouds = self.breath_clouds.lift();
        #[cfg(test)]
        super::entity::taken_pass_tests::after_take(self);
        let ticking = self.entity_ticking_area();
        let mut expired: Vec<u32> = Vec::new();
        clouds.retain_mut(|c| {
            let keep = c.potion.is_none()
                || !ticking(c.at[0].floor() as i32, c.at[2].floor() as i32)
                || self.tick_potion_cloud(c, &players);
            if !keep {
                expired.push(c.id);
            }
            keep
        });
        self.breath_clouds
            .restore(clouds, horizon_client::join_seq::RestoreOrder::SpawnsFirst);
        // A cloud that expired on its own takes its `noPhysics` write with it.
        for id in expired {
            self.forget_entity(id);
        }
    }
    /// `AreaEffectCloud.serverTick`: configured lifetime/radius changes and
    /// five-tick doses, with each victim's configured reapplication delay.
    /// False once the cloud is gone.
    fn tick_potion_cloud(&mut self, c: &mut Breath, players: &[FightPlayer]) -> bool {
        c.age = c.age.wrapping_add(1);
        let Some(potion) = c.potion.as_mut() else {
            return true;
        };
        if c.duration != -1 && c.age.wrapping_sub(potion.settings.wait_time) >= c.duration {
            return false;
        }
        let waiting = c.age < potion.settings.wait_time;
        cloud_client_tick(
            &mut self.particles,
            c.at,
            c.radius,
            waiting,
            &potion.particle(),
        );
        if waiting {
            return true;
        }
        if potion.radius_per_tick != 0.0 {
            c.radius += potion.radius_per_tick;
            if c.radius < 0.5 {
                return false;
            }
            c.radius = c.radius.clamp(0.0, 32.0);
        }
        if c.age % 5 != 0 {
            return true;
        }
        potion.victims.retain(|(_, until)| c.age < *until);
        if potion.effects.is_empty() {
            potion.victims.clear();
            return true;
        }
        let r = f64::from(c.radius);
        let area = Aabb::new(
            Vec3::new(c.at[0] - r, c.at[1], c.at[2] - r),
            Vec3::new(c.at[0] + r, c.at[1] + 0.5, c.at[2] + r),
        );
        let mut inside = Vec::new();
        for p in players.iter().filter(|p| p.bounds.intersects(area)) {
            inside.push((Victim::Player(p.who), p.position.x, p.position.z));
        }
        let effects = potion.effects.clone();
        // Only mobs that at least one of the cloud's effects can affect are
        // taken (`allEffects.stream().anyMatch(entity::canBeAffected)`), so
        // the others neither become victims nor use up its radius.
        let candidates: Vec<(u32, String, f64, f64)> = self
            .mobs
            .iter()
            .filter(|m| m.alive() && m.bounds().intersects(area))
            .map(|m| (m.id, m.kind.id.clone(), m.position.x, m.position.z))
            .collect();
        for (mob_id, kind, x, z) in candidates {
            let affected = effects
                .iter()
                .any(|(id, ..)| self.can_be_affected(Some(mob_id as i32), &kind, id));
            if affected {
                inside.push((Victim::Mob(mob_id), x, z));
            }
        }
        for (victim, x, z) in inside {
            let (dx, dz) = (x - c.at[0], z - c.at[2]);
            if potion.victims.iter().any(|(v, _)| *v == victim)
                || dx * dx + dz * dz > f64::from(c.radius * c.radius)
            {
                continue;
            }
            potion.victims.push((
                victim,
                c.age.wrapping_add(potion.settings.reapplication_delay),
            ));
            self.cloud_affect(
                victim,
                &effects,
                (
                    &potion.hidden,
                    &potion.ambient,
                    potion.settings.duration_scale,
                ),
                c.at,
                &potion.source,
                (potion.owner, potion.guest),
            );
            if potion.settings.radius_on_use != 0.0 {
                c.radius += potion.settings.radius_on_use;
                if c.radius < 0.5 {
                    return false;
                }
                c.radius = c.radius.clamp(0.0, 32.0);
            }
            if potion.settings.duration_on_use != 0 && c.duration != -1 {
                c.duration = c.duration.wrapping_add(potion.settings.duration_on_use);
                if c.duration <= 0 {
                    return false;
                }
            }
        }
        true
    }
    /// `Creeper.spawnLingeringCloud`: an `AreaEffectCloud` of radius 2.5, wait time 10, duration 300,
    /// shrinking by `-radius / duration` a tick and by 0.5 for each entity it affects, applying
    /// the creeper's effects at 0.25 of their remaining duration.
    pub(super) fn spawn_creeper_cloud(
        &mut self,
        at: [f64; 3],
        effects: Vec<(&'static str, i32, i32)>,
        hidden: Vec<&'static str>,
        ambient: Vec<&'static str>,
        color: u32,
    ) {
        let id = self.take_entity_id();
        self.breath_clouds.push(Breath {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id,
            at,
            motion: [0.0; 3],
            radius: 2.5,
            age: 0,
            duration: 300,
            from_dragon: false,
            victims: Vec::new(),
            potion: Some(PotionCloud {
                settings: CloudSettings::lingering(),
                effects,
                hidden,
                ambient,
                radius_per_tick: -2.5 / 300.0,
                color,
                custom_particle: None,
                victims: Vec::new(),
                source: Text::translate("entity.minecraft.area_effect_cloud", &[]),
                owner: None,
                guest: None,
            }),
        });
    }
    /// What a cloud puts on one entity: instant effects at half strength
    /// (`applyInstantaneousEffect` with 0.5), the others at the cloud's duration
    /// scale (`PotionContents.forEachEffect`).
    fn cloud_affect(
        &mut self,
        victim: Victim,
        effects: &[(&'static str, i32, i32)],
        (hidden, ambient, duration_scale): (&[&'static str], &[&'static str], f32),
        at: [f64; 3],
        source: &Text,
        (owner, guest): (Option<Who>, Option<u32>),
    ) {
        // `indirectMagic(cloud, owner)` names the cloud in the death message
        // (it gives no kill credit).
        let cloud_owner = PotionOwner {
            name: Some(source.clone()),
            ..Default::default()
        };
        // MobEffectInstance.withScaledDuration: floor(duration * scale), at least 1; an infinite
        // effect stays infinite.
        let scaled = |duration| horizon_client::effects::scaled_duration(duration, duration_scale);
        match victim {
            Victim::Player(who) => self.as_fight_player(who, |g| {
                for (id, duration, amp) in effects {
                    let Some(e) = brewing::effect(id) else {
                        continue;
                    };
                    if e.instant {
                        if let Some(a) = horizon_client::effects::instant(id, *amp, 0.5) {
                            g.apply_indirect_effect_action(a, &cloud_owner);
                        }
                    } else {
                        let mut instance = EffectInstance::new(e.id, scaled(*duration), *amp);
                        instance.visible = !hidden.contains(id);
                        instance.ambient = ambient.contains(id);
                        g.add_player_effect(instance);
                    }
                }
            }),
            Victim::Mob(mob) => {
                let Some(i) = self.mobs.iter().position(|m| m.id == mob) else {
                    return;
                };
                // Undead invert healing and harming.
                let tags = &world_stations!(self).enchantments.entity_tags;
                let inverted = tags
                    .get("minecraft:inverted_healing_and_harm")
                    .is_some_and(|t| t.contains(&self.mobs[i].kind.id));
                for (id, duration, amp) in effects {
                    // A mod's `onEffectAdded` may have changed the mobs.
                    let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob) else {
                        return;
                    };
                    match *id {
                        "instant_damage" | "instant_health" => {
                            let amount = |base: i32| {
                                (0.5 * f64::from(base.wrapping_shl(*amp as u32)) + 0.5) as i32
                                    as f32
                            };
                            if (*id == "instant_damage") != inverted {
                                // HealOrHarmMobEffect.applyInstantaneousEffect: indirectMagic(cloud, owner).
                                let source = Damage::indirect_magic(owner, at);
                                if m.damage_by(
                                    crate::game::entity::damage_level_of!(self, scoped),
                                    amount(6),
                                    source,
                                    "minecraft:area_effect_cloud",
                                ) && owner == Some(Who::Player)
                                    && let Some(uuid) = self.player_uuid_of(guest)
                                {
                                    self.last_hurt_by_player.insert(
                                        mob,
                                        super::combat::PlayerCredit(
                                            horizon_client::lan::playerdata::ints_uuid(uuid),
                                        ),
                                    );
                                }
                            } else {
                                m.heal(&self.loop_context, amount(4));
                            }
                        }
                        _ => {
                            // `LivingEntity.addEffect` refuses what the mob
                            // cannot be affected by: the borrow ends before
                            // the hook asks, and the mob is found again.
                            let kind = m.kind.id.clone();
                            if self.can_be_affected(Some(mob as i32), &kind, id)
                                && let Some(name) = mob_effect_id(id)
                            {
                                let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob) else {
                                    return;
                                };
                                let new = !m.has_effect(name);
                                m.add_effect_with(
                                    name,
                                    scaled(*duration),
                                    *amp,
                                    ambient.contains(id),
                                    !hidden.contains(id),
                                );
                                self.mod_effect_added(name, Some(mob as i32), *amp, new);
                            }
                        }
                    }
                }
            }
        }
    }
    /// `ThrowableProjectile.tick`'s `applyGravity` and `applyInertia`, which
    /// come before the move: gravity 0.05 (0.07 for an experience bottle),
    /// then drag 0.99 (0.8 in water).
    fn thrown_potion_motion(&self, t: &mut ThrownPotion) {
        let gravity = if &*t.stack.item == "minecraft:experience_bottle" {
            0.07
        } else {
            0.05
        };
        let at = t.position;
        let water = self
            .res
            .facts()
            .get(self.world.block(
                at.x.floor() as i32,
                at.y.floor() as i32,
                at.z.floor() as i32,
            ))
            .fluid
            == horizon_client::blocks::Fluid::Water;
        let inertia = if water { 0.8 } else { 0.99 };
        t.velocity = Vec3::new(
            t.velocity.x * inertia,
            (t.velocity.y - gravity) * inertia,
            t.velocity.z * inertia,
        );
    }
    /// `ThrowableProjectile.tick` on a server's client: it falls and moves
    /// by itself between the server's updates and stops at a block; only
    /// the server breaks it.
    pub(super) fn step_remote_thrown(&mut self) {
        let mut thrown = std::mem::take(&mut self.potions.thrown);
        for t in &mut thrown {
            t.previous = t.position;
            t.age += 1;
            self.thrown_potion_motion(t);
            let next = Vec3::new(
                t.position.x + t.velocity.x,
                t.position.y + t.velocity.y,
                t.position.z + t.velocity.z,
            );
            let state = self.world.block(
                next.x.floor() as i32,
                next.y.floor() as i32,
                next.z.floor() as i32,
            );
            if self.res.facts().get(state).full_collision {
                t.velocity = Vec3::ZERO;
            } else {
                t.position = next;
            }
        }
        self.potions.thrown = thrown;
    }
    /// `ThrownItemRenderer` for thrown splash and lingering potions and
    /// experience bottles: the item (GROUND) facing the camera at scale 1,
    /// lit at the light probe (the eye, 0.85 of the 0.25 height).
    pub(super) fn thrown_potion_geometry(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<Vertex>,
    ) {
        let orientation = glam::Mat4::from_quat(self.camera_orientation());
        for t in &self.potions.thrown {
            let previous = glam::DVec3::new(t.previous.x, t.previous.y, t.previous.z);
            let position = glam::DVec3::new(t.position.x, t.position.y, t.position.z);
            // ThrowableProjectile.shouldRenderAtSqrDistance: hidden for its
            // first two ticks within 3.5 blocks (not in the thrower's face).
            if t.age < 2 && position.distance_squared(camera) < 12.25 {
                continue;
            }
            if !self.standing_entity_in_view(
                camera,
                super::entity_culling::at(t.previous),
                super::entity_culling::at(t.position),
                alpha,
                0.25,
                0.25,
                64.0,
                0.0,
            ) {
                continue;
            }
            let p = previous.lerp(position, alpha);
            let owner = owner_at(p.to_array(), 0.0, &t.stack.item);
            let state = self.resolve_item(&t.stack, DisplayContext::Ground, Some(&owner), 0);
            if state.is_empty() {
                continue;
            }
            let pose = glam::Mat4::from_translation((p - camera).as_vec3()) * orientation;
            let light = self.world.light(
                p.x.floor() as i32,
                (p.y + 0.2125).floor() as i32,
                p.z.floor() as i32,
            );
            // Frame generation and temporal upscaling follow its own flight; one not yet
            // given an id (0) can't be told apart from another, so it keeps the camera's.
            let motion = if t.id == 0 {
                0
            } else {
                Self::MOTION_OBJECT | u64::from(t.id)
            };
            self.mark_motion(out.len(), motion);
            self.submit_item_world(&state, pose, vertex_light(light), out);
        }
        self.mark_motion(out.len(), 0);
    }
    /// `HealOrHarmMobEffect.applyInstantaneousEffect` with a source (a
    /// potion or a cloud): indirectMagic, not plain magic, credited to the
    /// mob that threw it when there is one.
    fn apply_indirect_effect_action(&mut self, a: EffectAction, owner: &PotionOwner) {
        let a = match a {
            EffectAction::Hurt("minecraft:magic", amount) => {
                match (owner.attacker, &owner.name) {
                    (Some(id), Some(name)) => {
                        self.note_named_attacker("minecraft:indirect_magic", id, name.clone())
                    }
                    _ => {
                        let name = owner.name.clone().unwrap_or_else(|| {
                            Text::translate("entity.minecraft.splash_potion", &[])
                        });
                        self.note_direct_entity("minecraft:indirect_magic", name);
                    }
                }
                EffectAction::Hurt("minecraft:indirect_magic", amount)
            }
            a => a,
        };
        self.apply_effect_action(a);
    }
    /// A splash potion's effects on the player in place, by their distance.
    fn splash_on_player(
        &mut self,
        effects: &'static [(&'static str, i32, i32)],
        at: Vec3,
        age: u32,
        owner: &PotionOwner,
    ) {
        if let Some(p) = self.player.as_ref() {
            let w = f64::from(p.dimensions.width) / 2.0;
            let bounds = Aabb::new(
                Vec3::new(p.position.x - w, p.position.y, p.position.z - w),
                Vec3::new(
                    p.position.x + w,
                    p.position.y + f64::from(p.dimensions.height),
                    p.position.z + w,
                ),
            );
            if let Some(d2) = splash_distance_sqr(at, age, bounds) {
                let intensity = 1.0 - d2.sqrt() / 4.0;
                for (id, duration, amp) in effects {
                    let instant = brewing::effect(id).is_some_and(|e| e.instant);
                    if instant {
                        if let Some(a) = horizon_client::effects::instant(id, *amp, intensity) {
                            self.apply_indirect_effect_action(a, owner);
                        }
                    } else {
                        let d = (intensity * f64::from(*duration) + 0.5) as i32;
                        if d > 20
                            && let Some(e) = brewing::effect(id)
                        {
                            self.add_player_effect(EffectInstance::new(e.id, d, *amp));
                        }
                    }
                }
            }
        }
    }
    /// `Projectile.getOwner` of a thrown potion, as [`splash`](Self::splash)
    /// names it: a player's, a mob's (a witch keeps the kill credit), or
    /// nobody's (a dispenser's, a summoned potion's).
    pub(super) fn potion_owner(&self, owner: Option<Who>) -> PotionOwner {
        match owner {
            Some(Who::Player) => PotionOwner {
                name: Some(self.player_display_text()),
                attacker: None,
                player: true,
                guest: self.lan_acting,
            },
            Some(Who::Mob(id)) => PotionOwner {
                name: self.mob_display_name(id),
                attacker: Some(id),
                player: false,
                guest: None,
            },
            None => PotionOwner::default(),
        }
    }
    /// `AbstractThrownPotion.onHit` with `ThrownSplashPotion.onHitAsPotion`, or
    /// `ThrownLingeringPotion.onHitAsPotion` for a lingering potion, read from
    /// the stack's `potion_contents`; `mob` is where it hit a mob, `hit` the
    /// block and face it hit, `age` its age, `silent` its Silent flag, `motion` its motion as it hit.
    pub(super) fn splash(
        &mut self,
        stack: &ItemStack,
        at: Vec3,
        mob: Option<Vec3>,
        hit: Option<([i32; 3], Direction)>,
        age: u32,
        owner: &PotionOwner,
        silent: bool,
        motion: Vec3,
    ) {
        let block = [at.x, at.y, at.z].map(|v| v.floor() as i32);
        if &*stack.item == "minecraft:experience_bottle" {
            // ThrownExperienceBottle.onHit: the water-coloured splash (level
            // event 2002, its sound 1053) and 3 to 11 experience.
            self.level_event_at(2002, block, 0xFF38_5DC6u32 as i32);
            if !silent {
                self.level_event_at(1053, block, 0);
            }
            let r = &mut self.world.random;
            let value = 3 + r.next_int_bound(5).unwrap_or(0) + r.next_int_bound(5).unwrap_or(0);
            // awardWithDirection: away from the face it hit, or back along
            // its own motion when it hit an entity (#11388).
            let roughly = match hit {
                Some((_, face)) => face.normal().map(f64::from),
                None => [-motion.x, -motion.y, -motion.z],
            };
            self.spawn_orbs_toward([at.x, at.y, at.z], roughly, value as u32);
            self.native_exp_bottle([at.x, at.y, at.z], value as u32);
            return;
        }
        // Projectile.onHit calls onHitBlock first, then the potion's
        // affectEntitiesAround; both read the stack's `potion_contents`.
        if let Some((cell, face)) = hit {
            // `AbstractThrownPotion.douseFire` passes `getOwner()` to the
            // campfire's `douse`, which is where the game event's source is.
            let who = if owner.player {
                Some(Who::Player)
            } else {
                owner.attacker.map(Who::Mob)
            };
            self.potion_hit_block(stack, cell, face, who);
        }
        self.affect_entities_around(stack, at, owner.player);
        let lingering = &*stack.item == "minecraft:lingering_potion";
        // AbstractThrownPotion.onHit: level event 2007 and sound 1054 for a
        // potion with instant effects, 2002 and 1053 otherwise.
        let potion = stack.potion().and_then(brewing::potion);
        let instant = potion.is_some_and(|d| {
            d.effects
                .iter()
                .any(|(id, ..)| brewing::effect(id).is_some_and(|e| e.instant))
        });
        let color = brewing::potion_color(stack.potion());
        self.level_event_at(if instant { 2007 } else { 2002 }, block, color as i32);
        if !silent {
            self.level_event_at(if instant { 1054 } else { 1053 }, block, 0);
        }
        // AbstractThrownPotion.affectEntitiesAround: a water potion (the only
        // #rehydrates_axolotls) refills the air of the axolotls in the
        // projectile's box inflated by 4, 2 and 4 (Axolotl.rehydrate, 1800).
        if stack.potion() == Some("minecraft:water") {
            let (lo, hi) = (
                [at.x - 0.125 - 4.0, at.y - 2.0, at.z - 0.125 - 4.0],
                [at.x + 0.125 + 4.0, at.y + 0.25 + 2.0, at.z + 0.125 + 4.0],
            );
            for m in self
                .mobs
                .iter_mut()
                .filter(|m| m.species() == horizon_client::mobs::Species::Axolotl && !m.removed)
            {
                let (w, h, _) = m.dims();
                let half = w / 2.0;
                let (x, y, z) = (m.position.x, m.position.y, m.position.z);
                if x - half < hi[0]
                    && x + half > lo[0]
                    && y < hi[1]
                    && y + h > lo[1]
                    && z - half < hi[2]
                    && z + half > lo[2]
                {
                    m.air = (m.air + 1800).min(horizon_client::mobs::axolotl::MAX_AIR);
                }
            }
        }
        let Some(def) = potion else {
            return;
        };
        if lingering {
            // The cloud sits at the mob it hit (its feet), else the impact;
            // radius 3, wait 10, 600 ticks, shrinking to nothing, and a
            // potion with no effects leaves none.
            if !def.effects.is_empty() {
                let at = mob.unwrap_or(at);
                let id = self.take_entity_id();
                self.breath_clouds.push(Breath {
                    attachment_lifetime: Default::default(),
                    join: Default::default(),
                    id,
                    at: [at.x, at.y, at.z],
                    motion: [0.0; 3],
                    radius: 3.0,
                    age: 0,
                    duration: 600,
                    from_dragon: false,
                    victims: Vec::new(),
                    potion: Some(PotionCloud {
                        settings: CloudSettings::lingering(),
                        effects: def.effects.to_vec(),
                        hidden: Vec::new(),
                        ambient: Vec::new(),
                        radius_per_tick: -3.0 / 600.0,
                        color,
                        custom_particle: None,
                        victims: Vec::new(),
                        source: owner.name.clone().unwrap_or_else(|| {
                            Text::translate("entity.minecraft.area_effect_cloud", &[])
                        }),
                        owner: owner.who(),
                        guest: owner.guest,
                    }),
                });
            }
            return;
        }
        // Every player in range (`ThrownSplashPotion.applySplash` over
        // `LivingEntity`s, the players among them).
        self.for_each_player(|g| g.splash_on_player(def.effects, at, age, owner));
        // Mobs take the instant effects (undead invert healing and harming).
        let undead = &self.stations().enchantments.entity_tags;
        let mut hits = Vec::new();
        for (i, m) in self.mobs.iter().enumerate() {
            if m.alive()
                && let Some(d2) = splash_distance_sqr(at, age, m.bounds())
            {
                let inverted = undead
                    .get("minecraft:inverted_healing_and_harm")
                    .is_some_and(|t| t.contains(&m.kind.id));
                hits.push((i, 1.0 - d2.sqrt() / 4.0, inverted));
            }
        }
        for (i, intensity, inverted) in hits {
            for (id, duration, amp) in def.effects {
                let harm = match *id {
                    "instant_damage" => !inverted,
                    "instant_health" => inverted,
                    _ => {
                        // `Potion.applySplash`: timed effects last in
                        // proportion to how close the mob was.
                        let d = (intensity * f64::from(*duration) + 0.5) as i32;
                        if d > 20
                            && let Some(name) = mob_effect_id(id)
                        {
                            self.mobs[i].add_effect(name, d, *amp);
                        }
                        continue;
                    }
                };
                let m = &mut self.mobs[i];
                if harm && m.species() == horizon_client::mobs::Species::Enderman {
                    // `Enderman.hurtServer` for a thrown potion: only water hurts
                    // (`hurtWithCleanWater`), then `repeatedlyTryToTeleport`, which
                    // the AI step runs and which does nothing while it rides.
                    if m.alive() && !m.extra.passenger {
                        m.extra.counter2 = 64;
                    }
                } else if harm {
                    // HealOrHarmMobEffect.applyInstantaneousEffect: indirectMagic(potion, owner).
                    let harm = (intensity * f64::from(6 << amp) + 0.5) as i32 as f32;
                    let source = Damage::indirect_magic(owner.who(), [at.x, at.y, at.z]);
                    let id = m.id;
                    let hurt = m.damage_by(
                        crate::game::entity::damage_level_of!(self, scoped),
                        harm,
                        source,
                        "minecraft:splash_potion",
                    );
                    if hurt
                        && owner.player
                        && let Some(uuid) = self.player_uuid_of(owner.guest)
                    {
                        self.last_hurt_by_player.insert(
                            id,
                            super::combat::PlayerCredit(
                                horizon_client::lan::playerdata::ints_uuid(uuid),
                            ),
                        );
                    }
                } else {
                    m.heal(
                        &self.loop_context,
                        (intensity * f64::from(4 << amp) + 0.5) as i32 as f32,
                    );
                }
            }
        }
    }
    /// `Screen.showsActiveEffects`: the survival and creative inventories
    /// draw their own effect list when `EffectsInInventory.canSeeEffects`
    /// (32 or more pixels right of the screen).
    fn screen_shows_active_effects(&self, batch: &HudBatch) -> bool {
        let (ox, image_width) = match self.screen {
            Screen::Inventory
                if self.container.is_none() && self.craft_size == 2 && !self.mount_active() =>
            {
                (self.inventory_origin(batch).0, 176)
            }
            Screen::Creative => (
                Self::creative_origin(batch).0,
                horizon_client::creative::Creative::WIDTH as i32,
            ),
            _ => return false,
        };
        batch.width as i32 - (ox as i32 + image_width + 2) >= 32
    }
    /// `Hud.extractEffects`: icons top-right, beneficial row then harmful row;
    /// every screen but the inventories keeps them.
    pub(super) fn draw_effect_icons(&self, batch: &mut HudBatch) {
        if self.potions.effects.list.is_empty() || self.screen_shows_active_effects(batch) {
            return;
        }
        let icons: Vec<hud_overlays::EffectIcon> = self
            .potions
            .effects
            .list
            .iter()
            .map(hud_overlays::EffectIcon::from)
            .collect();
        let draws = hud_overlays::effects(batch.width as i32, false, &icons);
        hud_layout::paint(batch, &self.gui.atlas, &draws);
    }
    /// `EffectsInInventory`: panels to the right of survival/creative inventory.
    /// The caller draws the returned tooltip after every other tooltip.
    /// `ScreenEvent.RenderInventoryMobEffects` fires when the build lays the
    /// panels out once a frame, and its answer is kept in the build.
    pub(super) fn draw_inventory_effects(
        &self,
        batch: &mut HudBatch,
        origin: (f32, f32),
        image_width: i32,
    ) -> Option<Vec<(Text, u32)>> {
        let space = Self::effects_space(batch, [origin.0, origin.1], image_width as f32);
        let layout = self.built_container(batch).map_or_else(
            || inventory_effects::EffectsLayout::vanilla(space.available()),
            |layout| layout.effects_layout,
        );
        let panels = self.effect_panels(&space, &layout);
        let hovered = inventory_effects::paint(
            batch,
            &self.gui.atlas,
            &panels,
            [self.cursor.0 as i32, self.cursor.1 as i32],
        )?;
        Some(self.effect_tooltip(batch, hovered))
    }
    /// A hovered panel's tooltip lines, after `GatherEffectScreenTooltipsEvent`
    /// (fired only while a mod listens).
    fn effect_tooltip(
        &self,
        batch: &HudBatch,
        hovered: inventory_effects::HoveredEffectPanel<'_, '_>,
    ) -> Vec<(Text, u32)> {
        let panel = hovered.panel;
        let mut lines = inventory_effects::tooltip(hovered);
        if mod_api::__private::listened(mod_api::HookId::GatherEffectScreenTooltips) {
            let view = self.screen_view(batch, self.screen);
            let effect = super::native_living_effects::active_effect(panel.effect);
            self.native_gather_effect_screen_tooltips(&view, &effect, &mut lines);
        }
        lines
    }
    /// The rectangle `draw_inventory_effects` covers, none with no effects
    /// or too little room right of the window, laid out as `layout` says.
    pub(super) fn inventory_effects_rect(
        &self,
        batch: &HudBatch,
        origin: [f32; 2],
        image_width: f32,
        layout: &inventory_effects::EffectsLayout,
    ) -> Option<[f32; 4]> {
        let space = Self::effects_space(batch, origin, image_width);
        let panels = self.effect_panels(&space, layout);
        let first = panels.first()?;
        let width = panels.iter().map(|p| p.width).max()?;
        let height = (panels.len() as i32 - 1) * first.spacing + 32;
        Some([first.x as f32, first.y as f32, width as f32, height as f32])
    }
    /// The room the effect panels lay out in (`EffectsInInventory`'s space):
    /// the window's width, the container's origin and its image width.
    pub(super) fn effects_space(
        batch: &HudBatch,
        origin: [f32; 2],
        image_width: f32,
    ) -> inventory_effects::EffectsSpace {
        inventory_effects::EffectsSpace {
            screen_width: batch.width as i32,
            origin: [origin[0] as i32, origin[1] as i32],
            image_width: image_width as i32,
        }
    }
    fn effect_panels(
        &self,
        space: &inventory_effects::EffectsSpace,
        layout: &inventory_effects::EffectsLayout,
    ) -> Vec<inventory_effects::EffectPanel<'_>> {
        inventory_effects::panels(
            space,
            layout,
            &self.potions.effects.list,
            self.clock.rate(),
            &|key| self.pack.names.display_get(key),
            &|text| self.gui.atlas.font.width(text).ceil() as i32,
        )
    }
    /// Vanilla's layout, as `native_screen_render_inventory_mob_effects`
    /// leaves it, for the panels and their exclusion rect. No fire with no
    /// effect to show or no room (vanilla returns first), nor with no mod
    /// listening. The answer is taken once a frame: the display's change
    /// builds twice in one frame (`apply_ui_settings` then `observe_screen`),
    /// and the second build reuses the first one's answer.
    pub(super) fn inventory_effects_layout(
        &self,
        space: &inventory_effects::EffectsSpace,
        rect: [f32; 4],
    ) -> inventory_effects::EffectsLayout {
        let mut layout = inventory_effects::EffectsLayout::vanilla(space.available());
        if self.potions.effects.list.is_empty()
            || space.available() < 32
            || !mod_api::__private::listened(mod_api::HookId::ScreenRenderInventoryMobEffects)
        {
            return layout;
        }
        if let Some(answer) =
            self.screen_state
                .build
                .effects_answer(self.frames, self.screen, *space)
        {
            return answer;
        }
        let view = self.screen_view_at(self.screen, rect);
        let mut canceled = false;
        self.native_screen_render_inventory_mob_effects(
            &view,
            space.available(),
            &mut layout.compact,
            &mut layout.offset,
            &mut canceled,
        );
        layout.hidden = canceled;
        self.screen_state
            .build
            .keep_effects_answer(self.frames, self.screen, *space, layout);
        layout
    }
    /// Effects saved in level.dat (`Player.active_effects`).
    pub(super) fn restore_effects(&mut self) {
        self.potions = PotionState::default();
        self.potions.effects = PlayerEffects::from_saved(&self.level.active_effects);
    }
    pub(super) fn effects_json(&self) -> serde_json::Value {
        serde_json::json!({
            "active": self.potions.effects.list.iter().map(|e| serde_json::json!({ "id": e.id, "duration": e.duration, "amplifier": e.amplifier })).collect::<Vec<_>>(),
            "thrown": self.potions.thrown.iter().map(|t| [t.position.x, t.position.y, t.position.z]).collect::<Vec<_>>(),
        })
    }
}

/// A mob's content mod effects tick in its own tick (`ai::tick_effects`).
impl horizon_client::mobs::ModEffectTicks for Game<'_> {
    fn tick_mod_effect(
        &self,
        level: &mut DamageLevel<'_>,
        effect: usize,
        mob: &mut Mob,
        tick_count: i32,
        amplifier: i32,
    ) -> bool {
        let Some(epoch) = self.native_lifecycle.epoch() else {
            return true;
        };
        let holder = LivingRef::new(EntityRef::new(epoch, mob.id));
        let Some(effect) = mod_api::__private::effect_key_by_index(effect) else {
            return true;
        };
        let (keep, changes) = self.mod_effect_tick(
            effect,
            Some(mob.id as i32),
            tick_count,
            amplifier,
            holder,
            mob.health,
            mob.attrs.max_health,
            None,
        );
        for change in changes {
            match change {
                EffectTickChange::Heal(amount) => mob.heal(&self.loop_context, amount),
                // An unregistered damage type hurts nothing.
                EffectTickChange::Hurt(source, amount) => {
                    let registry = &self.motion.health.damage;
                    if registry.id(&source).is_some() {
                        let kind = Self::damage_kind(registry, &source);
                        mob.damage(level, amount, Damage::of(kind));
                    }
                }
                _ => {}
            }
        }
        keep
    }
    fn effect_expired(&self, mob: u32, effect: usize, amplifier: i32, cancel: &mut bool) {
        if let Some(living) = self.effect_living_ref(Some(mob as i32))
            && let Some(effect) = mod_api::__private::effect_key_by_index(effect)
        {
            self.native_mob_effect_expired(living, effect, amplifier, cancel);
        }
    }
}

/// `LivingEntity.tickEffects` on a client: each `MobEffectInstance.tickClient`
/// counts its effect down, then ticks its `BlendState`. The darkness effect's
/// is the fade `darkness_lighting` and `darkness_fog` draw; the integrated
/// level's `tick_sculk_entities` is not run for a server's client (#3393).
fn client_tick_effects(effects: &mut PlayerEffects, darkness: &mut (f32, f32)) {
    for e in &mut effects.list {
        e.tick_client();
    }
    *darkness = super::sculk::darkness_blend_tick(effects, *darkness);
}

#[cfg(test)]
mod tests {
    use super::super::sculk::darkness_blend_added;
    use super::*;
    use horizon_client::{motion::PlayerMotion, screen_effects::nausea_blend_added};
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn lifecycle_cloud_commands_and_parked_owners() {
        super::super::entity::taken_pass_tests::LifecycleFixture {
            seed: |game, entries| {
                game.breath_clouds.clear();
                for &(id, at) in entries {
                    game.spawn_creeper_cloud(at, Vec::new(), Vec::new(), Vec::new(), 0);
                    let i = game.breath_clouds.len() - 1;
                    let mut cloud = game.breath_clouds.remove(i);
                    cloud.id = id;
                    game.breath_clouds.push(cloud);
                }
            },
            ids: |game| game.breath_clouds.iter().map(|c| c.id).collect(),
            target: super::super::commands::Target::Cloud,
            unloads_with_chunk: true,
            remote: None,
        }
        .run();
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn taken_breath_tick_keeps_both_spawns_before_survivors() {
        cloud_spawns_during_pass(false);
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn taken_potion_tick_keeps_both_spawns_before_survivors() {
        cloud_spawns_during_pass(true);
    }

    fn cloud_spawns_during_pass(potion_pass: bool) {
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.breath_clouds.clear();
            for _ in 0..2 {
                game.spawn_creeper_cloud([0.0, 80.0, 0.0], Vec::new(), Vec::new(), Vec::new(), 0);
            }
            if !potion_pass {
                game.breath_clouds.for_each_mut(|c| c.potion = None);
            }
            let first = game.breath_clouds[0].id;
            let second = game.breath_clouds[1].id;
            let next = game.next_entity;
            super::super::entity::taken_pass_tests::with_spawn(
                |game| {
                    assert!(game.breath_clouds.is_empty());
                    for _ in 0..2 {
                        game.spawn_creeper_cloud(
                            [0.0, 80.0, 0.0],
                            Vec::new(),
                            Vec::new(),
                            Vec::new(),
                            0,
                        );
                    }
                },
                || {
                    if potion_pass {
                        game.tick_potion_clouds();
                    } else {
                        game.tick_breath();
                    }
                },
            );
            assert_eq!(
                game.breath_clouds
                    .iter()
                    .map(|c| (c.id, c.age))
                    .collect::<Vec<_>>(),
                [(next + 1, 0), (next + 2, 0), (first, 1), (second, 1)]
            );
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn residency_cloud_load_expiry_bottling_and_late_ids() {
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.breath_clouds.clear();
            game.ominous_spawners.clear();
            let (tag, _) =
                horizon_core::nbt::parse_snbt("{Pos:[0d,80d,0d],Duration:2,WaitTime:0,Radius:3f}")
                    .unwrap();
            let potion = game.load_potion_cloud(&tag).unwrap();
            for (age, duration) in [(20, 1), (0, 40)] {
                game.breath_clouds.push(super::super::dragon::Breath {
                    attachment_lifetime: Default::default(),
                    join: Default::default(),
                    id: 0,
                    at: [0.0, 80.0, 0.0],
                    motion: [0.0; 3],
                    radius: 3.0,
                    age,
                    duration,
                    from_dragon: false,
                    victims: Vec::new(),
                    potion: None,
                });
            }
            game.add_remote_ominous_item_spawner(0, [0.0, 80.0, 0.0]);
            let before = game.next_entity;
            assert_eq!(game.breath_clouds[1].id, 0);
            game.lan_assign_ids();
            assert_eq!(game.ominous_spawners[0].id, before + 1);
            assert_eq!(game.breath_clouds[1].id, before + 2);
            assert_eq!(game.breath_clouds[2].id, before + 3);
            game.tick_breath();
            assert_eq!(
                game.breath_clouds
                    .iter()
                    .map(|c| (c.id, c.age))
                    .collect::<Vec<_>>(),
                [(potion, 0), (before + 3, 1)]
            );
            game.tick_potion_clouds();
            game.tick_potion_clouds();
            assert_eq!(game.breath_clouds.len(), 1);
            assert_eq!(game.breath_clouds[0].age, 1);
            game.breath_clouds.update(0, |c| c.from_dragon = true);
            assert!(
                collect_dragon_breath(
                    Aabb::new(Vec3::new(-0.3, 80.0, -0.3), Vec3::new(0.3, 81.8, 0.3)),
                    &mut game.breath_clouds
                )
                .is_some()
            );
            assert_eq!(game.breath_clouds[0].radius, 2.5);
            game.breath_clouds.clear();
            assert!(game.breath_clouds.is_empty());
        });
    }

    fn effect_test_mod() {
        use mod_api::{self, *};
        struct TestMod;
        #[mod_api::__subscriptions]
        impl Hooks for TestMod {
            fn should_apply_effect_tick_this_tick(
                &self,
                _: &dyn Host,
                event: &mut ShouldApplyEffectTickThisTickEvent,
            ) {
                event.apply = true;
            }
            fn apply_effect_tick(&self, _: &dyn Host, event: &mut ApplyEffectTickEvent<'_>) {
                match event.amplifier {
                    0 => {
                        event.heal(10.0);
                        event.eat(4, 0.5);
                        event.add_exhaustion(1.5);
                    }
                    1 => event.hurt("minecraft:magic", 3.0),
                    3 if event.health > 1.0 => event.hurt("minecraft:magic", 1.0),
                    3 => {}
                    _ => event.hurt("minecraft:wither", 100.0),
                }
            }
        }
        mod_platform::__private::install(std::sync::Arc::from([mod_api::ModEntry::new(
            "test-potions",
            Box::new(TestMod),
        )]));
        assert!(brewing::register_mod_effects([(
            "test:effect".into(),
            0,
            brewing::Category::Neutral,
            None,
        )]));
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn mod_tick_heals_and_feeds_player() {
        // Mod effects are frozen process-wide: another fixture may have registered first.
        if super::super::native_entity_refs::tests::run_fresh_cases(
            "game::potions::tests::mod_tick_heals_and_feeds_player",
            "HORIZON_POTION_EFFECT_CHILD",
            &["effect"],
        ) {
            return;
        }
        effect_test_mod();
        super::super::native_entity_refs::tests::with_game(|game| {
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            let player = game.player.as_mut().unwrap();
            player.set_health(18.0, false).unwrap();
            player.health.food.food = 14;
            player.health.food.saturation = 2.0;
            let id = brewing::effect("test:effect").unwrap().id;
            game.potions.effects.add(EffectInstance::new(id, 10, 0));
            game.tick_mob_effects();
            let player = game.player.as_ref().unwrap();
            assert_eq!(player.health.health(), 20.0);
            assert_eq!(player.health.food.food, 18);
            assert_eq!(player.health.food.saturation, 6.0);
            assert_eq!(player.health.food.exhaustion, 1.5);
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn mod_tick_magic_respects_player_hit_frames() {
        // Mod effects are frozen process-wide: another fixture may have registered first.
        if super::super::native_entity_refs::tests::run_fresh_cases(
            "game::potions::tests::mod_tick_magic_respects_player_hit_frames",
            "HORIZON_POTION_EFFECT_CHILD",
            &["effect"],
        ) {
            return;
        }
        effect_test_mod();
        super::super::native_entity_refs::tests::with_game(|game| {
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            let id = brewing::effect("test:effect").unwrap().id;
            game.potions.effects.add(EffectInstance::new(id, 10, 1));
            game.tick_mob_effects();
            assert_eq!(game.player.as_ref().unwrap().health.health(), 17.0);
            game.tick_mob_effects();
            assert_eq!(game.player.as_ref().unwrap().health.health(), 17.0);
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn mod_tick_kills_mob_through_death_and_drops() {
        // Mod effects are frozen process-wide: another fixture may have registered first.
        if super::super::native_entity_refs::tests::run_fresh_cases(
            "game::potions::tests::mod_tick_kills_mob_through_death_and_drops",
            "HORIZON_POTION_EFFECT_CHILD",
            &["effect"],
        ) {
            return;
        }
        effect_test_mod();
        super::super::native_entity_refs::tests::with_game(|game| {
            // Death drops read the server's loot, which a world open loads.
            game.load_server_loot();
            use horizon_client::chunk::Chunk;
            use std::sync::Arc;
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            let mut mob = Mob::new(12, kind, Vec3::new(1.0, 64.0, 1.0), 0.0);
            mob.extra
                .set_main_hand(Some(Arc::from("minecraft:iron_sword")));
            mob.extra.drop_chances = Some(Box::new([2.0; 8]));
            mob.add_effect(brewing::effect("test:effect").unwrap().id, 10, 2);
            game.mobs.push(mob);
            game.tick_creature_entities().unwrap();
            assert!(!game.mobs[0].alive());
            assert!(
                game.dropped
                    .iter()
                    .any(|item| &*item.stack.item == "minecraft:iron_sword")
            );
            for _ in 0..20 {
                game.tick_creature_entities().unwrap();
            }
            assert!(game.mobs.is_empty());
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn mod_tick_sees_prior_regeneration() {
        // Mod effects are frozen process-wide: another fixture may have registered first.
        if super::super::native_entity_refs::tests::run_fresh_cases(
            "game::potions::tests::mod_tick_sees_prior_regeneration",
            "HORIZON_POTION_EFFECT_CHILD",
            &["effect"],
        ) {
            return;
        }
        effect_test_mod();
        super::super::native_entity_refs::tests::with_game(|game| {
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            game.player
                .as_mut()
                .unwrap()
                .set_health(1.0, false)
                .unwrap();
            game.potions
                .effects
                .add(EffectInstance::new("regeneration", 50, 0));
            let id = brewing::effect("test:effect").unwrap().id;
            game.potions.effects.add(EffectInstance::new(id, 10, 3));
            game.tick_mob_effects();
            assert_eq!(game.player.as_ref().unwrap().health.health(), 1.0);
        });
    }

    /// A mod effect's damage type keeps its registry id and tags whether it is
    /// vanilla's or a datapack's, and one the registry lacks is refused.
    #[test]
    fn effect_damage_sources_retain_their_ids() {
        let type_json = |id: &str| {
            serde_json::json!({
                "id": id, "message_id": "m", "exhaustion": "00000000",
                "scaling": "never", "effects": "hurt", "death_message_type": "default"
            })
        };
        let catalog = horizon_world::damage_source::Catalog::from_json(
            &serde_json::json!({
                "data_version": horizon_world::DATA_VERSION,
                "types": [type_json("minecraft:magic"), type_json("demo:scorch")],
            }),
            &serde_json::json!({
                "minecraft:is_fire": "{\"values\":[\"demo:scorch\"]}",
                "minecraft:bypasses_armor": "{\"values\":[\"demo:scorch\",\"minecraft:magic\"]}",
            }),
        )
        .unwrap();
        for (id, tags) in [
            ("minecraft:magic", NamedDamage::BYPASSES_ARMOR),
            (
                "demo:scorch",
                NamedDamage::IS_FIRE | NamedDamage::BYPASSES_ARMOR,
            ),
        ] {
            let named = NamedDamage::registered(&catalog, id).unwrap();
            assert_eq!((named.name, named.tags), (id, tags));
            let kind = Game::damage_kind(&catalog, id);
            assert_eq!(
                super::super::creatures::source_damage_type(Damage::of(kind)),
                id
            );
        }
        assert_eq!(NamedDamage::registered(&catalog, "demo:missing"), None);
    }

    /// `AreaEffectCloud.addAdditionalSaveData` with `readAdditionalSaveData`: a
    /// lingering cloud written at a save reads back with its lifetime, radius,
    /// use settings and the effects it sprays, with the hidden and ambient
    /// flags it carried.
    #[test]
    fn a_lingering_cloud_survives_a_save() {
        let cloud = Breath {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id: 7,
            at: [1.5, 64.0, -2.5],
            motion: [0.6, 0.0, 0.0],
            radius: 3.5,
            age: 12,
            duration: 400,
            from_dragon: false,
            victims: Vec::new(),
            potion: Some(PotionCloud {
                settings: CloudSettings {
                    wait_time: 10,
                    reapplication_delay: 25,
                    radius_on_use: -0.5,
                    duration_on_use: -5,
                    duration_scale: 0.5,
                },
                effects: vec![("poison", 200, 1), ("slowness", 100, 0)],
                hidden: vec!["slowness"],
                ambient: vec!["poison"],
                radius_per_tick: -0.008_750,
                color: 0x0033_FF44,
                custom_particle: None,
                victims: Vec::new(),
                source: Text::translate("entity.minecraft.area_effect_cloud", &[]),
                owner: None,
                guest: None,
            }),
        };
        let tag = potion_cloud_to_nbt(&cloud, [1, 2, 3, 4], None);
        let (at, back) = potion_cloud_from_nbt(&tag).expect("the cloud reads back");
        assert_eq!(at, Vec3::new(1.5, 64.0, -2.5), "the position");
        assert_eq!(back.motion, [0.6, 0.0, 0.0], "the motion");
        assert_eq!((back.age, back.duration), (12, 400));
        assert!((back.radius - 3.5).abs() < 1.0e-6, "{}", back.radius);
        let potion = back.potion.expect("a potion cloud");
        assert_eq!(
            (
                potion.settings.wait_time,
                potion.settings.reapplication_delay,
                potion.settings.duration_on_use
            ),
            (10, 25, -5)
        );
        assert!((potion.settings.radius_on_use + 0.5).abs() < 1.0e-6);
        assert!((potion.settings.duration_scale - 0.5).abs() < 1.0e-6);
        assert!((potion.radius_per_tick + 0.008_750).abs() < 1.0e-6);
        assert_eq!(
            potion.effects,
            vec![("poison", 200, 1), ("slowness", 100, 0)]
        );
        assert_eq!(potion.hidden, vec!["slowness"]);
        assert_eq!(potion.ambient, vec!["poison"]);
        assert_eq!(potion.color, 0x0033_FF44, "the colour it was drawn in");
        // The vanilla field names the reader of another client would look for.
        for key in [
            "Age",
            "Duration",
            "Radius",
            "RadiusPerTick",
            "WaitTime",
            "ReapplicationDelay",
            "RadiusOnUse",
            "DurationOnUse",
            "potion_duration_scale",
            "potion_contents",
        ] {
            assert!(tag.get(key).is_some(), "{key}");
        }
        assert_eq!(
            horizon_client::entity_nbt::kind(&tag).as_deref(),
            Some("minecraft:area_effect_cloud")
        );
    }

    /// `WitherBoss.canBeAffected` refuses only the wither effect: its `addEffect`
    /// refuses the rest, but a cloud of strength or healing still counts the boss
    /// as a victim (`AreaEffectCloud.serverTick` asks `canBeAffected`).
    #[test]
    fn a_cloud_still_counts_the_wither_as_a_victim() {
        let tags = std::collections::HashMap::new();
        for effect in ["strength", "instant_health"] {
            assert!(
                mob_can_be_affected(&tags, "minecraft:wither", effect),
                "{effect}"
            );
        }
        assert!(!mob_can_be_affected(&tags, "minecraft:wither", "wither"));
    }

    /// #5876: a potion thrown straight up from inside its thrower's box does
    /// not hit them while it climbs out, then hits them on the way down;
    /// another player in its path is hit at once.
    #[test]
    fn a_thrown_potion_hits_its_thrower_only_after_leaving_them() {
        // A player's box: 0.6 wide, 1.8 high, feet at the origin.
        let body = |x: f64| Aabb::new(Vec3::new(x - 0.3, 0.0, -0.3), Vec3::new(x + 0.3, 1.8, 0.3));
        let thrower = Who::Mob(1);
        let players = [(thrower, body(0.0)), (Who::Mob(2), body(5.0))];
        let ridden = [body(0.0)];
        let eye = Vec3::new(0.0, 1.52, 0.0);
        let up = Vec3::new(0.0, 0.5, 0.0);
        let m = thrown_margin(10);
        assert!((m - 0.3).abs() < 1e-12 && thrown_margin(1) == 0.0);
        // Thrown from the eye: still within a block of its thrower.
        assert!(!thrown_left_owner(&ridden, eye, up));
        assert!(thrown_player_at(&players, &[thrower], eye, m).is_none());
        // Without the gate the thrower's box does hold it.
        assert_eq!(
            thrown_player_at(&players, &[], eye, m).map(|t| t.0),
            Some(thrower)
        );
        // Four blocks up and rising: clear of the box grown by a block.
        let high = Vec3::new(0.0, 5.0, 0.0);
        assert!(thrown_left_owner(&ridden, high, up));
        // Falling back (left_owner now set), the thrower is hit at their head.
        let head = Vec3::new(0.0, 2.0, 0.0);
        assert_eq!(
            thrown_player_at(&players, &[], head, m).map(|t| t.0),
            Some(thrower)
        );
        // Another player is hit though the thrower has not left; a miss is none.
        let beside = Vec3::new(5.2, 1.0, 0.0);
        assert_eq!(
            thrown_player_at(&players, &[thrower], beside, m).map(|t| t.0),
            Some(Who::Mob(2))
        );
        assert!(thrown_player_at(&players, &[thrower], Vec3::new(2.5, 1.0, 0.0), m).is_none());
        // A passenger sharing the thrower's vehicle is skipped like the thrower.
        assert!(thrown_player_at(&players, &[thrower, Who::Mob(2)], beside, m).is_none());
        // The vehicle's box keeps it from leaving: a mount 2 blocks wide.
        let mount = Aabb::new(Vec3::new(-1.0, -1.0, -1.0), Vec3::new(1.0, 1.0, 1.0));
        let beside_mount = Vec3::new(1.9, 0.0, 0.0);
        assert!(thrown_left_owner(&[body(0.0)], beside_mount, up));
        assert!(!thrown_left_owner(&[body(0.0), mount], beside_mount, up));
        // No thrower (a dispenser's potion): it has left them already.
        assert!(thrown_left_owner(&[], eye, up));
    }

    /// A server's client (multiplayer and the default single player) fades
    /// the darkness effect in over its 22 blend ticks, holds it, and fades
    /// it out over the last 22 (#3393).
    #[test]
    fn a_servers_client_blends_the_darkness_effect() {
        let mut effects = PlayerEffects::default();
        effects.add(EffectInstance::new("darkness", 60, 0));
        let mut darkness = (0.0, 0.0);
        client_tick_effects(&mut effects, &mut darkness);
        assert_eq!(darkness, (0.0, 1.0 / 22.0), "the first tick fades in");
        for _ in 1..22 {
            client_tick_effects(&mut effects, &mut darkness);
        }
        assert!((darkness.1 - 1.0).abs() < 1e-5, "{darkness:?}");
        // 38 ticks left: full until 22 remain, then the fade out.
        for _ in 0..16 {
            client_tick_effects(&mut effects, &mut darkness);
        }
        assert_eq!(effects.get("darkness").map(|e| e.duration), Some(22));
        assert!(
            darkness.1 < darkness.0,
            "fading out at 22 left: {darkness:?}"
        );
        for _ in 0..21 {
            client_tick_effects(&mut effects, &mut darkness);
        }
        assert!(darkness.1.abs() < 1e-5, "{darkness:?}");
    }

    /// `ClientPacketListener.handleUpdateMobEffect`: a new darkness fades in
    /// when the server blends it; one sent unblended (joining, changing
    /// dimension) is at full strength at once.
    #[test]
    fn a_new_darkness_starts_blended_or_at_once() {
        let e = EffectInstance::new("darkness", 260, 0);
        assert_eq!(darkness_blend_added(&e, true), (0.0, 0.0));
        assert_eq!(darkness_blend_added(&e, false), (1.0, 1.0));
        let ending = EffectInstance::new("darkness", 10, 0);
        assert_eq!(darkness_blend_added(&ending, false), (0.0, 0.0));
        let infinite = EffectInstance::new("darkness", -1, 0);
        assert_eq!(darkness_blend_added(&infinite, false), (1.0, 1.0));
    }

    /// `ClientPacketListener.handleUpdateMobEffect`: a new nausea fades in
    /// when the server blends it; one sent unblended (joining, changing
    /// dimension) is at full strength at once, unless it is already within
    /// its blend-out ticks.
    #[test]
    fn a_new_nausea_starts_blended_or_at_once() {
        let e = EffectInstance::new("nausea", 600, 0);
        assert_eq!(nausea_blend_added(&e, true), (0.0, 0.0));
        assert_eq!(nausea_blend_added(&e, false), (1.0, 1.0));
        let ending = EffectInstance::new("nausea", 40, 0);
        assert_eq!(nausea_blend_added(&ending, false), (0.0, 0.0));
        let infinite = EffectInstance::new("nausea", -1, 0);
        assert_eq!(nausea_blend_added(&infinite, false), (1.0, 1.0));
    }

    /// `BottleItem.use`: collecting dragon's breath from an ender dragon's cloud (#8054).
    #[test]
    fn glass_bottle_collects_dragon_breath_from_dragon_cloud() {
        let player_box = Aabb::new(Vec3::new(-0.3, 64.0, -0.3), Vec3::new(0.3, 65.8, 0.3));
        let mut clouds = [super::super::dragon::Breath {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id: 1,
            at: [1.0, 64.0, 0.0],
            motion: [0.0; 3],
            radius: 5.0,
            age: 0,
            duration: 200,
            from_dragon: true,
            victims: Vec::new(),
            potion: None,
        }]
        .into_iter()
        .collect();
        assert!(collect_dragon_breath(player_box, &mut clouds).is_some());
        assert_eq!(clouds[0].radius, 4.5);
        assert!(collect_dragon_breath(player_box, &mut clouds).is_some());
        assert_eq!(clouds[0].radius, 4.0);
    }

    /// `BottleItem.use`: clouds that are expired, non-dragon, lingering potions,
    /// or out of reach cannot be bottled (#8054).
    #[test]
    fn glass_bottle_ignores_non_dragon_or_expired_or_out_of_range_clouds() {
        let player_box = Aabb::new(Vec3::new(-0.3, 64.0, -0.3), Vec3::new(0.3, 65.8, 0.3));
        // Non-dragon cloud (`from_dragon: false`).
        let mut non_dragon = [super::super::dragon::Breath {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id: 1,
            at: [0.0, 64.0, 0.0],
            motion: [0.0; 3],
            radius: 3.0,
            age: 0,
            duration: 600,
            from_dragon: false,
            victims: Vec::new(),
            potion: None,
        }]
        .into_iter()
        .collect();
        assert!(collect_dragon_breath(player_box, &mut non_dragon).is_none());
        assert_eq!(non_dragon[0].radius, 3.0);

        // Lingering potion cloud (`potion: Some(...)`).
        let mut lingering = [super::super::dragon::Breath {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id: 2,
            at: [0.0, 64.0, 0.0],
            motion: [0.0; 3],
            radius: 3.0,
            age: 0,
            duration: 600,
            from_dragon: true,
            victims: Vec::new(),
            potion: Some(PotionCloud {
                settings: CloudSettings::lingering(),
                effects: Vec::new(),
                hidden: Vec::new(),
                ambient: Vec::new(),
                radius_per_tick: -3.0 / 600.0,
                color: 0,
                custom_particle: None,
                victims: Vec::new(),
                source: Text::empty(),
                owner: None,
                guest: None,
            }),
        }]
        .into_iter()
        .collect();
        assert!(collect_dragon_breath(player_box, &mut lingering).is_none());
        assert_eq!(lingering[0].radius, 3.0);

        // Its last tick before the wait-shifted end still holds breath
        // (`tickCount - waitTime < duration`).
        let mut last = [super::super::dragon::Breath {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id: 3,
            at: [0.0, 64.0, 0.0],
            motion: [0.0; 3],
            radius: 5.0,
            age: 219,
            duration: 200,
            from_dragon: true,
            victims: Vec::new(),
            potion: None,
        }]
        .into_iter()
        .collect();
        assert!(collect_dragon_breath(player_box, &mut last).is_some());
        assert_eq!(last[0].radius, 4.5);

        // Expired cloud (`tickCount - waitTime >= duration`).
        let mut expired = [super::super::dragon::Breath {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id: 3,
            at: [0.0, 64.0, 0.0],
            motion: [0.0; 3],
            radius: 5.0,
            age: 220,
            duration: 200,
            from_dragon: true,
            victims: Vec::new(),
            potion: None,
        }]
        .into_iter()
        .collect();
        assert!(collect_dragon_breath(player_box, &mut expired).is_none());
        assert_eq!(expired[0].radius, 5.0);

        // Out of horizontal reach: cloud at 10 blocks away with radius 1:
        // cloud X range [9.0, 11.0], player inflated X range [-2.3, 2.3] -> no overlap.
        let mut far = [super::super::dragon::Breath {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id: 4,
            at: [10.0, 64.0, 0.0],
            motion: [0.0; 3],
            radius: 1.0,
            age: 0,
            duration: 200,
            from_dragon: true,
            victims: Vec::new(),
            potion: None,
        }]
        .into_iter()
        .collect();
        assert!(collect_dragon_breath(player_box, &mut far).is_none());
        assert_eq!(far[0].radius, 1.0);

        // Out of vertical reach: cloud at Y=64.0 (Y bounds [64.0, 64.5]),
        // player at Y=67.0 (inflated Y bounds [65.0, 70.8] does not reach 64.5).
        let high_player = Aabb::new(Vec3::new(-0.3, 67.0, -0.3), Vec3::new(0.3, 68.8, 0.3));
        let mut ground_cloud = [super::super::dragon::Breath {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id: 5,
            at: [0.0, 64.0, 0.0],
            motion: [0.0; 3],
            radius: 1.0,
            age: 0,
            duration: 200,
            from_dragon: true,
            victims: Vec::new(),
            potion: None,
        }]
        .into_iter()
        .collect();
        assert!(collect_dragon_breath(high_player, &mut ground_cloud).is_none());
        assert_eq!(ground_cloud[0].radius, 1.0);
    }

    /// Real panel painting/hover selection dispatches even for a fully visible
    /// widget. Both inventories retain vanilla lines, suppression and layout rules.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_mod_adds_a_line_to_an_effect_tooltip() {
        use mod_api::{self, ClientHost, GatherEffectScreenTooltipsEvent, Hooks, ModEntry, Mods};
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };

        #[derive(Clone, Copy)]
        struct Case {
            room: i32,
            id: &'static str,
            name: &'static str,
            default_tooltip: bool,
        }
        #[derive(Clone)]
        struct Mod {
            screen: &'static str,
            case: Case,
            suppress: bool,
            calls: Arc<AtomicUsize>,
        }
        #[mod_api::__subscriptions]
        impl Hooks for Mod {
            fn gather_effect_screen_tooltips(
                &self,
                _: &dyn ClientHost,
                event: &mut GatherEffectScreenTooltipsEvent<'_>,
            ) {
                self.calls.fetch_add(1, Ordering::Relaxed);
                assert_eq!(
                    event.effect.id,
                    mod_platform::__private::part_tests::effect_key(self.case.id)
                );
                assert_eq!((event.effect.duration, event.effect.amplifier), (450, 1));
                assert_eq!(event.screen.id, self.screen);
                let initial: Vec<_> = event
                    .tooltip
                    .iter()
                    .map(|(text, color)| (text.clone(), *color))
                    .collect();
                if self.case.default_tooltip {
                    assert_eq!(
                        initial,
                        [
                            (mod_api::Text::literal(self.case.name), u32::MAX),
                            (mod_api::Text::literal("00:22"), u32::MAX)
                        ]
                    );
                } else {
                    assert!(initial.is_empty());
                }
                if self.suppress {
                    event.tooltip.clear();
                } else {
                    event
                        .tooltip
                        .push((mod_api::Text::literal("Fast feet"), 0xff55ffff));
                }
            }
        }
        struct HidePanels;
        #[mod_api::__subscriptions]
        impl Hooks for HidePanels {
            fn screen_render_inventory_mob_effects(
                &self,
                _: &dyn ClientHost,
                _: &mut mod_api::ScreenRenderInventoryMobEffectsEvent<'_>,
            ) -> mod_api::Flow {
                mod_api::Flow::Cancel
            }
        }
        super::super::native_entity_refs::tests::with_game(|game| {
            for (screen, screen_id, image_width) in [
                (Screen::Inventory, "inventory", 176),
                (Screen::Creative, "creative", 195),
            ] {
                game.screen = screen;
                // Fixed panel position; right-side room selects wide or compact.
                // The longer effect name also clips in the smallest wide layout.
                for case in [
                    Case {
                        room: 200,
                        id: "speed",
                        name: "Speed II",
                        default_tooltip: false,
                    },
                    Case {
                        room: 100,
                        id: "speed",
                        name: "Speed II",
                        default_tooltip: true,
                    },
                    Case {
                        room: 120,
                        id: "fire_resistance",
                        name: "Fire Resistance II",
                        default_tooltip: true,
                    },
                ] {
                    game.potions.effects.list = vec![EffectInstance::new(case.id, 450, 1)];
                    let left = 102 + image_width;
                    let origin = (100.0, 50.0);
                    let mut batch = HudBatch::scratch([(left + case.room) as u32, 480]);
                    game.cursor = ((left + 7) as f32, 57.0);
                    let vanilla = if case.default_tooltip {
                        vec![
                            (case.name.to_owned(), u32::MAX),
                            ("00:22".to_owned(), u32::MAX),
                        ]
                    } else {
                        Vec::new()
                    };
                    let lines = game
                        .draw_inventory_effects(&mut batch, origin, image_width)
                        .unwrap();
                    assert_eq!(
                        lines
                            .iter()
                            .map(|(text, color)| (text.plain(), *color))
                            .collect::<Vec<_>>(),
                        vanilla
                    );
                    // The existing shared renderer draws nothing for an unchanged
                    // empty list, without running other tooltip hooks.
                    let painted = batch.vertices.len();
                    game.draw_tooltip_lines(&mut batch, &lines, game.cursor.0, game.cursor.1);
                    assert_eq!(batch.vertices.len() > painted, case.default_tooltip);

                    for suppress in [false, true] {
                        let calls = Arc::new(AtomicUsize::new(0));
                        let listener = Mod {
                            screen: screen_id,
                            case,
                            suppress,
                            calls: calls.clone(),
                        };
                        let mods: Mods =
                            vec![ModEntry::new("effect-tips", Box::new(listener.clone()))].into();
                        mod_platform::testing::with_mods(mods, || {
                            let lines = game
                                .draw_inventory_effects(&mut batch, origin, image_width)
                                .unwrap();
                            let plain: Vec<_> = lines
                                .iter()
                                .map(|(text, color)| (text.plain(), *color))
                                .collect();
                            let mut expected = vanilla.clone();
                            if suppress {
                                expected.clear();
                            } else {
                                expected.push(("Fast feet".to_owned(), 0xff55ffff));
                            }
                            assert_eq!(plain, expected);
                            assert_eq!(calls.load(Ordering::Relaxed), 1);
                            let painted = batch.vertices.len();
                            game.draw_tooltip_lines(
                                &mut batch,
                                &lines,
                                game.cursor.0,
                                game.cursor.1,
                            );
                            assert_eq!(batch.vertices.len() > painted, !suppress);

                            // A miss, a hidden layout and too little room must
                            // never gather tooltips, even with a subscriber.
                            game.cursor = (0.0, 0.0);
                            assert!(
                                game.draw_inventory_effects(&mut batch, origin, image_width)
                                    .is_none()
                            );
                            game.cursor = ((left + 7) as f32, 57.0);
                            let hidden: Mods = vec![
                                ModEntry::new("effect-tips", Box::new(listener.clone())),
                                ModEntry::new("hide-panels", Box::new(HidePanels)),
                            ]
                            .into();
                            mod_platform::testing::with_mods(hidden, || {
                                // Layout hooks run at screen construction, not painting.
                                game.frames += 1;
                                let space = Game::effects_space(
                                    &batch,
                                    [origin.0, origin.1],
                                    image_width as f32,
                                );
                                let layout = game.inventory_effects_layout(
                                    &space,
                                    [origin.0, origin.1, image_width as f32, 166.0],
                                );
                                let panels = game.effect_panels(&space, &layout);
                                assert!(
                                    inventory_effects::paint(
                                        &mut batch,
                                        &game.gui.atlas,
                                        &panels,
                                        [game.cursor.0 as i32, game.cursor.1 as i32],
                                    )
                                    .is_none()
                                );
                            });
                            let mut narrow = HudBatch::scratch([(left + 31) as u32, 480]);
                            assert!(
                                game.draw_inventory_effects(&mut narrow, origin, image_width)
                                    .is_none()
                            );
                            assert_eq!(calls.load(Ordering::Relaxed), 1);
                        });
                    }
                }
            }
        });
    }
}

/// `MobEffect.withSoundOnAdded`: the sound `onEffectAdded` plays when the
/// effect is new on an entity (`MobEffects`: the three omens).
pub(super) fn sound_on_added(effect: &str) -> Option<&'static str> {
    match effect.strip_prefix("minecraft:").unwrap_or(effect) {
        "bad_omen" => Some("minecraft:event.mob_effect.bad_omen"),
        "trial_omen" => Some("minecraft:event.mob_effect.trial_omen"),
        "raid_omen" => Some("minecraft:event.mob_effect.raid_omen"),
        _ => None,
    }
}

#[cfg(test)]
mod sound_on_added_tests {
    use horizon_client::effects::EffectInstance;

    /// `LivingEntity.addEffect` → `MobEffect.onEffectAdded`: Bad Omen, Trial
    /// Omen and Raid Omen sound once when they are new on the player, at its
    /// feet; a renewal and any other effect are silent (#11373).
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_new_omen_sounds_once_at_the_feet() {
        super::super::native_entity_refs::tests::with_game(|game| {
            game.player = Some(
                horizon_client::motion::PlayerMotion::new(
                    game.motion,
                    horizon_core::geometry::Vec3::new(10.5, 64.0, -3.25),
                )
                .unwrap(),
            );
            game.audio.capture = Some(Vec::new());
            for (id, amplifier) in [
                ("bad_omen", 0),
                ("bad_omen", 1),
                ("speed", 0),
                ("trial_omen", 0),
                ("raid_omen", 0),
            ] {
                game.add_player_effect(EffectInstance::new(id, 2400, amplifier));
            }
            let played: Vec<(String, [f64; 3])> = game
                .audio
                .capture
                .take()
                .unwrap()
                .into_iter()
                .map(|s| (s.event, s.position))
                .collect();
            let feet = [10.5, 64.0, -3.25];
            assert_eq!(
                played,
                [
                    ("minecraft:event.mob_effect.bad_omen".to_owned(), feet),
                    ("minecraft:event.mob_effect.trial_omen".to_owned(), feet),
                    ("minecraft:event.mob_effect.raid_omen".to_owned(), feet),
                ]
            );
        });
    }
}

#[cfg(test)]
mod xp_bottle_tests {
    use super::*;

    /// `ThrownExperienceBottle.onHit` uses `awardWithDirection`: every fresh
    /// orb starts 0.25 out from the face it hit, or back along the bottle's
    /// motion after an entity hit, moving that way (#11388).
    #[test]
    #[ignore = "needs prepared resources (HORIZON_TEST_RESOURCES)"]
    fn bottle_orbs_start_away_from_what_it_hit() {
        super::super::native_entity_refs::tests::with_game(|game| {
            let bottle = ItemStack::new("minecraft:experience_bottle", 1);
            let at = Vec3::new(8.5, 70.5, 8.5);
            // (hit face, the bottle's motion, the axis and sign the orbs leave by)
            for (face, motion, axis, sign) in [
                (
                    Some(([9, 70, 8], Direction::West)),
                    Vec3::new(1.0, 0.0, 0.0),
                    0,
                    -1.0,
                ),
                (None, Vec3::new(0.0, 0.0, 1.0), 2, -1.0),
            ] {
                game.orbs.clear();
                game.splash(
                    &bottle,
                    at,
                    None,
                    face,
                    0,
                    &PotionOwner::default(),
                    false,
                    motion,
                );
                assert!(!game.orbs.is_empty());
                let at = [at.x, at.y, at.z];
                for o in game.orbs.iter() {
                    let p = [o.position.x, o.position.y, o.position.z];
                    let v = [o.velocity.x, o.velocity.y, o.velocity.z];
                    assert!((p[axis] - (at[axis] + sign * 0.25)).abs() < 1e-9, "{p:?}");
                    assert!(v[axis] * sign >= 0.0, "{v:?}");
                }
            }
        });
    }
}

#[cfg(test)]
mod mob_absorption_max_tests {
    use crate::game::{Audio, Game, Options, Progress, Settings, load_resources};
    use horizon_client::{chunk::Chunk, motion::PlayerMotion};
    use horizon_core::{geometry::Vec3, position::ChunkPos};
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
                .join(format!("h203-{:?}", std::thread::current().id())),
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
        assert!(game.run_command("forceload add 0 0"));
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(2.5, 200.0, 2.5)).unwrap());
        game.creative = true;
        assert!(game.run_command("scoreboard objectives add h203 dummy"));
        game.load_ticking_neighbourhood(0, 0);
        test(&mut game);
    }
    fn pig(game: &Game<'_>, n: i32) -> usize {
        let i: Vec<usize> = (0..game.mobs.len())
            .filter(|&i| game.persist.uuid_of(game.mobs[i].id) == Some([203, 0, 0, n]))
            .collect();
        assert_eq!(i.len(), 1, "one live pig with UUID {n}");
        i[0]
    }
    /// `/attribute ... max_absorption get`, read back through a score.
    fn max_absorption(game: &mut Game<'_>, tag: &str) -> i32 {
        assert!(game.run_command(&format!(
            "execute store result score #max h203 run attribute @e[tag={tag},limit=1] minecraft:max_absorption get"
        )));
        (0..=64)
            .find(|v| game.run_command_quietly(&format!("execute if score #max h203 matches {v}")))
            .unwrap_or(-1)
    }

    /// Expected values from the official 26.3 server: Absorption adds
    /// 4 * (amplifier + 1) to max_absorption; its add raises the amount to at
    /// least that much within the maximum; when it goes, the amount is
    /// clamped to the maximum left (LivingEntity.onAttributeUpdated), so a
    /// base-20 mob keeps it and a base-0 one loses it.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES and a scratch HORIZON_HOME"]
    fn absorption_follows_the_max_absorption_it_adds_and_leaves() {
        with_game(|game| {
            // name, UUID word, base max_absorption, amplifier, duration seconds
            let cases = [
                ("b0_clear", 1, None, 1, 100),
                ("b0_expiry", 2, None, 0, 1),
                ("b20_clear", 3, Some(20), 1, 100),
                ("b20_expiry", 4, Some(20), 0, 1),
                ("b20_upgrade", 5, Some(20), 0, 100),
            ];
            for (name, n, base, amp, secs) in cases {
                let tag = format!("h203_{name}");
                assert!(game.run_command(&format!(
                    "summon minecraft:pig {}.5 200 4.5 {{UUID:[I;203,0,0,{n}],Tags:[\"{tag}\"],PersistenceRequired:1b,Silent:1b,NoAI:1b,Health:10f}}",
                    2 * n
                )));
                if let Some(b) = base {
                    assert!(game.run_command(&format!(
                        "attribute @e[tag={tag},limit=1] minecraft:max_absorption base set {b}"
                    )));
                }
                assert!(game.run_command(&format!(
                    "effect give @e[tag={tag},limit=1] minecraft:absorption {secs} {amp}"
                )));
            }
            assert!(game.run_command(
                "effect give @e[tag=h203_b20_upgrade,limit=1] minecraft:absorption 100 1"
            ));
            let mut failures = Vec::new();
            let mut expect =
                |game: &mut Game<'_>, phase: &str, rows: [(&str, i32, f32, i32); 5]| {
                    for (name, n, amount, max) in rows {
                        let i = pig(game, n);
                        if game.mobs[i].absorption != amount {
                            failures.push(format!(
                                "{phase} {name}: amount {}, expected {amount}",
                                game.mobs[i].absorption
                            ));
                        }
                        let got = max_absorption(game, &format!("h203_{name}"));
                        if got != max {
                            failures.push(format!(
                                "{phase} {name}: max_absorption {got}, expected {max}"
                            ));
                        }
                    }
                };
            expect(
                game,
                "added",
                [
                    ("b0_clear", 1, 8.0, 8),
                    ("b0_expiry", 2, 4.0, 4),
                    ("b20_clear", 3, 8.0, 28),
                    ("b20_expiry", 4, 4.0, 24),
                    ("b20_upgrade", 5, 8.0, 28),
                ],
            );
            for tag in ["h203_b0_clear", "h203_b20_clear"] {
                assert!(game.run_command(&format!(
                    "effect clear @e[tag={tag},limit=1] minecraft:absorption"
                )));
            }
            expect(
                game,
                "cleared",
                [
                    ("b0_clear", 1, 0.0, 0),
                    ("b0_expiry", 2, 4.0, 4),
                    ("b20_clear", 3, 8.0, 20),
                    ("b20_expiry", 4, 4.0, 24),
                    ("b20_upgrade", 5, 8.0, 28),
                ],
            );
            for _ in 0..25 {
                game.tick_world(true).unwrap();
            }
            expect(
                game,
                "expired",
                [
                    ("b0_clear", 1, 0.0, 0),
                    ("b0_expiry", 2, 0.0, 0),
                    ("b20_clear", 3, 8.0, 20),
                    ("b20_expiry", 4, 4.0, 20),
                    ("b20_upgrade", 5, 8.0, 28),
                ],
            );
            assert!(failures.is_empty(), "mob absorption maximum: {failures:#?}");
        });
    }
}

#[cfg(test)]
mod mob_health_boost_tests {
    use crate::game::{Audio, Game, Options, Progress, Settings, load_resources};
    use horizon_client::{chunk::Chunk, motion::PlayerMotion};
    use horizon_core::{geometry::Vec3, position::ChunkPos};
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
                .join(format!("h205-{:?}", std::thread::current().id())),
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
        assert!(game.run_command("forceload add 0 0"));
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(2.5, 200.0, 2.5)).unwrap());
        game.creative = true;
        game.load_ticking_neighbourhood(0, 0);
        test(&mut game);
    }
    fn pig(game: &Game<'_>, n: i32) -> usize {
        let i: Vec<usize> = (0..game.mobs.len())
            .filter(|&i| game.persist.uuid_of(game.mobs[i].id) == Some([205, 0, 0, n]))
            .collect();
        assert_eq!(i.len(), 1, "one live pig with UUID {n}");
        i[0]
    }
    fn health(game: &Game<'_>, n: i32) -> f32 {
        game.mobs[pig(game, n)].health
    }
    fn boost(game: &Game<'_>, n: i32) -> Option<(i32, i32)> {
        let i = pig(game, n);
        game.mobs[i]
            .extra
            .effects
            .iter()
            .find(|e| e.id == "minecraft:health_boost")
            .map(|e| (e.duration, e.amplifier))
    }

    /// Expected values from the official 26.3 server: Health Boost II adds 8
    /// to max_health without healing; Instant Health heals up to it on the next
    /// tick; removing the effect (`/effect clear`, or running out) clamps
    /// Health to the maximum without it (LivingEntity.onAttributeUpdated).
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES and a scratch HORIZON_HOME"]
    fn health_boost_heals_to_its_maximum_and_clamps_when_it_goes() {
        with_game(|game| {
            // name, UUID word, Health, setup
            let cases: [(&str, i32, f32, &[&str]); 5] = [
                (
                    "boost",
                    1,
                    10.0,
                    &["effect give @e[tag=T,limit=1] minecraft:health_boost 100 1"],
                ),
                (
                    "expiry",
                    2,
                    10.0,
                    &["effect give @e[tag=T,limit=1] minecraft:health_boost 2 1"],
                ),
                ("noboost", 3, 10.0, &[]),
                ("heal", 5, 5.0, &[]),
                (
                    "base",
                    6,
                    10.0,
                    &["attribute @e[tag=T,limit=1] minecraft:max_health base set 14"],
                ),
            ];
            for (name, n, h, setup) in cases {
                let tag = format!("h205_{name}");
                assert!(game.run_command(&format!(
                    "summon minecraft:pig {}.5 200 4.5 {{UUID:[I;205,0,0,{n}],Tags:[\"{tag}\"],PersistenceRequired:1b,Silent:1b,NoAI:1b,Health:{h}f}}",
                    2 * n
                )));
                for c in setup {
                    assert!(game.run_command(&c.replace('T', &tag)), "{name}: {c}");
                }
            }
            // Setup controls: the effect is there, nothing healed on the add.
            assert_eq!(boost(game, 1), Some((2000, 1)));
            assert_eq!(boost(game, 2), Some((40, 1)));
            for (_, n, h, _) in cases {
                assert_eq!(health(game, n), h, "UUID {n}: no heal on the add");
            }
            for (name, n, _, _) in cases {
                let level = if name == "heal" { 0 } else { 1 };
                assert!(game.run_command(&format!(
                    "effect give @e[tag=h205_{name},limit=1] minecraft:instant_health 1 {level}"
                )));
                let _ = n;
            }
            game.tick_world(true).unwrap();
            // The one tick: the boosts counted down once.
            assert_eq!(boost(game, 1), Some((1999, 1)), "the mobs ticked once");
            let mut failures = Vec::new();
            let check = |game: &Game<'_>,
                         failures: &mut Vec<String>,
                         phase: &str,
                         expected: [(i32, f32); 5]| {
                for (n, h) in expected {
                    if health(game, n) != h {
                        failures.push(format!(
                            "{phase}: UUID {n} Health {}, expected {h}",
                            health(game, n)
                        ));
                    }
                }
            };
            check(
                game,
                &mut failures,
                "healed",
                [(1, 18.0), (2, 18.0), (3, 10.0), (5, 9.0), (6, 14.0)],
            );
            assert!(
                game.run_command("effect clear @e[tag=h205_boost,limit=1] minecraft:health_boost")
            );
            check(
                game,
                &mut failures,
                "cleared",
                [(1, 10.0), (2, 18.0), (3, 10.0), (5, 9.0), (6, 14.0)],
            );
            for _ in 0..40 {
                game.tick_world(true).unwrap();
            }
            if boost(game, 2).is_some() {
                failures.push("expiry: the effect did not run out in 40 ticks".into());
            }
            check(
                game,
                &mut failures,
                "expired",
                [(1, 10.0), (2, 10.0), (3, 10.0), (5, 9.0), (6, 14.0)],
            );
            assert!(failures.is_empty(), "mob health boost: {failures:#?}");
        });
    }

    /// Sulfur cube sizing writes an attribute base (`set_size` -> `set_base`),
    /// so Health Boost adds to that base without healing, as in the command
    /// parity corpus. Taming still owns the wolf's cached maximum outside the
    /// model; that maximum and its current health remain unchanged.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES and a scratch HORIZON_HOME"]
    fn health_boost_adds_to_size_attributes_and_keeps_the_tame_maximum() {
        with_game(|game| {
            // tag, UUID word, summon, amplifier
            let cases = [
                // size 3: attribute base and cache 12; Health Boost II adds 8
                ("h205_cube", 20, "sulfur_cube 4.5 200 8.5 {Size:2", 1),
                // size 5: attribute base and cache 20; Health Boost II adds 8
                ("h205_cube_equal", 22, "sulfur_cube 6.5 200 8.5 {Size:4", 1),
                // tame: cache 40, model 8; Health Boost II
                ("h205_wolf", 21, "wolf 8.5 200 8.5 {", 1),
                // tame: Health Boost VIII (+32) makes the model 40 = the cache
                ("h205_wolf_equal", 23, "wolf 10.5 200 8.5 {", 7),
            ];
            let mut ids = Vec::new();
            for (tag, n, summon, _) in cases {
                let sep = if summon.ends_with('{') { "" } else { "," };
                assert!(game.run_command(&format!(
                    "summon minecraft:{summon}{sep}UUID:[I;205,0,0,{n}],Tags:[\"{tag}\"],NoAI:1b,PersistenceRequired:1b,Silent:1b}}"
                )));
                let i = pig(game, n);
                if tag.starts_with("h205_wolf") {
                    horizon_client::mobs::tame::set_tame(&mut game.mobs[i], true);
                }
                ids.push(i);
            }
            let state = |game: &Game<'_>| -> Vec<(f32, f32)> {
                ids.iter()
                    .map(|&i| (game.mobs[i].attrs.max_health, game.mobs[i].health))
                    .collect()
            };
            let before = state(game);
            assert_eq!(
                before.iter().map(|s| s.0).collect::<Vec<_>>(),
                [12.0, 20.0, 40.0, 40.0]
            );
            assert_eq!(
                horizon_client::mobs::finalize::max_health(&game.mobs[ids[1]]),
                20.0,
                "sizing installs the cube's actual max-health attribute base"
            );
            for (tag, _, _, amp) in cases {
                assert!(game.run_command(&format!(
                    "effect give @e[tag={tag},limit=1] minecraft:health_boost 100 {amp}"
                )));
            }
            assert_eq!(
                horizon_client::mobs::finalize::max_health(&game.mobs[ids[3]]),
                40.0,
                "the coincidence: the boosted model equals the tame cache"
            );
            assert_eq!(
                state(game),
                [
                    (20.0, before[0].1),
                    (28.0, before[1].1),
                    (40.0, before[2].1),
                    (40.0, before[3].1),
                ],
                "size attributes gain the modifier without healing; tame caches keep their owner"
            );
            for (tag, _, _, _) in cases {
                assert!(game.run_command(&format!(
                    "effect clear @e[tag={tag},limit=1] minecraft:health_boost"
                )));
            }
            assert_eq!(state(game), before, "and after the clear");
        });
    }
}

#[cfg(test)]
mod depleted_absorption_tests {
    use crate::game::{Audio, Game, Options, Progress, Settings, commands::Target, load_resources};
    use horizon_client::{chunk::Chunk, motion::PlayerMotion};
    use horizon_core::{geometry::Vec3, position::ChunkPos};
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
                .join(format!("h204-{:?}", std::thread::current().id())),
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
        assert!(game.run_command("forceload add 0 0"));
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(2.5, 200.0, 2.5)).unwrap());
        game.creative = true;
        game.load_ticking_neighbourhood(0, 0);
        test(&mut game);
    }
    /// The mob in the level with UUID word `n`.
    fn pig(game: &Game<'_>, n: i32) -> usize {
        let i: Vec<usize> = (0..game.mobs.len())
            .filter(|&i| game.persist.uuid_of(game.mobs[i].id) == Some([204, 0, 0, n]))
            .collect();
        assert_eq!(i.len(), 1, "one live pig with UUID {n}");
        i[0]
    }
    /// Absorption's remaining ticks on the mob, if it has the effect.
    fn absorption(game: &Game<'_>, i: usize) -> Option<i32> {
        game.mobs[i]
            .extra
            .effects
            .iter()
            .find(|e| e.id == "minecraft:absorption")
            .map(|e| e.duration)
    }

    /// Expected values from the official 26.3 server:
    /// AbsorptionMobEffect.applyEffectTick is false once the amount is 0, so
    /// MobEffectInstance.tickServer drops the effect on the mob's next tick,
    /// before the duration counts down; with any amount left it stays.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES and a scratch HORIZON_HOME"]
    fn a_mob_loses_absorption_on_the_tick_after_it_is_used_up() {
        with_game(|game| {
            // name, UUID word, extra NBT, effect, hit
            let cases = [
                ("depleted", 1, "NoAI:1b", true, Some(4)),
                ("kept", 2, "NoAI:1b", true, None),
                ("partial", 3, "NoAI:1b", true, Some(2)),
                ("none", 4, "NoAI:1b", false, Some(4)),
                ("refused", 5, "NoAI:1b,Invulnerable:1b", true, Some(4)),
                ("ai", 6, "", true, Some(4)),
            ];
            for (name, n, extra, effect, _) in cases {
                let extra = if extra.is_empty() {
                    String::new()
                } else {
                    format!(",{extra}")
                };
                assert!(game.run_command(&format!(
                    "summon minecraft:pig {}.5 200 4.5 {{UUID:[I;204,0,0,{n}],Tags:[\"h204_{name}\"],PersistenceRequired:1b,Silent:1b,Health:10f{extra}}}",
                    2 * n
                )));
                if effect {
                    assert!(game.run_command(&format!(
                        "effect give @e[tag=h204_{name},limit=1] minecraft:absorption 100 0"
                    )));
                }
            }
            for (name, _, _, _, hit) in cases {
                if let Some(hit) = hit {
                    let accepted = game.run_command(&format!(
                        "damage @e[tag=h204_{name},limit=1] {hit} minecraft:generic"
                    ));
                    assert_eq!(accepted, name != "refused", "{name}: damage result");
                }
            }
            // Setup controls, independent of the tick: amount, health, effect.
            for (name, n, _, effect, hit) in cases {
                let i = pig(game, n);
                let m = &game.mobs[i];
                let amount = match (effect, hit) {
                    (false, _) => 0.0,
                    (true, Some(h)) if name != "refused" => 4.0 - h as f32,
                    _ => 4.0,
                };
                assert_eq!(m.absorption, amount, "{name}: absorption after the hit");
                let health = if !effect && hit.is_some() { 6.0 } else { 10.0 };
                assert_eq!(m.health, health, "{name}: health after the hit");
                assert_eq!(
                    absorption(game, i),
                    effect.then_some(2000),
                    "{name}: effect before the tick"
                );
            }

            // One level tick (`tick_world` runs after `tick` has advanced the
            // clock): the kept effect counting 2000 -> 1999 shows the mobs ticked once.
            let time = game.world.game_time;
            game.tick_world(true).unwrap();
            assert_eq!(
                game.world.game_time, time,
                "the level tick leaves the clock to `tick`"
            );
            assert_eq!(
                absorption(game, pig(game, 2)),
                Some(1999),
                "the mobs ticked exactly once"
            );
            let mut failures = Vec::new();
            for (name, n, _, effect, _) in cases {
                let i = pig(game, n);
                let expected = match name {
                    "depleted" | "ai" => None,
                    _ if effect => Some(1999),
                    _ => None,
                };
                if absorption(game, i) != expected {
                    failures.push(format!(
                        "{name}: effect {:?}, expected {expected:?}",
                        absorption(game, i)
                    ));
                }
                let saved = game
                    .command_target_nbt(Target::Mob(game.mobs[i].id))
                    .unwrap();
                let listed = matches!(saved.get("active_effects"), Some(horizon_core::nbt::Tag::List(l)) if !l.is_empty());
                if listed != expected.is_some() {
                    failures.push(format!("{name}: saved active_effects {listed}"));
                }
            }
            for (name, n, _, _, _) in [cases[0], cases[5]] {
                let i = pig(game, n);
                if game.mobs[i].absorption != 0.0 || game.mobs[i].health != 10.0 {
                    failures.push(format!("{name}: amount/health changed by the removal"));
                }
            }
            assert!(failures.is_empty(), "depleted absorption: {failures:#?}");
        });
    }

    /// The same for infinite effects (`MobEffectInstance.tickServer` asks
    /// `applyEffectTick` by the mob's tickCount): a used-up infinite
    /// Absorption goes, an unused one stays infinite, and another infinite
    /// effect is untouched.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES and a scratch HORIZON_HOME"]
    fn infinite_absorption_goes_only_once_used_up() {
        with_game(|game| {
            // name, UUID word, infinite effects, hit
            let cases: [(&str, i32, &[&str], bool); 4] = [
                ("inf_depleted", 11, &["absorption"], true),
                ("inf_kept", 12, &["absorption"], false),
                ("inf_other", 13, &["speed"], true),
                ("inf_both", 14, &["speed", "absorption"], true),
            ];
            for (name, n, effects, _) in cases {
                assert!(game.run_command(&format!(
                    "summon minecraft:pig {}.5 200 8.5 {{UUID:[I;204,0,0,{n}],Tags:[\"h204_{name}\"],PersistenceRequired:1b,Silent:1b,Health:10f,NoAI:1b}}",
                    n - 9
                )));
                for effect in effects {
                    assert!(game.run_command(&format!(
                        "effect give @e[tag=h204_{name},limit=1] minecraft:{effect} infinite 0"
                    )));
                }
            }
            for (name, _, _, hit) in cases {
                if hit {
                    assert!(game.run_command(&format!(
                        "damage @e[tag=h204_{name},limit=1] 4 minecraft:generic"
                    )));
                }
            }
            let effect = |game: &Game<'_>, i: usize, id: &str| {
                game.mobs[i]
                    .extra
                    .effects
                    .iter()
                    .find(|e| e.id == id)
                    .map(|e| e.duration)
            };
            for (name, n, effects, hit) in cases {
                let i = pig(game, n);
                let has = effects.contains(&"absorption");
                assert_eq!(
                    game.mobs[i].absorption,
                    if has && !hit { 4.0 } else { 0.0 },
                    "{name}: amount"
                );
                assert_eq!(
                    game.mobs[i].health,
                    if hit && !has { 6.0 } else { 10.0 },
                    "{name}: health"
                );
                assert_eq!(
                    effect(game, i, "minecraft:absorption"),
                    has.then_some(-1),
                    "{name}: before"
                );
            }
            let mut failures = Vec::new();
            for tick in 1..=2 {
                game.tick_world(true).unwrap();
                for (name, n, effects, hit) in cases {
                    let i = pig(game, n);
                    let absorption = (effects.contains(&"absorption") && !hit).then_some(-1);
                    let speed = effects.contains(&"speed").then_some(-1);
                    let got = (
                        effect(game, i, "minecraft:absorption"),
                        effect(game, i, "minecraft:speed"),
                    );
                    if got != (absorption, speed) {
                        failures.push(format!(
                            "{name} tick {tick}: (absorption, speed) {got:?}, expected {:?}",
                            (absorption, speed)
                        ));
                    }
                }
            }
            assert!(failures.is_empty(), "infinite absorption: {failures:#?}");
        });
    }
}

#[cfg(test)]
mod arrow_contents {
    //! Actual in-memory Game potion paths. No run loop, world load or server startup.
    use super::*;
    use crate::game::{Audio, Options, Progress, Settings, execution_role, load_resources};
    use horizon_client::chunk::Chunk;
    use serde_json::{Value, json};
    use std::{path::PathBuf, sync::Arc, time::Instant};

    fn stack(row: &Value) -> ItemStack {
        let mut stack = ItemStack::new("minecraft:tipped_arrow", 1);
        if let Some(potion) = row["potion"].as_str() {
            stack = stack.with_potion(&format!("minecraft:{potion}"));
        }
        let effects = row["custom"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| {
                Tag::compound([
                    (
                        "id",
                        Tag::String(
                            format!("minecraft:{}", e["id"].as_str().unwrap())
                                .as_str()
                                .into(),
                        ),
                    ),
                    ("duration", Tag::Int(e["duration"].as_i64().unwrap() as i32)),
                    ("amplifier", Tag::Int(e["amp"].as_i64().unwrap() as i32)),
                    (
                        "ambient",
                        Tag::Byte(i8::from(e["ambient"].as_bool().unwrap())),
                    ),
                    (
                        "show_particles",
                        Tag::Byte(i8::from(e["visible"].as_bool().unwrap())),
                    ),
                ])
            })
            .collect();
        stack
            .data_mut()
            .potion_extra
            .push(("custom_effects".into(), Tag::List(effects)));
        if let Some(color) = row["color"].as_i64() {
            stack
                .data_mut()
                .potion_extra
                .push(("custom_color".into(), Tag::Int(color as i32)));
        }
        if let Some(name) = row["name"].as_str() {
            stack
                .data_mut()
                .potion_extra
                .push(("custom_name".into(), Tag::String(name.into())));
        }
        stack
    }
    #[test]
    #[ignore = "explicit HORIZON_TEST_RESOURCES and isolated potion oracle input/output"]
    fn actual_arrow_contents_match_pinned_methods() {
        let root =
            PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").expect("explicit resources"));
        let input =
            PathBuf::from(std::env::var_os("HORIZON_POTION_INPUT").expect("explicit input"));
        let output =
            PathBuf::from(std::env::var_os("HORIZON_POTION_OUTPUT").expect("explicit output"));
        let expected: Vec<Value> = serde_json::from_slice(
            &std::fs::read(
                std::env::var_os("HORIZON_POTION_EXPECTED").expect("pinned method output"),
            )
            .unwrap(),
        )
        .unwrap();
        let rows: Vec<Value> = serde_json::from_slice(&std::fs::read(input).unwrap()).unwrap();
        let settings = Settings::default();
        let loaded = load_resources(
            horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            Instant::now(),
            false,
            crate::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.game_dir = Some(std::env::current_dir().unwrap());
        options.execution_role = execution_role::ExecutionRole::IntegratedServer;
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
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        game.world.set(0, 64, 0, stone).unwrap();
        assert_eq!(rows.len(), 5, "all five arrow cases must execute");
        let mut results = Vec::new();
        for (index, row) in rows.iter().enumerate() {
            let id = row["id"].clone();
            let mut arrow = Projectile::new(
                ProjectileKind::Arrow {
                    base_damage: 2.,
                    critical: false,
                    effect: None,
                },
                Vec3::new(0.5, 64.5, 0.5),
                None,
            );
            let st = arrow.arrow.as_mut().unwrap();
            st.pickup_item = stack(row);
            st.in_ground = true;
            st.in_ground_time = 598;
            st.last_state = Some(stone);
            arrow.stuck = true;
            let rain = |_| false;
            let scene = game.arrow_scene(&rain);
            horizon_client::arrows::tick(&mut arrow, &scene, &[]);
            let st = arrow.arrow.as_ref().unwrap();
            let item599 = st
                .pickup_item
                .item
                .trim_start_matches("minecraft:")
                .to_owned();
            let color599 = st.potion_color().map_or(-1, |v| v as i32);
            horizon_client::arrows::tick(&mut arrow, &scene, &[]);
            let st = arrow.arrow.as_ref().unwrap();
            results.push(json!({"id":id,"item599":item599,"color599":color599,"item600":st.pickup_item.item.trim_start_matches("minecraft:"),"color600":st.potion_color().map_or(-1,|v|v as i32)}));
            assert_eq!(
                results.last().unwrap(),
                &expected[index],
                "actual arrow tick: {}",
                id
            );
        }
        std::fs::write(output, serde_json::to_vec_pretty(&results).unwrap()).unwrap();
    }
}
