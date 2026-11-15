//! The `player_xp_level_change` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A server player's level is about to change through
    /// `Player.giveExperienceLevels`: once per level gained or lost by points,
    /// and for the anvil, `/experience add ... levels` and `PlayerRef::give_levels`.
    /// Change `levels`, or set `cancel` to leave the level unchanged. The
    /// player's state is swapped in, so `host.players().acting_player()`, `PlayerRef::tell`
    /// and the item helpers reach them. Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "PlayerXpEvent.LevelChange",
        cancellable = true
    )]
    fn player_xp_level_change();
}
