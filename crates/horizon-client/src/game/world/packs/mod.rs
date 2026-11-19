//! Resource pack selection bound to the game: the pack screen lists, and a
//! full resource reload after a change, while playing.
use super::{
    Game, PackResources, Resources, UiResources, build_renderer, prepare_renderer_textures,
};
use horizon_client::{
    assets::Assets,
    datapack::DataPack,
    gfx::PreparedTextures,
    hud::HudAtlas,
    lang, pack_browser,
    pack_screen::{self, Compat, PackInfo, Source as PackSource},
    packs::{self, Pack, Position},
    streaming::WorkerContext,
    text::{Style, Text},
};
use mod_api::__private::game::*;
use mod_api::{__private::reload::ReloadFiles, FileToIdConverter, Identifier};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

/// Where overlay roots are built (beside options.txt).
pub fn cache_dir() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_default()
        .join(".horizon-cache/resourcepacks")
}
pub fn packs_dir() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_default()
        .join("resourcepacks")
}
/// `Blaze3D.openPath`: the desktop's handler on the folder (the default file manager).
pub fn open_folder(dir: &Path) {
    if let Err(e) = horizon_client::desktop::open_path(dir) {
        log::warn!("Horizon: could not open {}: {e}", dir.display());
    }
}
/// `Blaze3D.openUri`: the system browser on the link.
pub fn open_uri(uri: &str) {
    if let Err(error) = horizon_client::desktop::open_uri(uri) {
        log::warn!("Horizon: could not open URI {uri}: {error}");
    }
}
/// The resource root to load from: the selection and a server's packs, which go on top
/// (`Pack.Position.TOP`, fixed), laid over `base`. `base` is the mods' content root
/// ([`super::mods::content_root`] of `vanilla`), so mod resources sit below the player's packs
/// as in vanilla, and a pack change leaves the content root's cache alone (#12403). Built-in
/// packs are found in `vanilla`.
pub fn startup_root(
    vanilla: &Path,
    selected: &[String],
    server: &[Pack],
    mod_sources: &super::mods::ModsState,
) -> horizon_client::Result<horizon_assets::resource_source::ResourceRoot> {
    let mut all = packs::discover(vanilla, &packs_dir());
    all.extend(super::pack_finders::discover(
        mod_api::PackType::ClientResources,
        mod_sources,
    ));
    let mut enabled = packs::enabled(&all, selected);
    enabled.extend_from_slice(server);
    let base = super::mods::content_root(vanilla, mod_sources)?;
    let source = packs::resources(base.source().clone(), &enabled)?;
    Ok(horizon_assets::resource_source::ResourceRoot::new(
        base.prepared().to_path_buf(),
        cache_dir(),
        source,
    )
    .with_data(base.data().clone(), base.data_changes().clone()))
}
fn info(p: &Pack) -> PackInfo {
    if let Some(info) = &p.selection {
        return info.clone();
    }
    let built_in = !p.id.starts_with("file/");
    PackInfo {
        id: p.id.clone(),
        title: p.title.clone(),
        description: p.raw_description.clone(),
        description_style: p.description_style.clone(),
        source: if built_in {
            PackSource::BuiltIn
        } else {
            PackSource::Default
        },
        compat: p.compatibility,
        icon: p.icon_png().as_deref().and_then(pack_screen::load_icon),
        required: false,
        position: Position::Top,
        fixed: false,
        hidden: false,
    }
}
/// A pack from `DownloadedPackSource.loadRequestedPacks`: fixed at the top of
/// the selected list (`DOWNLOADED_PACK_SELECTION`), labelled as a server pack
/// (or a world's own pack when its id is the local-world uuid).
fn downloaded_info(p: &Pack) -> PackInfo {
    // `WorldOpenFlows.WORLD_PACK_ID` as hex in `server/{uuid}/…`.
    const WORLD: &str = "server/640a6a92b6cb48a0b391831586500359/";
    let world = p.id.starts_with(WORLD);
    PackInfo {
        id: p.id.clone(),
        title: p.title.clone(),
        description: p.raw_description.clone(),
        description_style: p.description_style.clone(),
        source: if world {
            PackSource::World
        } else {
            PackSource::Server
        },
        compat: p.compatibility,
        icon: p.icon_png().as_deref().and_then(pack_screen::load_icon),
        required: true,
        position: Position::Top,
        fixed: true,
        hidden: false,
    }
}
/// The vanilla pack: required, at the bottom (`BuiltInPackSource`), with the
/// client JAR's `pack.png` that the prepared resources hold.
fn vanilla_info(vanilla: &Path) -> PackInfo {
    PackInfo {
        id: "vanilla".into(),
        title: Text::translate("resourcePack.vanilla.name", &[]),
        description: Text::translate("resourcePack.vanilla.description", &[]),
        description_style: Style::default(),
        source: PackSource::BuiltIn,
        compat: Compat::Compatible,
        icon: std::fs::read(vanilla.join("pack.png"))
            .ok()
            .as_deref()
            .and_then(pack_screen::load_icon),
        required: true,
        position: Position::Bottom,
        fixed: false,
        hidden: false,
    }
}
/// Packs the Resource Packs screen lists: folder and built-in packs, then any
/// active downloaded server (or world) packs, then vanilla.
pub(super) fn pack_infos(vanilla: &Path, server: &[Pack], additional: &[Pack]) -> Vec<PackInfo> {
    let mut infos: Vec<PackInfo> = packs::discover(vanilla, &packs_dir())
        .iter()
        .map(info)
        .collect();
    infos.extend(additional.iter().map(info));
    infos.extend(server.iter().map(downloaded_info));
    infos.push(vanilla_info(vanilla));
    infos
}
/// The folder packs' Polytone config entries (#11486), by pack id, for those that have some:
/// what the Pack Options page edits.
pub(super) fn pack_configs(vanilla: &Path) -> std::collections::BTreeMap<String, packs::Configs> {
    packs::discover(vanilla, &packs_dir())
        .into_iter()
        .filter(|p| !p.polytone_configs.entries().is_empty())
        .map(|p| (p.id, p.polytone_configs))
        .collect()
}

