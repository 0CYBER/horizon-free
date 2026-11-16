//! The `register_block_tint_sources` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The tint sources of blocks (NeoForge's `RegisterColorHandlersEvent.BlockTintSources`,
    /// vanilla's `BlockColors.register`): `register(layers, blocks)`, and a model face with
    /// `tintindex` i is multiplied by layer i of its block's list; a face past the list is not
    /// tinted. A layer is one of vanilla's sources (`VanillaTint`) or the mod's own
    /// `BlockTintSource`. Blocks are ids, a mod's or vanilla's, and a later registration for a
    /// block replaces the earlier one, vanilla's own colours included. Asked once at start on the
    /// client, before resources load; every resource load, a pack reload included, applies the
    /// registrations again. An id no block has is logged and skipped.
    #[notes = r#"## Block tint sources

A block tint source (NeoForge's `RegisterColorHandlersEvent.BlockTintSources`) colours a greyscale block model,
as vanilla colours grass and leaves: `Hooks::register_block_tint_sources` registers each block's list of layers
once at start, and a model face with `tintindex` i is multiplied by layer i. A layer is one of vanilla's sources
(`VanillaTint`: `Constant`, `ConstantSplit`, `Grass`, `Foliage`, `Water`, `Redstone`...) or the mod's own
`BlockTintSource`, with vanilla's four methods:

- `color(state, metadata)`: the colour away from a level (a block in a minecart, a mod's drawn block);
- `color_in_world(state, metadata, level, pos)`: the chunk mesher's colour, and a block a piston moves; it defaults to
  `color`;
- `color_as_terrain_particle(state, metadata, level, pos)`: the bits the block breaks into; it defaults to
  `color_in_world`;
- `relevant_properties()`: the properties the colour depends on. Vanilla groups states for shared baked models
  by them; Horizon bakes every state and does not read it.

```rust,ignore
struct Dyed;
impl BlockTintSource for Dyed {
    fn color(&self, state: &BlockState, metadata: &dyn BlockMetadata) -> u32 {
        match state.property(metadata, "color").ok().flatten() {
            Some("red") => 0xFFB02E26,
            _ => 0xFFF9FFFE,
        }
    }
}

fn register_block_tint_sources(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterBlockTintSourcesEvent<'_>,
) {
    event.sources.register(
        vec![TintLayer::Source(Arc::new(Dyed)), VanillaTint::Grass.into()],
        vec!["example:dyed_lamp".into()],
    );
}
```

Colours are ARGB; the faces take their RGB, as vanilla's block renderer does. The mesher asks `color_in_world`
on its worker threads, once per block and tint index each time it meshes the section, so a source keeps no
mutable state and gets no `Host`. Each colour method receives an explicit immutable `BlockMetadata` resource, also used by `BlockState::property`, `with` and `of`. `level` is a `BlockAndTintGetter` over the section's chunk and its eight
neighbours: block and fluid states, light, face shading and `block_tint(pos, resolver)` for the biome colours.
Block entities are not part of it. A block with no mod layers meshes as before, and the registrations hold after
a pack change or F3+T. Fluids take their colour from their fluid model, not from these registrations."#]
    #[hook(
        side = "client",
        contract = "listen",
        context = "registration",
        neoforge = "RegisterColorHandlersEvent.BlockTintSources"
    )]
    fn register_block_tint_sources();
}
