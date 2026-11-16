//! Game-owned dimension codec and registered pack-entry conversion.
use horizon_world::worldgen::overrides::{LevelHeight, ModDimension};
use mod_api::{BiomeSourceKind, DimensionSettings};
use serde_json::{Value, json};

/// The dimension type's fields these settings set.
fn dimension_type(settings: &DimensionSettings) -> Value {
    let mut json = json!({
        "min_y": settings.min_y,
        "height": settings.height,
        "logical_height": settings.logical_height,
        "has_skylight": settings.has_skylight,
        "has_ceiling": settings.has_ceiling,
        "has_fixed_time": settings.has_fixed_time,
        "has_ender_dragon_fight": settings.has_ender_dragon_fight,
        "coordinate_scale": settings.coordinate_scale,
        "ambient_light": settings.ambient_light,
        "infiniburn": settings.infiniburn,
        "monster_spawn_light_level": {
            "type": "minecraft:uniform",
            "min_inclusive": settings.monster_spawn_light_level.0,
            "max_inclusive": settings.monster_spawn_light_level.1,
        },
        "monster_spawn_block_light_limit": settings.monster_spawn_block_light_limit,
        "skybox": settings.skybox,
        "cardinal_light": settings.cardinal_light,
        "attributes": {
            "minecraft:gameplay/water_evaporates": settings.water_evaporates,
            "minecraft:gameplay/fast_lava": settings.fast_lava,
            "minecraft:gameplay/respawn_anchor_works": settings.respawn_anchor_works,
            "minecraft:gameplay/nether_portal_spawns_piglin": settings.nether_portal_spawns_piglin,
            "minecraft:gameplay/piglins_zombify": settings.piglins_zombify,
            "minecraft:gameplay/snow_golem_melts": settings.snow_golem_melts,
            "minecraft:gameplay/can_start_raid": settings.can_start_raid,
            "minecraft:gameplay/sky_light_level": settings.sky_light_level,
            "minecraft:gameplay/bed_rule": {
                "can_sleep": settings.bed_can_sleep,
                "can_set_spawn": settings.bed_can_set_spawn,
                "destroy_on_use": settings.bed_destroy_on_use,
                "destroy_on_leave": settings.bed_destroy_on_leave,
            },
            "minecraft:gameplay/straw_bed_rule": {
                "can_sleep": settings.straw_bed_can_sleep,
                "can_set_spawn": settings.straw_bed_can_set_spawn,
                "destroy_on_use": settings.straw_bed_destroy_on_use,
                "destroy_on_leave": settings.straw_bed_destroy_on_leave,
            },
        },
    });
    let attributes = json["attributes"]
        .as_object_mut()
        .expect("attributes is an object");
    if let Some(color) = &settings.sky_color {
        attributes.insert("minecraft:visual/sky_color".into(), json!(color));
    }
    if let Some(color) = &settings.fog_color {
        attributes.insert("minecraft:visual/fog_color".into(), json!(color));
    }
    if let Some(height) = settings.cloud_height {
        attributes.insert("minecraft:visual/cloud_height".into(), json!(height));
    }
    let (timelines, clock) = match settings.day_cycle.as_str() {
        "nether" => (json!("#minecraft:in_nether"), None),
        "end" => (json!("#minecraft:in_end"), Some("minecraft:the_end")),
        "none" => (json!([]), None),
        _ => (
            json!("#minecraft:in_overworld"),
            Some("minecraft:overworld"),
        ),
    };
    json["timelines"] = timelines;
    if let Some(clock) = clock {
        json["default_clock"] = json!(clock);
    }
    json
}
/// The level stem's `biome_source`.
fn biome_source(settings: &DimensionSettings) -> Value {
    if let Some(source) = &settings.mod_biome_source {
        return json!({"type": source});
    }
    match settings.biome_source {
        BiomeSourceKind::Fixed => json!({"type": "minecraft:fixed", "biome": settings.biome}),
        BiomeSourceKind::Checkerboard => json!({
            "type": "minecraft:checkerboard",
            "biomes": settings.biomes,
            "scale": settings.biome_scale,
        }),
        BiomeSourceKind::MultiNoise => {
            json!({"type": "minecraft:multi_noise", "preset": settings.biome_preset})
        }
        BiomeSourceKind::End => json!({"type": "minecraft:the_end"}),
        BiomeSourceKind::Points => {
            let interval = |(min, max): (f32, f32)| json!([min, max]);
            let biomes: Vec<Value> = settings
                .biome_points
                .iter()
                .map(|point| {
                    json!({
                        "biome": point.biome,
                        "parameters": {
                            "temperature": interval(point.temperature),
                            "humidity": interval(point.humidity),
                            "continentalness": interval(point.continentalness),
                            "erosion": interval(point.erosion),
                            "depth": interval(point.depth),
                            "weirdness": interval(point.weirdness),
                            "offset": point.offset,
                        },
                    })
                })
                .collect();
            json!({"type": "minecraft:multi_noise", "biomes": biomes})
        }
        _ => unreachable!("unknown API biome source"),
    }
}

