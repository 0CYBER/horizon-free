# APIs at a glance

Horizon is built to be built on. Mods, shaders, agents, server panels, launchers' scripts and tools all get a documented API with a version, a stability level and a reference generated from the code, so it never says something the game doesn't do.

## What you can build on

| Surface | For | Today | Stability |
| --- | --- | --- | --- |
| **Mods** | mod authors | Rust compiled to sandboxed WebAssembly: read the player and the clock, chat, a HUD panel, storage, commands, tick and frame hooks; data-driven blocks and items. Lua scripts, run in the same sandbox, are planned | preview |
| **Shaders** | shader authors | a new shader API in WGSL is being designed: stages with typed inputs, options, reload on save, a debug overlay | planned |
| **Agent control** | AI agents, scripts, tests | `horizon-ctl` and its MCP server: 96 commands to watch and play the game | preview |
| **Servers** | hosts, panels, bots | `horizon-server`'s commands, `server.toml` and a local HTTP control API | stable |
| **Launcher** | your agent, pack makers | `horizon-launcher`'s 46 commands and its MCP server | preview |
| **Data formats** | tools that read Horizon's files | benchmark reports, telemetry, settings, release checksums; worlds are vanilla's format | per format |

The full pages, one per surface, with their references: [the API portal](/docs/api), starting with [the Mod API](/docs/api/mods). Mods for players: [Mods](/docs/mods). Agents: [Control Horizon with your AI agent](/docs/agents). Hosting: [Host a server](/docs/hosting).

## What the levels mean

- **Stable**: only additions within a major version. A change that would break you comes as a new major, served beside the old one for at least 180 days.
- **Preview**: documented and tested. It may change at a new line (0.1 to 0.2), after a deprecation of at least 30 days during which old and new both work.
- **Experimental**: opt-in only, and it may change in any release.

Every API has its own version (the mod API is `horizon:mods@0.1.0`, the server's control API is `/api/v1`), separate from the game's release number. The whole policy: [stability and versioning](https://github.com/ludovic111/horizon/blob/main/docs/api/stability.md).

## How they are designed

Each API starts from what its users need to do, and from how Horizon is built: Rust, wgpu, WebAssembly and a server thread that runs every world. Horizon's APIs are its own: they are not modelled on another mod loader's, renderer's or server platform's. Vanilla Minecraft 26.3 is the reference for game concepts such as ids, commands and data packs.

They are safe by default (mods run in a sandbox with the permissions you allow; control sockets only listen on your computer, behind a token) and cost nothing when unused.

## References that can't drift

Each reference is written by code from the source it describes, and a test fails when the two differ: the mod API from its WIT contract, `horizon-ctl` and the launcher from their command tables, the server from its settings template and route table. A change to an API changes its reference in the same pull request.
