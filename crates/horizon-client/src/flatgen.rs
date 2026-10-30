//! FlatLevelSource: superflat worlds (`minecraft:flat` generator).
use crate::{
    chunk::{Chunk, LevelShape},
    streaming::ChunkGenerator,
};
use horizon_world::{nbt::Tag, registry::BlockRegistry};

/// The layers of a flat generator, bottom up from the world's minimum Y.
pub struct FlatTerrain {
    /// The Overworld's shape: the layers start at its minimum Y and stop at
    /// its top.
    shape: LevelShape,
    layers: Vec<u32>,
    biome: u16,
    surface: Option<crate::lod::Surface>,
}
impl FlatTerrain {
    /// The overworld generator of `world_gen_settings` (`data.dimensions`), if it
    /// is flat. Its `features` and `lakes` decorate the world; a world without
    /// them (the default) gets only its layers, as FlatLevelSource places them.
    pub fn from_settings(
        settings: &Tag,
        registry: &BlockRegistry,
        biomes: &crate::biome::Biomes,
        shape: LevelShape,
    ) -> Option<Self> {
        let generator = settings
            .get("dimensions")?
            .get("minecraft:overworld")?
            .get("generator")?;
        if crate::save::text(generator.get("type"))?.trim_start_matches("minecraft:") != "flat" {
            return None;
        }
        let s = generator.get("settings")?;
        let biome = crate::save::text(s.get("biome")).unwrap_or_else(|| "minecraft:plains".into());
        let mut layers = Vec::new();
        if let Some(Tag::List(list)) = s.get("layers") {
            for layer in list {
                let block = crate::save::text(layer.get("block"))?;
                let height = crate::save::number(layer.get("height")).unwrap_or(0).max(0);
                let state = registry.default_state(&block)?.0;
                layers.extend(std::iter::repeat_n(state, height as usize));
            }
        }
        let biome = biomes.id(&biome);
        let mut water = None;
        let mut surface = None;
        for (i, &id) in layers.iter().take(shape.height as usize).enumerate().rev() {
            let state = horizon_world::registry::BlockStateId(id);
            let name = registry.state(state)?.name.as_str();
            if matches!(
                name,
                "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
            ) {
                continue;
            }
            let height = shape.min_y + i as i32 + 1;
            if matches!(name, "minecraft:water" | "minecraft:lava") {
                water.get_or_insert((height, state));
            } else {
                surface = Some(crate::lod::Surface {
                    height,
                    state,
                    water,
                    biome,
                });
                break;
            }
        }
        Some(Self {
            shape,
            layers,
            biome,
            surface,
        })
    }
    /// FlatLevelSource.getSpawnHeight: the top of the layers.
    pub fn spawn_height(&self) -> i32 {
        self.shape.min_y + self.layers.len().min(self.shape.height as usize) as i32
    }
}
/// DimensionType.Y_SIZE: the most layers FlatLevelGeneratorSettings takes.
const Y_SIZE: i64 = 4064;

