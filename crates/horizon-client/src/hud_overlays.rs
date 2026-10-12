//! The `Hud` overlays above the hotbar and decorations, laid out as
//! [`GuiDraw`] calls like [`crate::hud_layout`]: status effect icons
//! (`extractEffects`), titles and the action bar (`extractTitle`,
//! `extractOverlayMessage` and the title timers of `Hud.tick`), and the
//! selected item name (`extractSelectedItemName` and its `toolHighlightTimer`).
use crate::{
    font::Font,
    hud_layout::{GuiDraw, PoseOp, fill, hsv_to_argb, multiply, white, width},
    text::{Style, Text},
};
use std::{borrow::Cow, cmp::Ordering};

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

/// `graphics.textWithBackdrop`; `background` is `Options.getBackgroundColor(0.0F)`.
pub fn text_with_backdrop(
    out: &mut Vec<GuiDraw>,
    t: Text,
    x: i32,
    y: i32,
    w: i32,
    color: u32,
    background: u32,
) {
    if background != 0 {
        out.push(fill(
            x - 2,
            y - 2,
            x + w + 2,
            y + 9 + 2,
            multiply(background, color),
        ));
    }
    text(out, t, x, y, color, true);
}

/// `Arrays.sort(T[], Comparator)` for the short arrays the HUD sorts: Java's
/// TimSort below `MIN_MERGE` (a natural run, then binary insertion), which
/// matters when the comparator is not a total order. Longer arrays fall back
/// to a stable sort.
pub fn java_sort<T>(a: &mut [T], cmp: impl Fn(&T, &T) -> Ordering) {
    let n = a.len();
    if n < 2 {
        return;
    }
    if n >= 32 {
        a.sort_by(cmp);
        return;
    }
    // countRunAndMakeAscending.
    let mut run = 2;
    if cmp(&a[1], &a[0]) == Ordering::Less {
        while run < n && cmp(&a[run], &a[run - 1]) == Ordering::Less {
            run += 1;
        }
        a[..run].reverse();
    } else {
        while run < n && cmp(&a[run], &a[run - 1]) != Ordering::Less {
            run += 1;
        }
    }
    // binarySort.
    for start in run..n {
        let (mut left, mut right) = (0, start);
        while left < right {
            let mid = (left + right) / 2;
            if cmp(&a[start], &a[mid]) == Ordering::Less {
                right = mid;
            } else {
                left = mid + 1;
            }
        }
        a[left..=start].rotate_right(1);
    }
}

/// A `MobEffectInstance` as `extractEffects` sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct EffectIcon {
    /// The effect id without namespace (`speed`): the sprite is `mob_effect/<id>`.
    pub id: String,
    /// `MobEffect.isBeneficial`.
    pub beneficial: bool,
    /// `MobEffect.getColor`.
    pub color: i32,
    pub ambient: bool,
    /// Remaining ticks, -1 for infinite.
    pub duration: i32,
    pub show_icon: bool,
}

/// `MobEffectInstance.compareTo`.
fn compare_effects(a: &EffectIcon, o: &EffectIcon) -> Ordering {
    if (a.duration <= 32147 || o.duration <= 32147) && !(a.ambient && o.ambient) {
        a.ambient
            .cmp(&o.ambient)
            .then((a.duration == -1).cmp(&(o.duration == -1)))
            .then(a.duration.cmp(&o.duration))
            .then(a.color.cmp(&o.color))
    } else {
        a.ambient.cmp(&o.ambient).then(a.color.cmp(&o.color))
    }
}

