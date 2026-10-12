# Block behaviours and ticking: parity status

Area: random and scheduled block ticks, growth, bone meal, TNT and
explosions, hoppers, dispensers and droppers, comparators reading
containers, weather-driven blocks. The library code lives in
`crates/horizon-world/src/level/`. The client glue lives in
`crates/horizon-client/src/game/` (`growth.rs`, `tnt.rs`, `explosion.rs`,
`machines.rs`, `blockctl.rs`).

## Implemented (ported from the 26.3 sources)

- **Growth through live feature placement.** `worldgen/live.rs` decodes
  configured and placed features by name from the worldgen bundle. It shares
  the generator's `GeneratorData`. `level/feature_world.rs` runs features on
  the live level as a `FeatureWorld` and `PlacementContext`, and writes
  blocks through `Behaviours::set_block` with the feature's flags. Tree
  placement also calls `updateShapeAtEdge`.
- **`TreeGrower.growTree`** (`level/growth.rs`) covers every vanilla grower:
  oak and its bee variants (with the flower check), spruce, birch, jungle,
  acacia, cherry, mangrove, azalea and poplar. It also covers the 2×2 forms:
  dark oak, pale oak, mega spruce and pine, and mega jungle. It keeps the
  random draw order: mega feature, then flowers, then feature. Saplings are
  removed with flags 818 and restored with flags 260 when the tree does not
  fit. Saplings, propagules and azaleas grow both on random ticks and with
  bone meal.
