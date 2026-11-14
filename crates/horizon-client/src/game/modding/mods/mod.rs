// game-module-visibility: pub(crate)
//! Mods in the game (`docs/modding/src/how-to/content-mods.md`): content mods' blocks, items, data and
//! assets (`content_root`), and the Mods page's rows. Code mods are native (`native_hooks.rs`)
//! or wasm (`horizon-mod-wasm`, loaded at startup); any other executable file is refused.
use super::{Game, execution_role::ExecutionRole, registration_capability_coverage as coverage};
mod menu_books;
mod standalone;
use horizon_client::{
    lang,
    mod_list::{self, Installed},
};
use horizon_mod_package::manifest::{MissingJar, collect_jar_requirements, missing_jars};
use horizon_world::{
    item_defaults::ItemData,
    native_load_context::{LoadContext, LoadInputs},
};
use mod_api::__private::game::*;
use mod_api::RegistrationContext;
use standalone::collect_standalone_models;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

/// Read filesystem-bound package declarations concurrently, retaining dependency order and
/// keeping an individual package's refusal attached to that package.
fn read_contents(
    requests: &[(&Path, &horizon_mod_package::Manifest, Vec<String>)],
) -> Vec<Result<horizon_mod_package::Content, String>> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let next = AtomicUsize::new(0);
    let threads = horizon_threads::worker_count().clamp(1, requests.len().max(1));
    let mut done: Vec<_> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                scope.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        let Some((path, manifest, declared)) = requests.get(index) else {
                            return done;
                        };
                        let content =
                            horizon_mod_package::Content::read_declaring(path, manifest, declared);
                        if declared.is_empty()
                            && let Ok(content) = &content
                        {
                            remember_verdict(path, content);
                        }
                        done.push((index, content));
                    }
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|worker| {
                worker
                    .join()
                    .unwrap_or_else(|e| std::panic::resume_unwind(e))
            })
            .collect()
    });
    done.sort_by_key(|(index, _)| *index);
    done.into_iter().map(|(_, result)| result).collect()
}

#[derive(Default)]
pub(super) struct CodeSwitches {
    disabled: std::collections::BTreeSet<String>,
    applied: std::sync::atomic::AtomicBool,
}

#[derive(Clone, Default)]
pub(super) struct ModsState {
    pub(super) active_condition_root: Option<Arc<()>>,
    pub(super) load_condition_candidate: Option<horizon_world::conditions::LoadConditions>,
    pub(super) load_conditions:
        Arc<std::sync::OnceLock<Result<crate::game::load_conditions::ConditionTable, String>>>,
    pub(super) profession_declarations:
        Arc<std::sync::OnceLock<Vec<(String, mod_api::ProfessionDefinition)>>>,
    /// Saved switches gate the first data inventory while registration keeps
    /// its existing startup ordering. Runtime dispatch owns later switches.
    pub(super) code_switches: Option<Arc<CodeSwitches>>,
    /// The original prepared input is fixed for this engine load, shared by
    /// the window and its server. A later data reload must not overlay an
    /// already generated root recursively.
    pub(super) content_base: Arc<std::sync::OnceLock<PathBuf>>,
    /// `--dev-mod PATH`: content mods always on.
    pub(super) dev: Vec<PathBuf>,
    pub(super) native: std::sync::Arc<horizon_mod_package::runtime::Runtime>,
    pub(super) installation_side: horizon_mod_package::manifest::Side,
    pub(super) overrides_dir: PathBuf,
    pub(super) load_notices: Arc<Mutex<Vec<horizon_mod_package::dependencies::Notice>>>,
    pub(super) selected: std::sync::Arc<[horizon_mod_package::selection::SelectedPackage]>,
    /// The content mods the loaded resources were built with: a switch the
    /// game can apply live moves it to the player's choice (`switch_mods`).
    pub(super) applied: Option<std::collections::BTreeSet<String>>,
    /// A live switch's resource reload is running: when it applied, the server
    /// reloads its data from the new root. Its start, for the log.
    pub(super) live: Option<std::time::Instant>,
    /// The content mods installed, and those of them with only assets and
    /// data (`mod_content::live_switch`): they switch with a world open.
    pub(super) content_ids: std::collections::BTreeSet<String>,
    pub(super) live_ids: std::collections::BTreeSet<String>,
    /// The server's data comes from a root a live switch built, not the one
    /// `res` was loaded from: the world tables are built from it even with no
    /// data pack (`stage_world_tables`).
    pub(super) data_switched: bool,
    /// Structured load refusals, rendered only at the existing Mods-page boundary.
    pub(super) refused: Arc<Mutex<Vec<MissingJar>>>,
    /// The entries of the mods' datapack registries this world session has published, at the
    /// generation of the load or reload that built them; `None` when no mod declared one.
    pub(super) datapack_registries: Option<Arc<mod_api::DatapackRegistrySet>>,
}

impl ModsState {
    pub(super) fn capture_code_switches(&mut self, disabled: &std::collections::BTreeSet<String>) {
        self.code_switches = Some(Arc::new(CodeSwitches {
            disabled: disabled.clone(),
            applied: Default::default(),
        }));
    }

    fn apply_code_switches(&self, disabled: &std::collections::BTreeSet<String>) {
        for id in mod_api::__private::installed_code_ids() {
            mod_api::__private::set_mod_enabled(id, !disabled.contains(id));
        }
        if let Some(switches) = &self.code_switches {
            switches
                .applied
                .store(true, std::sync::atomic::Ordering::Release);
        }
    }

    pub(super) fn code_owner_enabled(&self, owner: &str) -> bool {
        let initially_disabled = self.code_switches.as_ref().is_some_and(|switches| {
            !switches.applied.load(std::sync::atomic::Ordering::Acquire)
                && switches.disabled.contains(owner)
        });
        !initially_disabled
            && mod_api::__private::mod_index(owner).is_some_and(mod_api::__private::mod_index_on)
    }

    /// One admission rule for initial composition and later code-only toggles.
    /// Content acceptance belongs to the generation; code admission belongs to
    /// the installed dispatch entries, including their live enabled switches.
    fn enabled_inventory(
        &self,
        accepted_content: impl Fn(&str) -> bool,
    ) -> horizon_mod_package::enabled::EnabledMods {
        let native = self
            .native
            .packages()
            .iter()
            .map(|package| (&package.manifest, true));
        let selected = self.selected.iter().map(|selected| {
            (
                &selected.package.manifest,
                selected.package.manifest.wasm.is_some(),
            )
        });
        horizon_mod_package::enabled::EnabledMods::new(native.chain(selected).filter_map(
            |(manifest, code)| {
                let enabled = if code {
                    self.code_owner_enabled(&manifest.id)
                } else {
                    accepted_content(&manifest.id)
                };
                enabled.then(|| (manifest.id.clone(), manifest.side))
            },
        ))
    }

    pub(super) fn select(
        &mut self,
        game_dir: &std::path::Path,
        enabled: &std::collections::BTreeSet<String>,
    ) {
        self.selected =
            horizon_mod_package::selection::content_packages(game_dir, enabled, &self.dev)
                .filter(|selected| {
                    let id = &selected.package.manifest.id;
                    // A refused wasm mod's content stays out with its code.
                    !self.native.built().iter().any(|built| built.id == *id)
                        && !crate::entry::code_startup_refused(id)
                })
                .collect();
        self.refused = Default::default();
    }

    fn report_refusals(&self, rows: &mut [Installed]) {
        let refused = self.refused.lock().unwrap();
        for row in rows {
            let problems: Vec<_> = refused
                .iter()
                .filter(|missing| missing.owners.contains(&row.id))
                .map(ToString::to_string)
                .collect();
            if !problems.is_empty() {
                row.problem = Some(problems.join("\n"));
            }
        }
    }
}

/// Capture load inputs at the resource-generation boundary, before workers share them.
pub(super) fn load_item_data(root: &Path, role: ExecutionRole) -> horizon_client::Result<ItemData> {
    let phase = mod_api::__private::PhaseOwner::scope(if role.is_client() {
        mod_api::__private::ModPhase::Setup
    } else {
        mod_api::__private::ModPhase::World
    });
    let _reload = phase.reloading()?;
    let inputs = LoadInputs::new(builtin_registry_names(root)?, Game::mod_data_root());
    let inventory = horizon_mod_package::enabled::EnabledMods::read(root)?;
    let ids: Vec<_> = if role.is_logical_server() {
        inventory.server_ids().collect()
    } else {
        inventory.client_ids().collect()
    };
    let context = LoadContext::new(role.data_load_origin(), Arc::new(inputs), ids);

    Ok(ItemData::load(root, context)?)
}

/// The built-in registries' names in ID order, with Horizon's own data components.
fn builtin_registry_names(root: &Path) -> horizon_client::Result<BTreeMap<String, Vec<String>>> {
    let mut registries: BTreeMap<_, _> = builtin_registries(root)?
        .registries
        .into_iter()
        .map(|(name, ids)| (name, ids.names))
        .collect();
    if let Some(types) = registries.get_mut("minecraft:data_component_type") {
        types.extend(horizon_protocol::item::custom::entries().map(|(_, name)| name.to_owned()));
    }
    Ok(registries)
}

/// Capture registration's stable registries, settings and stock capability type owners.
pub(super) fn install_registration_data(root: &Path) -> horizon_client::Result<()> {
    let packages = mod_api::__private::loaded_mods();
    let directory = Game::game_dir();
    #[cfg(unix)]
    let directory = {
        use std::os::unix::ffi::OsStrExt;
        mod_api::InstanceDirectory::Unix(directory.as_os_str().as_bytes().to_vec())
    };
    #[cfg(windows)]
    let directory = {
        use std::os::windows::ffi::OsStrExt;
        mod_api::InstanceDirectory::Windows(directory.as_os_str().encode_wide().collect())
    };
    (horizon_engine_slots::platform()?.install_registration_data)(
        mod_api::__private::RegistrationData::new(
            builtin_registry_names(root)?,
            horizon_world::native_load_context::capture_settings(
                &Game::mod_data_root(),
                packages.ids(),
            ),
            directory,
            crate::entry::runtime_environment::get().clone(),
        )
        .with_capability_coverage(coverage::builtin, coverage::block_entity),
    );
    Ok(())
}

fn builtin_registries(root: &Path) -> horizon_client::Result<horizon_protocol::registry::Builtin> {
    let path = horizon_protocol::registry::Builtin::find(root).ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "registries.json not found")
    })?;
    Ok(horizon_protocol::registry::Builtin::load(&path)?)
}

/// Freeze native component IDs before reading content or opening a connection.
/// With no mod registering any, nothing is installed, which every reader
/// treats as no custom components, and `registries.json` isn't read.
pub(super) fn register_components(root: &std::path::Path) -> horizon_client::Result<()> {
    let mut names = Vec::new();
    Game::native_component_types(&RegistrationContext::new(), &mut names);
    if names.is_empty() {
        return Ok(());
    }
    let builtin = builtin_registries(root)?;
    let types = builtin
        .get("minecraft:data_component_type")
        .ok_or("missing data component registry")?;
    let first = i32::try_from(types.names.len())?;
    horizon_protocol::item::custom::install(first, names)
        .map_err(|error| format!("invalid native component registration: {error}"))?;
    Ok(())
}

/// Finish every once-only content declaration on the registration thread.
pub(super) fn freeze_declarations() -> horizon_client::Result<()> {
    let declared = native_declared();
    if !declared.errors.is_empty() {
        return Err(declared.errors.join("\n").into());
    }
    Ok(())
}

struct NativeDeclared {
    errors: Vec<String>,
    blocks: Vec<horizon_mod_package::content::Block>,
    items: Vec<horizon_mod_package::content::Item>,
    types: Vec<horizon_mod_package::content::BlockEntityType>,
    fluids: Vec<horizon_mod_package::content::FluidLook>,
}

/// The same native declarations serve startup and every pack reload.
fn native_declared() -> &'static NativeDeclared {
    static DECLARED: std::sync::OnceLock<NativeDeclared> = std::sync::OnceLock::new();
    DECLARED.get_or_init(|| {
        let (mut blocks, mut items) = (Vec::new(), Vec::new());
        let context = RegistrationContext::new();
        Game::native_blocks(&context, &mut blocks);
        Game::native_items(&context, &mut items);
        let mut errors = mod_api::__private::take_declaration_refusals(&context);
        let types = super::api_values::block_entity_declarations();
        use super::api_values::TryIntoEngine;
        let mut blocks: Vec<horizon_mod_package::content::Block> = blocks
            .iter()
            .filter_map(|v| {
                v.try_into_engine(&())
                    .map_err(|e| {
                        errors.push(format!(
                            "native blocks and items of {}: {e}",
                            v.id.split(':').next().unwrap_or("")
                        ))
                    })
                    .ok()
            })
            .collect();
        let mut items: Vec<horizon_mod_package::content::Item> = items
            .iter()
            .filter_map(|v| {
                v.try_into_engine(&())
                    .map_err(|e| {
                        errors.push(format!(
                            "native blocks and items of {}: {e}",
                            v.id.split(':').next().unwrap_or("")
                        ))
                    })
                    .ok()
            })
            .collect();
        let fluids: Vec<_> = mod_api::__private::declared_world_fluids()
            .iter()
            .map(|draft| draft.try_into_engine(&()).unwrap())
            .collect();
        blocks.extend(fluids.iter().filter_map(|f| f.world_block()));
        items.extend(fluids.iter().filter_map(|f| f.world_bucket()));
        NativeDeclared {
            errors,
            blocks,
            items,
            types,
            fluids,
        }
    })
}
/// The native mods' standalone models (`native_standalone_models`), set once
/// before the resources first load.
pub(super) fn register_standalone_models(role: ExecutionRole) {
    if let Some(names) = collect_standalone_models(role) {
        horizon_client::models::register_standalone(names);
    }
    register_fluid_models(role);
}

/// Recollect before each resource bake, retaining registration-phase attribution.
pub(super) fn register_fluid_models(role: ExecutionRole) {
    if let Some(models) = collect_fluid_models(role) {
        horizon_client::fluid_models::register(models);
    }
}

fn collect_fluid_models(
    role: ExecutionRole,
) -> Option<Vec<horizon_mod_package::content::FluidModel>> {
    if !role.is_client() {
        return None;
    }
    let mut models = Vec::new();
    Game::native_register_fluid_models(&RegistrationContext::new(), &mut models);
    Some(
        models
            .into_iter()
            .map(|model| horizon_mod_package::content::FluidModel {
                fluid: model.fluid,
                still: model.still,
                flowing: model.flowing,
                overlay: model.overlay,
            })
            .collect(),
    )
}

pub(super) fn collect_key_mappings<'declared>(
    role: ExecutionRole,
    declared: &'declared [(&'static str, mod_api::KeyRegistration)],
) -> &'declared [(&'static str, mod_api::KeyRegistration)] {
    if role.is_client() { declared } else { &[] }
}

