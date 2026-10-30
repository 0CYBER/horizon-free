# Survival multiplayer (SMP): the official server's mod

The official Horizon server (horizon.bunchy7s.dev) wants an organised survival game: a currency players earn in
game, a server market, safe player-to-player trades, small games of chance with that currency, and spawn, homes,
teleport requests, warps, land claims and rules.

**None of it is in Horizon.** The server people download stays a plain server, like vanilla's server jar: no SMP
command, no SMP setting in `server.toml`, nothing in the release build. The SMP is a **native mod**, `smp`, that
only the official server builds into its engine. What Horizon gains are general **server hook points** any native
mod can use (commands, players, messages, inventories, the world folder, then interactions, damage, menus): the
mod API grows, and a server owner who wants their own SMP, shop or claims mod can write it on the same points.

Status: design (this page); built in phases below.

## Architecture

| Part | Where | What |
|---|---|---|
| Server hook points | this repository, `crates/horizon-client/src/game/native_hooks.rs` ([native-mods.md](native-mods.md)) | stable, versioned (`NATIVE_HOOKS_API`), empty when no mod hooks them |
| The `smp` mod | [horizon-rust/horizon-mods](https://github.com/horizon-rust/horizon-mods), `native/smp/` | a horizon-mixins bundle: `runtime.rs` holds the rules and data (its own crate and tests, no game types), small helpers hook the points |
| The official server's engine | built from a Horizon release's commit with the bundle applied | `horizon-server`'s `[engine] path` points at it; the rest of the server is unchanged |

Why a native mod: it is the mod system Horizon has now (WebAssembly code mods were removed), it runs on the server
that simulates the world, and it keeps the SMP out of everyone else's game. Why hook points rather than reaching into
the game's private code: a mod on the hook points builds on any later Horizon (`"hooks": N`), so the official server
updates with each release without the mod being ported again.

Why not a datapack: no command arguments, no inventory transactions, no trade window, no way to refuse an action in a
claim.

### Hook points the SMP needs

Each is added in its own pull request, raising `NATIVE_HOOKS_API`, documented in [native-mods.md](native-mods.md),
and free when no mod hooks it (an empty function). Where a contributor's pull request already adds one (block
breaking and placing, item use: #4483, #4302), the SMP uses theirs.

| Phase | Hook points and helpers |
|---|---|
| 1. Economy | `native_register_commands` (commands every player's chat box completes), `native_perform_command` (a player's command on the server, with that player's state as the acting player); helpers: the online players, a line to one player or to all, the acting player's items (count, take, give), the world's folder |
| 2. Trading | a menu the server opens for a player, whose slots and clicks the mod rules; a player left or died |
| 3. Gambling | nothing new |
| 4. Organisation | teleport a player (across dimensions), damage taken, a player's interaction with a block or entity that a mod can refuse (with #4302 and #4483), the player's time played |

## Mojang's rules

The currency is earned in game only (the market, trades, playing): it can't be bought with real money and can't be
cashed out, and nothing random is sold for money. Games of chance take and pay only the in-game currency, within
limits the owner sets. Operators' corrections (`/eco`) are logged like every other transaction.

## Commands

Player commands work for every player; operator commands need the permission level shown.

| Command | Does |
|---|---|
| `/balance [player]`, `/bal` | your balance, or another player's |
| `/pay <player> <amount>` | gives money to an online player |
| `/baltop [page]` | the richest players |
| `/eco give\|take\|set <player> <amount>` (level 3) | operator corrections, logged |
| `/sell hand\|all`, `/buy <item> [count]`, `/worth [item]` | the server market, at the configured prices |
| `/trade <player>`, `/trade accept\|deny\|cancel`, `/trade money <amount>` | a two-sided trade window |
| `/coinflip <amount>`, `/coinflip accept\|deny <player>` | a 50/50 bet between two players, the money held until it is decided |
| `/lottery [buy <tickets>]` | tickets for the next draw; the pot goes to one ticket drawn at random |
| `/spawn`, `/setspawn` (level 2) | the server's spawn |
| `/sethome [name]`, `/home [name]`, `/delhome <name>`, `/homes` | up to `homes.max` homes |
| `/tpa <player>`, `/tpahere <player>`, `/tpaccept`, `/tpdeny` | teleport requests |
| `/warp [name]`, `/setwarp <name>`, `/delwarp <name>` (level 2) | warps set by operators |
| `/claim [radius]`, `/unclaim`, `/trust <player>`, `/untrust <player>`, `/claims` | land claims |
| `/rules` | the server's rules |

Teleports wait `teleport.warmup_seconds` and are cancelled when the player moves more than half a block or takes
damage.

**Trades.** `/trade Alex` asks Alex; once accepted, both see a 6-row window: their own 16 offer slots on the left, the
other's on the right (shown, never takeable), the money each offers, and a confirm button. Any change to either offer
clears both confirmations; when both have confirmed, the items and the money are swapped in one step of the server's
tick. An item is always in exactly one place (the inventory, the cursor, the player's own offer slots), and every way
out returns it to the player who put it in (closing the window, `/trade cancel`, disconnecting, dying, the other
player leaving): to their inventory, what doesn't fit dropped at their feet. Tests cover each way out.

**Claims.** A rectangle of columns in one dimension, owned by a player who trusts others by name. In a claim, players
who are neither the owner, trusted nor an operator can't break or place blocks, open containers, or use doors,
buttons and levers. Claims start empty: existing builds are not claimed for anyone, their owners claim them. Each
player may claim `claims.initial_blocks` plus `claims.blocks_per_hour` for each hour played (vanilla's `play_time`
statistic, so past hours count), up to `claims.max_blocks`. Spawn protection is a claim of `spawn.protection_radius`
around `/setspawn` that only operators change. Not covered at first: pistons, fluids, explosions and fire entering a
claim from outside.

## Configuration and data

Everything is in the world's folder, `world/horizon-smp/`, so a backup of the world carries it, and a world without
that folder is untouched: the mod creates it the first time it runs.

| File | Holds |
|---|---|
| `config.toml` | the settings below, written with their defaults on first run |
| `economy.json` | balances by player UUID, and names last seen |
| `homes.json`, `warps.json`, `spawn.json` | positions with their dimension |
| `claims.json` | claims: id, owner, dimension, corners, trusted UUIDs |
| `lottery.json` | the current draw's tickets |
| `audit.log` | one JSON line per transaction: time, kind, from, to, amount, items, balances after |

Each `.json` file is written whole to a temporary file, flushed and renamed over the old one, at the end of a server
tick that changed it (at most once a tick) and when the world closes: a crash leaves the old file or the new one,
never half of one. `audit.log` is appended before the balances change.

```toml
currency = "coins"
starting_balance = 0

[market]
# What the server pays (sell) and asks (buy) for one item; 0 closes that side.
items = { "minecraft:diamond" = { buy = 400, sell = 100 }, "minecraft:wheat" = { buy = 0, sell = 1 } }

[teleport]
warmup_seconds = 3
cooldown_seconds = 30
request_seconds = 60

[homes]
max = 3

[claims]
tool = "minecraft:golden_shovel"
initial_blocks = 1024
blocks_per_hour = 100
max_blocks = 100000

[spawn]
protection_radius = 16

[gambling]
enabled = false
max_bet = 1000
cooldown_seconds = 30
lottery_minutes = 60
ticket_price = 10
max_tickets = 100

[rules]
lines = ["Be kind.", "No griefing: claim your builds with a golden shovel."]
```

## Building the official server's engine

A native mod is built into the game it runs in ([native-mods.md](native-mods.md)). For the server: the source of the
release the server runs, `horizon-mixins apply` with the `smp` bundle, then the server build
(`cargo build --release --no-default-features -p horizon-client`), and `horizon-server`'s `[engine] path` set to
the result. When to deploy it on the official server is the owners' decision; the mod's README holds the exact
commands.

## Cost

With no mod hooking them, the hook points are empty functions. With `smp`, a server tick writes the files that
changed (none most ticks) and looks at the teleports waiting; a claim check is a hash lookup of the chunk's claims,
made only when a player acts on a block. Each phase measures a server tick with 55 bots (`tools/server_bench.py`).

## Phases

Each is pull requests under ~500 lines, each tested: the hook points here (unit tests, a game test that hooks
them), the mod in horizon-mods (its crate's tests, and a proof: a dedicated server built with the bundle and real
clients driven by `horizon-ctl`, in fresh folders).

1. **Economy**: command and player hook points; the mod's ledger, data files and audit log, `/balance`, `/pay`,
   `/baltop`, `/eco`; then the items helpers and the market (`/sell`, `/buy`, `/worth`).
2. **Trading**: the server menu hook points; `/trade`, with a test for each way out (close, cancel, disconnect,
   death, a full inventory, the other player leaving).
3. **Gambling**: `/coinflip` with escrow, then the lottery; limits, cooldown, logged.
4. **Organisation**: teleport and damage hook points, `/spawn`, homes, `/rules`, `/tpa`, warps; the interaction
   hook points, claims, spawn protection and limits that grow with time played.
