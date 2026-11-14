//! The player's `CombatTracker`: the damage taken lately, who dealt it and
//! where the player fell from, turned into the vanilla death message.
use super::{Game, commands::Target};

use horizon_client::{
    health::{Attack, HealthEvent},
    mobs::{Mob, Species, stuck::Stuck},
    motion::MotionEvent,
    particles::{ParticleOptions, ParticleType},
    text::Text,
};
use horizon_core::random::RandomSource;

/// `EntityEvent.DROWN_PARTICLES`: broadcast with the drowning damage.
pub(super) const DROWN_EVENT: i8 = 67;

struct Entry {
    damage_type: String,
    damage: f32,
    fall_location: Option<&'static str>,
    fall_distance: f32,
    /// The causing entity: its id (for comparisons) and display name.
    attacker: Option<(u32, Text)>,
    /// The causing player's main-hand stack when it has a custom name, as
    /// held when the hit landed (a mob's is read when the message is built).
    item: Option<Text>,
    /// The direct entity's display name when no entity caused the damage
    /// (`DamageSource.directEntity`: a potion with no thrower, a cloud).
    direct: Option<Text>,
}
struct PendingAttacker {
    damage_type: String,
    attacker: (u32, Text),
    item: Option<Text>,
    living: bool,
    credit: bool,
}

/// `LivingEntity.lastHurtByPlayer`: a reference that survives a disconnect.
#[derive(Clone, Copy)]
pub(super) struct PlayerCredit(pub(super) u128);
impl CombatTracker {
    /// `LivingEntity.lastHurtByPlayer`, remembered for 100 ticks (taken, as a
    /// dead player's memory goes with them).
    pub(super) fn take_hurt_by_player(&mut self, now: i64) -> Option<PlayerCredit> {
        let credit = self.player_credit(now);
        self.last_hurt_by_player = None;
        credit
    }
    /// Whether a player hurt this body within the last 100 ticks
    /// (`lastHurtByPlayerMemoryTime > 0`), without forgetting them.
    pub(super) fn recently_hit_by_player(&self, now: i64) -> bool {
        self.player_credit(now).is_some()
    }
    /// The player reference while its 100-tick memory is active.
    pub(super) fn player_credit(&self, now: i64) -> Option<PlayerCredit> {
        self.last_hurt_by_player
            .and_then(|(credit, at)| (now - at < 100).then_some(credit))
    }
    /// `LivingEntity.setLastHurtByPlayer(player, 100)`.
    pub(super) fn note_hurt_by_player(&mut self, credit: PlayerCredit, now: i64) {
        self.last_hurt_by_player = Some((credit, now));
    }
}
#[derive(Default)]
pub(super) struct CombatTracker {
    /// The arrows and bee stingers stuck in the body.
    pub(super) stuck: Stuck,
    entries: Vec<Entry>,
    last_damage: i64,
    in_combat: bool,
    /// Attackers of damage dealt this tick, claimed by the matching
    /// `RecordedDamage` event.
    pending: Vec<PendingAttacker>,
    /// `pending` for a direct entity with no causing entity: only its name.
    pending_direct: Vec<(String, Text)>,
    /// The lengths of `pending` and `pending_direct` before the first
    /// attacker noted for the hurt being dealt now (`forget_noted`).
    noted_since: Option<(usize, usize)>,
    /// `LivingEntity.lastHurtByMob` and its timestamp (the kill credit).
    last_hurt_by: Option<((u32, Text), i64)>,
    /// `LivingEntity.lastHurtByPlayer` and its timestamp.
    last_hurt_by_player: Option<(PlayerCredit, i64)>,
    /// The mob the player last hurt (`getLastHurtMob`) and when.
    pub(super) last_hurt_mob: Option<(u32, i64)>,
    /// The attack the hits on the player land in now: who attacks and what they hold. Set only
    /// by [`Game::with_attack`], which restores the previous one on return.
    attack: Attack,
    /// The mob whose projectile is hitting the player now.
    pub(super) projectile_owner: Option<u32>,
    /// The display name of the projectile hitting the player now: the
    /// `DamageSource.directEntity` the death message names when no owner is
    /// known (`AbstractArrow.onHitEntity` makes an ownerless arrow its own
    /// cause).
    pub(super) projectile_direct: Option<Text>,
    /// The player whose primed TNT is going off now (`PrimedTnt.getOwner`):
    /// its entity id and display name, the blast's causing entity.
    pub(super) blast_owner: Option<(u32, Text)>,
    /// `Some(None)` is an explicitly ownerless crystal/cart DamageSource.
    pub(super) explicit_explosion_cause: Option<Option<super::explosion::ExplosionCause>>,
    pub(super) explicit_explosion_direct: Option<Text>,
    /// The guest behind `blast_owner`, when that player is a LAN guest; the
    /// host's own blast leaves it None.
    pub(super) blast_guest: Option<u32>,
    /// `blast_owner` is a player (`getIndirectSourceEntity() instanceof
    /// Player`), not a mob whose UUID the TNT's `owner` names.
    pub(super) blast_owner_player: bool,
    pub(super) projectile_attacker: Option<(u32, Text)>,
    pub(super) enchantment_projectile_attacker: Option<u32>,
    /// The projectile hitting the player now is another player's that
    /// `ServerPlayer.canHarmPlayer` refuses (the `pvp` rule, an ally).
    pub(super) refuse_player_hit: bool,
    /// The hit on the player now runs its post-attack effects (thorns) only
    /// if the player lives through it (`WitherSkull.onHitEntity`).
    pub(super) post_attack_if_alive: bool,
    /// The mob whose swing is being applied, when the caller knows it. Without
    /// it the attacker is the mob nearest to where the hit came from, which
    /// walks every mob.
    pub(super) melee_mob: Option<u32>,
    /// The attack event handled last hurt its target (`doHurtTarget` returned
    /// true). The events of one blow read it: `Zombie.doHurtTarget` burns only
    /// once `super.doHurtTarget` did (`MobEvent::ZombieIgnite` follows the
    /// attack it rides).
    pub(super) hit_landed: bool,
    /// A firework rocket's burst is hurting the player now: the rocket is
    /// the direct entity (`rocket_hurt_player`).
    pub(super) rocket_hit: bool,
    /// The arrow hurting the player now pierces (`AbstractArrow.getPierceLevel` above 0):
    /// `LivingEntity.applyItemBlocking` lets it past a raised shield (`arrow_hits_player`).
    pub(super) piercing_hit: bool,
    /// The player whose melee hit is landing on the player now: where they
    /// stand and the seconds their weapon disables a shield for
    /// (`getSecondsToDisableBlocking`).
    pub(super) player_attacker: Option<([f64; 3], f32)>,
    /// While set, a sweep attack's reach to the other players is kept here
    /// instead of landing, for the caller that holds the LAN host.
    pub(super) sweeps: Option<Vec<super::lan_pvp::Sweep>>,
    /// The mob exploding now (`ServerExplosion.source`): its team's friendly
    /// fire decides whom the blast may hurt.
    pub(super) explosion_source: Option<u32>,
    /// The primed TNT, TNT cart, fireball, wither skull or end crystal
    /// exploding now, by entity id (`Explosion.getDirectSourceEntity`; a
    /// mob's is `explosion_source`).
    pub(super) explosion_direct: Option<u32>,
    /// The exploding mob as predicates see it (`ServerExplosion.source`,
    /// `ServerPlayer.currentExplosionCause`), for a mob already out of the list.
    pub(super) explosion_cause: Option<horizon_client::advancements::EntityView>,
    /// The exploding mob named itself (`MobEvent::ExplosionSource`), so the
    /// blast leaves it out (`getEntities(source, box)`). A source guessed
    /// from the mobs near the centre is still hurt: it may be a bystander.
    pub(super) explosion_source_named: bool,
    /// The entity list a listening `explosion_detonate` left behind, in the
    /// order the blast visits it: the listeners took the spared entities out
    /// and may have added others. `None` when no mod listened, and the walks
    /// visit every entity as before (`explosion_detonate.rs`).
    pub(super) blast_hit_list: Option<Vec<u32>>,
    /// `blast_hit_list`'s ids as a set, so a walk's membership check stays
    /// constant time however large the list is.
    pub(super) blast_hit_ids: std::collections::HashSet<u32>,
    /// The blast's direct source is in water (`Explosion.getDirectSourceEntity`
    /// `.isInWater`): paintings, item frames and cushions ignore it
    /// (`BlockAttachedEntity.ignoreExplosion`).
    pub(super) explosion_in_water: bool,
    /// The blast now going off comes from a primed TNT, creeper, wither
    /// skull, wither or TNT minecart, the sources that empty a hive they
    /// destroy (`BeehiveBlock.getDrops`).
    pub(super) hive_emptying_blast: bool,
    /// The blast now going off comes from a primed TNT minecart, which
    /// neither resists on nor breaks a rail or the block under one
    /// (`MinecartTNT.getBlockExplosionResistance`, `shouldBlockExplode`).
    pub(super) rail_proof_blast: bool,
    /// The mob that owns the projectile exploding now (`getIndirectSourceEntity`):
    /// the mobs the blast hurts record it as their attacker.
    pub(super) explosion_owner: Option<u32>,
    /// `WitherSkull.getBlockExplosionResistance`: a dangerous skull's blast
    /// caps every destroyable block's resistance at this value.
    pub(super) blast_resistance_cap: Option<f32>,
    /// A mod blast's own damage type, calculator and effects
    /// (`level_explode`'s settings), while it goes off.
    pub(super) blast_settings: super::explosion_settings::BlastSettings,
    /// Charged creepers whose blast just went off and that have not dropped
    /// a head yet (`Creeper.droppedSkulls`): they are gone from the mob list,
    /// but the mobs they killed still ask for their loot.
    pub(super) charged_exploders: Vec<u32>,
    /// The mobs whose blast went off this tick and that left the mob list
    /// (`MobEvent::ExplosionSource`): their names, for the death messages of
    /// the mobs the blast killed.
    pub(super) exploded: Vec<(u32, Text)>,
    /// The entity types of `exploded`: a player their blast killed is still
    /// credited to them (`LivingEntity.getKillCredit` during the explosion).
    pub(super) exploded_kinds: Vec<(u32, String)>,
    /// The player's fall distance before this tick's movement (as of the
    /// end of the last one).
    fall_before: f64,
}
impl CombatTracker {
    /// Whether the blast now hurting entities visits `id`: the storages are
    /// the walk, unless a listening `explosion_detonate` left a list, and
    /// then only what that list names is.
    pub(super) fn visits(&self, id: u32) -> bool {
        self.blast_hit_list.is_none() || self.blast_hit_ids.contains(&id)
    }