- **Bone meal** (`level/bonemeal.rs`) ports `BoneMealItem.useOn`,
  `growCrop` and `growWaterPlant`, including coral from bone meal in warm
  ocean biomes. It also ports every vanilla `BonemealableBlock`:
  - saplings, propagules, mushrooms, fungi and azaleas
  - grass (with the biome's `getBoneMealFeatures`), nylium, netherrack and
    moss
  - rooted dirt, mangrove leaves, crops (including beetroot and torchflower
    increments) and stems
  - pitcher, bamboo and bamboo sapling, cocoa and sweet berries
  - tall grass and fern, tall flowers, bush and firefly bush, flower beds
  - kelp, twisting and weeping vines, cave vines (head and body), seagrass
    and sea pickles
  - big and small dripleaf, glow lichen (`MultifaceSpreader`), hanging moss
    and pale moss carpet
  - short and tall dry grass, and shelf mushroom

  Level event 1505 plays the bone meal sound.
- **Fixes to existing ticks.** Torchflower crops now mature into
  torchflowers. Kelp grows into any water block, as in vanilla.
- **Primed TNT** (`game/tnt.rs`):
  - It has an 80-tick fuse, gravity 0.04, 0.98 drag and the 0.7/−0.5 ground
    bounce.
  - It explodes with power 4 at `y + 0.06125`.
  - It renders with the `TntRenderer` swell and a white flash every 5 ticks
    (entity overlay 4 in `entity.wgsl`).
  - TNT is primed by redstone, fire, flint and steel or fire charges
    (`TntBlock.useItemOn`), and by players breaking unstable TNT.
  - Explosions prime TNT with a random short fuse (chain reactions).
  - Dispensers prime TNT too.
- **`ServerExplosion`** (`game/explosion.rs`):
  - It marches 1352 rays through a Java `HashSet<BlockPos>`
    (`java_hash_map`), so iteration and the `Util.shuffle` draws match
    vanilla.
  - Each block costs `max(block, fluid)` resistance.
  - Damage and knockback scale with exposure for the player, mobs, items
    and TNT. Creative players take no damage.
  - TNT explosions drop every block. Mob explosions use drop decay.
  - Drops merge in stacks of up to 16. Container contents spill.
- **Hoppers** (`game/machines.rs`):
  - They push and pull through `HopperBlockEntity.tryMoveItems`.
  - The 8-tick cooldown applies, with the transfer cooldown hand-off between
    hoppers.
  - They insert into furnace faces (`WorldlyContainer` slots, fuel and
    bucket rules) and into double chests (right half first).
  - They respect `does_not_block_hoppers` and pick up item entities from the
    suck box.
  - The `enabled` property stops them.
- **Dispensers and droppers.** They use `getRandomSlot` and the drop
  position and velocity from `DefaultDispenseItemBehavior.spawnItem`. The
  dispense behaviours are:
  - bone meal
  - TNT
  - flint and steel (fire, TNT, campfires and candles)
  - water and lava buckets (including waterlogging)
  - empty buckets (fluids, waterlogged blocks and powder snow)
  - arrows (power 1.1, uncertainty 6)

  Droppers insert into containers. Clicks and failure sounds play.
- **Comparators reading containers.** `GameLevel` answers
  `block_entity_analog_signal` with
  `AbstractContainerMenu.getRedstoneSignalFromContainer`. Chests blocked by
  a conductor read 0. Double chests are combined. Container changes from
  hoppers, dispensers and the player call `container_changed`, which runs
  `updateNeighbourForOutputSignal`.
- **Weather** (`level/weather.rs`) uses biome temperatures from the worldgen
  bundle, with the height adjustment and frozen noise. They feed
  `ServerLevel.tickPrecipitation`: ice formation, snow accumulation, and
  cauldrons filling with water or powder snow. They also feed
  `Level.isRainingAt`, which drives farmland moisture and fire going out in
  rain.
- **Day-timeline blocks** (`level/timeline.rs`):
  - Eyeblossoms open and close, and potted eyeblossoms follow. Neighbours are
    scheduled in the `BlockScanUtils` scan order.
  - Turtle eggs crack and hatch on sand, using the dawn hatch chance.
  - Dried ghasts hydrate while waterlogged.
- **Landing writes.** Farmland trampling and turtle eggs crushed by landing
  now run neighbour and shape updates, so crops on trampled farmland pop.
- **Smaller fixes.**
  - Hopper, dispenser and dropper block entities save with their own ids.
    Barrels and trapped chests do too.
  - Their screens use the vanilla hopper and dispenser layouts.
  - `GameLevel::max_y` is exclusive, as `LevelAccess` documents.

- **Block entity removal** (`level/removal.rs`, `game/spill.rs`):
  `LevelChunk.setBlockState` runs `BlockEntity.preRemoveSideEffects` when a
  block's block entity goes with it. Containers spill through
  `Containers.dropItemStack`, jukeboxes and lecterns pop their item, shulker
  boxes keep theirs, copper chests and statues keep theirs across weathering.
  `Block.dropResources` draws when the behaviour calls it and awards ore,
  sculk and spawner experience; the item entities join the world before the
  behaviour returns.
- **Copper golems** (`mobs/copper_golem.rs`, `game/copper_golems.rs`): built
  from a pumpkin on a copper block (`CopperChestBlock.getFromCopperBlock`),
  sheared and equipped by dispensers, weathering, waxing, statues.

## Verification

- `tools/test_block_removal_parity.py`: 4,000 replacements, destructions and
  removals of block entity holders with contents match vanilla. The dispenser
  harness builds, shears and equips copper golems (6,000 cases match).

- Unit tests in `crates/horizon-world/src/level/growth_tests.rs` cover:
  - every sapling kind, including the 2×2 trees
  - azalea and mangrove growth
  - random-tick stage advance followed by growth
  - grass bone meal with flowers
  - huge mushrooms and crimson fungi
  - crop ages, torchflowers, ferns and tall flowers
  - snow, ice and rain answers
  - eyeblossoms and turtle eggs

  They need the feature block exports in `worldgen/blocks`
  (`block-properties.nbt`). The shared `.local` bundle lacks those files, so
  run the tests with `HORIZON_RESOURCES` pointing at a complete prepared
  bundle. Without one, the tests skip.
- Client tests cover the Java `Vec3i` hash and compare order and the
  explosion stack merging.
- In the game on the virtual screen, driven by `horizon-ctl`:
  - Bone meal on oak, birch, cherry, 2×2 dark oak, spruce and jungle
    saplings grew all of them.
  - Flint and steel on a TNT cluster: the TNT flashed white and chain
    exploded, leaving a crater with drops.
  - A redstone block primed TNT.
  - A chest drained through a hopper chain into a chest at 2.5 items/s. The
    comparator lit a lamp.
  - A dispenser bone-mealed a sapling into a tree. A dropper ejected an item.
  - Landing on farmland turned it to dirt and popped the wheat.
  - The hopper screen shows the vanilla layout.

  New `horizon-ctl` commands drive these checks: `setblock`, `fill`,
  `getblock` and `container`.

## Known gaps

- **Random draws.**
  - Features placed from block behaviour draw from a copy of the level
    random. Their `next_world_*` draws and any draws in neighbour updates
    during placement do not reach the real stream.
  - The client picks randomly ticking sections from their palettes, so a
    section whose palette still lists a ticking state no block uses any more
    gets random ticks that vanilla's exact counts would skip.
- **Explosion details.** Exposure uses full-collision blocks only.
  `getEntities` order is approximate. Experience orbs, falling blocks and
  arrows are not pushed.
- **Missing entities.**
  - Dispensers have no behaviour for fire charges, other projectiles,
    armour, shulker boxes, boats, minecarts or spawn eggs. Fire charges and
    most other items drop instead; projectiles, boats, minecarts and spawn
    eggs are not entities in Horizon.
  - Turtle hatchlings and happy ghastlings are reported as `BlockEvent`s and
    not spawned.
  - Hoppers do not see minecart containers. Item entities do not trigger the
    hopper's `entityInside`; suction covers the same box on the hopper tick.
- **Not implemented.** The sculk vibration system (sensors, shriekers,
  catalyst spread) and creaking hearts. Lightning (`tickThunder`).
  Particles for bone meal, explosions and TNT smoke.
- **Not saved.** Primed TNT and hopper cooldowns (`TransferCooldown`).
