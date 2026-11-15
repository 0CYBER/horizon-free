//! The `explosion_knockback` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// An explosion at `center` in `dimension`, about to break `blocks`, pushes
    /// `entity` (None for a player, whom `host.players().acting_player()` names):
    /// `ServerExplosion.hurtEntities`, after the knockback is calculated. Change `knockback_velocity` to change
    /// what is added to the entity's velocity; a player's client is sent the
    /// same. Wind bursts too, except dragons and end crystals. End crystals
    /// are also omitted from ordinary blasts: Horizon removes them without a
    /// velocity to change. Server-side.
    #[hook(
        side = "server",
        contract = "collect",
        out = "knockback_velocity",
        neoforge = "ExplosionKnockbackEvent",
        cancellable = false
    )]
    fn explosion_knockback();
}
