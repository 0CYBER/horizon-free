/// Identifiers may contain repeated separators, but never escape the dimension root.
fn valid_dimension_key(id: &str) -> bool {
    let Some((namespace, path)) = id.split_once(':') else {
        return false;
    };
    horizon_core::level_id::valid_level_namespace(namespace)
        && !path.is_empty()
        && !path.starts_with('/')
        && path
            .split('/')
            .filter(|part| !part.is_empty())
            .all(horizon_core::level_id::valid_level_namespace)
}

/// Structural boundary checks; generator-specific codecs run on world loading.
pub(crate) fn valid_world_gen_dimensions(dimensions: &Tag) -> bool {
    use horizon_core::identifier::Identifier;
    let Tag::Compound(stems) = dimensions else {
        return false;
    };
    dimensions.get("minecraft:overworld").is_some()
        && stems.iter().all(|(id, stem)| {
            let Some(generator) = stem.get("generator") else {
                return false;
            };
            let Some(kind) = text(generator.get("type")).and_then(|id| Identifier::parse(&id).ok())
            else {
                return false;
            };
            valid_dimension_key(&id.to_string_lossy())
                && match stem.get("type") {
                    Some(Tag::String(id)) => id
                        .to_string()
                        .ok()
                        .is_some_and(|id| Identifier::parse(&id).is_ok()),
                    Some(Tag::Compound(_)) => true,
                    _ => false,
                }
                && (kind.namespace() != "minecraft"
                    || kind.path() != "flat"
                    || generator
                        .get("settings")
                        .is_some_and(crate::flatgen::valid_layer_heights))
        })
}

/// [`write_world_gen_settings`] with a flat overworld's own generator
/// settings (FlatLevelGeneratorSettings, as `flatgen::settings_from_json`
/// reads them) instead of the Superflat preset's classic layers, and with the
/// biome the single-biome preset fixes (`CreateBuffetWorldScreen`'s choice,
/// `minecraft:plains` when none was made).
/// `dimensions` replaces the preset's stems with a validated editor/data-pack result.
pub fn write_world_gen_settings_with(
    dir: &Path,
    seed: i64,
    options: WorldGen,
    flat_settings: Option<Tag>,
    single_biome: Option<&str>,
    dimensions: Option<Tag>,
) -> Result<()> {
    let mut settings = new_world_gen_settings(seed, options, flat_settings, single_biome);
    if let Some(dimensions) = dimensions {
        if !valid_world_gen_dimensions(&dimensions) {
            return Err("invalid world preset dimensions".into());
        }
        let Tag::Compound(settings) = &mut settings else {
            unreachable!()
        };
        settings.insert("dimensions".into(), dimensions);
    }
    fs::create_dir_all(dir)?;
    write_saved_data(dir, "world_gen_settings", settings)
}
/// Build the vanilla preset's creation settings without touching disk.
pub(crate) fn new_world_gen_settings(
    seed: i64,
    options: WorldGen,
    flat_settings: Option<Tag>,
    single_biome: Option<&str>,
) -> Tag {
    // worldgen/world_preset/flat.json's overworld: the classic flat layers.
    let layer = |block: &str, height: i32| {
        Tag::compound([("block", string(block)), ("height", Tag::Int(height))])
    };
    let classic = || {
        Tag::compound([
            ("biome", string("minecraft:plains")),
            ("features", Tag::Byte(0)),
            ("lakes", Tag::Byte(0)),
            (
                "layers",
                Tag::List(vec![
                    layer("minecraft:bedrock", 1),
                    layer("minecraft:dirt", 2),
                    layer("minecraft:grass_block", 1),
                ]),
            ),
            (
                "structure_overrides",
                Tag::List(vec![
                    string("minecraft:strongholds"),
                    string("minecraft:villages"),
                ]),
            ),
        ])
    };
    let flat = Tag::compound([
        ("type", string("minecraft:overworld")),
        (
            "generator",
            Tag::compound([
                ("type", string("minecraft:flat")),
                ("settings", flat_settings.unwrap_or_else(classic)),
            ]),
        ),
    ]);
    let dimension = |kind: &str, settings: &str, source: Tag| {
        Tag::compound([
            ("type", string(kind)),
            (
                "generator",
                Tag::compound([
                    ("type", string("minecraft:noise")),
                    ("settings", string(settings)),
                    ("biome_source", source),
                ]),
            ),
        ])
    };
    let preset = |p: &str| {
        Tag::compound([
            ("type", string("minecraft:multi_noise")),
            ("preset", string(p)),
        ])
    };
    // The overworld generator of worldgen/world_preset/<preset>.json: the
    // noise settings and the biome source each preset uses.
    let overworld = match options.preset {
        WorldPreset::Flat => flat,
        WorldPreset::LargeBiomes => dimension(
            "minecraft:overworld",
            "minecraft:large_biomes",
            preset("minecraft:overworld"),
        ),
        WorldPreset::Amplified => dimension(
            "minecraft:overworld",
            "minecraft:amplified",
            preset("minecraft:overworld"),
        ),
        WorldPreset::SingleBiomeSurface => dimension(
            "minecraft:overworld",
            "minecraft:overworld",
            Tag::compound([
                ("type", string("minecraft:fixed")),
                ("biome", string(single_biome.unwrap_or("minecraft:plains"))),
            ]),
        ),
        WorldPreset::Normal => dimension(
            "minecraft:overworld",
            "minecraft:overworld",
            preset("minecraft:overworld"),
        ),
        // worldgen/world_preset/debug_all_block_states.json's overworld.
        WorldPreset::Debug => Tag::compound([
            ("type", string("minecraft:overworld")),
            (
                "generator",
                Tag::compound([("type", string("minecraft:debug"))]),
            ),
        ]),
    };
    Tag::compound([
        ("seed", Tag::Long(seed)),
        (
            "generate_structures",
            Tag::Byte(i8::from(options.structures)),
        ),
        ("bonus_chest", Tag::Byte(i8::from(options.bonus_chest))),
        (
            "dimensions",
            Tag::compound([
                ("minecraft:overworld", overworld),
                (
                    "minecraft:the_nether",
                    dimension(
                        "minecraft:the_nether",
                        "minecraft:nether",
                        preset("minecraft:nether"),
                    ),
                ),
                (
                    "minecraft:the_end",
                    dimension(
                        "minecraft:the_end",
                        "minecraft:end",
                        Tag::compound([("type", string("minecraft:the_end"))]),
                    ),
                ),
            ]),
        ),
    ])
}

#[cfg(test)]
mod dimension_key_tests {
    #[test]
    fn normalized_dimension_keys_stay_inside_the_root() {
        for (id, expected) in [
            ("example:foo//bar", true),
            ("example:../x", false),
            ("example:a/../../b", false),
            (":path", false),
            ("example:", false),
            ("example:/absolute", false),
            ("example:a/./b", false),
        ] {
            assert_eq!(super::valid_dimension_key(id), expected, "{id}");
        }
    }
}