/// The confirmation's callback, shared by the startup menu and in-game options.
pub(super) fn import_resource_packs(
    ui: &mut horizon_client::options_ui::OptionsUi,
    vanilla: &Path,
    server: &[Pack],
    paths: &[PathBuf],
    additional: &[Pack],
) {
    let dir = packs_dir();
    let mut failed = false;
    for path in paths {
        match packs::copy_pack(path, &dir) {
            Ok(()) => log::info!("Horizon: added resource pack {}", path.display()),
            Err(error) => {
                log::warn!(
                    "Horizon: could not copy resource pack {}: {error}",
                    path.display()
                );
                failed = true;
            }
        }
    }
    ui.update_pack_infos(pack_infos(vanilla, server, additional));
    ui.pack_configs = pack_configs(vanilla);
    if failed {
        ui.resource_error = Some(Text::translate("pack.copyFailure", &[]).plain());
    }
}
/// The player changed Pack Options: `chosen` replaces their choices for `entries`, kept in
/// `polytone-configs.json`.
pub(super) fn save_pack_options(
    entries: &[String],
    chosen: std::collections::BTreeMap<String, packs::ConfigValue>,
) {
    let mut all = packs::polytone_choices();
    all.retain(|id, _| !entries.contains(id));
    all.extend(chosen);
    if let Err(e) = packs::save_polytone_choices(&packs::polytone_choices_file(), &all) {
        log::warn!("could not save the pack options: {e}");
    }
    packs::set_polytone_choices(all);
}
impl Game<'_> {
    pub(super) fn drop_pack_file(&mut self, path: &Path) {
        let size = self.gui_size();
        let handled = self.screen == super::Screen::Options
            && self
                .ui
                .options
                .drop_resource_pack(path.to_path_buf(), &self.gui.atlas, size);
        if !handled
            && self.title_input_active()
            && let Some(title) = &mut self.title
        {
            title.drop_file(path.to_path_buf());
        }
    }
    pub(super) fn resource_pack_infos(&mut self) -> Vec<PackInfo> {
        let additional = self.discover_mod_packs(mod_api::PackType::ClientResources);
        pack_infos(&self.vanilla_root(), &self.server_packs.active, &additional)
    }
    pub(super) fn vanilla_root(&self) -> PathBuf {
        if self.ui.vanilla_root.as_os_str().is_empty() {
            self.options.resources.clone()
        } else {
            self.ui.vanilla_root.clone()
        }
    }
    /// Whether all active packs preserve vanilla's block models. The server
    /// overlay sits above the local selection and must be included too.
    pub(super) fn model_packs_are_vanilla(&self) -> bool {
        self.settings.horizon.vanilla_models_with_server_packs(
            self.settings.resource_packs.iter().map(String::as_str),
            self.server_packs.active.iter().map(|pack| pack.id.as_str()),
        )
    }
    /// The leaf mode that is safe for the current local and server packs.
    pub(super) fn effective_leaf_culling(&self) -> horizon_client::graphics::LeafCullingMode {
        self.settings
            .horizon
            .leaf_culling_effective_with_server_packs(
                self.settings.resource_packs.iter().map(String::as_str),
                self.server_packs.active.iter().map(|pack| pack.id.as_str()),
            )
    }
    /// The packs the pack screen offers and the current selection
    /// (`PackSelectionScreen` makes its `PackSelectionModel` from them).
    pub(super) fn refresh_pack_lists(&mut self) {
        let infos = self.resource_pack_infos();
        let configs = pack_configs(&self.vanilla_root());
        let browsing = self.settings.extras.pack_browser;
        let o = &mut self.ui.options;
        o.pack_browser = browsing;
        o.pack_infos = infos;
        o.pack_configs = configs;
        o.pack_selection = self.settings.resource_packs.clone();
        o.pack_screen = None;
    }
    /// `Minecraft.reloadResourcePacks`: rebuild the overlay and reload every
    /// client resource from it, as vanilla's reload listeners do: block
    /// models and sprites (every section is meshed again,
    /// `LevelRenderer.allChanged`), the shared atlas (GUI, item, entity,
    /// particle and font textures), item models, sounds, languages, GUI
    /// metadata and the cloud and world border textures. The loading runs on
    /// a worker under the progress overlay (`LoadingOverlay` over
    /// `ReloadableResourceManager.createReload`'s background executor), so
    /// the window keeps drawing; `drive_pack_reload` applies it.
    /// Horizon's pack browser: what its worker finished since the last
    /// frame. A pack that landed becomes an entry in the available list.
    pub(super) fn drive_pack_browser(&mut self) {
        let Some(screen) = &mut self.ui.options.pack_screen else {
            return;
        };
        let Some(browser) = &mut screen.browser else {
            return;
        };
        let Some(message) = browser.poll() else {
            return;
        };
        if let pack_browser::Message::Results(_) = &message {
            // The rows draw their icons as they land.
            browser.fetch_icons();
        }
        if let pack_browser::Message::Icon(id, png) = &message {
            let key = pack_screen::icon_texture(id);
            if let Some(icon) = pack_screen::load_fetched_icon(png) {
                // Tracked, so closing the browser frees it again.
                screen.keep_browser_icon(key, icon);
            }
            return;
        }
        if let pack_browser::Message::Downloaded(name) = message {
            log::info!("Horizon: added the resource pack {name}");
            // Rebuilding the screen would shut the list; keep it and only
            // refresh what the lists read.
            let selection = self.settings.resource_packs.clone();
            let infos = self.resource_pack_infos();
            self.ui.options.pack_infos = infos;
            self.ui.options.pack_configs = pack_configs(&self.vanilla_root());
            self.ui.options.pack_selection = selection;
            self.ui.options.pack_screen = None;
        }
    }
    pub(super) fn reload_packs(&mut self) {
        // Pack reload updates tail-owned mob metadata. Keep menus responsive
        // until those resources exist, then apply the latest selection once.
        // A reload already under its overlay finishes first (vanilla's
        // `pendingReload`, which `runTick` starts once the overlay is gone).
        if reload_waits(self.rest_loader.is_some(), self.pack_reload.is_some()) {
            self.pending_pack_reload = true;
            return;
        }
        if let Err(e) = self.spawn_pack_reload(ReloadKind::Selection) {
            self.selection_reload_failed(&format!("could not start the pack reload: {e}"));
        }
    }
    /// `reloadResourcePacks`' failure (`Minecraft.rollbackResourcePacks`): with
    /// a pack selected, all selected packs are dropped and the reload runs
    /// again, with the load-failure toast; with none left to drop, the pack
    /// screen again, with the error.
    fn selection_reload_failed(&mut self, error: &str) {
        log::warn!("Horizon: resource reload failed: {error}");
        let mut all = packs::discover(&self.vanilla_root(), &packs_dir());
        all.extend(self.discover_mod_packs(mod_api::PackType::ClientResources));
        if packs::able_to_clear_any_pack(&all, &self.settings.resource_packs) {
            // clearResourcePacksOnError.
            log::info!("Caught error loading resourcepacks, removing all selected resourcepacks");
            self.settings.resource_packs.clear();
            self.settings.save(&Self::options_path());
            if self.spawn_pack_reload(ReloadKind::Selection).is_ok() {
                // addResourcePackLoadFailToast(null), SystemToast.addOrUpdate.
                let title = horizon_client::text::Text::translate("resourcePack.load_fail", &[]);
                match &mut self.server_packs.load_fail {
                    Some(toast) => toast.reset(title, None),
                    None => {
                        self.server_packs.load_fail = self.add_system_toast(
                            horizon_client::toasts::SystemToast::new(title, None),
                        );
                    }
                }
                return;
            }
        }
        if self.screen != super::Screen::Options {
            self.open_options_at(
                self.title.is_some(),
                horizon_client::options_ui::Page::Packs,
            );
        }
        self.ui.options.resource_error = Some(error.to_string());
    }
    /// Stages the selection with the server's active packs on top on a
    /// worker (`createReload`'s background executor); the frame draws its
    /// progress meanwhile, and the overlay takes no input (`LoadingOverlay`),
    /// so what is held is released (`KeyMapping.releaseAll`).
    pub(super) fn spawn_pack_reload(&mut self, kind: ReloadKind) -> std::io::Result<()> {
        self.keys.clear();
        self.physical_keys.clear();
        self.left = false;
        self.right = false;
        let vanilla = self.vanilla_root();
        if self.ui.vanilla_root.as_os_str().is_empty() {
            self.ui.vanilla_root = vanilla.clone();
        }
        let enabled = self.applied_mods();
        // Only content-only packages are selectable on reload. Keep code packages' validated
        // startup manifests: changing their content must not install code or registries live.
        let content = horizon_mod_package::selection::content_packages(
            &Self::game_dir(),
            &enabled,
            &self.mods.dev,
        )
        .filter(|p| {
            p.package.manifest.wasm.is_none()
                && !self
                    .mods
                    .native
                    .built()
                    .iter()
                    .any(|built| built.id == p.package.manifest.id)
        });
        let code = self
            .mods
            .selected
            .iter()
            .filter(|p| p.package.manifest.wasm.is_some())
            .map(|p| horizon_mod_package::selection::SelectedPackage {
                path: p.path.clone(),
                package: horizon_mod_package::Package {
                    manifest: p.package.manifest.clone(),
                    icon: p.package.icon.clone(),
                    layouts: p.package.layouts.clone(),
                },
            });
        self.mods.selected = content.chain(code).collect();
        self.mods.refused = Default::default();
        let mod_sources = self.mods.clone();
        let (selected, server) = (
            self.settings.resource_packs.clone(),
            self.server_packs.active.clone(),
        );
        let (assets, res, context) = (
            self.pack.assets.clone(),
            self.res.clone(),
            self.pack.context.clone(),
        );
        let (unicode, japanese) = (self.settings.force_unicode, self.settings.japanese_glyphs);
        // The reload's mod listeners register here, on the thread that owns
        // the game, before the staging worker starts: each listener keeps its
        // registering mod for crash attribution.
        let client_listeners = self.register_client_reload_listeners();
        let runtime_sprites = super::client_reload::collect_runtime_sprites();
        let texture_atlases = super::client_reload::collect_texture_atlases();
        super::mods::register_fluid_models(self.options.execution_role);
        let stage = move |progress: &crate::game::Progress| {
            stage_pack(
                &vanilla,
                &selected,
                &server,
                &mod_sources,
                client_listeners,
                &runtime_sprites,
                &texture_atlases,
                &assets,
                &res,
                &context,
                unicode,
                japanese,
                progress,
            )
            .map_err(|e| e.to_string())
        };
        #[cfg(test)]
        let stage = mod_api::testing::inherit_mods(stage);
        self.pack_reload = Some(PackReload::spawn(kind, stage)?);
        Ok(())
    }
    /// `LoadingOverlay.tick` once the reload is done: the staged pack is
    /// applied on the render thread, and its outcome goes to whoever asked
    /// (`reloadResourcePacks`' callback, or `PackReloadConfig.Callbacks`).
    pub(super) fn drive_pack_reload(&mut self) {
        let Some((kind, staged)) = self.pack_reload.as_mut().and_then(PackReload::finished) else {
            return;
        };
        self.pack_reload = None;
        let result =
            staged.and_then(|staged| self.apply_staged_pack(staged).map_err(|e| e.to_string()));
        match kind {
            ReloadKind::Selection => {
                self.mods_reloaded(result.is_ok());
                if let Err(e) = result {
                    self.selection_reload_failed(&e);
                }
            }
            ReloadKind::Server => {
                if let Err(e) = &result {
                    log::warn!("Horizon: server resource pack reload failed: {e}");
                }
                self.finish_pack_reload(result.is_ok());
            }
        }
    }
    /// The render-thread half: the renderer on the new pack's textures, then
    /// the swap (`ReloadInstance`'s application listeners).
    pub(super) fn apply_staged_pack(&mut self, staged: StagedPack) -> horizon_client::Result<()> {
        let StagedPack {
            root,
            client_applies,
            gui,
            pack,
            textures,
            cem,
            block_entity_shapes,
            waypoint_styles,
            armor_stand_assets,
            frame_models,
            sounds,
            ui_metadata,
            language,
        } = staged;
        let loop_context = self.prepare_loop_context(&root)?;
        // Refresh modifiers from the newly staged packs, including removals.
        let tab_modifiers = horizon_client::polytone_creative_tabs::TabModifier::read_all(&root);
        self.creative_screen.set_pack_modifiers(tab_modifiers);
        let standalone_geometry =
            super::native_block_entity_draw::StandaloneGeometry::new(&pack, &gui);
        // Keep the working GPU resources, and hand its window surface over.
        if let Some(renderer) = &mut self.renderer {
            renderer.release_surface();
        }
        let renderer = match self
            .window
            .clone()
            .map(|window| {
                build_renderer(
                    window,
                    self.options.offscreen,
                    &pack,
                    &gui,
                    self.renderer.as_ref(),
                    Some(textures),
                    self.settings.horizon.api,
                    self.settings.horizon.translucent_batching,
                    self.settings.vsync,
                    &self.settings.horizon,
                    super::shader_packs::picked_pack_shape(&self.settings),
                    self.renderer
                        .as_ref()
                        .map(|r| r.display_reading())
                        .unwrap_or_default(),
                    (
                        self.settings.file.int("textureFiltering"),
                        self.settings.file.int("maxAnisotropyBit"),
                    ),
                )
            })
            .transpose()
        {
            Ok(renderer) => renderer,
            Err(error) => {
                if let Some(renderer) = &mut self.renderer {
                    renderer.recreate_surface().map_err(|restore| {
                        format!(
                            "{error}; could not restore the previous rendering surface: {restore}"
                        )
                    })?;
                }
                return Err(error);
            }
        };
        // Everything loaded: switch over.
        [
            self.frame_model,
            self.glow_frame_model,
            self.map_frame_model,
            self.glow_map_frame_model,
        ] = frame_models;
        self.waypoint_styles = waypoint_styles;
        self.armor_stand_assets.apply_resources(armor_stand_assets);
        self.random_tick_sections.clear();
        self.portals.sections.clear();
        // Each level keeps its own portal answers: the parked ones are stale too.
        for level in self.levels.parked.values_mut() {
            level.portal_sections.clear();
        }
        self.options.resources = root.clone();
        self.falling_vertices.get_mut().clear();
        self.pack = pack;
        let skip = self
            .settings
            .extras
            .skip_modded_splashes_if_vanilla_modified;
        self.gui = gui.with_splashes(&self.pack.source, &self.vanilla_root(), skip);
        self.standalone_geometry = standalone_geometry;
        self.instance_groups.borrow_mut().clear();
        // The new atlas's skin and map slots are blank: players and maps
        // upload into it again instead of naming sprites it does not have.
        self.skin_slots.forget();
        self.skin_downloads
            .borrow_mut()
            .set_resource_root(self.pack.source.clone());
        self.maps.slots.forget();
        self.map_patches.clear();
        self.text_patches.get_mut().clear();
        // Kept mod block entities name the old atlas's sprites, and their view distances the old
        // registry's states.
        self.kept_block_entities.get_mut().clear();
        self.live_scenes.get_mut().clear();
        self.mod_view_distances.take();
        // The atlas's sprites moved: the particle engine re-reads every rect
        // and the packs' sprite sets, or every particle draws a stale region.
        self.particles.reload(
            &self.pack.source,
            &self.gui.atlas.sprites,
            &self.gui.atlas.translucent,
        );
        horizon_client::ui_theme::reset_atlas();
        // Villager hats come from the pack's texture metadata (shared data, swapped in place).
        *self
            .res
            .mob_data()
            .villager_hats
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            horizon_client::mobs::villager_hats::VillagerHats::load(&self.pack.source);
        self.client_professions()?;
        *self
            .res
            .mob_data()
            .cem
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = cem;
        self.res
            .block_entity_render()
            .set_shapes(block_entity_shapes);
        // The random entity textures and emissive overlays the new atlas packs. A new scan is
        // a new generation:
        // every mob (every splitscreen player's too) chooses again on its next tick.
        *self
            .res
            .mob_data()
            .random_textures
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            std::mem::take(&mut self.gui.atlas.random_textures);
        *self
            .res
            .mob_data()
            .emissive
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = self.gui.atlas.emissive.clone();
        *self
            .res
            .mob_data()
            .blinks
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            std::mem::take(&mut self.gui.atlas.blinks);
        horizon_client::ui::set_metadata(ui_metadata);
        // Read with the language in use when the reload started: one picked since then wins.
        let code = lang::code();
        lang::set(if language.code == code {
            language
        } else {
            lang::Language::load(&[&self.pack.source], &code)
        });
        // Book search terms and resolved result icons belong to these resources.
        self.recipe_ui.reset_screen();
        self.refresh_remote_language();
        self.audio.reload_with(&self.pack.source, sounds);
        self.clear_lod();
        // A pack that may replace leaf models falls back to `Off` before the
        // reload marks every section dirty.
        self.streaming.leaf_culling = self.effective_leaf_culling();
        let context = self.worker_context();
        self.streaming.reload_resources(&mut self.world, context);
        self.pending_uploads.clear();
        // New GPU textures: a renderer built from the reloaded resources; the
        // sections are uploaded again as they are meshed.
        if let Some(mut renderer) = renderer {
            // The old one's size: a splitscreen player's view is smaller than the window the new
            // one was sized from (it drew a window-sized frame into the view, cut off).
            if let Some(old) = &self.renderer {
                renderer.resize(old.config.width, old.config.height);
            }
            self.renderer = Some(renderer);
            self.apply_ui_settings();
        }
        // Horizon's own shader pack is built again: its sprite tags name the old atlas's
        // layers, and a new renderer runs no pack.
        self.native_shaders = super::native_shaders::NativeShaders::picking(&std::mem::take(
            &mut self.native_shaders.picked,
        ));
        self.ui.chat.rescale(&self.gui.atlas);
        // The screen survives reload, but names and mod-collected contents may change.
        self.creative_screen.invalidate_build();
        super::client_reload::apply_client_reload(&self.mod_phase, client_applies);
        self.install_loop_context(loop_context);
        self.resolve_entity_renderers();
        super::native_special_renderers::reload();
        self.texture_atlases_stitched();
        self.native_model_baking_completed();
        self.native_client_resources_reloaded();
        Ok(())
    }
}