/// The native mods' entity model layers (`native_register_layer_definitions`), kept
/// once at start for `bake_layer`, then their skull types (`native_create_skull_models`), baked
/// once from those layers (or vanilla's under `vanilla_root`), so `minecraft:head` item models may
/// name their kinds.
pub(super) fn register_layer_definitions(role: ExecutionRole, vanilla_root: &std::path::Path) {
    if !role.is_client() {
        return;
    }
    let mut layers = Vec::new();
    Game::native_register_layer_definitions(&RegistrationContext::new(), &mut layers);
    horizon_client::mobs::model_layers::register(layers);
    let mut models = Vec::new();
    Game::native_create_skull_models(&RegistrationContext::new(), &mut models);
    horizon_client::skull_models::register(models, vanilla_root);
}
/// The native mods' F3 debug entries (`native_register_debug_entries`), kept once at start.
pub(super) fn register_debug_entries(role: ExecutionRole) {
    if !role.is_client() {
        return;
    }
    let mut entries = Vec::new();
    Game::native_register_debug_entries(&RegistrationContext::new(), &mut entries);
    horizon_client::debug_entries::register(entries);
}
/// The native mods' map decoration types (`native_map_decoration_types`), frozen once at start on
/// every side.
pub(super) fn register_map_decoration_types() {
    let mut types = Vec::new();
    Game::native_map_decoration_types(&RegistrationContext::new(), &mut types);
    horizon_client::map_decoration_types::register(types, Game::is_vanilla_decoration_sprite);
}
/// The native mods' HUD layers (`native_register_gui_layers`), placed among vanilla's once at
/// start.
pub(super) fn register_gui_layers(role: ExecutionRole) {
    if !role.is_client() {
        return;
    }
    let mut layers = Vec::new();
    Game::native_register_gui_layers(&RegistrationContext::new(), &mut layers);
    horizon_client::gui_layers::register(layers);
}
/// The native mods' debug renderers now (`native_register_debug_renderers`), asked at each
/// refresh of the list (`Game::refresh_debug_renderers`).
pub(super) fn debug_renderers(role: ExecutionRole) -> Vec<mod_api::DebugRenderer> {
    let mut renderers = Vec::new();
    if role.is_client() {
        Game::native_register_debug_renderers(&RegistrationContext::new(), &mut renderers);
    }
    renderers
}
/// The native mods' numeric item model properties
/// (`native_register_range_select_item_model_property`), kept once at start, before
/// resources load, so `range_dispatch` models on them read the mod's number.
pub(super) fn register_range_select_item_model_properties(role: ExecutionRole) {
    if !role.is_client() {
        return;
    }
    let mut properties = Vec::new();
    Game::native_register_range_select_item_model_property(
        &RegistrationContext::new(),
        &mut properties,
    );
    horizon_assets::item_model::register_range_properties(properties);
}
/// The native mods' categorical item model properties
/// (`native_register_select_item_model_property`), kept once at start, before resources
/// load, so `select` models on them read the mod's value.
pub(super) fn register_select_item_model_properties(role: ExecutionRole) {
    if !role.is_client() {
        return;
    }
    let mut properties = Vec::new();
    Game::native_register_select_item_model_property(&RegistrationContext::new(), &mut properties);
    horizon_assets::item_model::register_select_properties(properties);
}
/// The types native mods register for resource definitions, kept once at start, before
/// resources load: item tint sources (`native_register_item_tint_sources`), so `tints` entries on
/// them read the mod's colour; atlas sprite sources (`native_register_sprite_sources`), so atlas
/// definitions' entries on them list the mod's sprites; model loaders
/// (`native_register_model_loaders`), so models naming one get the mod's quads; item model node
/// types (`native_register_item_models`), so item definitions' nodes of them draw what the mod
/// answers; and animation targets and interpolations (`native_register_json_animation_types`), so
/// animation files may name them.
/// Block-state
/// definition types (`native_register_block_state_models`), so blockstate files naming one get
/// the models the mod picks; and special item model renderers
/// (`native_register_special_model_renderer`), so special models of their types draw what the
/// mod answers.
pub(super) fn register_resource_sources(role: ExecutionRole) {
    if !role.is_client() {
        return;
    }
    let mut sources = Vec::new();
    Game::native_register_item_tint_sources(&RegistrationContext::new(), &mut sources);
    horizon_assets::item_model::register_tint_sources(sources);
    let mut sources = Vec::new();
    Game::native_register_sprite_sources(&RegistrationContext::new(), &mut sources);
    horizon_client_render::mod_sprite_sources::register(sources);
    let mut loaders = Vec::new();
    Game::native_register_model_loaders(&RegistrationContext::new(), &mut loaders);
    horizon_client_render::mod_model_loaders::register(loaders);
    let mut models = Vec::new();
    Game::native_register_item_models(&RegistrationContext::new(), &mut models);
    horizon_assets::item_model::register_item_models(models);
    let mut types = Vec::new();
    Game::native_register_json_animation_types(&RegistrationContext::new(), &mut types);
    horizon_client_render::mod_animations::register(types);
    let mut definitions = Vec::new();
    Game::native_register_block_state_models(&RegistrationContext::new(), &mut definitions);
    horizon_client_render::mod_block_state_models::register(definitions);
    register_special_model_renderers();
}
/// The native mods' special item model renderers (`native_register_special_model_renderer`),
/// kept once at start, before resources load: their types are what a `minecraft:special` model
/// may name besides vanilla's, or in place of one. A later registration of a type replaces the
/// earlier one, and the log names both mods.
fn register_special_model_renderers() {
    let mut registry = mod_api::SpecialModelRendererRegistry::default();
    Game::native_register_special_model_renderer(&RegistrationContext::new(), &mut registry);
    for (kind, owner) in registry.initial_owners() {
        if horizon_assets::item_model::SpecialModel::is_builtin_type(kind) {
            log::warn!(
                "Horizon: special model type {kind} registered by the game is replaced by {}'s registration",
                owner.unwrap_or("the game")
            );
        }
    }
    let (mut renderers, replaced) = registry.into_renderers();
    for replaced in replaced {
        let name = |id: Option<&str>| id.unwrap_or("the game").to_owned();
        log::warn!(
            "Horizon: special model type {} registered by {} is replaced by {}'s registration",
            replaced.kind,
            name(replaced.earlier),
            name(replaced.later)
        );
    }
    // In slot order: the types' models resolve to these slots as they decode.
    let kept = horizon_assets::item_model::register_special_model_types(
        renderers.keys().cloned().collect(),
    );
    super::native_special_renderers::register(
        kept.into_iter()
            .filter_map(|kind| renderers.remove(&kind).map(|renderer| (kind, renderer)))
            .collect(),
    );
}
/// The client presentation registrations, kept once at start, before resources load: the
/// dimension transition screens, the entity spectator shaders, the render pipelines and the entity
/// renderers.
pub(super) fn register_client_presentations(role: ExecutionRole) {
    register_preset_editors(role);
    register_dimension_transition_screens(role);
    register_entity_spectator_shaders(role);
    register_render_pipelines(role);
    register_entity_renderers(role);
}

/// Resolve client dimension-transition registrations once, before the first level opens.
fn register_dimension_transition_screens(role: ExecutionRole) {
    if !role.is_client() {
        return;
    }
    let mut screens = Vec::new();
    Game::native_register_dimension_transition_screen(&RegistrationContext::new(), &mut screens);
    horizon_client_render::dimension_transition::register(screens);
}

/// Resolve client spectator post-effect registrations once, before the first level opens.
pub(super) fn register_entity_spectator_shaders(role: ExecutionRole) {
    if !role.is_client() {
        return;
    }
    let mut shaders = Vec::new();
    Game::native_register_entity_spectator_shaders(&RegistrationContext::new(), &mut shaders);
    horizon_client_render::spectator_shaders::register(shaders);
}

pub(super) fn register_preset_editors(role: ExecutionRole) {
    if !role.is_client() {
        return;
    }
    let mut editors = Vec::new();
    Game::native_register_preset_editors(&RegistrationContext::new(), &mut editors);
    horizon_client::title::preset_editors::register(editors);
}

/// Keep the mods' render pipeline registrations once at startup, before resources load: every
/// load builds them from its own shader files.
pub(super) fn register_render_pipelines(role: ExecutionRole) {
    if !role.is_client() {
        return;
    }
    let mut pipelines = Vec::new();
    Game::native_register_render_pipelines(&RegistrationContext::new(), &mut pipelines);
    horizon_client_render::mod_pipelines::register(pipelines);
}

/// The native mods' meshing registrations, kept once at start, before resources load: colour
/// resolvers, then the block tint sources that read them, then section geometry renderers.
pub(super) fn register_meshing(role: ExecutionRole) {
    register_color_resolvers(role);
    register_block_tint_sources(role);
    add_section_geometry(role);
}

/// The native mods' colour resolvers (`native_register_color_resolvers`), kept once at start,
/// before the block tint sources that read them and before resources load.
fn register_color_resolvers(role: ExecutionRole) {
    if !role.is_client() {
        return;
    }
    let mut registry = mod_api::ColorResolverRegistry::default();
    Game::native_register_color_resolvers(&RegistrationContext::new(), &mut registry);
    horizon_client::mesher::register_color_resolvers(registry.into_resolvers());
}

/// The native mods' block tint sources (`native_register_block_tint_sources`), kept once at
/// start, before resources load; every resource load applies them to its mesh tables.
fn register_block_tint_sources(role: ExecutionRole) {
    if !role.is_client() {
        return;
    }
    let mut registry = mod_api::BlockTintRegistry::default();
    Game::native_register_block_tint_sources(&RegistrationContext::new(), &mut registry);
    horizon_client::mesher::register_block_tint_sources(registry.into_layers());
}
/// The native mods' section geometry renderers (`native_add_section_geometry`), kept once at
/// start, before resources load; every mesh of a section asks them to add their quads.
fn add_section_geometry(role: ExecutionRole) {
    if !role.is_client() {
        return;
    }
    let mut registry = mod_api::SectionGeometryRegistry::default();
    Game::native_add_section_geometry(&RegistrationContext::new(), &mut registry);
    horizon_client::mesher::register_section_geometry(registry.into_renderers());
}
/// The native mods' entity renderers (`native_register_entity_renderers`), kept once at start,
/// before resources load; every resource load resolves each type's renderer.
pub(super) fn register_entity_renderers(role: ExecutionRole) {
    if !role.is_client() {
        return;
    }
    let mut registry = mod_api::EntityRendererRegistry::default();
    Game::native_register_entity_renderers(&RegistrationContext::new(), &mut registry);
    super::native_entity_renderers::register(registry.into_renderers());
}
/// The native mods' particle providers (`native_register_particle_providers`) and render groups
/// (`native_register_particle_groups`), kept once at start, before resources load; every particle
/// sprite-set load resolves each type's provider and group.
pub(super) fn register_particles(role: ExecutionRole) {
    if !role.is_client() {
        return;
    }
    let mut registry = mod_api::ParticleProviderRegistry::default();
    Game::native_register_particle_providers(&RegistrationContext::new(), &mut registry);
    horizon_client::particles::register_particle_providers(registry.into_providers());
    let mut groups = Vec::new();
    Game::native_register_particle_groups(&RegistrationContext::new(), &mut groups);
    horizon_client::particles::register_particle_groups(groups);
}

/// `root` with the content mods that are on layered over it
/// (docs/modpacks.md): their blocks, items, data and assets. Read once at
/// start, before the resources load; a mod whose content is refused is left
/// out here and its row on the Mods page says why.
/// The merged content root and the accepted content mods behind it, before
/// any process-wide registration. A caller that only reads the overlay (the
/// reload tests) must not freeze the mod registries for the rest of the
/// process.
pub(super) struct ContentOverlay {
    /// The root with the mods' content merged in.
    pub(super) root: horizon_assets::resource_source::ResourceRoot,
    /// The accepted content mods, in overlay order.
    pub(super) mods: Vec<horizon_client::mod_content::ContentMod>,
    /// The protocol entries of the accepted mods.
    pub(super) announced: Vec<horizon_protocol::mod_list::Entry>,
    /// Whether the protocol's local mod list is still unset.
    pub(super) announce: bool,
    /// The accepted content-only mods, each with its version, for the worlds saved to record.
    pub(super) content_mods: Vec<horizon_client::save::level_mods::ModRecord>,
}

