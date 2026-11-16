# Cache and disk

The native build key includes the pinned game and horizon-mixins commits, Rust version, target,
compilation settings and mod files. A matching installed build is reused. Editing a mod creates
a new build; the launcher compiles mods and their patch library, not the game.

The launcher keeps these managed caches under its data folder:

- `native/prebuilt/`: the SDK slices of each build and target (`<build id>-<target>`, holding
  `runtime/`, `api/` and `engine/` as fetched), evicted as a unit with whatever an interrupted
  download left in it.
- `native/sdk/`: each build's small SDK files (`sdk.json`, `surface.json`, `build-data/`) and the
  build id of each release seen; kept, so a mod's fit is told offline. What the published releases keep is in
  [Retention](../../reference/sdk/07-retention.md).
- `native/source/`: pinned source used to copy functions for mixins.
- `native/tools/`: pinned horizon-mixins executables.
- `native/prebuilt-build/`: mod, registry dependency, patch-library and linking outputs.
- `native/internals-sync/`: separate metadata-only discovery source and check outputs.
- `rust/` and `cargo/`: the pinned compiler and Cargo downloads.

`native/build.log` records build diagnostics. `native-mods.json` in an installed build records its
inputs and embedded mod inventory. Client builds share their base version's prepared resources,
so retain that base while its native builds are installed. Dedicated server exports are standalone.

Run `horizon-launcher storage` to inspect disk use and cleanup eligibility. Storage keeps
in-use versions and shared caches protected through runtime leases and publication locks. A busy
or unverifiable unit is retained with a reason; it is not counted as freed. Cargo lock files also
prevent unsafe cleanup. Do not delete managed cache directories while a build or game is running.
