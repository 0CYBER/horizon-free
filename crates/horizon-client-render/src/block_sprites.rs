//! Where a block's textures are in the block sprite arrays, side by side, for drawing the block
//! somewhere the mesher does not: a native mod's own pass drawing blocks as voxels
//! (`block_sprites`, `WorldDrawn::block_sprites`; docs/modding/src/how-to/native-mods.md). The answer is the
//! model's own faces, as the mesher bakes them: each face's sprite (its array and layer), where on
//! the block's side it lies and how its texture runs across it, and its tint source.
//!
//! Each sprite is a whole layer of a square texture array (`sprites::Sprites`), so a face's UVs run
//! 0 to 1 within its layer; the renderer's split (`Renderer::sprite_split`) layers to an array.
use crate::{mesher::TintSource, models::catalog::BakedFace, streaming::WorkerContext};
use horizon_assets::model::Direction;
use horizon_core::position::BlockPos;
use horizon_world::registry::{BlockRegistry, BlockStateId};
pub use mod_api::block::block_sprites::{BlockSprites, FaceSprite, SpriteRef};

/// The two axes a side's `rect` and `uv` count along, as indices into (x, y, z): down and up across x then z; north
/// and south across x then y; west and east across z then y. (The block's corner at 0, 0, 0 is 0 on each.)
pub fn side_axes(side: Direction) -> (usize, usize) {
    match side {
        Direction::Down | Direction::Up => (0, 2),
        Direction::North | Direction::South => (0, 1),
        Direction::West | Direction::East => (2, 1),
    }
}

/// The axis a side faces along, and where its plane is (0 or 1).
fn side_plane(side: Direction) -> (usize, f32) {
    match side {
        Direction::Down => (1, 0.0),
        Direction::Up => (1, 1.0),
        Direction::North => (2, 0.0),
        Direction::South => (2, 1.0),
        Direction::West => (0, 0.0),
        Direction::East => (0, 1.0),
    }
}

/// A block state from `name[property=value,...]`, the namespace `minecraft:` when it has none; properties left
/// out take the block's default. None for a block or a property the game does not know.
pub fn parse_state(registry: &BlockRegistry, text: &str) -> Option<BlockStateId> {
    let (name, props) = match text.trim().split_once('[') {
        Some((n, rest)) => (n, rest.strip_suffix(']')?),
        None => (text.trim(), ""),
    };
    let name = if name.contains(':') {
        name.to_owned()
    } else {
        format!("minecraft:{name}")
    };
    let default = registry.default_state(&name)?;
    let mut state = registry.state(default)?.clone();
    for pair in props.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let (k, v) = pair.split_once('=')?;
        let slot = state.properties.get_mut(k.trim())?;
        *slot = v.trim().to_owned();
    }
    registry.resolve(&state).ok()
}

/// What a fluid block (`minecraft:water`, a bubble column, lava) shows on every side instead of a model:
/// its still texture, its tint source (water's is the biome's, `MeshTables::water_tint`, at tint index 0)
/// and whether it is translucent. None for any other block.
pub fn fluid_faces(block: &str) -> Option<(&'static str, Option<TintSource>, bool)> {
    match block {
        "minecraft:water" | "minecraft:bubble_column" => {
            Some(("block/water_still", Some(TintSource::Water), true))
        }
        "minecraft:lava" => Some(("block/lava_still", None, false)),
        _ => None,
    }
}

/// Each sprite layer's texture name, as `native_draw_cube` and `native_draw_quad` take it: vanilla's
/// without its namespace (`block/stone`), a mod's with it (`mymod:block/gear`).
pub fn texture_names(
    sprite_layers: &std::collections::BTreeMap<String, u32>,
) -> std::collections::HashMap<u32, String> {
    sprite_layers
        .iter()
        .map(|(name, layer)| (*layer, name.trim_start_matches("minecraft:").to_owned()))
        .collect()
}

/// The sprites `context`'s resources hold, `split` layers to an array (`Renderer::sprite_split`), and
/// their names (`texture_names`).
pub struct Lookup<'a> {
    context: &'a WorkerContext,
    split: u32,
    names: &'a std::collections::HashMap<u32, String>,
}

