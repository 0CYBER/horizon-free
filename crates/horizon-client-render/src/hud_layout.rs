//! Layout of vanilla's `Hud` as draw calls (`GuiGraphicsExtractor` blits,
//! fills and text): the hotbar with its selection, off-hand slot, items and
//! attack indicator, the crosshair, armour, hearts, food, air bubbles, mount
//! hearts and the experience bar, plus the draw-call types and colour
//! helpers the other layouts share (`hud_overlays`, `chat`). Computed without
//! drawing so the parity fixture can compare them with the vanilla client.
use crate::{font::Font, text::Text};
use horizon_core::random::{LegacyRandom, RandomSource};
pub use mod_api::client::hud_layout::{Blend, HealthInputs, HeartKind, PoseOp};
use smallvec::{SmallVec, smallvec};
use std::borrow::Cow;

/// One `GuiGraphicsExtractor` call.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: a closed set the game matches without a wildcard
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

/// A JOML `Matrix3x2f` as `[m00, m01, m10, m11, m20, m21]`.
pub type Pose = [f32; 6];
pub const IDENTITY: Pose = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// Applies `op` like JOML's `translate` / `scale` (same float order).
pub fn pose_apply(m: &mut Pose, op: PoseOp) {
    match op {
        PoseOp::Translate(x, y) => {
            m[4] += m[0] * x + m[2] * y;
            m[5] += m[1] * x + m[3] * y;
        }
        PoseOp::Scale(x, y) => {
            m[0] *= x;
            m[1] *= x;
            m[2] *= y;
            m[3] *= y;
        }
    }
}

pub const WHITE: u32 = 0xFFFF_FFFF;

/// The highest top, in GUI px, of the draws that sit on the view (`None` for none): where a row
/// of the HUD begins.
pub fn draws_top(draws: &[GuiDraw]) -> Option<i32> {
    draws
        .iter()
        .filter_map(|d| match *d {
            GuiDraw::Sprite { y, .. }
            | GuiDraw::AtlasSprite { y, .. }
            | GuiDraw::Blit { y, .. }
            | GuiDraw::Text { y, .. }
            | GuiDraw::Item { y, .. } => Some(y),
            GuiDraw::Fill { y0, .. } | GuiDraw::FillGradient { y0, .. } => Some(y0),
            GuiDraw::Push(_) | GuiDraw::Pop => None,
        })
        .min()
}

fn sprite(sprite: &'static str, x: i32, y: i32, w: i32, h: i32) -> GuiDraw {
    GuiDraw::Sprite {
        sprite: Cow::Borrowed(sprite),
        x,
        y,
        w,
        h,
        region: None,
        color: WHITE,
        blend: Blend::Textured,
    }
}
fn region(
    sprite: &'static str,
    size: [i32; 2],
    uv: [i32; 2],
    at: [i32; 2],
    w: i32,
    h: i32,
) -> GuiDraw {
    GuiDraw::Sprite {
        sprite: Cow::Borrowed(sprite),
        x: at[0],
        y: at[1],
        w,
        h,
        region: Some([size[0], size[1], uv[0], uv[1]]),
        color: WHITE,
        blend: Blend::Textured,
    }
}
/// A draw through `RenderPipelines.CROSSHAIR`.
fn crosshair_blend(mut d: GuiDraw) -> GuiDraw {
    if let GuiDraw::Sprite { blend, .. } = &mut d {
        *blend = Blend::Crosshair;
    }
    d
}
/// `graphics.fill`: the corners are sorted.
pub fn fill(x0: i32, y0: i32, x1: i32, y1: i32, color: u32) -> GuiDraw {
    GuiDraw::Fill {
        x0: x0.min(x1),
        y0: y0.min(y1),
        x1: x0.max(x1),
        y1: y0.max(y1),
        color,
    }
}
/// `graphics.text`: dropped when fully transparent.
fn text(out: &mut Vec<GuiDraw>, text: Text, x: i32, y: i32, color: u32, shadow: bool) {
    if color >> 24 != 0 {
        out.push(GuiDraw::Text {
            text,
            x,
            y,
            color,
            shadow,
        });
    }
}
/// `Options.getBackgroundColor(float)`: `chat_only` = `backgroundForChatOnly`.
pub fn background_color(chat_only: bool, opacity: f64, default: f32) -> u32 {
    let a = if chat_only { default } else { opacity as f32 };
    color_from_float(a, 0.0, 0.0, 0.0)
}

/// `Mth.floor`.
pub fn floor(x: f32) -> i32 {
    let i = x as i32;
    if x < i as f32 { i - 1 } else { i }
}
/// `Mth.ceil`.
pub fn ceil(x: f32) -> i32 {
    let i = x as i32;
    if x > i as f32 { i + 1 } else { i }
}
/// `ARGB.as8BitChannel`.
pub fn as8(v: f32) -> u32 {
    floor(v * 255.0) as u32
}
/// `ARGB.white(float)`.
pub fn white(alpha: f32) -> u32 {
    (as8(alpha) << 24) | 0xFF_FFFF
}
/// `ARGB.colorFromFloat`.
pub fn color_from_float(a: f32, r: f32, g: f32, b: f32) -> u32 {
    argb(as8(a), as8(r), as8(g), as8(b))
}
/// `ARGB.color(a, r, g, b)`.
pub fn argb(a: u32, r: u32, g: u32, b: u32) -> u32 {
    (a & 255) << 24 | (r & 255) << 16 | (g & 255) << 8 | (b & 255)
}
/// `ARGB.multiply`.
pub fn multiply(l: u32, r: u32) -> u32 {
    if l == WHITE {
        return r;
    }
    if r == WHITE {
        return l;
    }
    let c = |s: u32| ((l >> s) & 255) * ((r >> s) & 255) / 255;
    argb(c(24), c(16), c(8), c(0))
}
/// `Mth.lerpDiscrete`.
pub fn lerp_discrete(alpha: f32, p0: i32, p1: i32) -> i32 {
    p0 + floor(alpha * (p1 - p0 - 1) as f32) + i32::from(alpha > 0.0)
}
/// `Font.width` of styled text.
pub fn width(font: &Font, t: &Text) -> i32 {
    font.width_text(t) as i32
}

/// `Hud.extractItemHotbar` inputs.
pub struct HotbarInputs {
    pub selected: i32,
    /// The main arm is the right one (the off hand sits left of the hotbar).
    pub right_handed: bool,
    pub offhand: bool,
    /// Pop time minus partial tick (`ItemStack.getPopTime`) per hotbar slot
    /// that holds an item, then the off hand.
    pub slots: [Option<f32>; 9],
    pub offhand_pop: f32,
    /// Attack indicator progress in HOTBAR mode, usually `getAttackStrengthScale(0)`.
    pub attack: Option<f32>,
}