    /// The attack lent by the enclosing [`Game::with_attack`] (`damage_level_of!(game, scoped)`).
    pub(super) fn attack(&self) -> &Attack {
        &self.attack
    }

    /// The newest damage of this fight for `events`: when it landed (game
    /// time), its damage type, the amount and the name of what dealt it.
    pub(super) fn last_hit(&self) -> Option<(i64, String, Option<f32>, Option<String>)> {
        let e = self.entries.last()?;
        let by = e
            .attacker
            .as_ref()
            .map(|(_, t)| t.plain())
            .or_else(|| e.direct.as_ref().map(Text::plain));
        Some((self.last_damage, e.damage_type.clone(), Some(e.damage), by))
    }
    /// The scalar death-tail/death-message fallback, not the full accepted killing source.
    pub(super) fn last_blow(&self) -> Option<(&str, u32)> {
        let e = self.entries.last()?;
        Some((&e.damage_type, e.attacker.as_ref().map_or(0, |a| a.0)))
    }
    fn mark(&mut self) {
        let marks = (self.pending.len(), self.pending_direct.len());
        self.noted_since.get_or_insert(marks);
    }
    /// `CombatTracker.getKiller`'s credited attacker: the entity that last
    /// hurt the body, within a hundred ticks of the hurt.
    pub(super) fn kill_credit(&self, now: i64) -> Option<u32> {
        let ((id, _), at) = self.last_hurt_by.as_ref()?;
        (now - at <= 100).then_some(*id)
    }
    /// `LivingEntity.getLastDamageSource` for an advancement's
    /// `killing_blow`: the newest combat entry's type and causing entity.
    pub(super) fn killing_blow(&self) -> Option<(String, Option<u32>)> {
        self.entries.last().map(|e| {
            (
                e.damage_type.clone(),
                e.attacker.as_ref().map(|(id, _)| *id),
            )
        })
    }
    /// The player's hurt is over: when a native mod canceled it, the
    /// attackers noted for it go, as no combat entry takes them and the next
    /// hit of the same damage type must not be credited to them.
    pub(super) fn forget_noted(&mut self, canceled: bool) {
        if let Some((attackers, directs)) = self.noted_since.take()
            && canceled
        {
            self.pending.truncate(attackers);
            self.pending_direct.truncate(directs);
        }
    }
}
impl Game<'_> {
    /// Runs `f` with `attack` as the attack the hits on the player land in, and puts the previous
    /// one back when it returns: the one writer of `combat.attack`, so the restore cannot be
    /// skipped. Mob victims take their attack explicitly (`DamageLevel::attacked_by`); this is
    /// the one implicit scope left, because the player-victim path (`hurt_player_from`,
    /// `mob_hits_player`) has ~30 callers that would all pass it through.
    pub(super) fn with_attack<R>(&mut self, attack: Attack, f: impl FnOnce(&mut Self) -> R) -> R {
        let outer = std::mem::replace(&mut self.combat.attack, attack);
        let result = f(self);
        self.combat.attack = outer;
        result
    }
    /// The attack the hits on the player land in now (a reader of `combat.attack`).
    pub(super) fn current_attack(&self) -> Attack {
        self.combat.attack.clone()
    }

    /// `LivingEntity.makeDrownParticles`: `count` bubbles around the entity
    /// (vanilla's eight), moving with it.
    pub(super) fn drown_particles(&mut self, at: [f64; 3], velocity: [f64; 3], count: i32) {
        let bubble = ParticleOptions::simple(ParticleType::Bubble);
        for _ in 0..count {
            let mut triangle = || {
                let r = self.particles.random();
                r.next_double() - r.next_double()
            };
            let p = [at[0] + triangle(), at[1] + triangle(), at[2] + triangle()];
            self.particles.add(&bubble, p, velocity);
        }
    }
    /// Entity event 67 from a server, for this client's own player or an
    /// entity it draws.
    pub(super) fn remote_drown(&mut self, id: i32) {
        let own = self.remote.as_ref().is_some_and(|r| r.entity_id == id);
        let body = if own {
            self.player.as_ref().map(|p| {
                let (at, v) = (p.position, p.velocity);
                ([at.x, at.y, at.z], [v.x, v.y, v.z])
            })
        } else {
            self.mobs.iter().find(|m| m.id == id as u32).map(|m| {
                let (at, v) = (m.position, m.velocity);
                ([at.x, at.y, at.z], [v.x, v.y, v.z])
            })
        };
        if let Some((at, v)) = body {
            self.drown_particles(at, v, 8);
        }
    }
    /// `LivingDrownEvent`'s `bubbles` for an entity drowning at `at`: true for
    /// vanilla's eight, which the caller sends as entity event 67 and draws.
    /// Another positive count goes to the players around as that many bubble
    /// particles and is drawn here, since a vanilla client's event 67 always
    /// draws eight; none sends nothing.
    pub(super) fn drown_bubbles(&mut self, at: [f64; 3], velocity: [f64; 3], bubbles: i32) -> bool {
        if bubbles == 8 {
            return true;
        }
        if bubbles > 0 {
            let bubble = ParticleOptions::simple(ParticleType::Bubble);
            self.lan_level_particles(&bubble, at, [0.5; 3], 0.0, bubbles as u32, None);
            self.drown_particles(at, velocity, bubbles);
        }
        false
    }
    /// A mob's drowning tick (`MobEvent::Drown`).
    pub(super) fn mob_drown(&mut self, mob: u32, at: [f64; 3], bubbles: i32) {
        let v = self
            .mobs
            .iter()
            .find(|m| m.id == mob)
            .map_or([0.0; 3], |m| [m.velocity.x, m.velocity.y, m.velocity.z]);
        if self.drown_bubbles(at, v, bubbles) {
            self.lan_entity_event(mob as i32, DROWN_EVENT, at);
            self.drown_particles(at, v, 8);
        }
    }
    /// The display name of a mob: its custom name or its type's name.
    pub(super) fn mob_display_name(&self, id: u32) -> Option<Text> {
        let m = self.mobs.iter().find(|m| m.id == id)?;
        Some(match &m.custom_name {
            Some(name) => Text::literal(name.clone()),
            None => Text::translate(&format!("entity.{}", m.kind.id.replace(':', ".")), &[]),
        })
    }
    /// `getLastHurtByMob`: the last mob to hurt the player, for 100 ticks.
    pub(super) fn player_hurt_by(&self) -> Option<(horizon_client::mobs::Who, i64)> {
        let ((id, _), at) = self.combat.last_hurt_by.as_ref()?;
        (self.world.game_time - at <= 100).then_some((horizon_client::mobs::Who::Mob(*id), *at))
    }
    /// Remember who deals the next `damage_type` damage to the player.
    pub(super) fn note_attacker(&mut self, damage_type: &str, mob: u32) {
        if let Some(name) = self.mob_display_name(mob) {
            self.note_named_attacker(damage_type, mob, name);
        }
    }
    /// `note_attacker` for an entity that is not one of `self.mobs` (the
    /// ender dragon).
    pub(super) fn note_named_attacker(&mut self, damage_type: &str, id: u32, name: Text) {
        self.note_player_attacker(damage_type, id, name, None);
    }
    /// `note_named_attacker` for a player, with their custom-named main-hand
    /// item for the death message's `.item` form.
    pub(super) fn note_player_attacker(
        &mut self,
        damage_type: &str,
        id: u32,
        name: Text,
        item: Option<Text>,
    ) {
        self.combat.mark();
        self.combat.pending.push(PendingAttacker {
            damage_type: damage_type.to_owned(),
            attacker: (id, name),
            item,
            living: true,
            credit: true,
        });
    }
    /// A non-living or already removed cause remains in the damage source,
    /// but cannot resolve as an EntityReference for kill credit.
    pub(super) fn note_uncredited_attacker(
        &mut self,
        damage_type: &str,
        id: u32,
        name: Text,
        item: Option<Text>,
        living: bool,
    ) {
        self.combat.mark();
        self.combat.pending.push(PendingAttacker {
            damage_type: damage_type.to_owned(),
            attacker: (id, name),
            item,
            living,
            credit: false,
        });
    }
    /// The `DamageSource.directEntity` of the next `damage_type` damage when
    /// nothing caused it (a splash potion nobody threw): the death message
    /// names it instead (`getLocalizedDeathMessage`). It is no kill credit.
    pub(super) fn note_direct_entity(&mut self, damage_type: &str, name: Text) {
        self.combat.mark();
        self.combat
            .pending_direct
            .push((damage_type.to_owned(), name));
    }
    /// `LivingEntity.resolvePlayerResponsibleForDamage` for the player in place,
    /// hit by entity `id`, which only a native mod reads: a player's hit names
    /// them, a tame wolf's its owner (no one without one), and other mobs leave
    /// it as it was.
    fn resolve_player_responsible_for_player(&mut self, id: u32) {
        let now = self.world.game_time;
        match self.mobs.iter().find(|m| m.id == id) {
            None => {
                let uuid = self.lan.as_ref().and_then(|host| {
                    host.hub
                        .guests
                        .iter()
                        .find(|g| g.entity_id as u32 == id)
                        .map(|g| g.uuid)
                });
                if let Some(uuid) = uuid {
                    self.combat.note_hurt_by_player(PlayerCredit(uuid), now);
                }
            }
            // `instanceof Wolf`: a content mod's type copying the wolf is one.
            Some(m) if m.extra.tame && m.species() == Species::Wolf => {
                self.combat.last_hurt_by_player = m.extra.owner.map(|o| {
                    let owner = horizon_client::lan::playerdata::ints_uuid(o);
                    (PlayerCredit(owner), now)
                });
            }
            Some(_) => {}
        }
    }
    /// `CombatTracker.recordDamage` for the motion events of this tick.
    pub(super) fn record_combat(&mut self, events: &[MotionEvent]) {
        self.recheck_combat();
        // The attacker of the hit whose damage was just recorded, for its criterion.
        let mut recorded: Option<(String, u32)> = None;
        for e in events {
            let (name, amount) = match e {
                // LivingEntity.hurtServer reached resolvePlayerResponsibleForDamage:
                // every hit that got past the cooldown, absorbed or blocked too.
                MotionEvent::Health(HealthEvent::Criterion { source, .. }) => {
                    if let Some(d) = self.motion.health.damage.definition(*source) {
                        let attacker = match recorded.take() {
                            Some((t, id)) if t == d.name => Some(id),
                            _ => {
                                if let Some(pending) = self
                                    .combat
                                    .pending
                                    .iter()
                                    .find(|pending| pending.damage_type == d.name)
                                {
                                    self.combat.last_hurt_by =
                                        Some((pending.attacker.clone(), self.world.game_time));
                                    Some(pending.attacker.0)
                                } else {
                                    None
                                }
                            }
                        };
                        if let Some(id) = attacker {
                            self.resolve_player_responsible_for_player(id);
                        }
                    }
                    continue;
                }
                // LivingEntity.getDamageAfterMagicAbsorb (Resistance) and
                // Player.actuallyHurt (absorption hearts) and
                // LivingEntity.hurtServer (the shield, only for an accepted
                // hit), in tenths.
                MotionEvent::Health(HealthEvent::Statistic { name, amount }) => {
                    match *name {
                        "resisted" => self.award_custom_stat("damage_resisted", *amount),
                        "absorbed" => self.award_custom_stat("damage_absorbed", *amount),
                        "blocked" => self.award_custom_stat("damage_blocked_by_shield", *amount),
                        // MotionWorld emits the authoritative jump award here too.
                        // Both local and guest event drains share this consumer.
                        "jump" => self.award_custom_stat("jump", *amount),
                        _ => {}
                    }
                    continue;
                }
                MotionEvent::Health(HealthEvent::RecordedDamage { source, amount }) => {
                    let Some(d) = self.health_resources.damage.definition(*source) else {
                        continue;
                    };
                    (d.name.clone(), *amount)
                }
                _ => continue,
            };
            let (attacker, item, living, credit) = self
                .combat
                .pending
                .iter()
                .position(|p| p.damage_type == name)
                .map(|i| {
                    let PendingAttacker {
                        attacker,
                        item,
                        living,
                        credit,
                        ..
                    } = self.combat.pending.remove(i);
                    (Some(attacker), item, living, credit)
                })
                .unwrap_or_default();
            let direct = match attacker {
                Some(_) => None,
                None => self
                    .combat
                    .pending_direct
                    .iter()
                    .position(|(t, _)| *t == name)
                    .map(|i| self.combat.pending_direct.remove(i).1),
            };
            let (fall_location, fall_distance) = self.fall_location();
            let now = self.world.game_time;
            if let Some(a) = &attacker {
                if living {
                    self.combat.last_hurt_by = credit.then(|| (a.clone(), now));
                }
                recorded = Some((name.clone(), a.0));
                if living && !self.combat.in_combat {
                    self.combat.in_combat = true;
                }
            }
            if let Some(tenths) = damage_taken_award(amount) {
                self.award_custom_stat("damage_taken", tenths);
            }
            self.combat.entries.push(Entry {
                damage_type: name,
                damage: amount,
                fall_location,
                fall_distance,
                attacker,
                item,
                direct,
            });
            self.combat.last_damage = now;
            // InfestedMobEffect.onMobHurt.
            if self.potions.effects.get("infested").is_some() {
                self.player_infested();
            }
        }
        self.combat.pending.clear();
        self.combat.pending_direct.clear();
        self.combat.noted_since = None;
        self.combat.projectile_direct = None;
        self.combat.projectile_attacker = None;
        self.combat.fall_before = self.player.as_ref().map_or(0.0, |p| p.fall_distance);
    }
    /// `FallLocation.getCurrentFallLocation` and the fall distance so far.
    fn fall_location(&self) -> (Option<&'static str>, f32) {
        let Some(p) = self.player.as_ref() else {
            return (None, 0.0);
        };
        let distance = p.fall_distance.max(self.combat.fall_before) as f32;
        let location = if let Some(c) = p.last_climbable {
            let name = self
                .res
                .facts()
                .get(self.world.block(c.x, c.y, c.z))
                .name
                .clone();
            let trapdoor = (self.res.items().block_tags)
                .get("minecraft:trapdoors")
                .is_some_and(|tag| tag.contains(&*name));
            Some(fall_location_of(&name, trapdoor))
        } else {
            let f = self.res.facts().get(self.world.block(
                p.position.x.floor() as i32,
                (p.position.y + 0.1).floor() as i32,
                p.position.z.floor() as i32,
            ));
            (f.fluid == horizon_client::blocks::Fluid::Water || f.waterlogged).then_some("water")
        };
        (location, distance)
    }
    /// `CombatTracker.recheckStatus`: forget old fights.
    fn recheck_combat(&mut self) {
        let reset = if self.combat.in_combat { 300 } else { 100 };
        if !self.combat.entries.is_empty() && self.world.game_time - self.combat.last_damage > reset
        {
            self.combat.entries.clear();
            self.combat.in_combat = false;
        }
    }
    fn damage_in_tag(&self, name: &str, tag: &str) -> bool {
        let damage = &self.health_resources.damage;
        damage.id(name).is_some_and(|id| {
            damage.in_tag(id, tag) || damage.in_tag(id, &format!("minecraft:{tag}"))
        })
    }
    /// `CombatTracker.getMostSignificantFall`.
    fn most_significant_fall(&self) -> Option<usize> {
        let (mut result, mut alternative) = (None, None);
        let (mut best_fall, mut alt_damage) = (0.0f32, 0.0f32);
        for (i, e) in self.combat.entries.iter().enumerate() {
            let fake = self.damage_in_tag(&e.damage_type, "always_most_significant_fall");
            let distance = if fake { f32::MAX } else { e.fall_distance };
            if (self.damage_in_tag(&e.damage_type, "is_fall") || fake)
                && distance > 0.0
                && (result.is_none() || distance > best_fall)
            {
                result = Some(i.saturating_sub(1));
                best_fall = distance;
            }
            if e.fall_location.is_some() && (alternative.is_none() || e.damage > alt_damage) {
                alternative = Some(i);
                alt_damage = e.damage;
            }
        }
        if best_fall > 5.0 && result.is_some() {
            result
        } else if alt_damage > 5.0 {
            alternative
        } else {
            None
        }
    }
    /// The display component of a custom-named held stack.
    pub(super) fn mob_main_hand_name(&self, id: u32) -> Option<Text> {
        let m = self.mobs.iter().find(|m| m.id == id)?;
        let mut stack = m.extra.main_hand_stack()?.clone();
        if !has_custom_name(&stack) {
            return None;
        }
        if let Some(g) = horizon_client::mobs::gear::gear(m) {
            stack.data_mut().enchantments = g.slots[horizon_client::mobs::gear::MAIN].clone();
        }
        Some(self.stack_display_name(&stack))
    }
    /// `getMainHandItem().getDisplayName()` of the player in place, when the
    /// stack has a custom name.
    pub(super) fn main_hand_custom_name(&self) -> Option<Text> {
        let stack = self.inventory.get(self.selected)?.as_ref()?;
        has_custom_name(stack).then(|| self.stack_display_name(stack))
    }
    /// `main_hand_custom_name` of a player who is not in place: the guest
    /// `conn` between turns, or none for the host while a guest acts (their
    /// hotbar slot is kept by the swap).
    pub(super) fn player_main_hand_custom_name(&self, player: Option<u32>) -> Option<Text> {
        if player == self.lan_acting {
            return self.main_hand_custom_name();
        }
        let g = self.lan.as_ref()?.hub.guest(player?)?;
        let stack = g.inventory.get(g.selected)?.as_ref()?;
        has_custom_name(stack).then(|| self.stack_display_name(stack))
    }
    /// The killer's item for `getLocalizedDeathMessage` and
    /// `getMessageForAssistedFall`: a player's as recorded, a mob's now.
    fn entry_item(&self, e: &Entry) -> Option<Text> {
        e.item
            .clone()
            .or_else(|| self.mob_main_hand_name(e.attacker.as_ref()?.0))
    }
    /// `CombatTracker.getDeathMessage` for a player whose display name is
    /// `me`: a server's guest is not `PLAYER_NAME`.
    pub(super) fn combat_death_message_for(&mut self, me: Text) -> Text {
        self.recheck_combat();
        let entries = std::mem::take(&mut self.combat.entries);
        self.combat.in_combat = false;
        let Some(killing) = entries.last() else {
            return Text::translate("death.attack.generic", &[me]);
        };
        let def = self
            .health_resources
            .damage
            .id(&killing.damage_type)
            .and_then(|id| self.health_resources.damage.definition(id))
            .map(|d| (d.message_id.clone(), d.death_message_type.clone()))
            .unwrap_or_else(|| ("generic".into(), "default".into()));
        self.combat.entries = entries;
        let knock = self.most_significant_fall();
        let entries = std::mem::take(&mut self.combat.entries);
        let killing = entries.last().expect("checked above");
        let killer = killing.attacker.clone();
        let direct = killing.direct.clone();
        if def.1 == "fall_variants"
            && let Some(knock) = knock.and_then(|i| entries.get(i))
        {
            let fall = self.damage_in_tag(&knock.damage_type, "is_fall")
                || self.damage_in_tag(&knock.damage_type, "always_most_significant_fall");
            if fall {
                let at = knock.fall_location.unwrap_or("generic");
                return Text::translate(&format!("death.fell.accident.{at}"), &[me]);
            }
            // CombatTracker.getMessageForAssistedFall: the attacker's item
            // with a custom name gets the `.item` message.
            let (key, args) = match (&knock.attacker, &killer) {
                (Some(a), k) if k.as_ref().is_none_or(|k| k.0 != a.0) => {
                    horizon_client::scoreboard::death_message_args(
                        "death.fell.assist",
                        &me,
                        &a.1,
                        self.entry_item(knock).as_ref(),
                    )
                }
                (_, Some(k)) => horizon_client::scoreboard::death_message_args(
                    "death.fell.finish",
                    &me,
                    &k.1,
                    self.entry_item(killing).as_ref(),
                ),
                _ => ("death.fell.killer".into(), vec![me]),
            };
            return Text::translate(&key, &args);
        }
        let key = format!("death.attack.{}", def.0);
        if def.1 == "intentional_game_design" {
            let link = Text::literal("[")
                .append(Text::translate(&format!("{key}.link"), &[]))
                .append(Text::literal("]"));
            return Text::translate(&format!("{key}.message"), &[me, link]);
        }
        // `getLocalizedDeathMessage`: the causing entity's name, or the
        // direct entity's when nothing caused the damage.
        if let Some((_, name)) = killer {
            // DamageSource.getLocalizedDeathMessage: the cause's item with a
            // custom name gets the `<id>.item` message.
            let (key, args) = horizon_client::scoreboard::death_message_args(
                &key,
                &me,
                &name,
                self.entry_item(killing).as_ref(),
            );
            return Text::translate(&key, &args);
        }
        if let Some(name) = direct {
            return Text::translate(&key, &[me, name]);
        }
        // getKillCredit: the last mob that hurt the player in the past 100 ticks.
        if let Some(((_, name), at)) = &self.combat.last_hurt_by
            && self.world.game_time - at <= 100
        {
            return Text::translate(&format!("{key}.player"), &[me, name.clone()]);
        }
        // No entity at all: the one-argument message, as vanilla sends it (a
        // template that names a killer then shows as written).
        Text::translate(&key, &[me])
    }
    /// `TamableAnimal.die`: with `showDeathMessages`, a tame wolf, cat,
    /// parrot or nautilus's owner, when online in any dimension
    /// (`EntityReference.getEntity` looks through every level), is sent how it
    /// died. `killer` is the player who last hurt it (`None` inside for the
    /// host), from `last_hurt_by_player`.
    pub(super) fn tell_owner_pet_died(&mut self, m: &Mob, killer: Option<Option<u32>>) {
        let tamable = matches!(
            m.species(),
            Species::Wolf
                | Species::Cat
                | Species::Parrot
                | Species::Nautilus
                | Species::ZombieNautilus
        );
        let Some(owner) = m.extra.owner.filter(|_| tamable && m.extra.tame) else {
            return;
        };
        if !self
            .ui
            .rules
            .bool_at(horizon_client::gamerules::vanilla::SHOW_DEATH_MESSAGES)
        {
            return;
        }
        let host = self.player.is_some()
            && self.options.dedicated.is_none()
            && horizon_client::mobs::tame::owned_by(
                Some(owner),
                self.level.player_uuid.unwrap_or_default(),
            );
        let uuid = horizon_client::lan::playerdata::ints_uuid(owner);
        let guest = !host
            && self
                .lan
                .as_ref()
                .is_some_and(|l| l.hub.guests.iter().any(|g| g.placed && g.uuid == uuid));
        if !host && !guest {
            return;
        }
        let mut built = Some(self.mob_death_message(m, killer));
        let kind = super::native_hooks::mob_kind(m);
        self.native_living_death_message(m.id, &kind, &mut built);
        // A native mod may have changed the message, or set it to None to tell the owner nothing.
        let Some(message) = built else {
            return;
        };
        if host {
            self.system_message(message);
        } else if let Some(lan) = &self.lan {
            let content = horizon_client::lan::data::component_nbt(&message.component());
            lan.hub.system_message_to_player(uuid, content);
        }
    }
    /// `CombatTracker.getDeathMessage` for a mob, from the hits it recorded
    /// (`Mob::combat`), told the way the player's own tracker tells them.
    pub(super) fn mob_death_message(&mut self, m: &Mob, killer: Option<Option<u32>>) -> Text {
        use horizon_client::mobs::{DamageKind, Who};
        let me = mob_name(m);
        // The player behind a `Who::Player` hit: the last one to hurt it, or
        // the one in place (a server with nobody acting has none).
        let player = match killer {
            Some(Some(conn)) => Some((u32::MAX - 1 - conn, self.target_name(Target::Guest(conn)))),
            Some(None) => Some((u32::MAX, self.target_name(Target::Player))),
            None if self.player.is_some() || self.lan_acting.is_some() => {
                Some((u32::MAX, self.target_name(Target::Player)))
            }
            None => None,
        };
        // DamageSource.getLocalizedDeathMessage reads that player's main hand
        // as the message is built.
        let player_item = match killer {
            Some(conn) => self.player_main_hand_custom_name(conn),
            None => player.as_ref().and_then(|_| self.main_hand_custom_name()),
        };
        // The cause's display name: a mob that is gone (a creeper that blew
        // up) is still named by its blast.
        let name = |g: &Self, who: Who| match who {
            Who::Player => player.clone(),
            Who::Mob(id) => g
                .mob_display_name(id)
                .or_else(|| {
                    g.combat
                        .exploded
                        .iter()
                        .find(|(e, _)| *e == id)
                        .map(|(_, n)| n.clone())
                })
                .map(|n| (id, n)),
        };
        let entries = m
            .combat
            .entries
            .iter()
            .map(|e| {
                let attacker = e.source.attacker.and_then(|w| name(self, w));
                let item = (e.source.attacker == Some(Who::Player))
                    .then(|| player_item.clone())
                    .flatten();
                // AbstractArrow.onHitEntity: an arrow nobody shot is its own
                // cause; a rocket nobody fired still names itself.
                let direct = match e.source.kind {
                    _ if attacker.is_some() => None,
                    DamageKind::Arrow => Some(Text::translate("entity.minecraft.arrow", &[])),
                    DamageKind::Fireworks => {
                        Some(Text::translate("entity.minecraft.firework_rocket", &[]))
                    }
                    _ => None,
                };
                Entry {
                    damage_type: super::creatures::source_damage_type(e.source).to_owned(),
                    damage: e.damage,
                    fall_location: e.in_water.then_some("water"),
                    fall_distance: e.fall_distance,
                    attacker,
                    item,
                    direct,
                }
            })
            .collect();
        // LivingEntity.getKillCredit: the player who hurt it lately, else the
        // last mob to hurt it within 100 ticks.
        let credit = if m.ai.last_hurt_by_player > 0 && player.is_some() {
            player.clone()
        } else {
            m.ai.last_hurt_by
                .filter(|_| m.age as i32 - m.ai.last_hurt_by_time <= 100)
                .and_then(|w| name(self, w))
        };
        let now = self.world.game_time;
        let mut tracker = CombatTracker {
            entries,
            last_damage: now,
            in_combat: m.combat.in_combat,
            last_hurt_by: credit.map(|c| (c, now)),
            ..CombatTracker::default()
        };
        std::mem::swap(&mut self.combat, &mut tracker);
        let text = self.combat_death_message_for(me);
        std::mem::swap(&mut self.combat, &mut tracker);
        text
    }
}

