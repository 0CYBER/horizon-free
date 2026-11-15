//! The `player_load_from_file` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0258

pub trait Hooks {
    /// A player was loaded from the world's save (NeoForge's
    /// `PlayerEvent.LoadFromFile`, `PlayerList.loadPlayerData`): their `.dat`
    /// has been read and applied, and they are not placed in a level yet.
    /// `directory` is the world's player directory (`players/data`): a mod
    /// reads its own file there, named `<uuid>.<suffix>` as
    /// `getPlayerFile(suffix)` names it. Heard once for each join, for the
    /// owner of an integrated server as for anyone else, on a server, with no
    /// player's state swapped in. Not cancellable.
    ///
    /// Not on wasm: it gives a path in the game's file system, which a wasm mod has none of; a wasm mod keeps a player's data in attachments.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "PlayerEvent.LoadFromFile (not cancellable)",
        cancellable = false
    )]
    fn player_load_from_file();
}
