# Horizon

A Rust rewrite of Minecraft: Java Edition 26.3 that aims for one-to-one vanilla parity, checked against the game itself with differential tests.

**Status:** in development, no releases. The native Linux client is playable; Java parity is 80% on `main` (see [Project status](#project-status)).

> Not an official Minecraft product. Not approved by or associated with Mojang or Microsoft. Horizon contains no Minecraft code, assets or data: you need your own legally obtained copy of Minecraft: Java Edition 26.3.

## What it does

Horizon reimplements the Minecraft Java client and its game logic in Rust, one subsystem at a time, and compares each piece with the vanilla game. Each `tools/test_*_parity.py` harness drives the same inputs through Horizon and through the vanilla client and server bytecode from your own copy of the game, then checks that the outputs are identical. It is for players and developers interested in game reimplementation, and for anyone who wants to see how far a behaviour-exact native rewrite can go.

The [native client](docs/native-client.md) opens on a vanilla-style title screen with world creation and selection, and generates worlds with the Rust port of the vanilla generators: noise terrain, aquifers, surface rules, carvers and placed features, all checked against vanilla. It streams and lights chunks on worker threads and renders every vanilla block model with smooth lighting, biome tints, the vanilla sky, sun, moon, stars and clouds. Gameplay includes:

- validated player physics, health and hunger;
- mining with vanilla tool rules and loot tables;
- placing, crafting, chests and furnaces;
- dropped items, particles and sounds;
- passive and hostile mobs with combat, including the wither and the ender dragon fight;
- villagers that take jobs, trade, restock, meet at the bell and sleep in beds;
- boats, minecarts, item frames, paintings and armor stands;
- shields, maps, books, fireworks, beacons, advancements and statistics;
- Anvil saves, joining vanilla servers, and opening a world to LAN.

## Getting it

There are no binary releases: Horizon builds from source and prepares its resources from your own copy of the game.

**Platform.** Linux on x86-64 with an NVIDIA GPU (Vulkan). macOS and Windows are out of scope.

**Requirements.** Rust 1.90 or newer, Python 3.10 or newer, JDK 25 (for resource preparation) and the ALSA development headers (`libasound2-dev` on Debian and Ubuntu).

### Bring your own copy

Horizon contains no game files and never downloads them. It builds its resources from **your own copy** of Minecraft: Java Edition 26.3, verified against the checksums in `research-lock.json`:

1. **Client:** install Java Edition with the official launcher and play 26.3 once. Horizon reads `versions/26.3/`, `libraries/` and `assets/` from the launcher's game directory (`~/.minecraft` on Linux). If yours lives elsewhere, pass `--minecraft-dir` or set `HORIZON_MINECRAFT_DIR`.
2. **Server jar:** download the 26.3 server jar yourself, from [minecraft.net/download/server](https://www.minecraft.net/en-us/download/server) while 26.3 is the current release, or otherwise from the `downloads.server.url` in your launcher's `versions/26.3/26.3.json`. Pass it with `--server-jar` or set `HORIZON_SERVER_JAR`. It is only needed the first time.

### Build and run

```sh
git clone https://github.com/ludovic111/horizon.git
cd horizon
python3 tools/prepare_native_client.py --prepare --server-jar ~/Downloads/server.jar   # needs JDK 25
python3 tools/fetch_sounds.py                      # optional: sounds from your launcher install
cargo run --release -p horizon-client
```

Everything prepared lands in the gitignored `.local/`. It is derived from the game: never commit it or share it.

## Usage

The client starts on the title screen. Command-line options (`--world`, `--seed`, `--creative`, `--render-distance`), controls, scripting through `horizon-ctl`, and packaging for another of your own Linux machines are described in [the native client guide](docs/native-client.md).

## Documentation

- [Native client](docs/native-client.md): running, options, controls, scripting and architecture.
- [Rust rewrite status](docs/rust-parity.md): completion criteria and per-area scope.
- Parity notes: [block behaviours and ticking](docs/parity/blocks.md), [section meshing](docs/parity/meshing.md).
- [Decompilation and Rust research lab](docs/research-lab.md): the local game inspection workspace and the first research crates.
- [Decompilation status](docs/decompilation-status.md).
- [Java performance packs](docs/java-packs.md): the earlier, no longer developed Java track.

## Project status

Parity is **not** complete. The table below is the project's own audit of each subsystem; work that is not yet merged into `main` raises its own audit to 84%.

### Java parity: 80%

Each subsystem scores 0 (absent), 0.25 (started, not playable), 0.5 (playable,
major vanilla features missing), 0.75 (playable, mostly complete) or 1 (complete
and verified against vanilla by differential tests). The percentage is the
average; `tests/test_parity.py` keeps it in sync with this table. Last audited
2026-10-10 against the code, not the docs: of the 232 `tools/test_*_parity.py`
differential harnesses, 223 pass, 8 fail and the item model harness runs out of
memory; the failures are new harnesses that still find divergences.

| Subsystem | Score | Main gaps |
| --- | ---: | --- |
| Window, input and game loop | 0.75 | Game loop and options match vanilla; camera, auto-jump, toggle sprint/sneak and the frame limiter not compared |
| Assets and resources | 0.75 | Item model pipeline not yet used for all GUI icons; item model harness does not finish; no pack screen |
| Network protocol and multiplayer | 0.75 | Opens worlds to LAN, but hosting is not tested against a vanilla client; no dedicated server mode |
| Chunk storage and palettes | 1 | None: client and saves share paletted containers, identical to vanilla over 266 scenarios |
| Section meshing | 1 | None: 112 scenes and over a million quads identical to vanilla |
| Rendering | 0.75 | Entity model harness fails; block entity renderers and HUD overlays not compared |
| Lighting | 1 | None: saved and relit light matches vanilla, slabs and stairs included |
| World generation | 0.75 | Whole-chunk harness still finds single-block differences near camps, shipwrecks and villages |
| Region files and saves | 0.75 | Save round-trip harness fails; older DataVersions are not upgraded |
| Entities and physics | 0.75 | Attributes and dimensions match; AI, loot and spawn rules not compared; drowned, villagers, traders and the wither unverified |
| Interaction and inventory | 0.75 | Menu click harness fails |
| Menus, HUD and chat | 0.75 | Command parse harness fails; no /item modify, /spectate, /waypoint or /dialog; no tab list or world border rendering |
| Audio | 0.75 | Sound engine, music and ambience not compared with vanilla |
| Block behaviours and ticking | 0.75 | Block update, random tick and redstone harnesses fail; a few dispenser behaviours missing |

### Roadmap

1. Fix the divergences the failing harnesses report: block updates, random ticks, redstone, command parsing, menu clicks, save round-trip, whole chunks and entity models.
2. Let the item model harness finish, and draw every GUI icon through the item model pipeline.
3. An audio harness: sound engine attenuation, weighted sound events, music and ambience.
4. Mob AI, loot and spawn-rule harnesses, and an oracle for drowned, villagers, traders and the wither.
5. The missing commands, the tab list and world border rendering.
6. Test LAN hosting with a vanilla client, and upgrade saves from older DataVersions.

The rewrite is complete when it reaches one-to-one parity and a packaged native Linux build passes the launch checks in [Rust rewrite status](docs/rust-parity.md#completion-criteria-and-playable-builds).

## Contributing

Issues and pull requests are welcome. Before opening a pull request, run the offline checks:

```sh
git config core.hooksPath .githooks
python3 -m unittest discover -s tests -v
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
python3 tools/check_public_tree.py --history
```

CI runs these checks on GitHub's Linux runners. The vanilla comparisons need the game, which Horizon never downloads, so they run only when started by hand on a self-hosted runner holding the maintainer's own copy. Never commit game files, decompiled source or prepared resources: `tools/check_public_tree.py` rejects them, and tracked Java is allowed only under the project's own `dev/horizonlab` namespace.

## Licence

[MIT](LICENSE), for the project's original work only. Dependencies keep their own licences. Minecraft is a trademark of Mojang Synergies AB; Minecraft code and assets belong to Mojang and Microsoft and are covered by the [Minecraft EULA](https://www.minecraft.net/en-us/eula). Decompiled game source and prepared resources stay on your machine and are never published.

## Author

Made by [Ludovic Marie](https://lu4ovic.com) ([@ludovic111](https://github.com/ludovic111)).
