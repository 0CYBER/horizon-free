//! Presentation of a registered dimension-transition screen.

pub trait Hooks {
    /// Draw the complete selected loading screen with its GUI drawing context. `screen` is the
    /// id registered at startup; ignore ids you do not own. `progress` is the integrated server's
    /// smoothed progress, or None without server progress. The game's existing tracker closes
    /// this screen when ready; drawing cannot change readiness or the timeout. Client only.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "none (vanilla Screen.render of the screen RegisterDimensionTransitionScreenEvent's factory builds; the event only registers it)"
    )]
    fn dimension_transition_screen_draw();
}
