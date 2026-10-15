# Version ports: from a new Minecraft release to Horizon

Goal: every Minecraft Java release is turned into Rust by Horizon with as little human work as possible. A new
release is detected, downloaded and prepared on the Mac mini, compared with the version Horizon pins, everything
data-driven is regenerated, every parity harness runs against it, and what is left is a ranked list of agent-sized
work items, each a GitHub issue and a Kira card. This page is the design; `tools/version_port.py` implements it
(status at the end).

The downloads are for development only, on the Mac mini: nothing from them is ever committed, published or shipped
to players. Players still prepare their own copy from their official launcher (`horizon-prepare`), as today.

## How Horizon pins and prepares 26.3 today

- **The pin.** `research-lock.json` `minecraft`: the version id, the sha1 of Mojang's version JSON (and of the
  republished ones it accepts), the client and server jar sha1s, the Java major version. Vineflower, REA and JADX
  are pinned beside it.
- **The vanilla data.** `tools/lab.py prepare` imports the client jar, the server jar and the client libraries from
  the user's own launcher install, unpacks the server bundler into `server-game.jar`, and writes the class
  inventories; `tools/lab.py decompile` runs Vineflower on both jars. On the Mac mini all of this lives once in
  `~/code/horizon-parity/shared-local/` (`minecraft/26.3/{jars,libraries,sources,reports}`, 366 MB), and every
  worktree links to it from `.local/`. Release builds never do this: `horizon-prepare` imports the user's copy.
- **The data generator.** `horizon-prepare` runs `java -DbundlerMainClass=net.minecraft.data.Main -jar server.jar
  --reports` into `.local/datagen/out/reports/`: `blocks.json`, `registries.json`, `packets.json`,
  `commands.json`, `datapack.json`, item default components, biome parameters. It also runs Horizon's Java
  exporters (`mods/horizon/src/test/java/dev/horizonlab/parity/Vanilla*Exporter.java`) against the jar for block
  behaviours, worldgen bundles and the like. CI does the same through `tools/ci/vanilla-data.sh`, keyed by the
  version in `research-lock.json`.
- **Version facts baked into the code.** The client jar's own `version.json` states them (26.3: world version 5023,
  protocol 777, resource pack 97.1, data pack 121.0, Java 25); Horizon copies them by hand today:

  | Where | What |
  | --- | --- |
  | `crates/horizon-core/src/lib.rs` | `VANILLA_VERSION` |
  | `crates/horizon-protocol/src/lib.rs` | `PROTOCOL_VERSION`, `GAME_VERSION` |
  | `crates/horizon-protocol/src/packets/configuration.rs` | the `minecraft:core` known pack version |
  | `crates/horizon-world/src/lib.rs`, `crates/horizon-client/src/game/commands/extra.rs` | `DATA_VERSION` (twice) |
  | `crates/horizon-client/src/save.rs`, `crates/horizon-world/src/datafix/files.rs`, `game/commands/extra.rs` | the version name (three times) |
  | `crates/horizon-client/src/crash.rs`, `bin/horizon-ctl/launch.rs`, `launcher/src/backend.rs` | `"26.3"` fallbacks |
  | `tools/perf_bench.py`, `tools/vanilla_client.py` | the vanilla client's `--version` |

- **Generated tables** (each has its generator in `tools/`):

  | Output | Generator | Input |
  | --- | --- | --- |
  | `crates/horizon-protocol/src/ids.rs` | `generate_packet_ids.py` | `packets.json` report |
  | `crates/horizon-client/src/lan/tracking.rs` | `generate_entity_tracking.py` | decompiled `EntityTypes` |
  | `crates/horizon-client/src/lan/command_suggestions.rs` | `generate_command_suggestions.py` | `VanillaCommandSuggestionsExporter` |
  | `crates/horizon-client/src/lan/command_order.rs` | `generate_lan_command_order.py` | vanilla's command tree |
  | `crates/horizon-client/src/command/vanilla_order.rs` | `generate_command_order.py` | command-parse harness export |
  | `crates/horizon-world/src/datafix/block_state_data.txt` | `gen_block_state_data.py` | decompiled `BlockStateData` |
  | `crates/horizon-world/src/datafix/flat_tables.rs` | `gen_flat_tables.py` | 1.13 flattening fixers (never change) |

  Everything else data-driven (block behaviours, items, worldgen, models, sounds, languages) is not committed: the
  game reads what `horizon-prepare` exports from the user's own jar, so it follows a new version by itself as long
  as the exporters still compile and the formats still parse.

