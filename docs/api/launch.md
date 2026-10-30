# Launch API

What another launcher (Prism Launcher, MultiMC, ATLauncher, Modrinth App, HMCL or your own) needs to start Horizon:
the options `horizon-client` takes, how it hands over an account, and how it ends. `horizon-launcher` is the
reference implementation. Proposed in horizon-rust/horizon#3545.

## Overview

| | |
| --- | --- |
| Version | 0.1.0, printed by `horizon-client --launch-api` |
| Level | experimental ([stability.md](stability.md)): preview once preparing and the guide are covered too |
| Covers | starting `horizon-client`: its options, the session JSON, the exit codes, the crash report's path |
| Not yet | finding a build, preparing it with `horizon-prepare` (see Not there yet) |

## Guide

1. Start Horizon only for a player signed in with a Microsoft account that owns Minecraft: Java Edition, and
   hand over that account's session (`--session-stdin`, or `--session-file`), never on the command line. This
   is the same owner gate `horizon-launcher` applies before every start. Horizon never downloads Mojang's
   files; a launcher that does downloads them only for that owner.
2. Ask the build what it takes: `horizon-client --launch-api` prints JSON with `launch_api` (the version) and
   `options`. Pass only options it lists: any other is refused with exit code 2.
3. Start it with a folder per instance:

   ```sh
   horizon-client --resources PREPARED --game-dir INSTANCE --home INSTANCE --width 1280 --height 720 --session-stdin
   ```

4. Watch the exit code. On 2, stderr names the refused option (`Horizon: --width: …`). On 1 or 101, the last
   line on stderr names the crash report.

## Reference

[launch-reference.md](launch-reference.md), generated from `crates/horizon-client/src/game/launch_api.rs`: every
option, the `--launch-api` reply and the exit codes.

### The session JSON

The account the game joins online-mode servers with ([microsoft-account.md](../microsoft-account.md)):

```json
{"name": "Alex", "uuid": "069a79f444e94726a5befca90e38aaf5", "access_token": "…", "user_type": "msa", "xuid": "…"}
```

- `name`, `uuid` (with or without dashes) and `access_token` are required; `user_type`, when present, is `msa`;
  `xuid` is optional.
- `--session-stdin`: write the JSON to the game's standard input and close it.
- `--session-file PATH`: a file the game reads once and deletes at once. On Unix its mode must be 0600; a file
  older than two minutes is refused (and deleted).
- A session the game can't read is reported on stderr (`Horizon: not signed in: …`), and online-mode servers
  will refuse the player.

## Examples

```sh
horizon-client --launch-api
printf '%s' "$SESSION_JSON" | horizon-client --resources ~/horizon/prepared --game-dir ~/instances/a --session-stdin --fullscreen
```

## Stability

Experimental: the options in the reference, the session JSON, the exit codes and the crash report line may still
change before preview. From preview on, they change only as [stability.md](stability.md) says, and the reference's
test keeps the contract and the code together.

## Not there yet

- **Preparing:** `horizon-prepare` with `--output`, explicit inputs (`--version-json`, `--client-jar`,
  `--libraries-dir`, `--assets-dir`), one complete preparation, `--progress json` and `--check`.
- **Finding a build:** a signed release index and builds kept longer than seven days.
- **A guide**, "Running Horizon from another launcher", from picking a build to playing.

## Changelog

- 0.1.0: `--game-dir`, `--home`, `--fullscreen`, `--session-stdin`, `--launch-api`, exit code 2 for a refused
  command line.