/// `!stack.isEmpty() && stack.has(DataComponents.CUSTOM_NAME)`: the held
/// item that turns a death message into its `.item` form.
fn has_custom_name(stack: &horizon_client::items::ItemStack) -> bool {
    &*stack.item != "minecraft:air"
        && stack.count > 0
        && stack.data().is_some_and(|d| d.custom_name.is_some())
}

/// `Entity.getDisplayName` of a mob: its custom name or its type's name.
fn mob_name(m: &Mob) -> Text {
    match (&m.custom_name_text, &m.custom_name) {
        (Some(text), _) => text.clone(),
        (None, Some(name)) => Text::literal(name.clone()),
        (None, None) => Text::translate(&format!("entity.{}", m.kind.id.replace(':', ".")), &[]),
    }
}

/// `Player.actuallyHurt`'s DAMAGE_TAKEN award: `dmg * 10.0F` rounded as Java
/// rounds a float (`jmath::round_f32`), only for damage under 3.4028235E37F.
/// A huge hit (`/damage @s 3.4e38`) awards nothing, not a saturated count
/// (#11339).
fn damage_taken_award(amount: f32) -> Option<i32> {
    (amount < 3.402_823_5e37).then(|| horizon_core::jmath::round_f32(amount * 10.0))
}

/// `FallLocation.blockToFallLocation`: what a climbable block stands for in a fall's death message.
/// `trapdoor` is whether the block is in `#minecraft:trapdoors`, which counts as a ladder.
fn fall_location_of(name: &str, trapdoor: bool) -> &'static str {
    match name {
        _ if trapdoor => "ladder",
        "minecraft:ladder" => "ladder",
        "minecraft:vine" => "vines",
        "minecraft:weeping_vines" | "minecraft:weeping_vines_plant" => "weeping_vines",
        "minecraft:twisting_vines" | "minecraft:twisting_vines_plant" => "twisting_vines",
        "minecraft:scaffolding" => "scaffolding",
        _ => "other_climbable",
    }
}

