//! The rule `ServerPlayer.broadcastToPlayer` applies to a player another client may be shown.

/// A target player is shown to a viewer who is a spectator (or not) of permission `permission`
/// when the target, if the viewer spectates, watches through its own camera, or, if the viewer
/// does not, is no spectator; and a target a native mod hid below level `hidden` only to
/// viewers of at least that level.
pub fn player_shown(
    spectator: bool,
    permission: u8,
    target_spectator: bool,
    target_watching: bool,
    hidden: Option<u8>,
) -> bool {
    let shown = if spectator {
        !target_watching
    } else {
        !target_spectator
    };
    shown && hidden.is_none_or(|level| permission >= level)
}