/// The CPU half of a pack reload (`PackReload` runs it on a worker;
/// `apply_staged_pack` does the GPU half and the swap): the overlay
/// root, the models and sprites, the menu bundle, the pack's tables and its
/// CPU mip chains.
pub(super) fn stage_pack(
    vanilla_root: &Path,
    selected: &[String],
    server: &[Pack],
    mod_sources: &super::mods::ModsState,
    client_listeners: mod_api::ClientListenerRegistry,
    runtime_sprites: &[mod_api::RuntimeSprite],
    texture_atlases: &[mod_api::TextureAtlasRegistration],
    assets: &Arc<Assets>,
    res: &Resources,
    context: &WorkerContext,
    force_unicode: bool,
    japanese: bool,
    progress: &super::Progress,
) -> horizon_client::Result<StagedPack> {
    let resource_root = startup_root(vanilla_root, selected, server, mod_sources)?;
    let resource_root = super::client_reload::stage_runtime_sprites(resource_root, runtime_sprites);
    let resource_root = super::client_reload::stage_texture_atlases(resource_root, texture_atlases);
    let root = resource_root.prepared().to_path_buf();
    let source = resource_root.source().clone();
    progress.set(1);
    // The mod listeners prepare against the merged root before the heavy
    // staging builds run: a listener that fails aborts the reload without the
    // renderer work, keeping every listener's previous assets.
    let client_applies = super::client_reload::stage_client_reload(
        &source,
        client_listeners,
        mod_api::__private::ModPhase::World,
    )
    .map_err(|e| e.to_string())?;
    // The world's models and sprites and the GUI atlas read the same root and nothing of
    // each other: about a second each, side by side.
    let mod_image = super::resource_tables::mod_image_slot();
    // The sound events, the GUI sprites' metadata and the language are read here too, not in
    // the frame that applies the reload (about 20 ms of it).
    let (render, atlas, panorama, (sounds, ui_metadata, language)) = std::thread::scope(|s| {
        let atlas = super::resource_load::named_scope(s, "pack-assets", || {
            HudAtlas::build_over_with_image(&source, &source, mod_image).map_err(|e| e.to_string())
        });
        let panorama = super::resource_load::named_scope(s, "pack-assets", || {
            horizon_client::gfx::panorama::Faces::load(&source, &source)
        });
        let small = super::resource_load::named_scope(s, "pack-assets", || {
            (
                horizon_client::sound_events::SoundRegistry::load(&source),
                horizon_client::ui::read_metadata(&source),
                lang::Language::load(&[&source], &lang::code()),
            )
        });
        (
            assets.reload(&resource_root),
            atlas.join(),
            panorama.join(),
            small
                .join()
                .unwrap_or_else(|p| std::panic::resume_unwind(p)),
        )
    });
    let render = Arc::new(render?);
    let mut atlas = atlas.unwrap_or_else(|p| std::panic::resume_unwind(p))?;
    atlas.font.uniform = force_unicode;
    atlas.font.japanese = japanese;
    let gui = UiResources {
        atlas,
        panorama: Arc::new(panorama.unwrap_or_else(|p| std::panic::resume_unwind(p))),
        // `Game::apply_staged_pack` fills it with the client's setting.
        splashes: Arc::from(""),
    };
    progress.set(2);
    let mut pack = PackResources::load(
        &resource_root,
        assets.clone(),
        render,
        res.facts(),
        context.biomes.clone(),
        context.light.clone(),
        &res.core.item_data,
    )?;
    pack.additional_data = super::pack_finders::data_packs(mod_sources).into();
    let textures = prepare_renderer_textures(&pack);
    let cem = Arc::new(horizon_client::mobs::cem::CemModels::load(
        &source,
        res.mob_types(),
    ));
    let block_entity_shapes = res.block_entity_render().shapes_for(&source);
    let waypoint_styles = horizon_client::locator_bar::WaypointStyles::load(&source);
    let armor_stand_assets = super::armor_stands::ArmorStandAssets::load(&source, None);
    let frame_models = super::frames::bake_frames(&source);
    progress.set(3);
    Ok(StagedPack {
        root,
        client_applies,
        gui,
        pack,
        textures,
        cem,
        block_entity_shapes,
        waypoint_styles,
        armor_stand_assets,
        frame_models,
        sounds,
        ui_metadata,
        language,
    })
}