/// Builds the content overlay: every enabled or dev content mod's files
/// merged over `root`. Registration of what the mods declare is
/// [`content_root`]'s step.
pub(super) fn content_overlay(
    root: &std::path::Path,
    sources: &ModsState,
    jar_store: &std::path::Path,
) -> horizon_client::Result<ContentOverlay> {
    use horizon_client::mod_content::{self, ContentMod};
    let here = std::env::current_dir().unwrap_or_default();
    let original_base = root.to_owned();

    let requirements = collect_jar_requirements(
        sources
            .native
            .packages()
            .iter()
            .map(|p| &p.manifest)
            .chain(sources.selected.iter().map(|p| &p.package.manifest))
            .map(|m| (m.id.as_str(), m.jar.as_slice())),
    )?;
    let refused = missing_jars(jar_store, &requirements)?;
    *sources.refused.lock().unwrap() = refused.clone();
    if let Some(missing) = refused.iter().find(|missing| {
        missing
            .owners
            .iter()
            .any(|id| sources.native.built().iter().any(|built| built.id == id))
    }) {
        return Err(missing.clone().into());
    }

    let mut known = None;
    let mut mods: Vec<ContentMod> = Vec::new();
    let mut content_mods = Vec::new();
    let announce = !horizon_protocol::mod_list::is_set();
    let mut announced: Vec<_> = if announce {
        sources
            .native
            .built()
            .iter()
            .map(|m| announced_entry(m.id, m.version, m.side))
            .collect()
    } else {
        Vec::new()
    };
    struct PackageSource<'a> {
        path: &'a std::path::Path,
        manifest: &'a horizon_mod_package::Manifest,
        content_only: bool,
    }
    let native = sources.native.packages().iter().map(|p| PackageSource {
        path: &p.path,
        manifest: &p.manifest,
        content_only: false,
    });
    let content = sources.selected.iter().map(|p| PackageSource {
        path: &p.path,
        manifest: &p.package.manifest,
        content_only: true,
    });
    let mut seen = std::collections::HashSet::new();
    let packages: Vec<_> = native
        .chain(content)
        .filter(|source| {
            !refused
                .iter()
                .any(|missing| missing.owners.contains(&source.manifest.id))
        })
        .filter(|source| {
            let code = source.manifest.native.is_some() || source.manifest.wasm.is_some();
            !code || seen.insert(source.manifest.id.as_str())
        })
        .collect();
    let manifests: Vec<_> = packages.iter().map(|source| source.manifest).collect();
    let overrides = horizon_mod_package::dependencies::Overrides::read(&sources.overrides_dir)?;
    let resolution = horizon_mod_package::dependencies::resolve_manifests(
        &manifests,
        sources.installation_side,
        &overrides,
    )
    .map_err(|error| error.to_string())?;
    let order = resolution.order;
    // Hooks are queried on the calling thread, never from a content reader.
    let native_items = &native_declared().items;
    let requests: Vec<_> = order
        .iter()
        .map(|&index| {
            let source = &packages[index];
            let declared =
                super::native_content_items::owned_item_ids(source.manifest, native_items);
            (source.path, source.manifest, declared)
        })
        .collect();
    let read = read_contents(&requests);
    for (index, content) in order.into_iter().zip(read) {
        let source = &packages[index];
        let m = source.manifest;
        let content = match content {
            Ok(content) => content,
            Err(e) => {
                log::warn!("Horizon: mod {} content not loaded: {e}", m.id);
                continue;
            }
        };
        let known = match &mut known {
            Some(types) => types,
            None => match mod_content::known_components(root) {
                Ok(types) => known.insert(types),
                Err(e) => {
                    log::warn!("Horizon: content mods not loaded: {e}");
                    return Err(e);
                }
            },
        };
        let candidate = ContentMod {
            id: m.id.clone(),
            content,
            jars: m.jar.clone(),
        };
        match mod_content::accept_mod(&mut mods, candidate, known) {
            Ok(_) if source.content_only => {
                content_mods.push(horizon_client::save::level_mods::ModRecord {
                    id: m.id.clone(),
                    version: m.version.clone(),
                });
                if announce {
                    announced.push(announced_entry(&m.id, &m.version, m.side));
                }
            }
            Ok(_) => {}
            Err(e) => log::warn!("Horizon: mod {} content not loaded: {e}", m.id),
        }
    }
    let NativeDeclared {
        errors,
        blocks,
        items,
        types,
        fluids,
    } = native_declared();
    if !errors.is_empty() {
        return Err(errors.join("\n").into());
    }
    if !blocks.is_empty() || !items.is_empty() || !types.is_empty() {
        let known = match &mut known {
            Some(types) => types,
            None => known.insert(mod_content::known_components(root)?),
        };
        let errors = mod_content::accept_declared(&mut mods, blocks, items, types, known);
        if !errors.is_empty() {
            return Err(errors.join("\n").into());
        }
        mod_content::accept_world_fluids(&mut mods, fluids);
        // A file item's remainder named a code-declared item that was refused: that mod
        // alone is left out, not every content mod through the overlay's check.
        for e in mod_content::drop_dangling_remainders(&mut mods) {
            log::warn!("Horizon: {e}");
        }
    }
    // Content validation may refuse a package. Revalidate against what really
    // loaded so dependents cannot start with a missing required content mod.
    let loaded: Vec<_> = manifests
        .into_iter()
        .filter(|package| {
            mods.iter().any(|content| content.id == package.id)
                || sources
                    .native
                    .built()
                    .iter()
                    .any(|native| native.id == package.id)
        })
        .collect();
    let resolved = horizon_mod_package::dependencies::resolve_manifests(
        &loaded,
        sources.installation_side,
        &overrides,
    )
    .map_err(|error| error.to_string())?;
    let cache = here.join(".horizon-cache");
    // Content admission and the live code-dispatch boundary jointly determine
    // what loaded. Namespaces and raw startup metadata cannot answer this.
    let inventory = sources.enabled_inventory(|id| mods.iter().any(|loaded| loaded.id == id));
    let prepared = super::load_conditions::content_root_with_conditions(
        root, jar_store, &mods, &cache, &inventory, sources,
    )?;
    let root = mod_content::resolved_resources(root, prepared, jar_store, &mods, cache)?;
    let _ = sources.content_base.set(original_base);
    *sources.load_notices.lock().unwrap() = resolved.warnings;
    Ok(ContentOverlay {
        root,
        mods,
        announced,
        announce,
        content_mods,
    })
}

fn resolve_particle_providers(
    source: &horizon_assets::resource_source::ResourceSource,
) -> Result<(), String> {
    if mod_api::__private::particles_resolved() {
        return Ok(());
    }
    // Resolve all particle providers once, alongside the other load-owned registries.
    let mut particles: Vec<String> = horizon_client::particles::ParticleType::ALL
        .iter()
        .map(|ty| format!("minecraft:{}", ty.name()))
        .collect();
    particles.extend(
        source
            .list_under("assets", &["particles"])
            .into_iter()
            .filter_map(|path| {
                let path = path.strip_prefix("assets/")?;
                let (namespace, path) = path.split_once("/particles/")?;
                let path = path.strip_suffix(".json")?;
                let id = format!("{namespace}:{path}");
                mod_api::Identifier::parse_namespaced(&id)?;
                // A vanilla-namespace file the game has no particle for is a leftover of an
                // older resource extraction, not a mod's declaration: skip it, never refuse startup.
                if namespace == "minecraft"
                    && horizon_client::particles::ParticleType::from_name(&id).is_none()
                {
                    log::warn!("Horizon: ignoring {id}: no such vanilla particle in this version");
                    return None;
                }
                Some(id)
            }),
    );
    mod_api::__private::resolve_particles(particles.iter().map(String::as_str), |id| {
        horizon_client::particles::ParticleType::from_name(id)
            .map(|ty| mod_api::__private::ParticleKind::Vanilla(ty as u32))
            .or_else(|| {
                (!id.starts_with("minecraft:")).then_some(mod_api::__private::ParticleKind::Custom)
            })
    })
    .map_err(|errors| errors.join("\n"))?;
    Ok(())
}
#[cfg(test)]
mod particle_registry_reload_tests {
    #[test]
    fn an_unlisted_vanilla_provider_file_does_not_refuse_startup() {
        mod_platform::__private::part_tests::install_parts_fixture(&[]).unwrap();
        let source = horizon_assets::resource_source::ResourceSource::indexed(
            [(
                "assets/minecraft/particles/ambient_entity_effect.json".into(),
                horizon_assets::resource_source::Resource::bytes(Vec::new()),
            )]
            .into(),
        );
        super::resolve_particle_providers(&source).unwrap();
    }

    #[test]
    fn repeated_particle_content_load_keeps_the_published_keys() {
        mod_platform::__private::part_tests::install_parts_fixture(&[]).unwrap();
        let source = horizon_assets::resource_source::ResourceSource::indexed(
            [(
                "assets/invalid namespace/particles/bad name.json".into(),
                horizon_assets::resource_source::Resource::bytes(Vec::new()),
            )]
            .into(),
        );
        super::resolve_particle_providers(&source).unwrap();
        let first = mod_api::__private::particle_vanilla_key(
            horizon_client::particles::ParticleType::Smoke as u32,
        )
        .unwrap();
        super::resolve_particle_providers(&source).unwrap();
        assert_eq!(
            mod_api::__private::particle_vanilla_key(
                horizon_client::particles::ParticleType::Smoke as u32
            ),
            Some(first)
        );
    }
}

/// The content root with everything the mods declare registered against the
/// process: block entities, mob effects, entity and menu types, fluid looks,
/// and the protocol's local mod list, each set once.
pub(super) fn content_root(
    root: &std::path::Path,
    sources: &ModsState,
) -> horizon_client::Result<horizon_assets::resource_source::ResourceRoot> {
    let _step = horizon_client::spans::startup::step("mods.content_root");
    let jar_store = horizon_client::logging::game_home().join("jars");
    let overlay = {
        let _step = horizon_client::spans::startup::step("mods.overlay");
        content_overlay(root, sources, &jar_store)?
    };
    let world_fluids = horizon_client::mod_content::register_world_fluids(&overlay.mods)?;
    register_block_entity_types(overlay.root.prepared(), &overlay.mods)?;
    if overlay.announce {
        horizon_protocol::mod_list::set_local(overlay.announced);
    }
    // What every level.dat saved from here on records, and what a world is checked against
    // before it opens (`level_mods`).
    horizon_client::save::level_mods::set_content_mods(Some(overlay.content_mods));
    if overlay.root.prepared() != root {
        log::info!(
            "Horizon: content from {} mod(s) in {}",
            overlay.mods.len(),
            overlay.root.prepared().display()
        );
    }
    register_mob_effects(&overlay.mods);
    resolve_particle_providers(overlay.root.source())?;
    mod_api::__private::resolve_effects(
        horizon_client_core::potions::all_effects()
            .map(|def| horizon_client_core::potions::namespaced(def.id)),
        |id| horizon_client_core::potions::effect(id).map(|def| def.index),
    )
    .map_err(|errors| errors.join("\n"))?;
    let behavior: serde_json::Value =
        horizon_world::json_file::read(&overlay.root.prepared().join("block-behaviours.json"))?;
    let fluids = behavior["fluids"]
        .as_array()
        .ok_or("missing fluid registry")?;
    let mut fluids = fluids
        .iter()
        .map(|value| value.as_str().ok_or("invalid fluid registry entry"))
        .collect::<Result<Vec<_>, _>>()?;
    fluids.extend(world_fluids.iter().map(String::as_str));
    mod_api::__private::resolve_fluids(
        &fluids,
        &horizon_world::fluid::FluidKind::ALL.map(|kind| kind.name()),
    )
    .map_err(|errors| {
        errors
            .into_iter()
            .map(|error| error.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    })?;
    register_entity_types(&overlay.mods);
    register_menu_types(&overlay.mods)?;
    register_iterated_registries(overlay.root.prepared())?;
    horizon_client::mod_content::set_fluid_looks(&overlay.mods);
    Ok(overlay.root)
}

/// Publish the built-in registry orders mods iterate. The report already
/// includes the types and menus the content overlay registered, in registration
/// order (`BuiltInRegistries` protocol ids).
pub(in crate::game) fn register_iterated_registries(
    root: &std::path::Path,
) -> horizon_client::Result<()> {
    use horizon_protocol::registry::Builtin;
    let path = Builtin::find(root).ok_or("registries.json not found")?;
    let builtin = Builtin::load(&path)?;
    if let Some(ids) = builtin.get("minecraft:entity_type") {
        mod_api::__private::resolve_entity_types(ids.names.iter().map(String::as_str))
            .map_err(|errors| errors.join("\n"))?;
    }
    if let Some(ids) = builtin.get("minecraft:menu") {
        mod_api::__private::resolve_menus(ids.names.iter().map(String::as_str))
            .map_err(|errors| errors.join("\n"))?;
    }
    Ok(())
}

fn register_block_entity_types(
    root: &std::path::Path,
    mods: &[horizon_client::mod_content::ContentMod],
) -> horizon_client::Result<()> {
    use horizon_client::block_entity_types::{self, BlockEntityTypes};
    use horizon_protocol::registry::Builtin;
    let path = Builtin::find(root).ok_or("registries.json not found")?;
    let builtin = Builtin::load(&path)?;
    let names = &builtin
        .get("minecraft:block_entity_type")
        .ok_or("missing block entity type registry")?
        .names;
    let membership = mods
        .iter()
        .flat_map(|m| &m.content.block_entity_types)
        .flat_map(|kind| {
            kind.blocks
                .iter()
                .map(|block| (block.clone(), kind.id.clone()))
        })
        .collect();
    let mut types = BlockEntityTypes::new(membership, names.clone());
    let blocks = builtin.get("minecraft:block");
    for block in blocks
        .into_iter()
        .flat_map(|ids| &ids.names)
        .filter(|name| name.starts_with("minecraft:"))
    {
        if let Some(kind) = horizon_client::lan::chunks::block_entity_type(block) {
            types.template(block.clone(), block.clone(), kind.to_owned());
        }
    }
    for block in mods.iter().flat_map(|m| &m.content.blocks) {
        if let Some(kind) = horizon_client::lan::chunks::block_entity_type(&block.template) {
            types.template(block.id.clone(), block.template.clone(), kind.to_owned());
        }
    }
    mod_api::__private::resolve_block_entity_types(types.names(), |id| types.key(id))
        .map_err(|errors| errors.join("\n"))?;
    let additions: Vec<(String, String)> = if let Some(current) = block_entity_types::current() {
        current
            .memberships()
            .map(|(block, kind)| (kind.to_owned(), block.to_owned()))
            .collect()
    } else {
        let keys: Vec<_> = types
            .names()
            .filter_map(mod_api::__private::block_entity_type_key)
            .collect();
        let mut additions = Vec::new();
        Game::native_block_entity_type_add_blocks(
            &RegistrationContext::new(),
            &keys,
            &mut additions,
        );
        additions
            .into_iter()
            .map(|(kind, block)| {
                Ok((
                    mod_api::__private::block_entity_type_name(kind)
                        .ok_or("unknown block entity type key")?
                        .to_owned(),
                    mod_api::__private::block_key_id(block)
                        .ok_or("unknown block key")?
                        .to_owned(),
                ))
            })
            .collect::<horizon_client::Result<_>>()?
    };
    if additions.is_empty() {
        block_entity_types::install(types)?;
        return Ok(());
    }
    // Empty registration needs no block IDs or class data; requested additions do.
    let blocks = blocks.ok_or("missing block registry")?;
    let behavior: serde_json::Value =
        horizon_world::json_file::read(&root.join("block-behaviours.json"))?;
    let classes: std::collections::BTreeMap<&str, Vec<&str>> = behavior["blocks"]
        .as_array()
        .ok_or("missing block behaviors")?
        .iter()
        .filter_map(|entry| {
            Some((
                entry["name"].as_str()?,
                entry["class"]
                    .as_array()?
                    .iter()
                    .filter_map(|v| v.as_str())
                    .collect(),
            ))
        })
        .collect();
    // Use the original valid sets for every addition; listener order cannot relax the constraint.
    let bases: std::collections::BTreeMap<String, String> = types
        .names()
        .filter_map(|kind| {
            let mut valid = types
                .memberships()
                .filter(|(_, k)| *k == kind)
                .filter_map(|(b, _)| classes.get(b));
            let first = valid.next()?;
            let rest: Vec<_> = valid.collect();
            first
                .iter()
                .find(|class| rest.iter().all(|chain| chain.contains(class)))
                .map(|class| (kind.to_owned(), (*class).to_owned()))
        })
        .collect();
    for (kind, block) in additions {
        let (kind, block) = (kind.as_str(), block.as_str());
        if blocks.id(block).is_none() {
            return Err(format!("unknown block {block}").into());
        }
        if let Some(base) = bases.get(kind)
            && !classes
                .get(block)
                .is_some_and(|chain| chain.contains(&base.as_str()))
        {
            return Err(format!("block {block} must inherit {base} to be valid for {kind}").into());
        }
        types.add_valid(block, kind)?;
    }
    block_entity_types::install(types)?;
    Ok(())
}

/// Manifest identity for the configuration-phase mod check.
fn announced_entry(
    id: &str,
    version: &str,
    side: mod_api::Side,
) -> horizon_protocol::mod_list::Entry {
    use horizon_protocol::mod_list;
    use mod_api::Side;
    mod_list::Entry {
        id: id.to_owned(),
        version: version.to_owned(),
        // `Side` is open to new kinds; until the list carries one, such a mod stays local.
        side: if side == Side::Both {
            mod_list::Side::Both
        } else {
            mod_list::Side::Client
        },
    }
}

/// The mods' mob effects join vanilla's (`brewing::effect`), once the root
/// holding their network ids is built. The first root that declares effects
/// sets them, until restart: a pack reload after a Mods switch builds a root
/// with others. A root declaring none freezes nothing, and a game without
/// mob effects reads the same empty list either way.
fn register_mob_effects(mods: &[horizon_client::mod_content::ContentMod]) {
    use horizon_client::brewing::{self, Category};
    use horizon_mod_package::content::EffectCategory;
    let effects = horizon_client::mod_content::mob_effects(mods);
    if effects.is_empty() {
        return;
    }
    let ids: Vec<String> = effects.iter().map(|e| e.id.clone()).collect();
    let set = brewing::register_mod_effects(effects.into_iter().map(|e| {
        let category = match e.category {
            EffectCategory::Beneficial => Category::Beneficial,
            EffectCategory::Harmful => Category::Harmful,
            EffectCategory::Neutral => Category::Neutral,
        };
        (e.id.clone(), e.color, category, e.sort_order)
    }));
    if !set
        && brewing::mod_effects()
            .iter()
            .map(|e| e.id)
            .ne(ids.iter().map(String::as_str))
    {
        log::warn!("Horizon: the mods' mob effects changed; restart the game to use them");
    }
}

/// the mods' entity types' templates, which the tracker reads their settings
/// from, and their thrown types, as their mob effects: set once, from the
/// startup root. Living types are mobs, loaded with vanilla's from the
/// content root's `entity-types.json`.
fn register_entity_types(mods: &[horizon_client::mod_content::ContentMod]) {
    use horizon_client::{lan::tracking, projectiles};
    let mut types = horizon_client::mod_content::entity_types(mods);
    let templates = types.iter().map(|t| (t.id.clone(), t.template.clone()));
    let templates_set = tracking::register_templates(templates);
    let templates_changed = !templates_set
        && tracking::templates()
            .iter()
            .copied()
            .ne(types.iter().map(|t| (t.id.as_str(), t.template.as_str())));
    types.retain(|t| t.item.is_some());
    let set = projectiles::register_thrown_types(
        types
            .iter()
            .filter_map(|t| Some((t.id.clone(), t.item.clone()?, t.damage))),
    );
    let thrown_changed = !set
        && projectiles::thrown_types()
            .iter()
            .map(|t| t.id)
            .ne(types.iter().map(|t| t.id.as_str()));
    if templates_changed || thrown_changed {
        log::warn!("Horizon: the mods' entity types changed; restart the game to use them");
    }
}

/// the mods' menu types, as their entity types: set once, from the startup
/// root.
fn register_menu_types(
    mods: &[horizon_client::mod_content::ContentMod],
) -> horizon_client::Result<()> {
    use horizon_client::containers;
    let menus = horizon_client::mod_content::menu_types(mods);
    menu_books::validate_registered(&menus)?;
    let set = containers::register_mod_menu_types(menus.iter().map(|m| {
        let results = m.results.iter().map(|&r| usize::from(r)).collect();
        let crafting = m.crafting.map(|c| containers::ModCrafting {
            grid: c.grid.into(),
            size: c.size.into(),
            result: c.result.into(),
        });
        let links = m.links.map(|l| containers::ModLinks {
            from: l.from.into(),
            count: l.count.into(),
        });
        (
            m.id.clone(),
            m.slots.into(),
            m.data_slots.into(),
            results,
            crafting,
            links,
        )
    }));
    if !set
        && containers::mod_menu_types()
            .iter()
            .map(|m| m.id)
            .ne(menus.iter().map(|m| m.id.as_str()))
    {
        log::warn!("Horizon: the mods' menu types changed; restart the game to use them");
    }
    Ok(())
}

/// The embedded inventory, formatted for the crash report.
fn native_mods_detail(built: &[mod_api::__private::native::BuiltMod]) -> Option<String> {
    let list = built
        .iter()
        .map(|m| format!("{} {}", m.id, m.version))
        .collect::<Vec<_>>()
        .join(", ");
    (!list.is_empty()).then_some(list)
}

/// Whether a content mod's files can switch with a world open (`mod_content::live_switch`);
/// `None` when its content can't be read.
type Verdict = Option<std::result::Result<(), &'static str>>;

/// Each mod's [`Verdict`] by its path and modification time. Reading a mod's content walks and
/// checks every file (seconds for a large pack), and a start read each mod three times: the
/// content root, then the window's and the server thread's `start_mods`.
/// shortcut: a file edited inside an unpacked mod folder keeps the folder's time; `--dev-mod`
/// folders are never remembered, a player's are replaced whole by the launcher.
static VERDICTS: std::sync::Mutex<
    std::collections::BTreeMap<(PathBuf, std::time::SystemTime), Verdict>,
> = std::sync::Mutex::new(std::collections::BTreeMap::new());

fn verdict_key(path: &std::path::Path) -> Option<(PathBuf, std::time::SystemTime)> {
    let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok()?;
    Some((path.to_owned(), modified))
}

/// Remembers the verdict of `content`, read from `path` as [`horizon_mod_package::Content::read`] does.
fn remember_verdict(path: &std::path::Path, content: &horizon_mod_package::Content) {
    if let Some(key) = verdict_key(path) {
        let verdict = Some(horizon_client::mod_content::live_switch(content));
        VERDICTS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key, verdict);
    }
}

