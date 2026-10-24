# Launcher commands

<!-- Generated from launcher/src/cli.rs by cli_tools::markdown(); a test fails when this file is out of date. Update it with: HORIZON_WRITE_DOCS=1 cargo test --manifest-path launcher/Cargo.toml the_launcher_reference -->

Every command of `horizon-launcher`, and the tool `horizon-launcher mcp` offers for it. A tool's arguments are the command's, in snake_case. A tool runs the command with `--json` and returns what it printed. The tools that change the player's things for good need `confirm: true`. The tools that send something or reach other people say "only when the player asks". The game's own tools, `game_*`, are in [horizon-ctl-commands.md](horizon-ctl-commands.md). With No AI on, `mcp` and `agent-config` are hidden and refuse; every other command stays. See [agent-control.md](agent-control.md#no-ai).

## `horizon-launcher versions`

List Horizon releases, or the installed versions.

    horizon-launcher versions

- `--installed`: Only the versions installed here (no network)

MCP: `versions`.

## `horizon-launcher install`

Download and install a release (a tag such as v2026.10.13, or "latest").

    horizon-launcher install <TAG>

- `TAG`

MCP: `install`.

## `horizon-launcher remove`

Remove an installed version.

    horizon-launcher remove <TAG>

- `TAG`

MCP: `remove`, needs `confirm: true`.

## `horizon-launcher prepare`

Prepare a version's resources from your own Minecraft install.

    horizon-launcher prepare

- `--minecraft`: The game folder of the official launcher, Prism Launcher, MultiMC, the Modrinth App, ATLauncher or GDLauncher (default: the launcher's, then every launcher's default)

MCP: `prepare`.

## `horizon-launcher play`

Start the client.

    horizon-launcher play

- `--world`: Open or create this world (a folder under saves) at once
- `--creative`: Creative mode for a new world
- `--join`: Join a friend's world with its invite code
- `--arg`: An extra argument for the game, passed through as-is (repeatable), for example --in-frame

MCP: `play`, then the `game_*` tools.

## `horizon-launcher server`

Run a dedicated server here; type commands into its console.

    horizon-launcher server

- `--dir`: The server's folder (default: the one the launcher uses)
- `--accept-eula`: Agree to the Minecraft EULA (https://aka.ms/MinecraftEULA)

MCP: `server`, `server_command`, `server_log`, `server_stop`.

## `horizon-launcher notes`

Show a release's patch notes (default: the newest).

    horizon-launcher notes [TAG]

- `TAG`

MCP: `notes`.

## `horizon-launcher links`

Show the Discord, sponsor and website links.

    horizon-launcher links

MCP: `links`.

## `horizon-launcher account list`

The signed-in accounts and which one plays.

    horizon-launcher account list

MCP: `account_list`.

## `horizon-launcher account sign-in`

Sign in with a Microsoft account in your browser (device code).

    horizon-launcher account sign-in

MCP: `account_sign_in`.

## `horizon-launcher account sign-out`

Sign out: the account's tokens leave the keychain.

    horizon-launcher account sign-out <WHO>

- `WHO`: Its name or UUID

MCP: `account_sign_out`, needs `confirm: true`.

## `horizon-launcher account select`

Play as this account.

    horizon-launcher account select <WHO>

- `WHO`

MCP: `account_select`.

## `horizon-launcher account offline`

Play offline, without an account.

    horizon-launcher account offline

MCP: `account_offline`.

## `horizon-launcher settings`

Show the launcher's settings, or set one (the Settings page): selected-version TAG, minecraft-dir PATH, server-dir PATH ("default" resets a folder), install-new-versions on|off, opening-animation on|off.

    horizon-launcher settings [KEY] [VALUE]

- `KEY` (selected-version, minecraft-dir, server-dir, install-new-versions, opening-animation)
- `VALUE`: The new value (with KEY)

MCP: `settings`.

## `horizon-launcher game-args`

Show or set the extra arguments the game starts with (for example --in-frame); quotes group a word with spaces in it.

    horizon-launcher game-args [ARGS]

- `ARGS`: Omit to show the current arguments

MCP: `game_args`.

## `horizon-launcher logs`

Print the end of the game's log (logs/latest.log in the game folder), or its newest crash report.

    horizon-launcher logs

- `--lines`: How many lines of latest.log
- `--crash`: The newest crash report instead

MCP: `logs`.

## `horizon-launcher update`

Look for a newer Horizon release and launcher, and install them.

    horizon-launcher update

- `--check`: Only report what is available

MCP: `update`.

## `horizon-launcher desktop-entry`

Linux: put Horizon in the applications menu (an entry and the icon, under $XDG_DATA_HOME), or take it out with --remove.

    horizon-launcher desktop-entry

- `--remove`: Remove the entry and the icon, and stop adding them back

MCP: `desktop_entry`.

## `horizon-launcher data status`

What is allowed, the install ID and how many items wait (the default).

    horizon-launcher data status

MCP: `data_status`.

## `horizon-launcher data allow`

Turn kinds on: performance, crashes, bugs, benchmarks, or all.

    horizon-launcher data allow <KINDS>...

- `KINDS`

MCP: `data_allow`, only when the player asks.

## `horizon-launcher data deny`

Turn kinds off: performance, crashes, bugs, benchmarks, or all.

    horizon-launcher data deny <KINDS>...

- `KINDS`

MCP: `data_deny`.

## `horizon-launcher data show`

Print exactly what would be sent.

    horizon-launcher data show

MCP: `data_show`.

## `horizon-launcher data delete`

Delete everything sent from here, and what waits, and take a new install ID.

    horizon-launcher data delete

MCP: `data_delete`, needs `confirm: true`.

## `horizon-launcher data send`

Send what is waiting now.

    horizon-launcher data send

MCP: `data_send`, only when the player asks.

## `horizon-launcher data bug`

Send a bug report you write; it is printed first, as it would leave.

    horizon-launcher data bug <TEXT>

- `TEXT`
- `--log`: Add the end of the game's log (scrubbed, without chat)
- `--yes`: Send without asking

MCP: `data_bug`, only when the player asks.

## `horizon-launcher data benchmark`

Upload a benchmark report (docs/benchmark-report.md); it is printed first.

    horizon-launcher data benchmark <FILE>

- `FILE`
- `--yes`

MCP: `data_benchmark`, only when the player asks.

## `horizon-launcher modpack list`

The imported packs and their coverage (the default).

    horizon-launcher modpack list

MCP: `modpack_list`.

## `horizon-launcher modpack import`

Import a .mrpack, a CurseForge zip or an instance folder, and write its coverage report.

    horizon-launcher modpack import <PATH>

- `PATH`

MCP: `modpack_import`.

## `horizon-launcher modpack port`

Get the jars of what this computer can port and carry them over.

    horizon-launcher modpack port <PACK>

- `PACK`: The pack's folder name, as list shows it
- `--from`: A folder with your own jars (an installed instance, a downloads folder)

MCP: `modpack_port`.

## `horizon-launcher modpack play`

Play a ported pack as its own profile.

    horizon-launcher modpack play <PACK>

- `PACK`

MCP: `modpack_play`.

## `horizon-launcher modpack remove`

Remove an imported pack: its jars, ports and the profile's saves.

    horizon-launcher modpack remove <PACK>

- `PACK`

MCP: `modpack_remove`, needs `confirm: true`.

## `horizon-launcher mods list`

What the browser installed (the default).

    horizon-launcher mods list

MCP: `mods_list`.

## `horizon-launcher mods search`

Search for packs Horizon loads.

    horizon-launcher mods search [TEXT]...

- `TEXT`
- `--kind` (mod, resource, shader, modpack)
- `--everything`: Also what was made for other versions or runs only partly, each marked
- `--source` (modrinth, curseforge)

MCP: `mods_search`.

## `horizon-launcher mods check`

Say how well a Java mod runs in Horizon: its jar is downloaded (checksum checked), carried over and read; the answer is kept.

    horizon-launcher mods check <PROJECT>

- `PROJECT`
- `--everything`: Allow a version made for another Minecraft version (1.21 and later)
- `--source` (modrinth, curseforge)

MCP: `mods_check`.

## `horizon-launcher mods install`

Install a pack and what it needs, by its id or slug (checksums checked); a Java mod is ported into horizon-mods and turned on; a modpack is imported.

    horizon-launcher mods install <PROJECT>

- `PROJECT`
- `--everything`: Allow a version made for another Minecraft version
- `--source` (modrinth, curseforge)

MCP: `mods_install`.

## `horizon-launcher mods update`

Update what the browser installed (or one pack).

    horizon-launcher mods update [PROJECT]

- `PROJECT`
- `--check`: Only say what is newer

MCP: `mods_update`.

## `horizon-launcher mods remove`

Remove a pack the browser installed.

    horizon-launcher mods remove <PROJECT>

- `PROJECT`

MCP: `mods_remove`, needs `confirm: true`.

## `horizon-launcher servers list`

The servers, their folder and whether they run.

    horizon-launcher servers list

MCP: `servers_list`.

## `horizon-launcher servers create`

Create a server; with no options it is a new world, survival, normal, 10 players, friends only.

    horizon-launcher servers create

- `--name`
- `--seed`: The new world's seed (default: random)
- `--copy-world`: Start from a copy of this world folder (one of your saves)
- `--mode` (survival, creative, adventure, spectator)
- `--difficulty` (peaceful, easy, normal, hard)
- `--max-players`
- `--open`: Let anyone in (no whitelist)
- `--check-accounts`: Check players' Minecraft accounts (they must sign in with Microsoft)
- `--player`: Who may join (repeatable); the first becomes an operator
- `--accept-eula`: Agree to the Minecraft EULA (https://aka.ms/MinecraftEULA)

MCP: `servers_create`, only when the player asks.

## `horizon-launcher servers set-up`

Copy an installed, prepared version's programs and game files for the servers (again).

    horizon-launcher servers set-up

MCP: `servers_set_up`.

## `horizon-launcher servers run`

Run a horizon-server command on a server, for example: servers run "My server" -- status.

    horizon-launcher servers run <SERVER> -- <ARGS>...

- `SERVER`: The server's name or folder
- `ARGS`

MCP: `servers_run`.

## `horizon-launcher servers forget`

Take a server off the list (its folder stays).

    horizon-launcher servers forget <SERVER>

- `SERVER`

MCP: `servers_forget`, needs `confirm: true`.

## `horizon-launcher mcp`

Serve the Model Context Protocol on stdin and stdout, for AI agents (refused while No AI is on).

    horizon-launcher mcp

MCP: none: this command is the MCP server.

## `horizon-launcher agent-config`

How to connect an AI agent (Claude Code, Codex, Cursor, any MCP client) to `mcp`: the config to paste and the command that adds it (refused while No AI is on).

    horizon-launcher agent-config

- `--client` (claude-code, codex, cursor, generic): Only this client
- `--test`: Start the MCP server as a client would and say whether it answered

MCP: none: it prints how to connect a client to `mcp`.

## `horizon-launcher no-ai`

Show or set No AI: on hides and turns off every AI feature (the companion, agent control, horizon-ctl and both MCP servers); off, the default, makes them available.

    horizon-launcher no-ai [STATE]

- `STATE` (on, off): Omit to show the current state

MCP: none: the player's switch.

## `horizon-launcher benchmark`

Measure the launcher: starts (the first is the cold one), page switching, idle CPU and memory, the release list fetch; writes a benchmark report to benchmarks/ in its data folder.

    horizon-launcher benchmark

- `--repeats`: How many times to start it

MCP: `benchmark`.