/// `Hud.extractItemHotbar`.
pub fn hotbar(w: i32, h: i32, i: &HotbarInputs) -> Vec<GuiDraw> {
    let mut out = Vec::new();
    hotbar_into(w, h, i, &mut out);
    out
}

/// Appends [`hotbar`] draws to `out` without allocating a new buffer.
pub fn hotbar_into(w: i32, h: i32, i: &HotbarInputs, out: &mut Vec<GuiDraw>) {
    let center = w / 2;
    out.push(sprite("hud/hotbar", center - 91, h - 22, 182, 22));
    out.push(sprite(
        "hud/hotbar_selection",
        center - 91 - 1 + i.selected * 20,
        h - 22 - 1,
        24,
        23,
    ));
    // The off hand is opposite the main arm.
    let offhand_left = i.right_handed;
    if i.offhand {
        if offhand_left {
            out.push(sprite(
                "hud/hotbar_offhand_left",
                center - 91 - 29,
                h - 23,
                29,
                24,
            ));
        } else {
            out.push(sprite(
                "hud/hotbar_offhand_right",
                center + 91,
                h - 23,
                29,
                24,
            ));
        }
    }
    let slot = |slot: u32, x: i32, y: i32, seed: i32, pop: f32, out: &mut Vec<GuiDraw>| {
        let (scale, pivot) = if pop > 0.0 {
            let squeeze = 1.0 + pop / 5.0;
            (
                [1.0 / squeeze, (squeeze + 1.0) / 2.0],
                [(x + 8) as f32, (y + 12) as f32],
            )
        } else {
            ([1.0, 1.0], [0.0, 0.0])
        };
        out.push(GuiDraw::Item {
            slot,
            x,
            y,
            seed,
            scale,
            pivot,
        });
    };
    let mut seed = 1;
    for (n, pop) in i.slots.iter().enumerate() {
        if let Some(pop) = pop {
            slot(
                n as u32,
                center - 90 + n as i32 * 20 + 2,
                h - 16 - 3,
                seed,
                *pop,
                out,
            );
        }
        seed += 1;
    }
    if i.offhand {
        let x = if offhand_left {
            center - 91 - 26
        } else {
            center + 91 + 10
        };
        slot(40, x, h - 16 - 3, seed, i.offhand_pop, out);
    }
    if let Some(scale) = i.attack
        && scale < 1.0
    {
        let y = h - 20;
        let x = if offhand_left {
            center + 91 + 6
        } else {
            center - 91 - 22
        };
        let progress = (scale * 19.0) as i32;
        out.push(sprite(
            "hud/hotbar_attack_indicator_background",
            x,
            y,
            18,
            18,
        ));
        out.push(region(
            "hud/hotbar_attack_indicator_progress",
            [18, 18],
            [0, 18 - progress],
            [x, y + 18 - progress],
            18,
            progress,
        ));
    }
}

/// `Hud.extractCrosshair`'s full attack indicator: at full strength on a
/// living, alive `crosshairPickEntity` (`target`) with a weapon slower than
/// `MIN_CROSSHAIR_ATTACK_SPEED` (`delay` is
/// `getCurrentItemAttackStrengthDelay`).
pub fn full_attack_indicator(scale: f32, delay: f32, target: Option<bool>) -> bool {
    scale >= 1.0 && delay > 5.0 && target == Some(true)
}

/// `Hud.extractCrosshair` (first person, not hidden for spectators), every
/// sprite through the inverting `CROSSHAIR` pipeline: `attack` is
/// `getAttackStrengthScale(0)` in CROSSHAIR indicator mode and `full`
/// whether the full indicator shows ([`full_attack_indicator`]).
pub fn crosshair(w: i32, h: i32, attack: Option<f32>, full: bool) -> Vec<GuiDraw> {
    let mut out = Vec::new();
    crosshair_into(w, h, attack, full, &mut out);
    out
}

/// Appends [`crosshair`] draws to `out` without allocating a new buffer.
pub fn crosshair_into(w: i32, h: i32, attack: Option<f32>, full: bool, out: &mut Vec<GuiDraw>) {
    out.push(crosshair_blend(sprite(
        "hud/crosshair",
        (w - 15) / 2,
        (h - 15) / 2,
        15,
        15,
    )));
    if let Some(scale) = attack {
        let (x, y) = (w / 2 - 8, h / 2 - 7 + 16);
        if full {
            out.push(crosshair_blend(sprite(
                "hud/crosshair_attack_indicator_full",
                x,
                y,
                16,
                16,
            )));
        } else if scale < 1.0 {
            let progress = (scale * 17.0) as i32;
            out.push(crosshair_blend(sprite(
                "hud/crosshair_attack_indicator_background",
                x,
                y,
                16,
                4,
            )));
            out.push(crosshair_blend(region(
                "hud/crosshair_attack_indicator_progress",
                [16, 4],
                [0, 0],
                [x, y],
                progress,
                4,
            )));
        }
    }
}

/// `ExperienceBar.extractBackground` and `ContextualBar.extractExperienceLevel`;
/// `width` measures text with the font.
pub fn experience(
    w: i32,
    h: i32,
    progress: f32,
    level: i32,
    bar: bool,
    width: &dyn Fn(&str) -> i32,
) -> Vec<GuiDraw> {
    let mut out = Vec::new();
    experience_into(w, h, progress, level, bar, width, &mut out);
    out
}

/// Appends [`experience`] draws to `out` without allocating a new buffer.
pub fn experience_into(
    w: i32,
    h: i32,
    progress: f32,
    level: i32,
    bar: bool,
    width: &dyn Fn(&str) -> i32,
    out: &mut Vec<GuiDraw>,
) {
    experience_colored_into([None; 2], w, h, progress, level, bar, width, out);
}

/// [`experience`] with a resource pack's level number colours (Polytone `text.xpbar`: the
/// number's, then its outline's; `None` keeps vanilla's).
pub fn experience_colored(
    [number, outline]: [Option<u32>; 2],
    w: i32,
    h: i32,
    progress: f32,
    level: i32,
    bar: bool,
    width: &dyn Fn(&str) -> i32,
) -> Vec<GuiDraw> {
    let mut out = Vec::new();
    experience_colored_into(
        [number, outline],
        w,
        h,
        progress,
        level,
        bar,
        width,
        &mut out,
    );
    out
}