/// `Hud.extractEffects`: `effects` in `getActiveEffects` order; `demo` moves
/// the icons below the demo timer.
pub fn effects(w: i32, demo: bool, effects: &[EffectIcon]) -> Vec<GuiDraw> {
    let mut out = Vec::new();
    let mut sorted: Vec<&EffectIcon> = effects.iter().collect();
    // Ordering.natural().reverse().sortedCopy.
    java_sort(&mut sorted, |a, b| compare_effects(b, a));
    let (mut beneficial, mut harmful) = (0, 0);
    for e in sorted.into_iter().filter(|e| e.show_icon) {
        let mut x = w;
        let mut y = 1 + if demo { 15 } else { 0 };
        if e.beneficial {
            beneficial += 1;
            x -= 25 * beneficial;
        } else {
            harmful += 1;
            x -= 25 * harmful;
            y += 26;
        }
        let mut alpha = 1.0f32;
        let background = if e.ambient {
            "hud/effect_background_ambient"
        } else {
            if e.duration != -1 && e.duration <= 200 {
                let remaining = e.duration;
                let used = 10 - remaining / 20;
                alpha = (remaining as f32 / 10.0 / 5.0 * 0.5).clamp(0.0, 0.5)
                    + horizon_core::geometry::trig::cos(f64::from(
                        remaining as f32 * std::f32::consts::PI / 5.0,
                    )) * (used as f32 / 10.0 * 0.25).clamp(0.0, 0.25);
                alpha = alpha.clamp(0.0, 1.0);
            }
            "hud/effect_background"
        };
        out.push(GuiDraw::Sprite {
            sprite: Cow::Borrowed(background),
            x,
            y,
            w: 24,
            h: 24,
            region: None,
            color: crate::hud_layout::WHITE,
        });
        out.push(GuiDraw::Sprite {
            sprite: Cow::Owned(format!("mob_effect/{}", e.id)),
            x: x + 3,
            y: y + 3,
            w: 18,
            h: 18,
            region: None,
            color: white(alpha),
        });
    }
    out
}

/// Titles and the action bar: `Hud.title`, `subtitle`, `titleTime`, the
/// fade times and `overlayMessageString` / `overlayMessageTime`.
#[derive(Clone, Debug, PartialEq)]
pub struct Titles {
    pub title: Option<Text>,
    pub subtitle: Option<Text>,
    pub time: i32,
    pub fade_in: i32,
    pub stay: i32,
    pub fade_out: i32,
    pub overlay: Option<Text>,
    pub overlay_time: i32,
    /// `animateOverlayMessageColor`: the "Now Playing" rainbow.
    pub overlay_animated: bool,
}
impl Default for Titles {
    fn default() -> Self {
        Self {
            title: None,
            subtitle: None,
            time: 0,
            fade_in: 10,
            stay: 70,
            fade_out: 20,
            overlay: None,
            overlay_time: 0,
            overlay_animated: false,
        }
    }
}
impl Titles {
    /// `setTitle`.
    pub fn set_title(&mut self, title: Text) {
        self.title = Some(title);
        self.time = self.fade_in + self.stay + self.fade_out;
    }
    /// `setSubtitle`.
    pub fn set_subtitle(&mut self, subtitle: Text) {
        self.subtitle = Some(subtitle);
    }
    /// `setTimes`: negative values keep the current time.
    pub fn set_times(&mut self, fade_in: i32, stay: i32, fade_out: i32) {
        if fade_in >= 0 {
            self.fade_in = fade_in;
        }
        if stay >= 0 {
            self.stay = stay;
        }
        if fade_out >= 0 {
            self.fade_out = fade_out;
        }
        if self.time > 0 {
            self.time = self.fade_in + self.stay + self.fade_out;
        }
    }
    /// `clearTitles`.
    pub fn clear(&mut self) {
        self.title = None;
        self.subtitle = None;
        self.time = 0;
    }
    /// `resetTitleTimes`.
    pub fn reset_times(&mut self) {
        (self.fade_in, self.stay, self.fade_out) = (10, 70, 20);
    }
    /// `setOverlayMessage`.
    pub fn set_overlay(&mut self, text: Text, animate: bool) {
        self.overlay = Some(text);
        self.overlay_time = 60;
        self.overlay_animated = animate;
    }
    /// The title and action bar part of `Hud.tick`.
    pub fn tick(&mut self) {
        if self.overlay_time > 0 {
            self.overlay_time -= 1;
        }
        if self.time > 0 {
            self.time -= 1;
            if self.time <= 0 {
                self.title = None;
                self.subtitle = None;
            }
        }
    }
    /// `Hud.extractOverlayMessage`.
    pub fn overlay_message(
        &self,
        w: i32,
        h: i32,
        partial: f32,
        font: &Font,
        background: u32,
    ) -> Vec<GuiDraw> {
        let mut out = Vec::new();
        let Some(message) = &self.overlay else {
            return out;
        };
        if self.overlay_time <= 0 {
            return out;
        }
        let t = self.overlay_time as f32 - partial;
        let alpha = ((t * 255.0 / 20.0) as i32).min(255);
        if alpha > 0 {
            out.push(GuiDraw::Push(vec![PoseOp::Translate(
                (w / 2) as f32,
                (h - 68) as f32,
            )]));
            let color = if self.overlay_animated {
                hsv_to_argb(t / 50.0, 0.7, 0.6, alpha as u32)
            } else {
                crate::hud_layout::argb(alpha as u32, 255, 255, 255)
            };
            let width = width(font, message);
            text_with_backdrop(
                &mut out,
                message.clone(),
                -width / 2,
                -4,
                width,
                color,
                background,
            );
            out.push(GuiDraw::Pop);
        }
        out
    }
    /// `Hud.extractTitle`.
    pub fn title(
        &self,
        w: i32,
        h: i32,
        partial: f32,
        font: &Font,
        background: u32,
    ) -> Vec<GuiDraw> {
        let mut out = Vec::new();
        let Some(title) = &self.title else {
            return out;
        };
        if self.time <= 0 {
            return out;
        }
        let t = self.time as f32 - partial;
        let mut alpha = 255;
        if self.time > self.fade_out + self.stay {
            let time = (self.fade_in + self.stay + self.fade_out) as f32 - t;
            alpha = (time * 255.0 / self.fade_in as f32) as i32;
        }
        if self.time <= self.fade_out {
            alpha = (t * 255.0 / self.fade_out as f32) as i32;
        }
        let alpha = alpha.clamp(0, 255);
        if alpha > 0 {
            out.push(GuiDraw::Push(vec![PoseOp::Translate(
                (w / 2) as f32,
                (h / 2) as f32,
            )]));
            out.push(GuiDraw::Push(vec![PoseOp::Scale(4.0, 4.0)]));
            let title_width = width(font, title);
            let color = crate::hud_layout::argb(alpha as u32, 255, 255, 255);
            text_with_backdrop(
                &mut out,
                title.clone(),
                -title_width / 2,
                -10,
                title_width,
                color,
                background,
            );
            out.push(GuiDraw::Pop);
            if let Some(subtitle) = &self.subtitle {
                out.push(GuiDraw::Push(vec![PoseOp::Scale(2.0, 2.0)]));
                let subtitle_width = width(font, subtitle);
                text_with_backdrop(
                    &mut out,
                    subtitle.clone(),
                    -subtitle_width / 2,
                    5,
                    subtitle_width,
                    color,
                    background,
                );
                out.push(GuiDraw::Pop);
            }
            out.push(GuiDraw::Pop);
        }
        out
    }
}

