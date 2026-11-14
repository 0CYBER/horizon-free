use super::*;

pub(super) fn load_world_rest(
    resource_root: &horizon_assets::resource_source::ResourceRoot,
    resources: &Resources,
    assets: &Assets,
    behaviour_json: &serde_json::Value,
    gameplay_projection: &[horizon_client::blocks::GameplayProjection],
    entity_models: Option<&horizon_client::mobs::EntityModelDocument>,
    item_defaults: &horizon_world::item_defaults::ItemDefaults,
    visibility: horizon_world::block_render::FaceVisibility,
    progress: &Progress,
    started: Instant,
    // False on a dedicated server, which draws nothing: no block entity
    // renderer or animate tables (client-side display ticks).
    draws: bool,
) -> Result<WorldRest> {
    let root = resource_root.prepared();
    let facts = resources.facts();
    let biomes = resources.biomes();
    let threads = resources.threads();
    let registry = assets.registry;
    let content = horizon_client::datapack::DataPack::from_source(
        "horizon-content".into(),
        resource_root.data_changes().clone(),
    );
    let files = horizon_client::datapack::DataStore::load(&[&content])
        .files
        .into_iter()
        .collect::<Vec<_>>();
    let inventory = horizon_mod_package::enabled::EnabledMods::read(root)?;
    let conditions = horizon_world::conditions::LoadConditions::new(inventory.server_ids());
    let overlay = &horizon_world::loot::PackOverlay {
        conditions: &conditions,
        files: &files,
    };
    progress.set(6);
    // Independent tables load together, within the worker budget the settings
    // allow. Only the recipes and the stations read another table: the items.
    let load_items = || -> Result<_> {
        let mut items = startup::time("rest.items", || {
            Items::load(root, registry, facts, overlay, resources.item_defaults())
        })?;
        items.bind_component_damage_catalog(&resources.motion().health.damage);
        api_values::resolve_item_registry(&items)?;
        log::info!("load items: {:.2}s", started.elapsed().as_secs_f32());
        progress.set(7);
        // The recipe tables, the stations and the recipe book read one folder:
        // hold its files so it is read once.
        let _recipe_files = horizon_client::json_folder::read(&root.join("data/minecraft/recipe"));
        let sources = startup::time("rest.recipe_sources", || {
            lan::sync::initial_sources(root, &resources.core.item_data, overlay)
        })?;
        let recipes = startup::time("rest.recipes", || {
            Recipes::from_resolved(root, &items, overlay, &sources)
        })?;
        let stations = startup::time("rest.stations", || {
            horizon_client::stations::StationData::from_sources(
                root,
                &items,
                overlay,
                resources.item_defaults(),
                (&sources, &recipes),
            )
        });
        Ok((items, recipes, stations))
    };
    let load_behaviours = || {
        startup::time("rest.behaviours", || {
            Behaviours::load_with_gameplay(registry, root, behaviour_json, gameplay_projection)
        })
    };
    let load_block_entity_render = || -> Result<_> {
        let _step = startup::step("rest.block_entity_render");
        if !draws {
            return Ok(Default::default());
        }
        let mut renderer = BlockEntityRenderer::load_with_visibility(
            resource_root,
            registry,
            visibility,
            resources.item_defaults(),
        )?;
        renderer.add_mod_signs(facts, registry);
        Ok(renderer)
    };
    let load_animate = || resource_tables::load_animate_tables(root, registry, draws, biomes);
    let load_mobs = || -> Result<_> {
        let _step = startup::step("rest.mobs");
        let catalog = &resources.motion().attributes;
        let mut types = if draws {
            match entity_models {
                Some(document) => MobTypes::load_with_document_and_defaults(
                    root,
                    document,
                    item_defaults.entries(),
                    catalog,
                )?,
                None => MobTypes::load_with_defaults(root, item_defaults.entries(), catalog)?,
            }
        } else {
            MobTypes::load_for_server(root, catalog)?
        };
        // #845: the declared sets take their types' rows — the merged rows
        // the live attribute map's catalog took in core (the template's with
        // the declared set on top) — as their attribute map and spawn
        // fields, so a mob of the type spawns with what the map answers.
        for declaration in mod_api::__private::entity_attribute_declarations() {
            let row = catalog.entity_row(&declaration.id).map(std::sync::Arc::new);
            match (types.types.get_mut(&declaration.id), row) {
                (Some(kind), Some(row)) => kind.take_attribute_row(row, catalog),
                (Some(_), None) => log::warn!(
                    "Horizon: mod entity attributes of {} never reached the catalog; ignored",
                    declaration.id
                ),
                (None, _) => log::warn!(
                    "Horizon: mod entity attributes of {} name no loaded entity type; ignored",
                    declaration.id
                ),
            }
        }
        // #846: the types mods added attributes to take their grown rows the
        // same way, so a new mob of the type spawns with them.
        resource_load::take_modified_attribute_rows(
            &mut types.types,
            catalog,
            mod_api::__private::entity_attribute_modifications(),
        );
        let mob_types: HashMap<_, _> = types
            .types
            .into_iter()
            .map(|(k, v)| (k, Arc::new(v)))
            .collect();
        let mut mob_data = horizon_client::mobs::MobData::load(resource_root, facts);
        // #847: the mods' spawn rules (`RegisterSpawnPlacementsEvent`) take
        // their types' placement, heightmap and predicate here, once, so a
        // spawn check reads one table with no pass over the mods.
        for declaration in mod_api::__private::spawn_placement_declarations() {
            if !mob_types.contains_key(&declaration.id) {
                return Err(format!(
                    "mod spawn placement of {} names no loaded entity type",
                    declaration.id
                )
                .into());
            }
            mob_data.register_mod_spawn(&declaration.id, &declaration.rule);
        }
        mob_data.cem = std::sync::RwLock::new(Arc::new(if draws {
            horizon_client::mobs::cem::CemModels::load(resource_root.source(), &mob_types)
        } else {
            horizon_client::mobs::cem::CemModels::default()
        }));
        horizon_client::mobs::trial_spawner::load_registry(root);
        Ok((mob_types, mob_data))
    };
    let load_transformers = || -> Result<_> {
        let _step = startup::step("rest.transformers_models");
        let transformers = horizon_client::block_transform::Transformers::load(root, registry)?;
        let player_type = match entity_models {
            Some(document) => perspective::player_type_with_document(root, document),
            None => perspective::player_type(root),
        };
        let models = (
            player_type,
            player_arms(root, "minecraft:player#main", entity_models),
            player_arms(root, "minecraft:player_slim#main", entity_models),
            match entity_models {
                Some(document) => document.model("minecraft:spin_attack#main"),
                None => horizon_client::mobs::load_model(root, "minecraft:spin_attack#main"),
            },
        );
        Ok((transformers, models))
    };
    // The errors cross threads as text.
    fn text<T, E: std::fmt::Display>(
        result: std::result::Result<T, E>,
    ) -> std::result::Result<T, String> {
        result.map_err(|e| e.to_string())
    }
    let (
        items,
        behaviours,
        block_entity_render,
        animate,
        mobs,
        transformers,
        providers,
        music,
        ambient,
        nether_environment,
        end_environment,
    ) = if threads > 1 {
        std::thread::scope(|scope| {
            let items = named_scope(scope, "items", || text(load_items()));
            let behaviours = named_scope(scope, "behaviours", || text(load_behaviours()));
            let block_entity_render = named_scope(scope, "block_entity_render", || {
                text(load_block_entity_render())
            });
            let animate = named_scope(scope, "animate", || text(load_animate()));
            let mobs = named_scope(scope, "mobs", || text(load_mobs()));
            let transformers = named_scope(scope, "transformers", || text(load_transformers()));
            let providers = named_scope(scope, "providers", || {
                startup::time("rest.providers", || Providers::load(root))
            });
            let music = named_scope(scope, "music", || {
                startup::time("rest.music", || {
                    horizon_client::music::MusicTables::load(root)
                })
            });
            let ambient = named_scope(scope, "ambient", || {
                startup::time("rest.ambient", || {
                    horizon_client::ambience::AmbientTables::load(root)
                })
            });
            let nether_environment = named_scope(scope, "nether_environment", || {
                startup::time("rest.nether_environment", || {
                    text(Environment::load_dimension(
                        &horizon_assets::resource_source::ResourceSource::directory(
                            root.to_path_buf(),
                        ),
                        "the_nether",
                    ))
                })
            });
            let end_environment = named_scope(scope, "end_environment", || {
                startup::time("rest.end_environment", || {
                    text(Environment::load_dimension(
                        &horizon_assets::resource_source::ResourceSource::directory(
                            root.to_path_buf(),
                        ),
                        "the_end",
                    ))
                })
            });
            (
                items.join().unwrap(),
                behaviours.join().unwrap(),
                block_entity_render.join().unwrap(),
                animate.join().unwrap(),
                mobs.join().unwrap(),
                transformers.join().unwrap(),
                providers.join().unwrap(),
                music.join().unwrap(),
                ambient.join().unwrap(),
                nether_environment.join().unwrap(),
                end_environment.join().unwrap(),
            )
        })
    } else {
        (
            text(load_items()),
            text(load_behaviours()),
            text(load_block_entity_render()),
            text(load_animate()),
            text(load_mobs()),
            text(load_transformers()),
            Providers::load(root),
            startup::time("rest.music", || {
                horizon_client::music::MusicTables::load(root)
            }),
            startup::time("rest.ambient", || {
                horizon_client::ambience::AmbientTables::load(root)
            }),
            startup::time("rest.nether_environment", || {
                text(Environment::load_dimension(
                    &horizon_assets::resource_source::ResourceSource::directory(root.to_path_buf()),
                    "the_nether",
                ))
            }),
            startup::time("rest.end_environment", || {
                text(Environment::load_dimension(
                    &horizon_assets::resource_source::ResourceSource::directory(root.to_path_buf()),
                    "the_end",
                ))
            }),
        )
    };
    let (items, recipes, stations) = items?;
    let (animate, fog) = animate?;
    let (mob_types, mob_data) = mobs?;
    let (transformers, (player_type, player_arm, player_arm_slim, spin_attack)) = transformers?;
    let nether_environment = nether_environment?;
    let end_environment = end_environment?;
    log::info!(
        "Horizon: resources loaded in {:.1}s; {} block states, {} items, {} biomes; {} worker threads",
        started.elapsed().as_secs_f32(),
        facts.states.len(),
        items.items.len(),
        biomes.list.len(),
        threads
    );
    progress.set(8);
    Ok(WorldRest {
        stations,
        items,
        recipes,
        transformers,
        providers,
        block_entity_render: block_entity_render?,
        animate,
        fog,
        music,
        ambient,
        nether_environment,
        end_environment,
        behaviours: behaviours?,
        mob_types,
        mob_data,
        player_type,
        player_arm,
        player_arm_slim,
        spin_attack,
    })
}
