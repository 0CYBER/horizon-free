# Design: client extensions

Provisional answer for [arismoko/horizon#895](https://github.com/arismoko/horizon/issues/895).
This proposes ownership and registration, not an implemented API or a completed
NeoForge parity audit. Implementation requires review first.

## Reference and category discrepancy

The named question is the complete extension surface for items, blocks, fluids
and entities, and its mapping to a typed client-extension registrar. The issue's
actual ask and batch 20 triage instead name **effects** as the fourth registration
category. Do not silently equate an entity renderer with a status-effect renderer.
The proposed four counterpart builders are item, block, fluid and effect;
entity capabilities resolve to existing entity rendering owners, outside that
four-category registrar until public documentation establishes otherwise.

The public [26.3 registration Javadoc](https://aldak.netlify.app/javadoc/26.3.x/net/neoforged/neoforge/client/extensions/common/registerclientextensionsevent)
returned HTTP 503 during this investigation. Triage reports four registration
categories and registration queries, but does not establish the complete method
surface. NeoForge 26.3 method names, defaults, overloads, duplicate policy and
limits therefore remain **unverified**. Require accessible public documentation
and a method-by-method disposition before adopting the final counterpart contract.
No NeoForge source, patches or other project's source are inputs. The following
is a complete proposed capability disposition, not a claimed complete census of
NeoForge methods. Unsupported methods must never become silent no-ops.

Vanilla has rendering consumers rather than Horizon's registrar. The vanilla
26.3 SectionCompiler.compile oracle query is recorded below; terrain compilation
is the relevant block/fluid convergence boundary. Other vanilla consumer details
remain unverified here; the linked merged designs retain their own evidence.

## Surface and ownership mapping

Reuse [Part instance state](part-instance-state.md),
[baked models and ModelData](baked-models.md), and
[section meshing](section-meshing.md). Their lifecycle, geometry, budgets and
invalidation contracts remain authoritative; this registrar adds no second store,
mesher, generation counter or draw bus.

| Category and proposed complete capability families | Owner and disposition |
| --- | --- |
| Item arm/use poses and first-person hand transforms | Existing get_arm_pose and item declarations; register into those owners, not a second callback at their moment. Humanoid extraction remains arismoko/horizon#1024. |
| Item GUI/held/ground/frame/equipment geometry, special rendering and model selection | Baked-model preparation and existing DrawContext submission; refuse capabilities requiring unapproved renderer interfaces. |
| Item equipment textures/models, fonts and presentation overrides | Typed resource references resolved at reload; refuse UnsupportedClientExtensionCapability until each consumer and public counterpart are audited. |
| Block hit/break particles and particle sprites | Existing block particle/Part paths; preserve vanilla fallback and existing particle ownership. |
| Block model data, terrain geometry, tint and special block-entity drawing | Baked models, section meshing and R2 respectively; reference their registrations rather than wrapping them with a duplicate event. |
| Fluid still/flow/overlay textures, tint, light and world appearance | Existing fluid appearance plus terrain fluid consumer; resolve resources before frozen worker input. Fluid world behaviour remains arismoko/horizon#793. |
| Fluid camera overlay, fog and other view effects | Existing view-effect extraction/draw owners; refuse until their typed consumer contract is approved. |
| Effect inventory/HUD visibility, icon and custom drawing | Separate effect-keyed presentation capability; ordinary HUD and inventory consumers must both be audited before support. Never key by EntityKey. |
| Entity renderer factory, layers, extraction, model/texture and visibility | Existing entity rendering, arismoko/horizon#896/#897 and general entity design; no entity builder masquerading as the effect counterpart. Unsupported general renderer requests explicitly refuse. |

## Current route census

Verified file:line anchors at this worktree's base ff35cdee95 describe current
owners, not proof that proposed dispatch already exists.

| Route | Current owner | Required integration |
| --- | --- | --- |
| Item declaration and humanoid pose | `crates/horizon-mod-vocab/src/content/content_declarations.rs:228`; `crates/horizon-mod-vocab/src/hooks/get_arm_pose.rs:23` | Reuse one resolved pose path for player and humanoid consumers. |
| Block particle sprite and destroy particles | `crates/horizon-client/src/game/blocks/block_particles.rs:47`, `:83` | Resolve block extension through the existing particle owner; hit route still needs caller audit. |
| Fluid box drawing and appearance lookup | `crates/horizon-client/src/game/modding/native_fluid_render.rs:15`, `:40` | Reuse appearance; this box route alone does not establish terrain/camera coverage. |
| Mob/player geometry and layers | `crates/horizon-client/src/game/modding/native_mob_draw.rs:57`, `:74`, `:97`, `:119` | Preserve existing entity submission; non-mob families require arismoko/horizon#896/#897 audit. |
| Terrain workers, reload, cancellation and upload | Section meshing's verified route table | Same frozen plan, leases and stale-result checks; no live registrar callbacks on workers. |
| Item baking and GUI/hand/ground/frame/equipment submission | Baked-model design's route table | Complete its explicitly outstanding caller audit before claiming coverage. |
| Effect inventory/HUD; fluid camera and fog | Concrete complete owners not verified | Registration refuses these capabilities pending owner/caller audit; do not claim coverage. |
| Native/wasm setup and teardown | Proposed generated game adapter, no implementation yet | Same schemas, dependency order, refusals and lifecycle on both targets. |

Every route either uses the listed owner or is explicitly unsupported. Initial
load, resource reload, world switch, unload, mod teardown and failed reload must
be acceptance cases. Dedicated servers do not dispatch client registration.

## Typed registration contract

Propose one register_client_extensions setup moment with ClientHost and a scoped
ClientExtensionRegistrar. Typed item/block/fluid/effect keys select small builders;
queries inspect staged registrations without invoking providers. Resolve keys
once after declarations and before rendering. Dependency order is shared by native
and wasm. A second provider for the same key/capability refuses with both owners
named; distinct capabilities can compose. Extensions to another mod's declaration
require an explicit dependency, not accidental load order.

Proposed API-owned Wire types: ClientExtensionRegistrationWire,
ClientExtensionCapabilityWire, ClientExtensionTargetWire,
ClientExtensionResourceRefWire and ClientExtensionRefusalWire. These carry explicit
tags, bounded lengths and typed keys; no Rust trait object, guest pointer, engine
model or GPU object crosses the boundary. Callback references resolve through
the generated runtime adapter. Refusals include UnknownExtensionTarget,
DuplicateClientExtension, UnsupportedClientExtensionCapability,
MissingClientExtensionWire and StaleClientExtensionResource. Unknown-target
refusal applies only to explicit registrar requests, never existing vanilla no-ops.

Stage an immutable indexed table; validate all requests before atomic publication.
Reload prepares a replacement and keeps the old valid table on failure. Resource
keys and callbacks lease existing resource/mod lifecycle holders; no second epoch.
Stop new dispatch, drain leases, then unload code. Object state follows existing
holders: stack components for items, existing scoped entity/block-entity handles
where available, and no invented plain-block or fluid instance holder.

Callbacks use their existing owner Host and decide/fire/apply transaction. Recheck
captured versions at apply and refuse stale decisions. In lifted passes, defer
retirement and journal emission until root commit; nested abort exposes no partial
registration or state. Saves refuse while an action savepoint is open, as specified
by Part state. DrawContext is supplied only by the owning draw moment.

## Untrusted input, bounds and cost

Validate native and wasm identically: enum tags, key ownership, resource existence,
finite numeric values, length arithmetic and declared schemas before allocation
or publication. Propose 4096 new registrations per mod and 64 KiB encoded/decoded
registration records; these bound extension additions, never vanilla registry
capacity. Geometry and state payloads retain their owners' bounds. Reject negative,
NaN or infinite values where finite nonnegative values are required; capability
specific ranges require public-contract review, not arbitrary clamps.

Catch native unwinds and wasm traps at the adapter, discard the whole tentative
result, diagnose once and disable the faulty capability for the current lifecycle;
use the unchanged built-in result. Native abort cannot be recovered. Wasm receives
finite fuel; native callbacks cannot be forcibly preempted safely.

Resolve dense slots at setup and select the ordinary path when no capability is
registered: O(1) dispatch selection, zero extension allocation/callbacks and no
all-mod scans. Workers consume the existing frozen representations. Performance
is proposed, not measured.

## Acceptance matrix

Future implementation tests, not results of this note:

| Case | Concrete expected value |
| --- | --- |
| No registrations, built-in cube | 6 faces; 0 extension callbacks and allocations |
| Item pose override for A, unregistered B | A uses selected pose; B retains its baseline pose |
| Duplicate item A pose providers | 1 DuplicateClientExtension; 0 partial publications |
| Unknown explicit fluid target | 1 UnknownExtensionTarget; existing empty-fluid draw still emits 0 quads |
| Entity renderer requested through effect builder | Typed API rejects it; malformed Wire gives 1 refusal |
| Unsupported equipment font/HUD capability | 1 UnsupportedClientExtensionCapability before first draw |
| Record of 65537 bytes or registration 4097 | Whole offending registration batch refused; vanilla registry unchanged |
| NaN, infinity, negative required-nonnegative value, native panic or wasm trap | 0 partial results; original built-in output retained |
| Reload B fails; delayed A job completes after successful C | B publishes 0 tables; A publishes 0 stale uploads |
| Root abort after tentative holder edit | 0 journal emissions and retirements; prior value restored |
| Save during open action savepoint | 1 save refusal; 0 tentative bytes saved |
| Identical native/wasm registration inputs | Same accepted slots, refusal tags and rendered outputs |
| Approved effect icon in inventory and HUD | Exactly 1 owner callback per visible icon in each route |

## Proposed stages

1. Public 26.3 method census and category approval (file none): enumerate every
   interface method/default/overload and assign one owner or named refusal.
2. Consumer audit (file none): complete item, particle, terrain/camera fluid,
   effect inventory/HUD and non-mob entity routes with verified caller anchors.
3. Typed registrar and generated adapters (file none): approve Wire schemas,
   duplicate policy and lifecycle integration together for native and wasm.
4. Owner integration (file none): implement only approved capabilities through
   existing Parts/rendering; run the matrix and measure unused-path cost.

## Open questions

- Fourth category? Recommend effects for the documented counterpart and explicit
  separate entity ownership; obtain maintainer agreement on the task wording.
- Full 26.3 method surface and semantics? Recommend public documentation as a
  hard implementation gate; current family matrix cannot establish full parity.
- Unsupported families and incomplete caller coverage? Recommend named refusal
  until each owning design and route audit is approved, never silent acceptance.
- Registration ceilings? Recommend the proposed additive limits, measured against
  representative mods without reducing built-in capacity.