/// The mod at `path`'s [`Verdict`], read once per version of it (never remembered for `dev`).
fn verdict(path: &std::path::Path, manifest: &horizon_mod_package::Manifest, dev: bool) -> Verdict {
    let key = if dev { None } else { verdict_key(path) };
    if let Some(key) = &key
        && let Some(verdict) = VERDICTS.lock().unwrap_or_else(|e| e.into_inner()).get(key)
    {
        return *verdict;
    }
    let verdict = horizon_mod_package::Content::read(path, manifest)
        .ok()
        .map(|content| horizon_client::mod_content::live_switch(&content));
    if let Some(key) = key {
        VERDICTS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key, verdict);
    }
    verdict
}

/// Notes a content mod's id, and whether its files can switch with a world
/// open (`mod_content::live_switch`).
fn live_switch_of(
    path: &std::path::Path,
    package: &horizon_mod_package::Package,
    dev: bool,
    content_ids: &mut std::collections::BTreeSet<String>,
    live_ids: &mut std::collections::BTreeSet<String>,
) {
    if package.is_native() || package.manifest.wasm.is_some() {
        return;
    }
    let id = &package.manifest.id;
    content_ids.insert(id.clone());
    match verdict(path, &package.manifest, dev) {
        // Jar layers can declare worldgen that Content.files does not list.
        Some(Ok(())) if package.manifest.jar.is_empty() => {
            live_ids.insert(id.clone());
        }
        Some(Err(why)) => log::debug!("Horizon: mod {id} switches on a restart: {why}"),
        _ => {}
    }
}

/// Installed native mods not compiled into this executable get an explanatory row.
fn not_built(
    dir: &std::path::Path,
    built: &[mod_api::__private::native::BuiltMod],
    rows: &[Installed],
) -> Vec<Installed> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|d| d.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    paths.retain(|p| p.extension().is_some_and(|e| e == "hmod"));
    paths.sort();
    let mut out: Vec<Installed> = Vec::new();
    for path in paths {
        let Ok(bundle) = horizon_mod_package::index::native::Bundle::read(&path) else {
            continue;
        };
        let m = bundle.manifest;
        let listed = rows.iter().chain(&out).any(|row| row.id == m.id);
        if !listed && !built.iter().any(|b| b.id == m.id) {
            out.push(Installed {
                title: format!("{} {}", m.id, m.version),
                id: m.id,
                description: String::new(),
                problem: Some(mod_list::NOT_BUILT.to_owned()),
                dev: false,
                native: None,
            });
        }
    }
    out
}

pub(super) struct PreparedLoopContext {
    game: Arc<horizon_world::native_load_context::LoopContext>,
    audio: Option<horizon_world::native_load_context::LoopContext>,
    /// A server's context for the features it generates (`alter_ground`).
    worldgen: Option<horizon_world::native_load_context::WorldgenContext>,
}

impl Game<'_> {
    /// Observe the platform phase at construction, after bootstrapping the real test runtime.
    pub(super) fn mod_phase_for_construction() -> mod_api::__private::ModPhase {
        #[cfg(test)]
        Self::initialize_mod_runtime_for_tests();
        mod_api::__private::current_phase()
    }

    #[cfg(test)]
    pub(super) fn initialize_mod_runtime_for_tests() {
        static INITIALIZED: std::sync::Once = std::sync::Once::new();
        INITIALIZED.call_once(|| {
            super::api_values::install_test_platform();
            mod_platform::__private::initialize()
                .expect("engine test runtime initializes before mod dispatch");
        });
    }

    /// Build this game's loop context over the loaded registries and the mods'
    /// current settings. Called where a resource generation starts or reloads: a
    /// client at window start and after a resource reload, a server when it starts
    /// its mods and on `/reload`. A client hands a second one to its audio; a
    /// server publishes one for its world generation.
    pub(super) fn refresh_loop_context(&mut self) -> horizon_client::Result<()> {
        let context = self.prepare_loop_context(&self.options.resources)?;
        self.install_loop_context(context);
        Ok(())
    }

    pub(super) fn prepare_loop_context(
        &self,
        resources: &std::path::Path,
    ) -> horizon_client::Result<PreparedLoopContext> {
        let side = self.options.execution_role.loop_side();
        let inventory = horizon_mod_package::enabled::EnabledMods::read(resources)?;
        let ids: Vec<_> = if self.is_logical_server() {
            inventory.server_ids().collect()
        } else {
            inventory.client_ids().collect()
        };
        let item_data = self.res.core.item_data.context();
        let audio = self
            .options
            .execution_role
            .is_client()
            .then(|| item_data.loop_context(side, ids.iter().copied()));
        let worldgen = self
            .is_logical_server()
            .then(|| item_data.worldgen_context(ids.iter().copied()));
        Ok(PreparedLoopContext {
            game: Arc::new(item_data.loop_context(side, ids.iter().copied())),
            audio,
            worldgen,
        })
    }

    pub(super) fn install_loop_context(&mut self, context: PreparedLoopContext) {
        self.protect_condition_root();
        self.loop_context = context.game;
        self.refresh_recipe_property_context();
        self.setting_overrides.borrow_mut().clear();
        if let Some(audio) = context.audio {
            self.audio.set_loop_context(audio);
        }
        if let Some(worldgen) = context.worldgen {
            horizon_world::native_worldgen_hooks::publish_context(Arc::new(worldgen));
        }
    }

    /// Stage a fresh root when admission changes or registered conditions need reevaluation.
    /// The caller publishes it after every server reload listener prepared.
    pub(super) fn stage_enabled_inventory(
        &self,
    ) -> horizon_client::Result<Option<horizon_assets::resource_source::ResourceRoot>> {
        let previous = horizon_mod_package::enabled::EnabledMods::read(&self.options.resources)?;
        let next = self
            .mods
            .enabled_inventory(|id| previous.client_ids().any(|loaded| loaded == id));
        if previous == next && !self.condition_root_needs_reload() {
            return Ok(None);
        }
        let base = self
            .mods
            .content_base
            .get()
            .cloned()
            .unwrap_or_else(|| self.vanilla_root());
        let jars = horizon_client::logging::game_home().join("jars");
        Ok(Some(content_overlay(&base, &self.mods, &jars)?.root))
    }

    /// Lists the installed mods on the Mods page. Content mods load with the resources
    /// (`content_root`); native ones are built into the game (`native_hooks.rs`). Unsupported
    /// files get the same unreadable-file row as any other invalid package.
    pub(super) fn start_mods(&mut self) {
        let dir = Self::game_dir().join(mod_list::DIR);
        let mut paths = horizon_mod_package::find(&dir);
        paths.retain(|p| !self.mods.dev.contains(p));
        paths.extend(self.mods.dev.iter().cloned());
        let (mut content_ids, mut live_ids) = Default::default();
        let mut info = std::collections::BTreeMap::new();
        self.settings.mods.installed = paths
            .iter()
            .map(|path| {
                let file = path.file_name().unwrap_or_default().to_string_lossy();
                let package = match horizon_mod_package::Package::read(path) {
                    Ok(package) => package,
                    Err(e) => {
                        return Installed {
                            id: String::new(),
                            title: file.into_owned(),
                            description: String::new(),
                            problem: Some(format!("Can't be read: {e}")),
                            dev: false,
                            native: None,
                        };
                    }
                };
                let m = &package.manifest;
                let refused = crate::entry::code_refusal(&m.id);
                live_switch_of(
                    path,
                    &package,
                    self.mods.dev.contains(path),
                    &mut content_ids,
                    &mut live_ids,
                );
                info.insert(
                    m.id.clone(),
                    mod_list::ModInfo::of(m, package.icon.as_ref()),
                );
                Installed {
                    id: m.id.clone(),
                    title: format!("{} {}", m.name, m.version),
                    description: m.description.clone(),
                    problem: refused.map(|why| format!("Not loaded: {why}")),
                    dev: self.mods.dev.contains(path),
                    native: None,
                }
            })
            .collect();
        for (id, info) in &mut info {
            info.warnings = mod_api::__private::registration_warnings(id)
                .into_iter()
                .map(|w| (w.key().to_owned(), w.args().to_vec()))
                .collect();
        }
        self.settings.mods.info = info;
        (self.mods.content_ids, self.mods.live_ids) = (content_ids, live_ids);
        if self.mods.applied.is_none() {
            self.mods.applied = Some(self.settings.mods.enabled.clone());
        }
        // A mod with a native version built in says so instead, and isn't news.
        self.list_native_mods();
        self.sync_native_switches();
        let built = self.mods.native.built();
        let built_mods = native_mods_detail(built);
        if let Some(list) = built_mods {
            mod_api::content::crash::set_detail("Native Mods", list);
        }

        let native_dir = Self::game_dir().join(mod_list::NATIVE_DIR);
        let unbuilt = not_built(&native_dir, built, &self.settings.mods.installed);
        self.settings.mods.installed.extend(unbuilt);
        self.mods.report_refusals(&mut self.settings.mods.installed);
        self.settings
            .mods
            .apply_dependency_notices(&self.mods.load_notices.lock().unwrap());
    }
    /// The content mods the resources load with: the ones applied, which a
    /// switch that needs a restart doesn't change.
    pub(super) fn applied_mods(&self) -> std::collections::BTreeSet<String> {
        self.mods
            .applied
            .clone()
            .unwrap_or_else(|| self.settings.mods.enabled.clone())
    }
    /// The player turned content mods on or off (Options > Mods, or an
    /// install from Browse Mods): those with only assets and data apply now
    /// (docs/design/mod-hotload.md), through the resource reload and then the
    /// server's data reload (`mods_reloaded`); those that add blocks, items or
    /// worldgen wait for a restart, and the Mods page says so.
    pub(super) fn switch_mods(&mut self) {
        let Some(applied) = self.mods.applied.clone() else {
            return;
        };
        let wanted = self.settings.mods.enabled.clone();
        if applied == wanted {
            return;
        }
        // Native mods' `mod.<id>` lines follow the player's choice as they are
        // (`sync_native_switches`); a content mod with blocks, items or
        // worldgen keeps the state it loaded with until a restart.
        let content = &self.mods.content_ids;
        let mut next = wanted.clone();

        let mut waiting = Vec::new();
        for id in applied.symmetric_difference(&wanted) {
            if !content.contains(id) || self.mods.live_ids.contains(id) {
                continue;
            }
            log::info!("Horizon: mod {id} needs a restart to switch");
            waiting.push(id.clone());
            if applied.contains(id) {
                next.insert(id.clone());
            } else {
                next.remove(id);
            }
        }
        self.ui.options.mods_note = (!waiting.is_empty())
            .then(|| lang::tr("horizon.mods.switch.restart", &[&waiting.join(", ")]));
        let changed = applied
            .symmetric_difference(&next)
            .any(|id| content.contains(id));
        if !changed {
            self.mods.applied = Some(next);
            return;
        }
        log::info!(
            "Horizon: content mods switched live: {:?} -> {:?}",
            applied,
            next
        );
        self.mods.applied = Some(next);
        self.mods.live = Some(std::time::Instant::now());
        self.reload_packs();
    }
    /// A live switch's resources applied: the server reloads its data packs
    /// from the new root, as `/reload` does (recipes, tags, loot tables,
    /// advancements, functions), and sends them to its players.
    pub(super) fn mods_reloaded(&mut self, applied: bool) {
        let Some(started) = self.mods.live.take() else {
            return;
        };
        let resources_ms = started.elapsed().as_millis();
        if !applied {
            return;
        }
        let root = self.options.resources.clone();
        let data_source = self.pack.data_source.clone();
        let data_changes = self.pack.data_changes.clone();
        let additional_data = self.pack.additional_data.clone();
        let enabled = self.applied_mods();
        let selected = self.mods.selected.clone();
        let refused = self.mods.refused.clone();
        let reload = move |g: &mut Game<'_>| {
            let at = std::time::Instant::now();
            g.options.resources = root;
            g.pack.data_source = data_source;
            g.pack.data_changes = data_changes;
            g.pack.additional_data = additional_data;
            g.settings.mods.enabled = enabled.clone();
            g.mods.applied = Some(enabled);
            g.mods.selected = selected;
            g.mods.refused = refused;
            g.mods.data_switched = true;
            if g.world_root.as_os_str().is_empty() {
                return;
            }
            if let Err(error) = g.reload_datapacks(true, &g.level.enabled_features.clone()) {
                log::warn!("Horizon: content mods data reload failed: {error}");
                return;
            }
            log::info!(
                "Horizon: content mods switched live: server data reloaded in {} ms",
                at.elapsed().as_millis()
            );
        };
        log::info!("Horizon: content mods switched live: resources reloaded in {resources_ms} ms");
        if let Some(server) = &self.integrated {
            server.post(reload);
        } else if self.remote.is_none() {
            reload(self);
        }
    }
    /// Apply the player's switches at the canonical native and wasm dispatch boundary.
    pub(super) fn sync_native_switches(&self) {
        self.instance_groups
            .borrow_mut()
            .retire_disabled(&self.settings.mods.disabled);
        self.mods.apply_code_switches(&self.settings.mods.disabled);
    }
}