/// The actual engine dimension-type codec, called before registration is installed.
pub(in crate::game) fn valid_shape(settings: &DimensionSettings) -> bool {
    LevelHeight::of_json(&dimension_type(settings)).is_some()
}

pub(in crate::game) fn installed_dimensions() -> Vec<ModDimension> {
    mod_api::__private::dimension_declarations()
        .into_iter()
        .map(|(id, settings)| {
            let stem = json!({
                "type": id,
                "generator": {
                    "type": settings.generator,
                    "settings": settings.noise_settings,
                    "biome_source": biome_source(&settings),
                },
            });
            ModDimension {
                id,
                stem: stem.to_string(),
                dimension_type: dimension_type(&settings).to_string(),
                sea_level: settings.sea_level,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #[cfg(test)]
    use super::*;
    #[cfg(test)]
    use crate::game::api_values::install_parts;
    #[cfg(test)]
    use mod_api::*;

    /// A name that is no safe folder of the world refuses the mods at startup, before any world
    /// opens: a level's folder is its id under the world's `dimensions` folder.
    #[test]
    fn a_name_that_escapes_the_worlds_folder_refuses_registration() {
        use mod_api::DimensionSettings;
        const BAD: [&str; 10] = [
            "../minecraft/overworld",
            "a/../../b",
            "a/../b",
            "/abs",
            "trailing/",
            "a//b",
            ".",
            "..",
            "Pocket",
            "my pocket",
        ];
        struct Invalid;
        #[mod_api::__subscriptions]
        impl Hooks for Invalid {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                for name in BAD {
                    r.register_dimension(name, DimensionSettings::default());
                }
            }
        }
        let errors = install_parts(&[ModEntry::new("test", Box::new(Invalid))]).unwrap_err();
        assert_eq!(errors.len(), BAD.len(), "{errors:?}");
        for (error, name) in errors.iter().zip(BAD) {
            assert!(error.contains(&format!("test:{name}\"")), "{name}: {error}");
        }
    }

    /// Nested names with the characters ids allow are fine.
    #[test]
    fn a_nested_name_registers() {
        use mod_api::DimensionSettings;
        struct Nested;
        #[mod_api::__subscriptions]
        impl Hooks for Nested {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                r.register_dimension("a/b", DimensionSettings::default());
                r.register_dimension("test:deep/pocket-1.x", DimensionSettings::default());
            }
        }
        install_parts(&[ModEntry::new("test", Box::new(Nested))]).unwrap();
        let ids: Vec<String> = installed_dimensions().into_iter().map(|d| d.id).collect();
        assert_eq!(ids, ["test:a/b", "test:deep/pocket-1.x"]);
    }

    /// Each dimension a mod may not register: every one is refused at
    /// startup, naming the mod.
    #[test]
    fn invalid_dimensions_refuse_registration() {
        use mod_api::DimensionSettings;
        struct Invalid;
        #[mod_api::__subscriptions]
        impl Hooks for Invalid {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                r.register_dimension("pocket", DimensionSettings::default());
                r.register_dimension("pocket", DimensionSettings::default());
                r.register_dimension("other:pocket", DimensionSettings::default());
                r.register_dimension("", DimensionSettings::default());
                r.register_dimension(
                    "uneven",
                    DimensionSettings {
                        height: 40,
                        logical_height: 40,
                        ..Default::default()
                    },
                );
                r.register_dimension(
                    "low_ceiling",
                    DimensionSettings {
                        height: 64,
                        ..Default::default()
                    },
                );
                r.register_dimension(
                    "too_tall",
                    DimensionSettings {
                        min_y: 2016,
                        height: i32::MAX - 15,
                        logical_height: 0,
                        ..Default::default()
                    },
                );
                r.register_dimension(
                    "custom",
                    DimensionSettings {
                        generator: "test:void".into(),
                        ..Default::default()
                    },
                );
            }
        }
        let errors = install_parts(&[ModEntry::new("test", Box::new(Invalid))]).unwrap_err();
        assert_eq!(errors.len(), 7, "{errors:?}");
        assert!(errors[0].contains("test registered dimension test:pocket twice"));
        assert!(errors[1].contains("namespace"));
        assert!(errors[2].contains("test declares invalid identifier \"test:\""));
        assert!(errors[3].contains("test:uneven"));
        assert!(errors[4].contains("test:low_ceiling"));
        assert!(errors[5].contains("test:too_tall"));
        assert!(errors[6].contains("test:custom with the unknown generator \"test:void\""));
    }

    #[cfg(test)]
    struct Void;
    #[cfg(test)]
    impl mod_api::ChunkGenerator for Void {
        fn build_terrain(&self, _: &mut dyn mod_api::ChunkAccess) {}
    }

    /// A generator's name is held to the same rule, so `Void` or `../x` refuses the mods too.
    #[test]
    fn a_generator_name_that_is_no_folder_refuses_registration() {
        const BAD: [&str; 6] = ["../x", "a//b", "/abs", "Void", "my void", "."];
        struct Invalid;
        #[mod_api::__subscriptions]
        impl Hooks for Invalid {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                for name in BAD {
                    r.register_chunk_generator(name, Void);
                }
                r.register_chunk_generator("gen/void_2", Void);
            }
        }
        let errors = install_parts(&[ModEntry::new("test", Box::new(Invalid))]).unwrap_err();
        assert_eq!(errors.len(), BAD.len(), "{errors:?}");
        for (error, name) in errors.iter().zip(BAD) {
            assert!(error.contains(&format!("test:{name}\"")), "{name}: {error}");
        }
    }

    /// Each generator a mod may not register: every one is refused at startup,
    /// naming the mod.
    #[test]
    fn invalid_chunk_generators_refuse_registration() {
        struct Invalid;
        #[mod_api::__subscriptions]
        impl Hooks for Invalid {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                r.register_chunk_generator("void", Void);
                r.register_chunk_generator("void", Void);
                r.register_chunk_generator("other:void", Void);
                r.register_chunk_generator("", Void);
            }
        }
        let errors = install_parts(&[ModEntry::new("test", Box::new(Invalid))]).unwrap_err();
        assert_eq!(errors.len(), 3, "{errors:?}");
        assert!(errors[0].contains("test registered chunk generator test:void twice"));
        assert!(errors[1].contains("test declares \"other:void\" in unowned namespace \"other\""));
        assert!(errors[2].contains("test declares invalid identifier \"test:\""));
    }

    /// A mod with vanilla's id can't take over every noise level.
    #[test]
    fn the_noise_generator_refuses_registration() {
        struct Noise;
        #[mod_api::__subscriptions]
        impl Hooks for Noise {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                r.register_chunk_generator("noise", Void);
            }
        }
        let errors = install_parts(&[ModEntry::new("minecraft", Box::new(Noise))]).unwrap_err();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("minecraft:noise, which is the game's own"));
    }

    /// The generator check runs after every mod registered.
    #[test]
    fn a_later_mods_generator_satisfies_an_earlier_mods_dimension() {
        struct Dimension;
        #[mod_api::__subscriptions]
        impl Hooks for Dimension {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                r.register_dimension(
                    "pocket",
                    mod_api::DimensionSettings {
                        generator: "later:void".into(),
                        ..Default::default()
                    },
                );
            }
        }
        struct Generator;
        #[mod_api::__subscriptions]
        impl Hooks for Generator {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                r.register_chunk_generator("void", Void);
            }
        }
        install_parts(&[
            ModEntry::new("earlier", Box::new(Dimension)),
            ModEntry::new("later", Box::new(Generator)),
        ])
        .unwrap();
        let dimensions = installed_dimensions();
        let stem: serde_json::Value = serde_json::from_str(&dimensions[0].stem).unwrap();
        assert_eq!(stem["generator"]["type"], "later:void");
        let generators = crate::game::native_chunk_generator::installed_chunk_generators();
        assert!(generators.contains_key("later:void"));
    }

    /// What the engine calls is the mod's own code, under its namespaced id.
    #[test]
    fn a_registered_generator_reaches_the_engine() {
        use mod_api::{ChunkAccess, ChunkPos};
        // The engine's chunk: what the worker threads hand the generator.
        use horizon_world::registry::BlockStateId;
        use horizon_world::worldgen::mod_generators::ChunkAccess as EngineChunk;
        struct Marker(mod_api::BlockKey);
        impl mod_api::ChunkGenerator for Marker {
            fn build_terrain(&self, chunk: &mut dyn ChunkAccess) {
                assert_eq!(chunk.level(), "test:pocket");
                let state = chunk.state(self.0);
                assert_eq!(chunk.metadata().name(state.block()), Ok("minecraft:stone"));
                assert!(chunk.set_block([1, chunk.min_y(), 2], state));
                let invalid =
                    mod_api::BlockState::from(mod_api::BlockStateRecord::new(self.0, u16::MAX));
                assert!(!chunk.set_block([2, chunk.min_y(), 2], invalid));
            }
        }
        struct Writes;
        #[mod_api::__subscriptions]
        impl Hooks for Writes {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                let block = r.block_key("minecraft:stone");
                r.register_chunk_generator("test:marker", Marker(block));
            }
        }
        struct Written(
            Vec<([i32; 3], BlockStateId)>,
            &'static horizon_world::registry::BlockRegistry,
        );
        impl EngineChunk for Written {
            fn registry(&self) -> &'static horizon_world::registry::BlockRegistry {
                self.1
            }
            fn pos(&self) -> ChunkPos {
                ChunkPos::new(0, 0)
            }
            fn level(&self) -> &str {
                "test:pocket"
            }
            fn seed(&self) -> i64 {
                0
            }
            fn min_y(&self) -> i32 {
                -64
            }
            fn height(&self) -> i32 {
                384
            }
            fn state(&self, name: &str) -> BlockStateId {
                horizon_world::worldgen::structure::legacy::block_state(self.1, name, &[])
            }
            fn state_with(&self, name: &str, properties: &[(&str, &str)]) -> BlockStateId {
                horizon_world::worldgen::structure::legacy::block_state(self.1, name, properties)
            }
            fn set_block(&mut self, pos: [i32; 3], state: BlockStateId) -> bool {
                self.0.push((pos, state));
                true
            }
        }
        install_parts(&[ModEntry::new("test", Box::new(Writes))]).unwrap();
        let generators = crate::game::native_chunk_generator::installed_chunk_generators();
        let facts = r#"{"defaults":{"minecraft:air":0,"minecraft:stone":1},"states":[["a",true,false,"","minecraft:air"],["s",false,false,"","minecraft:stone"]]}"#;
        let registry = Box::leak(Box::new(
            horizon_world::registry::BlockRegistry::from_facts_json(facts.as_bytes()).unwrap(),
        ));
        crate::game::api_values::resolve_block_definitions(registry).unwrap();
        let generator = generators["test:marker"].clone();
        let writes = std::thread::spawn(move || {
            let mut chunk = Written(Vec::new(), registry);
            generator.build_terrain(&mut chunk);
            chunk.0
        })
        .join()
        .unwrap();
        assert_eq!(writes, [([1, -64, 2], BlockStateId(1))]);
    }

    /// The vanilla levels are in the `minecraft` namespace, so only a mod with
    /// that id gets as far as naming one.
    #[test]
    fn a_vanilla_level_refuses_registration() {
        struct Vanilla;
        #[mod_api::__subscriptions]
        impl Hooks for Vanilla {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                r.register_dimension("the_end", mod_api::DimensionSettings::default());
            }
        }
        let errors = install_parts(&[ModEntry::new("minecraft", Box::new(Vanilla))]).unwrap_err();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("minecraft:the_end, which is a vanilla level"));
    }

    /// A registered dimension reaches the engine as the level stem and
    /// dimension type a data pack would give.
    #[test]
    fn a_registered_dimension_reaches_the_engine_as_pack_entries() {
        use horizon_world::worldgen::overrides::{DIMENSION_TYPES, DIMENSIONS, WorldgenOverrides};
        struct Pocket;
        #[mod_api::__subscriptions]
        impl Hooks for Pocket {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                r.register_dimension(
                    "test:pocket",
                    mod_api::DimensionSettings {
                        min_y: -128,
                        height: 448,
                        logical_height: 320,
                        has_skylight: false,
                        has_ceiling: true,
                        noise_settings: "minecraft:caves".into(),
                        biome: "minecraft:lush_caves".into(),
                        ..Default::default()
                    },
                );
            }
        }
        install_parts(&[ModEntry::new("test", Box::new(Pocket))]).unwrap();
        let dimensions = installed_dimensions();
        assert_eq!(dimensions.len(), 1);
        let mut overrides = WorldgenOverrides::default();
        overrides.add_mod_dimension(&dimensions[0]);
        let height = overrides.level_height("test:pocket").unwrap();
        assert_eq!(
            (
                height.min_y,
                height.height,
                height.logical_height,
                height.has_skylight,
                height.has_ceiling
            ),
            (-128, 448, 320, false, true)
        );
        assert_eq!(overrides.entries(DIMENSION_TYPES).count(), 1);
        let (id, stem) = overrides.entries(DIMENSIONS).next().unwrap();
        assert_eq!(id, "test:pocket");
        let stem: serde_json::Value = serde_json::from_str(stem).unwrap();
        assert_eq!(
            stem,
            serde_json::json!({
                "type": "test:pocket",
                "generator": {
                    "type": "minecraft:noise",
                    "settings": "minecraft:caves",
                    "biome_source": {"type": "minecraft:fixed", "biome": "minecraft:lush_caves"},
                },
            })
        );
    }
    #[test]
    fn dimension_shape_edges_refuse_before_install_with_exact_mod_message() {
        struct Shape(DimensionSettings);
        #[mod_api::__subscriptions]
        impl Hooks for Shape {
            fn register(&self, registrar: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                registrar.register_dimension("edge", self.0.clone());
            }
        }
        for (min_y, height, logical_height) in [
            (-2032, 16, 0),
            (-2032, 4064, 4064),
            (2016, 16, 16),
            (-64, 384, 384),
        ] {
            let settings = DimensionSettings {
                min_y,
                height,
                logical_height,
                ..Default::default()
            };
            install_parts(&[ModEntry::new(
                "shape_mod",
                Box::new(Shape(settings.clone())),
            )])
            .unwrap();
            assert_eq!(
                mod_api::__private::dimension_declarations(),
                vec![("shape_mod:edge".into(), settings)]
            );
        }
        let retained = mod_api::__private::dimension_declarations();
        for (min_y, height, logical_height) in [
            (-2048, 16, 0),
            (-63, 16, 0),
            (0, 0, 0),
            (0, 17, 0),
            (0, 16, -1),
            (0, 16, 17),
            (2016, 32, 0),
            (2016, 2147483632, 0),
        ] {
            let settings = DimensionSettings {
                min_y,
                height,
                logical_height,
                ..Default::default()
            };
            let error = install_parts(&[ModEntry::new("shape_mod", Box::new(Shape(settings)))])
                .unwrap_err();
            assert_eq!(
                error,
                vec![format!(
                    "shape_mod registered dimension shape_mod:edge with a build height the game refuses: min_y {min_y}, height {height}, logical_height {logical_height}"
                )]
            );
            assert_eq!(mod_api::__private::dimension_declarations(), retained);
        }
    }
}

