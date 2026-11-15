//! The `living_visibility` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// How visible `entity` is to `looking` (NeoForge's
    /// `LivingVisibilityEvent`, last in `LivingEntity.getVisibilityPercent`):
    /// the range a mob's targeting check sees it from is scaled by
    /// `visibility`. It comes in as vanilla's percent (sneaking 0.8,
    /// invisibility by armour cover, a worn head's `mob_visibility`: 0.5 to
    /// its own mob type); multiply it to scale. Values below 0 count as 0,
    /// and the result is clamped to 10. Asked for any living target, by the
    /// mobs whose targeting checks scale by visibility (hostile targeting,
    /// sensors, the happy ghast's tempt). A mob target comes in without the
    /// sneaking factor, which vanilla reads through `isDiscrete`.
    /// Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        host = "read",
        neoforge = "LivingEvent.LivingVisibilityEvent"
    )]
    fn living_visibility();
}
