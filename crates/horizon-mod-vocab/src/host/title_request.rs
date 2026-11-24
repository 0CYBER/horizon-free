//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;

/// The client host's methods of the `title_request` area, implemented by the game and reached through [`crate::ClientHost`].
#[horizon_mod_api_macros::host_area]
pub trait TitleRequestClientHost {
    /// Side: client.
    /// Asks the title for what its main page's buttons do, for a mod that draws its own page there (it cancels
    /// `screen_render_pre` on the `title` screen and takes the clicks in the screen input hooks): `WorldList`
    /// (Singleplayer), `CreateWorld`, `ServerList` (Multiplayer), `Connect(address)`, `Options`, `Mods`, `Quit` or `MainPage`. The
    /// request is queued, and the title's next frame runs it through the same path as the button, so a world list,
    /// a create screen or the server list opens over the title as usual and their Cancel or Back comes back to
    /// the main page. Only on the title's main page with no world open; `MainPage` goes from any title screen (the server
    /// list, a disconnect screen), for a mod that keeps its players on its own page.
    ///
    /// `Connect` joins as Direct Connection's Join does and keeps the address as the last direct one; only the
    /// calling mod's own bundle's `[native] world_control` grants it (default off), even if another enabled mod
    /// declares it, and a caller that is unidentified is refused with `NotPermitted`. The other requests are what
    /// any player can click on the title and need no permission.
    ///
    /// Answers `Queued` (accepted: the title's next frame does it), `NotTitle`, `NotPermitted`, `InvalidRequest`
    /// (a `Connect` address Direct Connection would not join) or `Busy` (another request is queued for the next
    /// title frame, or a world entry is already waiting for its resources).
    #[access(ClientSetup)]
    fn request_title(&mut self, request: TitleRequest) -> NativeTitleResult;
}
