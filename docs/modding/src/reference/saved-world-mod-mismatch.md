# Saved-world mod mismatches

`Hooks::mod_mismatch` reports changed and removed saved mods, code and content mods alike,
whenever one of the world's saved lists exists, including empty and unchanged lists; newly
added mods and `minecraft` are excluded. It fires once per open.

A world records its code mods in `level.dat`'s `fml.LoadingModList` and its content mods in
`horizon.ContentModList`, beside the namespaces of the mod block states registered when it was
saved (`horizon.BlockNamespaces`) and the game commit that last saved it
(`horizon.LastSavedCommit`).

## When a saved mod is missing

A saved mod that is no longer loaded, or a saved block namespace that nothing registers now,
would lose its blocks, and what they hold, for good on the next save. So the world asks before
it opens:

- **Client:** a blocking question lists each missing mod with its saved version and what it
  loses. **Don't Open** is the default (and Escape) and leaves every file as it was.
  **Back Up and Open** zips the whole world into the backups folder first, shows where, and
  opens only once the copy exists; a backup that fails cancels the open and says why.
  **Open Without a Backup** opens at once.
- **Dedicated server:** it refuses to start that world, with one log line per missing mod and a
  last line naming the switch: `--allowMissingMods`, or `allow = true` under `[missing_mods]`
  in `server.toml` (off by default). With the switch it logs the same list as a warning and
  opens.

In that case the hook fires before the world opens, on the side that asks: `HostSide::Client`
in singleplayer, the dedicated server's `HostSide::Server`, with no world loaded yet. An id a
listener resolves is taken off the question, so a migration mod can let the world open without
asking. The open that follows uses this answer and does not fire the hook again. Otherwise the
hook fires on the opening server (the integrated one in singleplayer) before world-state
restoration, after world preparation.

## The event

Versions use the loader's semantic version comparison, padding omitted minor and
patch components with zero; invalid or empty versions compare as exact text.
Mark a migration with `event.resolved.push("my_mod".to_owned().into())`.
The first listener to mark an id owns its `resolver` attribution; later listeners
cannot erase or replace that marker. Markers for other ids are allowed.
Differences are logged sorted by mod id, resolved at debug with the resolver and
self/other distinction, unresolved at warn. The event itself cannot be cancelled; only a
missing mod holds the open back, a changed version never does.
The current mod lists are written on the next save, not when this event fires.