/// Appends [`experience_colored`] draws to `out` without allocating a new buffer.
pub fn experience_colored_into(
    [number, outline]: [Option<u32>; 2],
    w: i32,
    h: i32,
    progress: f32,
    level: i32,
    bar: bool,
    width: &dyn Fn(&str) -> i32,
    out: &mut Vec<GuiDraw>,
) {
    if bar {
        let (left, top) = (w / 2 - 91, h - 32 + 3);
        let filled = (progress * 183.0) as i32;
        out.push(sprite("hud/experience_bar_background", left, top, 182, 5));
        if filled > 0 {
            out.push(region(
                "hud/experience_bar_progress",
                [182, 5],
                [0, 0],
                [left, top],
                filled,
                5,
            ));
        }
    }
    if level > 0 {
        let text = level.to_string();
        let x = (w - width(&text)) / 2;
        let y = h - 24 - 9 - 2;
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            self::text(
                out,
                Text::literal(&text),
                x + dx,
                y + dy,
                outline.map_or(0xFF00_0000, opaque),
                false,
            );
        }
        let color = number.map_or(0xFF80_FF20, opaque);
        self::text(out, Text::literal(text), x, y, color, false);
    }
}

/// State kept between frames (`Hud.lastHealth`, `displayHealth`, ...).
#[derive(Clone, Debug, Default)]
pub struct HealthHud {
    pub last_health: i32,
    pub display_health: i32,
    /// `Util.getMillis()` of the last health change.
    pub last_health_time: i64,
    pub health_blink_time: i64,
    /// `Hud.lastBubblePopSoundPlayed`.
    pub last_bubble_pop_sound: i32,
    /// The `ui.hud.bubble_pop` volume and pitch `playAirBubblePoppedSound`
    /// asked for this frame; the caller plays it and clears it.
    pub bubble_pop_sound: Option<(f32, f32)>,
}

/// `Hud.getVehicleMaxHearts` for a vehicle that shows its health.
pub fn vehicle_max_hearts(max_health: f32) -> i32 {
    (((max_health + 0.5) as i32) / 2).min(30)
}

/// `Hud.extractVehicleHealth`.
pub fn vehicle_health(w: i32, h: i32, hearts: i32, health: f32) -> Vec<GuiDraw> {
    let mut out = Vec::new();
    vehicle_health_into(w, h, hearts, health, &mut out);
    out
}

/// Appends [`vehicle_health`] draws to `out` without allocating a new buffer.
pub fn vehicle_health_into(w: i32, h: i32, hearts: i32, health: f32, out: &mut Vec<GuiDraw>) {
    let current = f64::from(health).ceil() as i32;
    let x_right = w / 2 + 91;
    let mut yo = h - 39;
    let mut hearts = hearts;
    let mut base = 0;
    while hearts > 0 {
        let row = hearts.min(10);
        hearts -= row;
        for i in 0..row {
            let xo = x_right - i * 8 - 9;
            out.push(sprite("hud/heart/vehicle_container", xo, yo, 9, 9));
            if i * 2 + 1 + base < current {
                out.push(sprite("hud/heart/vehicle_full", xo, yo, 9, 9));
            }
            if i * 2 + 1 + base == current {
                out.push(sprite("hud/heart/vehicle_half", xo, yo, 9, 9));
            }
        }
        yo -= 10;
        base += 20;
    }
}

fn heart(
    kind: Option<HeartKind>,
    absorbing: bool,
    hardcore: bool,
    half: bool,
    blink: bool,
) -> &'static str {
    // Hud.HeartType sprites: [full, full_blinking, half, half_blinking] x [normal, hardcore].
    let table: [&'static str; 8] = match (kind, absorbing) {
        (None, _) => [
            "hud/heart/container",
            "hud/heart/container_blinking",
            "hud/heart/container",
            "hud/heart/container_blinking",
            "hud/heart/container_hardcore",
            "hud/heart/container_hardcore_blinking",
            "hud/heart/container_hardcore",
            "hud/heart/container_hardcore_blinking",
        ],
        (_, true) => [
            "hud/heart/absorbing_full",
            "hud/heart/absorbing_full_blinking",
            "hud/heart/absorbing_half",
            "hud/heart/absorbing_half_blinking",
            "hud/heart/absorbing_hardcore_full",
            "hud/heart/absorbing_hardcore_full_blinking",
            "hud/heart/absorbing_hardcore_half",
            "hud/heart/absorbing_hardcore_half_blinking",
        ],
        (Some(HeartKind::Normal), _) => [
            "hud/heart/full",
            "hud/heart/full_blinking",
            "hud/heart/half",
            "hud/heart/half_blinking",
            "hud/heart/hardcore_full",
            "hud/heart/hardcore_full_blinking",
            "hud/heart/hardcore_half",
            "hud/heart/hardcore_half_blinking",
        ],
        (Some(HeartKind::Poisoned), _) => [
            "hud/heart/poisoned_full",
            "hud/heart/poisoned_full_blinking",
            "hud/heart/poisoned_half",
            "hud/heart/poisoned_half_blinking",
            "hud/heart/poisoned_hardcore_full",
            "hud/heart/poisoned_hardcore_full_blinking",
            "hud/heart/poisoned_hardcore_half",
            "hud/heart/poisoned_hardcore_half_blinking",
        ],
        (Some(HeartKind::Withered), _) => [
            "hud/heart/withered_full",
            "hud/heart/withered_full_blinking",
            "hud/heart/withered_half",
            "hud/heart/withered_half_blinking",
            "hud/heart/withered_hardcore_full",
            "hud/heart/withered_hardcore_full_blinking",
            "hud/heart/withered_hardcore_half",
            "hud/heart/withered_hardcore_half_blinking",
        ],
        (Some(HeartKind::Frozen), _) => [
            "hud/heart/frozen_full",
            "hud/heart/frozen_full_blinking",
            "hud/heart/frozen_half",
            "hud/heart/frozen_half_blinking",
            "hud/heart/frozen_hardcore_full",
            "hud/heart/frozen_hardcore_full_blinking",
            "hud/heart/frozen_hardcore_half",
            "hud/heart/frozen_hardcore_half_blinking",
        ],
    };
    table[usize::from(hardcore) * 4 + usize::from(half) * 2 + usize::from(blink)]
}

impl HealthHud {
    /// `Hud.extractPlayerHealth` for a screen of `w` x `h` GUI pixels.
    pub fn layout(&mut self, i: &HealthInputs, w: i32, h: i32) -> Vec<GuiDraw> {
        let mut out = Vec::new();
        self.layout_into(i, w, h, &mut out);
        out
    }