/// The selected item as `Hud.tick` compares it: its item id and hover name.
#[derive(Clone, Debug, PartialEq)]
pub struct HeldName {
    pub item: String,
    /// `ItemStack.getHoverName`.
    pub hover_name: Text,
    /// `getRarity().color()`.
    pub rarity_color: u32,
    /// The stack has `CUSTOM_NAME`.
    pub custom_name: bool,
}

/// `Hud.toolHighlightTimer` and `lastToolHighlight`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToolHighlight {
    pub timer: i32,
    pub last: Option<HeldName>,
}
impl ToolHighlight {
    /// The selected-item part of `Hud.tick`; `display_time` is the
    /// `notificationDisplayTime` option.
    pub fn tick(&mut self, selected: Option<HeldName>, display_time: f64) {
        match &selected {
            None => self.timer = 0,
            Some(s) => {
                let changed = self
                    .last
                    .as_ref()
                    .is_none_or(|l| l.item != s.item || l.hover_name != s.hover_name);
                if changed {
                    self.timer = (40.0 * display_time) as i32;
                } else if self.timer > 0 {
                    self.timer -= 1;
                }
            }
        }
        self.last = selected;
    }
    /// `Hud.extractSelectedItemName`; `can_hurt` is
    /// `gameMode.canHurtPlayer()` (survival and adventure).
    pub fn draw(
        &self,
        w: i32,
        h: i32,
        can_hurt: bool,
        font: &Font,
        background: u32,
    ) -> Vec<GuiDraw> {
        let mut out = Vec::new();
        let Some(last) = &self.last else {
            return out;
        };
        if self.timer <= 0 {
            return out;
        }
        let mut name = last.hover_name.clone().with_color(last.rarity_color);
        if last.custom_name {
            name = name.with_style(&Style {
                italic: Some(true),
                ..Style::default()
            });
        }
        let str_width = width(font, &name);
        let x = (w - str_width) / 2;
        let mut y = h - 59;
        if !can_hurt {
            y += 14;
        }
        let alpha = ((self.timer as f32 * 256.0 / 10.0) as i32).min(255);
        if alpha > 0 {
            let color = crate::hud_layout::argb(alpha as u32, 255, 255, 255);
            text_with_backdrop(&mut out, name, x, y, str_width, color, background);
        }
        out
    }
}
