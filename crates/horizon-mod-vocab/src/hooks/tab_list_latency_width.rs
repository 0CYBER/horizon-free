//! The `tab_list_latency_width` hook declaration (see `hooks/mod.rs`).

pub trait Hooks {
    /// Before the visible player list is laid out, request the width it allots
    /// to each entry's latency column. `width` is initially 13 GUI pixels. The
    /// final request is clamped to 13–128 pixels and limited by available space.
    /// Client-side; this does not run while the player list is hidden.
    #[hook(side = "client", contract = "listen")]
    fn tab_list_latency_width();
}
