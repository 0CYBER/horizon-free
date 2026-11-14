//! What a mod's blast takes beyond vanilla's short `Level.explode` (#4009): its
//! damage type, its damage calculator as data (`SimpleExplosionDamageCalculator`)
//! and what players see and hear of it. `level_explode` resolves them once; the
//! combat state holds them while the blast goes off, and the rays, the entity
//! walk and the effects read them here. Any other blast has none of them and
//! goes off as before.
use super::Game;
use horizon_client::mobs::{DamageKind, NamedDamage};
use horizon_client::particles::{ExplosionParticle, ParticleOptions, ParticleType};
use std::collections::HashSet;

/// What `SimpleExplosionDamageCalculator` gives a block of its immune set.
const IMMUNE_RESISTANCE: f32 = 3_600_000.0;

/// A mod blast's own choices; each `None` is vanilla's.
#[derive(Default)]
pub(super) struct BlastSettings {
    /// The damage type, in place of the one the blast derives.
    pub(super) damage: Option<NamedDamage>,
    pub(super) calculator: Option<BlastCalculator>,
    /// The calculator the blast's source brings, used only when the mod
    /// passes none (`ServerExplosion.makeDamageCalculator`).
    pub(super) source_calculator: SourceCalculator,
    pub(super) effects: Option<Box<BlastEffects>>,
}

/// The resistance rule a mod blast's source brings with it.
#[derive(Default)]
pub(super) enum SourceCalculator {
    /// The default calculator.
    #[default]
    Default,
    /// `PrimedTnt.USED_PORTAL_DAMAGE_CALCULATOR`, a primed TNT's that went
    /// through a portal: a nether portal neither resists nor breaks.
    PortalProof,
    /// `AbstractWindCharge`'s calculator: the blocks of
    /// `#blocks_wind_charge_explosions` resist 3,600,000 and every other
    /// cell, fluids included, nothing.
    WindCharge(HashSet<String>),
}

/// `SimpleExplosionDamageCalculator(explodesBlocks, damagesEntities,
/// knockbackMultiplier, immuneBlocks)`.
pub(super) struct BlastCalculator {
    pub(super) explodes_blocks: bool,
    pub(super) damages_entities: bool,
    pub(super) knockback: f32,
    /// The immune tag's blocks: when given, the only resistance there is.
    pub(super) immune: Option<HashSet<String>>,
}

/// The particles and sound of `ServerLevel.explode`'s full overload.
#[derive(Clone)]
pub(super) struct BlastEffects {
    /// The centre particle of a small blast (`ServerExplosion.isSmall`).
    pub(super) small: ParticleOptions,
    pub(super) large: ParticleOptions,
    pub(super) block_particles: Vec<ExplosionParticle>,
    pub(super) sound: String,
}
impl Default for BlastEffects {
    /// What `Level.explode`'s shorter forms pass.
    fn default() -> Self {
        Self {
            small: ParticleOptions::simple(ParticleType::Explosion),
            large: ParticleOptions::simple(ParticleType::ExplosionEmitter),
            block_particles: ExplosionParticle::defaults(),
            sound: "minecraft:entity.generic.explode".into(),
        }
    }
}
impl BlastEffects {
    /// The centre particle: the small one for a radius under 2 or a blast
    /// that keeps its blocks.
    pub(super) fn emitter(&self, small: bool) -> &ParticleOptions {
        if small { &self.small } else { &self.large }
    }
}

impl Game<'_> {
    /// The damage type the blast deals: the mod's, else `DamageSources.explosion`'s
    /// (`minecraft:player_explosion` for a caused blast).
    pub(super) fn blast_damage_type(&self) -> &'static str {
        match self.combat.blast_settings.damage {
            Some(damage) => damage.name,
            None => super::creatures::damage_type(
                DamageKind::Explosion,
                self.blast_attacker().is_some(),
            ),
        }
    }
    /// The mobs' side of `blast_damage_type`: the mod's type, else the
    /// explosion kind, which tells a caused blast by its attacker.
    pub(super) fn blast_damage_kind(&self) -> DamageKind {
        let damage = self.combat.blast_settings.damage;
        damage.map_or(DamageKind::Explosion, DamageKind::Named)
    }
    /// What a ray's step through block `name` takes and whether the block is
    /// spared, when the blast's calculator overrides the resistance rule: the
    /// mod's calculator when it passes one, else its source's
    /// (`SourceCalculator`). An immune set (the mod's `immune_blocks`, a wind
    /// charge's tag) resists 3,600,000 in its blocks and nothing in every
    /// other cell, fluids included, and no part is asked; a portal-proof
    /// blast passes through a nether portal and spares it. None for the
    /// usual rule. The rays and every resistance read take it from here.
    pub(super) fn blast_calculator_step(&self, name: &str) -> Option<(Option<f32>, bool)> {
        let settings = &self.combat.blast_settings;
        let immune = match (&settings.calculator, &settings.source_calculator) {
            (Some(c), _) => c.immune.as_ref()?,
            (None, SourceCalculator::WindCharge(tag)) => tag,
            (None, SourceCalculator::PortalProof) => {
                return (name == "minecraft:nether_portal").then_some((None, true));
            }
            (None, SourceCalculator::Default) => return None,
        };
        let taken = if immune.contains(name) {
            Some(IMMUNE_RESISTANCE)
        } else {
            None
        };
        Some((taken, false))
    }
    /// `shouldBlockExplode`: false when the calculator explodes no blocks.
    pub(super) fn blast_explodes_blocks(&self) -> bool {
        let calculator = self.combat.blast_settings.calculator.as_ref();
        calculator.is_none_or(|c| c.explodes_blocks)
    }
    /// `shouldDamageEntity`.
    pub(super) fn blast_damages_entities(&self) -> bool {
        let calculator = self.combat.blast_settings.calculator.as_ref();
        calculator.is_none_or(|c| c.damages_entities)
    }
    /// `getKnockbackMultiplier`: 1 by default; the calculator's, or 0 for a
    /// flying player (`Abilities.flying`).
    pub(super) fn blast_knockback(&self, flying_player: bool) -> f32 {
        match &self.combat.blast_settings.calculator {
            Some(_) if flying_player => 0.0,
            Some(c) => c.knockback,
            None => 1.0,
        }
    }
    /// What players see and hear of the blast.
    pub(super) fn blast_effects(&self) -> BlastEffects {
        match &self.combat.blast_settings.effects {
            Some(effects) => (**effects).clone(),
            None => BlastEffects::default(),
        }
    }
}