/// Everything a pack reload builds on the CPU (`apply_staged_pack` finishes
/// it on the render thread).
pub(super) struct StagedPack {
    pub root: PathBuf,
    pub waypoint_styles: horizon_client::locator_bar::WaypointStyles,
    pub armor_stand_assets: super::armor_stands::ArmorStandAssets,
    pub frame_models: [super::frames::FrameModel; 4],
    /// The mod reload listeners' prepared applies, run when the rest of the
    /// staged pack is published.
    pub client_applies: Vec<super::client_reload::PreparedClientReload>,
    pub gui: UiResources,
    pub pack: PackResources,
    pub textures: PreparedTextures,
    /// The packs' custom entity models (OptiFine CEM).
    pub cem: Arc<horizon_client::mobs::cem::CemModels>,
    /// The block entities' models, the packs' custom entity models over vanilla's.
    pub block_entity_shapes: Arc<horizon_client::block_entity_render::BlockEntityShapes>,
    /// The root's sound events, GUI sprite metadata and current language.
    pub sounds: horizon_client::sound_events::SoundRegistry,
    pub ui_metadata: horizon_client::ui::UiMetadata,
    pub language: lang::Language,
}

/// `LoadingOverlay`'s stage lines for a pack reload, indexed by `progress`.
const RELOAD_LINES: [&str; 4] = [
    "Preparing resources",
    "Loading assets",
    "Loading pack tables",
    "Applying resources",
];
/// Whether a reload asked for now waits: for the world's resources, or for
/// the reload already under its overlay (`Minecraft.reloadResourcePacks`
/// hands back `pendingReload` while a `LoadingOverlay` is up).
fn reload_waits(world_loading: bool, in_flight: bool) -> bool {
    world_loading || in_flight
}
/// Who asked for a pack reload: its outcome goes back there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ReloadKind {
    /// The player's own selection (`Minecraft.reloadResourcePacks`).
    Selection,
    /// A server's packs (`DownloadedPackSource.startReload`).
    Server,
}
type StageResult = std::result::Result<StagedPack, String>;
/// A pack reload whose CPU half runs on a worker while `progress` feeds the
/// loading overlay; `Game::drive_pack_reload` applies it on the render thread.
pub(super) struct PackReload {
    pub(super) progress: super::Progress,
    kind: ReloadKind,
    handle: Option<std::thread::JoinHandle<StageResult>>,
    /// When the overlay went up. `LoadingOverlay.fadeInStart`, set on the
    /// first frame it draws.
    started: Instant,
    /// How long the overlay stays up before the reload may be applied.
    hold: Duration,
}
/// `LoadingOverlay.isReadyToFadeOut`: a reload started from inside the game
/// (`fadeIn`) is not ready while `Util.getMillis() - fadeInStart < 1000L`, so
/// a pack that stages in a few frames still shows its overlay for a second
/// instead of flashing.
pub(super) const OVERLAY_HOLD: Duration = Duration::from_millis(1000);
impl PackReload {
    /// Starts `stage` on its own thread and returns at once.
    fn spawn(
        kind: ReloadKind,
        stage: impl FnOnce(&super::Progress) -> StageResult + Send + 'static,
    ) -> std::io::Result<Self> {
        Self::spawn_holding(kind, OVERLAY_HOLD, stage)
    }
    /// `spawn` with the overlay's hold spelled out, so a test need not wait it.
    fn spawn_holding(
        kind: ReloadKind,
        hold: Duration,
        stage: impl FnOnce(&super::Progress) -> StageResult + Send + 'static,
    ) -> std::io::Result<Self> {
        let progress = super::Progress::with(&RELOAD_LINES);
        let worker = progress.clone();
        let handle = horizon_guest_stack::builder()
            .name("pack-reload".into())
            .spawn(move || stage(&worker))?;
        Ok(Self {
            progress,
            kind,
            handle: Some(handle),
            started: Instant::now(),
            hold,
        })
    }
    pub(super) fn kind(&self) -> ReloadKind {
        self.kind
    }
    /// The staged pack once the worker is done and the overlay has had its
    /// second (taken once); none while either is outstanding.
    fn finished(&mut self) -> Option<(ReloadKind, StageResult)> {
        if self.started.elapsed() < self.hold || !self.handle.as_ref()?.is_finished() {
            return None;
        }
        let staged = self
            .handle
            .take()?
            .join()
            .unwrap_or_else(|_| Err("the pack reload worker panicked".into()));
        Some((self.kind, staged))
    }
}

