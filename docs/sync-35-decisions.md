# Sync 35 decisions

## Pins and scope

- Initial fork integration base: `11fadb75ddb963c01b7bb86b9931d613f2bb48f9`.
- Preparation main refresh: `fa5ed5ab45` (rollup 63), merged once after the initial full workspace/all-target compile passed. Its sole conflict retained main's direct DamageHooks shape; unused deferral guards were subsequently removed during static review.
- Opening main refresh, explicitly cleared after rollup 65: `2d3964c330b1e51b69eb7944ddc100f6a129d618`, merged once. Its sole conflict preserves the canonical split item-construction/admission helpers and restores main's constructor signature. Existing prepared state, including hook-edited age, passes through unchanged; only fresh no-source death-drop state initializes its age before construction.
- Post-rollup-68 refresh: `714a656cd967a676ef252133d3e7f23387171805`, merged once after explicit clearance. Take both halves of main's use-hand extraction, retaining its destination and porting the sync sound helper there. Preserve main's immediate teleport effects while carrying typed admission reasons; combine test declarations and documentation. Deduplicate TNT state fields introduced by clean automatic merges, retaining the authoritative runtime value.
- Upstream: `9f42064c11efe7ef3e5ed2baad7ddc2a3b36eea0`.
- Upstream's own main-push checks passed in run `43353661596`.
- The final own-green refresh beyond the initial `36eb98b6` pin changes only `tools/test_pet_owner_teleport_parity.py`: it distinguishes wolves that followed from those left behind using a 16-block radius, allowing subsequent random stroll. That upstream refresh itself changed no Rust/build inputs.
- Previous upstream ancestor in this base: `f24ee1accf1c816736e6e067939d04f3853f474e`.
- This directly integrates the pinned upstream into fork main, not through a stacked sync branch.

The merge began with 80 conflicts. Resolution areas were mobs/entities (23), gameplay and build wiring (24), rendering/network/saves (16), engine/tools/launcher (14), and the three lead-owned documentation/sampler/stack-dump paths. Adjacent automatic merges were audited alongside each area.

## Conflict policy

Keep fork typed ownership, attachment and join lifetimes, generated module discovery, canonical hook moments, native/wasm contracts and the launcher's repository/update route. Take upstream vanilla fixes and retain the restored Horizon layer, including telemetry, cosmetics, shop and Horizon extras. Do not revive deleted monoliths or add compatibility aliases when upstream extractions overlap existing fork modules.

### Mobs and entities

- Preserve canonical `borrowed_tick`, typed damage/loot continuations and lifetime-aware entity construction.
- Keep authoritative Equipment storage, selected-player gaze routing and full-stack piglin finish-admiring ownership. Age-bearing custom death drops bypass stack-only table modifiers and join the later typed LivingDrops collection as complete records, following the existing clean-room death-loot ordering contract; mutable table edits cannot transfer their age to another item.
- Port crystal/frame persistence, TNT runtime portal state, shulker cart behavior, fish voice/flop/sting behavior, goat flags, happy-ghast cloud healing and aged wither-star loot into their canonical owners.
- Initialize both fork lifetime/join state and incoming fields in `Crystal::new` and local TNT constructors.
- Keep one unconditional Vex gravity reset in `finish_tick`; preserve the unchanged endermite lifetime helper move and AI/NoAI callers.
- Move incoming fixture adaptations into the existing canonical test modules, deleting obsolete paths rather than registering duplicates. Pufferfish sting fixtures live beside their canonical owner rather than in the quarantined parallel-mobs folder.

### Gameplay and generated wiring

- Preserve generated declarations and native API generation without duplicating executable resource linking. Omit unconsumed upstream grouped-turn discovery and deferral support; the retained conservative Group/Islands implementation does not call it.
- Keep `use_hand`, transactional menu handling and prepared-item RNG in their existing canonical implementations. Port substantive fixes from incoming extractions rather than restoring duplicate modules.
- Keep typed item-on/without-item jukebox interactions while taking component-aware song selection, loaded-clock behavior, events and statistics.
- Preserve mod dimensions and registered sea levels when taking map and world-loading fixes.
- Preserve live entity/vehicle transfer tokens. Consume `PendingEntity` in the extracted persist loader, setting TNT runtime portal context before admission/join hooks; never serialize the runtime flag into NBT.
- Extras glue remains under `game/interface/extras`, retaining counters/minimap hooks and the existing reconnect and PvP owners.
- Preserve sampler calibration, span-owner arbitration, report restoration and CPU attribution. Omit unconsumed mod-only/helper-thread sampler support: its upstream diagnostic consumers and grouped helper path are not imported. Retain the canonical single report accumulator rather than unused parallel APIs.

### Rendering, protocol and saves

