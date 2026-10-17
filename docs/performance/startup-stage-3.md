# Progressive world resources (#712)

Stage 3a changes shared resource storage and accessors. Pack resources keep their existing per-game ownership for pack reloads. Stage 3b publishes core before the simulation tail, builds the real renderer using the previous device after releasing its surface, and gates backdrop streaming on that renderer. Stage 3c queues an early Play or Connect action and draws the existing level-loading progress screen while polling the worker each event-loop turn. Close and resize events continue to work. After the worker completes, the queued local/server action resumes; integrated servers receive the resource handle only after that join. The synchronous harness and dedicated path returns only fully loaded resources.

Startup logs distinguish `core-ready`, `world renderer ready`, `starting title backdrop`, and `rest-ready`. Opt-in performance summaries carry `to_core_ms` and `to_rest_ms` separately from the existing UI-driven `to_title_ms`. Loader failures and panics propagate to the client; a tail taking more than 120 seconds fails rather than waiting indefinitely.

## Stage 3d: interactive menu before core

An ordinary launch now owns a `StartupMenu` inside `Loading`. The first menu atlas and panorama message makes the existing title screens interactive; assets, facts, biomes, pack tables, collision and motion continue on the resource worker. Pack-overlay and mod-content preparation also run on that worker. The live landscape still starts only after the core renderer exists.

World browsing, world creation, server address entry and options use the existing title/options widgets. Play and Connect queue a `PendingWorldEntry`; the progress screen covers both the core and tail waits. Escape cancels the queued entry in either phase. Close and resize continue to work. The handoff transfers the title object, options screen, settings, cursor, clipboard selection, timing session and pending action instead of rebuilding the user's screen.

Pack changes during core loading request another loader generation. The old core and tail finish before replacement begins so cache writes and tail registry initialization cannot overlap. Each generation has its own message channel and progress counter. Language/GUI metadata publication stays on the event-loop thread, using the current language selection rather than the worker's stale settings snapshot. Pack changes after the core handoff also wait for the tail before touching mob metadata.

The menu still needs its own artwork and renderer before interaction. The core handoff still builds the full renderer and starts audio on the main thread; menu click sounds and controller navigation start with `Game`. This stage does not change synchronous harness/dedicated startup or introduce a render-facts export.

Regression coverage publishes menu artwork while a fake core worker is blocked, edits a world draft, replaces fallback artwork without losing it, queues entry and draws progress, and checks that loader failure reaches the app. Additional checks cover server identity and cancelled-entry timing.

## Render facts assessment

`MeshTables::new` reads names, registry properties, class membership, collision shape flags, fluid and render properties from `BlockFacts`. `LightTables::new/load` reads emission, dampening, occlusion and shape-occlusion flags; geometry comes from `occlusion.nbt`. Neither table needs recipes, item data, AI or the other tail tables.

A render subset is possible, but the current facts decoder also validates registry order and builds the gameplay projection from the same document. Meshing additionally depends on class chains from `block-behaviours.json`. Avoiding the 54 MB read requires an exported render-facts artifact (and its prepare/cache/version checks), or another decoder plus a second full read at world entry. This is beyond a cheap startup ownership change. Stage 4 can add that artifact and compare the resulting mesh/light tables against the full loader before moving facts out of core.

## Measurement

Measurements on this machine are warm runs with `HORIZON_NO_AUDIO=1`. Other compilation and parity jobs were running, so elapsed times are diagnostic rather than a controlled benchmark.

Before phasing (3a, `42d65465`), `--offscreen --frames 3 --smoke-report` completed successfully with peak RSS 762,536 KiB. The report contained 3 frames, no chunks, no generated or meshed sections, no player, and no error. This is the synchronous harness path; the progressive path is measured separately after 3c.
