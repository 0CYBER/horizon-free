//! The selected Horizon shader pack (`shaderPack` in `horizon-graphics.txt`,
//! docs/shader-packs.md): loaded from `shaderpacks/`, given to the renderer,
//! and loaded again when the selection changes, on F3 + T, or when its files
//! change (checked twice a second). A pack that fails leaves the last good
//! one (or the vanilla picture) on and says why in chat.
use super::Game;
use horizon_client::shader_pack::{self, ShaderPack, Stamp};
use horizon_client::text::Text;
use horizon_shaderpack::properties::SkyDirectives;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Default)]
pub(super) struct ShaderPacks {
    /// The pack the renderer runs.
    pack: Option<ShaderPack>,
    /// The selection and its files at the last load, success or not.
    selected: String,
    stamp: Stamp,
    checked: Option<Instant>,
}

impl ShaderPacks {
    /// The loaded pack's folder name.
    pub(super) fn id(&self) -> Option<&str> {
        self.pack.as_ref().map(|p| p.id.as_str())
    }
    /// What a loaded Java pack changes in the vanilla sky of `dimension`
    /// (0 overworld, -1 the Nether, 1 the End): its `sunPathRotation`, and
    /// whether it keeps the clouds, sun and moon (Iris's sky mixins).
    pub(super) fn java_sky(&self, dimension: i8) -> Option<(f32, SkyDirectives)> {
        let java = self.pack.as_ref()?.java.as_ref()?;
        let folder = &java.folders[java.folder_for(dimension)?];
        Some((folder.directives.sun_path_rotation, java.sky))
    }
}

pub(super) fn dir() -> PathBuf {
    shader_pack::folder()
}

impl Game<'_> {
    /// Load the selected pack and give it to the renderer.
    pub(super) fn load_shader_pack(&mut self) {
        let id = self.settings.horizon.shader_pack.clone();
        let dir = dir();
        let state = &mut self.shader_packs;
        state.selected = id.clone();
        state.checked = Some(Instant::now());
        let pack = if id.is_empty() {
            state.stamp = Stamp::default();
            None
        } else {
            state.stamp = Stamp::of(&dir.join(&id));
            // Java packs are told which GPU they run on.
            let (vendor, renderer) = self.renderer.as_ref().map_or(("Unknown", ""), |r| {
                let name = r.info.name.as_str();
                (
                    horizon_client::java_pack::vendor_name(r.info.vendor, name),
                    name,
                )
            });
            let values = shader_pack::load_values(&dir, &id);
            match ShaderPack::load_any(&dir.join(&id), &values, vendor, renderer) {
                Ok(pack) => Some(pack),
                Err(e) => return self.shader_pack_error(&id, &e),
            }
        };
        if let Some(r) = self.renderer.as_mut()
            && let Err(e) = r.set_shader_pack(pack.as_ref())
        {
            return self.shader_pack_error(&id, &e.to_string());
        }
        self.shader_packs.pack = pack;
        self.ui.options.shader_pack_error = None;
    }
    /// Give the loaded pack to the renderer (also after it is rebuilt).
    pub(super) fn apply_shader_pack(&mut self) -> horizon_client::Result<()> {
        match self.renderer.as_mut() {
            Some(r) => r.set_shader_pack(self.shader_packs.pack.as_ref()),
            None => Ok(()),
        }
    }
    /// Every frame: a new selection or edited pack files load it again.
    pub(super) fn poll_shader_pack(&mut self) {
        let state = &mut self.shader_packs;
        if self.settings.horizon.shader_pack != state.selected {
            return self.load_shader_pack();
        }
        if state.selected.is_empty()
            || state
                .checked
                .is_some_and(|t| t.elapsed() < Duration::from_millis(500))
        {
            return;
        }
        state.checked = Some(Instant::now());
        if Stamp::of(&dir().join(&state.selected)) != state.stamp {
            self.load_shader_pack();
        }
    }
    fn shader_pack_error(&mut self, id: &str, error: &str) {
        log::warn!("Horizon: shader pack {id}: {error}");
        // The Shader Packs page says why on the pack's row: the first line,
        // since translator errors go on with a drawn source excerpt.
        let first = error.lines().map(str::trim).find(|l| !l.is_empty());
        self.ui.options.shader_pack_error =
            Some((id.to_owned(), first.unwrap_or(error).to_owned()));
        self.system_message(Text::literal(format!("Shader pack {id}: {error}")));
    }
}