#[cfg(test)]
mod fall_location_tests {
    use super::fall_location_of;

    /// The branches of `blockToFallLocation`: a ladder or a block of `#minecraft:trapdoors` is
    /// `ladder`, then vine, the weeping and twisting vines (heads and plants), scaffolding, and any
    /// other climbable (cave vines, here) is `other_climbable`.
    #[test]
    fn a_trapdoor_counts_as_a_ladder_and_the_other_climbables_keep_their_own_place() {
        for (name, trapdoor, want) in [
            ("minecraft:ladder", false, "ladder"),
            ("minecraft:oak_trapdoor", true, "ladder"),
            ("minecraft:iron_trapdoor", true, "ladder"),
            ("minecraft:vine", false, "vines"),
            ("minecraft:weeping_vines", false, "weeping_vines"),
            ("minecraft:weeping_vines_plant", false, "weeping_vines"),
            ("minecraft:twisting_vines", false, "twisting_vines"),
            ("minecraft:twisting_vines_plant", false, "twisting_vines"),
            ("minecraft:scaffolding", false, "scaffolding"),
            ("minecraft:cave_vines", false, "other_climbable"),
        ] {
            assert_eq!(fall_location_of(name, trapdoor), want, "{name}");
        }
    }
}

/// Hits one mob's tick deals another, handled after the mob loop.
impl Game<'_> {
    /// A melee hit on another mob, then `postAttack`; whether it landed.
    pub(super) fn mob_attacks_mob(
        &mut self,
        target: u32,
        amount: f32,
        source: horizon_client::mobs::Damage,
    ) -> bool {
        let hurt = self.mob_hits_mob(target, amount, source);
        if hurt {
            self.mob_post_attack(
                target,
                source.attacker,
                None,
                Some(super::creatures::damage_type(source.kind, false)),
            );
        }
        hurt
    }
    /// Whether a hit from a mob's tick on `target`, handled after the mob
    /// loop, comes after a tick of the target that vanilla runs after the hit:
    /// `ServerLevel.tick` ticks entities in list order and the attacker deals
    /// the hit in its own tick, so a target later in the list counts its
    /// `HurtTime` and invulnerability down once more that tick. PARTIAL: the
    /// rest of that tick (its movement, its own goals) still ran before the hit.
    /// The caller compensates only newly reset timers: a stronger hit during
    /// cooldown leaves timers that already counted down this tick unchanged.
    pub(super) fn catch_up_countdowns(
        &self,
        attacker: Option<horizon_client::mobs::Who>,
        target: u32,
    ) -> bool {
        let Some(horizon_client::mobs::Who::Mob(attacker)) = attacker else {
            return false;
        };
        let ticked = &self.creatures.ticked;
        if !self.creatures.deferred_hits || !ticked.contains(&attacker) || !ticked.contains(&target)
        {
            return false;
        }
        // The mobs added during the tick come first in `self.mobs`; the
        // ticked ones keep their tick order after them.
        let at = |id: u32| self.mobs.iter().position(|m| m.id == id);
        matches!((at(attacker), at(target)), (Some(a), Some(t)) if a < t)
    }
}

