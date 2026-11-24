//! Where blocks' textures are in the block sprite arrays, for a client mod drawing blocks with the player's own
//! textures (resource packs included) in its own pass: with `WorldDrawn::block_sprites`, the arrays themselves.
use super::Game;
use horizon_client::block_sprites::{BlockSprites, Lookup, SpriteRef};
use horizon_world::registry::BlockStateId;
use mod_api::{AtlasRegion, EntityRef, Identifier, PlayerSkin};

/// The most pixel bytes one `sprite_pixels` call returns: the default texture budget
/// (`sprites::DEFAULT_TEXTURE_BUDGET`), so with it every layer of the arrays fits in one call.
pub(super) const MAX_SPRITE_PIXEL_BYTES: u64 = 256 << 20;

#[allow(dead_code)]
impl Game<'_> {
    /// For a client mod: what each block (`minecraft:grass_block`, `oak_log[axis=x]`; properties left out take the
    /// block's default) draws on each of its six sides, as the mesher bakes its model: each face's sprite (array and
    /// layer, as `WorldDrawn::block_sprites` cuts them), where it lies on the side, how its texture runs across it,
    /// its tint source and whether it is cut out, translucent or animated; its particle sprite; whether it is a plain
    /// cube. None for a block the game does not know. The model is the state's first random variant. API
    /// 380.
    pub(super) fn block_sprites(
        &self,
        blocks: &[horizon_world::registry::BlockStateId],
    ) -> Vec<Option<BlockSprites>> {
        let lookup = self.native_sprite_lookup();
        blocks
            .iter()
            .map(|state| {
                self.pack.context.assets.registry.state(*state)?;
                Some(lookup.state(*state))
            })
            .collect()
    }
    /// For a client mod: each texture's sprite by name (`minecraft:block/stone`, `block/stone`), none for one no
    /// model uses. API 380.
    pub(super) fn texture_sprites(&self, textures: &[&str]) -> Vec<Option<SpriteRef>> {
        let lookup = self.native_sprite_lookup();
        textures
            .iter()
            .map(|texture| lookup.texture(texture))
            .collect()
    }
    /// For a client mod: each sprite's RGBA8 as the resources loaded it, square at the arrays' edge; none for a
    /// sprite past the layers or a client that loaded no pixels. `Err`, reading nothing, for more than
    /// `MAX_SPRITE_PIXEL_BYTES` in one call.
    pub(super) fn sprite_pixels(
        &self,
        sprites: &[SpriteRef],
    ) -> Result<Vec<Option<Vec<u8>>>, String> {
        let size = u64::from(self.pack.context.render.sprite_size);
        let asked = (sprites.len() as u64).saturating_mul(size * size * 4);
        if asked > MAX_SPRITE_PIXEL_BYTES {
            return Err(format!(
                "{} sprites of {size} px are {asked} bytes, more than {MAX_SPRITE_PIXEL_BYTES} a call",
                sprites.len()
            ));
        }
        Ok(self.native_sprite_lookup().pixels(sprites))
    }
    /// For a client mod drawing a block's faces itself (a copycat drawing its material's, from
    /// `block_sprites`): the 0xRRGGBB colour a face of `state` with tint index `tint_index` is
    /// multiplied by at `pos`, as the chunk mesher tints it there (`MeshTables::block_tints`:
    /// `BlockColors`, biome blend included). None for an untinted face. A fluid's face
    /// (`block_sprites::fluid_faces`: water at tint index 0) takes the water colour the fluid
    /// mesher gives it (`MeshTables::water_tint`, the packs' water colormap included). This client's copy of the
    /// world: the chunks around `pos` it has, as break particles read them (`particle_sprite`).
    pub(super) fn block_tint_color(
        &self,
        state: BlockStateId,
        tint_index: i32,
        pos: [i32; 3],
    ) -> Option<u32> {
        let (cx, cz) = (pos[0] >> 4, pos[2] >> 4);
        let region: [[Option<&horizon_client::chunk::Chunk>; 3]; 3] = std::array::from_fn(|dz| {
            std::array::from_fn(|dx| {
                self.world
                    .chunks
                    .get(&(cx + dx as i32 - 1, cz + dz as i32 - 1))
                    .map(|c| &**c)
            })
        });
        let block = self
            .pack
            .context
            .assets
            .registry
            .state(state)
            .map_or("", |s| s.name.as_str());
        if let Some((_, tint, _)) = horizon_client::block_sprites::fluid_faces(block) {
            return (tint.is_some() && tint_index == 0).then(|| {
                self.pack.context.tables.water_tint(
                    pos,
                    &region,
                    world_biomes!(self),
                    self.streaming.biome_zoom,
                    self.streaming.biome_blend,
                )
            });
        }
        let tints = self.pack.context.tables.block_tints(
            state,
            pos,
            &region,
            world_biomes!(self),
            self.streaming.biome_zoom,
            self.streaming.biome_blend,
            self.streaming.nether_shading,
        );
        tints(tint_index)
    }
    /// For a client mod: changes each time the resources load (a resource pack turned on or off can move every
    /// sprite): ask `block_sprites` again then. API 380.
    pub(super) fn block_sprites_version(&self) -> u64 {
        self.pack.context.render.generation
    }
    /// For a client mod: where GUI sprite `sprite` lies in the GUI atlas and its size, none for one the atlas lacks
    /// (a server's metrics-only atlas lacks them all). The atlas names vanilla's sprites by path alone
    /// (`gui/sprites/container/slot`) and a mod's with its namespace (`mymod:gui/panel`).
    pub(super) fn gui_sprite(&self, sprite: &Identifier) -> Option<AtlasRegion> {
        let r = if sprite.namespace() == "minecraft" {
            self.gui.atlas.sprites.get(sprite.path())
        } else {
            self.gui.atlas.sprites.get(&sprite.to_string())
        }?;
        Some(AtlasRegion::new([r.u0, r.v0, r.u1, r.v1], [r.w, r.h]))
    }
    /// For a client mod: the player's or mannequin's skin and cape in the GUI atlas and its model,
    /// as this client draws it (`Mob::skin`, `slim`, `cape`, set each frame by `update_remote_skins`).
    pub(super) fn player_skin(&self, entity: EntityRef) -> Option<PlayerSkin> {
        use super::commands::Target;
        let mob = match self.native_client_entity_target(entity)? {
            Target::Player => self.player_model.as_ref()?,
            Target::Mob(id) => self.mobs.iter().find(|m| m.id == id && !m.removed)?,
            _ => return None,
        };
        skin_regions(
            &self.gui.atlas.sprites,
            mob.skin.as_deref()?,
            mob.slim,
            mob.cape.as_deref(),
        )
    }
    fn native_sprite_lookup(&self) -> Lookup<'_> {
        // (no renderer, no arrays: every layer counts as array 0's)
        let split = self
            .renderer
            .as_ref()
            .map_or(u32::MAX, |r| r.sprite_split());
        Lookup::new(&self.pack.context, split, &self.pack.layer_names)
    }
}

