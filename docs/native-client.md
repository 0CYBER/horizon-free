# Horizon native development client

`horizon-client` opens a native Rust window and renders a controlled block scene
stored in Horizon's resident chunks. Every occupied cell selects models from its
registry state, including the terrain and sixteen featured placements. It loads
block definitions, cuboid models, sprites and occlusion grids from pinned Java
Edition 26.3 resources. Model selection uses the default block position seed;
neighbor visibility uses the validated Rust face rules across chunk boundaries.
The default camera is an inspection tool. F4 enables a standing-sized collision
volume that queries the rendered resident chunks through the shared world
collision implementation. The camera can still fly; it does not simulate a player.
F5 switches to a dry walking test with gravity, ground support, stepping and
held-jump timing over the same resident chunks. Its movement runs at 20 ticks/s;
the camera interpolates between tick positions. Landing damage now updates stored
health, absorption and hurt cooldown state. Hunger and regeneration tick after
movement. Left Shift crouches, with pose-dependent collision bounds and eye height.
F3 diagnostics include health, food, sprinting and pose.

Eligible opaque unit-cube faces use greedy rectangles only when texture layer,
UV rotation/mirroring and shading match. Each rectangle retains its signed UV
basis, so weighted texture variation survives merging. Other baked geometry and
binary-alpha cutouts use the separate model-face instance pipeline. Geometry
with nonintegral UV-lock rounding also stays on that path.

