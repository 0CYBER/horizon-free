//! `EffectsInInventory`: active effect panels beside survival and creative
//! inventories, with compact icons when there is little room on the right.
use std::borrow::Cow;

use horizon_client_core::text::Text;

use crate::{
    effects::EffectInstance,
    hud::{HudAtlas, HudBatch},
    hud_overlays::{EffectIcon, compare_effects, java_sort},
};

#[derive(Debug, PartialEq)]
pub struct EffectPanel<'a> {
    /// The effect the panel shows.
    pub effect: &'a EffectInstance,
    pub icon: EffectIcon,
    pub name: String,
    pub duration: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub spacing: i32,
}

/// Where the effect panels may go: the window's width, the container's
/// origin (including recipe book shifts) and its image width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EffectsSpace {
    pub screen_width: i32,
    pub origin: [i32; 2],
    pub image_width: i32,
}

impl EffectsSpace {
    fn left(&self) -> i32 {
        self.origin[0] + self.image_width + 2
    }

    /// The width right of the container (`availableSpace`); vanilla draws
    /// nothing below 32.
    pub fn available(&self) -> i32 {
        self.screen_width - self.left()
    }
}

/// How the panels are laid out: compact icons or full panels, shifted
/// `offset` GUI pixels right, or `hidden` (a native mod drew its own).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EffectsLayout {
    pub compact: bool,
    pub offset: i32,
    pub hidden: bool,
}

impl EffectsLayout {
    /// Vanilla's: full panels from 120 GUI pixels of room, compact below.
    pub fn vanilla(available: i32) -> Self {
        Self {
            compact: available < 120,
            offset: 0,
            hidden: false,
        }
    }
}

/// Lay out panels as `layout` says. Vanilla hides them when fewer than 32 GUI
/// pixels remain.
pub fn panels<'a>(
    space: &EffectsSpace,
    layout: &EffectsLayout,
    effects: &'a [EffectInstance],
    tick_rate: f32,
    translate: &dyn Fn(&str) -> Option<String>,
    text_width: &dyn Fn(&str) -> i32,
) -> Vec<EffectPanel<'a>> {
    let available = space.available();
    if effects.is_empty() || available < 32 || layout.hidden {
        return Vec::new();
    }
    let x = space.left() + layout.offset;
    let origin = space.origin;
    let max_width = if layout.compact { 32 } else { available - 7 };
    let spacing = if effects.len() > 5 {
        132 / (effects.len() as i32 - 1)
    } else {
        33
    };
    let mut sorted: Vec<_> = effects.iter().map(|e| (e, EffectIcon::from(e))).collect();
    java_sort(&mut sorted, |a, b| compare_effects(&a.1, &b.1));
    sorted
        .into_iter()
        .enumerate()
        .map(|(i, (effect, icon))| {
            let mut name = translate(&horizon_client_core::potions::description_id(effect.id))
                .unwrap_or_else(|| effect.id.to_owned());
            if (1..=9).contains(&effect.amplifier) {
                let level = effect.amplifier + 1;
                let suffix = translate(&format!("enchantment.level.{level}"))
                    .unwrap_or_else(|| crate::enchant::level_name(level as u32));
                name = format!("{name} {suffix}");
            }
            let duration = if effect.duration == -1 {
                translate("effect.duration.infinite").unwrap_or_else(|| "∞".to_owned())
            } else {
                // StringUtil.formatTickDuration uses the level's current rate.
                crate::brewing::format_tick_duration(effect.duration, tick_rate)
            };
            let width = max_width.min(39 + text_width(&name).max(text_width(&duration)));
            EffectPanel {
                effect,
                icon,
                name,
                duration,
                x,
                y: origin[1] + i as i32 * spacing,
                width,
                spacing,
            }
        })
        .collect()
}

