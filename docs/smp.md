# Survival multiplayer (SMP): economy, trading, gambling, organisation

A survival server's own layer on Horizon's dedicated server: a currency players earn in game, a server
market, safe player-to-player trades, small games of chance with that currency, and the things an organised
server needs (spawn, homes, teleport requests, warps, land claims, rules). **Off by default**: a server
without `[smp] enabled = true` in `server.toml` plays exactly like vanilla 26.3, and nothing here touches the
world's files or anyone's inventory until the owner turns it on. Everything is additive: turning it on later
on an old world changes no block and no player; claims start empty, and owners claim their builds.

Status: design (this page), built in phases below.

## Architecture

Three ways were weighed:

| | Server mod (WebAssembly, API 0.3) | Datapack (scoreboards, `/trigger`) | **Built in, behind `[smp]`** |
|---|---|---|---|
| Commands | typed commands: yes | `/trigger` only, no arguments but a number | yes, in the command tree like vanilla's |
| Inventories (market, trades) | none: no read, no write | `/clear`, `/give`: no counting, no atomic swap | the guest's own slots, in the server's tick |
| A window for trades | none: no server-opened menus | none | the hub's container menus |
| Refusing a break, place or use (claims) | no: `block-broken` / `block-placed` come after the fact | no | the checks spawn protection already has |
| Teleports, damage, death | none | `/tp`, no events | yes |
| Data | 64 files, 4 MiB, in `horizon-mods/` | scoreboards in the world | `world/horizon-smp/`, any size |
| Cost | a sandbox call per event | functions every tick | a branch when off; native code when on |

**Chosen: built in.** The two features that must never go wrong, trades without duplication and claims that
refuse griefing, need the game's own code: an item has to be in exactly one place at every step of a trade
(inventory, cursor, trade slot), and a claim has to refuse an action before it happens, where vanilla's spawn
protection does (`ServerPlayer.mayInteractWith`). The mod API would have to grow server-opened menus,
inventory transactions and cancellable events first, as stable WIT, and the SMP's safety would still rest on
those host functions; a datapack can do neither. Server features are shared with the integrated server, so
this lives in this repository.

How it is split, so the rules stay small and testable without a game:

- **`crates/horizon-smp`**: no game types. The `[smp]` configuration, the ledger (balances, transfers,
  the audit log), the market's prices, the trade state machine, coinflips and the lottery, homes, warps,
  teleport requests, the claims and their index, the data files. Unit tests run in seconds.
- **`crates/horizon-client/src/game/smp*.rs`**: the glue on the server's path (the hub's players, as
  `as_guest` and spawn protection see them): commands and their tree, moving items, the trade window,
  teleports, the claim checks. `Game.smp` is `None` unless enabled: one branch per hook when off.
- **`crates/horizon-server`**: `server.toml`'s `[smp]` section, checked with the same types; the game
  reads it from `server.toml` in its working folder (the server folder) when it starts.

Later, the ledger and the claims can be lent to server mods (a `smp` capability: read a balance, ask for a
payment the player confirms), so mods extend the SMP rather than re-implement it.

## Mojang's rules

The currency is earned in game only (the market, trades, playing): there is no way to buy it with real money
and no way to cash it out, and nothing random is sold for money. Games of chance take and pay only the
in-game currency, within limits the owner sets. Operators' `/eco` corrections are logged like every other
transaction.

## Commands

Player commands work for every player; operator commands need the permission level shown.

| Command | Does |
|---|---|
| `/balance [player]`, `/bal` | your balance, or another player's |
| `/pay <player> <amount>` | gives money to an online player |
| `/baltop [page]` | the richest players |
| `/eco give\|take\|set <player> <amount>` (level 3) | operator corrections, logged |
| `/sell hand\|all [count]`, `/buy <item> [count]`, `/worth [item]` | the server market, at the configured prices |
| `/trade <player>`, `/trade accept\|deny\|cancel`, `/trade money <amount>` | a two-sided trade window |
| `/coinflip <amount>`, `/coinflip accept\|deny <player>` | a 50/50 bet between two players, money held by the server until it is decided |
| `/lottery [buy <tickets>]` | tickets for the next draw; the pot goes to one ticket drawn at random |
| `/spawn`, `/setspawn` (level 2) | the server's spawn |
| `/sethome [name]`, `/home [name]`, `/delhome <name>`, `/homes` | up to `homes.max` homes |
| `/tpa <player>`, `/tpahere <player>`, `/tpaccept`, `/tpdeny` | teleport requests |
| `/warp [name]`, `/setwarp <name>`, `/delwarp <name>` (level 2) | warps set by operators |
| `/claim [radius]`, `/unclaim`, `/trust <player>`, `/untrust <player>`, `/claims` | land claims |
| `/rules` | the server's rules |

Teleports (`/spawn`, `/home`, `/warp`, an accepted `/tpa`) wait `teleport.warmup_seconds` and are cancelled
when the player moves more than half a block or takes damage; they go through vanilla's `/tp` code, across
dimensions too.

