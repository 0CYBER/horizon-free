//! The game's shared read-only tables: `Resources` (blocks, items, recipes, mob types, biomes),
//! `UiResources` and `PackResources`.
use super::*;
use std::path::Path;

pub(super) fn load_animate_tables(
    root: &Path,
    registry: &horizon_world::registry::BlockRegistry,
    draws: bool,
    biomes: &Biomes,
) -> Result<(
    horizon_client::animate_tick::AnimateTables,
    horizon_client::fog::FogTables,
)> {
    let _step = startup::step("rest.animate_fog");
    let animate = if draws {
        horizon_client::animate_tick::AnimateTables::load(root, registry)?
    } else {
        Default::default()
    };
    Ok((animate, horizon_client::fog::FogTables::load(root, biomes)))
}

impl Resources {
    pub(super) fn rest(&self) -> &WorldRest {
        self.rest
            .get()
            .expect("world resources joined before entry")
    }
    pub(super) fn facts(&self) -> &BlockFacts {
        &self.core.facts
    }
    pub(super) fn item_defaults(&self) -> &horizon_world::item_defaults::ItemDefaults {
        self.core.item_data.defaults()
    }
    pub(super) fn items(&self) -> &Items {
        &self.rest().items
    }
    pub(super) fn recipes(&self) -> &Recipes {
        &self.rest().recipes
    }
    pub(super) fn transformers(&self) -> &horizon_client::block_transform::Transformers {
        &self.rest().transformers
    }
    pub(super) fn biomes(&self) -> &Arc<Biomes> {
        &self.core.biomes
    }
    pub(super) fn environment(&self) -> &Environment {
        &self.core.environment
    }
    /// The environment of a dimension type id; a type with no table of its own gets the overworld's.
    pub(super) fn environment_of(&self, dimension_type: &str) -> &Environment {
        match dimension_type {
            "minecraft:the_nether" => self.nether_environment(),
            "minecraft:the_end" => self.end_environment(),
            _ => self.environment(),
        }
    }
    pub(super) fn mob_types(&self) -> &HashMap<String, Arc<MobType>> {
        &self.rest().mob_types
    }
    pub(super) fn mob_data(&self) -> &horizon_client::mobs::MobData {
        &self.rest().mob_data
    }
    pub(super) fn providers(&self) -> &Providers {
        &self.rest().providers
    }
    pub(super) fn block_entity_render(&self) -> &BlockEntityRenderer {
        &self.rest().block_entity_render
    }
    pub(super) fn animate(&self) -> &horizon_client::animate_tick::AnimateTables {
        &self.rest().animate
    }
    pub(super) fn fog(&self) -> &horizon_client::fog::FogTables {
        &self.rest().fog
    }
    pub(super) fn music(&self) -> &horizon_client::music::MusicTables {
        &self.rest().music
    }
    pub(super) fn ambient(&self) -> &horizon_client::ambience::AmbientTables {
        &self.rest().ambient
    }
    pub(super) fn nether_environment(&self) -> &Environment {
        &self.rest().nether_environment
    }
    pub(super) fn end_environment(&self) -> &Environment {
        &self.rest().end_environment
    }
    pub(super) fn behaviours(&self) -> &Behaviours {
        &self.rest().behaviours
    }
    pub(super) fn collision(&self) -> &BlockCollisionRegistry<'static> {
        &self.core.collision
    }
    pub(super) fn suffocation(&self) -> &horizon_world::suffocation::SuffocationRegistry<'static> {
        &self.core.suffocation
    }
    pub(super) fn motion(&self) -> &MotionResources {
        &self.core.motion
    }
    /// The variant registries finalizeSpawn selects from
    /// (`data/minecraft/<registry>`), shared with world generation.
    pub fn spawn_registries(&self) -> &horizon_client::mobs::finalize::SpawnRegistries {
        &self.core.spawn_registries
    }
    pub(super) fn threads(&self) -> usize {
        self.core.threads
    }
    pub(super) fn player_arm(&self, slim: bool, left: bool) -> &Option<horizon_client::mobs::Part> {
        let arms = if slim {
            &self.rest().player_arm_slim
        } else {
            &self.rest().player_arm
        };
        &arms[usize::from(left)]
    }
    pub(super) fn player_type(&self) -> &Option<Arc<horizon_client::mobs::MobType>> {
        &self.rest().player_type
    }
    pub(super) fn spin_attack(&self) -> &Option<horizon_client::mobs::Part> {
        &self.rest().spin_attack
    }
    pub(super) fn stations(&self) -> &horizon_client::stations::StationData {
        &self.rest().stations
    }
}

/// Everything the title and menus draw: the GUI atlas with its font, the
/// title panorama and the splash pool. Kept apart from the world bundle so the
/// title can appear before the block sprites, models and gameplay tables are ready.
pub struct UiResources {
    /// The GUI, font, particle, map and skin atlas.
    pub atlas: HudAtlas,
    /// The title screen's panorama faces.
    pub panorama: Arc<horizon_client::gfx::panorama::Faces>,
    /// The title screen's splashes (`horizon_client::splashes::pool`).
    pub splashes: Arc<str>,
}

