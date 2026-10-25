# Launcher API

What a player's agent, a script or a pack maker can do with `horizon-launcher`: install and run versions, manage
accounts, settings, mods, modpacks and hosted servers, from a terminal or over MCP. The launcher itself:
[launcher/README.md](../../launcher/README.md).

## Overview

Every page of the launcher has a command, and every command but three has an MCP tool with the same arguments; a tool
runs the command with `--json`. Tools that change the player's things for good need `confirm: true`; with No AI on,
the MCP server is off.

| | Count |
| --- | --- |
| Commands | 46: versions, installs, play, the server, notes, links, accounts, settings, logs, updates, data collection, modpacks, mods, hosted servers, MCP, benchmark |
| MCP tools | 43, plus the game's tools as `game_<tool>` ([agent-control.md](agent-control.md)) |
| Modpack formats it imports | 3: a Modrinth `.mrpack`, a CurseForge zip, an instance folder |

## Guide

1. `horizon-launcher versions`, `horizon-launcher install latest`, `horizon-launcher play`.
2. Modpacks: `horizon-launcher modpack import <file>`, then `modpack port` and `modpack play`
   ([modpacks.md](../modpacks.md)).
3. For an agent: `horizon-launcher agent-config` prints how to connect a client to `horizon-launcher mcp`.

## Reference

[launcher-commands.md](../launcher-commands.md), generated from `launcher/src/cli.rs`: every command, its arguments
and its MCP tool.

## Examples

```sh
horizon-launcher versions --installed --json
horizon-launcher mods search minimap --kind mod --json
horizon-launcher servers create --name "Weekend" --player Alex --json
```

## Stability

Preview ([stability.md](stability.md)): command names, arguments and MCP tool names are the contract; `--json`
output shapes are not yet described in the reference.

## Not there yet

- **A Horizon modpack manifest.** A pack made for Horizon (not a port of a Java pack) needs its own file: the pack's
  id, name and version; the Horizon API lines and the release line it needs; its `.hmod` mods (id, version, SHA-256,
  where to download it, client, server or both); resource packs; default settings; and whether it has a server side.
  One manifest serves the launcher, `horizon-server` (a server installs the server side of the same pack) and the mod
  browser. Designed from what a pack maker does, versioned per [stability.md](stability.md), with a schema test.
- **JSON output schemas** for every command, in the generated reference, so MCP tools declare their output.
- **A version** in MCP `serverInfo` and `--json` output, as for agent control.
- **Compatibility tests**: golden `--json` output per command.

## Changelog

- Unversioned so far: the command table and the generated reference are the contract.