#[cfg(test)]
mod damage_taken_tests {
    use super::damage_taken_award;

    #[test]
    fn tenths_rounded_as_java_does_and_nothing_for_a_huge_hit() {
        assert_eq!(damage_taken_award(2.0), Some(20));
        // Java rounds 0.5F up to 1: ties go up.
        assert_eq!(damage_taken_award(0.05), Some(1));
        assert_eq!(damage_taken_award(0.04), Some(0));
        // Below the threshold the count saturates, as Java's float rounding does.
        assert_eq!(damage_taken_award(3.4e37), Some(i32::MAX));
        assert_eq!(damage_taken_award(3.402_823_5e37), None);
        assert_eq!(damage_taken_award(3.4e38), None);
        assert_eq!(damage_taken_award(f32::INFINITY), None);
    }
}

#[cfg(test)]
mod held_item_tests {
    use super::has_custom_name;
    use horizon_client::items::ItemStack;
    use horizon_core::nbt::Tag;

    /// `DamageSource.getLocalizedDeathMessage`: only a non-empty stack with
    /// a `custom_name` component names the item.
    #[test]
    fn only_a_renamed_stack_names_the_item() {
        let mut sword = ItemStack::new("minecraft:diamond_sword", 1);
        assert!(!has_custom_name(&sword));
        // Other components (here a repair cost) are not a custom name.
        sword.data_mut().repair_cost = 1;
        assert!(!has_custom_name(&sword));
        sword.data_mut().custom_name = Some(Tag::String("Peacemaker".into()));
        assert!(has_custom_name(&sword));
        // An empty stack never does, even with the component.
        let mut gone = sword.clone();
        gone.count = 0;
        assert!(!has_custom_name(&gone));
    }
}
#[cfg(test)]
mod noted_tests {
    use super::{CombatTracker, PendingAttacker};
    use horizon_client::text::Text;

