//! `ToastManager`'s slide, shared by the game's toasts and the title
//! screens' system toasts.
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
