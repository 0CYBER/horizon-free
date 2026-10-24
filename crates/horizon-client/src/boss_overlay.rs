//! `BossHealthOverlay` and `LerpingBossEvent`: the boss bars a client is
//! showing, in the order their add packets arrived.
use crate::{hud_overlays::BossBar, text::Text};

/// `LerpingBossEvent.LERP_MILLISECONDS`.
const LERP_MS: u128 = 100;

/// `Util.getMillis`.
pub fn now_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// One `LerpingBossEvent`.
#[derive(Clone, Debug)]
struct Event {
    id: u128,
    name: Text,
    /// The progress the lerp started from (`progress`).
    progress: f32,
    /// `targetPercent`.
    target: f32,
    /// `setTime`.
    set_at: u128,
    color: u8,
    overlay: u8,
    darken: bool,
    music: bool,
    fog: bool,
}
impl Event {
    /// `LerpingBossEvent.getProgress`.
    fn displayed(&self, now: u128) -> f32 {
        let t = ((now.saturating_sub(self.set_at)) as f32 / LERP_MS as f32).clamp(0.0, 1.0);
        self.progress + (self.target - self.progress) * t
    }
    /// `LerpingBossEvent.setProgress`: the bar starts the next lerp from
    /// whatever it is showing now.
    fn set_progress(&mut self, progress: f32, now: u128) {
        self.progress = self.displayed(now);
        self.target = progress;
        self.set_at = now;
    }
}

/// `BossHealthOverlay`: the events, plus `GameRenderer`'s world darkening.
#[derive(Clone, Debug, Default)]
pub struct BossOverlay {
    events: Vec<Event>,
    /// `bossOverlayWorldDarkening` and the previous tick's value.
    darken: f32,
    darken_o: f32,
}
impl BossOverlay {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    /// `Handler.add`. An id already shown keeps its place.
    pub fn add(
        &mut self,
        id: u128,
        name: Text,
        progress: f32,
        color: u8,
        overlay: u8,
        darken: bool,
        music: bool,
        fog: bool,
        now: u128,
    ) {
        let event = Event {
            id,
            name,
            progress,
            target: progress,
            set_at: now,
            color: color.min(6),
            overlay: overlay.min(4),
            darken,
            music,
            fog,
        };
        if let Some(slot) = self.events.iter_mut().find(|e| e.id == id) {
            *slot = event;
        } else {
            self.events.push(event);
        }
    }
    pub fn remove(&mut self, id: u128) {
        self.events.retain(|e| e.id != id);
    }
    pub fn set_progress(&mut self, id: u128, progress: f32, now: u128) {
        if let Some(e) = self.events.iter_mut().find(|e| e.id == id) {
            e.set_progress(progress, now);
        }
    }
    pub fn set_name(&mut self, id: u128, name: Text) {
        if let Some(e) = self.events.iter_mut().find(|e| e.id == id) {
            e.name = name;
        }
    }
    pub fn set_style(&mut self, id: u128, color: u8, overlay: u8) {
        if let Some(e) = self.events.iter_mut().find(|e| e.id == id) {
            e.color = color.min(6);
            e.overlay = overlay.min(4);
        }
    }
    pub fn set_properties(&mut self, id: u128, darken: bool, music: bool, fog: bool) {
        if let Some(e) = self.events.iter_mut().find(|e| e.id == id) {
            e.darken = darken;
            e.music = music;
            e.fog = fog;
        }
    }
    /// `GameRenderer.tick` while the level runs.
    pub fn tick_darken(&mut self, running: bool) {
        if !running {
            return;
        }
        self.darken_o = self.darken;
        if self.should_darken() {
            self.darken = (self.darken + 0.05).min(1.0);
        } else if self.darken > 0.0 {
            self.darken = (self.darken - 0.0125).max(0.0);
        }
    }
    /// `GameRenderer.bossOverlayWorldDarkening`.
    pub fn darken_amount(&self, partial: f32) -> f32 {
        let t = partial.clamp(0.0, 1.0);
        self.darken_o + (self.darken - self.darken_o) * t
    }
    pub fn should_darken(&self) -> bool {
        self.events.iter().any(|e| e.darken)
    }
    pub fn should_play_music(&self) -> bool {
        self.events.iter().any(|e| e.music)
    }
    pub fn should_create_world_fog(&self) -> bool {
        self.events.iter().any(|e| e.fog)
    }
    /// The bars `extractRenderState` walks, with the lerped progress.
    pub fn bars(&self, now: u128) -> Vec<BossBar> {
        self.events
            .iter()
            .map(|e| BossBar {
                name: e.name.clone(),
                progress: e.displayed(now),
                color: e.color as usize,
                overlay: e.overlay as usize,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_lerps_over_a_tenth_of_a_second() {
        let mut overlay = BossOverlay::default();
        overlay.add(1, Text::literal("W"), 0.0, 5, 0, true, false, false, 0);
        overlay.set_progress(1, 1.0, 0);
        assert!((overlay.bars(50)[0].progress - 0.5).abs() < 1.0e-5);
        assert!((overlay.bars(100)[0].progress - 1.0).abs() < 1.0e-5);
        // A new target starts from the value on screen.
        overlay.set_progress(1, 0.0, 50);
        assert!((overlay.bars(150)[0].progress - 0.0).abs() < 1.0e-5);
    }

    #[test]
    fn darken_ramps_and_fades() {
        let mut overlay = BossOverlay::default();
        overlay.add(1, Text::literal("W"), 1.0, 5, 0, true, false, false, 0);
        overlay.tick_darken(true);
        assert!((overlay.darken_amount(1.0) - 0.05).abs() < 1.0e-5);
        overlay.remove(1);
        overlay.tick_darken(true);
        assert!((overlay.darken - 0.0375).abs() < 1.0e-5);
        assert!(!overlay.should_play_music());
    }

    #[test]
    fn bars_keep_arrival_order() {
        let mut overlay = BossOverlay::default();
        overlay.add(2, Text::literal("b"), 1.0, 0, 0, false, true, true, 0);
        overlay.add(1, Text::literal("a"), 1.0, 0, 0, false, false, false, 0);
        overlay.add(2, Text::literal("c"), 0.5, 1, 2, false, true, true, 0);
        let names: Vec<_> = overlay
            .bars(0)
            .into_iter()
            .map(|b| b.name.plain())
            .collect();
        assert_eq!(names, ["c", "a"]);
        assert!(overlay.should_create_world_fog());
        overlay.remove(2);
        assert!(!overlay.should_create_world_fog());
    }
}