#[cfg(test)]
mod main_dimension_tests {
    use super::*;

    #[test]
    fn the_default_settings_are_accepted_and_name_the_overworlds_type() {
        let settings = DimensionSettings::default();
        assert!(valid_shape(&settings));
        assert_eq!(settings.invalid_field(), None);
        let json = dimension_type(&settings);
        assert_eq!(json["coordinate_scale"], 1.0);
        assert_eq!(json["infiniburn"], "#minecraft:infiniburn_overworld");
        assert_eq!(json["monster_spawn_light_level"]["max_inclusive"], 7);
        assert_eq!(
            json["attributes"]["minecraft:gameplay/piglins_zombify"],
            true
        );
        assert_eq!(json["attributes"]["minecraft:gameplay/fast_lava"], false);
    }

    #[test]
    fn a_field_out_of_range_is_refused_by_name() {
        let refused = |edit: fn(&mut DimensionSettings)| {
            let mut settings = DimensionSettings::default();
            edit(&mut settings);
            settings.invalid_field().unwrap_or_default()
        };
        assert!(refused(|s| s.coordinate_scale = 0.0).contains("coordinate_scale"));
        assert!(refused(|s| s.coordinate_scale = f64::NAN).contains("coordinate_scale"));
        // below the codec's 1.0E-5, a ratio with a normal scale is infinite
        assert!(refused(|s| s.coordinate_scale = f64::from_bits(1)).contains("coordinate_scale"));
        assert!(refused(|s| s.coordinate_scale = 1.0e-6).contains("coordinate_scale"));
        assert_eq!(refused(|s| s.coordinate_scale = f64::from(1.0e-5_f32)), "");
        // the codec sets no range on the ambient light, only that it is a number
        assert_eq!(refused(|s| s.ambient_light = 1.5), "");
        assert!(refused(|s| s.ambient_light = f32::NAN).contains("ambient_light"));
        assert!(refused(|s| s.sky_light_level = 16.0).contains("sky_light_level"));
        assert!(
            refused(|s| s.monster_spawn_light_level = (8, 7)).contains("monster_spawn_light_level")
        );
        assert!(refused(|s| s.monster_spawn_block_light_limit = 16).contains("block_light_limit"));
        assert!(refused(|s| s.skybox = "moon".into()).contains("skybox"));
        assert!(refused(|s| s.cardinal_light = "up".into()).contains("cardinal_light"));
        assert!(
            refused(|s| s.infiniburn = "minecraft:infiniburn_end".into()).contains("infiniburn")
        );
    }
}