/// Mob types loaded from a scratch root whose `entity-types.json` has a
/// zombie, a villager, a wandering trader, a wolf, a cat and a parrot, and the
/// rows a content root writes for a mod's types (`mod_content`): `ruby:ghoul`
/// (a zombie with 40 health), `ruby:settler` (a villager as it is),
/// `ruby:merchant` (a wandering trader), the tameable `ruby:hound`,
/// `ruby:tabby` and `ruby:macaw` (a wolf, a cat and a parrot) and `ruby:bolt`
/// (a thrown snowball, not a mob).
#[cfg(test)]
pub(super) fn living_mod_types()
-> std::collections::HashMap<String, std::sync::Arc<horizon_client::mobs::MobType>> {
    let zombie =
        r#""width":0.6,"height":1.95,"eye_height":1.74,"category":"monster","fire_immune":false"#;
    let villager =
        r#""width":0.6,"height":1.95,"eye_height":1.62,"category":"misc","fire_immune":false"#;
    let trader =
        r#""width":0.6,"height":1.95,"eye_height":1.62,"category":"creature","fire_immune":false"#;
    let wolf =
        r#""width":0.6,"height":0.85,"eye_height":0.68,"category":"creature","fire_immune":false"#;
    let cat =
        r#""width":0.6,"height":0.7,"eye_height":0.35,"category":"creature","fire_immune":false"#;
    let parrot =
        r#""width":0.5,"height":0.9,"eye_height":0.54,"category":"creature","fire_immune":false"#;
    let table = format!(
        r#"{{"minecraft:zombie":{{{zombie},"attributes":{{"minecraft:max_health":20.0,"minecraft:movement_speed":0.23000000417232513}}}},
        "minecraft:villager":{{{villager},"attributes":{{"minecraft:max_health":20.0,"minecraft:movement_speed":0.5}}}},
        "minecraft:wandering_trader":{{{trader},"attributes":{{"minecraft:max_health":20.0,"minecraft:movement_speed":0.7}}}},
        "minecraft:wolf":{{{wolf},"attributes":{{"minecraft:max_health":8.0,"minecraft:movement_speed":0.30000001192092896}}}},
        "minecraft:cat":{{{cat},"attributes":{{"minecraft:max_health":10.0,"minecraft:movement_speed":0.30000001192092896}}}},
        "minecraft:parrot":{{{parrot},"attributes":{{"minecraft:max_health":6.0,"minecraft:movement_speed":0.20000000298023224}}}},
        "minecraft:snowball":{{"width":0.25,"height":0.25,"eye_height":0.0,"category":"misc","fire_immune":false}},
        "ruby:ghoul":{{{zombie},"attributes":{{"minecraft:max_health":40.0,"minecraft:movement_speed":0.23000000417232513}},"template":"minecraft:zombie"}},
        "ruby:settler":{{{villager},"attributes":{{"minecraft:max_health":20.0,"minecraft:movement_speed":0.5}},"template":"minecraft:villager"}},
        "ruby:merchant":{{{trader},"attributes":{{"minecraft:max_health":20.0,"minecraft:movement_speed":0.7}},"template":"minecraft:wandering_trader"}},
        "ruby:hound":{{{wolf},"attributes":{{"minecraft:max_health":8.0,"minecraft:movement_speed":0.30000001192092896}},"template":"minecraft:wolf"}},
        "ruby:tabby":{{{cat},"attributes":{{"minecraft:max_health":10.0,"minecraft:movement_speed":0.30000001192092896}},"template":"minecraft:cat"}},
        "ruby:macaw":{{{parrot},"attributes":{{"minecraft:max_health":6.0,"minecraft:movement_speed":0.20000000298023224}},"template":"minecraft:parrot"}},
        "ruby:bolt":{{"width":0.25,"height":0.25,"eye_height":0.0,"category":"misc","fire_immune":false,"template":"minecraft:snowball"}}}}"#
    );
    // A folder per call: tests run in parallel in one process, and each removes its own.
    static CALLS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let call = CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "horizon-living-mod-types-{}-{call}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("entity-types.json"), table).unwrap();
    // No attribute catalog: the rows' values stay as written.
    let attributes = &horizon_world::attributes::entity::Catalog::from_json(&serde_json::json!({
        "data_version": horizon_world::DATA_VERSION,
        "attributes": [],
        "entities": {},
    }))
    .unwrap();
    let types = horizon_client::mobs::MobTypes::load_for_server(&root, attributes).unwrap();
    let _ = std::fs::remove_dir_all(&root);
    types
        .types
        .into_iter()
        .map(|(id, t)| (id, std::sync::Arc::new(t)))
        .collect()
}

/// The block states native mods add to vanilla POI types (`native_extend_poi_types`),
/// resolved against `registry` once its block definitions are published and before any
/// world's POI tables are built; collected again when the resources reload. Each refusal
/// is logged once.
pub(super) fn extend_poi_types(registry: &horizon_world::registry::BlockRegistry) {
    let (additions, refusals) = collect_poi_extensions(registry);
    for refusal in refusals
        .into_iter()
        .chain(horizon_world::poi::extend(additions))
    {
        log::warn!("Horizon: POI type extension refused: {refusal}");
    }
}

/// Ask the mods, then turn each predicate into the complete states of `registry` it
/// selects, in registration order. A tag predicate is refused here: additions name blocks.
pub(in crate::game) fn collect_poi_extensions(
    registry: &horizon_world::registry::BlockRegistry,
) -> (Vec<horizon_world::poi::PoiExtension>, Vec<String>) {
    let Some(metadata) = mod_api::__private::block_definitions() else {
        return (Vec::new(), Vec::new());
    };
    let mut extensions = mod_api::PoiTypeExtensionRegistry::new(metadata);
    Game::native_extend_poi_types(&RegistrationContext::new(), &mut extensions);
    let mut additions = Vec::new();
    let mut refusals = Vec::new();
    for (poi, predicate) in extensions.into_entries() {
        let record = mod_api::StatePredicateRecord::from(&predicate);
        let block = match record.target {
            mod_api::StatePredicateTarget::Block(block) => block,
            mod_api::StatePredicateTarget::Tag(_) => {
                refusals.push(format!(
                    "{poi} asked for a block tag's states; an addition names a block"
                ));
                continue;
            }
        };
        // The predicate was built on this metadata, so its block is known.
        let Ok(name) = mod_api::BlockMetadata::name(metadata, block) else {
            continue;
        };
        for index in 0..registry.len() {
            let state = registry
                .state(horizon_world::registry::BlockStateId(index as u32))
                .expect("registry index is in bounds");
            if state.name == name
                && record.constraints.iter().all(|constraint| {
                    state.properties.get(constraint.name.as_str()) == Some(&constraint.value)
                })
            {
                additions.push(horizon_world::poi::PoiExtension {
                    state: state.clone(),
                    poi: poi.to_string(),
                });
            }
        }
    }
    (additions, refusals)
}

/// The fluid contents native mods give their cauldron blocks
/// (`native_register_cauldron_fluid_content`), resolved against `registry` once its block
/// definitions are published, in one table with vanilla's water and lava, which the fluid
/// capability reads; collected again when the resources reload. Each refusal is logged once.
pub(super) fn register_cauldron_fluid_contents(registry: &horizon_world::registry::BlockRegistry) {
    let (contents, refusals) = collect_cauldron_fluid_contents(registry);
    for refusal in refusals {
        log::warn!("Horizon: cauldron fluid content refused: {refusal}");
    }
    crate::game::machines::cauldron_contents::install(contents);
}

/// Ask the mods, then resolve each registration against `registry` into the table after
/// vanilla's contents, in registration order: a block or fluid already in the table, an
/// unknown block or fluid, a level property the block lacks or that isn't an integer, and a
/// total of 0 or one the levels don't divide evenly are refused, with their reasons.
pub(in crate::game) fn collect_cauldron_fluid_contents(
    registry: &horizon_world::registry::BlockRegistry,
) -> (
    crate::game::machines::cauldron_contents::CauldronContents,
    Vec<String>,
) {
    use crate::game::machines::cauldron_contents::{CauldronContent, CauldronContents};
    let mut registrations = mod_api::CauldronFluidContentRegistry::default();
    Game::native_register_cauldron_fluid_content(&RegistrationContext::new(), &mut registrations);
    let mut contents = CauldronContents::vanilla();
    let mut refusals = Vec::new();
    for registration in registrations.into_entries() {
        let resolved = match (
            mod_api::__private::block_key_id(registration.block),
            mod_api::__private::fluid_id(registration.fluid),
        ) {
            (None, _) => Err("an unknown block was given a cauldron fluid content".to_owned()),
            (Some(block), None) => Err(format!("{block} was given an unknown fluid")),
            (Some(block), Some(fluid)) => CauldronContent::resolve(
                registry,
                block,
                fluid,
                registration.total,
                registration.level.as_deref(),
            ),
        };
        if let Err(refusal) = resolved.and_then(|content| contents.add(content)) {
            refusals.push(refusal);
        }
    }
    (contents, refusals)
}

/// Registered callbacks indexed directly by the slots resolved in enchantment content.
pub(in crate::game) static ENCHANTMENT_DAMAGE_EFFECTS: std::sync::OnceLock<
    Vec<std::sync::Arc<dyn mod_api::EnchantmentDamageEffect>>,
> = std::sync::OnceLock::new();

pub(super) fn register_enchantment_damage_effects() {
    let mut effects = mod_api::EnchantmentDamageRegistry::default();
    Game::native_register_enchantment_damage_effects(
        &mod_api::RegistrationContext::new(),
        &mut effects,
    );
    let (ids, callbacks): (Vec<_>, Vec<_>) = effects
        .into_entries()
        .into_iter()
        .map(|(id, effect)| (id.to_string(), effect))
        .unzip();
    horizon_items::enchant::register_damage_effects(ids);
    assert!(
        ENCHANTMENT_DAMAGE_EFFECTS.set(callbacks).is_ok(),
        "damage effects already registered"
    );
}