/// The inventory effect background has a four-pixel nine-slice border. Since
/// its height stays 32, only horizontal slicing/tiling is needed.
fn background(batch: &mut HudBatch, atlas: &HudAtlas, panel: &EffectPanel) {
    let name = if panel.icon.ambient {
        "gui/sprites/container/inventory/effect_background_ambient"
    } else {
        "gui/sprites/container/inventory/effect_background"
    };
    let Some(rect) = atlas.sprites.get(name).copied() else {
        return;
    };
    let mut strip = |dx: i32, sx: i32, width: i32| {
        let mut r = rect;
        let du = rect.u1 - rect.u0;
        r.u0 = rect.u0 + du * sx as f32 / 32.0;
        r.u1 = r.u0 + du * width as f32 / 32.0;
        batch.rect_uv(
            (panel.x + dx) as f32,
            panel.y as f32,
            width as f32,
            32.0,
            r,
            u32::MAX,
        );
    };
    strip(0, 0, 4);
    let mut dx = 4;
    while dx < panel.width - 4 {
        let width = 24.min(panel.width - 4 - dx);
        strip(dx, 4, width);
        dx += width;
    }
    strip(panel.width - 4, 28, 4);
}

/// A rendered panel under the cursor, whether or not vanilla gives it tooltip lines.
pub struct HoveredEffectPanel<'p, 'a> {
    /// The effect widget under the cursor.
    pub panel: &'p EffectPanel<'a>,
    /// Compact or clipped widgets show their full name and duration in a tooltip.
    pub has_default_tooltip: bool,
}

/// Paint panels and return the hovered panel with its default-tooltip eligibility.
/// Its hover height follows the spacing of overlapping rows.
pub fn paint<'p, 'a>(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    panels: &'p [EffectPanel<'a>],
    cursor: [i32; 2],
) -> Option<HoveredEffectPanel<'p, 'a>> {
    let mut hovered = None;
    let mut icon = String::new();
    for panel in panels {
        background(batch, atlas, panel);
        let text_width = panel.width - 39;
        let clipped = atlas.font.width(&panel.name).ceil() as i32 > text_width;
        if text_width > 0 {
            let shown: Cow<'_, str> = if clipped {
                let available = text_width as f32 - atlas.font.width("...");
                let count = atlas.font.fit(&panel.name, available);
                let text: String = panel.name.chars().take(count).collect();
                Cow::Owned(format!("{text}..."))
            } else {
                Cow::Borrowed(panel.name.as_str())
            };
            batch.text(
                atlas,
                &shown,
                (panel.x + 32) as f32,
                (panel.y + 7) as f32,
                u32::MAX,
                true,
            );
            batch.text(
                atlas,
                &panel.duration,
                (panel.x + 32) as f32,
                (panel.y + 16) as f32,
                0xff808080,
                true,
            );
        }
        icon.clear();
        horizon_client_core::potions::push_sprite(&mut icon, &panel.icon.id);
        if let Some(r) = atlas.sprite_or_missing(&icon) {
            let (x, y) = ((panel.x + 7) as f32, (panel.y + 7) as f32);
            batch.rect_uv(x, y, 18.0, 18.0, r, u32::MAX);
        }
        if cursor[0] >= panel.x
            && cursor[0] <= panel.x + panel.width
            && cursor[1] >= panel.y
            && cursor[1] <= panel.y + panel.spacing
        {
            hovered = Some(HoveredEffectPanel {
                panel,
                has_default_tooltip: text_width <= 0 || clipped,
            });
        }
    }
    hovered
}

