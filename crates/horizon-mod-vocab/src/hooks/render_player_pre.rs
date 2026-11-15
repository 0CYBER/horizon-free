//! The `render_player_pre` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Before a player is drawn (NeoForge's `RenderPlayerEvent.Pre`): once a frame for each other
    /// player in view and for this window's player in third person (and F5 front view), before its
    /// `render_living_pre`. `player` is its render state, `who` its profile name and whether it is
    /// this window's own player, and `draw` the frame `render_living_pre` gets. Set `canceled` to
    /// skip all of the player's drawing this frame, `render_living_pre`, `render_living_post` and
    /// `render_player_post` included. Left uncanceled, `hidden.hide(part)` (or
    /// `hidden.hide_named(name)`) leaves only that part out of the vanilla model this frame (after
    /// Skin Customization), as a mod drawing its own outer skin layer does: a hidden part takes its
    /// children with it (the hat goes with the head), while its armour, held items and other
    /// layers, the cape among them, keep every part.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "RenderPlayerEvent.Pre",
        cancellable = true
    )]
    fn render_player_pre();
}
