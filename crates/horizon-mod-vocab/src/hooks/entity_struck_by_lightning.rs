//! The `entity_struck_by_lightning` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A lightning bolt is about to hit `entity` (NeoForge's
    /// `EntityStruckByLightningEvent`, before `Entity.thunderHit`): once per
    /// bolt strike for each entity whose box meets the strike box (3 blocks
    /// around, 9 up), players included. `bolt` is the lightning bolt. Set
    /// `cancel` to spare the entity: no fire or damage, and no conversion
    /// (pig, villager, mooshroom, creeper, copper golem). It still counts
    /// as struck for Channeling's advancement. Not asked for a visual-only
    /// bolt, which hits nothing. Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "EntityStruckByLightningEvent",
        cancellable = true
    )]
    fn entity_struck_by_lightning();
}
