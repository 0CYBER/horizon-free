//! The `entity_mob_griefing` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// May `entity` change the world? (NeoForge's `EntityMobGriefingEvent`,
    /// `EventHooks.canEntityGrief`.) `can_grief` starts as the `mob_griefing`
    /// game rule; set it either way. Asked for a mob
    /// explosion (`ExplosionInteraction.MOB`, for the creeper, wither, ghast
    /// fireball or wither skull), a mob picking up items, an enderman taking
    /// or placing a block, a sheep eating grass, a mob trampling
    /// farmland or turtle eggs, a ravager breaking leaves or crops, a wither
    /// breaking blocks or leaving a wither rose, a silverfish leaving or
    /// entering stone, a snow golem's snow, a fox's berries, a rabbit's
    /// carrots, a villager's farming, an evoker's sheep, a zombie trampling
    /// eggs, an allay's pickup, and a mob's arrow or small fireball changing
    /// a block (asked for its owner), a zombie breaking a door, the dragon
    /// breaking blocks, a ravager breaking pitcher crops, a burning mob
    /// melting powder snow, a weaving mob's cobwebs, and decorations and items
    /// in explosions (asked for the exploding entity, and for the mob that
    /// causes the blast). Asked often: answer fast. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        host = "read",
        neoforge = "EntityMobGriefingEvent"
    )]
    fn entity_mob_griefing();
}