#[cfg(test)]
mod rule_tests {
    use super::*;

    /// The day cycle names the timelines and the clock, and the game's facts read them back.
    #[test]
    fn the_day_cycle_reaches_the_games_facts() {
        use horizon_world::dimension_type::DimensionFacts;
        let facts = |cycle: &str| {
            let settings = DimensionSettings {
                day_cycle: cycle.into(),
                ..Default::default()
            };
            assert_eq!(settings.invalid_field(), None);
            DimensionFacts::patched(
                DimensionFacts::overworld(),
                "test:cycle",
                &dimension_type(&settings),
            )
            .unwrap()
        };
        let overworld = facts("overworld");
        assert!(overworld.has_timeline("minecraft:day"));
        assert_eq!(
            overworld.default_clock.as_deref(),
            Some("minecraft:overworld")
        );
        let nether = facts("nether");
        assert!(!nether.has_timeline("minecraft:day") && nether.default_clock.is_none());
        let end = facts("end");
        assert_eq!(end.default_clock.as_deref(), Some("minecraft:the_end"));
        let none = facts("none");
        assert!(none.timelines.is_empty() && none.default_clock.is_none());
        let bad = DimensionSettings {
            day_cycle: "forever".into(),
            ..Default::default()
        };
        assert!(bad.invalid_field().unwrap().contains("day_cycle"));
    }