impl<'a> Lookup<'a> {
    pub fn new(
        context: &'a WorkerContext,
        split: u32,
        names: &'a std::collections::HashMap<u32, String>,
    ) -> Self {
        Self {
            context,
            split: split.max(1),
            names,
        }
    }
    /// The sprite at `layer` of all the layers.
    pub fn sprite(&self, layer: u32) -> SpriteRef {
        let render = &self.context.render;
        SpriteRef {
            array: layer / self.split,
            layer: layer % self.split,
            animated: render.animations.iter().any(|a| a.layer == layer),
            cutout: render
                .sprite_mips
                .get(layer as usize)
                .is_some_and(|m| m.has_transparent),
        }
    }

    /// A texture by name (`minecraft:block/stone`, `block/stone`); none for one no model uses (the arrays hold only
    /// the textures models and the game's own drawing name).
    pub fn texture(&self, name: &str) -> Option<SpriteRef> {
        let layers = &self.context.render.sprite_layers;
        let layer = layers
            .get(name)
            .or_else(|| layers.get(&format!("minecraft:{name}")))?;
        Some(self.sprite(*layer))
    }

    /// Each sprite's pixels as the resources loaded them ([`sprite_pixels`]). The store is uncompressed once for
    /// the whole call, not per sprite.
    pub fn pixels(&self, sprites: &[SpriteRef]) -> Vec<Option<Vec<u8>>> {
        if sprites.is_empty() {
            return Vec::new();
        }
        let render = &self.context.render;
        sprite_pixels(
            &render.pixels.get(),
            render.sprite_size,
            self.split,
            sprites,
        )
    }

    /// What block `name` (`minecraft:oak_log[axis=x]`; properties left out take the default) draws on each side;
    /// none for a block the game does not know.
    pub fn block(&self, name: &str) -> Option<BlockSprites> {
        let state = parse_state(self.context.assets.registry, name)?;
        Some(self.state(state))
    }

    /// What `state` draws on each side. A fluid has its still texture on all six; a block drawn only as a block
    /// entity (a chest, a sign) or not at all (air) has empty sides, and its particle sprite if any.
    pub fn state(&self, state: BlockStateId) -> BlockSprites {
        let render = &self.context.render;
        let catalog = &render.model_catalog;
        let mut sides: [Vec<FaceSprite>; 6] = Default::default();
        let block = self
            .context
            .assets
            .registry
            .state(state)
            .map_or("", |s| s.name.as_str());
        let fluid = fluid_faces(block);
        if let Some((texture, tint, translucent)) = fluid {
            let sprite = self.texture(texture);
            if let Some(sprite) = sprite {
                for side in &mut sides {
                    side.push(FaceSprite {
                        sprite,
                        texture: texture.to_owned(),
                        tint_index: if tint.is_some() { 0 } else { -1 },
                        rect: [0.0, 0.0, 1.0, 1.0],
                        inset: 0.0,
                        uv: [0.0, 0.0, 1.0, 0.0, 0.0, 1.0],
                        tint,
                        translucent,
                    });
                }
            }
            return BlockSprites {
                sides,
                particle: sprite,
                cube: sprite.is_some(),
            };
        }
        let models = catalog
            .collect(state, BlockPos::new(0, 0, 0))
            .unwrap_or_default();
        let mut all = 0;
        for face in models.iter().flat_map(|(_, m)| &m.faces) {
            all += 1;
            let Some((side, sprite)) = self.face(state, face) else {
                continue;
            };
            sides[side as usize].push(sprite);
        }
        let particle = catalog
            .particle(state)
            .or_else(|| {
                models
                    .iter()
                    .flat_map(|(_, m)| &m.faces)
                    .map(|f| f.layer)
                    .next()
            })
            .map(|layer| self.sprite(layer));
        // a cube: every side has a whole face at its plane (and perhaps overlays laid on it), and the model has no
        // other faces
        let whole = |f: &FaceSprite| f.rect == [0.0, 0.0, 1.0, 1.0] && f.inset == 0.0;
        let cube = all == sides.iter().map(Vec::len).sum::<usize>()
            && sides.iter().all(|s| !s.is_empty() && s.iter().all(whole));
        BlockSprites {
            sides,
            particle,
            cube,
        }
    }

