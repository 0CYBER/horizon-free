//! Stage the sprites of mod-registered texture atlases (`register_texture_atlases`) into a
//! client resource generation, as GUI textures under each sprite's `sprite_name`.
use super::Game;
use horizon_assets::resource_source::{Resource, ResourceRoot, ResourceSource};
use horizon_client::atlas_sources::{self, SpriteOrigin};
use mod_api::__private::game::*;
use mod_api::{Identifier, TextureAtlasRegistration};
use std::collections::BTreeMap;

/// Atlases one load accepts, across all mods.
const MAX_ATLASES: usize = 32;
/// Sprites of those atlases one load stages, across all mods.
const MAX_SPRITES: usize = 4096;

pub(in crate::game) fn collect_texture_atlases() -> Vec<TextureAtlasRegistration> {
    let mut atlases = Vec::new();
    if mod_api::__private::listened(mod_api::HookId::RegisterTextureAtlases) {
        let _phase =
            mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::Registration);
        Game::native_register_texture_atlases(&mod_api::RegistrationContext::new(), &mut atlases);
    }
    atlases
}

/// The root with every registered atlas's sprites added as GUI textures: a file sprite as the
/// texture itself (with its `.mcmeta`), a generated one (`unstitch`, `paletted_permutations`)
/// encoded once. The runtime block sprites and passive data stay as they were.
pub(in crate::game) fn stage_texture_atlases(
    root: ResourceRoot,
    atlases: &[TextureAtlasRegistration],
) -> ResourceRoot {
    if atlases.is_empty() {
        return root;
    }
    let source = root.source();
    let mut textures = atlas_sources::list_textures(source);
    textures.sort_unstable();
    let mut files = BTreeMap::new();
    let mut seen = Vec::new();
    let mut sprites = 0;
    for registration in atlases {
        let atlas = &registration.atlas;
        if let Err(error) = admit(registration, &seen) {
            log::warn!("Horizon: texture atlas {atlas} skipped: {error}");
            continue;
        }
        let file = definition_file(&registration.definition);
        let Ok(text) = source.read_to_string(&file) else {
            log::warn!("Horizon: texture atlas {atlas} skipped: no definition {file}");
            continue;
        };
        let sources = match atlas_sources::decode(&text) {
            Ok(sources) => sources,
            Err(error) => {
                log::warn!("Horizon: texture atlas {atlas} skipped: {file}: {error}");
                continue;
            }
        };
        seen.push(atlas.clone());
        let mut ask = horizon_client_render::mod_sprite_sources::ask(atlas);
        for (id, origin) in atlas_sources::list(&sources, &textures, &mut ask) {
            if sprites == MAX_SPRITES {
                log::warn!("Horizon: texture atlas {atlas}: sprites past {MAX_SPRITES} skipped");
                break;
            }
            let Ok(sprite) = Identifier::parse(&id) else {
                continue;
            };
            let name = registration.sprite_name(&sprite);
            let path = atlas_sources::texture_resource(&name.to_string(), "textures");
            if source.contains(&path) || files.contains_key(&path) {
                continue;
            }
            if stage_sprite(source, &origin, &path, &mut files) {
                sprites += 1;
            } else {
                log::warn!("Horizon: texture atlas {atlas}: sprite {id} could not be loaded");
            }
        }
    }
    let blocks = root.runtime_block_sprites().to_vec();
    root.with_runtime_textures(files, blocks)
}

fn admit(registration: &TextureAtlasRegistration, seen: &[Identifier]) -> Result<(), &'static str> {
    if matches!(registration.atlas.namespace(), "minecraft" | "horizon") {
        return Err("the game's own atlases can't be registered");
    }
    if seen.contains(&registration.atlas) {
        return Err("the atlas is already registered");
    }
    if seen.len() == MAX_ATLASES {
        return Err("too many atlases");
    }
    Ok(())
}

/// The definition `assets/<namespace>/atlases/<path>.json`. The root already holds the file's
/// whole resource stack (`packs::resources` merges every pack's copy, bottom first, after
/// the pack filters, as `SpriteSourceList.load` does), so this one read sees all of them.
fn definition_file(id: &Identifier) -> String {
    format!("assets/{}/atlases/{}.json", id.namespace(), id.path())
}

