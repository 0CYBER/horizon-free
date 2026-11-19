//! Resolution fallback for model textures that exceed the shared HUD atlas.
use super::{
    HashMap, HashSet, HudAtlas, PackedImage, ResourceSource, Result, bitmap_font_textures,
};

/// The asset namespaces the mods that are on own: only their HUD textures may
/// lose resolution, so a game without mods draws its resource packs as before.
static MOD_NAMESPACES: std::sync::RwLock<Vec<String>> = std::sync::RwLock::new(Vec::new());

/// Sets the namespaces the mods that are on own (the content root does, as it
/// loads them); returns the list it replaced, for a test to put back.
pub fn set_mod_namespaces(namespaces: Vec<String>) -> Vec<String> {
    std::mem::replace(
        &mut *MOD_NAMESPACES.write().unwrap_or_else(|e| e.into_inner()),
        namespaces,
    )
}

/// The namespaces `set_mod_namespaces` gave; none before it.
pub(super) fn mod_namespaces() -> HashSet<String> {
    let names = MOD_NAMESPACES.read().unwrap_or_else(|e| e.into_inner());
    names.iter().cloned().collect()
}

/// Only model atlas sprites may lose resolution. Font sheets can also be
/// model textures, so protect both their file names and renamed sources.
pub(super) fn reducible_model_sprites(
    root: &ResourceSource,
    images: &[PackedImage],
    bordered: &HashSet<String>,
) -> HashSet<String> {
    use crate::atlas_sources::{self, SpriteOrigin, unasked};
    let fonts: HashSet<_> = bitmap_font_textures(root).into_iter().collect();
    let protected = |name: &str| {
        let path = name.split_once(':').map_or(name, |(_, path)| path);
        path.starts_with("gui/")
            || path.starts_with("font/")
            || fonts.contains(&atlas_sources::texture_resource(name, "textures"))
    };
    let mut models: HashSet<String> = images
        .iter()
        .filter(|(name, ..)| name.starts_with("item/") || name.starts_with("block/"))
        .map(|(name, ..)| name.clone())
        .collect();
    let textures = atlas_sources::list_textures(root);
    let mut protected_aliases = HashSet::new();
    for (atlas, sources) in atlas_sources::atlases(root) {
        if atlas != "minecraft:blocks" && atlas != "minecraft:items" {
            continue;
        }
        for (id, origin) in atlas_sources::list(&sources, &textures, &mut unasked) {
            let name = id.strip_prefix("minecraft:").unwrap_or(&id).to_owned();
            if let SpriteOrigin::File(texture) = origin {
                if protected(&texture) {
                    protected_aliases.insert(name);
                } else {
                    models.insert(name);
                }
            }
        }
    }
    models.retain(|name| {
        !bordered.contains(name) && !protected(name) && !protected_aliases.contains(name)
    });
    models
}

/// The images [`HudAtlas::layout_capping_mod_textures`] leaves at full size
/// besides vanilla's: font sheets, which glyph widths are measured on, and
/// blurred sprites with their wrapped border.
pub(super) fn full_size_sprites(
    root: &ResourceSource,
    images: &[PackedImage],
    bordered: &HashSet<String>,
) -> HashSet<String> {
    let fonts: HashSet<_> = bitmap_font_textures(root).into_iter().collect();
    images
        .iter()
        .map(|(name, ..)| name)
        .filter(|name| {
            bordered.contains(*name)
                || fonts.contains(&crate::atlas_sources::texture_resource(name, "textures"))
        })
        .cloned()
        .collect()
}

