//! `request_title(request)`: a native mod that drew its own title main page asks for what the page's buttons do.

/// What a mod's title page asks for, as the title's own buttons do it.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum TitleRequest {
    /// Singleplayer: the world list, or Create New World when there is no save (`TitleScreen`'s button).
    WorldList,
    /// Create New World, whose Cancel comes back to the title's main page.
    CreateWorld,
    /// Multiplayer: the server list, behind its first-time safety screen unless that was turned off.
    ServerList,
    /// Joins the server at this address as Direct Connection's Join does (`host` or `host:port`, or an invite code),
    /// and keeps it as the last direct address. Needs the caller's `[native] world_control`.
    Connect(String),
    /// Options.
    Options,
    /// Mods: the list of mods, as the title's Mods button opens it.
    Mods,
    /// Quit Game.
    Quit,
    /// Back to the title's main page from any title screen (the server list, a disconnect screen, the world list),
    /// as their Back or Done goes: for a mod that keeps its players on its own page.
    MainPage,
}

/// What `request_title` answered: a code, never a message (no UI text).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum NativeTitleResult {
    /// Accepted: the title's next frame does it. Nothing happens inside the mod's call.
    Queued,
    /// The title's main page is not up, or a world or another screen is (for `MainPage`: no title screen is up).
    NotTitle,
    /// `Connect` from a mod whose own bundle does not declare `world_control` in its `[native]` section, or whose
    /// caller is unidentified; another enabled mod's declaration does not count.
    NotPermitted,
    /// `Connect` to an address Direct Connection would not join (not `host` or `host:port`).
    InvalidRequest,
    /// Another request is queued for the next title frame, or a world entry is already waiting for its resources.
    Busy,
}
