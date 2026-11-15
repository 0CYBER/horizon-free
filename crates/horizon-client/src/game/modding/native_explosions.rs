//! Explosions for native server mods (docs/modding/src/how-to/native-mods.md, "Explosions"): a mod
//! sets one off as `Level.explode(source, ...)` does, in the level in place, and the blast's
//! causing entity comes from its source as vanilla's `Explosion.getIndirectSourceEntity` has it.
use super::{
    Game,
    commands::Target,
    explosion::{BlockInteraction, ExplosionSource},
    explosion_settings::{BlastCalculator, BlastEffects, BlastSettings, SourceCalculator},
    minecarts::CartKind,
    native_hooks::{mod_particles_ok, sound_id},
};
use horizon_client::particles::{ExplosionParticle, ParticleOptions, ParticleType};
use horizon_client::projectiles::ProjectileKind;
use horizon_client::{
    gamerules::vanilla,
    mobs::{NamedDamage, Who},
};
use mod_api::{EntityRef, ExplodeError, ExplosionInteraction, ExplosionSettings};

/// Whether `native_explode` takes this centre and radius. Vanilla bounds
/// neither: any finite radius goes, one of 0 or below finding no blocks. A
/// blast's 1352 rays are fixed; its cost is in the blocks they find (their
/// steps grow with the radius, the distinct positions roughly with its cube).
fn blast_ok(center: [f64; 3], radius: f32) -> bool {
    center
        .into_iter()
        .all(|v| v.is_finite() && v.abs() <= 30_000_000.0)
        && radius.is_finite()
}

/// A particle type a blast takes by id: one vanilla has that needs no options
/// (no block state, item or geyser data).
fn blast_particle(id: &str) -> Result<ParticleOptions, ExplodeError> {
    let options =
        ParticleOptions::simple(ParticleType::from_name(id).ok_or(ExplodeError::InvalidParticle)?);
    if !mod_particles_ok(&options, None, |_| false, [0.0; 3], [0.0; 3], 0.0) {
        return Err(ExplodeError::InvalidParticle);
    }
    Ok(options)
}

