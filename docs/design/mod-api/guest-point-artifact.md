# Design: Source-validated guest point artifact

Provisional proposal for [arismoko/horizon#2553](https://github.com/arismoko/horizon/issues/2553).
Docs only; this is not approval to skip source validation or a speedup claim.
The proposed answer is conditional: an SDK-produced, identity-bound artifact can
replace repeated discovery only when it carries the complete validated point,
private-type and site input. Keep the full-source generator as an independent
check. A supplied `--catalog` alone never authorizes this route.

Measure actual-mod cold builds after arismoko/horizon#2596's interface cache first. Do not
build or ship an artifact on an estimate; proceed only after the measured
post-cache baseline demonstrates worthwhile remaining work.

## What remains after the interface cache

[arismoko/horizon#2552](https://github.com/arismoko/horizon/pull/2552) and
[arismoko/horizon#2596](https://github.com/arismoko/horizon/pull/2596) are merged context.
The latter already caches source interfaces locally and preserves unchanged
output timestamps. Repeating the original uncached premise would overstate the
remaining problem. Verified owners on the current source tree:

| Route | Owner and observed behavior |
| --- | --- |
| Guest point build | `crates/horizon-mod-points/build.rs:1`: always calls `generate_points`; errors currently terminate the build through `expect`. |
| Cached generator | `crates/horizon-mod-api-gen/src/build.rs:223`: enters the interface cache; `:227` tracks game, point and site sources, reads private input before selection, discovers points and sites, then parses/formats generated Rust and writes only changed output (`:258`). |
| Cache admission | `crates/horizon-mod-api-gen/src/interface_cache.rs:17`: directory keyed by generator executable bytes; `:36` keys entries by source path and bytes. It verifies the summary digest and parses it; invalid summaries regenerate. Function bodies are stripped on cache creation. This is disposable local acceleration, not authenticated SDK evidence. |
| Discovery and types | `crates/horizon-mod-api-gen/src/points.rs:922`: resolves Wire types, excludes private targets from public discovery and scans source. |
| Selection and identity | `crates/horizon-mod-api-gen/src/points.rs:1166`: retains all public record shapes and IDs, limits guest codecs/exports by selection, and computes `POINT_LAYOUT` across the full public point set (`:1205`). Selection is not permission to renumber IDs or drop layout inputs. |
| Site discovery | `crates/horizon-mod-api-gen/src/sites/stage.rs:7` and `:50`: tracked inputs and generated site IDs; these also belong in artifact validation. |
| Wasm registration/package | `crates/cargo-horizon-mod/src/wasm.rs:248` and `wasm/group.rs:297`: catalog feeds registration preparation. `wasm/group.rs:637` passes selected names to compilation; `:569` binds the emitted points metadata digest into the package. None replaces source-derived Rust definitions. |

A target-cold build with an empty `OUT_DIR` must still create these cache entries.
A cache hit still reads/hashes inputs, parses smaller summaries, constructs point
and private metadata, discovers sites and emits/parses/formats Rust. Source
traversal, generator compilation and downstream guest compilation remain possible
costs. Their current proportions have not been measured here. Cache reuse across
build-script executables or fresh targets must be measured, not assumed.

Vanilla 26.3 is the gameplay specification; this Rust SDK generation facility is
Horizon-specific and changes no vanilla call site. No vanilla declaration is
claimed as a point-artifact contract. The [NeoForge public documentation](https://docs.neoforged.net/)
separately documents its modding API and Gradle toolchain; no equivalent 26.3
point/type/site artifact contract was verified. The public
26.3 javadoc could not be fetched (HTTP 503). NeoForge limits and equivalence remain unverified; public documentation
is required before adopting any such contract. No NeoForge source or patches
are inputs to this proposal.

## Proposed contract and alternatives

Prefer a generated input artifact consumed by the existing validator and emitter,
not arbitrary precompiled Rust supplied by a mod. The SDK producer runs full
source validation once against its exact staged source. Preserve canonical point
order, signatures, host side, head/return behavior, API and internals capability
refusals, private lexical types/fields/exports, site definitions and IDs, and the
complete inputs to layout computation. Use the existing API-owned representations
and one explicit `Wire` derivation; do not add a second serializer or substitute
internals access when an API capability is unsupported. Carry existing refusal
reasons through artifact generation and refuse unsupported selected capabilities
at registration with a named diagnostic.

A safely narrowed scanner is the alternative. It must resolve the complete
transitive module/type closure, imports, target-dependent paths, private lexical
scope, global point order and sites before pruning work. Selecting a single
function name is insufficient: unselected records still contribute public shape
and layout. Recommend retaining full scanning until a closure proof and equivalence
fixtures establish safe narrowing. Both candidates use the same emission rules;
the independent check bypasses artifact loading and the disposable interface cache.

The authoritative identity is a canonical manifest produced alongside the SDK,
anchored in the chosen engine distribution or locally validated source tree,
not an engine ID asserted inside mod input. The trusted manifest comes from
the Nightly build and is signed with the same key as that build; verify that
signature before trusting artifact digests. Bind:

- Exact engine build and staged-source identity, including private staging plans,
  source bytes and normalized repository-relative paths; directory membership,
  additions and deletions are inputs too.
- Generator/parser and artifact schema revision, API/Wire declarations and
  signatures/layout inputs, point/internals catalog bytes and site definitions.
- Canonical selected names (including head/return distinction), target/cfg/features,
  toolchain and relevant build/profile settings. A reusable complete SDK payload
  may serve several selections, but each derived build admission binds its exact set.

Use content digests rather than mtimes. Check artifact availability, bounded
header and expected identity before decoding its payload. Verify payload length
and digest against trusted manifest data, then run structural and semantic
validation; a self-reported checksum does not authenticate a modified payload.
A local source edit invalidates the SDK identity even if the release name is
unchanged. Regeneration establishes the new identity.
For an SDK-only build without matching source, refuse with the mismatched field
and rebuild/reinstall guidance. Missing, stale, unknown-schema or corrupt artifacts
may regenerate only from the exact matching available source through the current
full generator. This is recomputation of current inputs, not an old-mod
compatibility path; never silently accept stale definitions or omit a selected row.

Mod inputs, selection strings and artifact bytes are untrusted. Before allocation,
require bounded file size, row counts, string lengths, nesting and cumulative
payload bytes; reject duplicates, unknown selected names, invalid paths and
out-of-range IDs. Do not execute artifact Rust or callbacks during admission.
Proposed diagnostics are `ArtifactIdentityMismatch`, `ArtifactCorrupt`,
`ArtifactLimitExceeded` and `SelectedPointUnsupported` (names subject to review).
Malformed input returns a build error; internal generator panics abort that build,
never produce a partial artifact. Publish only complete output atomically.

Native and wasm keep one dependency and api/internals contract. Native retains
exact engine-build loading; wasm retains interface linking. Artifact build
identity is stricter provenance for generation, not a new wasm runtime exact-build
restriction. No new Host, callback, lifecycle holder or generation counter is
introduced. No world mutation, journal, retirement or save boundary is involved.
Admission happens during builds; unused feature cost is zero runtime work,
including zero tick scans, allocations and no-listener dispatch changes.

## Packaging and module identity

Coordinate packaging with open [arismoko/horizon#2815](https://github.com/arismoko/horizon/pull/2815).
Its reviewed diff adds versioned `mod-catalog.json` to SDK/source staging, with
`schema_version`, `api_version`, hooks and public point descriptions. It is a
discovery surface, not proof of complete private layouts or site definitions.
Do not overload it into an executable generation contract or assume it contains
private source plans. Recommend a sibling generation artifact referenced by a
trusted SDK manifest, owned by the same staging pipeline. Avoid a committed
shared hash that every point change must edit; regenerate and compare in drift
checks, with feature-scoped fixtures.

Open [arismoko/horizon#2697](https://github.com/arismoko/horizon/pull/2697) overlaps generator
`build.rs`, `points.rs` and `points/source.rs`; open
[arismoko/horizon#2524](https://github.com/arismoko/horizon/pull/2524) overlaps `build.rs`.
Reconcile their final ownership before code work. The interface-cache file and
point build script are not touched by those reviewed diffs. This note reserves
no implementation files.

Byte-identical module identity is the default admission policy. Refuse a
mismatch with a diagnostic naming its reason and the differing identity field;
never silently accept it on an estimate of equivalence.
Identical effective inputs must yield byte-identical generated definitions,
point/site IDs, layout values and module identity between routes and across
repeated runs. Preserve current path remapping and compiler metadata handling
(`crates/cargo-horizon-mod/src/wasm/group.rs:625`). Compare generated Rust,
embedded identities, package points digest and module bytes separately; a matching
`POINT_LAYOUT` alone is insufficient. Adding an artifact dependency or changing
build-script inputs can itself change compiler/build identities. Any unavoidable
difference needs an explicit explanation, loader/linker compatibility evidence
and maintainer approval before adoption; never normalize away the difference in
the test or repeat the rejected profile-override experiment as a solution.

## Acceptance and measurements

These are proposed literal fixtures, not executed results.

| Case | Required result |
| --- | --- |
| Full fixture: 2 point records, 1 private type, 3 site IDs | Both routes discover exactly 2/1/3 entries and identical validated signatures/layouts. |
| Select one point of two | Exactly 1 selected guest point export; invalid selected signature refuses; 0 silently omitted selected records. Retain the full route's unselected public shapes and stable IDs/layout inputs. |
| Change private field `u32` to `u64`; engine `A` to `B`; selection `["p"]` to `["q"]` | Each old admission is refused or regenerated before linking; 0 stale admissions. |
| Artifact truncated to 0 bytes or 1 payload byte modified | 0 successful stale loads; explicit diagnostic or full-source regeneration. |
| Identical inputs twice, then compare full and artifact routes | Byte-identical definitions and module identity; any intended difference is named and approved first. |
| Missing artifact/source; unknown schema; oversized payload; duplicate or unknown selected name | Refusal before linking, no partial output or capability downgrade. |
| Native and wasm, API and internals | Same applicable validation/refusal behavior; no new runtime restriction or dependency model. |

Measure an actual mod build with representative selected points and private type/site
use, after arismoko/horizon#2596. Run **3 cold actual-mod builds per route**: current cached full
generator, artifact candidate, and narrowed scanner only if implemented. Hold
engine, source, selected names, toolchain, target/features, profile, machine and
parallelism constant. Define cold as a fresh Cargo target and generator-interface
cache for each run; report filesystem/OS cache state and reuse of downloaded
sources separately. Report all 3 total wall times and all 3 generation times,
not just averages. Also report artifact production time/size, admission/hash/decode,
source discovery, emission/formatting and guest compilation time so moving work
to SDK production is visible. A separate warm-cache series characterizes arismoko/horizon#2596;
it must not be mixed into the cold series. Check equivalence before interpreting
timings. There is no speed threshold until evidence supports one, and historical
fixture timings establish no actual-mod speedup.

## Proposed stages and open questions

Stages: first collect the post-cache baseline and identity
inventory; then agree manifest/authentication and limits with SDK staging owners;
then implement artifact production/admission using existing validation/emission;
finally run equivalence, corruption and actual-mod measurements before deciding
whether to enable the route. Put future logic/tests in separate modules; no
growth of the over-limit `crates/horizon-client/src/game.rs` for instrumentation.

- **Trust anchor and packaging:** recommend a sibling artifact authenticated by
  the engine SDK manifest; agree its manifest owner and local-source verification
  procedure with arismoko/horizon#2815 before implementation. A payload digest alone is insufficient.
- **Concrete resource limits:** recommend finite byte/count/depth budgets derived
  from the complete current SDK plus measured headroom; choose literal limits
  before shipping. No vanilla capacity is reduced by this build-only proposal.
- **Identity differences:** recommend exact preservation by default. If artifact
  integration changes compiler identity despite equivalent output, obtain a
  maintainer decision with the measured native/wasm linking consequences.
- **Does this remove enough work?** Recommend measuring first and retaining full
  generation if admission plus remaining compilation erases the benefit. Safe
  source narrowing remains conditional on closure correctness, not a promised win.