#[cfg(test)]
mod tests {
    /// A mod's live-switch verdict is read once per version of the mod: a later look finds
    /// it without reading the files again, a `--dev-mod` is always read, and the mod
    /// replaced (a new modification time) is read again.
    #[test]
    fn a_mods_live_switch_verdict_is_read_once_per_version_of_it() {
        let dir = std::env::temp_dir().join(format!("horizon-verdict-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ruby = dir.join("ruby");
        std::fs::create_dir_all(ruby.join("assets/ruby/lang")).unwrap();
        std::fs::create_dir_all(ruby.join("data/ruby")).unwrap();
        std::fs::write(ruby.join("assets/ruby/lang/en_us.json"), "{}").unwrap();
        let manifest = horizon_mod_package::Manifest::parse(
            "id = \"ruby\"\nname = \"Ruby\"\nversion = \"0.1.0\"\napi = \"0.2\"\n\
             side = \"both\"\ncapabilities = [\"registries\"]\n",
        )
        .unwrap();
        assert_eq!(verdict(&ruby, &manifest, false), Some(Ok(())));
        // Worldgen added below `data/ruby`: the mod folder's own time stays.
        let worldgen = ruby.join("data/ruby/worldgen/configured_feature");
        std::fs::create_dir_all(&worldgen).unwrap();
        std::fs::write(worldgen.join("ore.json"), "{}").unwrap();
        let switches_on_restart = Some(Err("horizon.mods.switch.worldgen"));
        assert_eq!(verdict(&ruby, &manifest, true), switches_on_restart);
        assert_eq!(verdict(&ruby, &manifest, false), Some(Ok(())));
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
        let mut folder = std::fs::OpenOptions::new();
        folder.read(true);
        // Windows opens a folder only with FILE_FLAG_BACKUP_SEMANTICS, and sets its time only
        // with write access.
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            folder.write(true).custom_flags(0x0200_0000);
        }
        folder.open(&ruby).unwrap().set_modified(later).unwrap();
        assert_eq!(verdict(&ruby, &manifest, false), switches_on_restart);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    use super::*;
    use horizon_mod_package::runtime::Runtime;
    use mod_api::__private::native::BuiltMod;

    #[test]
    fn only_clients_collect_registered_keys() {
        let declared = [(
            "sample",
            mod_api::KeyRegistration::new("zoom", "key.keyboard.z"),
        )];
        assert_eq!(
            collect_key_mappings(ExecutionRole::Client, &declared).len(),
            1
        );
        assert!(collect_key_mappings(ExecutionRole::IntegratedServer, &declared).is_empty());
        assert!(collect_key_mappings(ExecutionRole::DedicatedServer, &declared).is_empty());
    }

    mod inventory {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/game/modding/mods/mod_inventory_tests.rs"
        ));
    }

    const BUILT: &[BuiltMod] = &[BuiltMod {
        id: "machine_sample",
        version: "0.3.0",
        side: mod_api::Side::Both,
    }];

    struct Fixture(PathBuf);
    impl Fixture {
        fn new(label: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("horizon-game-mods-{}-{label}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
        fn runtime(&self) -> Runtime {
            let dir = self.0.join("compiled");
            let package = dir.join(mod_list::NATIVE_DIR).join("machine_sample");
            let file = std::fs::File::open(self.0.join("machine_sample.hmod")).unwrap();
            zip::ZipArchive::new(file)
                .unwrap()
                .extract(&package)
                .unwrap();
            Runtime::load(&dir.join("horizon-engine"), BUILT).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }

    /// Runs `test` again in a child process whose working directory is its own
    /// empty folder: the overlay's `.horizon-cache` lives under the working
    /// directory, so children sharing one (the temp dir) race on that cache.
    fn run_in_child(test: &str, mode: &str) {
        let cwd = std::env::temp_dir().join(format!(
            "horizon-game-mods-cwd-{}-{mode}",
            std::process::id()
        ));
        std::fs::create_dir_all(&cwd).unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .current_dir(&cwd)
            .args(["--include-ignored", "--exact", test, "--nocapture"])
            .env(mode, "1")
            .status()
            .unwrap();
        std::fs::remove_dir_all(&cwd).unwrap();
        assert!(status.success());
    }

    /// A content mod's living type is a mob of its template's species under
    /// its own id, with its own attributes: it saves its id with every field
    /// its template's class saves, loads back as itself, and rolls its own
    /// loot table. A thrown type stays a projectile.
    #[test]
    fn a_mod_living_type_is_its_templates_mob_with_its_own_attributes() {
        use horizon_client::mobs::{Mob, Species, drops::death_loot_table};
        use horizon_core::geometry::Vec3;
        let types = living_mod_types();
        let (zombie, ghoul) = (&types["minecraft:zombie"], &types["ruby:ghoul"]);
        assert_eq!(ghoul.species, Species::Zombie);
        assert_eq!(ghoul.class_id(), "minecraft:zombie");
        assert_eq!(types["ruby:settler"].species, Species::Villager);
        assert!(!types.contains_key("ruby:bolt"), "a thrown type is no mob");
        assert_eq!((zombie.max_health, ghoul.max_health), (20.0, 40.0));
        assert_eq!(ghoul.speed.to_bits(), zombie.speed.to_bits());
        assert_eq!(
            (ghoul.sounds, ghoul.texture.as_str()),
            (zombie.sounds, zombie.texture.as_str())
        );

        let at = Vec3::new(0.5, 64.0, 0.5);
        let (zombie, ghoul) = (
            Mob::new(1, zombie.clone(), at, 0.0),
            Mob::new(2, ghoul.clone(), at, 0.0),
        );
        assert_eq!(ghoul.health, 40.0);
        let saved =
            |m: &Mob| horizon_client::entity_nbt::mob_to_nbt(m, [0, 0, 0, 2], None, None, |_| None);
        let keys = |tag: &horizon_core::nbt::Tag| -> Vec<String> {
            let horizon_core::nbt::Tag::Compound(map) = tag else {
                panic!("a mob saves a compound");
            };
            map.keys().map(|k| k.to_string_lossy()).collect()
        };
        let (zombie_tag, ghoul_tag) = (saved(&zombie), saved(&ghoul));
        let Some(horizon_core::nbt::Tag::String(id)) = ghoul_tag.get("id") else {
            panic!("a saved mob has its id");
        };
        assert_eq!(id.to_string_lossy(), "ruby:ghoul");
        assert_eq!(
            keys(&ghoul_tag),
            keys(&zombie_tag),
            "it saves what a zombie saves"
        );
        for key in ["IsBaby", "CanBreakDoors", "DrownedConversionTime"] {
            assert!(keys(&ghoul_tag).iter().any(|k| k == key), "{key}");
        }
        let loaded = horizon_client::entity_nbt::mob_from_nbt(&ghoul_tag, &types, 3, None)
            .expect("it loads back");
        assert_eq!(
            (loaded.kind.id.as_str(), loaded.health),
            ("ruby:ghoul", 40.0)
        );
        assert_eq!(
            death_loot_table(&ghoul, true).as_deref(),
            Some("ruby:entities/ghoul")
        );
        assert_eq!(
            death_loot_table(&zombie, true).as_deref(),
            Some("minecraft:entities/zombie")
        );
    }

    /// A content mod's tameable type (#4031) is its template's mob: a wolf, a
    /// cat or a parrot, tamed and sitting, saves its owner, sitting and its
    /// template's own field (a collar's colour, a parrot's variant) under its
    /// own id and loads back as itself with all of them.
    #[test]
    fn a_mod_tameable_type_saves_and_loads_its_owner() {
        use horizon_client::mobs::{Mob, Species};
        use horizon_core::geometry::Vec3;
        let types = living_mod_types();
        let owner = [1, 2, 3, 4];
        use horizon_core::nbt::Tag;
        for (id, species, own) in [
            ("ruby:hound", Species::Wolf, ("CollarColor", Tag::Byte(14))),
            ("ruby:tabby", Species::Cat, ("CollarColor", Tag::Byte(3))),
            ("ruby:macaw", Species::Parrot, ("Variant", Tag::Int(2))),
        ] {
            let kind = types[id].clone();
            assert_eq!(kind.species, species, "{id}");
            let mut pet = Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
            pet.extra.tame = true;
            pet.extra.owner = Some(owner);
            pet.extra.sitting = true;
            match species {
                Species::Parrot => pet.extra.variant = 2,
                _ => pet.extra.color = if species == Species::Wolf { 14 } else { 3 },
            }
            let tag =
                horizon_client::entity_nbt::mob_to_nbt(&pet, [0, 0, 0, 1], None, None, |_| None);
            let get = |key: &str| tag.get(key).cloned();
            assert_eq!(
                get("id"),
                Some(horizon_core::nbt::Tag::String(id.into())),
                "{id}"
            );
            assert_eq!(
                get("Owner"),
                Some(horizon_core::nbt::Tag::IntArray(owner.to_vec())),
                "{id}"
            );
            assert_eq!(
                get("Sitting"),
                Some(horizon_core::nbt::Tag::Byte(1)),
                "{id}"
            );
            assert_eq!(get(own.0), Some(own.1), "{id}");
            let loaded = horizon_client::entity_nbt::mob_from_nbt(&tag, &types, 2, None)
                .expect("it loads back");
            assert_eq!(
                (
                    loaded.kind.id.as_str(),
                    loaded.species(),
                    loaded.extra.tame,
                    loaded.extra.owner,
                    loaded.extra.sitting,
                    (loaded.extra.color, loaded.extra.variant)
                ),
                (
                    id,
                    species,
                    true,
                    Some(owner),
                    true,
                    (pet.extra.color, pet.extra.variant)
                )
            );
        }
    }

    /// A mod's declared attribute set (#845, `EntityAttributeCreationEvent`)
    /// reaches everything a mob of the type is: the fields it spawns with,
    /// the finalize defaults and the live attribute map's catalog row —
    /// built from the template's, so the reads of #1469 and #1493 answer the
    /// declared values with no second copy.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_mods_declared_entity_attributes_reach_spawn_and_the_live_map() {
        use horizon_client::mobs::Mob;
        use horizon_core::geometry::Vec3;
        use horizon_world::attributes::entity::Catalog;

        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let catalog = Catalog::from_json(
            &horizon_world::json_file::read(&root.join("attributes.json")).unwrap(),
        )
        .unwrap();
        let declared = [
            ("minecraft:max_health".to_string(), 55.0),
            ("minecraft:movement_speed".to_string(), 0.4),
        ];
        let merged = catalog
            .with_entity_row("ruby:ghoul", "minecraft:zombie", &declared)
            .unwrap();
        let base = |catalog: &Catalog, entity: &str, name: &str| {
            catalog.default_base(entity, catalog.id(name).unwrap())
        };
        // The declared values, and the template's row under them.
        assert_eq!(
            base(&merged, "ruby:ghoul", "minecraft:max_health"),
            Some(55.0)
        );
        assert_eq!(
            base(&merged, "ruby:ghoul", "minecraft:movement_speed"),
            Some(0.4)
        );
        assert_eq!(
            base(&merged, "ruby:ghoul", "minecraft:attack_damage"),
            base(&catalog, "minecraft:zombie", "minecraft:attack_damage")
        );
        // The live map builds for the type and answers the declared values.
        let live = horizon_client::living_attributes::LivingAttributes::default();
        assert_eq!(
            live.read(
                &std::sync::Arc::new(merged.clone()),
                "ruby:ghoul",
                "minecraft:max_health",
                horizon_world::attribute_map::ValueKind::Computed,
                Vec::new
            ),
            Some(55.0)
        );
        // The type's fields spawn with them: the merged row takes the type's
        // attribute map and fields (`take_attribute_row`).
        let mut types = living_mod_types();
        let ghoul = types.get_mut("ruby:ghoul").unwrap();
        Arc::make_mut(ghoul)
            .take_attribute_row(Arc::new(merged.entity_row("ruby:ghoul").unwrap()), &merged);
        let ghoul = Mob::new(7, types["ruby:ghoul"].clone(), Vec3::ZERO, 0.0);
        assert_eq!(ghoul.health, 55.0, "the spawn's health");
        assert_eq!(ghoul.attrs.max_health, 55.0);
        assert_eq!(ghoul.attrs.speed, 0.4);
        assert_eq!(
            ghoul.kind.attributes.get("minecraft:max_health"),
            Some(&55.0),
            "the finalize defaults read the declared value"
        );
    }

    /// #812: a mod's declared attribute type joins the catalog before the
    /// rows resolve (a mod entity's set names its own new attribute), and
    /// the live map, an item's modifier, the saves and the sync set carry it
    /// by string id — a guest's catalog built from the same mods decodes the
    /// same network id.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_mods_declared_attribute_reaches_rows_the_live_map_saves_and_sync() {
        use horizon_client::lan;
        use horizon_client::living_attributes::{self, LivingAttributes};
        use horizon_core::nbt::Tag;
        use horizon_world::attribute_map::ValueKind;
        use horizon_world::attributes::{
            Definition, Sentiment,
            entity::{AddedAttribute, Catalog},
        };

        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let catalog = Catalog::from_json(
            &horizon_world::json_file::read(&root.join("attributes.json")).unwrap(),
        )
        .unwrap();
        let vanilla = catalog.attributes().count();
        // Two mods' attributes, declared in an order that is not the sorted
        // one; one syncable, one not.
        let declared = |mod_id: &str, id: &str, default, syncable, sentiment| AddedAttribute {
            mod_id: mod_id.into(),
            id: id.into(),
            definition: Definition::ranged(default, 0.0, 100.0).unwrap(),
            syncable,
            tooltip: horizon_world::attributes::Tooltip {
                sentiment,
                ..Default::default()
            },
        };
        let grown = catalog
            .with_attributes(&[
                declared("zeta", "zeta:fury", 15.0, false, Sentiment::default()),
                declared("ruby", "ruby:mana", 10.0, true, Sentiment::Negative),
            ])
            .unwrap();
        // After every vanilla attribute, sorted by mod id: ruby's first
        // though zeta's declared first, and vanilla ids never shift.
        let (mana, fury) = (
            grown.id("ruby:mana").unwrap(),
            grown.id("zeta:fury").unwrap(),
        );
        assert_eq!(mana.registry_id() as usize, vanilla);
        assert_eq!(fury.registry_id() as usize, vanilla + 1);
        assert_eq!(
            grown.id("minecraft:max_health"),
            catalog.id("minecraft:max_health")
        );
        // A mod entity's row names its own new attribute (#845's rows
        // resolve after the attributes join).
        let merged = grown
            .with_entity_row(
                "ruby:ghoul",
                "minecraft:zombie",
                &[("ruby:mana".into(), 42.0), ("zeta:fury".into(), 7.0)],
            )
            .unwrap();
        let live = LivingAttributes::default();
        assert_eq!(
            live.read(
                &std::sync::Arc::new(merged.clone()),
                "ruby:ghoul",
                "ruby:mana",
                ValueKind::Computed,
                Vec::new
            ),
            Some(42.0)
        );
        // An item's `attribute_modifiers` entry naming the mod attribute
        // reaches the live map (the worn-stack path's own write).
        let mut map =
            living_attributes::new_live(&std::sync::Arc::new(merged.clone()), "ruby:ghoul")
                .unwrap();
        assert_eq!(map_value(&mut map, mana), 42.0);
        living_attributes::set_modifier(&mut map, "ruby:mana", "ruby:ring", 8.0, "add_value", true)
            .unwrap();
        assert_eq!(map_value(&mut map, mana), 50.0);
        // The sync set: a syncable attribute joins it, a non-syncable one
        // never does (`AttributeMap.getSyncableAttributes`).
        living_attributes::set_modifier(&mut map, "zeta:fury", "zeta:mark", 1.0, "add_value", true)
            .unwrap();
        let synced: std::collections::HashSet<i32> = lan::attributes::syncable(&map)
            .iter()
            .map(|s| s.attribute)
            .collect();
        assert!(synced.contains(&mana.registry_id()));
        assert!(!synced.contains(&fury.registry_id()));
        // Save → load round trip by string id.
        let saved = Tag::List(living_attributes::pack(&map, true));
        let mut again =
            living_attributes::new_live(&std::sync::Arc::new(merged.clone()), "ruby:ghoul")
                .unwrap();
        living_attributes::apply_saved(&mut again, Some(&saved)).unwrap();
        // The saved base; the ring's transient modifier is equipment, not
        // saved — as vanilla's `AttributeInstance.Packed` keeps permanent
        // modifiers only.
        assert_eq!(map_value(&mut again, mana), 42.0);
        // A save whose attribute the loaded catalog does not know is
        // skipped, as vanilla warns and skips: no panic, the map whole.
        let mut vanilla_map =
            living_attributes::new_live(&std::sync::Arc::new(catalog.clone()), "minecraft:zombie")
                .unwrap();
        living_attributes::apply_saved(&mut vanilla_map, Some(&saved)).unwrap();
        assert!(lan::attributes::syncable(&vanilla_map).is_empty());
        // The network id decodes on a guest's catalog built from the same
        // mods declaring in another order: the snapshot's numbers find the
        // same attributes.
        let guest_catalog = catalog
            .with_attributes(&[
                declared("ruby", "ruby:mana", 10.0, true, Sentiment::Negative),
                declared("zeta", "zeta:fury", 15.0, false, Sentiment::default()),
            ])
            .unwrap()
            .with_entity_row(
                "ruby:ghoul",
                "minecraft:zombie",
                &[("ruby:mana".into(), 42.0), ("zeta:fury".into(), 7.0)],
            )
            .unwrap();
        let snapshots = lan::attributes::syncable(&map);
        let mut guest =
            living_attributes::new_live(&std::sync::Arc::new(guest_catalog), "ruby:ghoul").unwrap();
        lan::attributes::apply(&mut guest, &snapshots);
        assert_eq!(map_value(&mut guest, mana), 50.0);

        fn map_value(
            map: &mut horizon_client::living_attributes::Live,
            id: horizon_world::attributes::entity::AttributeId,
        ) -> f64 {
            // The first read creates the instance, as `getInstance` does.
            let key = map
                .keys()
                .iter()
                .find(|key| key.attribute() == id)
                .cloned()
                .unwrap();
            let Ok(Some(handle)) = map.get_instance(Some(key)) else {
                panic!("the type has the attribute")
            };
            map.instance(&handle).unwrap().value()
        }
    }