fn stage_sprite(
    source: &ResourceSource,
    origin: &SpriteOrigin,
    path: &str,
    files: &mut BTreeMap<String, Resource>,
) -> bool {
    if let SpriteOrigin::File(texture) = origin {
        let file = atlas_sources::texture_resource(texture, "textures");
        if !source.contains(&file) {
            return false;
        }
        let mcmeta = format!("{file}.mcmeta");
        if source.contains(&mcmeta) {
            files.insert(
                format!("{path}.mcmeta"),
                Resource::file(source.clone(), mcmeta),
            );
        }
        files.insert(path.to_owned(), Resource::file(source.clone(), file));
        return true;
    }
    let load = |texture: &str, directory: &str| {
        let file = atlas_sources::texture_resource(texture, directory);
        horizon_client::hud::decode_bytes(&source.read(&file).ok()?).ok()
    };
    let Some(png) = atlas_sources::generate(origin, &load).and_then(encode) else {
        return false;
    };
    files.insert(path.to_owned(), Resource::bytes(png));
    true
}

fn encode((width, height, pixels): atlas_sources::Image) -> Option<Vec<u8>> {
    let mut png = Vec::new();
    let mut encoder = png::Encoder::new(&mut png, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .ok()?
        .write_image_data(&pixels)
        .ok()?;
    Some(png)
}

#[cfg(test)]
mod tests {
    use super::super::fixture_png;
    use super::*;

    fn registration(atlas: &str, definition: &str) -> TextureAtlasRegistration {
        TextureAtlasRegistration {
            atlas: Identifier::parse(atlas).unwrap(),
            definition: Identifier::parse(definition).unwrap(),
        }
    }

    fn root(files: &[(&str, Vec<u8>)]) -> ResourceRoot {
        let files = files
            .iter()
            .map(|(name, bytes)| ((*name).to_owned(), Resource::bytes(bytes.clone())))
            .collect();
        ResourceRoot::new(
            "prepared".into(),
            "cache".into(),
            ResourceSource::indexed(files),
        )
    }

    const ICONS: &str = r#"{"sources":[
        {"type":"directory","source":"icons","prefix":"icons/"},
        {"type":"single","resource":"icons_mod:misc/badge","sprite":"icons_mod:renamed"},
        {"type":"unstitch","resource":"icons_mod:misc/sheet","divisor_x":2,"divisor_y":1,
         "regions":[{"sprite":"icons_mod:left","x":0,"y":0,"width":1,"height":1}]}
    ]}"#;

    #[test]
    fn a_registered_atlas_stages_each_sprite_under_its_gui_name_and_refuses_the_rest() {
        let heart = fixture_png(2, 2, 200);
        let staged = stage_texture_atlases(
            root(&[
                ("assets/icons_mod/atlases/icons.json", ICONS.into()),
                ("assets/icons_mod/textures/icons/heart.png", heart.clone()),
                (
                    "assets/icons_mod/textures/icons/heart.png.mcmeta",
                    b"{}".to_vec(),
                ),
                (
                    "assets/icons_mod/textures/misc/badge.png",
                    fixture_png(3, 3, 9),
                ),
                (
                    "assets/icons_mod/textures/misc/sheet.png",
                    fixture_png(4, 2, 77),
                ),
                ("assets/minecraft/atlases/blocks.json", ICONS.into()),
            ]),
            &[
                registration("icons_mod:icons", "icons_mod:icons"),
                registration("icons_mod:icons", "icons_mod:icons"),
                registration("minecraft:blocks", "minecraft:blocks"),
                registration("icons_mod:missing", "icons_mod:missing"),
            ],
        );
        let source = staged.source();
        let atlas = registration("icons_mod:icons", "icons_mod:icons");
        let file = |sprite: &str| {
            let name = atlas.sprite_name(&Identifier::parse(sprite).unwrap());
            atlas_sources::texture_resource(&name.to_string(), "textures")
        };
        assert_eq!(
            file("icons_mod:icons/heart"),
            "assets/icons_mod/textures/gui/atlases/icons/icons_mod/icons/heart.png"
        );
        assert_eq!(source.read(&file("icons_mod:icons/heart")).unwrap(), heart);
        assert!(source.contains(&format!("{}.mcmeta", file("icons_mod:icons/heart"))));
        let badge = source.read(&file("icons_mod:renamed")).unwrap();
        assert_eq!(horizon_client::hud::png_bytes_size(&badge), Some((3, 3)));
        let left =
            horizon_client::hud::decode_bytes(&source.read(&file("icons_mod:left")).unwrap())
                .unwrap();
        assert_eq!(
            (left.0, left.1, left.2[0]),
            (2, 2, 77),
            "the sheet's left half"
        );
        let staged_names: Vec<_> = source
            .list("")
            .into_iter()
            .filter(|name| name.contains("/gui/atlases/"))
            .collect();
        assert_eq!(staged_names.len(), 4, "{staged_names:?}");
        assert!(
            !source
                .contains("assets/minecraft/textures/gui/atlases/blocks/icons_mod/icons/heart.png"),
            "the game's atlases are refused"
        );
    }

    #[test]
    fn a_reload_lists_the_sprites_from_the_definition_as_the_packs_now_give_it() {
        let registrations = [registration("icons_mod:icons", "icons_mod:icons")];
        let first = stage_texture_atlases(
            root(&[
                ("assets/icons_mod/atlases/icons.json", ICONS.into()),
                (
                    "assets/icons_mod/textures/icons/heart.png",
                    fixture_png(2, 2, 1),
                ),
            ]),
            &registrations,
        );
        let changed = r#"{"sources":[{"type":"directory","source":"other","prefix":""}]}"#;
        let second = stage_texture_atlases(
            root(&[
                ("assets/icons_mod/atlases/icons.json", changed.into()),
                (
                    "assets/icons_mod/textures/icons/heart.png",
                    fixture_png(2, 2, 1),
                ),
                (
                    "assets/icons_mod/textures/other/star.png",
                    fixture_png(2, 2, 5),
                ),
            ]),
            &registrations,
        );
        let heart = "assets/icons_mod/textures/gui/atlases/icons/icons_mod/icons/heart.png";
        let star = "assets/icons_mod/textures/gui/atlases/icons/icons_mod/star.png";
        assert!(first.source().contains(heart) && !first.source().contains(star));
        assert!(second.source().contains(star) && !second.source().contains(heart));
    }

    /// The definition is the packs' resource stack, as `SpriteSourceList.load` reads it: a
    /// mod's file and a pack's file above it both add their sprites, and a pack filter above
    /// them hides the lower files whole.
    #[test]
    fn the_definition_stacks_every_pack_and_a_filter_hides_the_ones_below() {
        use horizon_client::packs::{self, Pack, Source};
        let layer = |files: &[(&str, &[u8])]| {
            ResourceSource::indexed(
                files
                    .iter()
                    .map(|(name, bytes)| ((*name).to_owned(), Resource::bytes(bytes.to_vec())))
                    .collect(),
            )
        };
        let definition = |folder: &str| {
            format!(r#"{{"sources":[{{"type":"directory","source":"{folder}","prefix":""}}]}}"#)
        };
        let heart = fixture_png(2, 2, 1);
        let star = fixture_png(2, 2, 2);
        let moon = fixture_png(2, 2, 3);
        let mod_layer = Pack::mod_layer(
            "icons_mod".into(),
            Source::Opened(layer(&[
                (
                    "assets/icons_mod/atlases/icons.json",
                    definition("hearts").as_bytes(),
                ),
                ("assets/icons_mod/textures/hearts/heart.png", &heart),
            ])),
        );
        let pack = |meta: &str, folder: &str, texture: &str, png: &[u8]| {
            packs::pack_for(
                format!("file/{folder}"),
                horizon_client::text::Text::literal(folder),
                Source::Opened(layer(&[
                    ("pack.mcmeta", meta.as_bytes()),
                    (
                        "assets/icons_mod/atlases/icons.json",
                        definition(folder).as_bytes(),
                    ),
                    (texture, png),
                ])),
                packs::RESOURCE_FORMAT,
            )
            .unwrap()
        };
        let format = packs::RESOURCE_FORMAT.0;
        let plain = format!(r#"{{"pack":{{"pack_format":{format},"description":""}}}}"#);
        let filtering = format!(
            r#"{{"pack":{{"pack_format":{format},"description":""}},
                "filter":{{"block":[{{"namespace":"icons_mod","path":"atlases/icons\\.json"}}]}}}}"#
        );
        let stars = pack(
            &plain,
            "stars",
            "assets/icons_mod/textures/stars/star.png",
            &star,
        );
        let moons = pack(
            &filtering,
            "moons",
            "assets/icons_mod/textures/moons/moon.png",
            &moon,
        );
        let staged = |selected: &[Pack]| {
            let source = packs::resources(layer(&[]), selected).unwrap();
            let root = ResourceRoot::new("prepared".into(), "cache".into(), source);
            stage_texture_atlases(root, &[registration("icons_mod:icons", "icons_mod:icons")])
        };
        let sprite = |name: &str| {
            format!("assets/icons_mod/textures/gui/atlases/icons/icons_mod/{name}.png")
        };
        let both = staged(&[mod_layer.clone(), stars.clone()]);
        assert_eq!(both.source().read(&sprite("heart")).unwrap(), heart);
        assert_eq!(both.source().read(&sprite("star")).unwrap(), star);
        let filtered = staged(&[mod_layer, stars, moons]);
        assert!(!filtered.source().contains(&sprite("heart")));
        assert!(!filtered.source().contains(&sprite("star")));
        assert_eq!(filtered.source().read(&sprite("moon")).unwrap(), moon);
    }

    /// A mod registering an atlas on the first load only: its id and vanilla's
    /// `map_decorations` definition, already in the resources.
    struct Maps(std::sync::Arc<std::sync::Mutex<(u32, Vec<Option<mod_api::AtlasRegion>>)>>);

    fn maps() -> TextureAtlasRegistration {
        registration("maps_mod:maps", "minecraft:map_decorations")
    }

    /// An atlas whose name holds `presets`, which the menu's own picture folder also has.
    fn presets() -> TextureAtlasRegistration {
        registration("maps_mod:presets", "minecraft:map_decorations")
    }

    fn player(atlas: &TextureAtlasRegistration) -> Identifier {
        atlas.sprite_name(&Identifier::parse("minecraft:player").unwrap())
    }

    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Maps {
        fn register_texture_atlases(
            &self,
            _: &mod_api::RegistrationContext,
            event: &mut mod_api::RegisterTextureAtlasesEvent<'_>,
        ) {
            let mut kept = self.0.lock().unwrap();
            kept.0 += 1;
            if kept.0 == 1 {
                event.atlases.push(maps());
                event.atlases.push(presets());
            }
        }

        fn texture_atlas_stitched(
            &self,
            host: &mut dyn mod_api::ClientHost,
            event: &mut mod_api::TextureAtlasStitchedEvent,
        ) {
            if event.atlas == mod_api::TextureAtlas::Gui {
                self.0
                    .lock()
                    .unwrap()
                    .1
                    .push(host.gui_sprite(&player(&maps())));
            }
        }
    }

    /// The registered atlas's sprite is in the GUI atlas at its own size after the first load,
    /// drawable by its name, and gone after a reload that no longer registers it.
    #[test]
    #[ignore = "daily"]
    fn a_mod_atlas_sprite_is_stitched_into_the_gui_atlas_until_a_reload_drops_it() {
        let kept = std::sync::Arc::default();
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "maps_mod",
            Box::new(Maps(std::sync::Arc::clone(&kept))),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            crate::game::native_entity_refs::tests::with_client_game(|game| {
                game.texture_atlases_stitched();
                let player = player(&maps());
                assert_eq!(
                    game.gui_sprite_pixels(&player.to_string()),
                    Some([8.0, 8.0])
                );
                assert_eq!(
                    game.gui_sprite_pixels(&self::player(&presets()).to_string()),
                    Some([8.0, 8.0]),
                    "a sprite under an atlas named presets is stitched too"
                );
                {
                    let kept = kept.lock().unwrap();
                    assert_eq!(kept.0, 1);
                    assert_eq!(
                        kept.1.last().copied().flatten().map(|r| r.size),
                        Some([8, 8])
                    );
                }
                game.spawn_pack_reload(crate::game::packs::ReloadKind::Selection)
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
                assert_eq!(kept.0, 2);
                assert_eq!(
                    kept.1.last().copied(),
                    Some(None),
                    "looked up in the new atlas"
                );
                assert_eq!(game.gui_sprite_pixels(&player.to_string()), None);
            });
        });
    }

    #[test]
    fn no_registration_leaves_the_root_as_it_was() {
        let base = root(&[("assets/icons_mod/atlases/icons.json", ICONS.into())]);
        let staged = stage_texture_atlases(base.clone(), &[]);
        assert!(staged.source().same_generation(base.source()));
    }

    /// A mod's sprite source, answering its entries in `icons_mod:icons`: a 2x2 sprite of the
    /// entry's `shade`, and the removal of every listed star.
    struct Checker;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Checker {
        fn register_sprite_sources(
            &self,
            _: &mod_api::RegistrationContext,
            event: &mut mod_api::RegisterSpriteSourcesEvent<'_>,
        ) {
            event.sources.push("checker_mod:checker".into());
        }
        fn sprite_source(
            &self,
            _: &mod_api::RegistrationContext,
            event: &mut mod_api::SpriteSourceEvent<'_>,
        ) {
            if event.source != "checker_mod:checker" || event.atlas.path() != "icons" {
                return;
            }
            let shade = match event.config.get("shade") {
                Some(horizon_core::nbt::Tag::Byte(shade)) => *shade as u8,
                _ => 0,
            };
            event.output.push(mod_api::SpriteSourceOutput::Pixels {
                sprite: Identifier::parse("icons_mod:checker").unwrap(),
                width: 2,
                height: 2,
                rgba: [shade, shade, shade, 255].repeat(4),
            });
            for sprite in event.listed.iter().filter(|id| id.path().ends_with("star")) {
                event.output.push(mod_api::SpriteSourceOutput::Remove {
                    sprite: sprite.clone(),
                });
            }
        }
    }

    /// A registered source type lists the mod's sprites where its entry stands, a reload asks
    /// again with the definition as the packs now give it, and with no mod answering the entry
    /// lists nothing.
    #[test]
    fn a_mods_sprite_source_lists_its_sprites_and_answers_again_on_reload() {
        let stage = |shade: u8| {
            let definition = format!(
                r#"{{"sources":[
                    {{"type":"directory","source":"icons","prefix":"icons/"}},
                    {{"type":"checker_mod:checker","shade":{shade}}}
                ]}}"#
            );
            stage_texture_atlases(
                root(&[
                    (
                        "assets/icons_mod/atlases/icons.json",
                        definition.into_bytes(),
                    ),
                    (
                        "assets/icons_mod/textures/icons/heart.png",
                        fixture_png(2, 2, 1),
                    ),
                    (
                        "assets/icons_mod/textures/icons/star.png",
                        fixture_png(2, 2, 2),
                    ),
                ]),
                &[registration("icons_mod:icons", "icons_mod:icons")],
            )
        };
        let gui = "assets/icons_mod/textures/gui/atlases/icons/icons_mod";
        let (checker, heart, star) = (
            format!("{gui}/checker.png"),
            format!("{gui}/icons/heart.png"),
            format!("{gui}/icons/star.png"),
        );
        let checker_pixels = |staged: &ResourceRoot| {
            horizon_client::hud::decode_bytes(&staged.source().read(&checker).unwrap()).unwrap()
        };
        let mods: mod_api::Mods =
            vec![mod_api::ModEntry::new("checker_mod", Box::new(Checker))].into();
        mod_platform::testing::with_mods(mods, || {
            let mut types = Vec::new();
            Game::native_register_sprite_sources(&mod_api::RegistrationContext::new(), &mut types);
            assert_eq!(
                horizon_client_render::mod_sprite_sources::register(types),
                ["checker_mod:checker"]
            );
            let first = stage(40);
            assert_eq!(checker_pixels(&first), (2, 2, [40, 40, 40, 255].repeat(4)));
            assert!(first.source().contains(&heart));
            assert!(
                !first.source().contains(&star),
                "the source removed the star"
            );
            let reloaded = stage(90);
            assert_eq!(checker_pixels(&reloaded).2[..4], [90, 90, 90, 255]);
        });
        mod_platform::testing::with_mods(Vec::new().into(), || {
            let unanswered = stage(40);
            assert!(!unanswered.source().contains(&checker));
            assert!(unanswered.source().contains(&star) && unanswered.source().contains(&heart));
        });
    }
}
