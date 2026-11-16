//! The `mod_mismatch` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A saved mod list (code and content mods) was read while opening a world. Fires
    /// even when the list is empty or unchanged, never when absent, once per open.
    /// Versions use the loader's comparison; minecraft is excluded. Newly added mods
    /// are not differences. Push a mod id converted with `.into()` into `resolved`.
    /// Dispatch assigns the first resolving listener and preserves earlier markers.
    /// When a saved mod is no longer loaded, it fires before the world opens, on the
    /// side that asks (the client in singleplayer, the dedicated server), and a
    /// resolved id is taken off the question; otherwise it fires on the opening
    /// server before world-state restoration, after preparation.
    #[notes = r##"## Mod mismatch

A world records the code mods it was saved with at the root of `level.dat`, beside `Data`, under
`fml.LoadingModList`, the list NeoForge keeps: one `{ModId, ModVersion}`
compound per loaded mod, in load order, rewritten on every save of the world.
Its content mods go in the same form under `horizon.ContentModList`, beside the
namespaces of the mod block states registered when it was saved
(`horizon.BlockNamespaces`). The version is the mod's `horizon-mod.toml` `version`,
or `""` when the game doesn't know it. A world without either list (vanilla, or saved
before Horizon wrote one) is no mismatch: it fires nothing and gets the lists on its
next save.

`Hooks::mod_mismatch` runs whenever a saved list exists, even if empty or unchanged,
once per open. A saved mod that is no longer loaded (or a saved block namespace nothing
registers now) would lose its blocks, and what they hold, for good, so the world asks
before it opens: a blocking question on the client (Don't Open by default, Back Up and
Open, Open Without a Backup), and a refusal on a dedicated server unless
`--allowMissingMods` or `server.toml`'s `[missing_mods] allow = true` says to open it.
Then the hook fires before the world opens, on the side that asks (`HostSide::Client` in
singleplayer, the dedicated server's `HostSide::Server`), with no world loaded yet, and
each id a listener resolves is taken off the question. Otherwise it fires on the opening
server (the integrated one in singleplayer) before world-state restoration; preparation
has already upgraded the world at this boundary.
Versions compare using the loader's semantic version parser (omitted minor and patch
are zero); empty or invalid versions compare by exact text. `minecraft` is excluded.
Push `"your_mod".to_owned().into()` into `event.resolved` to mark a migration.
Dispatch records the first resolving mod in each marker's `resolver` and preserves
prior resolutions against deletion or replacement. Entries are logged sorted by id:
resolved at debug (self or other resolver), unresolved at warn. Only a missing mod
holds the open back; a changed version never does.
"##]
    #[hook(
        side = "both",
        contract = "collect",
        out = "resolved",
        collect_validator = "crate::game::mismatch_resolution::attribute",
        neoforge = "ModMismatchEvent"
    )]
    fn mod_mismatch();
}