    /// A declared set's merged row — the template's with the declared set on
    /// top — is the type's one record: a content row's own value (the
    /// ghoul's 40 health) gives way to the template's, so a mob of the type
    /// spawns with what the live attribute map answers (#845, review).
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_declared_type_spawns_with_its_merged_row_not_its_content_row() {
        use horizon_client::mobs::Mob;
        use horizon_core::geometry::Vec3;
        use horizon_world::attributes::entity::Catalog;

        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let catalog = Catalog::from_json(
            &horizon_world::json_file::read(&root.join("attributes.json")).unwrap(),
        )
        .unwrap();
        // Only the speed is declared: the merged row keeps the template's
        // health (a zombie's 20), not the content row's 40.
        let declared = [("minecraft:movement_speed".to_string(), 0.4)];
        let merged = catalog
            .with_entity_row("ruby:ghoul", "minecraft:zombie", &declared)
            .unwrap();
        let mut types = living_mod_types();
        let ghoul = types.get_mut("ruby:ghoul").unwrap();
        assert_eq!(ghoul.max_health, 40.0, "the content row's own value");
        Arc::make_mut(ghoul)
            .take_attribute_row(Arc::new(merged.entity_row("ruby:ghoul").unwrap()), &merged);
        let ghoul = Mob::new(7, types["ruby:ghoul"].clone(), Vec3::ZERO, 0.0);
        assert_eq!(
            ghoul.health, 20.0,
            "the template's health through the merged row"
        );
        assert_eq!(ghoul.attrs.max_health, 20.0);
        assert_eq!(ghoul.attrs.speed, 0.4, "the declared speed");
        let live = horizon_client::living_attributes::LivingAttributes::default();
        assert_eq!(
            live.read(
                &std::sync::Arc::new(merged),
                "ruby:ghoul",
                "minecraft:max_health",
                horizon_world::attribute_map::ValueKind::Computed,
                Vec::new
            ),
            Some(20.0),
            "the live map answers the same row"
        );
    }

    /// A mod that adds an attribute vanilla zombies lack (#846,
    /// `EntityAttributeModificationEvent`) changes the zombie type's default:
    /// a zombie spawned after load answers it through the live attribute map
    /// and carries it in its finalize defaults, while the rest of its row
    /// and its spawn fields stay the zombie's.
    #[test]
    fn a_mods_attribute_addition_reaches_new_vanilla_zombies() {
        use horizon_client::mobs::Mob;
        use horizon_core::geometry::Vec3;
        use horizon_world::attribute_map::ValueKind;
        use horizon_world::attributes::entity::Catalog;
        use mod_api::{Hooks, ModEntry, Registrar, RegistrationContext};

        struct Lucky;
        #[mod_api::__subscriptions]
        impl Hooks for Lucky {
            fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
                registrar
                    .modify_entity_attributes(
                        "minecraft:zombie",
                        vec![("minecraft:luck".to_string(), Some(2.5))],
                    )
                    .unwrap();
            }
        }
        crate::game::api_values::install_parts(&[ModEntry::new("lucky", Box::new(Lucky))]).unwrap();

        let bits = |n: f64| format!("{:016x}", n.to_bits());
        let catalog = Arc::new(
            Catalog::from_json(&serde_json::json!({
                "data_version": horizon_world::DATA_VERSION,
                "attributes": [
                    {"id": "minecraft:max_health", "default": bits(20.0), "min": bits(1.0), "max": bits(1024.0), "syncable": true},
                    {"id": "minecraft:movement_speed", "default": bits(0.7), "min": bits(0.0), "max": bits(1024.0), "syncable": true},
                    {"id": "minecraft:luck", "default": bits(0.0), "min": bits(-1024.0), "max": bits(1024.0), "syncable": true}
                ],
                "entities": {
                    "minecraft:zombie": {
                        "minecraft:max_health": bits(20.0),
                        "minecraft:movement_speed": bits(0.23000000417232513)
                    }
                }
            }))
            .unwrap(),
        );
        let read = |catalog: &Arc<Catalog>, name: &str| {
            horizon_client::living_attributes::LivingAttributes::default().read(
                catalog,
                "minecraft:zombie",
                name,
                ValueKind::Computed,
                Vec::new,
            )
        };
        assert_eq!(
            read(&catalog, "minecraft:luck"),
            None,
            "vanilla zombies lack it"
        );

        // core.motion, then rest.mobs, as the resource load runs them.
        let modifications = mod_api::__private::entity_attribute_modifications();
        assert_eq!(modifications.len(), 1);
        let added = Arc::new(
            super::super::resource_load::with_mod_attribute_additions(&catalog, modifications)
                .unwrap(),
        );
        let mut types: std::collections::HashMap<_, _> = living_mod_types()
            .into_iter()
            .map(|(id, kind)| (id, Arc::unwrap_or_clone(kind)))
            .collect();
        super::super::resource_load::take_modified_attribute_rows(
            &mut types,
            &added,
            modifications,
        );

        let zombie = Mob::new(
            7,
            Arc::new(types["minecraft:zombie"].clone()),
            Vec3::ZERO,
            0.0,
        );
        assert_eq!(read(&added, "minecraft:luck"), Some(2.5), "the live map");
        assert_eq!(
            zombie.kind.attributes.get("minecraft:luck"),
            Some(&2.5),
            "the finalize defaults"
        );
        assert_eq!(read(&added, "minecraft:max_health"), Some(20.0));
        assert_eq!(zombie.health, 20.0, "the spawn's health");
        assert_eq!(zombie.attrs.speed, 0.23000000417232513);
        // Types nobody modified keep their rows.
        assert_eq!(
            types["minecraft:villager"].attributes.get("minecraft:luck"),
            None
        );
    }

    /// An addition the catalog cannot place (an attribute it does not know,
    /// or a type with no attribute row) refuses the resource load, naming
    /// the type.
    #[test]
    fn a_mods_attribute_addition_to_an_unknown_row_refuses_the_load() {
        use horizon_world::attributes::entity::Catalog;
        use mod_api::__private::EntityAttributeModification;

        let catalog = Catalog::from_json(&serde_json::json!({
            "data_version": horizon_world::DATA_VERSION,
            "attributes": [
                {"id": "minecraft:luck", "default": "0000000000000000", "min": "c090000000000000", "max": "4090000000000000", "syncable": true}
            ],
            "entities": {"minecraft:zombie": {}}
        }))
        .unwrap();
        let addition = |entity: &str, attribute: &str| EntityAttributeModification {
            entity: entity.into(),
            attributes: vec![(attribute.into(), None)],
        };
        let error = super::super::resource_load::with_mod_attribute_additions(
            &catalog,
            &[addition("minecraft:zombie", "lucky:magic_resistance")],
        )
        .err()
        .unwrap();
        assert!(error.contains("minecraft:zombie"), "{error}");
        assert!(error.contains("lucky:magic_resistance"), "{error}");
        let error = super::super::resource_load::with_mod_attribute_additions(
            &catalog,
            &[addition("ruby:ghoul", "minecraft:luck")],
        )
        .err()
        .unwrap();
        assert!(error.contains("ruby:ghoul"), "{error}");
    }

    /// A content mod's type from the wandering trader, the neutral template
    /// whose behaviour is all goals: a trader under its own id, with a
    /// trader's goals, that saves what a trader saves and loads back.
    #[test]
    fn a_mod_trader_type_is_a_wandering_trader_under_its_own_id() {
        use horizon_client::mobs::{Mob, Species};
        use horizon_core::nbt::Tag;
        let types = living_mod_types();
        let merchant = &types["ruby:merchant"];
        assert_eq!(merchant.species, Species::WanderingTrader);
        assert_eq!(merchant.class_id(), "minecraft:wandering_trader");
        let at = horizon_core::geometry::Vec3::new(0.5, 64.0, 0.5);
        let trader = Mob::new(1, types["minecraft:wandering_trader"].clone(), at, 0.0);
        let merchant = Mob::new(2, merchant.clone(), at, 0.0);
        assert!(merchant.ai.goals.has("Float") && trader.ai.goals.has("Float"));
        let saved =
            |m: &Mob| horizon_client::entity_nbt::mob_to_nbt(m, [0, 0, 0, 2], None, None, |_| None);
        let keys = |tag: &Tag| -> Vec<String> {
            let Tag::Compound(map) = tag else {
                panic!("a mob saves a compound");
            };
            map.keys().map(|k| k.to_string_lossy()).collect()
        };
        let tag = saved(&merchant);
        assert_eq!(
            keys(&tag),
            keys(&saved(&trader)),
            "it saves what a trader saves"
        );
        let loaded =
            horizon_client::entity_nbt::mob_from_nbt(&tag, &types, 3, None).expect("it loads back");
        assert_eq!(
            (loaded.kind.id.as_str(), loaded.species()),
            ("ruby:merchant", Species::WanderingTrader)
        );
    }

    #[test]
    fn fluid_models_recollection_changes_model_identifiers() {
        use mod_api::{FluidModel, Hooks, ModEntry, RegisterFluidModelsEvent};
        use mod_platform::testing::with_mods;
        use std::sync::atomic::{AtomicUsize, Ordering};
        #[derive(Default)]
        struct Models(Arc<AtomicUsize>);
        #[mod_api::horizon_mod]
        impl Hooks for Models {
            fn register_fluid_models(
                &self,
                _: &RegistrationContext,
                event: &mut RegisterFluidModelsEvent<'_>,
            ) {
                let generation = self.0.load(Ordering::SeqCst);
                let mut model = FluidModel::default();
                model.fluid = "reload_fluid:oil".into();
                model.still = Some(format!("reload_fluid:block/oil_{generation}"));
                event.models.push(model);
            }
        }
        let generation = Arc::new(AtomicUsize::new(0));
        with_mods(
            vec![ModEntry::new(
                "reload_fluid",
                Box::new(Models(generation.clone())),
            )]
            .into(),
            || {
                mod_platform::__private::with_test_packages(&["reload_fluid"], || {
                    horizon_client::fluid_models::set_world_looks([
                        &horizon_mod_package::content::FluidLook {
                            id: "reload_fluid:oil".into(),
                            sprite: None,
                            tint: None,
                            world: Some(Default::default()),
                        },
                    ]);
                    register_fluid_models(ExecutionRole::Client);
                    assert_eq!(
                        horizon_client::fluid_models::resolved()[0].still,
                        "reload_fluid:block/oil_0"
                    );
                    generation.store(1, Ordering::SeqCst);
                    register_fluid_models(ExecutionRole::Client);
                    assert_eq!(
                        horizon_client::fluid_models::resolved()[0].still,
                        "reload_fluid:block/oil_1"
                    );
                    // An item's fluid layer follows the same reload.
                    let item_still = || {
                        horizon_client::fluid_models::item_appearance("reload_fluid:oil")
                            .map(|appearance| appearance.still)
                    };
                    assert_eq!(item_still().as_deref(), Some("reload_fluid:block/oil_1"));
                    horizon_client::fluid_models::register(vec![]);
                    horizon_client::fluid_models::set_world_looks([]);
                    assert_eq!(item_still(), None);
                });
            },
        );
    }

    #[test]
    fn fluid_models_collect_only_on_the_client() {
        use mod_api::FluidModel;
        use mod_api::{Hooks, ModEntry, RegisterFluidModelsEvent};
        use mod_platform::testing::with_mods;
        #[derive(Default)]
        struct Models;
        #[mod_api::horizon_mod]
        impl Hooks for Models {
            fn register_fluid_models(
                &self,
                _: &RegistrationContext,
                event: &mut RegisterFluidModelsEvent<'_>,
            ) {
                let mut model = FluidModel::default();
                model.fluid = "registration_test:oil".into();
                model.flowing = Some("registration_test:block/oil_flow".into());
                model.overlay = Some("registration_test:block/oil_overlay".into());
                event.models.push(model);
            }
        }
        with_mods(
            vec![ModEntry::new("registration_test", Box::new(Models))].into(),
            || {
                mod_platform::__private::with_test_packages(&["registration_test"], || {
                    for server in [
                        ExecutionRole::IntegratedServer,
                        ExecutionRole::DedicatedServer,
                    ] {
                        assert_eq!(collect_fluid_models(server), None);
                    }
                    let models = collect_fluid_models(ExecutionRole::Client).unwrap();
                    assert_eq!(models.len(), 1);
                    assert_eq!(models[0].fluid, "registration_test:oil");
                    assert_eq!(
                        models[0].flowing.as_deref(),
                        Some("registration_test:block/oil_flow")
                    );
                    assert_eq!(
                        models[0].overlay.as_deref(),
                        Some("registration_test:block/oil_overlay")
                    );
                });
            },
        );
    }

