//! The `tab_list_latency_draw` hook declaration (see `hooks/mod.rs`).

pub trait Hooks {
    /// Draw at a visible player's latency icon paint slot. `player_uuid` and
    /// `player_name` identify the profile; `latency_ms` is its signed latency in
    /// milliseconds (negative means unknown). `bounds` is the clipped column as
    /// `[left, top, right, bottom]` in GUI pixels, with exclusive right and bottom
    /// edges. The slot can be empty on a narrow screen. Draw with `draw` and
    /// return `Flow::Cancel` to suppress only this player's vanilla ping icon.
    /// `Flow::Continue` leaves the icon to draw on top. Client-side.
    #[hook(side = "client", contract = "cancel", out = "canceled")]
    fn tab_list_latency_draw();
}
