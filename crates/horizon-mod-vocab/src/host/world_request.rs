//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;

/// The client host's methods of the `world_request` area, implemented by the game and reached through [`crate::ClientHost`].
#[horizon_mod_api_macros::host_area]
pub trait WorldRequestClientHost {
    /// Side: client.
    /// Asks the player whether to open `saves/<folder>` the way the title screen's Play button does: the
    /// request is queued, and the title's next frame shows a question that names the mod and the save, with
    /// Yes and No. Yes runs Play's own checks (a locked world, links in the save, the session lock, recovery,
    /// an incompatible version, the backup before an upgrade or downgrade); No or Escape changes nothing.
    /// Only on the title's main page with no world open. Only the calling mod's own bundle's
    /// `[native] world_control` grants it (default off), even if another enabled mod declares it; a request
    /// whose caller is unidentified is refused with `NotPermitted`. Creating a world, and level create, unload
    /// or delete, are not in it.
    ///
    /// Not on wasm: a native mod's title request; a wasm mod has no title screen to ask on.
    ///
    /// Answers `Queued` (accepted: the title's next frame asks the player, and the world opens only on Yes),
    /// `NotTitle`, `NotPermitted` (also when the caller is unidentified), `InvalidRequest` (the folder is not one
    /// plain segment of 1 to 64 characters with no `/`, `\`, `:`, control character or leading dot, or
    /// `saves/<folder>` is not an existing save the title's world list recognizes: an empty directory is not one,
    /// a link that leaves `saves/` does not count, and nothing is created) or `Busy` (another request is queued for
    /// the next title frame, the question is on screen and unanswered, or a world entry is already waiting for its
    /// resources).
    #[access(ClientSetup)]
    fn request_world(&mut self, folder: &str) -> NativeWorldResult;
}
