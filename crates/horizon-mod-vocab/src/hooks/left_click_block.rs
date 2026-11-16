//! The `left_click_block` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0058

pub trait Hooks {
    /// A player left-clicks the block `state` at `pos`, `face` the face hit
    /// (`[0, 1, 0]` is the top): where `ServerPlayerGameMode.handleBlockBreakAction`
    /// takes a dig's start, after its reach, spawn protection and world border
    /// checks, before `Block.attack` (a note block plays) or a creative break.
    /// Set `canceled` to ignore the click: the block isn't attacked, dug or
    /// broken on the server, and the player's client gets it back (its own
    /// predicted crack still shows to the clicking player). Server-side, for
    /// every player of the world (the helpers read the acting one).
    #[hook(
        side = "server",
        contract = "cancel",
        out = "canceled",
        neoforge = "PlayerInteractEvent.LeftClickBlock"
    )]
    fn left_click_block();
}