/// FlatLevelGeneratorSettings.CODEC reading a dedicated server's
/// `generator-settings` (DedicatedServerProperties.WorldDimensionData for
/// the `minecraft:flat` preset): the settings as `world_gen_settings` stores
/// them, and the errors vanilla logs, in order (the constructor's, then
/// `resultOrPartial`'s). No settings means the preset's own layers; a layer
/// that doesn't parse is left out, as the codec's partial result leaves it.
/// `is_block` says whether a block id is registered, `is_biome` a biome id.
pub fn settings_from_json(
    json: &crate::server::json::Json,
    is_block: impl Fn(&str) -> bool,
    is_biome: impl Fn(&str) -> bool,
) -> (Option<Tag>, Vec<String>) {
    use crate::server::json::Json;
    let id = |s: &str| {
        if s.contains(':') {
            s.to_owned()
        } else {
            format!("minecraft:{s}")
        }
    };
    let Some(Json::Array(list)) = json.get("layers") else {
        let error = match json.get("layers") {
            None => format!("No key layers in MapLike[{}]", json.compact()),
            Some(v) => format!("Not a list: {}", v.compact()),
        };
        return (None, vec![error]);
    };
    let mut errors = Vec::new();
    let mut layers = Vec::new();
    let mut total = 0;
    for layer in list {
        let height = layer.get("height").and_then(Json::as_string);
        let height = height.as_deref().and_then(|h| h.parse::<i64>().ok());
        let block = layer.get("block").and_then(Json::as_string).map(|b| id(&b));
        match (height, block) {
            (Some(h), _) if !(0..=Y_SIZE).contains(&h) => {
                errors.push(format!("Value {h} outside of range [0:{Y_SIZE}]"));
            }
            (Some(h), Some(b)) if is_block(&b) => {
                total += h;
                layers.push(Tag::compound([
                    ("height", Tag::Int(h as i32)),
                    ("block", Tag::String(b.as_str().into())),
                ]));
            }
            (Some(_), Some(b)) => errors.push(format!(
                "Unknown registry key in ResourceKey[minecraft:root / minecraft:block]: {b}"
            )),
            (None, _) => errors.push(format!("No key height in MapLike[{}]", layer.compact())),
            (_, None) => errors.push(format!("No key block in MapLike[{}]", layer.compact())),
        }
    }
    // The constructor: Biome.CODEC is lenient, and a biome that is missing
    // or unknown is Plains.
    let biome = json
        .get("biome")
        .and_then(Json::as_string)
        .map(|b| id(&b))
        .filter(|b| is_biome(b));
    let mut logged = Vec::new();
    if biome.is_none() {
        logged.push("Unknown biome, defaulting to plains".to_owned());
    }
    let biome = biome.unwrap_or_else(|| "minecraft:plains".to_owned());
    // validateHeight.
    if total > Y_SIZE {
        errors.push(format!("Sum of layer heights is > {Y_SIZE}"));
        logged.push(errors.join("; "));
        return (None, logged);
    }
    let flag = |key: &str| match json.get(key) {
        Some(Json::Bool(b)) => *b,
        Some(Json::Number(n)) => n.parse::<f64>().is_ok_and(|v| v != 0.0),
        _ => false,
    };
    let mut settings = Vec::new();
    // RegistryCodecs.holderSet, lenient too: a tag or a list of ids.
    match json.get("structure_overrides") {
        Some(Json::String(s)) => {
            settings.push(("structure_overrides", Tag::String(s.as_str().into())))
        }
        Some(Json::Array(ids)) if ids.iter().all(|i| matches!(i, Json::String(_))) => settings
            .push((
                "structure_overrides",
                Tag::List(
                    ids.iter()
                        .filter_map(Json::as_string)
                        .map(|i| Tag::String(id(&i).as_str().into()))
                        .collect(),
                ),
            )),
        _ => {}
    }
    settings.extend([
        ("layers", Tag::List(layers)),
        ("lakes", Tag::Byte(i8::from(flag("lakes")))),
        ("features", Tag::Byte(i8::from(flag("features")))),
        ("biome", Tag::String(biome.as_str().into())),
    ]);
    if !errors.is_empty() {
        logged.push(errors.join("; "));
    }
    (Some(Tag::compound(settings)), logged)
}
impl ChunkGenerator for FlatTerrain {
    type Worker = ();
    fn worker(&self) {}
    fn lod_surface(&self, _: &mut (), _: i32, _: i32) -> Option<crate::lod::Surface> {
        self.surface
    }
    fn proto(&self, _: &mut (), cx: i32, cz: i32) -> crate::streaming::Proto {
        let mut chunk = Chunk::with_layout(cx, cz, self.shape.column());
        for s in &mut chunk.sections {
            std::sync::Arc::make_mut(s).biomes = horizon_core::paletted::PalettedContainer::new(
                crate::chunk::BIOME_STRATEGY,
                u32::from(self.biome),
            );
        }
        let height = self.shape.height as usize;
        for (i, &block) in self.layers.iter().enumerate().take(height) {
            if block == 0 {
                continue;
            }
            for z in 0..16 {
                for x in 0..16 {
                    chunk.set(x, self.shape.min_y + i as i32, z, block);
                }
            }
        }
        for s in &mut chunk.sections {
            std::sync::Arc::make_mut(s).compact();
        }
        Box::new(chunk)
    }
    /// FlatLevelSource.spawnOriginalMobs is empty: no generation-time spawns.
    fn finish(&self, proto: crate::streaming::Proto) -> Chunk {
        *proto.downcast::<Chunk>().expect("flat terrain proto")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::save::{number, text};
    use crate::server::json::{self, Json};

    fn read(text: &str) -> (Option<Tag>, Vec<String>) {
        let json: Json = json::parse(text, false).unwrap();
        settings_from_json(
            &json,
            |b| ["minecraft:stone", "minecraft:bedrock"].contains(&b),
            |b| ["minecraft:plains", "minecraft:desert"].contains(&b),
        )
    }

    #[test]
    fn generator_settings_read_as_flat_level_generator_settings() {
        // server.properties' default: the preset's own layers, and the codec's error.
        assert_eq!(
            read("{}"),
            (None, vec!["No key layers in MapLike[{}]".into()])
        );
        let (settings, errors) = read(
            r#"{"layers":[{"block":"bedrock","height":1},{"block":"minecraft:stone","height":3}],"biome":"desert","features":true}"#,
        );
        assert!(errors.is_empty());
        let settings = settings.unwrap();
        let Some(Tag::List(layers)) = settings.get("layers") else {
            panic!("{settings:?}")
        };
        assert_eq!(layers.len(), 2);
        assert_eq!(
            text(layers[0].get("block")).as_deref(),
            Some("minecraft:bedrock")
        );
        assert_eq!(number(layers[1].get("height")), Some(3));
        assert_eq!(
            text(settings.get("biome")).as_deref(),
            Some("minecraft:desert")
        );
        assert_eq!(number(settings.get("features")), Some(1));
        assert_eq!(number(settings.get("lakes")), Some(0));
        // A layer that doesn't parse is left out; an unknown biome is Plains, and logged first.
        let (settings, errors) = read(
            r#"{"layers":[{"block":"nope","height":2},{"block":"stone","height":2}],"biome":"x"}"#,
        );
        assert_eq!(errors.len(), 2);
        assert_eq!(errors[0], "Unknown biome, defaulting to plains");
        assert!(errors[1].contains("minecraft:nope"));
        let settings = settings.unwrap();
        let Some(Tag::List(layers)) = settings.get("layers") else {
            panic!("{settings:?}")
        };
        assert_eq!(layers.len(), 1);
        assert_eq!(
            text(settings.get("biome")).as_deref(),
            Some("minecraft:plains")
        );
        // validateHeight.
        assert_eq!(
            read(
                r#"{"layers":[{"block":"stone","height":4000},{"block":"stone","height":100}],"biome":"plains"}"#
            ),
            (None, vec!["Sum of layer heights is > 4064".into()])
        );
    }
}