    /// Appends [`HealthHud::layout`] draws to `out` without allocating a new buffer.
    pub fn layout_into(&mut self, i: &HealthInputs, w: i32, h: i32, out: &mut Vec<GuiDraw>) {
        let tick = i64::from(i.tick);
        let current = ceil(i.health);
        let blink = self.health_blink_time > tick && (self.health_blink_time - tick) / 3 % 2 == 1;
        if current < self.last_health && i.damage_cooldown {
            self.last_health_time = i.millis;
            self.health_blink_time = tick + 20;
        } else if current > self.last_health && i.damage_cooldown {
            self.last_health_time = i.millis;
            self.health_blink_time = tick + 10;
        }
        if i.millis - self.last_health_time > 1000 {
            self.display_health = current;
            self.last_health_time = i.millis;
        }
        self.last_health = current;
        let old = self.display_health;
        let mut random = LegacyRandom::new(i64::from(i.tick.wrapping_mul(312_871)));
        let mut next = |bound: i32| random.next_int_bound(bound).unwrap_or(0);
        let (x_left, x_right, y_base) = (w / 2 - 91, w / 2 + 91, h - 39);
        let max_health = i.max_health.max(old.max(current) as f32);
        let absorption = ceil(i.absorption);
        let rows = ceil((max_health + absorption as f32) / 2.0 / 10.0);
        let row_height = (10 - (rows - 2)).max(3);
        let mut y_air = y_base - 10;
        let offset_index = if i.regeneration {
            i.tick % ceil(max_health + 5.0)
        } else {
            -1
        };
        if i.armor > 0 {
            let y = y_base - (rows - 1) * row_height - 10;
            for n in 0..10 {
                let x = x_left + n * 8;
                if n * 2 + 1 < i.armor {
                    out.push(sprite("hud/armor_full", x, y, 9, 9));
                }
                if n * 2 + 1 == i.armor {
                    out.push(sprite("hud/armor_half", x, y, 9, 9));
                }
                if n * 2 + 1 > i.armor {
                    out.push(sprite("hud/armor_empty", x, y, 9, 9));
                }
            }
        }
        let containers = f64::from(max_health / 2.0).ceil() as i32;
        let absorb_containers = (f64::from(absorption) / 2.0).ceil() as i32;
        for c in (0..containers + absorb_containers).rev() {
            let (row, column) = (c / 10, c % 10);
            let x = x_left + column * 8;
            let mut y = y_base - row * row_height;
            if current + absorption <= 4 {
                y += next(2);
            }
            if c < containers && c == offset_index {
                y -= 2;
            }
            out.push(sprite(
                heart(None, false, i.hardcore, false, blink),
                x,
                y,
                9,
                9,
            ));
            let halves = c * 2;
            if c >= containers {
                let ah = halves - containers * 2;
                if ah < absorption {
                    let withered = i.kind == HeartKind::Withered;
                    let name = heart(
                        Some(i.kind),
                        !withered,
                        i.hardcore,
                        ah + 1 == absorption,
                        false,
                    );
                    out.push(sprite(name, x, y, 9, 9));
                }
            }
            if blink && halves < old {
                let name = heart(Some(i.kind), false, i.hardcore, halves + 1 == old, true);
                out.push(sprite(name, x, y, 9, 9));
            }
            if halves < current {
                let name = heart(
                    Some(i.kind),
                    false,
                    i.hardcore,
                    halves + 1 == current,
                    false,
                );
                out.push(sprite(name, x, y, 9, 9));
            }
        }
        if i.vehicle_hearts == 0 {
            let (empty, half, full) = if i.hunger {
                (
                    "hud/food_empty_hunger",
                    "hud/food_half_hunger",
                    "hud/food_full_hunger",
                )
            } else {
                ("hud/food_empty", "hud/food_half", "hud/food_full")
            };
            for n in 0..10 {
                let mut y = y_base;
                if i.saturation <= 0.0 && i.tick % (i.food * 3 + 1) == 0 {
                    y += next(3) - 1;
                }
                let x = x_right - n * 8 - 9;
                out.push(sprite(empty, x, y, 9, 9));
                if n * 2 + 1 < i.food {
                    out.push(sprite(full, x, y, 9, 9));
                }
                if n * 2 + 1 == i.food {
                    out.push(sprite(half, x, y, 9, 9));
                }
            }
            y_air -= 10;
        }
        // Hud.extractAirBubbles.
        let max_air = i.max_air.max(1);
        let air = i.air.clamp(0, max_air);
        if i.under_water || air < max_air {
            let rows = (f64::from(i.vehicle_hearts) / 10.0).ceil() as i32;
            let y = y_air - (rows - 1) * 10;
            let bubble = |offset: i32| ceil(((air + offset) * 10) as f32 / max_air as f32);
            let full = bubble(-2);
            let popping = bubble(0);
            let delay = i32::from(air != 0 && i.under_water);
            let empty = 10 - bubble(delay);
            if !i.under_water {
                self.last_bubble_pop_sound = 0;
            }
            for b in 1..=10 {
                let x = x_right - (b - 1) * 8 - 9;
                if b <= full {
                    out.push(sprite("hud/air", x, y, 9, 9));
                } else if full != popping && b == popping && i.under_water {
                    out.push(sprite("hud/air_bursting", x, y, 9, 9));
                    // Hud.playAirBubblePoppedSound.
                    if self.last_bubble_pop_sound != b {
                        let volume = 0.5 + 0.1 * (empty - 3 + 1).max(0) as f32;
                        let pitch = 1.0 + 0.1 * (empty - 5 + 1).max(0) as f32;
                        self.bubble_pop_sound = Some((volume, pitch));
                        self.last_bubble_pop_sound = b;
                    }
                } else if b > 10 - empty {
                    let wobble = if empty == 10 && i.tick % 2 == 0 {
                        next(2)
                    } else {
                        0
                    };
                    out.push(sprite("hud/air_empty", x, y + wobble, 9, 9));
                }
            }
        }
    }
}

/// Draws the calls with the HUD batch under their poses; items are left to
/// the caller (they need the item renderer) and so are the vignette and
/// nausea blits (their blending needs the screen-effect pipeline).
pub fn paint(batch: &mut crate::hud::HudBatch, atlas: &crate::hud::HudAtlas, draws: &[GuiDraw]) {
    if draws.is_empty() {
        return;
    }
    let mut scratch = crate::hud::HudGlyphScratch::new();
    paint_with_scratch(batch, atlas, draws, &mut scratch);
}

