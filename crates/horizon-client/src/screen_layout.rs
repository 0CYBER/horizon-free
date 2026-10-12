//! The loom and crafter screens' own drawing as `GuiGraphicsExtractor` calls:
//! `LoomScreen.extractBackground` (slot icons, the scroller, the pattern
//! buttons with their banner swatches and tooltip, the result banner or the
//! six-layer error) and `CrafterScreen`'s background, disabled slots and
//! redstone arrow. Computed without drawing so `tools/test_screen_layout_parity.py`
//! can compare them with the vanilla client; the game paints the same calls.
use crate::banner::{self, BannerLayer, Banners};
use std::sync::Arc;

/// One call a screen makes.
#[derive(Clone, Debug, PartialEq)]
pub enum ScreenDraw {
    /// `fillGradient(0, 0, width, height, ..)`: the dimmed world behind a menu.
    Gradient { x1: i32, y1: i32 },
    /// `blit(texture, x, y, 0, 0, w, h, tw, th)`: a container background.
    Blit {
        texture: &'static str,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        tw: i32,
        th: i32,
    },
    /// `blitSprite(sprite, x, y, w, h)` of a GUI sprite.
    Sprite {
        id: String,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
    },
    /// `bannerPattern(flag, base, layers, x0, y0, x1, y1)`: the result preview.
    Banner {
        base: String,
        layers: Vec<BannerLayer>,
        x0: i32,
        y0: i32,
        x1: i32,
        y1: i32,
    },
    /// `setTooltipForNextFrame` with a translation key.
    Tooltip { key: String },
    /// `getSprite(id)`: the atlas sprite the following `Atlas` blit uses.
    SpriteOf { id: String },
    /// `fill(x0, y0, x1, y1, argb)`.
    Fill {
        x0: i32,
        y0: i32,
        x1: i32,
        y1: i32,
        color: i32,
    },
    /// `blit(atlas, x0, y0, x1, y1, u0, u1, v0, v1)` with the UVs as fractions
    /// of the sprite named by the last `SpriteOf`.
    Atlas {
        x0: i32,
        y0: i32,
        x1: i32,
        y1: i32,
        uv: [f32; 4],
    },
}

/// What `LoomScreen.extractBackground` reads.
pub struct LoomView<'a> {
    pub width: i32,
    pub height: i32,
    pub left: i32,
    pub top: i32,
    pub has_banner: bool,
    pub has_dye: bool,
    pub has_pattern: bool,
    /// `menu.getSelectablePatterns()`.
    pub patterns: &'a [Arc<str>],
    /// `menu.getSelectedBannerPatternIndex()`.
    pub selected: i32,
    pub display: bool,
    pub has_max: bool,
    pub scroll: f32,
    pub start_row: usize,
    /// The result banner's colour and layers (`resultBannerPatterns`).
    pub result: Option<(&'a str, &'a [BannerLayer])>,
    /// The dye stack's colour (`DataComponents.DYE`, white without one).
    pub dye: &'a str,
    pub mouse: (f32, f32),
}

/// `DyeColor.getTextureDiffuseColor` as an opaque ARGB int.
fn diffuse(dye: &str) -> i32 {
    (banner::dye_rgb(dye) | 0xFF00_0000) as i32
}

fn sprite(id: &str, x: i32, y: i32, w: i32, h: i32) -> ScreenDraw {
    ScreenDraw::Sprite {
        id: id.to_owned(),
        x,
        y,
        w,
        h,
    }
}