#[cfg(test)]
mod texture_atlas_stitched_tests {
    use mod_api::{
        AtlasRegion, ClientHost, Hooks, Identifier, ModEntry, RegisterRuntimeSpritesEvent,
        RegistrationContext, RuntimeSprite, SpriteRef, TextureAtlas, TextureAtlasStitchedEvent,
    };
    use mod_platform::testing::with_mods;
    use std::sync::{Arc, Mutex};

    /// What the mod kept: each atlas it heard, and one sprite of each, looked up once there.
    #[derive(Default)]
    struct Kept {
        atlases: Vec<TextureAtlas>,
        stone: Option<SpriteRef>,
        slot: Option<AtlasRegion>,
        registrations: u32,
        runtime_block: Option<SpriteRef>,
        runtime_gui: Option<AtlasRegion>,
    }
    struct Keeper(Arc<Mutex<Kept>>);
    #[mod_api::__subscriptions]
    impl Hooks for Keeper {
        fn register_runtime_sprites(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterRuntimeSpritesEvent<'_>,
        ) {
            use crate::game::client_reload::fixture_png;
            let mut kept = self.0.lock().unwrap();
            kept.registrations += 1;
            if kept.registrations == 1 {
                event.sprites.push(RuntimeSprite {
                    atlas: TextureAtlas::Blocks,
                    id: id("keeper:block/marker"),
                    png: fixture_png(2, 2, 32),
                });
            }
            event.sprites.push(RuntimeSprite {
                atlas: TextureAtlas::Gui,
                id: id("keeper:gui/marker"),
                png: fixture_png(kept.registrations + 2, 3, 64),
            });
        }

