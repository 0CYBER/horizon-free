//! Materialize generated sprites into a staged client resource generation.
use super::Game;
use horizon_assets::resource_source::{Resource, ResourceRoot, ResourceSource, stays_inside};
use mod_api::__private::game::*;
use mod_api::{RuntimeSprite, TextureAtlas};
use std::collections::BTreeMap;

pub(in crate::game) fn collect_runtime_sprites() -> Vec<RuntimeSprite> {
    let mut sprites = Vec::new();
    if mod_api::__private::listened(mod_api::HookId::RegisterRuntimeSprites) {
        let _phase =
            mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::Registration);
        Game::native_register_runtime_sprites(&mod_api::RegistrationContext::new(), &mut sprites);
    }
    sprites
}

pub(in crate::game) fn stage_runtime_sprites(
    root: ResourceRoot,
    sprites: &[RuntimeSprite],
) -> ResourceRoot {
    let mut files = BTreeMap::new();
    let mut blocks = Vec::new();
    let mut pixels = 0;
    for sprite in sprites {
        match validate(root.source(), &files, sprite, pixels) {
            Ok((path, area)) => {
                pixels += area;
                files.insert(path, Resource::bytes(sprite.png.clone()));
                if sprite.atlas == TextureAtlas::Blocks {
                    blocks.push(sprite.id.to_string());
                }
            }
            Err(error) => log::warn!("Horizon: runtime sprite {} skipped: {error}", sprite.id),
        }
    }
    root.with_runtime_textures(files, blocks)
}

fn validate(
    source: &ResourceSource,
    files: &BTreeMap<String, Resource>,
    sprite: &RuntimeSprite,
    pixels: u32,
) -> Result<(String, u32), &'static str> {
    let prefix = match sprite.atlas {
        TextureAtlas::Blocks => "block/",
        TextureAtlas::Gui => "gui/",
        _ => return Err("unsupported atlas"),
    };
    let namespace = sprite.id.namespace();
    let name = sprite.id.path();
    if matches!(namespace, "minecraft" | "horizon")
        || !name.starts_with(prefix)
        || !stays_inside(name)
    {
        return Err("expected a mod namespace and a path for the target atlas");
    }
    if sprite.atlas == TextureAtlas::Gui
        && (name.contains("title/background") || name.contains("presets"))
    {
        return Err("path is excluded from the GUI atlas");
    }
    let path = format!("assets/{namespace}/textures/{name}.png");
    if files.contains_key(&path)
        || source.contains(&path)
        || source.contains(&format!("{path}.mcmeta"))
    {
        return Err("the texture already exists");
    }
    if files.len() >= 128 || sprite.png.len() > 256 * 1024 {
        return Err("sprite count or encoded size exceeds the limit");
    }
    let (width, height) = horizon_client::hud::png_bytes_size(&sprite.png).ok_or("expected PNG")?;
    if !(1..=256).contains(&width) || !(1..=256).contains(&height) {
        return Err("image must be between 1 and 256 pixels on each axis");
    }
    let area = width * height;
    if pixels + area > 1_000_000 {
        return Err("runtime sprite pixel budget exhausted");
    }
    horizon_client::hud::decode_bytes(&sprite.png).map_err(|_| "malformed PNG")?;
    Ok((path, area))
}

#[cfg(test)]
pub(in crate::game) fn fixture_png(width: u32, height: u32, channel: u8) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&vec![channel; (width * height * 4) as usize])
        .unwrap();
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::Identifier;

    #[test]
    fn valid_generated_assets_layer_without_replacing_pack_files_or_passive_data() {
        let pack_path = "assets/marker/textures/gui/from_pack.png";
        let base = ResourceSource::indexed(BTreeMap::from([(
            pack_path.into(),
            Resource::bytes(vec![42]),
        )]));
        let data = ResourceSource::default();
        let root = ResourceRoot::new("prepared".into(), "cache".into(), base.clone())
            .with_data(data.clone(), ResourceSource::default());
        let sprite = |atlas, id: &str, png| RuntimeSprite {
            atlas,
            id: Identifier::parse(id).unwrap(),
            png,
        };
        let valid = fixture_png(2, 2, 64);
        let staged = stage_runtime_sprites(
            root,
            &[
                sprite(TextureAtlas::Blocks, "marker:block/target", valid.clone()),
                sprite(
                    TextureAtlas::Blocks,
                    "marker:block/target",
                    fixture_png(2, 2, 32),
                ),
                sprite(TextureAtlas::Gui, "marker:gui/button", valid.clone()),
                sprite(TextureAtlas::Gui, "marker:gui/from_pack", valid.clone()),
                sprite(TextureAtlas::Gui, "marker:gui/bad", vec![0; 32]),
                sprite(TextureAtlas::Gui, "minecraft:gui/refused", valid.clone()),
                sprite(
                    TextureAtlas::Gui,
                    "marker:gui/large",
                    fixture_png(257, 1, 64),
                ),
            ],
        );
        assert_eq!(staged.runtime_block_sprites(), ["marker:block/target"]);
        assert_eq!(staged.source().read(pack_path).unwrap(), [42]);
        assert_eq!(
            staged
                .source()
                .read("assets/marker/textures/block/target.png")
                .unwrap(),
            valid
        );
        assert!(
            staged
                .source()
                .contains("assets/marker/textures/gui/button.png")
        );
        for rejected in [
            "marker/textures/gui/bad",
            "minecraft/textures/gui/refused",
            "marker/textures/gui/large",
        ] {
            assert!(!staged.source().contains(&format!("assets/{rejected}.png")));
        }
        assert!(staged.data().same_generation(&data));
        assert!(!base.contains("assets/marker/textures/gui/button.png"));
    }
}
