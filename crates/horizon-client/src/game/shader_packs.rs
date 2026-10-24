//! Shader packs, for development until the Shader Packs page exists: `HORIZON_SHADER_PACK=<name>`
//! (the runtime that uses it comes next). What the pack's uniforms need from the game.

use horizon_client::camera::CameraFluid;
use horizon_client::gfx::pack_uniforms::PackInputs;

use super::Game;

impl Game<'_> {
    /// This frame's inputs to the pack's uniforms.
    pub(super) fn shader_pack_inputs(&mut self, camera: glam::DVec3, partial: f32) -> PackInputs {
        if std::env::var_os("HORIZON_SHADER_PACK").is_none() {
            return PackInputs::default();
        }
        let lerp = |(old, new): (f32, f32)| old + (new - old) * partial;
        let at = camera.floor().as_ivec3();
        let light = self.world.light(at.x, at.y, at.z);
        PackInputs {
            day_time: self.world.day_time,
            rain: lerp(self.world.weather.rain_level),
            thunder: lerp(self.world.weather.thunder_level),
            eye_in: match self.camera_fluid(camera.to_array()) {
                CameraFluid::None => 0,
                CameraFluid::Water => 1,
                CameraFluid::Lava => 2,
                CameraFluid::PowderSnow => 3,
            },
            blindness: f32::from(u8::from(self.potions.effects.get("blindness").is_some())),
            eye_light: [i32::from(light & 15), i32::from(light >> 4)],
            hide_gui: self.ui.hide_gui,
            brightness: self.settings.gamma,
        }
    }
}