        fn texture_atlas_stitched(
            &self,
            host: &mut dyn ClientHost,
            event: &mut TextureAtlasStitchedEvent,
        ) {
            let mut kept = self.0.lock().unwrap();
            kept.atlases.push(event.atlas);
            match event.atlas {
                TextureAtlas::Blocks => {
                    kept.stone = host.texture_sprites(&["minecraft:block/stone"])[0];
                    kept.runtime_block = host.texture_sprites(&["keeper:block/marker"])[0];
                }
                TextureAtlas::Gui => {
                    kept.slot = host.gui_sprite(&id("gui/sprites/container/slot"));
                    kept.runtime_gui = host.gui_sprite(&id("keeper:gui/marker"));
                }
                _ => {}
            }
        }
    }

    fn id(text: &str) -> Identifier {
        Identifier::parse(text).unwrap()
    }

    /// `TextureAtlasStitchedEvent`: the mod hears the blocks' atlas and then the GUI's, finds a
    /// sprite of each, and hears both again after a resource reload.
    #[test]
    #[ignore = "daily"]
    fn a_client_mod_finds_its_sprites_each_time_an_atlas_is_stitched() {
        let kept = Arc::new(Mutex::new(Kept::default()));
        let mods: mod_api::Mods =
            vec![ModEntry::new("keeper", Box::new(Keeper(Arc::clone(&kept))))].into();
        with_mods(mods, || {
            super::super::native_entity_refs::tests::with_client_game(|game| {
                // The first load, as `run` fires it before `client_setup`.
                game.texture_atlases_stitched();
                {
                    let kept = kept.lock().unwrap();
                    assert_eq!(kept.atlases, [TextureAtlas::Blocks, TextureAtlas::Gui]);
                    assert!(kept.stone.is_some(), "stone is in the block arrays");
                    assert!(
                        kept.runtime_block.is_some(),
                        "no model references this block sprite"
                    );
                    assert_eq!(kept.runtime_gui.map(|sprite| sprite.size), Some([3, 3]));
                    let slot = kept.slot.expect("the slot sprite is in the GUI atlas");
                    assert_eq!(slot.size, [18, 18], "the slot's own pixels");
                    let [u0, v0, u1, v1] = slot.uv;
                    assert!(0.0 <= u0 && u0 < u1 && u1 <= 1.0, "{:?}", slot.uv);
                    assert!(0.0 <= v0 && v0 < v1 && v1 <= 1.0, "{:?}", slot.uv);
                }
                // A mod's namespace is part of the name; one the atlas lacks is none.
                assert_eq!(
                    game.gui_sprite(&id("mymod:gui/sprites/container/slot")),
                    None
                );
                assert_eq!(game.gui_sprite(&id("gui/no_such_sprite")), None);
                // F3+T: the reload stages on a worker and applies here.
                kept.lock().unwrap().atlases.clear();
                game.spawn_pack_reload(super::super::packs::ReloadKind::Selection)
                    .unwrap();
                let started = std::time::Instant::now();
                while game.pack_reload.is_some() {
                    assert!(
                        started.elapsed().as_secs() < 300,
                        "the reload never finished"
                    );
                    std::thread::sleep(std::time::Duration::from_millis(20));
                    game.drive_pack_reload();
                }
                let kept = kept.lock().unwrap();
                assert_eq!(kept.atlases, [TextureAtlas::Blocks, TextureAtlas::Gui]);
                assert_eq!(kept.registrations, 2);
                assert!(
                    kept.runtime_block.is_none(),
                    "omitted sprite leaves the new atlas"
                );
                assert_eq!(kept.runtime_gui.map(|sprite| sprite.size), Some([4, 3]));
                assert_eq!(
                    kept.slot.map(|s| s.size),
                    Some([18, 18]),
                    "looked up again in the new atlas"
                );
            });
        });
    }
}

/// `FaceSprite::quad_on`, the copycat side of `block_sprites`.
#[cfg(test)]
mod quad_on_tests {
    use horizon_core::geometry::Direction;

    fn face(uv: [f32; 6]) -> mod_api::FaceSprite {
        mod_api::FaceSprite {
            sprite: mod_api::SpriteRef {
                array: 0,
                layer: 0,
                animated: false,
                cutout: false,
            },
            texture: "block/oak_log".into(),
            rect: [0.0, 0.0, 1.0, 1.0],
            inset: 0.0,
            uv,
            tint: None,
            tint_index: -1,
            translucent: false,
        }
    }