- **The proof.** ~300 harnesses (`tools/test_*_parity.py`) each compare a Rust probe with a Java oracle that calls
  vanilla's own classes; `tools/parity.py` runs them and records each one's dependencies.

## The pipeline

```
detect ─► prepare NEW ─► diff OLD..NEW ─► regenerate ─► harnesses on NEW ─► port report ─► issues + cards
 (daily)   (download,     (data, protocol,  (generators   (candidate          (ranked work    (agents port,
           verify,         registries,       per kind)     worktree)           items)          pin bump PR,
           datagen,        assets, classes                                                     release)
           class index)    → Horizon map)
```

`tools/version_port.py` has one subcommand per stage and `port NEW` runs them all. Each stage writes its result
under the version store and can be rerun alone.

### The version store

`~/code/horizon-parity/versions/<id>/` (`HORIZON_VERSIONS` overrides it), next to `shared-local/` and never inside
it:

```
versions/26.4/
  version.json            Mojang's version JSON (sha1 checked against the manifest)
  jars/client.jar         sha1 checked against version.json
  jars/server.jar         the bundler; server-game.jar beside it, unpacked with sha256 checks
  datagen/reports/...     the data generator's --reports output
  classes.json            the class index: every class's methods, each with a normalized bytecode hash
  facts.json              world/protocol/pack/Java versions from the jar's own version.json
  prepared.json           what was made, with which tool versions
versions/diff-26.3-26.4/  the diff, the source diffs of changed classes, the port report
```

About 150 MB a version (jars 120 MB, reports 20 MB, index 10 MB): client libraries are not needed (the data
generator runs from the bundler, which carries its own) and the full Vineflower tree is not kept (only changed
classes are decompiled, below). The store keeps at most three versions, the pinned one (whose files already exist in
`shared-local/`, so it is only indexed, not copied), the previous one and the candidate; `version_port.py gc`
deletes the rest and every scratch directory. Snapshots, if prepared, are deleted when the next one is.

### Detect

`version_port.py detect` reads `https://piston-meta.mojang.com/mc/game/version_manifest_v2.json`, compares the
latest release (and, with `--snapshots`, the latest snapshot) with the pin and with
`versions/detect-state.json`, and prints what is new; exit status 0 means "nothing new", 10 "new release",
11 "new snapshot". It needs only the network, so it can run anywhere:

- **Mac mini, launchd (chosen):** a `~/fleet` launchd agent runs `version_port.py detect --then port` daily at
  14:00 UTC (Mojang publishes on weekdays around 10:00-14:00 UTC), in a dedicated worktree of `origin/main`,
  at `~/fleet/load prio low`. The whole port runs where the vanilla data, Java and the harnesses are.
- **GitHub Actions:** a scheduled job could run `detect` alone and open a "Minecraft X is out" issue; the Mac mini
  would still have to do the rest, so it adds a hop for nothing. Kept as the fallback if the Mac mini is down for
  long.

Snapshots are optional and only ever diffed (`port --snapshot`: prepare, diff, report, no issues), as an early
warning: agents can read what 26.4 changes weeks before it ships.

### Prepare

`version_port.py prepare NEW`:

1. Fetch the manifest, the version JSON (check its sha1 against the manifest), the client and server jars (check
   their sha1s against the version JSON); `lab.download` already refuses partial or mismatched files. Releases
   since 26.1 ship unobfuscated, so no mappings are fetched; an obfuscated version (`client_mappings` present)
   stops here, as `lab.py prepare` does.
2. Unpack the server bundler (`lab.unpack_server`: one version, sha256 per entry).
3. Read the jar's `version.json` into `facts.json`. If its Java major is newer than the JDK on the Mac mini, stop
   with a single work item, "install JDK N" (a Ludovic task: fleet toolchains).
4. Run the data generator (`--reports`) with that JDK into `datagen/reports/`.
5. Index the classes of `client.jar` and `server-game.jar` into `classes.json`: for each class its superclass,
   interfaces, fields, and methods; each method's hash is over its bytecode with every constant-pool reference
   replaced by what it names (class, member, descriptor, literal), so recompiling unchanged code with a shifted
   constant pool hashes the same, and lambda bodies (`lambda$name$N`) are folded into the method that declares them
   so renumbered lambdas don't show as changes. This is exact where a decompiled-source diff is noisy, and takes
   seconds instead of the 10 minutes of a full Vineflower run.

