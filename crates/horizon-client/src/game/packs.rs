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
pub fn startup_root(vanilla: &Path, selected: &[String]) -> std::io::Result<PathBuf> {
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
/// Shared by the startup menu, which has no world resources yet.
pub(super) fn pack_infos(vanilla: &Path) -> Vec<PackInfo> {
    let mut infos: Vec<PackInfo> = packs::discover(vanilla, &packs_dir())
        .iter()
        .map(info)
        .collect();
    infos.push(vanilla_info());
    infos
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
        let infos = pack_infos(&self.vanilla_root());
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
        // Pack reload updates tail-owned mob metadata. Keep menus responsive
        // until those resources exist, then apply the latest selection once.
        if self.rest_loader.is_some() || self.world_loader.is_some() {
            self.pending_pack_reload = true;
            return;
        }
        let vanilla = self.vanilla_root();
        if self.ui.vanilla_root.as_os_str().is_empty() {
            self.ui.vanilla_root = vanilla.clone();
        }
        let result = startup_root(&vanilla, &self.settings.resource_packs)
            .map_err(Into::into)
            .and_then(|root| self.reload_resources(&root));
        if let Err(e) = result {
            log::warn!("Horizon: resource reload failed: {e}");
            if self.screen != super::Screen::Options {
                self.open_options_at(
                    self.title.is_some(),
                    horizon_client::options_ui::Page::Packs,
                );
            }
            self.ui.options.resource_error = Some(e.to_string());
        }
    }
    fn reload_resources(&mut self, root: &Path) -> horizon_client::Result<()> {
        let render = std::sync::Arc::new(self.pack.assets.reload(root)?);
        let mut atlas = HudAtlas::build(root)?;
        atlas.font.uniform = self.settings.force_unicode;
        atlas.font.japanese = self.settings.japanese_glyphs;
        let gui = super::UiResources {
            atlas,
            panorama: std::sync::Arc::new(horizon_client::gfx::panorama::Faces::load(root)),
        };
        let pack = super::PackResources::load(
            root,
            self.pack.assets.clone(),
            render,
            self.res.facts(),
            self.pack.context.biomes.clone(),
            self.pack.context.light.clone(),
        )?;
        // Keep the working GPU resources, but release its exclusive window surface.
        if let Some(renderer) = &mut self.renderer {
            renderer.release_surface();
        }
        let renderer = match self
            .window
            .clone()
            .map(|window| self.renderer_for(window, &pack, &gui))
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
        self.random_tick_sections.clear();
        self.portals.sections.clear();
        self.options.resources = root.to_path_buf();
        let context = pack.context.clone();
        self.pack = pack;
        self.gui = gui;
        horizon_client::ui_theme::reset_atlas();
        // Villager hats come from the pack's texture metadata (shared data, swapped in place).
        *self
            .res
            .mob_data()
            .villager_hats
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            horizon_client::mobs::villager_hats::VillagerHats::load(root);
        horizon_client::ui::load_metadata(root);
        lang::set(lang::Language::load(&[&root.to_path_buf()], &lang::code()));
        self.audio.reload(root);
        self.clear_lod();
        self.streaming.reload_resources(&mut self.world, context);
        self.pending_uploads.clear();
        // New GPU textures: a renderer built from the reloaded resources; the
        // sections are uploaded again as they are meshed.
        if let Some(renderer) = renderer {
            self.renderer = Some(renderer);
            self.apply_ui_settings();
        }
        self.ui.chat.rescale(&self.gui.atlas);
        Ok(())
    }
}
