//! The `register_color_resolvers` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The colour resolvers block tints blend over the biomes around a block (NeoForge's
    /// `RegisterColorHandlersEvent.ColorResolvers`): `register(resolver)` adds the mod's own
    /// `BiomeColorResolver` and returns the `ColorResolver` its block tint sources pass to
    /// `BlockAndTintGetter::block_tint`. The game blends a mod's resolver over the biome blend
    /// radius and caches it per section while it meshes, as it does vanilla's grass, foliage and
    /// water. Asked once at start on the client, before `register_block_tint_sources` and before
    /// resources load.
    #[notes = r#"## Colour resolvers

A colour resolver (vanilla's `ColorResolver`, NeoForge's `RegisterColorHandlersEvent.ColorResolvers`) is a colour
calculated from a biome and a block column, which the game blends over the biomes around a block as it blends
grass, foliage and water: each block averages the resolver's colour over the `(2r+1)^2` columns of the biome blend
radius, each column in the biome the game zooms it to. `Hooks::register_color_resolvers` registers the mod's own
`BiomeColorResolver` once at start and gets back the `ColorResolver` to ask for, and the mod's `BlockTintSource`
reads it with `level.block_tint(pos, resolver)`:

```rust,ignore
static FROST: OnceLock<ColorResolver> = OnceLock::new();

struct Frost;
impl BiomeColorResolver for Frost {
    fn color(&self, _biome: &str, info: &ResolverBiome, _x: f64, _z: f64) -> u32 {
        if info.temperature < 0.15 { 0xE0F0FF } else { info.colors.foliage }
    }
}

struct FrostLeaves;
impl BlockTintSource for FrostLeaves {
    fn color(&self, _: &BlockState, _: &dyn BlockMetadata) -> u32 {
        0xFF48B518
    }
    fn color_in_world(
        &self,
        _: &BlockState,
        _: &dyn BlockMetadata,
        level: &dyn BlockAndTintGetter,
        pos: [i32; 3],
    ) -> u32 {
        level.block_tint(pos, *FROST.get().expect("registered at start"))
    }
}

fn register_color_resolvers(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterColorResolversEvent<'_>,
) {
    FROST.get_or_init(|| event.resolvers.register(Arc::new(Frost)));
}
```

`color(biome, info, x, z)` gets the biome's id, its temperature, downfall and colours (after `biome_colors`) and the
block column, and returns 0xRRGGBB. The mesher asks it on its worker threads, once per column the section's tinted
blocks blend over, so a resolver keeps no mutable state and gets no `Host`. Its colours are cached for one mesh of
one section only: a resource or pack reload, a biome colour refresh and a block change all mesh the section again
and ask the resolver again."#]
    #[hook(
        side = "client",
        contract = "listen",
        context = "registration",
        neoforge = "RegisterColorHandlersEvent.ColorResolvers"
    )]
    fn register_color_resolvers();
}