impl Game<'_> {
    /// What `entity` names as a blast's source: None for a stale or foreign
    /// handle, or a player in another level than the one in place.
    pub(super) fn native_explosion_source(&self, entity: EntityRef) -> Option<(u32, Target)> {
        let target = self.native_entity_target(entity)?;
        if let Target::Guest(conn) = target {
            let hub = &self.lan.as_ref()?.hub;
            hub.guest(conn).filter(|g| hub.here(g))?;
        }
        Some((entity.id(), target))
    }

    /// Explodes at `center` in the level in place as `Level.explode(source,
    /// x, y, z, radius, fire, interaction)` does: the blocks it reaches break
    /// or react as `interaction` says (`native_explosion_detonate` sees them
    /// first), entities are hurt and pushed, `fire` lights a third of the air
    /// above solid blocks, and everyone near hears and sees it. `source` is
    /// the exploding entity, already resolved; `settings` the damage type,
    /// calculator, particles and sound. Refused, with nothing done, on a
    /// client's copy of the world, for a chunk that isn't loaded, a centre or
    /// radius that isn't finite, or settings `native_blast_settings` refuses.
    /// A blast inside `native_explosion_detonate` starts that hook again.
    pub(super) fn native_explode(
        &mut self,
        source: Option<(u32, Target)>,
        center: [f64; 3],
        radius: f32,
        fire: bool,
        interaction: ExplosionInteraction,
        settings: ExplosionSettings,
    ) -> Result<(), ExplodeError> {
        if self.serving.is_none() && self.options.dedicated.is_none() {
            return Err(ExplodeError::Unavailable);
        }
        if !blast_ok(center, radius) {
            return Err(ExplodeError::InvalidBlast);
        }
        let chunk = (
            (center[0].floor() as i32) >> 4,
            (center[2].floor() as i32) >> 4,
        );
        if !self.world.chunks.contains_key(&chunk) {
            return Err(ExplodeError::Unavailable);
        }
        let settings = self.native_blast_settings(settings)?;
        let id = source.map(|(id, _)| id);
        let interaction = self.native_blast_interaction(interaction, id);
        // Called from inside another blast (`native_explosion_detonate`), it
        // neither takes that blast's source, cause or flags nor leaves its own
        // behind: that blast gets them back after.
        let outer = self.take_blast_state();
        let mod_calculator = settings.calculator.is_some();
        self.combat.blast_settings = settings;
        if let Some((id, target)) = source {
            self.attribute_native_blast(id, target);
            // `damageCalculator == null ? makeDamageCalculator(source)`: a
            // calculator the mod passes replaces the source's.
            if !mod_calculator {
                self.native_blast_calculator(id, target);
            }
        }
        // Only the source's `isSilent` keeps the blast quiet.
        let blast =
            ExplosionSource::default().with_silence(id.is_some_and(|id| self.entity_silent(id)));
        let done = self.server_explode_from(center, radius, interaction, fire, blast);
        self.restore_blast_state(outer);
        done.map_err(|_| ExplodeError::Unavailable)
    }

    /// `settings` checked against the game's registries and resolved for the
    /// blast: a registered damage type, a finite knockback multiplier, a block
    /// tag that exists, particle types that need no options with finite
    /// numbers and weights the packet holds (each at least 1, their total at
    /// most `i32::MAX`), and a sound event id (a `minecraft:` one the server has).
    fn native_blast_settings(
        &self,
        settings: ExplosionSettings,
    ) -> Result<BlastSettings, ExplodeError> {
        let damage = match &settings.damage_type {
            Some(id) => Some(
                NamedDamage::registered(&self.health_resources.damage, id)
                    .ok_or(ExplodeError::UnknownDamageType)?,
            ),
            None => None,
        };
        let calculator = match settings.calculator {
            Some(c) => {
                let knockback = c.knockback_multiplier.unwrap_or(1.0);
                if !knockback.is_finite() {
                    return Err(ExplodeError::InvalidBlast);
                }
                let immune = match &c.immune_blocks {
                    Some(tag) => Some(
                        self.res
                            .items()
                            .block_tags
                            .get(tag)
                            .cloned()
                            .ok_or(ExplodeError::UnknownBlockTag)?,
                    ),
                    None => None,
                };
                Some(BlastCalculator {
                    explodes_blocks: c.explodes_blocks,
                    damages_entities: c.damages_entities,
                    knockback,
                    immune,
                })
            }
            None => None,
        };
        let given = settings.small_particle.is_some()
            || settings.large_particle.is_some()
            || settings.block_particles.is_some()
            || settings.sound.is_some();
        let effects = if given {
            let mut effects = BlastEffects::default();
            if let Some(id) = &settings.small_particle {
                effects.small = blast_particle(id)?;
            }
            if let Some(id) = &settings.large_particle {
                effects.large = blast_particle(id)?;
            }
            if let Some(list) = &settings.block_particles {
                // The packet writes each weight as an `int` and `WeightedList`
                // refuses a total past `Integer.MAX_VALUE`.
                let total = list.iter().try_fold(0i32, |total, p| {
                    total.checked_add(i32::try_from(p.weight).ok()?)
                });
                if total.is_none() {
                    return Err(ExplodeError::InvalidParticle);
                }
                effects.block_particles = list
                    .iter()
                    .map(|p| {
                        let numbers = p.weight > 0 && p.scaling.is_finite() && p.speed.is_finite();
                        if !numbers {
                            return Err(ExplodeError::InvalidParticle);
                        }
                        Ok(ExplosionParticle {
                            options: blast_particle(&p.particle)?,
                            scaling: p.scaling,
                            speed: p.speed,
                            weight: p.weight,
                        })
                    })
                    .collect::<Result<_, _>>()?;
            }
            if let Some(sound) = settings.sound {
                if !sound_id(&sound) || !self.native_sound_known(&sound) {
                    return Err(ExplodeError::UnknownSound);
                }
                effects.sound = sound;
            }
            Some(Box::new(effects))
        } else {
            None
        };
        Ok(BlastSettings {
            damage,
            calculator,
            source_calculator: SourceCalculator::Default,
            effects,
        })
    }

    /// `ServerLevel.explode`'s `ExplosionInteraction` to the blast's, for a
    /// blast whose source is `id`.
    pub(super) fn native_blast_interaction(
        &self,
        interaction: ExplosionInteraction,
        id: Option<u32>,
    ) -> BlockInteraction {
        match interaction {
            ExplosionInteraction::None => BlockInteraction::Keep,
            ExplosionInteraction::Block => self.destroy_type(vanilla::BLOCK_EXPLOSION_DROP_DECAY),
            ExplosionInteraction::Mob if self.can_entity_grief(id) => {
                self.destroy_type(vanilla::MOB_EXPLOSION_DROP_DECAY)
            }
            ExplosionInteraction::Mob => BlockInteraction::Keep,
            ExplosionInteraction::Tnt => self.destroy_type(vanilla::TNT_EXPLOSION_DROP_DECAY),
            ExplosionInteraction::Trigger => BlockInteraction::Trigger,
            _ => unreachable!("ExplosionInteraction: unhandled variant"),
        }
    }

    /// The blast's source `id` and its causing entity
    /// (`Explosion.getIndirectSourceEntity`): a primed TNT's owner; a living
    /// entity itself (a player; a mob, which `blast_attacker` falls back to;
    /// the dragon or an armour stand); a projectile's owner when that is
    /// living (one in the projectile list, a thrown trident, a firework
    /// rocket, a fishing bobber); anything else causes none. The damage
    /// source and kill credit follow the cause.
    pub(super) fn attribute_native_blast(&mut self, id: u32, target: Target) {
        self.combat.explosion_source = Some(id);
        // `getEntities(this.source, ...)`: the source is no victim of its own blast.
        self.combat.explosion_source_named = true;
        let owner = match target {
            Target::Tnt(_) => return self.set_tnt_blast_owner(id),
            Target::Player => Some((Who::Player, self.lan_acting)),
            Target::Guest(conn) => Some((Who::Player, Some(conn))),
            Target::Dragon(_) | Target::Stand(_) => {
                let kind = match target {
                    Target::Dragon(_) => "minecraft:ender_dragon",
                    _ => "minecraft:armor_stand",
                };
                self.combat.blast_owner = Some((id, self.explosion_entity_name(id, kind)));
                return;
            }
            Target::Projectile(_) => self
                .creatures
                .projectiles
                .iter()
                .find(|p| p.id == id)
                .and_then(|p| Some((p.owner?, p.guest))),
            Target::Rocket(_) => self
                .rockets
                .iter()
                .find(|r| r.id == id)
                .and_then(|r| Some((r.owner?, r.guest))),
            // A thrown trident's owner is a player: a guest, or the player in place.
            Target::Trident(_) => self.tridents.iter().find(|t| t.id == id).and_then(|t| {
                let in_place = self.player.as_ref().map(|p| p.entity_id as u32);
                (t.guest.is_some() || in_place == Some(t.owner)).then_some((Who::Player, t.guest))
            }),
            Target::Hook(_) => self.fishing_hook_by_id(id).map(|(_, caster)| match caster {
                Target::Guest(conn) => (Who::Player, Some(conn)),
                _ => (Who::Player, self.lan_acting),
            }),
            _ => None,
        };
        match owner {
            Some((Who::Player, guest)) => self.set_blast_owner(Some(Who::Player), guest),
            Some((Who::Mob(mob), _)) => self.combat.explosion_owner = Some(mob),
            None => {}
        }
    }

    /// The damage calculator the source `id` brings to its blast when the mod
    /// passes none (`ServerExplosion.makeDamageCalculator`), as the source's
    /// own blast has it: a primed TNT that went through a portal the
    /// portal-proof one (`PrimedTnt.explode`), a wind charge the tag one
    /// (`AbstractWindCharge.explode`). Other sources bring the default
    /// calculator. The rays take it from the blast state
    /// (`blast_calculator_step`).
    pub(super) fn native_blast_calculator(&mut self, id: u32, target: Target) {
        match target {
            Target::Projectile(_)
                if self.creatures.projectiles.iter().any(|p| {
                    p.id == id && matches!(p.kind, ProjectileKind::WitherSkull { dangerous: true })
                }) =>
            {
                self.combat.blast_resistance_cap = Some(0.8);
                self.combat.explosion_direct = Some(id);
            }
            // MinecartTNT.isPrimed (fuse > -1).
            Target::Cart(_)
                if self
                    .carts
                    .iter()
                    .any(|c| c.id == id && c.kind == CartKind::Tnt && c.fuse >= 0) =>
            {
                self.combat.rail_proof_blast = true;
                self.combat.explosion_direct = Some(id);
            }
            _ => {}
        }
        let calculator = match target {
            Target::Tnt(_) if self.tnts.iter().any(|t| t.id == id && t.used_portal) => {
                SourceCalculator::PortalProof
            }
            Target::Projectile(_)
                if self.creatures.projectiles.iter().any(|p| {
                    p.id == id
                        && matches!(
                            p.kind,
                            ProjectileKind::WindCharge | ProjectileKind::BreezeWindCharge
                        )
                }) =>
            {
                let items = self.res.items();
                let tag = items
                    .block_tags
                    .get("minecraft:blocks_wind_charge_explosions");
                SourceCalculator::WindCharge(tag.cloned().unwrap_or_default())
            }
            _ => return,
        };
        self.combat.blast_settings.source_calculator = calculator;
    }

    /// The blast going off now, cleared to a sourceless one.
    pub(super) fn take_blast_state(&mut self) -> BlastState {
        let c = &mut self.combat;
        BlastState {
            explicit_cause: c.explicit_explosion_cause.take(),
            blast_owner: c.blast_owner.take(),
            blast_guest: c.blast_guest.take(),
            explicit_direct: c.explicit_explosion_direct.take(),
            source: c.explosion_source.take(),
            direct: c.explosion_direct.take(),
            cause: c.explosion_cause.take(),
            source_named: std::mem::take(&mut c.explosion_source_named),
            in_water: std::mem::take(&mut c.explosion_in_water),
            hive_emptying: std::mem::take(&mut c.hive_emptying_blast),
            rail_proof: std::mem::take(&mut c.rail_proof_blast),
            cap: c.blast_resistance_cap.take(),
            owner: c.explosion_owner.take(),
            tnt_owner_player: std::mem::take(&mut c.blast_owner_player),
            settings: std::mem::take(&mut c.blast_settings),
        }
    }

    /// Puts back what `take_blast_state` cleared.
    pub(super) fn restore_blast_state(&mut self, s: BlastState) {
        let c = &mut self.combat;
        c.explicit_explosion_cause = s.explicit_cause;
        c.blast_owner = s.blast_owner;
        c.blast_guest = s.blast_guest;
        c.explicit_explosion_direct = s.explicit_direct;
        c.explosion_source = s.source;
        c.explosion_direct = s.direct;
        c.explosion_cause = s.cause;
        c.explosion_source_named = s.source_named;
        c.explosion_in_water = s.in_water;
        c.hive_emptying_blast = s.hive_emptying;
        c.rail_proof_blast = s.rail_proof;
        c.blast_resistance_cap = s.cap;
        c.explosion_owner = s.owner;
        c.blast_owner_player = s.tnt_owner_player;
        c.blast_settings = s.settings;
    }
}