**Trades.** `/trade Alex` asks Alex; once accepted, both players see a 6-row window: their own 16 offer slots
on the left, the other's on the right (shown, never takeable), the money each offers (`/trade money
<amount>`), and a confirm button. Any change to either offer, items or money, clears both confirmations. When both have
confirmed, the items and the money are swapped in one step of the server's tick. Items never leave the
server's sight: they are in the player's inventory, on their cursor or in their own offer slots, and every
way out returns them to the player who put them in (closing the window, `/trade cancel`, disconnecting,
dying, the other player leaving or going out of range): to their inventory, and what doesn't fit is dropped
at their feet as vanilla does when a crafting grid closes. Tests cover each of those ways out.

**Claims.** A claim is a rectangle of columns (bedrock to the build limit) in one dimension, owned by a player
who trusts others by name. In a claim, players who are neither the owner, trusted nor an operator cannot
break or place blocks, open containers, or use doors, buttons, levers and other blocks; they are told why
as spawn protection tells them. The golden shovel (`claims.tool`) marks two corners with a right click, then
`/claim` claims them; `/claim <radius>` claims the square around the player. Each player has
`claims.initial_blocks` plus `claims.blocks_per_hour` for each hour played (read from vanilla's
`play_time` statistic, so existing players start with their past hours), up to `claims.max_blocks`. Claims
cannot overlap. Spawn protection: an operator-owned claim of `spawn.protection_radius` around `/setspawn`.
Not covered at first: pistons, fluids, explosions and fire entering from outside a claim (follow-up).

## Configuration

`server.toml`, in the server folder. Every key has a default; only `enabled` is needed.

```toml
[smp]
enabled = true            # off by default: vanilla
currency = "coins"        # shown after amounts
starting_balance = 0

[smp.market]
enabled = true
# Prices per item: what the server pays (sell) and asks (buy); 0 leaves that side closed.
items = { "minecraft:diamond" = { buy = 400, sell = 100 }, "minecraft:wheat" = { buy = 0, sell = 1 } }

[smp.teleport]
warmup_seconds = 3
cooldown_seconds = 30
request_seconds = 60       # how long a /tpa waits for an answer

[smp.homes]
max = 3

[smp.claims]
enabled = true
tool = "minecraft:golden_shovel"
initial_blocks = 1024
blocks_per_hour = 100
max_blocks = 100000

[smp.spawn]
protection_radius = 16

[smp.gambling]
enabled = false
max_bet = 1000
cooldown_seconds = 30
lottery_minutes = 60       # 0: no lottery
ticket_price = 10
max_tickets = 100          # per player and draw

[smp.rules]
lines = ["Be kind.", "No griefing: claim your builds with a golden shovel."]
```

## Data

Under the world folder, `world/horizon-smp/`, so a backup of the world carries it:

| File | Holds |
|---|---|
| `economy.json` | balances by player UUID, and names last seen |
| `homes.json`, `warps.json`, `spawn.json` | positions with their dimension |
| `claims.json` | claims: id, owner, dimension, corners, trusted UUIDs |
| `lottery.json`, `playtime.json` | the current draw's tickets; time played since the SMP was on |
| `audit.log` | one JSON line per transaction: time, kind, from, to, amount, items, balances after |

Each `.json` file is written whole to a temporary file, flushed and renamed over the old one, at the end of a
tick that changed it, at most once per tick, and when the world saves: a crash leaves the old file or the new
one, never half of one. `audit.log` is appended before the balances change and rolled over to
`audit-<date>.log` at 16 MiB.

## Cost

With `[smp]` off, each hook is one check of `Game.smp`. With it on, a tick looks at the teleports that are
waiting (none most ticks) and the lottery's clock; a claim check is a hash lookup of the chunk's claims, made
only when a player breaks, places or uses a block. Each phase's PR measures a server tick with 55 bots
(`tools/server_bench.py`).

## Phases

Each phase is one or more pull requests under ~500 lines, each tested on its own:

1. **Economy**: the crate, `[smp]`, the data files and the audit log, `/balance`, `/pay`, `/baltop`, `/eco`;
   then the market (`/sell`, `/buy`, `/worth`).
2. **Trading**: `/trade` and its window, with a test for each way out (close, cancel, disconnect, death, a
   full inventory, the other player leaving).
3. **Gambling**: `/coinflip` with escrow, then the lottery; limits, cooldown, logged.
4. **Organisation**: `/spawn`, homes, `/rules`; `/tpa` and warps; claims and their checks; spawn protection
   and limits that grow with time played.

Tests: the crate's unit tests; the game's tests for the glue; `tools/smp_e2e.py`, a Horizon dedicated server
with `[smp]` on and real clients driven by `horizon-ctl` (fresh folders for everything), and the dedicated
server harnesses (`python3 tools/parity.py dedicated_server server_entities`) to show vanilla is unchanged
when `[smp]` is off.