/// Reads one converter's directory from `packs`, later packs winning, and
/// collects the converted files: the shared read-and-log loop of both sides'
/// reloads. `top` is `"data"` or `"assets"`.
pub(super) fn reload_files(
    top: &str,
    packs: &[&DataPack],
    listener: &Identifier,
    converter: &FileToIdConverter,
) -> ReloadFiles {
    let mut selected = std::collections::BTreeMap::new();
    for pack in packs {
        for file in pack.list_under(top, &[converter.directory()]) {
            if let Some(id) = file
                .strip_prefix(&format!("{top}/"))
                .and_then(|path| converter.file_to_id(path))
            {
                selected.insert(id, (*pack, file));
            }
        }
    }
    let mut files = std::collections::BTreeMap::new();
    for (id, (pack, path)) in selected {
        // A file that vanishes or fails to read between listing and reading is one bad
        // entry, as vanilla's `scanDirectory` treats a parse failure: log it, skip it.
        match pack.read(&path) {
            Some(bytes) => {
                files.insert(id, bytes);
            }
            None => log::error!(
                "Couldn't read file '{id}' from '{}/{}' for listener {}",
                pack.id,
                path,
                listener
            ),
        }
    }
    ReloadFiles::new(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::mpsc, time::Duration};

    #[test]
    #[ignore = "daily"]
    fn fluid_models_reload_recollects_before_baking() {
        // Concurrent resource loads replace the process-wide fluid looks and models.
        if super::super::native_entity_refs::tests::run_fresh_cases(
            "game::packs::tests::fluid_models_reload_recollects_before_baking",
            "HORIZON_FLUID_RELOAD_CHILD",
            &["recollect"],
        ) {
            return;
        }
        use mod_api::{FluidModel, Hooks, ModEntry, RegisterFluidModelsEvent, RegistrationContext};
        use mod_platform::testing::with_mods;
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };

        #[derive(Default)]
        struct Models(Arc<AtomicUsize>);
        #[mod_api::horizon_mod]
        impl Hooks for Models {
            fn register_fluid_models(
                &self,
                _: &RegistrationContext,
                event: &mut RegisterFluidModelsEvent<'_>,
            ) {
                if self.0.load(Ordering::SeqCst) == 2 {
                    return;
                }
                let mut model = FluidModel::default();
                model.fluid = "reload_boundary:oil".into();
                model.still = Some(format!(
                    "reload_boundary:block/oil_{}",
                    self.0.load(Ordering::SeqCst)
                ));
                event.models.push(model);
            }
        }
        let dir = std::env::temp_dir().join(format!("horizon-fluid-reload-{}", std::process::id()));
        let fluids = dir.join("data/reload_boundary/horizon/fluid");
        std::fs::create_dir_all(&fluids).unwrap();
        std::fs::write(dir.join("horizon-mod.toml"),
                "id = \"reload_boundary\"\nname = \"Reload boundary\"\nversion = \"1\"\napi = \"0.2\"\ncapabilities = [\"registries\"]\n").unwrap();
        std::fs::write(fluids.join("oil.json"), br#"{"world":{}}"#).unwrap();
        let vanilla = PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let mut sources = super::super::mods::ModsState {
            dev: vec![dir.clone()],
            ..Default::default()
        };
        let enabled = ["reload_boundary".to_owned()].into();
        sources.select(&std::env::current_dir().unwrap(), &enabled);
        let generation = Arc::new(AtomicUsize::new(0));
        with_mods(
            vec![ModEntry::new(
                "reload_boundary",
                Box::new(Models(generation.clone())),
            )]
            .into(),
            || {
                let root = super::super::mods::content_root(&vanilla, &sources).unwrap();
                super::super::native_entity_refs::tests::with_game_at(
                    root.prepared().to_path_buf(),
                    |game| {
                        game.options.execution_role =
                            super::super::execution_role::ExecutionRole::Client;
                        game.ui.vanilla_root = vanilla;
                        game.mods = sources;
                        game.settings.mods.enabled = enabled;
                        // Drive the real entry point and join its CPU staging worker, without
                        // applying the staged textures (which would need a window/GPU).
                        let mut reload = |expected: Option<&str>| {
                            game.spawn_pack_reload(ReloadKind::Selection).unwrap();
                            let mut pending = game.pack_reload.take().unwrap();
                            let staged = pending.handle.take().unwrap().join().unwrap().unwrap();
                            let layers = &staged.pack.render.sprite_layers;
                            for id in ["reload_boundary:block/oil_0", "reload_boundary:block/oil_1"]
                            {
                                assert_eq!(layers.contains_key(id), expected == Some(id), "{id}");
                            }
                        };
                        mod_platform::__private::with_test_packages(&["reload_boundary"], || {
                            reload(Some("reload_boundary:block/oil_0"));
                            generation.store(1, Ordering::SeqCst);
                            reload(Some("reload_boundary:block/oil_1"));
                            // A provider that stops contributing must clear its old models.
                            generation.store(2, Ordering::SeqCst);
                            reload(None);
                        });
                        horizon_client::fluid_models::register(vec![]);
                        horizon_client::fluid_models::set_world_looks([]);
                        std::fs::remove_dir_all(dir).unwrap();
                    },
                );
            },
        );
    }

    fn wait(reload: &mut PackReload) -> (ReloadKind, StageResult) {
        for _ in 0..500 {
            if let Some(done) = reload.finished() {
                return done;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("the reload worker never finished");
    }

    #[test]
    #[ignore = "daily"]
    fn resource_pack_overrides_mod_and_vanilla() {
        use std::fs;
        let vanilla =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/horizon-client/26.3");
        assert!(
            vanilla.join("facts.json").is_file(),
            "prepared vanilla data required"
        );
        let dir =
            std::env::temp_dir().join(format!("horizon-pack-mod-order-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mod_dir = dir.join("mod");
        let pack_dir = dir.join("packs/override");
        let asset = "assets/order_test/textures/block/example.png";
        for root in [&mod_dir, &pack_dir] {
            fs::create_dir_all(root.join(asset).parent().unwrap()).unwrap();
        }
        fs::write(
            mod_dir.join("horizon-mod.toml"),
            "id = \"order_test\"\nname = \"Order test\"\nversion = \"1\"\napi = \"0.2\"\ncapabilities = [\"registries\"]\n",
        )
        .unwrap();
        fs::write(mod_dir.join(asset), b"mod").unwrap();
        fs::write(pack_dir.join(asset), b"pack").unwrap();
        fs::write(
            pack_dir.join("pack.mcmeta"),
            br#"{"pack":{"description":"test","pack_format":97}}"#,
        )
        .unwrap();
        {
            use std::io::Write;
            let mut archive =
                zip::ZipWriter::new(fs::File::create(dir.join("packs/override.zip")).unwrap());
            for name in ["pack.mcmeta", asset] {
                archive
                    .start_file(name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                archive
                    .write_all(&fs::read(pack_dir.join(name)).unwrap())
                    .unwrap();
            }
            archive.finish().unwrap();
            fs::remove_dir_all(&pack_dir).unwrap();
        }
        let mut sources = super::super::mods::ModsState {
            dev: vec![mod_dir],
            ..Default::default()
        };
        sources.select(&std::env::current_dir().unwrap(), &Default::default());
        let without_pack = startup_root(&vanilla, &[], &[], &sources).unwrap();
        assert_eq!(without_pack.source().read(asset).unwrap(), b"mod");
        let pack = packs::folder(&dir.join("packs"), packs::RESOURCE_FORMAT);
        assert_eq!(pack.len(), 1);
        let with_pack = startup_root(&vanilla, &[], &pack, &sources).unwrap();
        assert_eq!(with_pack.source().read(asset).unwrap(), b"pack");
        // The selected archive changes resource lookup without materializing
        // its override beside generated tables.
        assert_eq!(with_pack.prepared(), without_pack.prepared());
        assert_ne!(
            fs::read(with_pack.prepared().join(asset)).ok().as_deref(),
            Some(b"pack".as_slice())
        );
        assert!(!pack_dir.exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_reload_stages_on_a_worker_while_the_caller_goes_on() {
        // The stage blocks until the caller lets it go: `spawn` returning at
        // all means the window thread is free to draw the overlay.
        let (release, blocked) = mpsc::channel::<()>();
        let (thread, ran_on) = mpsc::channel();
        let mut reload = PackReload::spawn(ReloadKind::Selection, move |progress| {
            blocked.recv().unwrap();
            progress.set(2);
            thread
                .send(std::thread::current().name().map(str::to_owned))
                .unwrap();
            Err("staged".into())
        })
        .unwrap();
        assert!(reload.finished().is_none(), "still staging");
        assert_eq!(reload.progress.line(), "Preparing resources");
        release.send(()).unwrap();
        let (kind, staged) = wait(&mut reload);
        assert_eq!(kind, ReloadKind::Selection);
        assert_eq!(staged.err().as_deref(), Some("staged"));
        assert_eq!(reload.progress.line(), "Loading pack tables");
        assert_eq!(ran_on.recv().unwrap().as_deref(), Some("pack-reload"));
        assert!(reload.finished().is_none(), "the outcome is taken once");
    }

    #[test]
    fn a_panicking_stage_fails_the_reload() {
        let mut reload = PackReload::spawn(ReloadKind::Server, |_| panic!("bad pack")).unwrap();
        let (kind, staged) = wait(&mut reload);
        assert_eq!(kind, ReloadKind::Server);
        assert_eq!(
            staged.err().as_deref(),
            Some("the pack reload worker panicked")
        );
    }

    /// `LoadingOverlay.isReadyToFadeOut`: a reload that stages at once still
    /// keeps its overlay up - the 1000 ms `fadeInStart` gate - so a small pack
    /// does not flash the Mojang logo for a frame or two.
    #[test]
    fn a_fast_reload_keeps_the_overlay_up_before_it_applies() {
        assert_eq!(OVERLAY_HOLD, Duration::from_millis(1000));
        let mut quick = PackReload::spawn_holding(ReloadKind::Selection, Duration::ZERO, |_| {
            Err("staged".into())
        })
        .unwrap();
        assert_eq!(wait(&mut quick).0, ReloadKind::Selection);
        // A hold no test machine outlasts, however loaded: the worker is done long
        // before the overlay has had its time.
        let hold = Duration::from_secs(3600);
        let mut held =
            PackReload::spawn_holding(ReloadKind::Selection, hold, |_| Err("staged".into()))
                .unwrap();
        for _ in 0..500 {
            if held.handle.as_ref().is_some_and(|h| h.is_finished()) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(held.handle.as_ref().is_some_and(|h| h.is_finished()));
        assert!(held.finished().is_none(), "the overlay is still up");
        // Once the hold has passed, the staged pack is handed over.
        held.hold = Duration::ZERO;
        assert_eq!(wait(&mut held).0, ReloadKind::Selection);
    }

    #[test]
    fn a_reload_waits_for_world_resources_and_the_one_in_flight() {
        assert!(!reload_waits(false, false));
        assert!(reload_waits(true, false));
        assert!(reload_waits(false, true));
        assert!(reload_waits(true, true));
    }
}
