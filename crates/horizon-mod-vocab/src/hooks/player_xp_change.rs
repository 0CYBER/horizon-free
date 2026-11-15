//! The `player_xp_change` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A server player's experience points are about to change through
    /// `Player.giveExperiencePoints` (NeoForge's `PlayerXpEvent.XpChange`): an
    /// orb pickup after mending, an advancement reward, or `/xp add ... points`.
    /// Change the signed `amount` (in points) or set `cancel` to give nothing.
    /// Experience bottles and furnaces make orbs, so they come through the orb
    /// path. Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "PlayerXpEvent.XpChange",
        cancellable = true
    )]
    fn player_xp_change();
}
