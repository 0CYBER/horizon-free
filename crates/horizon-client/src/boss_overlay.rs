//! `BossHealthOverlay` and `LerpingBossEvent`: the boss bars a client is
//! showing, in the order their add packets arrived.
use crate::{
    hud_overlays::BossBar,
    scoreboard::{comp_text, tag_to_json},
    text::Text,
};
use horizon_protocol::packets::play::{BossAction, BossEvent};

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
    /// `ClientboundBossEventPacket.dispatch` into `BossHealthOverlay`'s
    /// handler. An id already shown keeps its place when added again.
    pub fn apply(&mut self, packet: &BossEvent, now: u128) {
        // An ordinal past the enum shows as its last value.
        let color = |c: i32| c.clamp(0, 6) as u8;
        let overlay = |o: i32| o.clamp(0, 4) as u8;
        let id = packet.id;
        if let BossAction::Add {
            name,
            progress,
            color: c,
            overlay: o,
            darken,
            music,
            fog,
        } = &packet.action
        {
            let event = Event {
                id,
                name: comp_text(&tag_to_json(name)),
                progress: *progress,
                target: *progress,
                set_at: now,
                color: color(*c),
                overlay: overlay(*o),
                darken: *darken,
                music: *music,
                fog: *fog,
            };
            match self.events.iter_mut().find(|e| e.id == id) {
                Some(slot) => *slot = event,
                None => self.events.push(event),
            }
            return;
        }
        if let BossAction::Remove = packet.action {
            self.events.retain(|e| e.id != id);
            return;
        }
        let Some(e) = self.events.iter_mut().find(|e| e.id == id) else {
            return;
        };
        match &packet.action {
            BossAction::UpdateProgress { progress } => e.set_progress(*progress, now),
            BossAction::UpdateName { name } => e.name = comp_text(&tag_to_json(name)),
            BossAction::UpdateStyle {
                color: c,
                overlay: o,
            } => {
                e.color = color(*c);
                e.overlay = overlay(*o);
            }
            BossAction::UpdateProperties { darken, music, fog } => {
                (e.darken, e.music, e.fog) = (*darken, *music, *fog);
            }
            BossAction::Add { .. } | BossAction::Remove => {}
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
    use horizon_world::nbt::Tag;

    fn add(id: u128, name: &str, progress: f32, darken: bool, fog: bool) -> BossEvent {
        BossEvent {
            id,
            action: BossAction::Add {
                name: Tag::String(name.into()),
                progress,
                color: 5,
                overlay: 0,
                darken,
                music: false,
                fog,
            },
        }
    }
    fn progress(id: u128, progress: f32) -> BossEvent {
        BossEvent {
            id,
            action: BossAction::UpdateProgress { progress },
        }
    }

    #[test]
    fn progress_lerps_over_a_tenth_of_a_second() {
        let mut overlay = BossOverlay::default();
        overlay.apply(&add(1, "W", 0.0, true, false), 0);
        overlay.apply(&progress(1, 1.0), 0);
        assert!((overlay.bars(50)[0].progress - 0.5).abs() < 1.0e-5);
        assert!((overlay.bars(100)[0].progress - 1.0).abs() < 1.0e-5);
        // A new target starts from the value on screen.
        overlay.apply(&progress(1, 0.0), 50);
        assert!((overlay.bars(150)[0].progress - 0.0).abs() < 1.0e-5);
    }

    #[test]
    fn darken_ramps_and_fades() {
        let mut overlay = BossOverlay::default();
        overlay.apply(&add(1, "W", 1.0, true, false), 0);
        overlay.tick_darken(true);
        assert!((overlay.darken_amount(1.0) - 0.05).abs() < 1.0e-5);
        overlay.apply(
            &BossEvent {
                id: 1,
                action: BossAction::Remove,
            },
            0,
        );
        overlay.tick_darken(true);
        assert!((overlay.darken - 0.0375).abs() < 1.0e-5);
        assert!(!overlay.should_play_music());
    }

    #[test]
    fn bars_keep_arrival_order() {
        let mut overlay = BossOverlay::default();
        overlay.apply(&add(2, "b", 1.0, false, true), 0);
        overlay.apply(&add(1, "a", 1.0, false, false), 0);
        overlay.apply(&add(2, "c", 0.5, false, true), 0);
        // An update for a bar this client never got is ignored.
        overlay.apply(&progress(9, 0.5), 0);
        let names: Vec<_> = overlay
            .bars(0)
            .into_iter()
            .map(|b| b.name.plain())
            .collect();
        assert_eq!(names, ["c", "a"]);
        assert!(overlay.should_create_world_fog());
        overlay.apply(
            &BossEvent {
                id: 2,
                action: BossAction::Remove,
            },
            0,
        );
        assert!(!overlay.should_create_world_fog());
    }
}
