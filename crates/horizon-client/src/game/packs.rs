//! Resource pack selection bound to the game: the pack screen lists, and a
//! full resource reload after a change, while playing.
use super::Game;
use horizon_client::{
    hud::HudAtlas,
    lang,
    pack_screen::{Compat, PackInfo, Source as PackSource},
    packs::{self, Pack, Position},
    text::{Style, Text},
};
use std::path::{Path, PathBuf};

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
/// `Util.getPlatform().openPath`: the system file manager on the folder.
pub fn open_folder(dir: &Path) {
    let program = if cfg!(target_os = "windows") {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    if let Err(e) = std::process::Command::new(program).arg(dir).spawn() {
        log::warn!("Horizon: could not open {}: {e}", dir.display());
    }
}
/// The resource root to load from, given the vanilla root and the selection.
pub fn startup_root(vanilla: &Path, selected: &[String]) -> PathBuf {
    let all = packs::discover(vanilla, &packs_dir());
    let enabled = packs::enabled(&all, selected);
    packs::resource_root(vanilla, &enabled, &cache_dir())
}
fn info(p: &Pack) -> PackInfo {
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
        icon: p.icon,
        required: false,
        position: Position::Top,
        fixed: false,
    }
}
/// The vanilla pack: required, at the bottom (`BuiltInPackSource`).
fn vanilla_info() -> PackInfo {
    PackInfo {
        id: "vanilla".into(),
        title: Text::translate("resourcePack.vanilla.name", &[]),
        description: Text::translate("resourcePack.vanilla.description", &[]),
        description_style: Style::default(),
        source: PackSource::BuiltIn,
        compat: Compat::Compatible,
        icon: true,
        required: true,
        position: Position::Bottom,
        fixed: false,
    }
}
impl Game<'_> {
    fn vanilla_root(&self) -> PathBuf {
        if self.ui.vanilla_root.as_os_str().is_empty() {
            self.options.resources.clone()
        } else {
            self.ui.vanilla_root.clone()
        }
    }
    /// The packs the pack screen offers and the current selection
    /// (`PackSelectionScreen` makes its `PackSelectionModel` from them).
    pub(super) fn refresh_pack_lists(&mut self) {
        let all = packs::discover(&self.vanilla_root(), &packs_dir());
        let mut infos: Vec<PackInfo> = all.iter().map(info).collect();
        infos.push(vanilla_info());
        let o = &mut self.ui.options;
        o.pack_infos = infos;
        o.pack_selection = self.settings.resource_packs.clone();
        o.pack_screen = None;
    }
    /// `Minecraft.reloadResourcePacks`: rebuild the overlay and reload every
    /// client resource from it, as vanilla's reload listeners do: block
    /// models and sprites (every section is meshed again,
    /// `LevelRenderer.allChanged`), the shared atlas (GUI, item, entity,
    /// particle and font textures), item models, sounds, languages, GUI
    /// metadata and the cloud and world border textures.
    pub(super) fn reload_packs(&mut self) {
        let vanilla = self.vanilla_root();
        if self.ui.vanilla_root.as_os_str().is_empty() {
            self.ui.vanilla_root = vanilla.clone();
        }
        let root = startup_root(&vanilla, &self.settings.resource_packs);
        if let Err(e) = self.reload_resources(&root) {
            log::warn!("Horizon: resource reload failed: {e}");
        }
    }
    fn reload_resources(&mut self, root: &Path) -> horizon_client::Result<()> {
        let assets = std::sync::Arc::new(self.res.assets.reload(root)?);
        let mut atlas = HudAtlas::build(root)?;
        atlas.font.uniform = self.settings.force_unicode;
        atlas.font.japanese = self.settings.japanese_glyphs;
        let item_renderer = horizon_client::item_render::ItemRenderer::load(root)?;
        let visibility = horizon_client::models::load_visibility(root, assets.registry)?;
        let tables = std::sync::Arc::new(horizon_client::mesher::MeshTables::new(
            &assets,
            &self.res.facts,
            visibility,
        )?);
        // Everything loaded: switch over.
        self.options.resources = root.to_path_buf();
        let context = std::sync::Arc::new(horizon_client::streaming::WorkerContext {
            assets: assets.clone(),
            tables,
            biomes: self.res.context.biomes.clone(),
            light: self.res.context.light.clone(),
        });
        self.res.layer_names = assets
            .sprite_layers
            .iter()
            .map(|(name, layer)| (*layer, name.trim_start_matches("minecraft:").to_owned()))
            .collect();
        self.res.assets = assets;
        self.res.context = context.clone();
        self.res.atlas = atlas;
        self.res.item_renderer = std::sync::Arc::new(item_renderer);
        self.res.clouds = super::load_clouds(root);
        self.res.forcefield = super::load_forcefield(root);
        self.res.names = super::Names::load(root);
        horizon_client::ui::load_metadata(root);
        lang::set(lang::Language::load(&[&root.to_path_buf()], &lang::code()));
        self.audio.reload(root);
        self.streaming.reload_resources(&mut self.world, context);
        self.pending_uploads.clear();
        // New GPU textures: a renderer built from the reloaded resources; the
        // sections are uploaded again as they are meshed.
        if let Some(window) = self.window.clone() {
            self.renderer = None;
            self.renderer = Some(self.new_renderer(window)?);
            self.apply_ui_settings();
        }
        self.ui.chat.rescale(&self.res.atlas);
        Ok(())
    }
}