/// `LoomScreen.extractBackground`.
pub fn loom(v: &LoomView, banners: &Banners) -> Vec<ScreenDraw> {
    let (xo, yo) = (v.left, v.top);
    let mut out = vec![
        ScreenDraw::Gradient {
            x1: v.width,
            y1: v.height,
        },
        ScreenDraw::Blit {
            texture: "textures/gui/container/loom.png",
            x: xo,
            y: yo,
            w: 176,
            h: 166,
            tw: 256,
            th: 256,
        },
    ];
    for (has, icon, x, y) in [
        (v.has_banner, "banner", 13, 26),
        (v.has_dye, "dye", 33, 26),
        (v.has_pattern, "banner_pattern", 23, 45),
    ] {
        if !has {
            out.push(sprite(
                &format!("container/slot/{icon}"),
                xo + x,
                yo + y,
                16,
                16,
            ));
        }
    }
    let sy = (41.0 * v.scroll) as i32;
    let active = v.display && v.patterns.len() > 16;
    out.push(sprite(
        if active {
            "container/loom/scroller"
        } else {
            "container/loom/scroller_disabled"
        },
        xo + 119,
        yo + 13 + sy,
        12,
        15,
    ));
    match v.result {
        Some((base, layers)) if !v.has_max => out.push(ScreenDraw::Banner {
            base: base.to_owned(),
            layers: layers.to_vec(),
            x0: xo + 141,
            y0: yo + 8,
            x1: xo + 161,
            y1: yo + 48,
        }),
        _ if v.has_max => out.push(sprite("container/loom/error", xo + 138, yo + 52, 26, 26)),
        _ => {}
    }
    if v.display {
        let (x, y) = (xo + 60, yo + 13);
        'rows: for row in 0..4 {
            for column in 0..4 {
                let index = (row + v.start_row) * 4 + column;
                let Some(pattern) = v.patterns.get(index) else {
                    break 'rows;
                };
                let (px, py) = (x + column as i32 * 14, y + row as i32 * 14);
                let (mx, my) = v.mouse;
                let highlighted = mx >= px as f32
                    && my >= py as f32
                    && mx < (px + 14) as f32
                    && my < (py + 14) as f32;
                let button = if index as i32 == v.selected {
                    "container/loom/pattern_selected"
                } else if highlighted {
                    out.push(ScreenDraw::Tooltip {
                        key: banners.layer_key(&BannerLayer::new(pattern, v.dye)),
                    });
                    "container/loom/pattern_highlighted"
                } else {
                    "container/loom/pattern"
                };
                out.push(sprite(button, px, py, 14, 14));
                // extractBannerOnButton: a grey 5x10 swatch with the pattern's front.
                out.push(ScreenDraw::SpriteOf {
                    id: banners.banner_texture(pattern),
                });
                let (bx, by) = (px + 4, py + 2);
                out.push(ScreenDraw::Fill {
                    x0: bx,
                    y0: by,
                    x1: bx + 5,
                    y1: by + 10,
                    color: diffuse("gray"),
                });
                let v0 = 1.0 / 64.0;
                out.push(ScreenDraw::Atlas {
                    x0: bx,
                    y0: by,
                    x1: bx + 5,
                    y1: by + 10,
                    uv: [0.0, 21.0 / 64.0, v0, v0 + 40.0 / 64.0],
                });
            }
        }
    }
    out
}

/// What `CrafterScreen` draws besides the generic container parts.
pub struct CrafterView {
    pub width: i32,
    pub height: i32,
    pub left: i32,
    pub top: i32,
    pub disabled: [bool; 9],
    pub powered: bool,
}

/// `CrafterScreen.extractBackground`, the disabled slots of `extractSlot`
/// and `extractRedstone`.
pub fn crafter(v: &CrafterView) -> Vec<ScreenDraw> {
    let mut out = vec![
        ScreenDraw::Gradient {
            x1: v.width,
            y1: v.height,
        },
        ScreenDraw::Blit {
            texture: "textures/gui/container/crafter.png",
            x: (v.width - 176) / 2,
            y: (v.height - 166) / 2,
            w: 176,
            h: 166,
            tw: 256,
            th: 256,
        },
    ];
    for (i, disabled) in v.disabled.iter().enumerate() {
        if *disabled {
            let (x, y) = (26 + (i % 3) as i32 * 18, 17 + (i / 3) as i32 * 18);
            out.push(sprite(
                "container/crafter/disabled_slot",
                v.left + x - 1,
                v.top + y - 1,
                18,
                18,
            ));
        }
    }
    out.push(sprite(
        if v.powered {
            "container/crafter/powered_redstone"
        } else {
            "container/crafter/unpowered_redstone"
        },
        v.width / 2 + 9,
        v.height / 2 - 48,
        16,
        16,
    ));
    out
}