/// [`paint`], reusing `scratch` for staged glyphs across any [`GuiDraw::Text`] calls.
pub fn paint_with_scratch(
    batch: &mut crate::hud::HudBatch,
    atlas: &crate::hud::HudAtlas,
    draws: &[GuiDraw],
    scratch: &mut crate::hud::HudGlyphScratch,
) {
    if draws.is_empty() {
        return;
    }
    const GUI_SPRITE_PREFIX: &str = "gui/sprites/";
    let mut stack: SmallVec<[Pose; 4]> = smallvec![IDENTITY];
    // Stack buffer for typical `gui/sprites/<name>` keys; spills only on >116-byte sprite names.
    let mut name_buf = [0u8; 128];
    name_buf[..GUI_SPRITE_PREFIX.len()].copy_from_slice(GUI_SPRITE_PREFIX.as_bytes());
    let mut name_overflow = String::new();
    for d in draws {
        let start = batch.vertices.len();
        let pose = *stack.last().unwrap_or(&IDENTITY);
        match d {
            GuiDraw::Sprite {
                sprite,
                x,
                y,
                w,
                h,
                region,
                color,
                blend,
            } => {
                // Effect sprites belong to the mob_effect atlas directory (a
                // content mod's under its namespace); the other blitSprite
                // ids refer to GUI sprites.
                let sprite: &str = sprite;
                let effect = sprite.starts_with("mob_effect/")
                    || sprite
                        .split_once(':')
                        .is_some_and(|(_, path)| path.starts_with("mob_effect/"));
                let key: &str = if effect {
                    sprite
                } else if GUI_SPRITE_PREFIX.len() + sprite.len() <= name_buf.len() {
                    let end = GUI_SPRITE_PREFIX.len() + sprite.len();
                    name_buf[GUI_SPRITE_PREFIX.len()..end].copy_from_slice(sprite.as_bytes());
                    std::str::from_utf8(&name_buf[..end]).expect("valid utf-8 sprite key")
                } else {
                    name_overflow.clear();
                    name_overflow.push_str(GUI_SPRITE_PREFIX);
                    name_overflow.push_str(sprite);
                    name_overflow.as_str()
                };
                let found = match effect {
                    true => atlas.sprite_or_missing(key),
                    false => atlas.sprites.get(key).copied(),
                };
                let Some(mut r) = found else {
                    continue;
                };
                if let Some([sw, sh, u, v]) = *region {
                    let (du, dv) = (r.u1 - r.u0, r.v1 - r.v0);
                    let u0 = r.u0 + du * u as f32 / sw as f32;
                    let v0 = r.v0 + dv * v as f32 / sh as f32;
                    r.u1 = u0 + du * *w as f32 / sw as f32;
                    r.v1 = v0 + dv * *h as f32 / sh as f32;
                    r.u0 = u0;
                    r.v0 = v0;
                }
                let at = [*x as f32, *y as f32, *w as f32, *h as f32];
                match crate::hud::sprite_modifiers::draws(atlas, key, r, at, abgr(*color)) {
                    // A Polytone screen sprite modifier: the sprite changed, then its overlays.
                    Some(draws) => {
                        for d in draws {
                            batch.rect_uv(d.at[0], d.at[1], d.at[2], d.at[3], d.rect, d.color);
                        }
                    }
                    None => batch.rect_uv(at[0], at[1], at[2], at[3], r, abgr(*color)),
                }
                if *blend == Blend::Crosshair {
                    batch.invert_since(start);
                }
            }
            GuiDraw::AtlasSprite {
                sprite,
                x,
                y,
                w,
                h,
                color,
            } => {
                batch.sprite_scaled(
                    atlas,
                    sprite,
                    *x as f32,
                    *y as f32,
                    *w as f32,
                    *h as f32,
                    abgr(*color),
                );
            }
            GuiDraw::Blit {
                texture,
                blend: Blend::Textured,
                x,
                y,
                w,
                h,
                uv,
                src,
                tex,
                color,
            } => {
                let Some(mut r) = atlas.sprites.get(texture.as_ref()).copied() else {
                    continue;
                };
                let (du, dv) = (r.u1 - r.u0, r.v1 - r.v0);
                let u0 = r.u0 + du * uv[0] / tex[0] as f32;
                let v0 = r.v0 + dv * uv[1] / tex[1] as f32;
                r.u1 = r.u0 + du * (uv[0] + src[0] as f32) / tex[0] as f32;
                r.v1 = r.v0 + dv * (uv[1] + src[1] as f32) / tex[1] as f32;
                r.u0 = u0;
                r.v0 = v0;
                batch.rect_uv(*x as f32, *y as f32, *w as f32, *h as f32, r, abgr(*color));
            }
            GuiDraw::Blit { .. } | GuiDraw::Item { .. } => {}
            GuiDraw::Fill {
                x0,
                y0,
                x1,
                y1,
                color,
            } => {
                batch.fill(
                    atlas,
                    *x0 as f32,
                    *y0 as f32,
                    (x1 - x0) as f32,
                    (y1 - y0) as f32,
                    abgr(*color),
                );
            }
            GuiDraw::FillGradient {
                x0,
                y0,
                x1,
                y1,
                top,
                bottom,
            } => {
                if x1 != x0 && y1 != y0 && (x1 > x0) == (y1 > y0) {
                    let (x, y) = ((*x0).min(*x1) as f32, (*y0).min(*y1) as f32);
                    let rect = (x, y, (*x0).max(*x1) as f32 - x, (*y0).max(*y1) as f32 - y);
                    let (top, bottom) = if y1 > y0 {
                        (*top, *bottom)
                    } else {
                        (*bottom, *top)
                    };
                    batch.fill_gradient(atlas, rect, abgr(top), abgr(bottom));
                }
            }
            GuiDraw::Text {
                text,
                x,
                y,
                color,
                shadow,
            } => {
                batch.text_styled_with_scratch(
                    atlas,
                    text,
                    *x as f32,
                    *y as f32,
                    abgr(*color),
                    *shadow,
                    scratch,
                );
            }
            GuiDraw::Push(ops) => {
                let mut m = pose;
                for op in ops {
                    pose_apply(&mut m, *op);
                }
                stack.push(m);
            }
            GuiDraw::Pop => {
                if stack.len() > 1 {
                    stack.pop();
                }
            }
        }
        if pose != IDENTITY {
            for v in &mut batch.vertices[start..] {
                let [x, y] = v.pos;
                v.pos = [
                    pose[0] * x + pose[2] * y + pose[4],
                    pose[1] * x + pose[3] * y + pose[5],
                ];
            }
        }
    }
}