    /// A model face as it lies on its side; none when it does not lie flat on one.
    fn face(&self, state: BlockStateId, face: &BakedFace) -> Option<(Direction, FaceSprite)> {
        let q = &face.quad;
        let side = q.facing;
        let (normal, plane) = side_plane(side);
        let depth = q.positions[0][normal];
        if q.positions.iter().any(|p| (p[normal] - depth).abs() > 1e-4) {
            return None;
        }
        let (a, b) = side_axes(side);
        let ab: [[f32; 2]; 4] = q.positions.map(|p| [p[a], p[b]]);
        let uv = affine(&ab, &q.uv)?;
        let (mut a0, mut b0, mut a1, mut b1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for [x, y] in ab {
            a0 = a0.min(x);
            b0 = b0.min(y);
            a1 = a1.max(x);
            b1 = b1.max(y);
        }
        let snap = |v: f32| {
            let r = (v * 4096.0).round() / 4096.0;
            if r == 0.0 { 0.0 } else { r }
        };
        Some((
            side,
            FaceSprite {
                sprite: self.sprite(face.layer),
                texture: self.names.get(&face.layer).cloned().unwrap_or_default(),
                tint_index: q.tint_index,
                rect: [a0, b0, a1, b1].map(snap),
                inset: snap((depth - plane).abs()),
                uv: uv.map(snap),
                tint: self.context.tables.tint_source(state, q.tint_index),
                translucent: face.translucent,
            },
        ))
    }
}

/// The affine map from a face's points on its side to its UVs, `[u0, v0, du_u, du_v, dv_u, dv_v]`, from three of
/// its corners that are not in a line (a model's faces are parallelograms on their side).
fn affine(ab: &[[f32; 2]; 4], uv: &[[f32; 2]; 4]) -> Option<[f32; 6]> {
    for [i, j, k] in [[0, 1, 2], [0, 1, 3], [0, 2, 3], [1, 2, 3]] {
        let d1 = [ab[j][0] - ab[i][0], ab[j][1] - ab[i][1]];
        let d2 = [ab[k][0] - ab[i][0], ab[k][1] - ab[i][1]];
        let det = d1[0] * d2[1] - d1[1] * d2[0];
        if det.abs() < 1e-6 {
            continue;
        }
        let e1 = [uv[j][0] - uv[i][0], uv[j][1] - uv[i][1]];
        let e2 = [uv[k][0] - uv[i][0], uv[k][1] - uv[i][1]];
        // e1 = d1.a * du + d1.b * dv, e2 = d2.a * du + d2.b * dv
        let du = [
            (e1[0] * d2[1] - e2[0] * d1[1]) / det,
            (e1[1] * d2[1] - e2[1] * d1[1]) / det,
        ];
        let dv = [
            (e2[0] * d1[0] - e1[0] * d2[0]) / det,
            (e2[1] * d1[0] - e1[1] * d2[0]) / det,
        ];
        let o = [
            uv[i][0] - ab[i][0] * du[0] - ab[i][1] * dv[0],
            uv[i][1] - ab[i][0] * du[1] - ab[i][1] * dv[1],
        ];
        return Some([o[0], o[1], du[0], du[1], dv[0], dv[1]]);
    }
    None
}

/// Each sprite's RGBA8, rows top to bottom, `size` × `size` (every layer of the arrays has the same edge), from
/// `pixels`: the block sprites' layers as `RenderAssets::pixels` holds them, one after another, `split` layers to an
/// array (`Renderer::sprite_split`). An animated sprite's is the frame it starts on. None for a sprite past the
/// layers `pixels` holds, all of them when it holds none (a server loads no pixels).
pub fn sprite_pixels(
    pixels: &[u8],
    size: u32,
    split: u32,
    sprites: &[SpriteRef],
) -> Vec<Option<Vec<u8>>> {
    let bytes = size as usize * size as usize * 4;
    sprites
        .iter()
        .map(|sprite| {
            let layer = sprite
                .array
                .checked_mul(split.max(1))?
                .checked_add(sprite.layer)?;
            let start = (layer as usize).checked_mul(bytes)?;
            (bytes > 0)
                .then(|| pixels.get(start..start.checked_add(bytes)?))
                .flatten()
                .map(<[u8]>::to_vec)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sprites_pixels_are_its_own_layer_of_its_array() {
        // Five 2x2 layers, two to an array: each pixel holds its layer's number.
        let pixels: Vec<u8> = (0..5u8).flat_map(|layer| [layer; 16]).collect();
        let at = |array, layer| SpriteRef {
            array,
            layer,
            animated: false,
            cutout: false,
        };
        let got = sprite_pixels(&pixels, 2, 2, &[at(0, 1), at(2, 0), at(1, 1), at(2, 1)]);
        assert_eq!(got[0], Some(vec![1; 16]));
        assert_eq!(got[1], Some(vec![4; 16]), "array 2 starts at layer 4");
        assert_eq!(got[2], Some(vec![3; 16]));
        assert_eq!(got[3], None, "past the last layer");
        // No renderer: one array holds every layer, and an array past it is none rather than an overflow.
        let single = sprite_pixels(&pixels, 2, u32::MAX, &[at(0, 3), at(1, 0)]);
        assert_eq!(single, [Some(vec![3; 16]), None]);
        // A server's store holds no pixels.
        assert_eq!(sprite_pixels(&[], 16, 2, &[at(0, 0)]), [None]);
    }

    #[test]
    fn a_faces_texture_runs_across_its_side() {
        // a whole face, the texture upright: u along a, v along b
        let ab = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        assert_eq!(
            affine(&ab, &[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]),
            Some([0.0, 0.0, 1.0, 0.0, 0.0, 1.0])
        );
        // turned a quarter (a log on its side): u runs down b, v back along a
        let turned = affine(&ab, &[[0.0, 1.0], [0.0, 0.0], [1.0, 0.0], [1.0, 1.0]]).unwrap();
        assert_eq!(turned, [0.0, 1.0, 0.0, -1.0, 1.0, 0.0]);
        // a slab's side: the lower half of the block, the lower half of the texture (v counts down)
        let half = [[0.0, 0.0], [1.0, 0.0], [1.0, 0.5], [0.0, 0.5]];
        let m = affine(&half, &[[0.0, 1.0], [1.0, 1.0], [1.0, 0.5], [0.0, 0.5]]).unwrap();
        assert_eq!(m, [0.0, 1.0, 1.0, 0.0, 0.0, -1.0]);
        // a face seen edge on has no map
        assert_eq!(affine(&[[0.0, 0.0]; 4], &[[0.0, 0.0]; 4]), None);
    }

    /// The game's own blocks from real prepared resources: a cube, a grass block's tinted top and overlaid sides, a
    /// log on its side (its texture turned), leaves (cut out, foliage), water, a slab and a flower.
    #[test]
    #[ignore = "daily"]
    fn real_blocks_name_their_sprites() -> crate::Result<()> {
        use crate::{
            assets::Assets, biome::Biomes, blocks::BlockFacts, light::LightTables,
            mesher::MeshTables,
        };
        use std::sync::Arc;
        let root = std::path::PathBuf::from(
            std::env::var_os("HORIZON_TEST_RESOURCES").expect("HORIZON_TEST_RESOURCES"),
        );
        let assets = Arc::new(Assets::load(&root)?);
        let render = Arc::new(assets.reload(
            &horizon_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
        )?);
        let facts = BlockFacts::load(&root, assets.registry)?;
        let tables = Arc::new(MeshTables::new(
            &assets,
            &render,
            &facts,
            crate::models::load_visibility(&root, assets.registry)?,
            None,
        )?);
        let context = WorkerContext {
            light: Arc::new(LightTables::load(&root, assets.registry, &facts)?),
            tables,
            assets: assets.clone(),
            render,
            biomes: Arc::new(Biomes::load(
                &horizon_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
            )?),
            hidden_states: Arc::from([]),
            part_tables: Default::default(),
        };
        let names = texture_names(&context.render.sprite_layers);
        let lookup = Lookup::new(&context, 2048, &names);
        let whole = [0.0, 0.0, 1.0, 1.0];
        let upright = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0];

        let stone = lookup.block("minecraft:stone").unwrap();
        assert!(stone.cube);
        let top = &stone.sides[Direction::Up as usize][0];
        assert_eq!(
            (top.rect, top.inset, top.tint, top.sprite.array),
            (whole, 0.0, None, 0)
        );
        // By the name the drawing helpers take, untinted.
        assert_eq!((top.texture.as_str(), top.tint_index), ("block/stone", -1));
        assert_eq!(lookup.texture("minecraft:block/stone"), Some(top.sprite));
        assert_eq!(lookup.texture("block/stone"), Some(top.sprite));
        assert!(!top.sprite.animated && !top.sprite.cutout);

        let grass = lookup.block("grass_block").unwrap();
        assert!(grass.cube);
        assert_eq!(
            grass.sides[Direction::Up as usize][0].tint,
            Some(TintSource::Grass)
        );
        assert_eq!(grass.sides[Direction::Down as usize][0].tint, None);
        let side = &grass.sides[Direction::North as usize];
        assert_eq!(side.len(), 2, "the side and its overlay");
        assert_eq!(
            (side[0].tint, side[1].tint),
            (None, Some(TintSource::Grass))
        );
        assert_eq!(
            lookup.texture("block/grass_block_side_overlay"),
            Some(side[1].sprite)
        );

        // seen from outside, a side's texture runs from its top left: on the north side (looking south, east on
        // the left) u falls with x and v with y; on top, u and v run with x and z (stone's first variant is turned)
        let planks = lookup.block("oak_planks").unwrap();
        let uv = |b: &BlockSprites, d: Direction| b.sides[d as usize][0].uv;
        let north = uv(&planks, Direction::North);
        assert_eq!(north, [1.0, 1.0, -1.0, 0.0, 0.0, -1.0]);
        assert_eq!(uv(&planks, Direction::Up), upright);
        assert_eq!(
            uv(&planks, Direction::South),
            [0.0, 1.0, 1.0, 0.0, 0.0, -1.0]
        );
        assert_eq!(
            uv(&planks, Direction::Down),
            [0.0, 1.0, 1.0, 0.0, 0.0, -1.0]
        );
        // an upright log's side runs as stone's; on its side (axis x) the texture is turned a quarter
        let log = lookup.block("oak_log").unwrap();
        let log_x = lookup.block("minecraft:oak_log[axis=x]").unwrap();
        assert_eq!(log.sides[Direction::North as usize][0].uv, north);
        let turned = log_x.sides[Direction::North as usize][0].uv;
        assert_eq!(
            (turned[2], turned[5]),
            (0.0, 0.0),
            "u runs along y, v along x: {turned:?}"
        );
        assert_eq!(
            log_x.sides[Direction::West as usize][0].sprite,
            log.sides[Direction::Up as usize][0].sprite,
            "its end faces west"
        );

        let leaves = lookup.block("oak_leaves").unwrap();
        let leaf = &leaves.sides[Direction::Up as usize][0];
        assert!(leaf.sprite.cutout);
        assert_eq!(leaf.tint, Some(TintSource::Foliage));
        assert_eq!(
            (leaf.texture.as_str(), leaf.tint_index),
            ("block/oak_leaves", 0)
        );
        assert!(matches!(
            lookup.block("spruce_leaves").unwrap().sides[0][0].tint,
            Some(TintSource::Fixed(0x61_9961))
        ));

        let water = lookup.block("water").unwrap();
        let w = &water.sides[Direction::Up as usize][0];
        assert!(w.sprite.animated && w.translucent);
        assert_eq!(w.tint, Some(TintSource::Water));
        assert_eq!(w.texture, "block/water_still");

        let slab = lookup.block("stone_slab").unwrap();
        assert!(!slab.cube);
        assert_eq!(slab.sides[Direction::Up as usize][0].inset, 0.5);
        assert_eq!(
            slab.sides[Direction::North as usize][0].rect,
            [0.0, 0.0, 1.0, 0.5]
        );

        let flower = lookup.block("dandelion").unwrap();
        assert!(!flower.cube && flower.sides.iter().all(Vec::is_empty));
        assert!(flower.particle.is_some_and(|p| p.cutout));

        assert_eq!(lookup.block("minecraft:no_such_block"), None);
        assert_eq!(lookup.block("stone[no_such=1]"), None);
        assert!(lookup.block("air").unwrap().sides.iter().all(Vec::is_empty));
        Ok(())
    }
}