    /// The point (a, b) of `side`, in its two axes, as `FaceSprite::uv` counts them.
    fn side_point(side: Direction, p: [f32; 3]) -> [f32; 2] {
        match side {
            Direction::Down | Direction::Up => [p[0], p[2]],
            Direction::North | Direction::South => [p[0], p[1]],
            Direction::West | Direction::East => [p[2], p[1]],
        }
    }

    /// Whatever the face's turn or mirror, the UV `quad` gives each corner (the rectangle's
    /// (u0, v0), (u0, v1), (u1, v1), (u1, v0) in turn) is the one the material's own side shows
    /// at that point, and the corners wind counterclockwise seen from outside.
    #[test]
    fn a_copycat_face_shows_its_material_turned_as_the_model_turns_it() {
        let upright = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0];
        // u = b, v = 1 - a: a quarter turn (a log on its side).
        let turned = [0.0, 1.0, 0.0, -1.0, 1.0, 0.0];
        // u = 1 - a, v = b: mirrored.
        let mirrored = [1.0, 0.0, -1.0, 0.0, 0.0, 1.0];
        let boxes = [([0.0; 3], [1.0; 3]), ([0.0, 0.0, 0.0], [1.0, 0.5, 1.0])];
        for uv in [upright, turned, mirrored] {
            let face = face(uv);
            for side in Direction::ALL {
                for (from, to) in boxes {
                    let (corners, [u0, v0, u1, v1]) = face
                        .quad_on(side, from, to)
                        .expect("a turn of 90 degrees or a mirror fits a rectangle");
                    let given = [[u0, v0], [u0, v1], [u1, v1], [u1, v0]];
                    for (corner, given) in corners.iter().zip(given) {
                        let [a, b] = side_point(side, *corner);
                        let want = [
                            (uv[0] + a * uv[2] + b * uv[4]) * 16.0,
                            (uv[1] + a * uv[3] + b * uv[5]) * 16.0,
                        ];
                        assert!(
                            (want[0] - given[0]).abs() < 1e-4 && (want[1] - given[1]).abs() < 1e-4,
                            "{side:?} {uv:?} {from:?}..{to:?}: {corner:?} shows {given:?}, not {want:?}"
                        );
                    }
                    let [p0, p1, p2, _] = corners;
                    let e = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
                    let f = [p2[0] - p0[0], p2[1] - p0[1], p2[2] - p0[2]];
                    let n = side.normal().map(|v| v as f32);
                    let out = (e[1] * f[2] - e[2] * f[1]) * n[0]
                        + (e[2] * f[0] - e[0] * f[2]) * n[1]
                        + (e[0] * f[1] - e[1] * f[0]) * n[2];
                    assert!(out > 0.0, "{side:?} {uv:?}: the face looks inward");
                }
            }
        }
        // The half box's north face is cut at half height: its corners lie on z = 0, y up to 0.5.
        let (corners, _) = face(upright)
            .quad_on(Direction::North, [0.0; 3], [1.0, 0.5, 1.0])
            .unwrap();
        assert!(corners.iter().all(|c| c[2] == 0.0 && c[1] <= 0.5));
        // A texture turned 22.5 degrees on the side, or sheared, fits no UV rectangle: no answer
        // rather than wrong UVs.
        let (s, c) = 22.5f32.to_radians().sin_cos();
        let slanted = [
            0.5 - 0.5 * c + 0.5 * s,
            0.5 - 0.5 * s - 0.5 * c,
            c,
            s,
            -s,
            c,
        ];
        let sheared = [0.0, 0.0, 1.0, 0.0, 1.0, 1.0];
        for uv in [slanted, sheared] {
            assert_eq!(
                face(uv).quad_on(Direction::North, [0.0; 3], [1.0; 3]),
                None,
                "{uv:?}"
            );
        }
    }
}