/// An ARGB colour as the HUD batch's packed RGBA.
pub fn abgr(color: u32) -> u32 {
    let [a, r, g, b] = color.to_be_bytes();
    crate::hud::rgba(r, g, b, a)
}

/// A pack colour without an alpha drawn opaque, as `ARGB.opaque` makes it (one with alpha keeps it).
fn opaque(color: u32) -> u32 {
    if color >> 24 == 0 {
        color | 0xFF00_0000
    } else {
        color
    }
}

#[cfg(test)]
#[path = "hud_layout_gradient_tests.rs"]
mod gradient_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hud::{HudAtlas, HudBatch, HudVertex};

    fn test_atlas() -> HudAtlas {
        HudAtlas::build(&crate::resource_source::ResourceSource::directory(
            "/nonexistent/horizon-test-assets".into(),
        ))
        .unwrap()
    }

    #[test]
    fn into_helpers_match_vec_constructors_and_reuse_capacity() {
        let sentinel = GuiDraw::Fill {
            x0: 1,
            y0: 2,
            x1: 3,
            y1: 4,
            color: 0xDEAD_BEEF,
        };
        let expect_sprite =
            |name: &'static str, x: i32, y: i32, w: i32, h: i32, blend: Blend| GuiDraw::Sprite {
                sprite: Cow::Borrowed(name),
                x,
                y,
                w,
                h,
                region: None,
                color: 0xFFFF_FFFF,
                blend,
            };
        let expect_region =
            |name: &'static str, region: [i32; 4], x: i32, y: i32, w: i32, h: i32, blend: Blend| {
                GuiDraw::Sprite {
                    sprite: Cow::Borrowed(name),
                    x,
                    y,
                    w,
                    h,
                    region: Some(region),
                    color: 0xFFFF_FFFF,
                    blend,
                }
            };
        let expect_text = |value: &str, x: i32, y: i32, color: u32| GuiDraw::Text {
            text: Text::literal(value),
            x,
            y,
            color,
            shadow: false,
        };

        let mut buf = Vec::new();

        // 1. hotbar_into (w = 427, h = 240 -> center = 213)
        let hotbar_in = HotbarInputs {
            selected: 3,
            right_handed: true,
            offhand: true,
            slots: [
                Some(0.0),
                Some(5.0),
                None,
                Some(15.0),
                None,
                None,
                None,
                None,
                Some(0.0),
            ],
            offhand_pop: 5.0,
            attack: Some(0.5),
        };
        let expected_hotbar = [
            expect_sprite("hud/hotbar", 122, 218, 182, 22, Blend::Textured),
            expect_sprite("hud/hotbar_selection", 181, 217, 24, 23, Blend::Textured),
            expect_sprite("hud/hotbar_offhand_left", 93, 217, 29, 24, Blend::Textured),
            GuiDraw::Item {
                slot: 0,
                x: 125,
                y: 221,
                seed: 1,
                scale: [1.0, 1.0],
                pivot: [0.0, 0.0],
            },
            GuiDraw::Item {
                slot: 1,
                x: 145,
                y: 221,
                seed: 2,
                scale: [0.5, 1.5],
                pivot: [153.0, 233.0],
            },
            GuiDraw::Item {
                slot: 3,
                x: 185,
                y: 221,
                seed: 4,
                scale: [0.25, 2.5],
                pivot: [193.0, 233.0],
            },
            GuiDraw::Item {
                slot: 8,
                x: 285,
                y: 221,
                seed: 9,
                scale: [1.0, 1.0],
                pivot: [0.0, 0.0],
            },
            GuiDraw::Item {
                slot: 40,
                x: 96,
                y: 221,
                seed: 10,
                scale: [0.5, 1.5],
                pivot: [104.0, 233.0],
            },
            expect_sprite(
                "hud/hotbar_attack_indicator_background",
                310,
                220,
                18,
                18,
                Blend::Textured,
            ),
            expect_region(
                "hud/hotbar_attack_indicator_progress",
                [18, 18, 0, 9],
                310,
                229,
                18,
                9,
                Blend::Textured,
            ),
        ];
        buf.push(sentinel.clone());
        hotbar_into(427, 240, &hotbar_in, &mut buf);
        assert_eq!(buf[0], sentinel);
        assert_eq!(&buf[1..], &expected_hotbar[..]);
        assert_eq!(
            hotbar(427, 240, &hotbar_in).as_slice(),
            &expected_hotbar[..]
        );
        let cap_after_hotbar = buf.capacity();
        assert!(cap_after_hotbar >= buf.len());

        // 2. crosshair_into (every sprite must use Blend::Crosshair)
        let crosshair_plain = [expect_sprite(
            "hud/crosshair",
            206,
            112,
            15,
            15,
            Blend::Crosshair,
        )];
        let crosshair_charging = [
            expect_sprite("hud/crosshair", 206, 112, 15, 15, Blend::Crosshair),
            expect_sprite(
                "hud/crosshair_attack_indicator_background",
                205,
                129,
                16,
                4,
                Blend::Crosshair,
            ),
            expect_region(
                "hud/crosshair_attack_indicator_progress",
                [16, 4, 0, 0],
                205,
                129,
                6,
                4,
                Blend::Crosshair,
            ),
        ];
        let crosshair_full = [
            expect_sprite("hud/crosshair", 206, 112, 15, 15, Blend::Crosshair),
            expect_sprite(
                "hud/crosshair_attack_indicator_full",
                205,
                129,
                16,
                16,
                Blend::Crosshair,
            ),
        ];
        for (attack, full, expected) in [
            (None, false, crosshair_plain.as_slice()),
            (Some(0.4), false, crosshair_charging.as_slice()),
            (Some(1.0), true, crosshair_full.as_slice()),
        ] {
            buf.clear();
            buf.push(sentinel.clone());
            crosshair_into(427, 240, attack, full, &mut buf);
            assert_eq!(buf[0], sentinel);
            assert_eq!(&buf[1..], expected);
            assert_eq!(crosshair(427, 240, attack, full).as_slice(), expected);
            assert!(buf.capacity() >= cap_after_hotbar);
        }

        // 3. experience_into (w = 427, h = 240 -> left = 122, top = 211; text "17" at x = 207, y = 205)
        let width = |s: &str| (s.len() as i32) * 6;
        let expected_exp = [
            expect_sprite(
                "hud/experience_bar_background",
                122,
                211,
                182,
                5,
                Blend::Textured,
            ),
            expect_region(
                "hud/experience_bar_progress",
                [182, 5, 0, 0],
                122,
                211,
                118,
                5,
                Blend::Textured,
            ),
            expect_text("17", 208, 205, 0xFF00_0000),
            expect_text("17", 206, 205, 0xFF00_0000),
            expect_text("17", 207, 206, 0xFF00_0000),
            expect_text("17", 207, 204, 0xFF00_0000),
            expect_text("17", 207, 205, 0xFF80_FF20),
        ];
        buf.clear();
        buf.push(sentinel.clone());
        experience_into(427, 240, 0.65, 17, true, &width, &mut buf);
        assert_eq!(buf[0], sentinel);
        assert_eq!(&buf[1..], &expected_exp[..]);
        assert_eq!(
            experience(427, 240, 0.65, 17, true, &width).as_slice(),
            &expected_exp[..]
        );
        assert!(buf.capacity() >= cap_after_hotbar);

        // 4. experience_colored_into (Polytone opaque level & outline colours)
        let expected_exp_colored = [
            expect_sprite(
                "hud/experience_bar_background",
                122,
                211,
                182,
                5,
                Blend::Textured,
            ),
            expect_region(
                "hud/experience_bar_progress",
                [182, 5, 0, 0],
                122,
                211,
                45,
                5,
                Blend::Textured,
            ),
            expect_text("42", 208, 205, 0xFF44_5566),
            expect_text("42", 206, 205, 0xFF44_5566),
            expect_text("42", 207, 206, 0xFF44_5566),
            expect_text("42", 207, 204, 0xFF44_5566),
            expect_text("42", 207, 205, 0xFF11_2233),
        ];
        buf.clear();
        buf.push(sentinel.clone());
        experience_colored_into(
            [Some(0x112233), Some(0x445566)],
            427,
            240,
            0.25,
            42,
            true,
            &width,
            &mut buf,
        );
        assert_eq!(buf[0], sentinel);
        assert_eq!(&buf[1..], &expected_exp_colored[..]);
        assert_eq!(
            experience_colored(
                [Some(0x112233), Some(0x445566)],
                427,
                240,
                0.25,
                42,
                true,
                &width,
            )
            .as_slice(),
            &expected_exp_colored[..]
        );
        assert!(buf.capacity() >= cap_after_hotbar);

        // 5. vehicle_health_into (15 hearts across 2 rows, 23 HP = 11 full + 1 half + 3 empty)
        let mut expected_vehicle = Vec::new();
        for i in 0..10 {
            let xo = 295 - i * 8;
            expected_vehicle.push(expect_sprite(
                "hud/heart/vehicle_container",
                xo,
                201,
                9,
                9,
                Blend::Textured,
            ));
            expected_vehicle.push(expect_sprite(
                "hud/heart/vehicle_full",
                xo,
                201,
                9,
                9,
                Blend::Textured,
            ));
        }
        for (i, heart_fill) in [
            (0, Some("hud/heart/vehicle_full")),
            (1, Some("hud/heart/vehicle_half")),
            (2, None),
            (3, None),
            (4, None),
        ] {
            let xo = 295 - i * 8;
            expected_vehicle.push(expect_sprite(
                "hud/heart/vehicle_container",
                xo,
                191,
                9,
                9,
                Blend::Textured,
            ));
            if let Some(sprite_name) = heart_fill {
                expected_vehicle.push(expect_sprite(sprite_name, xo, 191, 9, 9, Blend::Textured));
            }
        }
        buf.clear();
        buf.push(sentinel.clone());
        vehicle_health_into(427, 240, 15, 23.0, &mut buf);
        assert_eq!(buf[0], sentinel);
        assert_eq!(&buf[1..], expected_vehicle.as_slice());
        assert_eq!(vehicle_health(427, 240, 15, 23.0), expected_vehicle);
        let cap_after_vehicle = buf.capacity();
        assert!(cap_after_vehicle >= cap_after_hotbar);

        // 6. HealthHud::layout_into (armor, hearts with regen bounce + absorption, food, air)
        let health_in = HealthInputs {
            tick: 28,
            millis: 12_345,
            health: 15.0,
            max_health: 20.0,
            absorption: 3.0,
            armor: 13,
            food: 15,
            saturation: 0.0,
            kind: HeartKind::Normal,
            regeneration: true,
            hunger: false,
            hardcore: false,
            damage_cooldown: true,
            air: 181,
            max_air: 300,
            under_water: true,
            vehicle_hearts: 0,
        };
        let mut expected_health = Vec::new();
        // Armor: y = 201 - (2 - 1) * 10 - 10 = 181; 13 armor = 6 full, 1 half, 3 empty.
        for n in 0..10 {
            let name = match n {
                0..=5 => "hud/armor_full",
                6 => "hud/armor_half",
                _ => "hud/armor_empty",
            };
            expected_health.push(expect_sprite(name, 122 + n * 8, 181, 9, 9, Blend::Textured));
        }
        // Hearts (top-right to bottom-left): 2 absorption (1 full + 1 half),
        // 10 health (7 full + 1 half + 2 empty); regeneration offset_index = 28 % 25 = 3
        // lifts heart 3 by 2px (y = 199).
        let heart_rows: &[(i32, i32, Option<&'static str>)] = &[
            (130, 191, Some("hud/heart/absorbing_half")),
            (122, 191, Some("hud/heart/absorbing_full")),
            (194, 201, None),
            (186, 201, None),
            (178, 201, Some("hud/heart/half")),
            (170, 201, Some("hud/heart/full")),
            (162, 201, Some("hud/heart/full")),
            (154, 201, Some("hud/heart/full")),
            (146, 199, Some("hud/heart/full")),
            (138, 201, Some("hud/heart/full")),
            (130, 201, Some("hud/heart/full")),
            (122, 201, Some("hud/heart/full")),
        ];
        for &(x, y, heart_fill) in heart_rows {
            expected_health.push(expect_sprite(
                "hud/heart/container",
                x,
                y,
                9,
                9,
                Blend::Textured,
            ));
            if let Some(sprite_name) = heart_fill {
                expected_health.push(expect_sprite(sprite_name, x, y, 9, 9, Blend::Textured));
            }
        }
        // Food: y = 201; 15 food = 7 full, 1 half, 2 empty (28 % 46 != 0 -> no wobble).
        for n in 0..10 {
            let x = 295 - n * 8;
            expected_health.push(expect_sprite(
                "hud/food_empty",
                x,
                201,
                9,
                9,
                Blend::Textured,
            ));
            if n < 7 {
                expected_health.push(expect_sprite(
                    "hud/food_full",
                    x,
                    201,
                    9,
                    9,
                    Blend::Textured,
                ));
            } else if n == 7 {
                expected_health.push(expect_sprite(
                    "hud/food_half",
                    x,
                    201,
                    9,
                    9,
                    Blend::Textured,
                ));
            }
        }
        // Air: y = 191; air = 181 / 300 -> 6 full, 1 bursting (#7), 3 empty (#8..=10).
        for b in 1..=10 {
            let x = 295 - (b - 1) * 8;
            let name = match b {
                1..=6 => "hud/air",
                7 => "hud/air_bursting",
                _ => "hud/air_empty",
            };
            expected_health.push(expect_sprite(name, x, 191, 9, 9, Blend::Textured));
        }

        let mut hud_vec = HealthHud::default();
        let mut hud_into = HealthHud::default();
        buf.clear();
        buf.push(sentinel.clone());
        hud_into.layout_into(&health_in, 427, 240, &mut buf);
        assert_eq!(buf[0], sentinel);
        assert_eq!(&buf[1..], expected_health.as_slice());
        assert_eq!(hud_vec.layout(&health_in, 427, 240), expected_health);
        assert_eq!(hud_into.bubble_pop_sound, hud_vec.bubble_pop_sound);
        assert!(hud_into.bubble_pop_sound.is_some());

        // Subsequent clear + reuse keeps the high-water capacity without reallocating.
        let cap_after_health = buf.capacity();
        assert!(cap_after_health >= cap_after_vehicle);
        buf.clear();
        buf.push(sentinel.clone());
        hotbar_into(427, 240, &hotbar_in, &mut buf);
        assert_eq!(buf[0], sentinel);
        assert_eq!(&buf[1..], &expected_hotbar[..]);
        assert_eq!(buf.capacity(), cap_after_health);
    }

    #[test]
    fn paint_handles_deep_pose_stack_effect_sprites_and_long_sprite_names() {
        let mut atlas = test_atlas();
        let white = atlas.white;
        let long_name = format!("mod_long/{}", "x".repeat(140));
        atlas
            .sprites
            .insert(format!("gui/sprites/{long_name}"), white);
        atlas
            .sprites
            .insert("gui/sprites/hud/test_short".into(), white);
        atlas.sprites.insert("mob_effect/speed".into(), white);

        let draws = [
            // Push 6 levels deep (exceeds SmallVec<[Pose; 4]> inline capacity).
            GuiDraw::Push(vec![PoseOp::Translate(1.0, 2.0)]),
            GuiDraw::Push(vec![PoseOp::Translate(2.0, 3.0)]),
            GuiDraw::Push(vec![PoseOp::Translate(3.0, 4.0)]),
            GuiDraw::Push(vec![PoseOp::Translate(4.0, 5.0)]),
            GuiDraw::Push(vec![PoseOp::Translate(5.0, 6.0)]),
            GuiDraw::Push(vec![PoseOp::Translate(6.0, 7.0)]),
            sprite("hud/test_short", 0, 0, 8, 8),
            GuiDraw::Sprite {
                sprite: Cow::Owned(long_name),
                x: 10,
                y: 0,
                w: 8,
                h: 8,
                region: None,
                color: 0xFFFF_FFFF,
                blend: Blend::Textured,
            },
            sprite("mob_effect/speed", 20, 0, 8, 8),
            GuiDraw::Pop,
            GuiDraw::Pop,
            GuiDraw::Pop,
            GuiDraw::Pop,
            GuiDraw::Pop,
            GuiDraw::Pop,
            // Back at identity pose.
            fill(0, 0, 4, 4, 0xFF11_2233),
        ];

        let mut batch = HudBatch::scratch([854, 480]);
        paint(&mut batch, &atlas, &[]);
        assert!(batch.vertices.is_empty());

        paint(&mut batch, &atlas, &draws);
        // 3 sprites + 1 fill = 4 quads = 24 vertices.
        assert_eq!(batch.vertices.len(), 24);
        // Cumulative translation of the 6 pushes is (21, 27).
        assert_eq!(batch.vertices[0].pos, [21.0, 27.0]);
        assert_eq!(batch.vertices[6].pos, [31.0, 27.0]);
        assert_eq!(batch.vertices[12].pos, [41.0, 27.0]);
        // After 6 pops, pose returns to identity.
        assert_eq!(batch.vertices[18].pos, [0.0, 0.0]);
        assert_eq!(batch.vertices[18].color, abgr(0xFF11_2233));

        // Second paint into a fresh batch produces bit-identical vertices.
        let mut expected = HudBatch::scratch([854, 480]);
        paint(&mut expected, &atlas, &draws);
        assert_eq!(
            bytemuck::cast_slice::<HudVertex, u8>(&batch.vertices),
            bytemuck::cast_slice::<HudVertex, u8>(&expected.vertices),
        );
    }

    #[test]
    fn paint_with_scratch_matches_paint_and_reuses_glyph_scratch() {
        let atlas = test_atlas();
        let width = |s: &str| HudBatch::text_width(&atlas, s) as i32;
        let draws_long = experience_colored([None; 2], 427, 240, 0.5, 12345, true, &width);
        let draws_short = experience_colored([None; 2], 427, 240, 0.5, 42, true, &width);

        let mut scratch = crate::hud::HudGlyphScratch::new();
        let mut with_scratch = HudBatch::scratch([854, 480]);
        paint_with_scratch(&mut with_scratch, &atlas, &draws_long, &mut scratch);
        let cap = scratch.capacity();
        assert!(!with_scratch.vertices.is_empty());
        assert!(cap >= "12345".len());
        assert!(scratch.is_empty());

        let mut plain = HudBatch::scratch([854, 480]);
        paint(&mut plain, &atlas, &draws_long);
        assert_eq!(
            bytemuck::cast_slice::<HudVertex, u8>(&with_scratch.vertices),
            bytemuck::cast_slice::<HudVertex, u8>(&plain.vertices),
        );

        // Subsequent shorter text draws reuse the warm scratch buffer without growing it.
        with_scratch.reset(
            [854, 480],
            crate::hud::GuiScale {
                requested: 0,
                unicode: false,
            },
        );
        paint_with_scratch(&mut with_scratch, &atlas, &draws_short, &mut scratch);
        assert!(scratch.is_empty());
        assert_eq!(scratch.capacity(), cap);

        let mut plain_short = HudBatch::scratch([854, 480]);
        paint(&mut plain_short, &atlas, &draws_short);
        assert_eq!(
            bytemuck::cast_slice::<HudVertex, u8>(&with_scratch.vertices),
            bytemuck::cast_slice::<HudVertex, u8>(&plain_short.vertices),
        );
    }
}
