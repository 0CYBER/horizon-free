//! `ToastManager`'s slide, shared by the game's toasts and the title
//! screens' system toasts.
use crate::hud::{HudAtlas, HudBatch};
use std::time::Instant;

/// `ToastManager.ToastInstance`: slides a toast in and out over 600 ms,
/// eased by the square, and counts how long it has been fully shown.
#[derive(Default)]
pub struct Slide {
    /// When it first updated; the times below are milliseconds since.
    pub origin: Option<Instant>,
    begun: bool,
    /// `animationStartTime`.
    start: i64,
    /// `becameFullyVisibleAt`.
    fully_at: i64,
    show: bool,
    /// `visiblePortion`: 0 off screen, 1 fully in.
    pub portion: f32,
    /// `fullyVisibleFor`.
    pub fully_for: i64,
}
impl Slide {
    pub const MS: i64 = 600;
    /// `ToastInstance.update` at `now`; `wants_show` is the toast's own
    /// update. The visibility it switched to (the sound to play), and whether
    /// it has finished sliding out.
    pub fn update(
        &mut self,
        now: i64,
        wants_show: impl FnOnce(i64) -> bool,
    ) -> (Option<bool>, bool) {
        let was = self.show;
        if !self.begun {
            self.begun = true;
            self.start = now;
            self.show = true;
        }
        if self.show && now - self.start <= Self::MS {
            self.fully_at = now;
        }
        self.fully_for = now - self.fully_at;
        let progress = ((now - self.start) as f32 / Self::MS as f32).clamp(0.0, 1.0);
        let progress = progress * progress;
        self.portion = if self.show { progress } else { 1.0 - progress };
        let wanted = wants_show(self.fully_for);
        if wanted != self.show {
            self.start = now - ((1.0 - self.portion) * Self::MS as f32) as i64;
            self.show = wanted;
        }
        let finished = !self.show && now - self.start > Self::MS;
        ((self.show != was).then_some(self.show), finished)
    }
    /// `update` at the time since the first one.
    pub fn update_now(&mut self, wants_show: impl FnOnce(i64) -> bool) -> (Option<bool>, bool) {
        let origin = *self.origin.get_or_insert_with(Instant::now);
        self.update(origin.elapsed().as_millis() as i64, wants_show)
    }
}

/// `SystemToast`: a yellow title over a white message (or the title alone,
/// lower down) on the system background. It stays until its display time
/// has passed since its last change, or until it is hidden.
pub struct SystemToast {
    pub slide: Slide,
    /// `ToastInstance.firstSlotIndex`: its 32-pixel slot once shown.
    pub slot: Option<usize>,
    title: String,
    message: Option<String>,
    /// `changed`: the next update takes its time as `lastChanged`.
    changed: bool,
    last_changed: i64,
    force_hide: bool,
}
impl SystemToast {
    /// `SystemToastId.displayTime`'s default, in milliseconds.
    const DISPLAY_TIME: f64 = 5000.0;
    pub fn new(title: String, message: Option<String>) -> Self {
        Self {
            slide: Slide::default(),
            slot: None,
            title,
            message,
            changed: false,
            last_changed: 0,
            force_hide: false,
        }
    }
    /// `reset`: new text, and the display time starts again.
    pub fn reset(&mut self, title: String, message: Option<String>) {
        self.title = title;
        self.message = message;
        self.changed = true;
    }
    pub fn force_hide(&mut self) {
        self.force_hide = true;
    }
    /// `ToastInstance.update` with `SystemToast.update`, the display time
    /// scaled by the Notification Time option: the visibility it switched
    /// to (the sound to play), and whether it has slid out.
    pub fn update(&mut self, multiplier: f64) -> (Option<bool>, bool) {
        self.slide.update_now(|fully| {
            if std::mem::take(&mut self.changed) {
                self.last_changed = fully;
            }
            !self.force_hide
                && ((fully - self.last_changed) as f64) < Self::DISPLAY_TIME * multiplier
        })
    }
    /// `extractRenderState` at `y`, in from the right edge by the visible
    /// portion (`Toast.xPos`).
    pub fn draw(&self, atlas: &HudAtlas, batch: &mut HudBatch, y: f32) {
        let width = std::iter::once(&self.title)
            .chain(&self.message)
            .map(|line| HudBatch::text_width(atlas, line))
            .fold(0.0, f32::max)
            .ceil()
            .max(160.0)
            + 30.0;
        let x = batch.width - width * self.slide.portion;
        background(atlas, batch, x, y, width);
        let yellow = crate::hud::rgba(255, 255, 0, 255);
        match &self.message {
            Some(message) => {
                batch.text(atlas, &self.title, x + 18.0, y + 7.0, yellow, false);
                batch.text(atlas, message, x + 18.0, y + 19.0, u32::MAX, false);
            }
            None => {
                batch.text(atlas, &self.title, x + 18.0, y + 12.0, yellow, false);
            }
        }
    }
}
/// SystemToast's 32-pixel background keeps the notice icon and borders intact.
pub fn background(atlas: &HudAtlas, batch: &mut HudBatch, x: f32, y: f32, width: f32) {
    let Some(rect) = atlas.sprites.get("gui/sprites/toast/system").copied() else {
        return;
    };
    let source_x = [0.0, 17.0, rect.w as f32 - 4.0, rect.w as f32];
    let target_x = [x, x + 17.0, x + width - 4.0, x + width];
    for col in 0..3 {
        for (sy, sh, y, h) in [
            (0.0, 28.0, y, 28.0),
            (rect.h as f32 - 4.0, 4.0, y + 28.0, 4.0),
        ] {
            let mut piece = rect;
            let du = (rect.u1 - rect.u0) / rect.w as f32;
            let dv = (rect.v1 - rect.v0) / rect.h as f32;
            piece.u0 = rect.u0 + source_x[col] * du;
            piece.u1 = rect.u0 + source_x[col + 1] * du;
            piece.v0 = rect.v0 + sy * dv;
            piece.v1 = rect.v0 + (sy + sh) * dv;
            batch.rect_uv(
                target_x[col],
                y,
                target_x[col + 1] - target_x[col],
                h,
                piece,
                u32::MAX,
            );
        }
    }
}
