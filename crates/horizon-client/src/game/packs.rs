//! Resource pack selection bound to the game: the pack screen lists and
//! reloading after a change (atlas, fonts, GUI metadata, languages and item
//! icons live; block textures and models from the next start).
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
    /// Rebuild the overlay and reload what can change while running.
    pub(super) fn reload_packs(&mut self) {
        let vanilla = self.vanilla_root();
        if self.ui.vanilla_root.as_os_str().is_empty() {
            self.ui.vanilla_root = vanilla.clone();
        }
        let root = startup_root(&vanilla, &self.settings.resource_packs);
        self.options.resources = root.clone();
        horizon_client::ui::load_metadata(&root);
        match HudAtlas::build(&root) {
            Ok(mut atlas) => {
                atlas.font.uniform = self.settings.force_unicode;
                atlas.font.japanese = self.settings.japanese_glyphs;
                let same_size =
                    atlas.width == self.res.atlas.width && atlas.height == self.res.atlas.height;
                if same_size {
                    if let Some(texture) = self
                        .renderer
                        .as_ref()
                        .and_then(|r| r.hud.atlas_texture.clone())
                        && let Some(r) = self.renderer.as_ref()
                    {
                        r.queue.write_texture(
                            wgpu::TexelCopyTextureInfo {
                                texture: &texture,
                                mip_level: 0,
                                origin: wgpu::Origin3d::ZERO,
                                aspect: wgpu::TextureAspect::All,
                            },
                            &atlas.pixels,
                            wgpu::TexelCopyBufferLayout {
                                offset: 0,
                                bytes_per_row: Some(atlas.width * 4),
                                rows_per_image: None,
                            },
                            wgpu::Extent3d {
                                width: atlas.width,
                                height: atlas.height,
                                depth_or_array_layers: 1,
                            },
                        );
                    }
                    self.res.atlas = atlas;
                    match horizon_client::item_render::ItemRenderer::load(&root) {
                        Ok(r) => self.res.item_renderer = std::sync::Arc::new(r),
                        Err(e) => eprintln!("Horizon: item model reload failed: {e}"),
                    }
                } else {
                    eprintln!(
                        "Horizon: resource pack atlas size changed; restart to apply GUI textures"
                    );
                }
            }
            Err(e) => eprintln!("Horizon: resource pack reload failed: {e}"),
        }
        let code = lang::code();
        lang::set(lang::Language::load(&[&root], &code));
        self.ui.chat.rescale(&self.res.atlas);
    }
}
