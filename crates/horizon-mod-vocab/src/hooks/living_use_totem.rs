//! The `living_use_totem` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `LivingUseTotemEvent`: a killing blow on entity `entity` of type `kind`
    /// from `source` (named as `native_living_incoming_damage` names it) would be taken by the
    /// `death_protection` item `item` in its `hand` (main or off;
    /// `LivingEntity.checkTotemDeathProtection`). Set `cancel` and the entity gets no
    /// protection: it dies and the item is kept. A player or a mob holding a totem fires it; a
    /// source that bypasses invulnerability never reaches it. `player_totem` still
    /// hears a server player's totem that did save them, after this. The level is the hit's: its blocks, random and own
    /// attachments.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        host = "level",
        neoforge = "LivingUseTotemEvent",
        cancellable = true,
        version = 2,
        target = "horizon_client::game::native_hooks::living_use_totem::HitLevel<'_>::native_living_use_totem"
    )]
    fn living_use_totem();
}
