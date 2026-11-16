//! The `render_player_post` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// After a player is drawn (NeoForge's `RenderPlayerEvent.Post`): once a frame for each player
    /// whose `render_player_pre` wasn't canceled, after its `render_living_post`, or right after its
    /// `render_living_pre` when only that one was canceled. `player`, `who` and `draw` are as
    /// `render_player_pre` has them, and what it draws is kept; `parts` are the posed parts
    /// `render_living_post` gets, None when the living pre hook was canceled. The player's name tag, leash,
    /// flames and shadow are drawn after it, as `render_living_post` says.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "RenderPlayerEvent.Post",
        cancellable = false
    )]
    fn render_player_post();
}