    /// The bed rule goes into the dimension type as vanilla's `bed_rule` attribute, and the game's
    /// facts read it back from there.
    #[test]
    fn the_bed_rule_reaches_the_games_facts() {
        use horizon_world::dimension_type::{BedRuleTest, DimensionFacts};
        let settings = DimensionSettings {
            bed_can_sleep: "never".into(),
            bed_can_set_spawn: "when_dark".into(),
            bed_destroy_on_use: true,
            bed_destroy_on_leave: true,
            ..Default::default()
        };
        assert_eq!(settings.invalid_field(), None);
        let facts = DimensionFacts::patched(
            DimensionFacts::overworld(),
            "test:bedless",
            &dimension_type(&settings),
        )
        .unwrap();
        assert_eq!(facts.bed_rule.can_sleep, BedRuleTest::Never);
        assert_eq!(facts.bed_rule.can_set_spawn, BedRuleTest::WhenDark);
        assert!(facts.bed_rule.destroy_on_use && facts.bed_rule.destroy_on_leave);
        // the straw bed has its own rule, vanilla's by default
        assert_eq!(facts.straw_bed_rule.can_set_spawn, BedRuleTest::Never);
        assert!(!facts.straw_bed_rule.destroy_on_use && facts.straw_bed_rule.destroy_on_leave);
        let straw = DimensionSettings {
            straw_bed_can_sleep: "always".into(),
            straw_bed_destroy_on_use: true,
            straw_bed_destroy_on_leave: false,
            ..Default::default()
        };
        let facts = DimensionFacts::patched(
            DimensionFacts::overworld(),
            "test:straw",
            &dimension_type(&straw),
        )
        .unwrap();
        assert_eq!(facts.straw_bed_rule.can_sleep, BedRuleTest::Always);
        assert!(facts.straw_bed_rule.destroy_on_use && !facts.straw_bed_rule.destroy_on_leave);
        let bad = DimensionSettings {
            bed_can_sleep: "sometimes".into(),
            ..Default::default()
        };
        assert!(bad.invalid_field().unwrap().contains("bed_can_sleep"));
    }
}

