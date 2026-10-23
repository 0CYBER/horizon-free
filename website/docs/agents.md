# Control Horizon with your AI agent

Your own AI agent can drive the launcher and the game: Claude Code, Codex, Cursor or any client that speaks MCP (the Model Context Protocol). It runs on your computer and uses the same commands you can type in a terminal. It can install a version, start the game, play it, change settings and manage your worlds. Before it deletes anything, it asks you. If you don't want any of this, one **No AI** switch hides and turns off every AI feature.

## What an agent can do

Everything you can do with the mouse and keyboard, through commands instead of clicks:

- **In the launcher:**
  - install, remove and prepare versions;
  - start the game, alone or with a world, or a server;
  - your settings and the extra game arguments;
  - the mod browser (mods, resource packs, shader packs, modpacks), imported modpacks and hosted servers;
  - your accounts (names only), logs and crash reports, patch notes, the data collection settings and the benchmark.
- **In the game:**
  - read what is on screen (every button, slider, field and slot, by name), click and type;
  - move, look, mine, place, attack, use items and pick from the hotbar;
  - the inventory, chests and every other container, crafting, trading, books and signs;
  - every options page, including Horizon Settings, and any option by its key;
  - worlds (create, open, rename, back up, delete), servers, LAN and Play with Friends;
  - chat and commands, screenshots, F3 data, waiting for an event and the game's log.

The full lists are in the repository: [launcher commands](https://github.com/ludovic111/horizon/blob/main/docs/launcher-commands.md) and [game commands](https://github.com/ludovic111/horizon/blob/main/docs/horizon-ctl-commands.md).

## Connect it from the launcher

The easiest way. Open the launcher's **Settings** and scroll to **AI**:

1. Under **Connect an AI agent**, pick your client: **Claude Code**, **Codex**, **Cursor** or **Any MCP client**.
2. For Claude Code and Codex, **Add to Claude Code** (or **Add to Codex**) runs that client's own command to add Horizon. The command is shown above the button, and runs only when you click it. For the others, **Copy** the config and paste it into the file named above it.
3. **Test the connection** starts Horizon's MCP server the way your client will and says how many tools it offers.
4. Restart your agent if it was open, then ask it: "Say hello from Horizon".

One server, called `horizon`, covers both the launcher and the game. Ask your agent to start the game and it starts it under its control. The game's tools are the ones whose names begin with `game_`.

## Copy-paste config

The launcher prints these with the exact path of your copy: `horizon-launcher agent-config`. The paths below are the usual ones:

- **macOS:** `/Applications/Horizon.app/Contents/MacOS/horizon-launcher`
- **Windows:** where you extracted the zip, for example `C:\Horizon\horizon-launcher.exe`
- **Linux:** where you put the AppImage or binary

**Claude Code**, in a terminal:

```sh
claude mcp add --scope user horizon -- /Applications/Horizon.app/Contents/MacOS/horizon-launcher mcp
```

**Codex**, in a terminal:

```sh
codex mcp add horizon -- /Applications/Horizon.app/Contents/MacOS/horizon-launcher mcp
```

Or in `~/.codex/config.toml`:

```toml
[mcp_servers.horizon]
command = "/Applications/Horizon.app/Contents/MacOS/horizon-launcher"
args = ["mcp"]
```

**Cursor** (`~/.cursor/mcp.json`), **Claude Desktop**, and **any other MCP client** that takes the usual JSON:

```json
{
  "mcpServers": {
    "horizon": {
      "command": "/Applications/Horizon.app/Contents/MacOS/horizon-launcher",
      "args": ["mcp"]
    }
  }
}
```

The server talks over its standard input and output, so it needs no port and is never on the network.

## A game you started yourself

A game your agent starts is under its control. A game you start with **Play** is not, unless you allow it. In the game, open **Options**, then **Horizon Settings**, then **AI Agent...**:

- **Agent Control: On** lets agents on this computer reach the game you started. It is off by default.
- The page shows whether an agent is connected and when it last sent a command.
- **Copy Connection Details** copies where the game listens, a command to try, and an MCP config for `horizon-ctl`, the game's own command line, which comes with the game.

Turning **Agent Control** off disconnects agents at once.

## The command line

The same commands, for scripts and terminals. `--json` prints JSON.

- See the versions: `horizon-launcher versions`
- Install the newest: `horizon-launcher install latest`
- Play a world: `horizon-launcher play --world "My world"`
- Change a setting: `horizon-launcher settings install-new-versions on`
- Find a shader pack: `horizon-launcher mods search --kind shader sildurs`
- Read the game's log: `horizon-launcher logs`
- Turn No AI on: `horizon-launcher no-ai on`
- Start a game agents control: `horizon-ctl launch --world Test`
- See the game's state: `horizon-ctl state`
- See what is on screen: `horizon-ctl ui`
- Click a button by name: `horizon-ctl click singleplayer`
- Walk somewhere: `horizon-ctl goto 10 64 -3`
- Take a screenshot: `horizon-ctl screenshot shot.png`

`horizon-ctl help` lists every game command. `horizon-launcher help` lists the launcher's.

## Safety

- **Your computer only.** The launcher's server talks only to the agent that started it. The game listens on a local socket that only your user account can open; where there is none (Windows), it uses a localhost port and a secret token in a file only you can read. Nothing listens on your network.
- **It asks before deleting.** Removing a version, a modpack, a mod or a server, deleting a world or your collected data, and signing out need `confirm: true` from the agent. A call without it is refused and tells the agent to ask you first.
- **It asks before reaching out.** The tools that send something or reach other people say "only when the player asks". That covers joining a server, chatting on one, opening to LAN, hosting for friends, creating a server and sending data or a bug report. Data collection stays off until you allow it yourself.
- **No secrets.** No command returns your account tokens, session files or invite secrets. Accounts appear as names. Signing in stays a code that you enter in your own browser.
- **Your agent, your rules.** Clients such as Claude Code ask you before each tool they haven't been allowed to use. Allow only what you are happy for them to do.

## No AI: turn it all off

In the launcher's **Settings**, under **AI**, turn on **No AI**. You can also use **No AI** in the game's **Horizon Settings**, or type `horizon-launcher no-ai on`. Then:

- the **Connect an AI agent** section and the game's **AI Agent** page are hidden;
- the game opens no control socket, even when it is started with one;
- `horizon-ctl`, `horizon-ctl mcp` and `horizon-launcher mcp` refuse with "AI features are off: turn off No AI in the launcher's settings";
- the launcher's own commands (install, play, server, update and the rest) keep working, so scripts and packagers don't break.

Turn it off the same way to bring everything back.