- Take upstream renderer submission/frustum extraction, retaining fork frustum test support and mod particle groups.
- Adapt sprite renames and first-JSON metadata parsing to open `ResourceSource` traversal rather than restoring filesystem-only loading.
- Preserve namespace-aware GUI loading and fork Mods navigation alongside shell-hosted pack settings.
- Preserve attachment initialization and typed falling-block updates while taking duplicate-id handling and projectile spawn improvements.
- Retain tool/attribute component delivery, query/cookie/sequence handling, effect metadata caching and existing packet layouts.
- Preserve biome-region/mod save modules, block-state-aware decode validation and attachments. Apply upstream block-entity position/order correction and jukebox load behavior.

### Engine, tools and launcher

- Keep split native hooks, Wire/crossing/dispatch/capability modules and the fork dependency policy; do not resurrect upstream trait-loading or injection generation paths.
- Retain fork stack-aware enchantment `select`, centralizing modifier identifier normalization in `item_data::modifier_rows` rather than reviving removed adapters.
- Keep server permission/backup behavior alongside upstream exact-label, input-validation and pruning fixes.
- Preserve launcher `REPO`, updates, pinned native toolchain, registry dependency freedom and locked game packages. Incoming junction-expression cleanup does not restore obsolete build paths.
- Keep parity tooling/corpora improvements and explicit manual-proof exclusions. No vanilla assets, prepared resources or decompiled source are added.

### Lead-owned paths

- `AGENTS.md`: retain the fork package name and document the actual extras child paths.
- `server_perf/samples.rs`: take upstream's Linux-only explanatory comment; retain the existing Linux cfg and all calibration/ratio/progress behavior.
- `horizon-threads/stack_dump.rs`: one Release readiness store inside the busy frame, Acquire readiness wait using `yield_now`. Production capture is unchanged.
- The independently merged sampler span-count fix remains unchanged.

## Maintainer-ratified query limit

Carry upstream's `MAX_CHALLENGES = 65_536` pending query-handshake address limit, including its existing-address renewal and pruning behavior. Vanilla does not impose this cap; retaining this upstream defensive limit is an explicit maintainer ruling for the sync.

## Deliberate mod-contract differences

Upstream's `explosion_entities` hook is not imported: the fork's canonical `explosion_detonate` covers that moment; entity filtering there is a follow-up (maintainer ruling). No sibling event, alias, adapter, fragment, call site or test is introduced. Unrelated explosion fixes remain. Extending canonical detonation with affected-entity read/removal requires a clean-room ordering/contract specification first.

Unconsumed grouped-turn deferral machinery is not imported: no retained caller enters or replays such turns. Default typed callbacks dispatch directly without additional deferral TLS checks, and the unused staged-turn scan/environment emission is omitted. The default borrowed loop and conservative optional Group/Islands behavior remain unchanged; no claim of dynamic-mod parallel safety is made.

The existing `upstream-parallel-mobs` Group/Islands quarantine remains at its pre-sync implementation rather than importing context-free replay into an incompatible optional path. Its existing rider/typed-level signature mismatches are static findings, not observed compiler failures. Incoming grouped notification buffering, quiet-trait-mod parallel enabling, planned-start/deferred collection and raid grouping rewrite are not imported into that quarantined path. This sync does not revive it or replace the live borrowed loop. Default workspace/all-target compilation does not claim all-features coverage.

## Function-replacement inputs and verification

Source implementation moves include renderer in-flight/frustum code, crystal construction, canonical extras children and shared mob item-stack configuration. Incoming duplicate gameplay extraction paths are rejected where the fork already owns canonical implementations. Native source patches must follow those actual owners; generated discovery remains authoritative. Client build wiring and mod-api-gen staged detection change. No point/injection mechanism or native game-rebuild fallback is introduced.

Verification: both the initial integrated source and the authorized main refresh passed `CARGO_INCREMENTAL=0 cargo check --workspace --all-targets --keep-going -j 2`. Those checks reported unused incoming helpers. Static audit confirmed the active container-viewer implementation already exists, and the additional mod/helper sampler support has no imported consumer; duplicate/unconsumed helpers were removed without warning allowances. The full workspace/all-target compile-only check after that cleanup passed without source warnings (pass 7). Static-review carry corrections also passed the full compile-only command without source warnings (pass 9). After the opening main refresh and constructor-signature restoration, `CARGO_INCREMENTAL=0 cargo check --workspace --all-targets -j 2` passed without source warnings (pass 11). A fresh narrow Opus static review approved the three-file age/state correction. The post-rollup-68 refresh also passed the full compile-only command without warnings (pass 14), with independent static approval of the use-hand move and typed immediate-teleport resolution. Regression coverage is compiled but not run. No local tests, clippy, parity harnesses or game runs are permitted for this sync. Workers performed rustfmt parsing and static area audits only; CI remains the runtime proof.
