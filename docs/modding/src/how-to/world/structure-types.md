# Structure types

Register a `StructureType` in `Hooks::register` to generate structures named by a data pack's
`"type"` field. For example, a structure JSON with `"type": "mymod:tower"` uses your registered
`mymod:tower` implementation:

```rust,ignore
{{#include ../../../examples/src/worldgen.rs:structure}}
```

Use an ID in your mod's namespace. A duplicate ID, an ID in another mod's namespace, or an empty
name stops startup with a message naming your mod.

## Generating a start

`StructureType::find_generation_point(&self, context: &mut dyn GenerationContext)` returns
`Some(FoundStructure { position, pieces })` for a start, or `None` to skip the chunk. Supply at
least one piece, each an `Arc<dyn StructurePiece>`.

World generation has no `Host`. Starts run on worker threads, and pieces place per chunk. Your
`StructureType` must be `Send + Sync`, stateless and deterministic: the same seed produces the
same pieces, even when workers generate neighbouring chunks concurrently.

| `GenerationContext` access | Purpose |
|---|---|
| `seed`, `chunk`, `sea_level`, `min_y`, `max_y` | World and chunk coordinates and bounds |
| `random` | `StructureRandom` seeded with vanilla's `setLargeFeatureSeed` for the chunk |
| `base_height`, `base_columns`, `noise_column` | Height and base-column queries |
| `noise_biome`, `column_has_valid_biome` | Opaque biome identities and the structure's valid-biome test |
| `metadata` | Borrowed immutable block metadata |
| `config` | The structure JSON converted to a shared `Arc<Tag>` |

Config conversion runs when definitions load or reload, not per start. Nested JSON nulls are
refused at load. The engine handles structure-set spacing, `biomes` and `terrain_adaptation`.

Both contexts are traits. `StructureRandom` provides `next_int_bound`, `next_long`, `next_float`,
`next_double` and `next_boolean`. These interfaces also support structures in `mod.wasm`.
Wasm worker calls share the dependency group's store lock and use a per-call fuel or epoch
budget; a trap stops the group with a typed refusal.

## Placing pieces

Implement `StructurePiece` (`Debug + Send + Sync`) with these methods:

| Method | Contract |
|---|---|
| `bounding_box` | Return the piece's world-space `BoundingBox`. |
| `post_process` | Place blocks and entities through `&mut dyn PieceContext`; return `Result<(), StructureError>`. |
| `add_additional_save_data` | Write your piece's fields into its saved `Compound`. |
| `beardifier` | Optional `Option<PieceBeardifier>`, default `None`: override the terrain box, adjustment and ground-level delta. |

After the start's pieces place, `StructureType::after_place` runs once per chunk with every
piece's current bounding box. Its default implementation does nothing.

Construct an override with `PieceBeardifier::new(bounds, TerrainAdjustment::BeardThin, 2)`.
The bounds are independent of the placement box; the ground is `bounds.min[1] + ground_level_delta`.
The rules are `None`, `Bury`, `BeardThin`, `BeardBox` and `Encapsulate`. An answer with
adjustment `None` disables only that piece. No answer inherits the structure's setting.
An override works even when the structure's `terrain_adaptation` is `"none"`.
Native and wasm pieces are queried once when generated or loaded, and the answer is cached
on the engine piece; terrain generation never calls the mod for it per chunk.

Use `PieceContext` to work within the chunk:

| Method | Behaviour |
|---|---|
| `chunk`, `chunk_box` | Identify the chunk and writable box. |
| `set_block` | Write a `BlockState`; silently skip positions outside the writable box. |
| `block`, `height` | Read blocks and heightmap heights. |
| `state`, `state_with` | Resolve a registered `BlockKey` to an API-owned `BlockState`. |
| `random` | Draw from the piece's `StructureRandom`. |
| `metadata` | Borrow block metadata; `metadata().name(state.block())` returns the canonical name. |
| `solid_render` | Check whether a state renders as a solid block. |
| `entity` | Queue saved entity NBT, including its `id`, `Pos` and other fields. |
| `block_entity` | Place block entity NBT at a position. |

Entities remain in the generating chunk until it joins the world. Keep the `"horizon:finalize"`
byte carried by structure-template entities to request `Mob.finalizeSpawn` with the `STRUCTURE`
reason when the game loads the entity.

Resolve block states once during piece construction or loading. Retain those states for
`post_process` rather than resolving names for every placed block.

## Saving and loading

The engine saves the structure type ID and the fields `BB`, `O` and `GD`, then calls
`add_additional_save_data`. On chunk load, your type's
`load_piece(metadata, tag) -> Result<Arc<dyn StructurePiece>, StructureError>` rebuilds the piece
from that data; the engine does not regenerate it from the seed. Return `StructureError` for
invalid saved block names, properties or piece data. The loader receives the same retained
metadata snapshot that worker contexts borrow.

- Every piece saves with its structure type's ID. There is no separate `STRUCTURE_PIECE` registry.
  If your type uses several piece kinds, save a discriminator in `add_additional_save_data` and
  read it in `load_piece`.
- The engine writes `O` as `-1` and `GD` as `0`. Save your own orientation or depth fields if you
  need them; do not interpret these defaults as your piece's orientation or depth.
- Save canonical block names and properties, never process-local ordinals.

Mod pieces participate in start caching, cross-chunk placement, terrain blending through the
beardifier, and `/locate structure`. A data pack structure whose `"type"` has no registered
implementation places nothing.
