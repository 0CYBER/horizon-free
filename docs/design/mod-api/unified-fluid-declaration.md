# Design: unified fluid declaration

Provisional proposal for [arismoko/horizon#2436](https://github.com/arismoko/horizon/issues/2436).
Maintainer approval is required before implementation. No Rust changes land with
this note.

## Answer and reference behaviour

Use one `Fluid` declaration owned by `horizon-mod-package::content`, with an
explicit `TankOnly` or `World(WorldFluid)` placement mode and shared appearance.
`Content.fluids` becomes `Vec<Fluid>`. Files and native/wasm registration decode
into this same declaration and use one validation and publication pass. A client
model registration decorates an existing identity; it never declares a fluid.
Do not revive `horizon-mods`.

Reviewed upstream references:
[horizon-rust/horizon commit 73456ae0085f662006ce3ffe5866920be0d34e7a](https://github.com/horizon-rust/horizon/commit/73456ae0085f662006ce3ffe5866920be0d34e7a)
and [horizon-rust/horizon commit 4584e2d7b5b3494f1cb024269cf510bca5f49863](https://github.com/horizon-rust/horizon/commit/4584e2d7b5b3494f1cb024269cf510bca5f49863).
The first declares template-based fluids with optional physics, block and bucket
references, registers world fluids after vanilla, copies the liquid template,
and makes level states resolve to the mod's own fluid. Its client draws the
template; its tag masks widen to u128. The second moves the mod lookup behind
vanilla water/lava and the level-property test, but still uses a lock and scan.
Adopt template choice, validated physics and preservation of the mod's liquid
block through spreading. Reject the second declaration owner, fixed-width mod
membership masks, and hot-path linear lookup. Keep the fork's custom models and
data-carrying kinds. Diverges from upstream: generated block and optional bucket
are owned by the fluid declaration rather than independent block references.

Vanilla 26.3 was inspected through `tools/rea_vanilla.py method`: `WaterFluid.getTickDelay` returns 5;
`LavaFluid.getTickDelay` returns 10 for fast lava and 30 otherwise.
`FlowingFluid.spread` is the flow reference, not a declaration API. Preserve
vanilla timing, conversion, mixing and dimension-sensitive behaviour for vanilla
kinds; choosing a lava template must not classify a mod fluid as vanilla lava.

NeoForge's public fluid documentation and 26.3 `BaseFlowingFluid` Javadoc were
attempted at <https://docs.neoforged.net/docs/resources/server/fluids/> and
<https://aldak.netlify.app/javadoc/26.3.x/net/neoforged/neoforge/fluids/BaseFlowingFluid.html>.
Both were inaccessible. The upstream declaration describes separate FluidType
properties and flowing-fluid properties, but this is not verified evidence of
NeoForge 26.3 behaviour. Exact viscosity effects, mixing, legal limits and
callback contracts remain **unverified**; require public documentation before
adopting a NeoForge contract. No NeoForge source or patches were read.

## Proposed contract

`Fluid` contains a namespaced id, sprite/tint appearance and placement mode.
World mode selects typed `Water` or `Lava` template, tick delay, drop-off,
slope distance, source conversion, light level, viscosity and optional generated
bucket. Omitted timing uses the selected template, including fast-lava context;
explicit timing is fixed. Source conversion defaults false for mod fluids.
Viscosity is a positive metadata property, independent of tick delay and spread;
refuse an entity-motion viscosity capability until its implementation and public
contract are verified. Never advertise viscosity as working world drag merely
because its number decodes. Tank mode has no block, flowing registry entry,
bucket or scheduled ticks; world-only fields in tank mode are errors.

Proposed validation bounds: tick delay 1..=i32::MAX; drop-off 1..=7; slope
search 1..=4 initially; light 0..=15; viscosity 1..=i32::MAX. These are Horizon
policy, not claimed NeoForge limits. The slope bound preserves vanilla's maximum
search capacity; reject upstream's extra 5..=8 pending a bounded search design.
Water template defaults to tick delay 5, drop-off 1, slope 4 and light 0;
lava light defaults 15. Resolve remaining lava rules from the existing vanilla
context rather than flattening them into water defaults.

Registration returns named `FluidDeclarationError` reasons: `InvalidId`,
`DuplicateOwner`, `NameCollision`, `InvalidRange`, `IncoherentPlacement`,
`UnsupportedCapability` and `Capacity`. Include owner, id and field in diagnostics.
Validate all generated source/flowing/block/bucket names before publishing any.
Keep the fork's flowing-name convention (`a:flowing_dir/oil` for `a:dir/oil`),
reject collisions, and never silently rename it to upstream's last-segment rule.

The API-owned declaration, placement, template and appearance types each carry
an explicit `#[derive(horizon_mod_api_macros::Wire)]`, using the shared Wire
mechanism rather than catalog-generated per-type serialization forms. This
follows `crates/horizon-mod-vocab/src/world/game_rule_value.rs:7` and the derive
entry point at `crates/horizon-mod-api-macros/src/lib.rs:6`. The generator builds
host call tables only; engine structs stay out of the public API and ABI.
Before applying these derives in `horizon-mod-package`, move the shared Wire
contract and its supporting codec/layout primitives into a lower, API-owned
crate that depends on neither package nor API. Both crates depend directly on
that crate; `mod_api::wire` re-exports the same contract. Update the existing
derive to resolve that lower crate directly for package-owned declarations,
while retaining the same explicit derive mechanism for native and wasm. Do not
add a package-to-API dependency: the API already depends on the package
(`crates/horizon-mod-api/Cargo.toml:15`), and the current derive emits
`::mod_api::wire` (`crates/horizon-mod-api-macros/src/wire.rs:49`). This dependency
restructure is a prerequisite in stage 2, not a second serialization mechanism.
Explicit enum tags and bounded decoding remain requirements. Both targets
submit the same fields and receive the same refusal. Bound strings and list
sizes using existing package limits before allocation; reject malformed tags,
unknown fields, overflowing integers, NaN and infinity rather than clamp content.
No callback is needed for static declaration. Existing server fluid-rule hooks
retain a `ServerHost`; client model hooks retain `ClientHost`, and their panic
follows the existing dispatcher containment/abort policy, never unwinds through
wasm or publishes half a declaration.

## Identity, lifecycle and routes

Retain the single `FluidKind` set (`Empty`, water/lava source and flowing,
`Mod(ModFluid)`, `ModFlowing(ModFluid)`) and `tick_id()` mapping. Tank keys use the
existing fluid registry without fabricating world kinds. Saves use names, not
process indices. Use the existing content lifecycle holder for reload identity;
add no second generation or independent lifetime counter. Prepare immutable
resolved tables before publication; refuse removal while live references prevent
safe retirement. Retire old tables and emit journals only at root commit,
including lifted passes. Refuse saves during an open action savepoint.

Verified current owners (line numbers describe this checkout):

| Route | Owner and required change |
| --- | --- |
| File declarations, generated blocks/buckets | `crates/horizon-mod-package/src/content.rs:509`, `:521`, `:565`, `:877`: replace FluidFile/FluidLook split with one validated Fluid |
| Code declaration ingress | `crates/horizon-mod-vocab/src/parts/fluids.rs:589`, `:634`; `crates/horizon-client/src/game/modding/api_values/content_declarations.rs:448`: replace world-only registrar and conversion with unified input |
| Registration and merge | `crates/horizon-client/src/mod_content.rs:95`, `:131`, `:157`: remove independent merging/override semantics; one owner, one publication pass |
| Runtime identity and ticks | `crates/horizon-world/src/fluid.rs:150`, `:210`; `crates/horizon-world/src/level/tick.rs:1815`, `:1864`: retain kinds and tick matching |
| Flow, bucket placement and pickup | `crates/horizon-world/src/level/fluids.rs:184`, `:312`, `:365`: consume resolved template/rules and preserve own block |
| Client appearance | `crates/horizon-client-render/src/fluid_models.rs:32`, `:53`: index shared declaration appearance, retain model overrides |
| Save/cache/worldgen ticks | `crates/horizon-client/src/save.rs:627`; `crates/horizon-world/src/level/data/cache.rs:629`; `crates/horizon-world/src/worldgen/generator_decoration.rs:874`: retain name/tick conversions |

Single player, LAN and dedicated use the server simulation; remote clients only
resolve synchronized content and render, never run authoritative flow callbacks.
Use dense state-to-fluid and kind-to-rule tables resolved at load. No-listener
flow stays O(1), with no mod scan, string creation, allocation or lock; vanilla
water/lava retain their fast branch. Tags/predicates use resolved membership
sets, not a second FluidKind or a u128 ceiling that truncates mod identities.

[PR arismoko/horizon#2453](https://github.com/arismoko/horizon/pull/2453) owns transaction tokens,
close validation and fluid tank write authorization in `native_fluids.rs`,
`parts/fluids.rs` and the transaction runtime/Wire forms. This proposal consumes
that authority check; it does not redesign markers, mint authority or duplicate
tank mutation paths. Declaration changes must preserve its guarded insert/extract
and rollback semantics, rebasing against whichever version lands first.

## Acceptance matrix

| Scenario | Concrete expected result |
| --- | --- |
| Tank `m:oil` | One tank key; zero blocks, buckets, flowing entries or ticks |
| World `m:brine`, water defaults | Own liquid block and optional `m:brine_bucket`; delay 5, drop-off 1, slope 4, light 0 |
| World lava, normal context | Proposed template defaults: delay 30, drop-off 2, slope 2, light 15 |
| World lava, fast context | Proposed template defaults: delay 10, drop-off 1, slope 4, light 15 |
| Source conversion | Default false: two adjacent sources do not create a source; explicit true with the required support creates this mod identity, never vanilla water |
| Replacement | Preserve current mod-fluid refusal to be replaced; empty accepts flow; vanilla water accepts a different fluid only downward |
| Mixing | A lava-template mod keeps its own identity and does not acquire vanilla lava mixing solely from its template; vanilla water/lava mixing remains unchanged |
| Tag membership | A declared member matches its tag for source and flowing states; a nonmember does not; membership never truncates at 128 identities |
| Generated bucket | Placement creates the declared source block; pickup returns the declared bucket and removes that source; tank mode generates neither route |
| Client appearance | Shared sprite/tint supplies the default model; an explicit model override decorates the same identity without registering another fluid |
| World overrides | Delay 12, drop-off 2, slope 3, light 7 remain exactly 12/2/3/7 after native and wasm round trips |
| Invalid bounds | Delay 0, drop-off 8, slope 5, light 16, viscosity 0 each refuse with InvalidRange and zero publication |
| Unsupported viscosity motion | UnsupportedCapability; no silently ignored drag request |
| Source/flowing state | Level 0 resolves Mod; level 1 resolves ModFlowing for same identity; spread leaves own block |
| Vanilla timing | Water 5; lava normal 30, fast 10; no listeners produces unchanged results |
| Duplicate generated names | NameCollision; neither partial block nor bucket published |
| Tank-only plus world in one mod | Two Fluid entries, one validation pass, no second registration loop |
| Save/reload | Name round trip preserves identity despite changed process ordering; open savepoint refuses save |
| First upgrade or corrupt schema | Existing world with missing, unreadable or corrupt saved schema refuses before chunk loading or any write; original files remain byte-for-byte unchanged |
| New world schema | Persist the validated initial schema before allowing chunk publication or saves |
| Transaction abort | Tank insert followed by abort restores original amount; forged authority follows arismoko/horizon#2453 refusal |

Tag membership also requires replacing the current vanilla-only mask behavior
(`crates/horizon-world/src/fluid.rs:258`); custom membership is proposed here.
These are implementation acceptance requirements, not claims that the unified
API exists today. Current reference routes are
`crates/horizon-world/src/level/fluids.rs:77` (timing), `:189` (drop-off),
`:199` (slope), `:209` (conversion), `:101` (replacement), `:312` (container liquid
placement), `:365` (source pickup), `:750` (mixing), and
`crates/horizon-client-render/src/fluid_models.rs:32` (appearance collection).

Port upstream's world-physics fixtures through the fork API: own-block spreading,
source/flowing resolution, replacement and mixing, configurable timing/drop-off,
light and template rendering. Hand-write expected states and amounts; compare
native/wasm failures as well as successes. Audit upstream fixture expectations
against vanilla before treating its NeoForge claims as an oracle.

## Proposed stages (file none)

1. Approve declaration shape, refusal policy and publicly verified capabilities.
2. Move the shared Wire contract below package and API as specified above, then
   replace package and API declarations together; apply the shared explicit Wire
   derive to API-owned types for both native and wasm. Generate host call tables
   only, with no catalog-generated per-type Wire forms.
   Update every caller, sample, fixture, catalog note, docs/mods.md and affected
   modding book pages in that same implementation change.
3. Resolve world/template/light/model tables and port the upstream physics tests.
   Preserve transaction ownership from arismoko/horizon#2453; add native/wasm acceptance fixtures.

Migration is a clean break: delete FluidLook and the separate world declaration
entry point, rewrite existing content and samples to Fluid placement modes, and
rebuild mods. No alias, shim, fallback parser, deprecated name or version gate.

Saved-world policy (proposed): mode and generated source/flowing/block/bucket
names form the persistent content schema. A mode change or generated-name change
requires a new world; refuse opening an existing world with that schema mismatch
before loading chunks or writing saves, and report the owner and changed names.
An existing world with a missing, unreadable or corrupt saved schema also
refuses before chunk loading or any write, including on its first upgrade to
this design. Never infer a baseline from the currently installed declarations.
Only a newly created world records the validated initial schema before chunk
publication or saves. Rebuilding mods does not migrate saved data. Implement an explicit saved-schema
check in the implementation stage; this is not an existing loader guarantee.
Current scheduled-fluid ticks serialize names and resolve them on load
(`crates/horizon-client/src/save.rs:625`, `:632`, `:1982`), so renaming cannot be
treated as a process-index change. In particular, `read_ticks` uses
`filter_map` and drops unresolved names (`crates/horizon-client/src/save.rs:605`,
`:606`); accepting a new baseline could therefore silently lose old ticks. Add acceptance fixtures for both mode changes
and name changes: refusal leaves the original save untouched. Add no alias,
fallback parser or automatic remapping.

## Open questions

- What does viscosity promise beyond metadata? Recommend refusing motion support
  until the NeoForge 26.3 public contract and engine route are verified.
- Should slope search extend to eight? Recommend retaining four for this change;
  approve wider search only with an explicit capacity and cost review.