/// The fields of `CombatTracker` that describe the blast going off now.
pub(super) struct BlastState {
    explicit_cause: Option<Option<super::explosion::ExplosionCause>>,
    blast_owner: Option<(u32, horizon_client::text::Text)>,
    blast_guest: Option<u32>,
    explicit_direct: Option<horizon_client::text::Text>,
    source: Option<u32>,
    direct: Option<u32>,
    cause: Option<horizon_client::advancements::EntityView>,
    source_named: bool,
    in_water: bool,
    hive_emptying: bool,
    rail_proof: bool,
    cap: Option<f32>,
    owner: Option<u32>,
    tnt_owner_player: bool,
    settings: BlastSettings,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blast_needs_a_finite_centre_and_radius() {
        assert!(blast_ok([0.5, 64.0, 0.5], 4.0));
        assert!(blast_ok([0.5, 64.0, 0.5], 0.0));
        assert!(blast_ok([0.5, 64.0, 0.5], -1.0));
        assert!(blast_ok([0.5, 64.0, 0.5], 48.0));
        assert!(blast_ok([0.5, 64.0, 0.5], 1.0e6));
        assert!(!blast_ok([0.5, 64.0, 0.5], f32::NAN));
        assert!(!blast_ok([0.5, 64.0, 0.5], f32::INFINITY));
        assert!(!blast_ok([f64::INFINITY, 64.0, 0.5], 4.0));
        assert!(!blast_ok([0.5, 64.0, 4.0e7], 4.0));
    }

    #[test]
    fn a_blast_takes_particle_types_that_need_no_options() {
        assert_eq!(
            blast_particle("minecraft:flame").map(|o| o.ty),
            Ok(ParticleType::Flame)
        );
        assert_eq!(
            blast_particle("minecraft:dust").map(|o| o.ty),
            Ok(ParticleType::Dust)
        );
        for id in ["minecraft:block", "minecraft:item", "test:spark", ""] {
            assert_eq!(
                blast_particle(id),
                Err(ExplodeError::InvalidParticle),
                "{id}"
            );
        }
    }
}
