//! `request_world(folder)` answers with a code, never a message (no UI text).

/// What `request_world` answered: a code, never a message (no UI text).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum NativeWorldResult {
    /// Accepted for a deferred confirmation: the next title frame asks the player. The world opens only if
    /// the player answers Yes; No or Escape leaves it closed. This is not a successful open.
    Queued,
    /// The title screen's main page is not up, or a world or another screen is.
    NotTitle,
    /// The calling mod's own bundle does not declare `world_control` in its `[native]` section, or the request's
    /// caller is unidentified (then it is refused whatever other enabled mods declare); another enabled mod's
    /// declaration does not count.
    NotPermitted,
    /// The folder is not one plain segment of 1 to 64 characters (no `/`, `\`, `:`, control character or
    /// leading dot), or `saves/<folder>` is not an existing save the title's world list recognizes. An empty
    /// directory is not one, and a link that leaves `saves/` does not count.
    InvalidRequest,
    /// Another request is queued for the next title frame, the confirmation question is on screen and
    /// unanswered, or a world entry is already waiting for its resources.
    Busy,
}