This checkpoint has partial player movement, landing damage and hunger. Vanilla world generation, block
interaction, inventory, sound, networking and world save/load remain unbound in
the native client. It does not meet
the [playable-build acceptance criteria](rust-parity.md#completion-criteria-and-playable-builds).
The controlled chunk scene now uses blockstate dispatch, weighted variants,
multipart conditions and transformed baking. Full block-catalog material support,
fractional transparency, tinting, lighting, resource packs, chunk streaming and
full visual parity remain unfinished.

The resource gallery sits beside the pyramid and includes oak stairs, a slab,
an anvil, a torch, a lantern, connected fences and iron bars, a cobweb, a wall
grindstone, a ladder, a sideways amethyst cluster and four weighted stone
placements. Registry state properties select the models: the stairs are an
upside-down inner corner, the slab is a top slab and the fence gate is open.
These are resident chunk states; simulation callbacks remain unfinished. Binary-alpha cutouts are rendered;
the lantern's three frames advance every eight inspection ticks at 20 ticks/s.
The animation metadata codec and frame selection now use the shared
`horizon-assets` implementation checked against the pinned client. GPU
interpolation and synchronization with the eventual game loop remain unfinished.
Sprites currently require matching square frame dimensions.

## Run from a checkout

Use current stable Rust (the locked client dependencies require Rust 1.90 or
newer), Python 3.10+ and JDK 25. A native desktop display and compatible graphics
driver are required. Resource preparation verifies the pinned game artifacts,
exports the registry using the reference server and property schemas using the
reference client, and extracts selected client
resources into ignored `.local/horizon-client/26.3/`.

```sh
python3 tools/prepare_native_client.py --prepare
cargo +stable run --locked -p horizon-client
```

Java is used during preparation. The native executable itself runs without a
JVM. Prepared resources stay private; do not commit or publish the extracted game
files. Final distributable builds still need resource provisioning and packaging.

Use `--resources PATH` for a prepared resource directory. The executable also
looks for a sibling `resources/` directory before the checkout default. Use
`--radius N` to select 1–16 chunks around the origin; this is a finite test scene,
not a render-distance or world-streaming implementation.

| Control | Action |
| --- | --- |
| WASD | Move horizontally |
| Space / either Shift | Rise / descend in inspection mode; Space jumps in walking mode |
| Either Ctrl | Boost inspection speed |
| Left Shift | Crouch in walking mode |
| Left Ctrl while moving forward, or double-tap W | Start sprinting in walking mode when eligible |
| Arrow keys | Rotate camera |
| Left click | Capture mouse for camera rotation |
| Escape | Release mouse |
| F3 | Toggle FPS/chunk/mesh/health/food/sprint/pose diagnostics in the window title |
| F4 | Toggle inspection camera collisions; enabling requires an unobstructed position |
| F5 | Toggle walking test at the current camera position; enabling requires an unobstructed position |
| Ctrl+Q or window close | Exit |

Controls bind physical keyboard positions. Losing focus clears held keys and
releases the mouse. Walking pauses while unfocused and resets partial tick time
on focus changes. Sprinting continues after releasing Left Ctrl while forward
input remains held; hunger and collisions can stop it. Walking currently has no flight, item use or block interaction. F5 returns to inspection mode; the finite scene has no
streaming or boundary wall. `--trace-input` logs keyboard, mouse-button and focus events
for diagnosis. On headless Linux use `vscreen run ./target/debug/horizon-client`,
then `vscreen shot`. The `wtype` tool's default generated keymap does not preserve
physical key positions; typing the character W can report physical Escape.

## Validation

The client loads a separate, registry-checked collision-shape export alongside
render resources. `SceneCollisions` feeds its actual `TerrainChunk` instances
into registered shape selection, block scanning, world movement resolution and
occupancy checks. The scene has no other entities, block entities or border;
entity-dependent queries report an error until an entity runtime is supplied.
Camera collision uses a position context and disables stepping. The walking
binding uses the current pose bounds, descending input and registered step-height
attribute. Sneaking invokes ledge backoff before collision resolution. The
complete authoritative player tick remains unfinished.

```sh
python3 tools/prepare_native_client.py
python3 tools/test_native_scene_collision.py
```

The [scene collision comparison](../benchmarks/rust-native-scene-collision-2026-10-07/scoped-parity.json)
checks **14,150 movement requests**, each with before/after occupancy queries,
against both pinned reference JARs. Native code independently builds the nine-
and twenty-five-chunk scenes and all sixteen gallery placements. Movement
components compare by exact double bits. Cases include partial blocks, pyramid
ledges, ceilings, initial overlaps, negative and missing chunk boundaries, tiny
and zero movement, and varied step heights. This verifies the scene adapter;
it does not establish player locomotion or complete gameplay parity.

The movement binding stores a player's position, velocity, pose bounds,
ground/support state, collision flags, movement history and live registered
attributes. `MotionWorld` composes the existing air-travel, jumping, collision,
ground-support, restitution and block-coefficient code over the same scene.
The [stored-motion comparison](../benchmarks/rust-native-scene-motion-2026-10-07/scoped-parity.json)
checks 42,257 operations across 637 evolving scenarios against actual native
`Player.travel`, jumping and movement, using real native attributes and chunks.
It compares every resulting position, velocity and bounding-box component by
exact bits, along with support, climbing and collision state and both separate
velocity/position synchronization flags.

The dry movement tick additionally binds small-velocity cleanup, float input
damping, held-jump delay and server frost-modifier removal. The expanded
[tick comparison](../benchmarks/rust-native-motion-tick-2026-10-07/scoped-parity.json)
checks 47,393 operations in 847 scenarios against both pinned JARs, including
direct calls to the actual `LivingEntity.aiStep` body. It compares lazy attribute
instance creation, stored base/cache/dirty state, modifier values and permanence,
and pending update/sync membership alongside the motion fields. Queue iteration
order depends on runtime identity hashes and is outside this scene comparison.
The test caught an omission where matching positions concealed a retained
powder-snow modifier and missing attribute notifications; dry ticks now remove
that modifier after travel, as vanilla does.

The [walking movement comparison](../benchmarks/rust-native-walking-2026-10-07/scene-motion.json)
extends this to 65,893 operations in 947 scenarios, invoking actual
`Player.aiStep` with empty inventory/equipment and no nearby entities. It checks
head yaw, jump-trigger countdown and speed refresh after movement. Prepared
input is supplied at the native virtual input boundary. Separately, the
[local input comparison](../benchmarks/rust-native-walking-2026-10-07/local-input.json)
checks 13,988 cases against actual `KeyboardInput.tick` and
`LocalPlayer.modifyInput`, covering every key combination, diagonal adjustment,
item/riding/sneaking speed gates, signed zero and nonfinite values. This verifies
input arithmetic; the sprint and crouch integration checks below extend it.
The full `LocalPlayer` lifecycle remains unfinished.

The fixed-rate driver limits work per rendered frame while retaining overdue
ticks. Clock tests cover varied frame schedules and long stalls. The development
focus pause is explicit; no vanilla pause-menu or multiplayer timing is claimed.
The [Linux walking smoke check](../benchmarks/rust-native-walking-2026-10-07/interactive-smoke.json)
enabled F5, landed at ground height 4, walked, jumped, switched modes and exited
with Ctrl+Q. Ground, moved and airborne screenshots were visually inspected;
the run recorded 207 movement ticks and finished grounded without an error.
The [renderer startup check](../benchmarks/rust-native-walking-2026-10-07/startup-smoke.json)
also passes after the movement integration.

```sh
python3 tools/prepare_native_client.py
python3 tools/test_native_scene_motion.py
python3 tools/test_local_input_parity.py
```

This binding is not yet a complete player tick or input controller. Its current
domain is the dry resident scene with standing/crouching/crawling dimensions,
zero frozen ticks, empty inventory/equipment, and no entity interpolation,
vehicles or status effects. Selected `Player.aiStep` phases are bound; the full tick and local input
lifecycle remain unfinished.
Landing now accumulates and resets the stored fall distance, clears climbing
state on ground, and invokes registered block fall/bounce reactions. The
[composed landing comparison](../benchmarks/rust-native-landing-2026-10-07/scene-motion.json)
checks 73,025 operations in 1,449 scenarios against both pinned JARs. It compares
fall state, impulse exemptions, attributes and ordered damage, statistic,
particle, sound and hit-ground requests alongside the previous motion fields.
Actual native `LivingEntity.checkFallDamage`, registered block methods and
`Player`/`LivingEntity.causeFallDamage` execute in the reference harness.
The cases include hay, honey, slime, powder snow, pointed dripstone, farmland,
turtle eggs and shelf mushrooms, with seeded world randomness. Farmland pushes
the stored player and changes the resident block to dirt; repeated egg changes
preserve hatch state. The resulting block state is checked after each operation.

The client refreshes its mesh after a resident block revision changes. This is
a complete scene rebuild for now, with no performance improvement claimed.
Damage requests are counted in runtime reports, and the health binding described
below applies them synchronously. Full death/respawn, audiovisual playback and
networking still need consumers. Block writes currently update
resident storage; neighbor updates, drops and block-entity lifecycle remain
unfinished. Block-inside effects such as cobweb slowdown and the full entity
lifecycle are also unfinished. The reference harness explicitly controls these
boundaries, which remain outside the comparison's scope.
The earlier [scene collision comparison](../benchmarks/rust-native-scene-motion-2026-10-07/collision-regression.json)
and [renderer startup](../benchmarks/rust-native-scene-motion-2026-10-07/startup-smoke.json)
also pass with this binding and the additional prepared movement resources.

The accompanying [startup check](../benchmarks/rust-native-scene-collision-2026-10-07/startup-smoke.json)
passes after sharing scene loading between the GUI and headless collision probe.
An [interactive Linux check](../benchmarks/rust-native-scene-collision-2026-10-07/interactive-smoke.json)
enables F4, holds Shift to descend, verifies that the camera stops at the terrain
surface, and exits with Ctrl+Q. The window was visually inspected on Vulkan
llvmpipe; this is not a hardware performance measurement.

The separate `horizon-assets` crate decodes the pinned 26.3 cuboid model format.
Its native comparison covers all **2,883 block-model JSON files** in the client
JAR plus **2,457 synthetic cases**: element bounds, optional geometry, nullable
children, face UVs and quarter turns, single-axis/Euler rotations, texture
references and translucency materials, light emission, display transforms and
inheritance markers. Floating-point fields compare by their exact bits. The
comparison also covers primitive coercion, integer truncation/wraparound and
selected strict-reader numeric/syntax limits. The numeric scanner also preserves
vanilla's rejection when its wrapping integer accumulator hits zero before
another digit, including in ignored fields. Twenty newly exposed acceptance
mismatches were corrected; the complete comparison now covers 5,340 models.
The [resolution](../benchmarks/rust-resource-number-scanner-2026-10-06/model-resolution.json),
[baking](../benchmarks/rust-resource-number-scanner-2026-10-06/model-baking.json) and
[animation](../benchmarks/rust-resource-number-scanner-2026-10-06/sprite-animation.json)
comparisons also pass with the shared scanner correction.

```sh
python3 tools/test_cuboid_model_parity.py
```

The [archived decoder result](../benchmarks/rust-resource-number-scanner-2026-10-06/model-decoding.json)
records source, fixture and reference hashes. The oracle uses actual client
classes; those rendering APIs are absent from the server. The decoder, parent
resolver and baker now feed the development gallery. Blockstate selection, transformed
models and UV locking have the separate checks described below. Rendered
appearance still needs its own checks.

`ModelLibrary` resolves parent geometry, ambient occlusion, GUI lighting and
each display context. It shares inherited elements rather than cloning them.
Texture aliases bind after child overrides are applied; successful and failed
alias walks are cached. Missing resources use the native missing-model cube;
parent cycles invalidate their descendants. The
[resolution comparison](../benchmarks/rust-model-resolution-2026-10-06/scoped-parity.json)
matches **5,648 requested roots across 439 graphs**, including the full bundled
block-model catalog, missing parents, 702 cyclic roots, 1,024-model parent chains,
512-slot alias chains, explicit empty geometry and identity/null transforms.
It calls native `ModelDiscovery`, `ResolvedModel` and `TextureSlots` APIs.

```sh
python3 tools/test_model_resolution_parity.py
cargo +stable test --locked -p horizon-assets
```

Cuboid baking now produces positions in block units, normalized unit-atlas UVs,
face winding and facing, material references, tint indices, shade overrides and
light-emission metadata. It supports single-axis and Euler element rotations
with the pinned rescaling rule. The
[baking comparison](../benchmarks/rust-model-baking-2026-10-06/scoped-parity.json)
matches **7,100 roots across 440 graphs**, using native `FaceBakery` with a real
sprite and the default JOML math options. All bundled block models bake without
errors. Synthetic coverage includes reversed/degenerate bounds, signed zero,
arbitrary rotations, rescaling, null geometry children and parent cycles.
Position, UV and matrix fields compare by exact float bits for these fixtures.

```sh
python3 tools/test_model_baking_parity.py
```

That comparison uses the identity blockstate transform. Atlas packing,
transparency classification, actual tint/light evaluation and GPU rendering
remain separate from the geometry result. Other operating systems still need
their native validation runs.

`BlockModelTransform` now prepares all 64 XYZ quarter-turn combinations once
and shares them across baked models. It rotates geometry around the block center,
rotates culling directions and applies UV locking in the original face basis.
The [transformed baking comparison](../benchmarks/rust-blockstate-transforms-2026-10-06/scoped-parity.json)
checks every distinct model/rotation/UV-lock tuple in bundled blockstates and
all 128 rotation/lock combinations on 81 synthetic models, plus further arbitrary
element rotations. Positions, UVs and every orientation/inverse-face matrix
compare by exact float bits: 19,704 roots and 148,439 quads match, including
7,884 bundled model/transform tuples. Quaternion basis rounding and numeric identity
rechecking are retained. This remains geometry validation; atlas packing and pixel parity are separate work.

```sh
python3 tools/test_blockstate_transform_parity.py
```

Blockstate resources now decode into ordered variant selectors and multipart
conditions. The [native blockstate comparison](../benchmarks/rust-blockstate-model-decoding-2026-10-06/scoped-parity.json)
covers all 1,288 bundled definitions and 1,351 synthetic cases (2,639 total,
468 expected rejections). It uses the same `StrictJsonParser` and
`BlockStateModelDispatcher.CODEC` as the pinned loader. The Rust representation
preserves single versus weighted variants, all three quarter-turn axes, UV-lock
flags, positive weights and total-weight limits, ordered selectors, nested
AND/OR conditions and per-term negation.

```sh
python3 tools/test_blockstate_model_parity.py
```

Property binding now selects variant roots or multipart part indices for an
ordered native state list. The [binding comparison](../benchmarks/rust-blockstate-model-binding-2026-10-06/scoped-parity.json)
covers all 1,288 bundled definitions against 35,723 registered block states plus
the four special item-frame states, followed by synthetic overlap, malformed
selector and predicate cases: 3,802 definitions and 687,466 state results match,
with 220 expected binding errors. It checks the native replacement-before-abort
behavior for overlapping variants, eager multipart validation even when variants
cover every state, integer-property aliases, nested conditions and empty part
selections. The controlled chunk renderer now uses this binder for its model
catalog.

The Java integer-property parser is shared by `horizon-core`, client binding and
world-state decoding. Its [codec regression](../benchmarks/rust-blockstate-model-binding-2026-10-06/shared-integer-codec.json)
matches 221,261 output lines against each authenticated client/server JAR.

```sh
python3 tools/test_blockstate_binding_parity.py
```

`CompiledChoices` prepares cumulative weights once per definition and shares
those choices across bound states. Selecting an ordinary single variant consumes
no random draw; a weighted singleton still does. Multipart collection draws a
seed even for an empty subset and resets the same stream before each chosen
part. The [native choice comparison](../benchmarks/rust-blockstate-model-choice-2026-10-06/scoped-parity.json)
uses actual `WeightedVariants.collectParts` and `MultiPartModel.collectParts`
with inert observable leaves. All seven distinct bundled weight patterns,
synthetic large weights and random multipart graphs produce matching selections
and subsequent random values: 6,960 fixtures, 111,360 collections, both legacy
and xoroshiro streams. The development gallery now bakes and renders selected
models using the legacy stream and the default block position seed. Specialized
block seed callbacks remain separate work.

```sh
python3 tools/test_model_choice_parity.py
```

The sprite animation module reproduces the pinned metadata codec's integer
conversion, strict primitive types, optional null fields and frame dimensions.
It preserves repeated frames, filters out-of-image frame indices, and treats
sequences with fewer than two valid entries as static physical frame zero.
Frame lookup uses cumulative durations and binary search, so skipped rendering
frames do not require replaying all elapsed animation ticks. The native oracle
also checks dirty flags, draw requirements and interpolation timing, including
transitions near the maximum Java integer duration. Interpolation pixel output
and resource-loader failure recovery are outside this comparison.

```sh
python3 tools/test_sprite_animation_parity.py
```

The [animation comparison](../benchmarks/rust-sprite-animation-2026-10-06/scoped-parity.json)
records 907 fixtures and 258,996 native tick states from the actual client codec
and `SpriteContents.AnimationState.tick`, including all 63 bundled animated
texture metadata sections and synthetic boundary cases. The associated
[startup report](../benchmarks/rust-sprite-animation-2026-10-06/startup-smoke.json)
checks the renderer using this shared implementation. Images, input
fixtures and native traces stay private under `.local/`.

```sh
cargo +stable test -p horizon-client
cargo +stable clippy --workspace --all-targets -- -D warnings
python3 tools/test_native_client.py
# Headless Linux alternative:
python3 tools/test_native_client.py --vscreen
```

The mesh tests compare expanded rectangles against independently enumerated
exposed voxel faces, including solid neighbors, cavities, material boundaries,
negative chunk origins and random surroundings. The startup runner launches a
nine-chunk scene, requires 30 presented frames and a clean exit, verifies 3,169
greedy cube rectangles and 179 other model faces across 18 sprite layers, and writes GPU,
platform, binary and source information to ignored
`.local/horizon-client/startup-smoke.json`. It has an external timeout as well as
the application's frame deadline. These checks do not establish gameplay or
pixel parity with Java. Source and resource fingerprints must remain unchanged
during the run. An animation test covers duration boundaries, wrapping and
repeated physical frames independently of render frequency.

Initial local validation uses Linux x86_64, a headless Wayland compositor and
Vulkan llvmpipe software rendering. Textured geometry and resizing were visually
inspected; movement, arrow rotation and Ctrl+Q were exercised using virtual
physical key codes. Captured relative mouse movement and Escape release were
exercised through a virtual pointer. Native macOS/Metal and
Windows/Direct3D 12 runtime tests remain pending. Software-renderer frame rates
are not evidence of a hardware speedup.

The model gallery was also inspected at close range on Linux/Vulkan llvmpipe.
With the camera stationary, screenshot differences were confined to the lantern
as its frames changed. The [model-client startup report](../benchmarks/rust-native-model-client-2026-10-06/startup-smoke.json)
records the bounded presentation check and the scope of this visual inspection.

The [blockstate-client report](../benchmarks/rust-native-blockstate-client-2026-10-06/startup-smoke.json)
also checks all 16 gallery placements against actual Java registry binding, root
baking and model collection using `state.getSeed(position)`. A tagged empty
geometry adapter identifies the 20 selected model parts and their transforms;
the separate baking gate covers actual quad geometry. Preparation exports typed
property domains and canonical state order, which the Rust loader checks against
its imported registry before binding. Each distinct transformed model is baked
once during gallery preparation and reused across positions. The selected state
IDs, properties, positions and models are recorded in the startup report.

[Neighbor-face visibility](face-visibility.md) now has native client/server
coverage across all registered states and suppression-rule families. Integrating
that result with resident chunk model meshing is now covered by the scene check
below.

The [resident-model report](../benchmarks/rust-native-chunk-models-2026-10-06/startup-smoke.json)
checks all 10,919 occupied cells in the bounded startup scene against a separately
constructed native scene. Actual Java state-model selection, parent geometry,
rotated culling directions and `Block.shouldRenderFace` produce the same 5,991
visible and 59,636 culled model faces. The sixteen featured model selections also
retain their direct native comparison. Expanded greedy-rectangle tests cover
rotated and mirrored UV bases, texture and shade boundaries, negative origins,
neighbor seams and cavities; cube projection tests cover all XYZ quarter turns.
These checks cover CPU mesh behavior and GPU startup, not native shader pixels
or a hardware performance comparison.

The [landing interactive smoke check](../benchmarks/rust-native-landing-2026-10-07/interactive-smoke.json)
exercises F5 walking from the initial camera position. The player lands at y=4,
resets fall distance and records one damage request of 9.0. That historical
checkpoint preceded the health integration below. The Linux window exits cleanly after visual inspection.

The [health integration comparison](../benchmarks/rust-native-health-2026-10-07/scene-motion.json)
connects actual server-player hurt to the same live attributes, synchronized
health/absorption storage and hunger state used by the resident player. It checks
90,065 operations in 1,881 scenarios against both pinned JARs. Repeated hits,
absorption, armor/toughness, health limits, damage cooldown boundaries, lethal
hits and nonfinite incoming damage retain native results and event order.
Travel reads the live movement-speed attribute through the actual native player
getter, independently of the base-living stored speed field. This removes a
controlled getter from the older movement harness and corrects the runtime
query. Dead-or-dying state disables movement input and jumping. Server-player jumps now
add the native exhaustion amount and queue a jump statistic, including when
jump strength prevents upward motion. The damage-clock fields advance at 20 Hz;
clock operations in the comparison establish elapsed-time state rather than
invoking complete native tick methods. That checkpoint preceded the hunger
integration below.

The scene loads native-decoded damage definitions, damage tags and health field
IDs from the pinned private resources. Statistics, combat records, sound,
network broadcasts and advancement criteria are queued service requests. The
death consumer marks the stored actor dead and queues a request; drops, removal,
respawn and the death screen are unfinished. F5 constructs a fresh development
actor when walking is enabled. This is not vanilla respawn behavior.

The [health interactive smoke check](../benchmarks/rust-native-health-2026-10-07/interactive-smoke.json)
enables walking and F3 diagnostics, lands from the initial position, then walks
and jumps before exiting. Health falls from 20 to 11, matching the single damage
request of 9.0; the cooldown expires to zero. The final report is grounded at y=4
with 110 movement ticks and no error. The ground view was visually inspected.
The [startup check](../benchmarks/rust-native-health-2026-10-07/startup-smoke.json)
also passes with the additional health resources. These are Linux development
checks, not release acceptance or a hardware performance comparison.

The [hunger integration comparison](../benchmarks/rust-native-hunger-2026-10-07/scene-motion.json)
checks 196,489 operations in 2,837 scenarios against both pinned references.
The resident simulation now ticks hunger after movement. It binds actual food,
saturation, exhaustion and timer fields to the same live health and attribute
storage, including regeneration and starvation through registered damage sources.
The damage callback borrows the FoodData currently being ticked, so exhaustion
writes remain visible before its timer resets. Peaceful recovery also runs in
the native Player.aiStep phase before movement. Difficulty and natural
regeneration settings live with the resident world.

The reference executes actual `FoodData.tick`, `ServerPlayer.tickRegeneration`,
health setters, armor/absorption and hurt methods. Cases cover all four
difficulties, starvation health limits, absorption during starvation, changing
regeneration rules, fast/slow healing timers, timer overflow, signed zero,
nonfinite food state and combined falling/movement/food ticks. Explicit clock
setup supplies elapsed-time fields; complete enclosing `tick`/`doTick` methods
remain outside this comparison. The default walking scene uses Normal
difficulty with natural regeneration enabled. At that checkpoint food item use,
movement-distance exhaustion, effect ticks, full death/respawn services and save integration remained
unfinished.

The [interactive hunger check](../benchmarks/rust-native-hunger-2026-10-07/interactive-smoke.json)
enabled walking and F3 diagnostics on Linux. After one landing damage request
of 9, stored health recovered from 11 to 12.5 as saturation fell from 5 to 3;
food remained 20 and exhaustion ended at 1. The player remained alive and
grounded at `[25, 4, 25]`, with an expired hurt cooldown, after 42 movement ticks.
The ground view was visually inspected and the client exited without an error.
The development client pauses movement without focus, so wall time is not
simulation time. This checks the resident simulation and window, not full
gameplay, release acceptance or hardware performance.

The [movement statistics comparison](../benchmarks/rust-native-movement-statistics-2026-10-07/scene-motion.json)
checks 215,204 operations in 5,510 scenarios against both pinned references.
The resident simulation now feeds the change in player position to the movement
statistics method after movement and before hunger ticking. Sprint distance
adds exhaustion to the same stored food state; ordinary walking adds zero.
Climbing and airborne distance also produce their native statistic requests.
Statistics persistence and scoreboard delivery remain unfinished.

Vanilla invokes `ServerPlayer.checkMovementStatistics` from its packet listener
when accepting movement, separately from `Entity.move` and travel. The resident
scene directly accepts its resolved local displacement at this boundary; its
comparison invokes the actual native statistics method. It does not yet run the
packet listener's acceptance, rejection or scheduling logic. The shared kernel
also covers swimming, underwater walking, water-surface walking, crouching and
gliding. Direct method fixtures check all 512 overlapping query-flag combinations,
invulnerable abilities, float-versus-double centimeter rounding, integer overflow,
zero and nonfinite deltas. Those query overrides are restricted to statistics
fixtures; that checkpoint used dry, standing resident physics. Combined scenes exercise
repeated movement, sprint-state changes, jumps, hunger consumption and healing.
That checkpoint preceded the local sprint controls described below.

The [Linux window check](../benchmarks/rust-native-movement-statistics-2026-10-07/interactive-smoke.json)
enabled walking, showed diagnostics and held W for 650 ms. The player moved from
`[25, 4, 25]` to `[23.12939004788537, 4, 23.12939004788537]`, remained grounded
and alive, and exited without an error after 82 movement ticks. Health recovered
to 14 after the initial fall. The ground view was visually inspected; this is a
client smoke check, while statistic amounts and exhaustion are checked by the
native comparisons above.

The [sprint input comparison](../benchmarks/rust-native-sprint-2026-10-07/sprint-input.json)
checks 502,112 evolving steps in 146,648 scenarios. It executes the actual
`LocalPlayer.aiStep` prefix, including `KeyboardInput.tick` and the private
sprint eligibility and stop methods. A sentinel ends execution at the riding
boundary before parent movement. The fixtures supply environmental, effect,
vehicle and UI queries; actual food/ability storage and item `USE_EFFECTS`
components supply the remaining conditions. The comparison covers all combinations
of 17 condition flags, keyboard states and previous input, food thresholds,
timer boundaries, sprint windows from 0 to 10, and double-tap sequences. It
checks each ordered sprint setter request, including repeated requests.

The [combined sprint scene](../benchmarks/rust-native-sprint-2026-10-07/sprint-scene.json)
then checks 13,387 operations in 419 scenarios. Its Java input actor shares the
native scene actor's food and collision state, and its sprint setters update
that actor's real attributes. Actual input preparation, movement, distance
exhaustion, hunger and healing follow. Snapshots compare the sprint timer and
previous input alongside position, velocity, modifiers and health/food fields.
Cases include collisions, releasing the sprint key while moving, double taps,
jumps, and hunger crossing the sprint threshold. The
[common scene check](../benchmarks/rust-native-sprint-2026-10-07/scene-motion.json)
also passes against both pinned references.

The walking client now samples sprint input on each simulation tick and uses
the default seven-tick double-tap window. The live food and collision fields
control eligibility; sprint changes update the existing movement-speed
attribute. The F3 title shows sprint state, and exit reports include sprint
ticks and movement-distance counters. These counters are diagnostics, not saved
statistics. Fluid movement, vehicles, effects, item use, input remapping and the
complete client/server tick remain unfinished. The input comparison tests those
eligibility queries in isolation; the sprint checkpoint used dry, standing
actors with empty equipment. The crouch integration below extends that domain.

```sh
python3 tools/test_sprint_input_parity.py
python3 tools/test_native_scene_motion.py --sprint-input
```

Linux window checks exercised both [Left Ctrl](../benchmarks/rust-native-sprint-2026-10-07/sprint-key-smoke.json)
and [double-tap W](../benchmarks/rust-native-sprint-2026-10-07/double-tap-smoke.json).
The Ctrl run recorded 71 sprint ticks and 1,948 sprint centimeters. The double-tap
run visibly showed `Sprint true` during the second press and `Sprint false`
after releasing W. Both runs ended grounded and alive without an error, and
their views were visually inspected. These are development client checks;
macOS/Windows runtime acceptance and complete gameplay remain pending.

## Crouching and dry pose transitions

Left Shift now binds crouching to the resident player. Local crouch selection
uses the previous keyboard state before the new input is sampled; that local
state controls slowdown and sprint eligibility. The current Shift input controls
ledge backoff, descending collision context, bounce suppression and ladder
sliding. After movement, pose-fit checks choose standing, crouching or crawling
and refresh the cached bounds and eye height. Releasing Shift under a ceiling
does not force the actor into a standing volume that cannot fit.

The [combined input scene](../benchmarks/rust-native-crouch-2026-10-07/input-scene.json)
checks 21,241 operations in 514 scenarios against the pinned client, including
the actual `LocalPlayer.aiStep` input prefix and `modifyInput`. The
[shared scene comparison](../benchmarks/rust-native-crouch-2026-10-07/scene-motion.json)
checks 223,645 operations in 5,605 scenarios against both pinned JARs. It invokes
actual native pose selection, dimension refresh and edge backoff. The new cases
include low ceilings, blocked pose changes, scale and sneaking-speed attributes,
diagonal ledges, step-height thresholds, jumping, release transitions and landing
on different materials. Seven output lines per operation compare motion,
attributes, events, health, hunger, input history and pose/dimension bits.
The existing [isolated sprint comparison](../benchmarks/rust-native-crouch-2026-10-07/sprint-input.json)
also passes after updating its scene controller.

The native fixture controls pose storage and invokes the actual dimension-refresh
body when the pose changes. Full synchronized pose data and network notifications
remain unfinished. Scale changes use explicit refresh operations; the complete
pending attribute-update lifecycle is separate. Crawling here means dry collision
pose selection; swimming physics, sleeping, flight and death poses are unbound.

The client camera follows the cached eye height with the inspected native float
smoothing arithmetic. Full camera behavior and pixel parity are not established.
The [Linux window check](../benchmarks/rust-native-crouch-2026-10-07/interactive-smoke.json)
visibly switched from crouching to standing after releasing Shift. It recorded
100 crouching ticks and 613 crouching centimeters, ending alive and grounded
without an error. Both views were inspected. The
[renderer startup check](../benchmarks/rust-native-crouch-2026-10-07/startup-smoke.json),
seven client tests and all-target client Clippy also passed. These checks do not
establish full vanilla gameplay or final macOS/Linux/Windows release acceptance.