impl Game<'_> {
    /// A title screen over the current resources and their splash pool.
    pub(crate) fn new_title(&self, saves: PathBuf) -> Title {
        Title::new(saves, &self.options.resources, &self.gui.splashes)
    }
}

/// Whether the HUD atlas packs the mod image slot (`horizon/modimage`): whenever a client mod is
/// loaded, native or wasm, since any of them can call `ClientHost::mod_image`, with no mixin and
/// no opt-in (arismoko/horizon#4437). A game with no mod pays none of its 4 MiB. Read where the atlas is built:
/// at the first load and on a resource reload.
pub(in crate::game) fn mod_image_slot() -> bool {
    !mod_api::__private::mods().is_empty()
}

impl UiResources {
    /// The resolved resource generation, including the base layer and pack filters.
    /// Its splash pool is empty until `with_splashes`.
    pub fn load(root: &horizon_assets::resource_source::ResourceSource) -> Result<Self> {
        // The title screen waits for both: the panorama's six faces decode
        // beside the atlas.
        std::thread::scope(|scope| {
            let panorama = named_scope(scope, "panorama", || {
                startup::time("ui.panorama", || {
                    horizon_client::gfx::panorama::Faces::load(root, root)
                })
            });
            let mod_image = mod_image_slot();
            let atlas = startup::time("ui.atlas", || {
                HudAtlas::build_over_with_image(root, root, mod_image)
            });
            let panorama = panorama
                .join()
                .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
            Ok(Self {
                atlas: atlas?,
                panorama: Arc::new(panorama),
                splashes: Arc::from(""),
            })
        })
    }
    /// The title's splash pool from `stack` (`horizon_client::splashes::pool`).
    pub fn with_splashes(
        mut self,
        stack: &horizon_assets::resource_source::ResourceSource,
        vanilla: &std::path::Path,
        skip_if_modified: bool,
    ) -> Self {
        self.splashes = horizon_client::splashes::pool(stack, vanilla, skip_if_modified).into();
        self
    }
    /// What a server's game holds, since it draws nothing: the font, which books
    /// and chat measure with, and a one-pixel panorama.
    pub fn server(root: &horizon_assets::resource_source::ResourceSource) -> Result<Self> {
        Ok(Self {
            atlas: HudAtlas::fonts(root)?,
            panorama: Arc::new(horizon_client::gfx::panorama::Faces::colour([0; 4])),
            splashes: Arc::from(""),
        })
    }
}

