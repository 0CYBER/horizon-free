# The manifest and capabilities

Every mod has a `horizon-mod.toml` at its root. It names the mod, says which version of the mod API it was made for, and lists its capabilities: what it may do. Players read that list before they turn the mod on, and the game holds the mod to it.

## The manifest

```toml
id = "playtime"
name = "Playtime"
version = "0.1.0"
description = "How long you have played, in a corner of the screen"
authors = ["You"]
api = "0.1"
side = "client"
capabilities = ["hud", "chat", "storage"]
```

| Key | What it is |
| --- | --- |
| `id` | The mod's id: 1 to 64 characters of `a-z`, `0-9` and `_`. It names the mod's setting, its log lines, its storage folder and, for content, its namespace. Keep it once you have published the mod. |
| `name` | The name players see. |
| `version` | Your mod's version, such as `0.1.0`. Each version you publish needs a new one. |
| `description` | One or two sentences for the Mods screen. |
| `authors` | A list of names. |
| `api` | The mod API line the mod was made for: `"0.1"` for code, `"0.2"` for content. The game refuses a line it doesn't serve, with a message that says so. |
| `side` | `"client"` for a mod that runs in the player's game. Content mods use `"both"`. |
| `capabilities` | What the mod may do: the list below. |
| `namespaces` | Content mods only: extra namespaces the mod's files use (see [Content mods](/docs/content-mods#namespaces)). |

Unknown keys are refused, so a typo is an error, not a permission that silently does nothing.

## Capabilities

A capability is a part of the API the mod may use. Each one shows on the Mods screen as a sentence that starts with "May". Writing to the game log needs no capability.

| Capability | The player reads | What the code can do |
| --- | --- | --- |
| `game-read` | May see your position, health and the time | `game::get_player()`: position, facing (yaw and pitch), health, food and dimension. `game::get_clock()`: the game time and the time of day. |
| `chat` | May show you chat messages | `chat::show(text)`: a chat line only this player sees, after the mod's name. At most 8 per call into the mod. |
| `hud` | May draw text on your screen | `hud::set_panel(corner, lines)` and `hud::clear()`: a panel of up to 16 coloured lines of 256 characters in one corner. The game keeps drawing it until you change it. |
| `storage` | May save its own settings to disk | `storage::get`, `set` and `delete`: small files in a folder of the mod's own. Up to 64 files and 4 MiB in all, and at most 1 MiB written per call. Keys are 1 to 64 characters of `a-z`, `0-9`, `_`, `.` and `-`. |
| `registries` | May add blocks and items to your worlds | Content mods: blocks and items from the mod's data files. No code. |

Ask only for what the mod uses: players are more likely to turn on a mod that asks for little.

**The game checks.** When you build, and again when a game loads the mod, the compiled code's imports are compared with the manifest. A mod whose code calls `chat::show` without `chat` in its capabilities is refused (`uses capability chat, missing from horizon-mod.toml`). The list players read is what the code can do, not a promise.

## What no mod can do

A mod runs in a WebAssembly sandbox. Whatever its capabilities:

- It cannot open files (apart from its own storage), the network, other programs or the clock.
- It sees the game only through the snapshot the game gives it once a tick, and changes it only through the calls above. It cannot patch the game's code.
- It has 64 MiB of memory and a time budget for each call. A mod that runs past its budget, runs out of memory or crashes is switched off, with the reason on its row of the Mods screen. The game goes on.

## API versions

The mod API is versioned apart from the game. A mod made for API `0.1` keeps loading on every game that serves `0.1`, and new functions arrive as additions. A change that would break mods comes as a new line, served beside the old one for at least one release cycle. The [stability page](/docs/api/stability) has the whole policy, and the [Mod API's changelog](/docs/api/mods#changelog) what each line added.

Next: [write the code](/docs/mod-code).
