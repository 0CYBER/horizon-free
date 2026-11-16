/// Per-type behaviour switches derived from vanilla classes and tags.
#[derive(Clone, Copy, Debug, Default)]
pub struct Flags {
    /// WaterAnimal / fish: not affected by fluid physics (swims with its own control).
    pub aquatic_swimmer: bool,
    pub fall_immune: bool,
    /// FlyingMob-style travel (ghast, phantom): travelFlying without gravity.
    pub flying: bool,
    /// Spiders: onClimbable while colliding horizontally.
    pub climber: bool,
    /// Striders stand on lava.
    pub lava_walker: bool,
    /// EntityTypeTags.POWDER_SNOW_WALKABLE_MOBS, resolved at load.
    pub powder_snow_walkable: bool,
    /// LivingEntity.omnidirectionalAirMover (bees and parrots).
    pub omnidirectional: bool,
    pub fire_immune: bool,
    /// EntityTypeTags.BURN_IN_DAYLIGHT
    pub burns_in_day: bool,
    pub undead: bool,
    pub breathes_water: bool,
    /// Drowns out of water (fish, squid, dolphins after their air runs out).
    pub needs_water: bool,
    /// Hurt by water (endermen, blazes, striders out of lava, snow golems).
    pub water_sensitive: bool,
    pub animal: bool,
}
