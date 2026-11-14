//! The rest of vanilla's full `Level.explode` overload for `LevelRef::explode`: the damage
//! type, the damage calculator as data, the particles and the sound (#4009).

/// What a mod's blast takes beyond the source, centre, radius, fire and
/// interaction: vanilla's `Level.explode(source, damageSource, calculator, x,
/// y, z, radius, fire, interaction, smallParticles, largeParticles,
/// blockParticles, sound)`. Every field left `None` is vanilla's own choice,
/// so `ExplosionSettings::default()` is the short form.
#[derive(Clone, Debug, Default, PartialEq, horizon_mod_api_macros::Wire)]
// exhaustive: mods build it directly with `..Default::default()`, like the overload's arguments
pub struct ExplosionSettings {
    /// A registered damage type (`mymod:fuel_tank`): it names the death
    /// message and its tags decide armour, blast protection and immunity.
    /// `None` is the damage `explode` derives from the source
    /// (`minecraft:player_explosion` or `minecraft:explosion`).
    pub damage_type: Option<String>,
    /// The damage calculator (`SimpleExplosionDamageCalculator`); `None` is
    /// the source's own, as vanilla has it: a primed TNT that went through a
    /// portal passes through nether portals and spares them, a wind charge's
    /// blast resists 3,600,000 in `#blocks_wind_charge_explosions` and nothing
    /// in every other cell. Any other source, or none, takes the default one,
    /// which asks each block's parts and its own resistance. A calculator
    /// given replaces the source's.
    pub calculator: Option<ExplosionCalculator>,
    /// The centre particle of a small blast (a radius under 2, or one that
    /// keeps its blocks); `None` is `minecraft:explosion`.
    pub small_particle: Option<String>,
    /// The centre particle of any other blast; `None` is
    /// `minecraft:explosion_emitter`.
    pub large_particle: Option<String>,
    /// The particles the blast scatters through the air around it, one
    /// picked by weight for each; `None` is vanilla's `minecraft:poof`
    /// (scaling 0.5) and `minecraft:smoke`, one each. An empty list
    /// scatters none.
    pub block_particles: Option<Vec<ExplosionBlockParticle>>,
    /// The sound event (`mymod:fuel_tank_burst`); `None` is
    /// `minecraft:entity.generic.explode`.
    pub sound: Option<String>,
}

/// A damage calculator as data, vanilla's `SimpleExplosionDamageCalculator(
/// explodesBlocks, damagesEntities, knockbackMultiplier, immuneBlocks)`. The
/// damage an entity takes is the default formula. `Default` explodes blocks,
/// damages entities and leaves the rest `None`, as the default calculator does.
#[derive(Clone, Debug, PartialEq, horizon_mod_api_macros::Wire)]
// exhaustive: vanilla's four constructor arguments, built directly by mods
pub struct ExplosionCalculator {
    /// The blocks the rays reach break (or react, under `Trigger`); false
    /// finds none, so nothing breaks, burns or reaches `explosion_detonate`'s
    /// block list.
    pub explodes_blocks: bool,
    /// Entities in reach are hurt; false leaves their health alone (they are
    /// still pushed).
    pub damages_entities: bool,
    /// How hard entities are pushed; `None` is 1. A flying player
    /// (`Abilities.flying`, not creative mode) is pushed 0. An entity that is
    /// neither damaged nor pushed (`damages_entities` false and exactly 0) is
    /// not even checked for exposure.
    pub knockback_multiplier: Option<f32>,
    /// A block tag (`minecraft:logs`) whose blocks the blast never breaks.
    /// When given it replaces the whole resistance rule: its blocks resist
    /// 3,600,000 and every other cell resists nothing, obsidian and fluids
    /// included, and no part's `explosion_resistance` is asked.
    pub immune_blocks: Option<String>,
}

/// One entry of a blast's weighted block-particle list
/// (`ExplosionParticleInfo`). `Default` is vanilla's smoke entry:
/// `minecraft:smoke`, scaling 1, speed 1, weight 1.
#[derive(Clone, Debug, PartialEq, horizon_mod_api_macros::Wire)]
// exhaustive: vanilla's record of three values and a weight, built directly by mods
pub struct ExplosionBlockParticle {
    /// A particle type that takes no options (`minecraft:flame`).
    pub particle: String,
    /// How far from the centre it appears, as a share of the distance
    /// drawn for it (vanilla's poof has 0.5).
    pub scaling: f32,
    /// What its speed is multiplied by.
    pub speed: f32,
    /// Its weight in the pick, at least 1. The packet holds it as an `int`:
    /// a weight, or the list's total, past `i32::MAX` is refused.
    pub weight: u32,
}

impl Default for ExplosionCalculator {
    fn default() -> Self {
        Self {
            explodes_blocks: true,
            damages_entities: true,
            knockback_multiplier: None,
            immune_blocks: None,
        }
    }
}

impl Default for ExplosionBlockParticle {
    fn default() -> Self {
        Self {
            particle: "minecraft:smoke".into(),
            scaling: 1.0,
            speed: 1.0,
            weight: 1,
        }
    }
}

/// Why `LevelRef::explode` refused a blast. Nothing explodes and no hook
/// hears it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum ExplodeError {
    /// A client, a parked level or a stale handle, a `source` that is stale,
    /// names no entity or is in another level, or a centre in a chunk that
    /// isn't loaded.
    Unavailable,
    /// The centre is outside the world or not finite, or the radius or the
    /// knockback multiplier is not finite.
    InvalidBlast,
    /// The damage type is not registered.
    UnknownDamageType,
    /// The calculator's `immune_blocks` names no block tag.
    UnknownBlockTag,
    /// A particle is not a particle type that takes no options, or a block
    /// particle has a weight of 0 or past `i32::MAX` or a scaling or speed
    /// that is not finite, or the block particles' weights total past
    /// `i32::MAX`.
    InvalidParticle,
    /// The sound is not a sound event id, or is a `minecraft:` one the
    /// server does not have.
    UnknownSound,
}

impl crate::NoBackend for ExplodeError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}
