use super::{HudAtlas, MISSING_SPRITE, Rect};

/// The packed key of a GUI sprite id: `minecraft:boss_bar/pink_background` is
/// `gui/sprites/boss_bar/pink_background`, another namespace's `ns:gui/sprites/...`.
fn atlas_key_of_gui_sprite(id: &str) -> String {
    match id.split_once(':') {
        Some(("minecraft", path)) => format!("gui/sprites/{path}"),
        Some((namespace, path)) => format!("{namespace}:gui/sprites/{path}"),
        None => format!("gui/sprites/{id}"),
    }
}

impl HudAtlas {
    /// `AtlasGlyphProvider`: the sprite `sprite` of the atlas named `atlas`
    /// (`minecraft:gui` keeps its sprites under `gui/sprites/`), or the missing
    /// sprite when that atlas has none by that name.
    pub fn inline_sprite(&self, atlas: &str, sprite: &str) -> Option<Rect> {
        let plain = sprite.strip_prefix("minecraft:").unwrap_or(sprite);
        let gui = atlas_key_of_gui_sprite(sprite);
        let gui_atlas = atlas.strip_prefix("minecraft:").unwrap_or(atlas) == "gui";
        let (first, second) = if gui_atlas {
            (gui.as_str(), plain)
        } else {
            (plain, gui.as_str())
        };
        self.sprites
            .get(first)
            .or_else(|| self.sprites.get(second))
            .or_else(|| self.sprites.get(MISSING_SPRITE))
            .copied()
    }
    /// `inline_sprite` for an inline atlas object; `None` for any other object.
    pub(super) fn inline_object_sprite(&self, object: &crate::text::InlineObject) -> Option<Rect> {
        match object {
            crate::text::InlineObject::Atlas { atlas, sprite, .. } => {
                self.inline_sprite(atlas, sprite)
            }
            _ => None,
        }
    }
}