#[cfg(test)]
mod biome_source_tests {
    use super::*;
    use mod_api::BiomePoint;

    #[test]
    fn each_source_names_the_stem_the_data_pack_codec_reads() {
        let mut settings = DimensionSettings::default();
        assert_eq!(
            biome_source(&settings),
            json!({"type": "minecraft:fixed", "biome": "minecraft:plains"})
        );
        settings.biome_source = BiomeSourceKind::Checkerboard;
        settings.biomes = vec!["minecraft:desert".into(), "minecraft:plains".into()];
        settings.biome_scale = 3;
        assert_eq!(
            biome_source(&settings),
            json!({
                "type": "minecraft:checkerboard",
                "biomes": ["minecraft:desert", "minecraft:plains"],
                "scale": 3
            })
        );
        settings.biome_source = BiomeSourceKind::MultiNoise;
        settings.biome_preset = "minecraft:nether".into();
        assert_eq!(
            biome_source(&settings),
            json!({"type": "minecraft:multi_noise", "preset": "minecraft:nether"})
        );
        settings.biome_source = BiomeSourceKind::End;
        assert_eq!(
            biome_source(&settings),
            json!({"type": "minecraft:the_end"})
        );
        settings.biome_source = BiomeSourceKind::Points;
        settings.biome_points = vec![BiomePoint {
            biome: "mymod:glowing_forest".into(),
            temperature: (-0.5, 0.25),
            humidity: (0.5, 0.5),
            continentalness: (-1.0, 2.0),
            erosion: (-2.0, 0.0),
            depth: (0.0, 1.0),
            weirdness: (-0.5, 0.5),
            offset: 0.25,
        }];
        assert_eq!(
            biome_source(&settings),
            json!({
                "type": "minecraft:multi_noise",
                "biomes": [{
                    "biome": "mymod:glowing_forest",
                    "parameters": {
                        "temperature": [-0.5, 0.25],
                        "humidity": [0.5, 0.5],
                        "continentalness": [-1.0, 2.0],
                        "erosion": [-2.0, 0.0],
                        "depth": [0.0, 1.0],
                        "weirdness": [-0.5, 0.5],
                        "offset": 0.25
                    }
                }]
            })
        );
        // A mod's source replaces whichever kind is set.
        settings.mod_biome_source = Some("mymod:chaos".into());
        assert_eq!(biome_source(&settings), json!({"type": "mymod:chaos"}));
    }