impl HudAtlas {
    /// `layout`, but when `extras` (Custom Item Textures) make the images more than `max`
    /// holds, without them: they are left out, not the packs.
    pub(super) fn layout_leaving_out(
        images: &mut Vec<PackedImage>,
        animated: &mut Vec<String>,
        extras: &HashSet<String>,
        max: u32,
    ) -> Result<(u32, Vec<(u32, u32)>, u32)> {
        match Self::layout(images, max) {
            Err(e) if !extras.is_empty() => {
                images.retain(|i| !extras.contains(&i.0));
                animated.retain(|n| !extras.contains(n));
                let layout = Self::layout(images, max)?;
                log::warn!("left out the custom item textures: {e}");
                Ok(layout)
            }
            layout => layout,
        }
    }
    /// Last resort for a pack that exceeds the shared atlas: halve model
    /// textures until they fit, retaining their logical GUI sizes. Never
    /// shrink run-time slots (empty pixels), or below the vanilla 16 texels.
    pub(super) fn layout_reducing_models(
        images: &mut [PackedImage],
        gui_sizes: &mut HashMap<String, (u32, u32)>,
        models: &HashSet<String>,
        max: u32,
    ) -> Result<(u32, Vec<(u32, u32)>, u32)> {
        loop {
            let error = match Self::layout(images, max) {
                Ok(layout) => return Ok(layout),
                Err(error) => error,
            };
            let mut changed = false;
            for (name, w, h, pixels) in images.iter_mut() {
                if !models.contains(name) || pixels.is_empty() || (*w <= 16 && *h <= 16) {
                    continue;
                }
                let (nw, nh) = ((*w / 2).max((*w).min(16)), (*h / 2).max((*h).min(16)));
                gui_sizes.entry(name.clone()).or_insert((*w, *h));
                *pixels = scaled(pixels, (*w, *h), (nw, nh));
                (*w, *h) = (nw, nh);
                changed = true;
            }
            if !changed {
                return Err(error);
            }
            images.sort_by(|a, b| b.2.cmp(&a.2).then(b.1.cmp(&a.1)));
        }
    }
    /// When the model textures at their smallest still don't fit (a modpack's
    /// GUI and entity textures: All the Mods 10 brings over 200 million texels
    /// of GUI alone), cap the longer side of every other texture of a mod's
    /// namespace (`mods`), halving the cap until the atlas holds them: the
    /// largest lose resolution first and keep their logical GUI size, as the
    /// models do. Vanilla's textures, Horizon's, a resource pack's own
    /// namespaces, `keep` (font sheets, blurred sprites) and run-time slots stay
    /// as they are, and below 16 texels nothing shrinks.
    pub(super) fn layout_capping_mod_textures(
        images: &mut [PackedImage],
        gui_sizes: &mut HashMap<String, (u32, u32)>,
        keep: &HashSet<String>,
        mods: &HashSet<String>,
        max: u32,
    ) -> Result<(u32, Vec<(u32, u32)>, u32)> {
        let capped = |name: &str, pixels: &[u8]| {
            !pixels.is_empty()
                && !keep.contains(name)
                && name.split_once(':').is_some_and(|(ns, _)| {
                    ns != "minecraft" && ns != "horizon" && mods.contains(ns)
                })
        };
        let mut cap = images
            .iter()
            .filter(|(name, _, _, pixels)| capped(name, pixels))
            .map(|(_, w, h, _)| (*w).max(*h))
            .max()
            .unwrap_or(0);
        loop {
            let error = match Self::layout(images, max) {
                Ok(layout) => return Ok(layout),
                Err(error) => error,
            };
            // Halve the cap down to 16 texels, which is tried too, and no further.
            if cap <= 16 {
                return Err(error);
            }
            cap = (cap / 2).max(16);
            let mut changed = false;
            for (name, w, h, pixels) in images.iter_mut() {
                let side = (*w).max(*h);
                if side <= cap || !capped(name, pixels) {
                    continue;
                }
                // In 64 bits: a side times the cap passes `u32` for a very long particle strip.
                let fit = |v: u32| (u64::from(v) * u64::from(cap) / u64::from(side)).max(1) as u32;
                let (nw, nh) = (fit(*w), fit(*h));
                gui_sizes.entry(name.clone()).or_insert((*w, *h));
                *pixels = scaled(pixels, (*w, *h), (nw, nh));
                (*w, *h) = (nw, nh);
                changed = true;
            }
            if changed {
                images.sort_by(|a, b| b.2.cmp(&a.2).then(b.1.cmp(&a.1)));
            }
        }
    }
}

/// `pixels` (`from`, RGBA) at `to`, nearest texel.
fn scaled(pixels: &[u8], from: (u32, u32), to: (u32, u32)) -> Vec<u8> {
    let ((w, h), (nw, nh)) = (from, to);
    let mut out = Vec::with_capacity((nw * nh * 4) as usize);
    for y in 0..nh {
        for x in 0..nw {
            let (y, x) = (u64::from(y), u64::from(x));
            let row = y * u64::from(h) / u64::from(nh);
            let at = ((row * u64::from(w) + x * u64::from(w) / u64::from(nw)) * 4) as usize;
            out.extend_from_slice(&pixels[at..at + 4]);
        }
    }
    out
}