    /// A canceled hit forgets what was noted for it, also before its hurt
    /// started (a PvP blow, `/damage`), and nothing a landed hit noted.
    #[test]
    fn a_canceled_hit_forgets_only_its_own_attackers() {
        let mut c = CombatTracker::default();
        let arrow = |c: &mut CombatTracker, id| {
            c.mark();
            c.pending.push(PendingAttacker {
                damage_type: "minecraft:arrow".into(),
                attacker: (id, Text::literal("skeleton")),
                item: None,
                living: true,
                credit: true,
            });
        };
        arrow(&mut c, 1);
        c.forget_noted(false);
        arrow(&mut c, 2);
        c.mark();
        c.pending_direct
            .push(("minecraft:arrow".into(), Text::literal("arrow")));
        c.forget_noted(true);
        assert_eq!(
            c.pending.iter().map(|p| p.attacker.0).collect::<Vec<_>>(),
            [1]
        );
        assert!(c.pending_direct.is_empty());
        // A hit noted and landed, then one noted and canceled: the landed
        // one keeps its attacker.
        c.forget_noted(false);
        arrow(&mut c, 3);
        c.forget_noted(false);
        arrow(&mut c, 4);
        c.forget_noted(true);
        assert_eq!(
            c.pending.iter().map(|p| p.attacker.0).collect::<Vec<_>>(),
            [1, 3]
        );
    }
}
#[cfg(test)]
mod kill_credit_tests {
    use super::{CombatTracker, PlayerCredit};
    use horizon_client::text::Text;