/// `sprite_pixels`, a texture's pixels as a client mod reads them on the CPU.
#[cfg(test)]
mod sprite_pixels_tests {
    /// A mod reads a texture's pixels through the host: stone's are opaque and not one flat colour, and an oak
    /// leaf's have holes; a call past the byte cap reads nothing.
    #[test]
    #[ignore = "daily"]
    fn a_client_mod_reads_a_textures_pixels() {
        super::super::native_entity_refs::tests::with_client_game(|game| {
            let sprites: Vec<_> = game
                .texture_sprites(&["block/stone", "minecraft:block/oak_leaves"])
                .into_iter()
                .map(|s| s.expect("a vanilla texture"))
                .collect();
            let edge = game.pack.context.render.sprite_size as usize;
            let got = game.sprite_pixels(&sprites).unwrap();
            let [Some(stone), Some(leaves)] = &got[..] else {
                panic!("both textures have pixels: {got:?}");
            };
            assert_eq!(stone.len(), edge * edge * 4);
            assert_eq!(leaves.len(), edge * edge * 4);
            assert!(
                stone.as_chunks::<4>().0.iter().all(|p| p[3] == 255),
                "stone is opaque"
            );
            assert!(
                stone.as_chunks::<4>().0.iter().any(|p| p[..3] != stone[..3]),
                "stone is not one flat colour"
            );
            assert!(
                leaves.as_chunks::<4>().0.iter().any(|p| p[3] == 0),
                "leaves have holes"
            );
            let past = horizon_client::block_sprites::SpriteRef {
                array: 7,
                layer: u32::MAX - 1,
                animated: false,
                cutout: false,
            };
            assert_eq!(game.sprite_pixels(&[past]).unwrap(), [None]);
            let per = (edge * edge * 4) as u64;
            let many = vec![sprites[0]; (super::MAX_SPRITE_PIXEL_BYTES / per + 1) as usize];
            assert!(game.sprite_pixels(&many).is_err());
        });
    }
}