/// A hovered panel's default tooltip: the full name then duration for compact
/// or clipped panels, no lines when its information is fully visible.
pub fn tooltip(hovered: HoveredEffectPanel<'_, '_>) -> Vec<(Text, u32)> {
    if !hovered.has_default_tooltip {
        return Vec::new();
    }
    let panel = hovered.panel;
    vec![
        (Text::literal(&panel.name), u32::MAX),
        (Text::literal(&panel.duration), u32::MAX),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(width: i32, effects: &[EffectInstance]) -> Vec<EffectPanel<'_>> {
        let space = EffectsSpace {
            screen_width: width,
            origin: [72, 37],
            image_width: 176,
        };
        panels(
            &space,
            &EffectsLayout::vanilla(space.available()),
            effects,
            20.0,
            &|key| match key {
                "effect.minecraft.levitation" => Some("Levitation".into()),
                "effect.minecraft.speed" => Some("Speed".into()),
                _ => None,
            },
            &|text| text.chars().count() as i32 * 6,
        )
    }

    #[test]
    fn inventory_labels_levels_and_durations() {
        let effects = [
            EffectInstance::new("levitation", -1, 0),
            EffectInstance::new("speed", 450, 1),
        ];
        let p = layout(480, &effects);
        assert_eq!(p.len(), 2);
        // Natural order puts shorter durations before infinite effects.
        assert_eq!(
            (&p[0].name, &p[0].duration),
            (&"Speed II".into(), &"00:22".into())
        );
        assert_eq!(
            (&p[1].name, &p[1].duration),
            (&"Levitation".into(), &"∞".into())
        );
        assert_eq!((p[0].x, p[0].y, p[0].width), (250, 37, 87));
        assert_eq!((p[1].x, p[1].y, p[1].width), (250, 70, 99));
    }

    #[test]
    fn narrow_windows_hide_or_compact_the_panels() {
        let effects = [EffectInstance::new("levitation", -1, 0)];
        assert!(layout(281, &effects).is_empty());
        assert_eq!(layout(282, &effects)[0].width, 32);
        assert_eq!(layout(369, &effects)[0].width, 32);
        assert_eq!(layout(370, &effects)[0].width, 99);
        assert!(layout(480, &[]).is_empty());
    }

    #[test]
    fn crowded_effects_overlap_and_preserve_ambient_order() {
        let mut effects: Vec<_> = (0..7)
            .map(|i| EffectInstance::new("speed", (i + 1) * 100, 0))
            .collect();
        effects[0].ambient = true;
        let p = layout(480, &effects);
        assert_eq!(p[0].icon.duration, 200);
        assert!(p.last().unwrap().icon.ambient);
        assert_eq!(
            p.iter().map(|p| p.y).collect::<Vec<_>>(),
            [37, 59, 81, 103, 125, 147, 169]
        );
    }

    #[test]
    fn creative_origin_recipe_book_shift_and_tick_rate() {
        let effects = [EffectInstance::new("speed", 72000, 10)];
        let space = EffectsSpace {
            screen_width: 640,
            origin: [222, 43],
            image_width: 195,
        };
        let vanilla = EffectsLayout::vanilla(space.available());
        let p = panels(&space, &vanilla, &effects, 10.0, &|_| None, &|_| 60);
        assert_eq!((p[0].x, p[0].y), (419, 43));
        assert_eq!(p[0].name, "speed");
        assert_eq!(p[0].duration, "02:00:00");
        let space = EffectsSpace {
            screen_width: 480,
            origin: [149, 37],
            image_width: 176,
        };
        let vanilla = EffectsLayout::vanilla(space.available());
        let p = panels(&space, &vanilla, &effects, 20.0, &|_| None, &|_| 150);
        assert_eq!((p[0].x, p[0].width), (327, 146));
        // A mod's layout: compact icons, nudged right; hidden draws none.
        let moved = EffectsLayout {
            compact: true,
            offset: 5,
            hidden: false,
        };
        let p = panels(&space, &moved, &effects, 20.0, &|_| None, &|_| 150);
        assert_eq!((p[0].x, p[0].width), (332, 32));
        let hidden = EffectsLayout {
            hidden: true,
            ..moved
        };
        assert!(panels(&space, &hidden, &effects, 20.0, &|_| None, &|_| 150).is_empty());
    }
}