    #[test]
    fn player_reference_expires_after_one_hundred_ticks() {
        let mut tracker = CombatTracker::default();
        tracker.note_hurt_by_player(PlayerCredit(123), 1_000);
        assert_eq!(
            tracker.player_credit(1_099).map(|credit| credit.0),
            Some(123)
        );
        assert!(tracker.player_credit(1_100).is_none());
    }

    /// `CombatTracker.getKiller`'s window: a hundred ticks after the hurt
    /// still counts (`getLastHurtByMob` answers), the next tick no longer
    /// does — the same `<= 100` a mob's hurt-by memory reads.
    #[test]
    fn the_hundredth_tick_after_the_hurt_still_counts_and_the_next_does_not() {
        let hurt_at = |at: i64| CombatTracker {
            last_hurt_by: Some(((7, Text::literal("zombie")), at)),
            ..CombatTracker::default()
        };
        assert_eq!(hurt_at(1_000).kill_credit(1_099), Some(7));
        assert_eq!(hurt_at(1_000).kill_credit(1_100), Some(7));
        assert_eq!(hurt_at(1_000).kill_credit(1_101), None);
    }
}

#[cfg(test)]
mod pet_credit_tests {
    use horizon_client::lan::playerdata::uuid_ints;
    use horizon_client::mobs::Mob;
    use horizon_core::geometry::Vec3;
    use std::sync::Arc;

