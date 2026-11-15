//! Moved here from the game so mods can name it.
#[allow(unused_imports)]
use crate::*;
use std::borrow::Cow;
/// A `Matrix3x2fStack` operation applied to a copy of the current pose.
#[derive(Clone, Copy, Debug, PartialEq)]
// exhaustive: a closed set the game matches without a wildcard
#[derive(horizon_mod_api_macros::Wire)]
pub enum PoseOp {
    Translate(f32, f32),
    Scale(f32, f32),
}
/// The render pipeline of a texture blit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: a closed set the game matches without a wildcard
#[derive(horizon_mod_api_macros::Wire)]
pub enum Blend {
    /// `GUI_TEXTURED`.
    Textured,
    /// `VIGNETTE`: `dst * (1 - src)`.
    Vignette,
    /// `GUI_NAUSEA_OVERLAY`: additive.
    Nausea,
    /// `CROSSHAIR`: `BlendFunction.INVERT`, the colour under it inverted.
    Crosshair,
}
impl Blend {
    pub fn pipeline(self) -> &'static str {
        match self {
            Blend::Textured => "pipeline/gui_textured",
            Blend::Vignette => "pipeline/vignette",
            Blend::Nausea => "pipeline/gui_nausea_overlay",
            Blend::Crosshair => "pipeline/crosshair",
        }
    }
}
/// One `GuiGraphicsExtractor` call.
/// Text deeper than 512 nodes refuses the whole command buffer at the game boundary.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: a closed set the game matches without a wildcard
#[derive(horizon_mod_api_macros::Wire)]
pub enum GuiDraw {
    /// `blitSprite(pipeline, sprite, x, y, w, h, color)`; `region` =
    /// (spriteW, spriteH, u, v) for the partial overload.
    Sprite {
        sprite: Cow<'static, str>,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        region: Option<[i32; 4]>,
        color: u32,
        blend: Blend,
    },
    /// `blitSprite(pipeline, TextureAtlasSprite, ...)` with a block atlas
    /// sprite (the nether portal overlay).
    AtlasSprite {
        sprite: Cow<'static, str>,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        color: u32,
    },
    /// `blit(pipeline, texture, x, y, u, v, w, h, srcW, srcH, texW, texH,
    /// color)`; `texture` is the path under `textures/` without `.png`.
    Blit {
        texture: Cow<'static, str>,
        blend: Blend,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        uv: [f32; 2],
        src: [i32; 2],
        tex: [i32; 2],
        color: u32,
    },
    /// `fill(x0, y0, x1, y1, color)` with the corners sorted.
    Fill {
        x0: i32,
        y0: i32,
        x1: i32,
        y1: i32,
        color: u32,
    },
    /// `fillGradient(x0, y0, x1, y1, top, bottom)`: ARGB `top` along the `y0` edge
    /// blending to `bottom` along the `y1` edge, in the current pose and clip.
    /// Zero area or inversion on exactly one axis draws nothing. With both axes inverted,
    /// bounds are sorted and colours flipped: `top` stays on `y0`, now the lower edge.
    FillGradient {
        x0: i32,
        y0: i32,
        x1: i32,
        y1: i32,
        top: u32,
        bottom: u32,
    },
    /// `text(font, text, x, y, argb, dropShadow)`.
    Text {
        text: Text,
        x: i32,
        y: i32,
        color: u32,
        shadow: bool,
    },
    /// `item(owner, stack, x, y, seed)` for a hotbar slot (0-8, 40 = off hand)
    /// under the pose's x/y scale about (cx, cy) (the pop squeeze).
    Item {
        slot: u32,
        x: i32,
        y: i32,
        seed: i32,
        scale: [f32; 2],
        pivot: [f32; 2],
    },
    /// `pose().pushMatrix()` followed by `ops`.
    Push(Vec<PoseOp>),
    /// `pose().popMatrix()`.
    Pop,
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
// exhaustive: a closed set the game matches without a wildcard
#[derive(horizon_mod_api_macros::Wire)]
pub enum HeartKind {
    Normal,
    Poisoned,
    Withered,
    Frozen,
}
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct HealthInputs {
    /// `Hud.tickCount`.
    pub tick: i32,
    /// `Util.getMillis()` now.
    pub millis: i64,
    pub health: f32,
    pub max_health: f32,
    pub absorption: f32,
    pub armor: i32,
    pub food: i32,
    pub saturation: f32,
    pub kind: HeartKind,
    pub regeneration: bool,
    pub hunger: bool,
    pub hardcore: bool,
    /// `damageCooldownTime > 0`.
    pub damage_cooldown: bool,
    /// `getAirSupply`, `getMaxAirSupply`, `isEyeInFluid(WATER)`.
    pub air: i32,
    pub max_air: i32,
    pub under_water: bool,
    /// `getVehicleMaxHearts` of the ridden living vehicle (0 = none).
    pub vehicle_hearts: i32,
}