The pinned version is indexed from `shared-local/` the same way, without copying its jars.

### Diff

`version_port.py diff OLD NEW` writes `diff.json` (and readable sections of the report), one section per kind:

| Kind | From | What it lists |
| --- | --- | --- |
| Facts | `facts.json` | protocol, world (data) version, pack formats, Java |
| Packets | `packets.json` | per state and direction: added, removed, renumbered packets |
| Registries | `registries.json` | per registry: added and removed entries, protocol-id shifts, new registries |
| Blocks | `blocks.json` | added and removed blocks; per block: properties and values, state count, default state |
| Item components | `components/item` | per item: added, removed and changed default components |
| Commands | `commands.json` | added and removed nodes, changed arguments and executability |
| Data packs | jar `data/minecraft/**` | added, removed, changed files by kind (recipe, loot table, worldgen, tags...), with a JSON path diff for changed files |
| Assets | jar `assets/minecraft/**` | `en_us.json` keys added, removed, changed; models, blockstates, textures, sounds added, removed, changed (by hash) |
| Classes | `classes.json` | added, removed and changed classes; per changed class the methods added, removed and changed (name and descriptor), field changes, hierarchy changes |

Changed classes are then decompiled, only those, on both sides: Vineflower runs on a small jar holding just those
classes (with the full jar as a library, so types resolve), and a unified diff per class goes into
`diff-OLD-NEW/sources/`. These diffs stay on the Mac mini: decompiled game code is never committed or posted
(`tools/check_public_tree.py`).

### From vanilla classes to Horizon modules

The diff matters only through what it changes in Horizon. `version_port.py map` builds `vanilla_map.json`, from
the repository alone, in seconds:

1. **Oracles → classes.** Each Java oracle and exporter under `mods/horizon/src/test/java/dev/horizonlab/parity/`
   names the vanilla classes it imports and uses (`net.minecraft.*` imports and fully qualified names).
2. **Harnesses → oracles and Rust.** Each `tools/test_*_parity.py` names its oracle class; its probe (a crate's
   example or binary named in the script) and its `PARITY_RUST_SCOPE` give the Rust files it proves.
3. **Rust → classes.** Horizon's doc comments name the vanilla code they port (`/// SharedConstants.getProtocolVersion()`,
   `//! Fixes of 1.21.6 to 26.3`, `EntityType.Builder clientTrackingRange`...): every CamelCase identifier in a
   comment that is the simple name of exactly one vanilla class (or `Outer.Inner`, or `Class.method`) maps that
   Rust file, and the method when named, to the class.

A changed vanilla class then resolves to the Horizon files that port it and the harnesses that prove them; a
changed method resolves more precisely when a comment names it. A class with Horizon code but no harness, or with
neither, is reported as such.

### Regenerate