    fn hmod(dir: &std::path::Path, id: &str) {
        let manifest = format!(
            "id = '{id}'\nname = '{id}'\nversion = '0.3.0'\napi = '0.2'\nside = 'both'\ncapabilities = ['registries']\n[native]\ngame = '26.3'\nhorizon = '{}'\nmixins = '{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        let files: &[(&str, &[u8])] = &[
            (
                "Cargo.toml",
                b"[package]\nname='different_crate'\nversion='9.9.9'\n",
            ),
            ("src/lib.rs", b"pub fn hooks() {}"),
        ];
        let bytes = horizon_mod_package::Package::write_files(&manifest, files).unwrap();
        std::fs::write(dir.join(format!("{id}.hmod")), bytes).unwrap();
    }

    #[test]
    #[ignore = "daily"]
    fn content_overlay_validates_dependencies_and_orders_layers() {
        const MODE: &str = "HORIZON_CONTENT_DEPENDENCY_TEST";
        if std::env::var_os(MODE).is_none() {
            run_in_child(
                "game::mods::tests::content_overlay_validates_dependencies_and_orders_layers",
                MODE,
            );
            return;
        }
        let f = Fixture::new("content-dependencies");
        let write = |id: &str, version: &str, dependencies: &str| {
            let dir = f.0.join(id);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("horizon-mod.toml"), format!(
                "id='{id}'\nname='{id}'\nversion='{version}'\napi='0.2'\nside='both'\ncapabilities=['registries']\n{dependencies}"
            )).unwrap();
            dir
        };
        let addon = write("alpha", "1", "[dependencies]\nzbase='^1'");
        let base_mod = write("zbase", "1", "");
        let base = f.0.join("resources");
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("horizon-resources.json"), "{}").unwrap();
        let mut sources = ModsState {
            dev: vec![addon.clone(), base_mod.clone()],
            ..Default::default()
        };
        let enabled = Default::default();
        sources.select(&f.0, &enabled);
        let overlay = content_overlay(&base, &sources, &f.0.join("jars")).unwrap();
        assert_eq!(
            overlay
                .mods
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            ["zbase", "alpha"]
        );
        sources.dev = vec![addon.clone()];
        sources.select(&f.0, &enabled);
        let error = content_overlay(&base, &sources, &f.0.join("jars"))
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("alpha") && error.contains("zbase"));
        write(
            "alpha",
            "1",
            "[dependencies]\nzbase={version='^1', optional=true}",
        );
        sources.select(&f.0, &enabled);
        assert!(content_overlay(&base, &sources, &f.0.join("jars")).is_ok());
        write("zbase", "2", "");
        sources.dev.push(base_mod.clone());
        sources.select(&f.0, &enabled);
        let error = content_overlay(&base, &sources, &f.0.join("jars"))
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("installed version is 2"));
        write("zbase", "1", "[dependencies]\nalpha='^1'");
        sources.select(&f.0, &enabled);
        assert!(
            content_overlay(&base, &sources, &f.0.join("jars"))
                .err()
                .unwrap()
                .to_string()
                .contains("cycle")
        );
    }

    #[test]
    #[ignore = "daily"]
    fn native_content_follows_the_build_and_misplaced_content_follows_its_selection() {
        const MODE: &str = "HORIZON_MODS_SELECTION_TEST";
        let mode = std::env::var(MODE).ok();
        if mode.is_none() {
            // Inventory and content registries are process-wide, as in a real startup.
            for mode in ["compiled", "vanilla"] {
                let status = std::process::Command::new(std::env::current_exe().unwrap())
                    .current_dir(std::env::temp_dir())
                    .args(["--include-ignored", "--exact", "game::mods::tests::native_content_follows_the_build_and_misplaced_content_follows_its_selection", "--nocapture"])
                    .env(MODE, mode).status().unwrap();
                assert!(status.success());
            }
            return;
        }
        let f = Fixture::new(mode.as_deref().unwrap());
        hmod(&f.0, "machine_sample");
        let native = if mode.as_deref() == Some("compiled") {
            f.runtime()
        } else {
            Runtime::default()
        };
        let here = f.0.join("unrelated-game");
        let misplaced = here.join(mod_list::NATIVE_DIR);
        std::fs::create_dir_all(&misplaced).unwrap();
        hmod(&misplaced, "machine_sample");
        let manifest = "id='plain'\nname='Plain'\nversion='1.0.0'\napi='0.2'\nside='both'\ncapabilities=['registries']\n";
        let plain = horizon_mod_package::Package::write_files(manifest, &[]).unwrap();
        std::fs::write(misplaced.join("plain.hmod"), plain).unwrap();
        let base = f.0.join("resources");
        std::fs::create_dir_all(&base).unwrap();
        // A prepared root's registry report: block entity registration reads it (B1).
        std::fs::write(
            base.join("registries.json"),
            "{\"minecraft:block_entity_type\":{\"entries\":{}}}",
        )
        .unwrap();
        std::fs::write(base.join("horizon-resources.json"), "{}").unwrap();
        std::fs::write(base.join("block-behaviours.json"), r#"{"fluids":[]}"#).unwrap();
        std::env::set_current_dir(&here).unwrap();
        let mut sources = ModsState {
            native: std::sync::Arc::new(native),
            ..Default::default()
        };
        let enabled = std::collections::BTreeSet::from(["plain".to_owned()]);
        sources.select(&here, &enabled);
        let generated = content_root(&base, &sources).unwrap();
        assert_ne!(generated.prepared(), base);
        assert_eq!(
            horizon_mod_package::enabled::EnabledMods::read(generated.prepared())
                .unwrap()
                .server_ids()
                .collect::<Vec<_>>(),
            ["plain"],
        );
        let local = horizon_protocol::mod_list::local();
        let ids: Vec<_> = local.iter().map(|m| m.id.as_str()).collect();
        if mode.as_deref() == Some("compiled") {
            assert_eq!(ids, ["machine_sample", "plain"]);
            assert_eq!(local[0].version, "0.3.0");
        } else {
            assert_eq!(ids, ["plain"]);
        }
        // Out of the fixture: Windows won't remove a process's working directory.
        std::env::set_current_dir(std::env::temp_dir()).unwrap();
    }

    #[test]
    #[ignore = "daily"]
    fn a_shared_missing_jar_stops_native_load_and_refuses_each_content_mod_row() {
        const MODE: &str = "HORIZON_JAR_REFUSAL_TEST";
        if std::env::var_os(MODE).is_none() {
            // Native declarations, like the real startup, belong to one process.
            run_in_child(
                "game::mods::tests::a_shared_missing_jar_stops_native_load_and_refuses_each_content_mod_row",
                MODE,
            );
            return;
        }
        let f = Fixture::new("shared-jar-refusal");
        let jar = horizon_mod_package::manifest::JarRef::new(
            "original".into(),
            "1".into(),
            "9".repeat(128),
        )
        .unwrap();
        for id in ["machine_sample", "alpha", "beta", "gamma"] {
            let native = id == "machine_sample";
            let mut text = format!(
                "id='{id}'\nname='{id}'\nversion='0.3.0'\napi='0.2'\nside='both'\ncapabilities=['registries']\n"
            );
            if native {
                text.push_str(&format!(
                    "[native]\ngame='26.3'\nhorizon='{}'\nmixins='{}'\n",
                    "a".repeat(40),
                    "b".repeat(40)
                ));
            }
            if id != "gamma" {
                text.push_str(&format!(
                    "[[jar]]\nid='original'\nversion='1'\nsha512='{}'\n",
                    jar.sha512()
                ));
            }
            let code: &[(&str, &[u8])] = if native {
                &[
                    ("Cargo.toml", b"[package]\nname='example'\nversion='1'\n"),
                    ("src/lib.rs", b"pub fn hooks() {}"),
                ]
            } else {
                &[]
            };
            std::fs::write(
                f.0.join(format!("{id}.hmod")),
                horizon_mod_package::Package::write_files(&text, code).unwrap(),
            )
            .unwrap();
        }
        let selected: Arc<[horizon_mod_package::selection::SelectedPackage]> =
            ["alpha", "beta", "gamma"]
                .into_iter()
                .map(|id| {
                    let path = f.0.join(format!("{id}.hmod"));
                    horizon_mod_package::selection::SelectedPackage {
                        package: horizon_mod_package::Package::read(&path).unwrap(),
                        path,
                    }
                })
                .collect();
        let native = ModsState {
            native: Arc::new(f.runtime()),
            selected: selected.clone(),
            ..Default::default()
        };
        let base = f.0.join("vanilla");
        let store = f.0.join("jars");
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("horizon-resources.json"), "{}").unwrap();
        let error = content_overlay(&base, &native, &store).err().unwrap();
        let missing = error.downcast_ref::<MissingJar>().unwrap();
        assert_eq!(missing.owners, ["machine_sample", "alpha", "beta"]);
        assert_eq!(missing.jar, jar);
        assert_eq!(missing.path, jar.cache_path(&store));
        assert!(!store.exists());

        let content = ModsState {
            selected,
            ..Default::default()
        };
        let overlay = content_overlay(&base, &content, &store).unwrap();
        assert_ne!(overlay.root.prepared(), base);
        assert_eq!(
            horizon_mod_package::enabled::EnabledMods::read(overlay.root.prepared())
                .unwrap()
                .server_ids()
                .collect::<Vec<_>>(),
            ["gamma"],
        );
        assert_eq!(
            overlay
                .mods
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            ["gamma"]
        );
        let mut rows: Vec<_> = ["alpha", "beta", "gamma"]
            .into_iter()
            .map(|id| Installed {
                id: id.into(),
                title: id.into(),
                description: String::new(),
                problem: None,
                dev: false,
                native: None,
            })
            .collect();
        content.report_refusals(&mut rows);
        for row in &rows[..2] {
            let problem = row.problem.as_deref().unwrap();
            assert!(problem.contains("alpha, beta: original 1:"));
            assert!(problem.contains(&jar.cache_path(&store).display().to_string()));
        }
        assert!(rows[2].problem.is_none());
    }

    #[test]
    fn the_crash_report_lists_the_embedded_native_mods() {
        let built = [
            BUILT[0],
            BuiltMod {
                id: "pipes",
                version: "2.0",
                side: mod_api::Side::Client,
            },
        ];
        assert_eq!(
            native_mods_detail(&built).as_deref(),
            Some("machine_sample 0.3.0, pipes 2.0")
        );
        assert_eq!(native_mods_detail(&[]), None);
    }

    #[test]
    fn native_mods_installed_but_not_built_get_a_row_saying_so() {
        let f = Fixture::new("not-built");
        for id in ["veinminer", "zoom", "minimap", "machine_sample"] {
            hmod(&f.0, id);
        }
        std::fs::write(f.0.join("notes.txt"), "not a mod").unwrap();
        std::fs::write(f.0.join("broken.hmod"), "not a zip").unwrap();
        let rows = not_built(&f.0, &[], &[]);
        let ids: Vec<_> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["machine_sample", "minimap", "veinminer", "zoom"]);
        assert_eq!(rows[2].title, "veinminer 0.3.0");
        assert_eq!(rows[2].problem.as_deref(), Some(mod_list::NOT_BUILT));
        assert!(rows.iter().all(|r| r.native.is_none() && !r.dev));
        let listed = Installed {
            id: "minimap".into(),
            ..rows[0].clone()
        };
        let rows = not_built(&f.0, BUILT, &[listed]);
        let ids: Vec<_> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["veinminer", "zoom"]);
        assert!(not_built(&f.0.join("missing"), &[], &[]).is_empty());
    }

    /// A root with mods that declare no mob effects leaves the registry open for the first one
    /// that does.
    #[test]
    fn a_root_without_mob_effects_does_not_freeze_the_registry() {
        use horizon_client::{brewing, mod_content::ContentMod};
        // Another test may have set the process-wide list already.
        if !brewing::mod_effects().is_empty() {
            return;
        }
        let f = Fixture::new("no-effects");
        std::fs::write(
            f.0.join("horizon-mod.toml"),
            "id = 'plain'\nname = 'plain'\nversion = '1.0'\napi = '0.2'\ncapabilities = ['registries']\n",
        )
        .unwrap();
        let package = horizon_mod_package::Package::read(&f.0).unwrap();
        let plain = ContentMod {
            id: "plain".into(),
            content: horizon_mod_package::content::Content::read(&f.0, &package.manifest).unwrap(),
            jars: Vec::new(),
        };
        register_mob_effects(&[plain]);
        // The list `potions::tests` registers, so it holds whichever test runs first.
        assert!(brewing::register_mod_effects([(
            "test:effect".to_owned(),
            0,
            brewing::Category::Neutral,
            None
        )]));
    }
}

#[cfg(test)]
mod concurrent_content_tests {
    use super::*;
    #[test]
    fn mods_read_at_once_keep_their_order_and_their_own_errors() {
        let dir = std::env::temp_dir().join(format!("horizon-read-at-once-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let paths: Vec<PathBuf> = (0..12).map(|i| dir.join(format!("m{i}"))).collect();
        for (i, path) in paths.iter().enumerate() {
            std::fs::create_dir_all(path).unwrap();
            std::fs::write(
                path.join("horizon-mod.toml"),
                format!(
                    "id = \"m{i}\"\nname = \"M\"\nversion = \"0.1.0\"\napi = \"0.2\"\n\
                     side = \"both\"\ncapabilities = [\"registries\"]\n"
                ),
            )
            .unwrap();
            // Mod i has i + 1 language files; mod 5 also writes into another mod's namespace.
            let lang = path.join(format!("assets/m{i}/lang"));
            std::fs::create_dir_all(&lang).unwrap();
            for n in 0..=i {
                std::fs::write(lang.join(format!("l{n}.json")), "{}").unwrap();
            }
            if i == 5 {
                std::fs::create_dir_all(path.join("assets/m0/lang")).unwrap();
                std::fs::write(path.join("assets/m0/lang/x.json"), "{}").unwrap();
            }
        }
        let packages: Vec<(&PathBuf, horizon_mod_package::Package)> = paths
            .iter()
            .map(|p| (p, horizon_mod_package::Package::read(p).unwrap()))
            .collect();
        let requests: Vec<_> = packages
            .iter()
            .map(|(path, package)| (path.as_path(), &package.manifest, Vec::new()))
            .collect();
        let read = read_contents(&requests);
        assert_eq!(read.len(), 12);
        for (i, content) in read.iter().enumerate() {
            if i == 5 {
                assert!(content.as_ref().unwrap_err().contains("own namespaces"));
                continue;
            }
            let files = &content.as_ref().unwrap().files;
            assert_eq!(files.len(), i + 1, "mod {i}");
            assert!(
                files
                    .iter()
                    .all(|f| f.starts_with(&format!("assets/m{i}/")))
            );
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
