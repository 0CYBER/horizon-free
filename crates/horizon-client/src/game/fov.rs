//! Field of view (`Camera.tickFov`, `calculateFov`,
//! `AbstractClientPlayer.getFieldOfViewModifier`) and the spyglass
//! (`SpyglassItem`, `Hud.extractSpyglassOverlay`).
use super::Game;
use horizon_client::{
    fog::{FogType, fog_type},
    hud::{HudBatch, rgba},
};

/// `Abilities.walkingSpeed`.
const WALKING_SPEED: f32 = 0.1;
/// `options.fovEffectScale` (vanilla default 1).
const FOV_EFFECT_SCALE: f32 = 1.0;

impl Game<'_> {
    /// `Player.isScoping`: using a spyglass.
    pub(super) fn scoping(&self) -> bool {
        self.using
            .and_then(|(slot, _)| self.stack_in(slot))
            .is_some_and(|s| &*s.item == "minecraft:spyglass")
    }
    /// `AbstractClientPlayer.getFieldOfViewModifier` (first person).
    fn fov_target(&self) -> f32 {
        let Some(p) = &self.player else { return 1.0 };
        let mut modifier = 1.0f32;
        // Movement speed (sprinting, speed and slowness) relative to walking.
        modifier *= (p.speed / WALKING_SPEED + 1.0) / 2.0;
        if let Some((slot, ticks)) = self.using
            && let Some(stack) = self.stack_in(slot)
        {
            match &*stack.item {
                "minecraft:bow" => {
                    let scale = (ticks as f32 / 20.0).min(1.0);
                    modifier *= 1.0 - scale * scale * 0.15;
                }
                "minecraft:spyglass" => return 0.1,
                _ => {}
            }
        }
        1.0 + (modifier - 1.0) * FOV_EFFECT_SCALE
    }
    /// `Camera.tickFov`.
    pub(super) fn tick_fov(&mut self) {
        let target = self.fov_target();
        let [_, current] = self.fov_modifier;
        let next = (current + (target - current) * 0.5).clamp(0.1, 1.5);
        self.fov_modifier = [current, next];
    }
    /// `Camera.calculateFov` with `modifyFovBasedOnDeathOrFluid`.
    pub(super) fn fov_degrees(&self, partial: f32, camera: [f64; 3]) -> f32 {
        let [old, new] = self.fov_modifier;
        let mut fov = self.settings.fov * (old + (new - old) * partial);
        if let Some(p) = &self.player
            && p.health.dead_or_dying()
        {
            let ticks = self
                .ui
                .died_at
                .map_or(0.0, |t| t.elapsed().as_secs_f32() * 20.0);
            let duration = (ticks + partial).min(20.0);
            fov /= (1.0 - 500.0 / (duration + 500.0)) * 2.0 + 1.0;
        }
        if matches!(
            fog_type(&self.world, &self.res.facts, camera, self.forward()),
            FogType::Water | FogType::Lava
        ) {
            fov *= 1.0 + (0.857_142_87 - 1.0) * FOV_EFFECT_SCALE;
        }
        fov
    }
    /// `SpyglassItem.use`: start scoping.
    pub(super) fn use_spyglass(&mut self) {
        if self.using.is_none() {
            self.using = Some((self.selected, 0));
            let at = self.eye_position();
            self.audio.play("minecraft:item.spyglass.use", at, 1.0, 1.0);
        }
    }
    /// `SpyglassItem.releaseUsing` / `finishUsingItem` (after 1200 ticks).
    pub(super) fn stop_spyglass(&mut self) {
        let at = self.eye_position();
        self.audio
            .play("minecraft:item.spyglass.stop_using", at, 1.0, 1.0);
    }
    /// `Hud.extractCameraOverlays`: the scope, its size easing from 0.5 to
    /// 1.125 at half a step per tick of frame time.
    pub(super) fn draw_spyglass(&mut self, batch: &mut HudBatch) {
        if !self.scoping() {
            self.scope_scale = 0.5;
            return;
        }
        self.scope_scale += (1.125 - self.scope_scale) * (0.5 * self.frame_ticks).min(1.0);
        let atlas = &self.res.atlas;
        let (w, h) = (batch.width, batch.height);
        let src = w.min(h);
        let ratio = (w / src).min(h / src) * self.scope_scale;
        let (sw, sh) = ((src * ratio).floor(), (src * ratio).floor());
        let (left, top) = (((w - sw) / 2.0).floor(), ((h - sh) / 2.0).floor());
        let (right, bottom) = (left + sw, top + sh);
        batch.sprite_scaled(atlas, "misc/spyglass_scope", left, top, sw, sh, u32::MAX);
        let black = rgba(0, 0, 0, 255);
        batch.fill(atlas, 0.0, bottom, w, h - bottom, black);
        batch.fill(atlas, 0.0, 0.0, w, top, black);
        batch.fill(atlas, 0.0, top, left, bottom - top, black);
        batch.fill(atlas, right, top, w - right, bottom - top, black);
    }
}
