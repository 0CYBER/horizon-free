//! Entering a world without blocking the window: the deferred entry, world creation, the world
//! resources' background load, and the world's own content tables.
use super::*;

impl Game<'_> {
    /// The server's data-pack enchantments, or the local world's registry.
    pub(super) fn item_enchantments(&self) -> &Arc<horizon_client::enchant::Enchantments> {
        match self.remote.as_ref().and_then(|r| r.enchantments.as_ref()) {
            Some(e) => e,
            None => &self.stations().enchantments,
        }
    }

    pub(super) fn defer_world_entry(&mut self, entry: PendingWorldEntry) -> Result<bool> {
        self.poll_world_rest()?;
        if self.rest_loader.is_none() {
            return Ok(false);
        }
        self.perf.world_opening();
        if self.title.is_none() {
            let title = self
                .new_title(std::env::current_dir()?.join("saves"))
                .with_user(|| self.options.player_name());
            self.show_title(title);
        }
        self.pending_entry = Some(entry);
        log::info!("Horizon: world entry waiting for world resources");
        Ok(true)
    }

    pub(super) fn resume_world_entry(&mut self) -> Result<()> {
        self.poll_integrated_start()?;
        self.poll_world_rest()?;
        self.poll_world_creation();
        if self.rest_loader.is_some() {
            return Ok(());
        }
        let on_world_screens = self.title.as_ref().is_some_and(|title| {
            !matches!(
                title.mode,
                horizon_client::title::Mode::Main | horizon_client::title::Mode::Multiplayer(_)
            )
        });
        world_loading::preload_world_data(
            on_world_screens,
            &self.options.resources,
            &self.pack.data_changes,
            self.pack.assets.registry,
            &self.res.core.item_data,
        );
        if std::mem::take(&mut self.pending_pack_reload) {
            self.reload_packs();
        }
        match self.pending_entry.take() {
            Some(PendingWorldEntry::Local {
                folder,
                name,
                seed,
                creative,
                create,
            }) => {
                self.open_world(&folder, name.as_deref(), seed, creative, create.as_deref())?;
            }
            Some(PendingWorldEntry::Server {
                address,
                name,
                cookies,
                lan,
                realm,
            }) => {
                self.connect_server_with_options(&address, &name, cookies, lan, realm)?;
            }
            Some(PendingWorldEntry::StudioReplay(path)) => {
                let path_str = path.to_string_lossy().to_string();
                if let Err(e) = self.open_studio_replay(&path_str) {
                    log::warn!("Horizon Studio: could not open {path_str}: {e}");
                }
            }
            None => {}
        }
        Ok(())
    }

    /// `CreateWorldScreen.openCreateWorldScreen`: read the world generation
    /// data the create screen describes, on a worker, then open the screen
    /// (`title.preparing`).
    pub(super) fn poll_world_creation(&mut self) {
        let Some(title) = self.title.as_ref() else {
            return;
        };
        if !title.preparing() {
            return;
        }
        if self.creation_job.is_none() {
            let dir = self.options.resources.join("worldgen");
            let registry = self.pack.assets.registry;
            // The chosen packs' worldgen over the game's own:
            // `applyNewPackConfig` reloads with the new pack config.
            let overrides = title
                .creation_packs()
                .map(|(dir, packs)| {
                    world_loading::creation_worldgen_overrides(
                        dir.as_deref(),
                        &self.options.resources,
                        &packs,
                        self.res.biomes(),
                        &self.pack.additional_data,
                    )
                })
                .unwrap_or_default();
            let data_source = self.pack.data_changes.clone();
            let job = horizon_guest_stack::builder()
                .name("world-creation".into())
                .spawn(move || {
                    horizon_client::vanillagen::shared_data_from(
                        &dir,
                        &overrides,
                        registry,
                        &data_source,
                    )
                    .map(|_| ())
                    .map_err(|e| e.to_string())
                });
            match job {
                Ok(job) => self.creation_job = Some(job),
                Err(error) => {
                    log::warn!("Horizon: world creation worker could not start: {error}");
                    if let Some(title) = self.title.as_mut() {
                        title.finish_preparing();
                        title.failed(format!(
                            "{}: {error}",
                            horizon_client::lang::get("createWorld.preparing")
                        ));
                    }
                }
            }
            return;
        }
        if !self
            .creation_job
            .as_ref()
            .is_none_or(std::thread::JoinHandle::is_finished)
        {
            return;
        }
        let outcome = self.creation_job.take().and_then(|job| job.join().ok());
        if let Some(title) = self.title.as_mut() {
            match outcome {
                Some(Ok(())) => title.finish_preparing(),
                // `applyNewPackConfig`'s failure: back out of the read with
                // the message, the Create screen's options intact.
                outcome => {
                    let e = outcome
                        .and_then(|r| r.err())
                        .unwrap_or_else(|| "the read never ran".to_owned());
                    log::warn!("Horizon: preparing world creation failed: {e}");
                    // Clear the flag first: `failed` sets the error screen
                    // with Create as its back target, which
                    // `finish_preparing` would replace with a fresh world.
                    title.finish_preparing();
                    title.failed(format!(
                        "{}: {e}",
                        horizon_client::lang::get_or(
                            "createWorld.preparing",
                            "Preparing for world creation..."
                        )
                    ));
                }
            }
        }
    }

    pub(super) fn poll_world_rest(&mut self) -> Result<()> {
        if let Some(loader) = &self.rest_loader
            && loader.ready()?
        {
            self.rest_loader
                .take()
                .ok_or("missing world loader")?
                .finish()?;
        }
        Ok(())
    }

    // ---- the world's data packs' content tables ---------------------------

    /// The content tables the world plays with: its data packs' over the
    /// game's own when it has packs, `Resources.rest`'s own otherwise.
    pub(super) fn items(&self) -> &Items {
        world_items!(self)
    }
    pub(super) fn mob_suffocation(
        &self,
    ) -> &horizon_world::suffocation::SuffocationRegistry<'static> {
        self.world_tables
            .as_ref()
            .map_or_else(|| self.res.suffocation(), |t| &t.suffocation)
    }
    pub(super) fn recipes(&self) -> &Recipes {
        match &self.world_tables {
            Some(t) => &t.recipes,
            None => self.synced_recipes().unwrap_or_else(|| self.res.recipes()),
        }
    }
    pub(super) fn stations(&self) -> &horizon_client::stations::StationData {
        match &self.world_tables {
            Some(t) => &t.stations,
            None => self.res.stations(),
        }
    }

    /// What the chunk and far-terrain workers mesh with: the pack's context,
    /// over the world's biomes when it has its own.
    pub(super) fn worker_context(&self) -> Arc<WorkerContext> {
        match &self.world_biomes {
            Some(table) => self.pack.context_with(table.clone()),
            None => self.pack.context.clone(),
        }
    }
    pub(super) fn pack_files(&self) -> horizon_world::loot::PackOverlay<'_> {
        pack_files(&self.world_tables)
    }
    /// Load the world's data packs' content tables (`None`: the world has no
    /// packs, and `Resources.rest` stays in use). This re-reads the tables,
    /// so it runs once per world open and on `/reload`.
    pub(super) fn stage_world_tables(
        &mut self,
        stack: &functions::PackStack,
        reload: bool,
        root: &std::path::Path,
    ) -> Result<(
        Option<WorldTables>,
        horizon_client::health::HealthResources,
        Option<mod_api::DatapackRegistrySet>,
    )> {
        let phase = self.mod_phase.clone();
        let _reload = phase
            .reloading()
            .expect("server tables on their owning thread");
        // The ask of this load, both here and on `/reload`: a native mod
        // recomputes the pack files it contributes every time.
        let mut contributed = Vec::new();
        self.native_data_pack_files(&mut contributed);
        let files = functions::overlay_files(stack, contributed);
        let inventory = horizon_mod_package::enabled::EnabledMods::read(root)?;
        let conditions = self.registered_load_conditions(&inventory)?;
        let overlay = horizon_world::loot::PackOverlay {
            conditions: &conditions,
            files: &files,
        };
        let mut data_map_types = Vec::new();
        self.native_data_map_types(&mut data_map_types);
        let base = if reload {
            &self.health_resources
        } else {
            &self.res.motion().health
        };
        let health = base.with_packs(root, &overlay, reload)?;
        // With no pack, `res` holds the tables, unless content mods switched
        // live since it was loaded: their data is in the new root.
        let mut items = Items::load(
            root,
            self.pack.assets.registry,
            self.res.facts(),
            &overlay,
            self.res.item_defaults(),
        )?;
        class_tags::install(&mut items, self.res.facts());
        api_values::resolve_enchantment_registry(&items)?;
        // Item damage predicates use the same frozen registry as player hurt.
        Arc::make_mut(&mut items.enchantments).bind_damage_catalog(&health.damage);
        items.bind_component_damage_catalog(&health.damage);
        // The recipe tables, the stations and the recipe book read one folder:
        // hold its files so it is read once.
        let _recipe_files = horizon_client::json_folder::read(&root.join("data/minecraft/recipe"));
        let sources = if reload {
            lan::sync::prepare_server_sources(root, &self.res.core.item_data, &overlay)?
        } else {
            lan::sync::load_sources(
                root,
                &self.res.core.item_data,
                self.options.execution_role.data_load_origin(),
                &overlay,
            )?
        };
        let recipes = Recipes::from_resolved(root, &items, &overlay, &sources)?;
        let mut stations = horizon_client::stations::StationData::from_sources(
            root,
            &items,
            &overlay,
            self.res.item_defaults(),
            (&sources, &recipes),
        );
        self.prepare_professions(root, &items, &mut stations)?;
        // The world's loaded datapack registries are a mod-declared registry's
        // authority for the data maps' id checks, as the command ids are for
        // the registries they list.
        // Resolve membership against the very generation that will be published.
        // In particular, generation exhaustion refuses preparation before any state changes.
        let datapack_registries = self
            .datapack_candidate(stack, &conditions)?
            .map(
                |candidate| match self.mods.datapack_registries.as_ref().filter(|_| reload) {
                    Some(old) => old
                        .reload(candidate)
                        .map_err(|error| format!("datapack registry reload: {error:?}")),
                    None => Ok(mod_api::DatapackRegistrySet::first(
                        candidate,
                        horizon_client::datapack_net::next_session(),
                    )),
                },
            )
            .transpose()?;
        let received = horizon_client::datapack_net::received();
        let datapack_set = if self.is_logical_server() {
            datapack_registries.as_ref()
        } else {
            received.as_deref()
        };
        let registries = native_data_maps::WorldRegistries::new(
            self.ui
                .registries
                .by_registry
                .iter()
                .map(|(registry, entries)| (registry.as_str(), entries.ids.as_slice())),
            datapack_set,
            &items,
            root,
            &overlay,
        );
        let causes_suffocation = items
            .block_tags
            .get("minecraft:causes_suffocation")
            .into_iter()
            .flatten()
            .cloned()
            .collect();
        let suffocation = horizon_client::mobs::suffocation::bind(
            self.res.core.collision.clone(),
            &causes_suffocation,
        )?;
        // A registry whose pack tags don't load keeps the game's own tags
        // (`in_tag` falls back to them), rather than matching no tag at all.
        let pack_tags: HashMap<_, _> = WorldTables::PACK_TAG_REGISTRIES
            .into_iter()
            .chain(horizon_client::neoforge_mob_maps::MAPS.map(|definition| definition.directory))
            .filter(|directory| *directory != "item")
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .filter_map(|registry| {
                match horizon_client::items::load_tags(root, registry, &overlay) {
                    Ok(tags) => Some((registry, tags)),
                    Err(e) => {
                        log::warn!("Horizon: the world's {registry} tags are not applied: {e}");
                        None
                    }
                }
            })
            .collect();
        // Items already decoded the root's command export. Block-state-only
        // command metadata is not the registry universe, and professions and
        // biomes belong to the live world candidate rather than that export.
        for registry in ["minecraft:sound_event", "minecraft:game_event"] {
            if items.component_registry.ids(registry).is_empty() {
                return Err(format!(
                    "builtin data maps require {registry} in command-registries.json"
                )
                .into());
            }
        }
        let builtin = neoforge_mob_maps::load(
            &overlay,
            |directory| {
                if directory == "item" {
                    Some(&items.item_tags)
                } else {
                    pack_tags.get(directory)
                }
            },
            |registry, id| match registry {
                "minecraft:item" => items.items.contains_key(id),
                "minecraft:villager_profession" => stations.professions.resolve(id).is_some(),
                "minecraft:worldgen/biome" => self
                    .world_biomes
                    .as_ref()
                    .unwrap_or_else(|| self.res.biomes())
                    .contains(id),
                "minecraft:entity_type" => {
                    self.res.mob_types().contains_key(id)
                        || items
                            .component_registry
                            .ids(registry)
                            .binary_search_by(|known| known.as_str().cmp(id))
                            .is_ok()
                }
                _ => items
                    .component_registry
                    .ids(registry)
                    .binary_search_by(|known| known.as_str().cmp(id))
                    .is_ok(),
            },
        )?;
        let data_maps = native_data_maps::DataMaps::load(&data_map_types, &overlay, &registries)
            .with_builtins(builtin);
        drop(registries);
        conditions.check_errors()?;
        Ok((
            Some(WorldTables {
                conditions,
                suffocation,
                files,
                items: Arc::new(items),
                recipes,
                stations,
                reload_sources: reload.then_some(sources),
                data_maps,
                pack_tags,
            }),
            health,
            datapack_registries,
        ))
    }
    /// The entries of the mods' datapack registries in the winning pack files: decoded with
    /// each registry's schema and checked as one candidate, so a bad entry fails the load
    /// before any table is published. `None` when no mod declared a registry: nothing is
    /// scanned. A file whose `neoforge:conditions` fail is absent; the key is dropped before
    /// the schema sees the file.
    fn datapack_candidate(
        &self,
        stack: &functions::PackStack,
        conditions: &horizon_world::conditions::LoadConditions,
    ) -> Result<Option<mod_api::DatapackCandidate>> {
        let declarations = mod_api::__private::datapack_registry_declarations();
        if declarations.is_empty() {
            return Ok(None);
        }
        let keys: Vec<(&str, &str)> = declarations
            .iter()
            .filter_map(|declaration| declaration.key.split_once(':'))
            .collect();
        // Content files come before the world's enabled packs, lowest priority first.
        let packs: Vec<_> = std::iter::once(&stack.content)
            .chain(stack.enabled_packs())
            .collect();
        let files: Vec<_> = horizon_client::datapack::registry_files(
            &packs,
            &keys,
            !conditions.has_registered(),
            |path, pack, bytes| {
                !super::condition_reload_state::registry_versions(
                    vec![(pack.to_owned(), bytes.to_vec())],
                    conditions,
                    path,
                )
                .is_empty()
            },
        )
        .map_err(|error| {
            format!(
                "datapack registry {} entry {} (pack {}, file {}): {:?}",
                error.registry, error.entry, error.pack, error.file, error.reason
            )
        })?
        .into_iter()
        .collect();
        mod_api::DatapackCandidate::build(declarations, &files)
            .map(Some)
            .map_err(|error| {
                format!(
                    "datapack registry {} entry {} (pack {}, file {}): {:?}",
                    error.registry, error.entry, error.pack, error.file, error.reason
                )
                .into()
            })
    }
    /// A world open's tables, and the first generation of the registries the mods declared.
    pub(super) fn open_world_tables(
        &mut self,
        stack: &functions::PackStack,
        root: &std::path::Path,
    ) -> Result<()> {
        // A new world load: a saved base clamped into its bounds warns again (#3728).
        horizon_client::living_attributes::LOAD_CLAMPS.begin_load();
        let (tables, health, registries) = self.stage_world_tables(stack, false, root)?;
        self.world_tables = tables;
        self.health_resources = health;
        self.mods.datapack_registries = registries.map(Arc::new);
        self.data_map_registries = self
            .world_tables
            .as_ref()
            .map_or_else(Vec::new, |t| t.data_maps.registries());
        self.install_block_maps();
        self.install_profession_save_snapshots();
        self.share_mob_maps();
        horizon_client::datapack_net::serve(self.mods.datapack_registries.clone());
        Ok(())
    }
    /// Sends each joined Horizon client the snapshot of the set just published, built from the
    /// offer owned by its connection. A refused replacement ends that connection with its reason.
    fn send_datapack_snapshots(&mut self) {
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        for guest in host
            .hub
            .guests
            .iter()
            .filter(|g| g.placed && g.participates_online() && g.brand == "horizon")
        {
            horizon_client::datapack_net::reload_connection(&guest.sender);
        }
    }
    /// Reload the content tables with the world's packs, as
    /// `/reload` reloads a server's (`ReloadableServerResources`).
    /// Publish tables before function admission, leaving advancement rewards queued
    /// until the reload's new function store is available.
    pub(super) fn install_world_tables(&mut self, prepared: PreparedWorldReload) {
        let PreparedWorldReload {
            structure_spawns,
            tables,
            advancements,
            health,
            parts,
            wire,
            datapack_registries,
        } = prepared;
        lan::sync::forget_server_sources();
        if let Some(tables) = &tables
            && let Some(sources) = &tables.reload_sources
        {
            let overlay = horizon_world::loot::PackOverlay {
                conditions: &tables.conditions,
                files: &tables.files,
            };
            lan::sync::install_server_sources(
                &self.options.resources,
                &self.res.core.item_data,
                &overlay,
                sources.clone(),
            );
        }
        self.mods.datapack_registries = datapack_registries.map(Arc::new);
        self.health_resources = health;
        self.world_tables = tables;
        self.data_map_registries = self
            .world_tables
            .as_ref()
            .map_or_else(Vec::new, |t| t.data_maps.registries());
        self.install_block_maps();
        self.install_profession_save_snapshots();
        self.share_mob_maps();
        self.world.pois.clear();
        for world in self.levels.parked_worlds() {
            world.pois.clear();
        }
        horizon_client::datapack_net::serve(self.mods.datapack_registries.clone());
        self.send_datapack_snapshots();
        self.refresh_recipe_property_context();
        self.structure_spawns = structure_spawns;
        self.server_items = None;
        self.refresh_infiniburn();
        self.install_parts(parts);
        self.damage_tag_generation = self.damage_tag_generation.wrapping_add(1);
        self.forget_book();
        horizon_client::structure_blocks::forget_loot_data();
        self.load_server_loot();
        // Before the players are sent the reloaded data below.
        self.sync_recipe_content();
        // PlayerList.reloadResources: the tags and recipes go to the players
        // and their advancements reload. The synchronized registries (the
        // dialogs, and the commands' ids of them) stay the hub's.
        lan::sync::install_wire_reload(
            &self.options.resources,
            &self.res.core.item_data,
            &self.pack_files(),
            wire.clone(),
        );
        if self.lan.is_some()
            && let Some(sync) = wire
        {
            self.with_advancement_hub(|hub, owners| hub.reload_resources(sync, owners));
        }
        self.adv.tree = advancements;
        self.adv.reset_tree();
        self.announce_data_bound(mod_api::TagsUpdateCause::ServerDataLoad { reload: true });
        self.resolve_enchant_particles();
        self.announce_data_maps_updated(mod_api::DataMapsUpdateCause::ServerReload);
        self.build_screen();
    }

    pub(super) fn join_world_rest(&mut self) -> Result<()> {
        if self.rest_loader.is_some() {
            let waited = Instant::now();
            log::info!("Horizon: waiting for world resources");
            while self.rest_loader.is_some() {
                self.poll_world_rest()?;
                if self.rest_loader.is_some() {
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
            log::info!(
                "Horizon: world resources joined after {} ms",
                waited.elapsed().as_millis()
            );
        }
        if self.res.rest.get().is_none() {
            return Err("world resources are unavailable".into());
        }
        Ok(())
    }
}

impl Game<'_> {
    pub(super) fn active_enchantment_definitions(&self) -> &horizon_client::enchant::Enchantments {
        match self.remote.as_ref().and_then(|r| r.enchantments.as_ref()) {
            Some(e) => e,
            None => &self.stations().enchantments,
        }
    }
    /// Build and publish one server snapshot for this world load or reload.
    pub(super) fn load_server_loot(&mut self) {
        self.server_items = None;
        let overlay = pack_files(&self.world_tables);
        let loot = if mod_api::__private::mods().is_empty() {
            horizon_client::structure_blocks::world_loot_data(
                &self.options.resources,
                &overlay,
                self.res.item_defaults(),
            )
        } else {
            match horizon_world::loot::LootData::load(
                &self.options.resources,
                &overlay,
                self.res.item_defaults(),
            ) {
                Ok(mut loot) => {
                    let context = match lan::sync::server_data_load(
                        &self.options.resources,
                        &self.res.core.item_data,
                    ) {
                        Ok(context) => context,
                        Err(error) => {
                            log::warn!("Horizon: server loot inventory unavailable: {error}");
                            self.server_loot.set(None);
                            return;
                        }
                    };
                    if let Err(error) = loot.apply_native_table_loads(&context) {
                        log::warn!("Horizon: native loot tables refused: {error}");
                        self.server_loot.set(None);
                        return;
                    }
                    Some(Arc::new(loot))
                }
                Err(e) => {
                    log::warn!("Horizon: server loot tables unavailable: {e}");
                    None
                }
            }
        };
        if !mod_api::__private::mods().is_empty()
            && let Some(loot) = &loot
        {
            let mut items = self.items().clone();
            items.set_loot_tables(loot.tables().clone());
            self.server_items = Some(Arc::new(items));
        }
        self.server_loot.set(loot);
    }

    pub(super) fn prepare_parts(&self, items: &Items) -> Result<PreparedParts> {
        let registry = self.pack.assets.registry;
        let block_tags = &items.block_tags;
        let tags = self.block_tag_sets(items);
        let parts = mod_api::__private::prepare_parts(
            block_tags,
            &items.item_tags,
            mod_api::__private::KnownIds {
                block: &|id| registry.default_state(id).is_some(),
                item: &|id| items.items.contains_key(id),
            },
        )
        .map_err(|refused| match refused {
            mod_api::__private::ResolvePartsError::ItemKeys(errors) => format!(
                "Mod item keys refused:\n{}",
                errors
                    .into_iter()
                    .map(|e| format!("mod {} requested unknown item {}", e.mod_id, e.id))
                    .collect::<Vec<_>>()
                    .join("\n")
            ),
            mod_api::__private::ResolvePartsError::Parts(errors) => {
                format!("Mod parts refused:\n{}", errors.join("\n"))
            }
            _ => "Mod parts refused".to_owned(),
        })?;
        let block_states = api_values::prepare_block_states(registry)?;
        let cauldrons = self.prepare_cauldron_interactions(items)?;
        Ok(PreparedParts {
            cauldrons,
            block_states,
            tags,
            parts,
        })
    }
    /// Reload the content tables with the world's packs, as
    /// `/reload` reloads a server's (`ReloadableServerResources`).
    pub(super) fn prepare_world_reload(
        &mut self,
        stack: &functions::PackStack,
        root: &std::path::Path,
    ) -> Result<PreparedWorldReload> {
        let (tables, health, datapack_registries) = self.stage_world_tables(stack, true, root)?;
        let items = tables
            .as_ref()
            .map_or_else(|| self.res.items(), |tables| &tables.items);
        let parts = self.prepare_parts(items)?;
        let overlay = pack_files(&tables);
        let kept = self
            .lan
            .as_ref()
            .map(|host| host.hub.lan.shared.sync())
            .or_else(lan::sync::cached_reload_sync);
        let wire = if let Some(kept) = kept {
            let sources = match tables
                .as_ref()
                .and_then(|tables| tables.reload_sources.clone())
            {
                Some(sources) => sources,
                None => {
                    lan::sync::prepare_server_sources(root, &self.res.core.item_data, &overlay)?
                }
            };
            Some(
                lan::sync::prepare_wire_reload(
                    root,
                    &self.res.core.item_data,
                    &overlay,
                    &kept,
                    &sources,
                )
                .map_err(|error| format!("wire reload: {error}"))?,
            )
        } else {
            None
        };
        let advancements = Arc::new(horizon_client::advancements::Tree::load_with(
            &root.join("data"),
            &overlay,
        ));
        overlay.conditions.check_errors()?;
        Ok(PreparedWorldReload {
            advancements,
            structure_spawns: horizon_client::mobs::spawn_selection::StructureSpawns::load(root),
            tables,
            health,
            parts,
            wire,
            datapack_registries,
        })
    }

    /// Resolve the mods' parts against the tags this game plays with (`/reload` again): mods
    /// that attach parts to a block or item it doesn't have are an error.
    pub(super) fn resolve_parts(&self) -> Result<()> {
        let prepared = self.prepare_parts(self.items())?;
        self.install_parts(prepared);
        self.resolve_enchant_particles();
        Ok(())
    }
    /// Ask the parts of each block `isPathfindable` once per state and type: the answer depends
    /// on nothing else, so path finding and spawning read it with no dispatch. A server also
    /// publishes its parts for its world generation.
    pub(super) fn install_parts(&self, prepared: PreparedParts) {
        cauldron_interactions::install(prepared.cauldrons);
        if self.is_logical_server() {
            native_worldgen_parts::publish(
                prepared.parts.as_ref(),
                &prepared.block_states,
                self.pack.assets.registry,
            );
        }
        api_values::install_block_states(prepared.block_states);
        native_world_read::install_block_tags(prepared.tags);
        mod_api::__private::install_resolved_parts(prepared.parts);
        self.resolve_block_paths();
        self.resolve_piston_reactions();
    }
    pub(super) fn resolve_block_paths(&self) {
        use horizon_client::mobs::data::PartAnswers;
        use mod_api::{__private::has_scoped_hook, HookId, PathComputationType};
        let pathfindable: &dyn Fn(BlockStateId, PathComputationType, bool) -> bool =
            &|state, kind, mut pathfindable| {
                self.native_is_pathfindable(state, kind, &mut pathfindable);
                pathfindable
            };
        self.res.mob_data().resolve_parts(
            self.res.facts(),
            &PartAnswers {
                pathfindable: has_scoped_hook(HookId::IsPathfindable).then_some(pathfindable),
            },
        );
    }
}