Each generated output has a generator entry in `version_port.py` (`regen`): its command, its input in the version
store, the files it writes. On a candidate worktree (`port/NEW` from `origin/main`, with `.local/` linking to the
new version), `regen` runs them all and records, per generator: unchanged, regenerated (with the `--stat`), or
failed (the generator's own error: a source pattern no longer matches, a report changed shape). Two additions make
this complete:

- `version_consts`: a generator for the version facts table above, from `facts.json`. Before it, a small refactor
  makes every Rust site use the `horizon-core` / `horizon-protocol` constants instead of its own copy, so the pin
  bump touches one generated file.
- `horizon-prepare` against the new version: its Java exporters compile and run against the new jar, or fail to
  compile, which names exactly the vanilla API they used that changed.

Generators that read decompiled sources (`EntityTypes`, `BlockStateData`) get those two files from the changed-class
decompilation, so no full Vineflower run is needed for them either.

### Run every harness against the new version

In the candidate worktree, with `research-lock.json` pinned to NEW and `.local/` linking to the new store
(`.local/minecraft/NEW` holds `jars/`, `datagen` the new reports, `horizon-client` what the candidate's
`horizon-prepare` made), `tools/parity.py --all` runs every harness. Each result is one of:

- **pass**: Horizon already matches NEW there;
- **oracle does not compile**: vanilla's API changed under the oracle (javac names the symbol);
- **fails**: behaviour changed (or Horizon's data reading broke); the log has the first differing case.

The known failures of `main` (`tools/ci/parity_issues.py`'s audit) are subtracted, so only what NEW broke counts.

### The port report

`diff-OLD-NEW/report.md` and `report.json`:

1. Summary: facts changed, counts per kind, generators (clean / regenerated / failed), harnesses (pass / oracle
   broken / fail).
2. Work items, ranked. An item is one Horizon area, not one vanilla class: changed classes are grouped by the
   Horizon module (or, for unported code, by vanilla package) that owns them, so each item is a PR-sized task. Rank:
   1. blocks the switch: protocol or save format (packets, registries the protocol syncs, data version and the new
      DataFixer schemas), a failed generator, an exporter that no longer compiles;
   2. a harness that fails or whose oracle no longer compiles;
   3. a changed class Horizon ports but no harness covers (the risky kind: nothing will catch a difference);
   4. new content in data packs and assets the game reads at runtime (usually nothing to do: listed so it gets a
      look);
   5. changed vanilla code Horizon doesn't port yet (informational, no issue).

   Within a rank, by the number of changed methods. Each item lists the vanilla classes and methods, the Horizon
   files and harnesses, the data-diff excerpts that concern it, and the local path of its source diffs.
3. Everything regenerated cleanly, so the reader knows what not to look at.

`version_port.py publish` opens one GitHub issue per item of ranks 1 to 3 (labels `version-port` and
`port-NEW`, title `[NEW] <area>: <summary>`; an open issue with that title is updated instead), with the structured
diff attached (names, signatures, data-report diffs, harness output; no decompiled code), and a Kira card per issue
(`card_add` with the issue URL, column `todo`) so contributors and agents can claim them. Rank 4 and 5 go into one
tracking issue.

### The switch and the release

Work items are fixed on `port/NEW` branches stacked on the candidate (or as normal PRs into it). The pin bump PR is
the candidate branch itself: `research-lock.json` (version, sha1s, Java), the regenerated tables and
`version_consts`, the oracle fixes, and the ports. It merges when the report says no rank 1 or 2 item is left: every
harness passes against NEW, CI's vanilla data (`tools/ci/vanilla-data.sh`) and the Mac mini's `shared-local/` have
been switched to NEW (the version store's files are copied in, then `lab.py decompile` runs once for the full
sources), and a playtest shows a world loads and a vanilla NEW client can join a Horizon LAN world.

A Horizon release then ships as usual (`docs/release-flow.md`), labelled `feature` with "Minecraft NEW" in its
title. Its `research-lock.json` makes `horizon-prepare` ask for NEW, so players must have played NEW once in the
official launcher; the launcher already reads each release's game version from its `research-lock.json`, so players
who haven't can keep the previous Horizon release. Supporting two Minecraft versions in one build is out of scope.

## What stays manual, and how small it gets

Automatic: detecting, downloading, verifying, the data generator, every diff, the class-to-module map, regenerating
data-driven tables, rerunning every harness, the report, issues and cards, and, once the report is clean, the pin
bump branch.

Manual, by agents (and Ludovic's review):

- **New mechanics** (a new mob, block behaviour, worldgen feature, UI screen): someone has to port the new code. The
  pipeline hands them the exact classes and methods, the decompiled diff, the Horizon module and the harness to
  extend, so the agent starts at the code instead of searching for it.
- **Changed behaviour under a failing harness**: the harness names the case, the class diff names the method.
- **Oracles whose vanilla API moved**: javac names the symbol; usually a short fix.
- **New harnesses** for rank-3 items, where Horizon ports code that no harness checks.
- **New JDK**: a fleet toolchain install, once per Java bump.
- **Judgment on the switch**: merging the pin bump and releasing stay Ludovic's call.

The replay of 26.2 → 26.3 below measures how much that is for one release.

## Implementation status

```sh
python3 tools/version_port.py detect --snapshots     # exit 10: new release, 11: new snapshot
python3 tools/version_port.py prepare 26.3           # the pinned version: indexes .local, downloads nothing
python3 tools/version_port.py prepare 26.4           # downloads, verifies, runs the data generator (about 10 s)
python3 tools/version_port.py diff 26.3 26.4         # summary; the full diff in versions/diff-26.3-26.4/diff.json
python3 tools/version_port.py gc                     # keep the pinned version and the newest two others
```

| Slice | Status |
| --- | --- |
| This design | this page |
| `detect`, `prepare`, `diff` (facts, packets, registries, blocks, items, commands, data, assets), `gc` | done |
| Class index, class diff, `vanilla_map.json` | next PR |
| Port report (`report`), `port` | next PR |
| Changed-class decompilation, `regen`, `version_consts` refactor, candidate harness run, `publish`, launchd agent | later |