    #[test]
    fn a_source_the_codec_refuses_is_refused_by_name() {
        let refused = |edit: fn(&mut DimensionSettings)| {
            let mut settings = DimensionSettings::default();
            edit(&mut settings);
            settings.invalid_biome_source().unwrap_or_default()
        };
        assert_eq!(refused(|_| {}), "");
        // vanilla's codec takes a holder set of any size, so 65 biomes are accepted
        assert_eq!(
            refused(|s| {
                s.biome_source = BiomeSourceKind::Checkerboard;
                s.biomes = (0..65).map(|n| format!("test:biome_{n}")).collect();
            }),
            ""
        );
        assert!(
            refused(|s| s.biome_source = BiomeSourceKind::Checkerboard).contains("checkerboard")
        );
        assert!(
            refused(|s| {
                s.biome_source = BiomeSourceKind::Checkerboard;
                s.biomes = vec!["minecraft:plains".into()];
                s.biome_scale = 63;
            })
            .contains("biome_scale")
        );
        assert!(
            refused(|s| {
                s.biome_source = BiomeSourceKind::MultiNoise;
                s.biome_preset = "minecraft:end".into();
            })
            .contains("biome_preset")
        );
        assert!(
            refused(|s| {
                s.biome_source = BiomeSourceKind::End;
                s.generator = "mymod:void".into();
            })
            .contains("fixed biome source")
        );
        assert_eq!(
            refused(|s| {
                s.biome_source = BiomeSourceKind::MultiNoise;
            }),
            ""
        );
        // A points source: at least one point, each as the climate codec reads it.
        let point = || BiomePoint {
            biome: "mymod:glowing_forest".into(),
            temperature: (-2.0, 2.0),
            humidity: (0.0, 0.0),
            continentalness: (0.0, 0.0),
            erosion: (0.0, 0.0),
            depth: (0.0, 0.0),
            weirdness: (0.0, 0.0),
            offset: 1.0,
        };
        let points = |edit: fn(&mut BiomePoint)| {
            let mut p = point();
            edit(&mut p);
            DimensionSettings {
                biome_source: BiomeSourceKind::Points,
                biome_points: vec![p],
                ..Default::default()
            }
            .invalid_biome_source()
            .unwrap_or_default()
        };
        assert_eq!(points(|_| {}), "");
        assert!(refused(|s| s.biome_source = BiomeSourceKind::Points).contains("at least one"));
        assert!(points(|p| p.temperature = (0.5, -0.5)).contains("biome point"));
        assert!(points(|p| p.humidity = (-2.5, 0.0)).contains("biome point"));
        assert!(points(|p| p.erosion = (0.0, 2.001)).contains("biome point"));
        assert!(points(|p| p.depth = (f32::NAN, 0.0)).contains("biome point"));
        assert!(points(|p| p.offset = 1.5).contains("biome point"));
        assert!(points(|p| p.offset = -0.1).contains("biome point"));
        assert!(points(|p| p.biome.clear()).contains("biome point"));
        // A valid points source still refuses a chunk generator of the mod's own.
        assert!(
            DimensionSettings {
                biome_source: BiomeSourceKind::Points,
                biome_points: vec![point()],
                generator: "mymod:void".into(),
                ..Default::default()
            }
            .invalid_biome_source()
            .unwrap_or_default()
            .contains("fixed biome source")
        );
    }
}

#[cfg(test)]
mod visual_tests {
    use super::*;

    #[test]
    fn the_sky_fog_and_clouds_are_written_only_when_set() {
        let plain = dimension_type(&DimensionSettings::default());
        assert!(
            plain["attributes"]
                .get("minecraft:visual/sky_color")
                .is_none()
        );
        let settings = DimensionSettings {
            sky_color: Some("#112233".into()),
            fog_color: Some("#445566".into()),
            cloud_height: Some(192.0),
            ..Default::default()
        };
        assert_eq!(settings.invalid_field(), None);
        let attributes = &dimension_type(&settings)["attributes"];
        assert_eq!(attributes["minecraft:visual/sky_color"], "#112233");
        assert_eq!(attributes["minecraft:visual/fog_color"], "#445566");
        assert_eq!(attributes["minecraft:visual/cloud_height"], 192.0);
    }

    #[test]
    fn a_colour_that_is_not_rrggbb_is_refused() {
        for bad in ["112233", "#12345", "#12345g", "#1234567"] {
            let settings = DimensionSettings {
                sky_color: Some(bad.into()),
                ..Default::default()
            };
            assert!(
                settings.invalid_field().unwrap().contains("#rrggbb"),
                "{bad}"
            );
        }
    }
}
