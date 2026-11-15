//! The `player_swap_hands` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0262

pub trait Hooks {
    /// Heard before a non-spectator player's swap-hands action.
    /// The two mutable stacks start as the off-hand and main-hand stacks;
    /// the hands receive the listener's outputs unless it cancels.
    /// slots. Set `cancel` to keep the items where they are: the player's
    /// client is sent both slots again. The player's
    /// state is swapped in, so `host.players().acting_player()`,
    /// `PlayerRef::tell` and the item helpers reach them. Once for each press,
    /// on a server: a LAN or dedicated server's players, the single-player
    /// owner and a LAN host. Not heard: the same swap made in an open inventory
    /// (hovering a slot and pressing the key, a menu click).
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        version = 2,
        neoforge = "LivingSwapItemsEvent.Hands"
    )]
    fn player_swap_hands();
}
