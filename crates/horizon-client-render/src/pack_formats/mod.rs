//! The resource pack formats vanilla doesn't read (OptiFine's, Polytone's) on the engine's
//! extension points ([`crate::pack_hooks`]). This is platform code (docs/layers.md) kept in this
//! crate until the platform's executable installs it: it uses only the crate's public API, so it
//! moves to a platform crate unchanged, and nothing else in the crate names it.

mod colors;
mod custom_sky;
mod item_bars;

use crate::resource_source::ResourceSource;
use std::sync::Arc;

use crate::pack_hooks::PackFormats;

/// OptiFine's and Polytone's formats.
pub struct Formats;

impl PackFormats for Formats {
    fn colors(&self, root: &ResourceSource) -> Option<crate::pack_palette::PackColors> {
        Some(colors::read(root))
    }

    fn emissive_suffix(&self, root: &ResourceSource) -> Option<String> {
        Some(horizon_optifine::emissive::suffix(root))
    }

    fn custom_sky(
        &self,
        root: &crate::resource_source::ResourceSource,
    ) -> Option<Arc<dyn crate::custom_sky::SkyLayers>> {
        let sky = custom_sky::CustomSky::load(root);
        (!sky.is_empty()).then(|| Arc::new(sky) as _)
    }

    fn item_bars(&self, root: &ResourceSource) -> Option<Arc<dyn crate::pack_palette::ItemBars>> {
        item_bars::read(root)
    }
}

/// Installs these formats ([`crate::pack_hooks::install`]); the game does at startup.
pub fn install() {
    crate::pack_hooks::install(&Formats);
}
