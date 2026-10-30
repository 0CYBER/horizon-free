# The dedicated server's anticheat

Horizon's dedicated server can check what players send beyond what vanilla checks. It is **off**: a
server that doesn't turn it on runs vanilla's checks only ("moved too quickly", "moved wrongly"...)
and costs nothing more. An owner turns it on in `server.toml`:

```toml
[anticheat]
enabled = true
alerts = true              # tell online operators (alert_level, default 2) in their chat
alert_level = 2
log = "logs/anticheat.log" # every flag, one line each; "" writes none

[anticheat.timer]          # each check has a table like this one; every key is optional
enabled = true
setback = 2
alert = 5
kick = 20
decay = 0.02
```

horizon-server checks the section before every start; the game reads it from `server.toml` when it
starts (`horizon-client --dedicated` run by hand reads the same file from its folder, if there is one).

## How a check acts

A check that fails **flags** the player: the check's violation level (VL) rises (by 1 for most flags) and
falls back by `decay` every second. From `setback` on, the move or action is cancelled and the player is
put back where the server last had them; from `alert` on, the console, the log and the online operators
are told (at most once a second per player and check); from `kick` on, the player is disconnected with
"Kicked by the server's anticheat". A threshold at 0 turns that action off. Nothing bans: the owner reads
the log and decides.

Every flag goes to `log`, with the time, the player, the check, its level, what it saw and what was done:

```
[2026-10-30 17:29:55] Steve failed timer (VL 5.9): 301 ms ahead, sent back
```

## Checks

| Check | Catches | How |
|---|---|---|
| `timer` | a sped-up game (more ticks a second than the server's clock) | vanilla's client ends each tick with ServerboundClientTickEndPacket, at the server's tick rate. Each one spends one tick of a balance that wall-clock time fills. A client more than `tolerance_ms` (300) ahead is flagged; a quiet connection banks at most `max_lag_ms` (1000), so a lag spike's backlog isn't flagged and a pause can't be saved for a burst. A slow server tick adds its own time before its backlog is counted. |

More checks (movement, combat, world, anti-xray) come in their own pull requests; this table lists the
ones in this version.

## Writing a check

- The rules are in `crates/horizon-anticheat` (no game state: unit-test them there); the hub runs them
  where it handles the packet (`crates/horizon-client/src/lan/`) and calls `Hub::flag`, which carries out
  setback, alert and kick (`lan/anticheat.rs`).
- A check is one entry in the `checks!` table of `horizon-anticheat/src/lib.rs`: its name in
  `server.toml`, its default thresholds and its own settings.
- Predict what vanilla allows (effects, ice, elytra, riptide, vehicles, knockback, levitation, slime,
  bubble columns, latency). A false positive is worse than a missed cheat: when unsure, don't flag.
- A clean player must cost nothing: no work per tick unless the check needs it, and none at all while the
  anticheat is off (`Hub::anticheat` is `None`).
- Original code only (CONTRIBUTING.md): written from vanilla 26.3 and public descriptions of what cheats
  do. Never read, copy or paraphrase another anticheat's source (Grim, NoCheatPlus, Vulcan, Matrix,
  Spartan, Paper's anti-xray...), its decompiled jars or docs quoting its code.
- Test it both ways in `tools/anticheat_e2e.py`: a horizon-bots cheat is caught, legit bots are never
  flagged, and nothing happens with the anticheat off.

```sh
cargo build --profile fast -p horizon-client --bin horizon-client
cargo build --profile fast -p horizon-bots
python3 tools/anticheat_e2e.py --bin-dir target/fast
python3 tools/server_bench.py --servers horizon --bots 60 --behaviour walk --anticheat   # its cost
```