impl PackResources {
    /// Everything a pack overlay at `root` provides, over `assets` and `render`
    /// loaded from it.
    pub(super) fn load(
        root: &horizon_assets::resource_source::ResourceRoot,
        assets: Arc<Assets>,
        render: Arc<RenderAssets>,
        facts: &BlockFacts,
        biomes: Arc<Biomes>,
        light: Arc<LightTables>,
        item_data: &horizon_world::item_defaults::ItemData,
    ) -> Result<Self> {
        let visibility = models::load_visibility(root.prepared(), assets.registry)?;
        api_values::resolve_block_definitions(assets.registry)?;
        mods::extend_poi_types(assets.registry);
        mods::register_cauldron_fluid_contents(assets.registry);
        let tables = Arc::new(
            MeshTables::new(
                &assets,
                &render,
                facts,
                visibility,
                api_values::mod_tint_values(assets.registry)?,
            )?
            .with_polytone_colormaps(root.source(), assets.registry),
        );
        let item_renderer = horizon_client::item_render::ItemRenderer::load_with_library(
            root,
            render.model_library.clone(),
            item_data,
        )?;
        Ok(Self::assemble(
            root,
            assets,
            render,
            tables,
            item_renderer,
            biomes,
            light,
        ))
    }
    /// The workers' context over `biomes`: this pack's own when `biomes` is
    /// the table it was built with, else the same parts with that table.
    pub(super) fn context_with(&self, biomes: Arc<Biomes>) -> Arc<WorkerContext> {
        let old = &self.context;
        if Arc::ptr_eq(&old.biomes, &biomes) {
            return old.clone();
        }
        Arc::new(WorkerContext {
            assets: old.assets.clone(),
            render: old.render.clone(),
            tables: old.tables.clone(),
            biomes,
            light: old.light.clone(),
            hidden_states: old.hidden_states.clone(),
            part_tables: old.part_tables.clone(),
        })
    }
    /// The pack tables over their parts, which `load_resources` builds on
    /// separate workers.
    pub(super) fn assemble(
        root: &horizon_assets::resource_source::ResourceRoot,
        assets: Arc<Assets>,
        render: Arc<RenderAssets>,
        tables: Arc<MeshTables>,
        item_renderer: horizon_client::item_render::ItemRenderer,
        biomes: Arc<Biomes>,
        light: Arc<LightTables>,
    ) -> Self {
        // These pack-local resources do not depend on one another. Decode the
        // images and language map together so a custom-sky pack cannot put all
        // of the pack assembly on the critical path behind its own files.
        let (names, clouds, forcefield, custom_sky, environments, polytone_particles) =
            std::thread::scope(|scope| {
                let names = named_scope(scope, "pack.names", || Names::load(root.source()));
                let clouds = named_scope(scope, "pack.clouds", || load_clouds(root.source()));
                let forcefield =
                    named_scope(scope, "pack.forcefield", || load_forcefield(root.source()));
                let custom_sky = named_scope(scope, "pack.custom_sky", || {
                    horizon_client::custom_sky::load(root.source())
                });
                let environments =
                    named_scope(scope, "pack.environments", || pack_environments(root));
                let particles = named_scope(scope, "pack.particles", || {
                    pack_particles(root, assets.registry)
                });
                (
                    names.join().unwrap(),
                    clouds.join().unwrap(),
                    forcefield.join().unwrap(),
                    custom_sky.join().unwrap(),
                    environments.join().unwrap(),
                    particles.join().unwrap(),
                )
            });
        let polytone_post = horizon_client::polytone_post::PolytonePost::read(
            root.source(),
            horizon_client::packs::polytone_configs(&horizon_client::packs::Source::Opened(
                root.source().clone(),
            )),
        )
        .map(Arc::new);
        Self {
            layer_names: Arc::new(horizon_client::block_sprites::texture_names(
                &render.sprite_layers,
            )),
            context: Arc::new(WorkerContext {
                assets: assets.clone(),
                render: render.clone(),
                tables,
                biomes,
                light,
                hidden_states: Arc::from([]),
                part_tables: Default::default(),
            }),
            block_registries: native_block_states::block_registries(assets.registry),
            assets,
            render,
            item_renderer: {
                let renderer = Arc::new(item_renderer);
                // The pack's own item models baked on a worker, before they show in the world. It
                // holds the renderer weakly, so an unloaded pack stops it, and starts only when
                // there is work: a pack without models of its own leaves the renderer unshared.
                let names = renderer.pack_model_names();
                if !names.is_empty() {
                    let ahead = Arc::downgrade(&renderer);
                    let _ = std::thread::Builder::new()
                        .name("Item models".into())
                        .spawn(move || {
                            horizon_client::item_render::ItemRenderer::bake_ahead(&ahead, names)
                        });
                }
                renderer
            },
            source: root.source().clone(),
            data_source: root.data().clone(),
            data_changes: root.data_changes().clone(),
            additional_data: Arc::from([]),
            names: Arc::new(names),
            clouds: Arc::new(clouds),
            forcefield: Arc::new(forcefield),
            custom_sky,
            environments,
            polytone_particles,
            polytone_post,
            polytone_colors: Arc::new(read_colors(root)),
        }
    }
}

/// The overworld's, Nether's and End's attribute stacks with the Polytone biome and dimension
/// modifiers of the packs merged at `root` (#11486); `None` when no pack has any. The overlay
/// root links the vanilla data the stacks start from.
fn pack_environments(
    root: &horizon_assets::resource_source::ResourceRoot,
) -> Option<Arc<[Environment; 3]>> {
    let modifiers = horizon_client::environment::PackModifiers::read(root.source());
    if modifiers.is_empty() {
        return None;
    }
    let modifiers = modifiers.with_configs(horizon_client::packs::polytone_configs(
        &horizon_client::packs::Source::Opened(root.source().clone()),
    ));
    let load = |d: &str| Environment::load_dimension_with(root.data(), d, &modifiers);
    match (load("overworld"), load("the_nether"), load("the_end")) {
        (Ok(o), Ok(n), Ok(e)) => Some(Arc::new([o, n, e])),
        (o, n, e) => {
            for error in [o.err(), n.err(), e.err()].into_iter().flatten() {
                log::warn!("pack environment: {error}");
            }
            None
        }
    }
}

/// The custom particles and block emitters of the packs merged at `root` (#11486); `None` when
/// no pack has any.
fn pack_particles(
    root: &horizon_assets::resource_source::ResourceRoot,
    blocks: &'static horizon_world::registry::BlockRegistry,
) -> Option<Arc<horizon_client::particles::polytone::Registry>> {
    let configs = horizon_client::packs::polytone_configs(&horizon_client::packs::Source::Opened(
        root.source().clone(),
    ));
    let registry =
        horizon_client::particles::polytone::Registry::read(root.source(), configs, blocks);
    (!registry.is_empty()).then(|| Arc::new(registry))
}

/// The packs' custom colours; their dye and sheep colours go to the draw paths' table.
fn read_colors(
    root: &horizon_assets::resource_source::ResourceRoot,
) -> horizon_client::pack_palette::PackColors {
    let colors = horizon_client::pack_palette::read(root.source());
    horizon_client::pack_palette::set(&colors);
    horizon_client::brewing::set_pack_colors(colors.effects.clone());
    colors
}