/// `block_tint_color`, the colour a copycat multiplies its material's tinted faces by.
#[cfg(test)]
mod block_tint_color_tests {
    /// A grass block's tinted face takes a colour where it stands, and so does water's face
    /// (tint index 0, as `block_sprites` reports it); stone has no tint, and an untinted
    /// face (-1) none either.
    #[test]
    #[ignore = "daily"]
    fn a_tinted_face_takes_its_colour_where_it_stands() {
        super::super::native_entity_refs::tests::with_client_game(|game| {
            let registry = game.pack.context.assets.registry;
            let grass = registry.default_state("minecraft:grass_block").unwrap();
            let stone = registry.default_state("minecraft:stone").unwrap();
            let water = registry.default_state("minecraft:water").unwrap();
            use horizon_assets::resource_source::{ResourceRoot, ResourceSource};
            use std::sync::Arc;
            static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../.local")
                .join(format!(
                    "tint-fixture-{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                ));
            let write = |path: &str, bytes: &[u8]| {
                let path = root.join(path);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, bytes).unwrap();
            };
            let png = |rgb: Vec<u8>| {
                let mut bytes = Vec::new();
                let mut encoder = png::Encoder::new(&mut bytes, 256, 256);
                encoder.set_color(png::ColorType::Rgb);
                encoder.set_depth(png::BitDepth::Eight);
                let mut writer = encoder.write_header().unwrap();
                writer.write_image_data(&rgb).unwrap();
                writer.finish().unwrap();
                bytes
            };
            // Synthetic pixels only. Vanilla clamps t and r, then samples at
            // x = trunc(255 * (1 - t)), y = trunc(255 * (1 - r*t)).
            // (1,1) -> (0,0); (0.5,0.5) -> (127,191), index 49023.
            // Neither biome has a grass override/modifier. Uniform chunks make
            // the BiomeColors blend equal to that one pixel, independent of zoom.
            let mut pixels = vec![0; 256 * 256 * 3];
            pixels[..3].copy_from_slice(&[0x12, 0x34, 0x56]);
            pixels[49023 * 3..49023 * 3 + 3].copy_from_slice(&[0xAB, 0xCD, 0xEF]);
            let image = png(pixels);
            for name in ["grass", "foliage", "dry_foliage"] {
                write(
                    &format!("assets/minecraft/textures/colormap/{name}.png"),
                    &image,
                );
            }
            write(
                "data/minecraft/worldgen/biome/plains.json",
                br#"{"temperature":1.0,"downfall":1.0}"#,
            );
            write(
                "data/minecraft/worldgen/biome/cool.json",
                br#"{"temperature":0.5,"downfall":0.5}"#,
            );
            // A separate, constant water map: every sample and every blend is
            // 0x2468AC, never either grass pixel or a biome's default water colour.
            write(
                "assets/test/polytone/colormaps/water.png",
                &png([0x24, 0x68, 0xAC].repeat(256 * 256)),
            );
            write(
                "assets/test/polytone/colors.json",
                br#"{"biome_colors":{"water":"test:water"}}"#,
            );
            let biomes = Arc::new(
                horizon_client::biome::Biomes::load(&ResourceRoot::directory(root.clone()))
                    .unwrap(),
            );
            let source = ResourceSource::directory(root.clone());
            let old = &game.pack.context;
            let tables = horizon_client::mesher::MeshTables::new(
                &old.assets,
                &old.render,
                game.res.facts(),
                horizon_client::models::load_visibility(&game.options.resources, registry).unwrap(),
                None,
            )
            .unwrap()
            .with_polytone_colormaps(&source, registry);
            game.pack.context = Arc::new(horizon_client::streaming::WorkerContext {
                assets: old.assets.clone(),
                render: old.render.clone(),
                tables: Arc::new(tables),
                biomes: biomes.clone(),
                light: old.light.clone(),
                hidden_states: old.hidden_states.clone(),
                part_tables: old.part_tables.clone(),
            });
            game.world_biomes = Some(biomes.clone());
            game.streaming.biome_blend = 0;
            game.world.chunks.clear();
            for (cx, name) in [(0, "minecraft:plains"), (4, "minecraft:cool")] {
                for dz in -1..=1 {
                    for dx in -1..=1 {
                        let mut chunk = horizon_client::chunk::Chunk::empty(cx + dx, dz);
                        for y in 0..96 {
                            for z in 0..4 {
                                for x in 0..4 {
                                    chunk.set_biome_cell(x, y, z, biomes.id(name));
                                }
                            }
                        }
                        game.world.chunks.insert((cx + dx, dz), Arc::new(chunk));
                    }
                }
            }
            assert_eq!(game.block_tint_color(grass, 0, [8, 64, 8]), Some(0x123456));
            assert_eq!(game.block_tint_color(grass, 0, [72, 64, 8]), Some(0xABCDEF));
            for pos in [[8, 64, 8], [72, 64, 8]] {
                assert_eq!(game.block_tint_color(water, 0, pos), Some(0x2468AC));
                assert_eq!(game.block_tint_color(grass, -1, pos), None);
                let untinted = game.block_tint_color(stone, 0, pos);
                assert_eq!(untinted, None);
                assert_eq!(untinted.unwrap_or(0xFFFFFF), 0xFFFFFF);
                assert_eq!(game.block_tint_color(water, 1, pos), None);
            }
            std::fs::remove_dir_all(root).unwrap();
        });
    }
}

/// A skin and cape by their atlas names, as [`PlayerSkin`]: none when the skin is not in the atlas
/// (its slot taken since it was set), and no cape when the cape's is not.
fn skin_regions(
    sprites: &std::collections::HashMap<String, horizon_client::hud::Rect>,
    skin: &str,
    slim: bool,
    cape: Option<&str>,
) -> Option<PlayerSkin> {
    let region = |name: &str| {
        let r = sprites.get(name)?;
        Some(AtlasRegion::new([r.u0, r.v0, r.u1, r.v1], [r.w, r.h]))
    };
    Some(PlayerSkin::new(region(skin)?, slim, cape.and_then(region)))
}

#[cfg(test)]
mod player_skin_tests {
    use super::*;
    use horizon_client::hud::Rect;

    fn rect(u0: f32, v0: f32) -> Rect {
        Rect {
            u0,
            v0,
            u1: u0 + 0.25,
            v1: v0 + 0.25,
            w: 64,
            h: 64,
        }
    }

    #[test]
    fn a_skin_reads_its_and_its_capes_atlas_regions() {
        let sprites = std::collections::HashMap::from([
            ("skins/abc".to_owned(), rect(0.5, 0.0)),
            ("capes/def".to_owned(), rect(0.0, 0.5)),
        ]);
        let skin = skin_regions(&sprites, "skins/abc", true, Some("capes/def")).unwrap();
        assert_eq!(
            skin.skin,
            AtlasRegion::new([0.5, 0.0, 0.75, 0.25], [64, 64])
        );
        assert!(skin.slim);
        assert_eq!(
            skin.cape,
            Some(AtlasRegion::new([0.0, 0.5, 0.25, 0.75], [64, 64]))
        );
        // A cape whose slot was taken is left out; a skin whose slot was taken answers none.
        assert_eq!(
            skin_regions(&sprites, "skins/abc", false, Some("capes/x"))
                .unwrap()
                .cape,
            None
        );
        assert_eq!(skin_regions(&sprites, "skins/gone", false, None), None);
    }
}
