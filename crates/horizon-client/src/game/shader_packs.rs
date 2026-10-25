//! Shader packs, for development until the Shader Packs page exists: `HORIZON_SHADER_PACK=<name>`
//! runs `shaderpacks/<name>` (a folder or a zip beside `options.txt`) over the world. The
//! programs of the current dimension's folder are loaded when the dimension changes (spec 1.2).

use horizon_client::camera::CameraFluid;
use horizon_client::gfx::pack_uniforms::PackInputs;
use horizon_shaders::{Macros, Pack, Properties, dimension_folder, load_post};

use super::{Dim, Game};

impl Game<'_> {
    /// Loads the pack for the current dimension when it changed, and returns this frame's
    /// inputs to the pack's uniforms.
    pub(super) fn shader_pack_inputs(&mut self, camera: glam::DVec3, partial: f32) -> PackInputs {
        let Some(name) = std::env::var_os("HORIZON_SHADER_PACK") else {
            return PackInputs::default();
        };
        let dimension = match self.dim {
            Dim::Overworld => 0,
            Dim::Nether => -1,
            Dim::End => 1,
        };
        if let Some(renderer) = &mut self.renderer
            && renderer.shader_pack_dimension() != Some(dimension)
        {
            let name = name.to_string_lossy();
            let started = std::time::Instant::now();
            let path = std::env::current_dir()
                .unwrap_or_default()
                .join("shaderpacks")
                .join(&*name);
            match Pack::open(&path) {
                Ok(pack) => {
                    let macros = Macros::standard();
                    let text = pack.read_text("shaders.properties").unwrap_or_default();
                    let properties = Properties::parse(&text, &macros);
                    for note in &properties.notes {
                        log::warn!(
                            "shader pack {name}: shaders.properties:{}: {}",
                            note.line,
                            note.message
                        );
                    }
                    let folder = dimension_folder(&pack, dimension);
                    let inputs = renderer.pack_inputs();
                    let post = load_post(&pack, &folder, &properties, &macros, &inputs);
                    if let Some((programs, notes)) = renderer.set_shader_pack(dimension, Some(post))
                    {
                        for note in notes {
                            log::warn!("shader pack {name}: {note}");
                        }
                        log::info!(
                            "shader pack {name}: folder `{folder}`, {} programs ({}) in {} ms",
                            programs.len(),
                            programs.join(" "),
                            started.elapsed().as_millis()
                        );
                    }
                }
                Err(e) => {
                    log::warn!("shader pack {name}: {e}");
                    renderer.set_shader_pack(dimension, None);
                }
            }
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