    /// A tame wolf's hit on the player in place is its owner's
    /// (`resolvePlayerResponsibleForDamage` asks `instanceof Wolf`), so a
    /// content mod's type copying the wolf (#4031) names its owner too, and
    /// an untamed one names no one.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_tame_mod_wolf_types_hit_is_its_owners() {
        crate::game::native_entity_refs::tests::with_game(|game| {
            let wolf = game.res.mob_types().get("minecraft:wolf").cloned().unwrap();
            let mut hound = (*wolf).clone();
            hound.id = "ruby:hound".into();
            for (kind, tame, credit) in [
                (wolf.clone(), true, Some(12)),
                (Arc::new(hound.clone()), true, Some(12)),
                (Arc::new(hound), false, None),
            ] {
                game.combat.last_hurt_by_player = None;
                game.mobs.clear();
                let mut pet = Mob::new(7, kind, Vec3::ZERO, 0.0);
                pet.extra.tame = tame;
                pet.extra.owner = Some(uuid_ints(12));
                game.mobs.push(pet);
                game.resolve_player_responsible_for_player(7);
                assert_eq!(game.combat.last_hurt_by_player.map(|(c, _)| c.0), credit);
            }
        });
    }
}

#[cfg(test)]
mod guest_hit_tests {
    use horizon_client::mobs::{Mob, MobEvent};
    use horizon_client::motion::PlayerMotion;
    use horizon_core::geometry::Vec3;

    /// A bystander stands exactly under the mob that bites, listed first: the
    /// search by position ignores height and would have named the bystander.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn an_effect_hit_on_a_guest_credits_the_mob_that_swung() {
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 100.0, 0.5)).unwrap());
            for (id, kind, y) in [
                (4, "minecraft:zombie", 90.0),
                (5, "minecraft:cave_spider", 100.0),
            ] {
                let kind = game.res.mob_types().get(kind).cloned().unwrap();
                game.mobs
                    .push(Mob::new(id, kind, Vec3::new(1.5, y, 0.5), 0.0));
            }
            let from = [1.5, 100.0, 0.5];
            let hits = [
                MobEvent::AttackPlayerWithEffect {
                    amount: 2.0,
                    from,
                    effect: "minecraft:poison",
                    ticks: 140,
                    amplifier: 0,
                },
                MobEvent::StingPlayer {
                    amount: 2.0,
                    from,
                    poison_ticks: 200,
                },
            ];
            for hit in hits {
                game.combat.pending.clear();
                game.guest_mob_event(5, hit);
                let noted: Vec<u32> = (game.combat.pending.iter()).map(|p| p.attacker.0).collect();
                assert_eq!(noted, [5]);
                assert_eq!(game.combat.melee_mob, None, "the hit used it up");
            }
        });
    }
}

#[cfg(test)]
#[path = "combat/zombie_ignite_tests.rs"]
mod zombie_ignite_tests;
