//! The `effect_particle_modification` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `EffectParticleModificationEvent`, vanilla's `LivingEntity.updateSynchronizedMobEffectParticles`: asked for each active effect `effect` of the living entity `entity` (a [`LivingRef`], a mob or a player), the ones vanilla hides included, as the server is about to send their swirls in its synched data (`DATA_EFFECT_PARTICLES`, `DATA_EFFECT_AMBIENCE_ID`). `original_particle` is vanilla's for the effect: an `entity_effect` in its colour (alpha 38 of 255 when ambient), the omens' own particles. `particle` starts as it: replace it, or set it to `None` to go back to the original. `visible` starts as the effect's own: set it to show a hidden effect's swirl or hide a visible one. The visible effects' particles are sent in the order the effects are kept; particles that need a block state, and item, trail and vibration particles, can't be sent and are left out. A mod's own particle (`ParticleOptions::simple` with its registered `ParticleKey`) goes only to Horizon clients: the others get the list without it. Whether the swirls are ambient (drawn fainter and rarer) follows from the answers as vanilla's `areAllEffectsAmbient` does: a visible effect that is not ambient makes it false, an effect a mod made visible counts as visible and one it hid doesn't, and it is true when none is visible. Asked when the entity's effects change (one is added, removed, or changes its amplifier, visibility or ambience), as vanilla recomputes them only then, and only while a mod listens; the answers are sent until the next change. The window's own player draws its swirls from its own effects, not from these answers. Server-side.
    #[notes = r#"## Effect particles

`effect_particle_modification` (NeoForge's `EffectParticleModificationEvent`) is asked once for each active
effect of a living entity, hidden ones included, and changes the swirl the server sends for it (`DATA_EFFECT_PARTICLES`):
the mobs and the players every client is sent. A listener replaces the effect's `particle`, sets it to `None` for the
original, or flips `visible`, which can show a hidden effect's swirl. A named particle goes only to Horizon clients;
vanilla clients (whose brand isn't Horizon's) get the list without it. `DATA_EFFECT_AMBIENCE_ID` is decided from the
result, as `areAllEffectsAmbient`: only effects visible after the mods count. As in vanilla it is asked when the
entity's effects change, not every tick, and the answers are sent until the next change; with no listener nothing is
kept and vanilla's list goes out. The window's own player draws its swirls from its own effects."#]
    #[hook(
        side = "server",
        contract = "listen",
        host = "read",
        neoforge = "EffectParticleModificationEvent",
        cancellable = false
    )]
    fn effect_particle_modification();
}
