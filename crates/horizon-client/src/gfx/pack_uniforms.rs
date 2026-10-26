//! The uniforms a shader pack's post-processing programs read once per frame (spec 5.2, class
//! F), from the renderer's `Frame` and the few values only the game knows (`PackInputs`).
//! Matrices follow GL's conventions (5.5): `gbufferProjection` maps view depth to clip z in
//! [-w, w], and the depth buffers packs read hold GL window depth.

use glam::{Mat4, Vec3, Vec4};

use super::Frame;

/// What the game passes for the pack's uniforms, beyond what `Frame` carries.
#[derive(Clone, Copy, Debug, Default)]
pub struct PackInputs {
    /// `Level.getDayTime`.
    pub day_time: i64,
    /// The interpolated rain and thunder levels, 0..1.
    pub rain: f32,
    pub thunder: f32,
    /// The fluid at the camera: 0 none, 1 water, 2 lava, 3 powder snow.
    pub eye_in: i32,
    pub blindness: f32,
    /// Block and sky light at the camera, 0..15.
    pub eye_light: [i32; 2],
    pub hide_gui: bool,
    /// The brightness option, 0..1.
    pub brightness: f32,
    /// The biome at the camera (3.4): registry id, `CAT_*`, `PPT_*`, temperature, downfall.
    pub biome: i32,
    pub biome_category: i32,
    pub biome_precipitation: i32,
    pub temperature: f32,
    pub rainfall: f32,
}

/// One uniform's value: up to 16 floats or 3 integers.
#[derive(Clone, Copy, Debug)]
pub enum Value {
    F([f32; 16]),
    I([i32; 3]),
}

impl Value {
    fn f(v: &[f32]) -> Self {
        let mut a = [0.0; 16];
        a[..v.len()].copy_from_slice(v);
        Value::F(a)
    }
    fn i(v: &[i32]) -> Self {
        let mut a = [0; 3];
        a[..v.len()].copy_from_slice(v);
        Value::I(a)
    }

    /// Writes the value as the type the pack declared, at `offset` of a std140 block.
    pub fn write(&self, ty: &str, out: &mut [u8], offset: usize) {
        let ints = matches!(ty, "int" | "bool" | "uint") || ty.starts_with(['i', 'u']);
        let (floats, integers): ([f32; 16], [i32; 16]) = match *self {
            Value::F(f) => (f, f.map(|v| v as i32)),
            Value::I(i) => {
                let mut f = [0.0; 16];
                let mut v = [0; 16];
                for k in 0..3 {
                    f[k] = i[k] as f32;
                    v[k] = i[k];
                }
                (f, v)
            }
        };
        let count = match ty {
            "mat4" => 16,
            "mat3" => {
                // std140: three columns, each padded to a vec4, from the mat4's upper left.
                for c in 0..3 {
                    let column: [f32; 3] = std::array::from_fn(|r| floats[c * 4 + r]);
                    put(out, offset + c * 16, bytemuck::cast_slice(&column));
                }
                return;
            }
            t => match t.as_bytes().last() {
                Some(n @ b'2'..=b'4') if t.contains("vec") => (n - b'0') as usize,
                _ => 1,
            },
        };
        if ints {
            put(out, offset, bytemuck::cast_slice(&integers[..count]));
        } else {
            put(out, offset, bytemuck::cast_slice(&floats[..count]));
        }
    }
}

fn put(out: &mut [u8], offset: usize, bytes: &[u8]) {
    if let Some(dst) = out.get_mut(offset..offset + bytes.len()) {
        dst.copy_from_slice(bytes);
    }
}

/// The uniforms provided, in the order of [`FrameUniforms::values`].
pub const NAMES: &[&str] = &[
    "cameraPosition",
    "previousCameraPosition",
    "eyeAltitude",
    "upPosition",
    "sunPosition",
    "moonPosition",
    "shadowLightPosition",
    "sunAngle",
    "shadowAngle",
    "worldTime",
    "worldDay",
    "moonPhase",
    "frameCounter",
    "frameTime",
    "frameTimeCounter",
    "viewWidth",
    "viewHeight",
    "aspectRatio",
    "near",
    "far",
    "rainStrength",
    "wetness",
    "thunderStrength",
    "fogColor",
    "skyColor",
    "fogMode",
    "fogStart",
    "fogEnd",
    "fogDensity",
    "fogShape",
    "isEyeInWater",
    "blindness",
    "nightVision",
    "darknessFactor",
    "darknessLightFactor",
    "screenBrightness",
    "hideGUI",
    "eyeBrightness",
    "eyeBrightnessSmooth",
    "heldItemId",
    "heldItemId2",
    "heldBlockLightValue",
    "heldBlockLightValue2",
    "renderStage",
    "biome",
    "biome_category",
    "biome_precipitation",
    "temperature",
    "rainfall",
    "gbufferModelView",
    "gbufferModelViewInverse",
    "gbufferProjection",
    "gbufferProjectionInverse",
    "gbufferPreviousModelView",
    "gbufferPreviousProjection",
    "atlasSize",
    "shadowModelView",
    "shadowModelViewInverse",
    "shadowProjection",
    "shadowProjectionInverse",
];

/// Values carried from frame to frame.
pub struct FrameUniforms {
    pub values: Vec<Value>,
    /// The block atlas's size (`atlasSize`), 0 without one.
    pub atlas: [u32; 2],
    start: std::time::Instant,
    last: Option<std::time::Instant>,
    frame: u32,
    previous: Option<(Vec3, Mat4, Mat4)>,
    wetness: f32,
    eye_smooth: [f32; 2],
}

impl Default for FrameUniforms {
    fn default() -> Self {
        Self {
            values: Vec::new(),
            atlas: [0; 2],
            start: std::time::Instant::now(),
            last: None,
            frame: 0,
            previous: None,
            wetness: 0.0,
            eye_smooth: [0.0; 2],
        }
    }
}

/// `(0, 100, 0)` on the frame the sky draws a body on: Y -90°, then X by its angle in degrees.
fn body(angle: f32) -> Vec3 {
    (Mat4::from_rotation_y((-90.0f32).to_radians()) * Mat4::from_rotation_x(angle.to_radians()))
        .transform_vector3(Vec3::new(0.0, 100.0, 0.0))
}

/// `value` moved toward `target` with a half-life of `half_life` seconds over `dt` (5.6).
fn smooth(value: f32, target: f32, half_life: f32, dt: f32) -> f32 {
    target + (value - target) * 0.5f32.powf(dt / half_life)
}

impl FrameUniforms {
    /// This frame's values for a render size of `size` pixels.
    pub fn update(&mut self, frame: &Frame, inputs: &PackInputs, size: [u32; 2]) {
        let now = std::time::Instant::now();
        let dt = self.last.map_or(0.0, |l| (now - l).as_secs_f32());
        self.last = Some(now);
        let aspect = size[0] as f32 / size[1].max(1) as f32;
        let view = crate::camera::view_matrix(frame.yaw, frame.pitch);
        let projection = Mat4::perspective_rh_gl(
            frame.fov_degrees.to_radians(),
            aspect,
            0.05,
            frame.depth_far,
        ) * frame.bob;
        let camera = frame.camera.as_vec3();
        let (previous_camera, previous_view, previous_projection) =
            self.previous
                .unwrap_or((Vec3::ZERO, Mat4::ZERO, Mat4::ZERO));
        self.previous = Some((camera, view, projection));

        let to_view = |v: Vec3| view.transform_vector3(v);
        let sun_angle = (frame.sky.sun_angle / 360.0 + 0.25).rem_euclid(1.0);
        let day = sun_angle < 0.5;
        let sun = to_view(body(frame.sky.sun_angle));
        let moon = to_view(body(frame.sky.moon_angle));
        // Half-lives are given in ticks (7.2); smoothing runs per frame in seconds (OQ-26).
        let half_life = if inputs.rain > self.wetness {
            600.0
        } else {
            200.0
        };
        self.wetness = smooth(self.wetness, inputs.rain, half_life / 20.0, dt);
        let eye = inputs.eye_light.map(|l| (l * 16) as f32);
        for (smoothed, target) in self.eye_smooth.iter_mut().zip(eye) {
            *smoothed = smooth(*smoothed, target, 10.0 / 20.0, dt);
        }
        let (fog_color, fog_start, fog_end) = match &frame.fog {
            Some(f) => (
                [f.color[0], f.color[1], f.color[2]],
                f.environmental_start.min(f.render_distance_start),
                f.environmental_end.min(f.render_distance_end),
            ),
            None => (frame.sky.fog_color, frame.distance * 0.9, frame.distance),
        };
        // The shadow camera (OQ-6, impl-plan.md): orthographic, `shadowDistance` (160) blocks
        // each side, 100 blocks from the player toward the shadow light, world +z up.
        let light = view
            .inverse()
            .transform_vector3(if day { sun } else { moon })
            .normalize();
        let shadow_view = Mat4::look_at_rh(light * 100.0, Vec3::ZERO, Vec3::Z);
        let shadow_projection = Mat4::orthographic_rh_gl(-160.0, 160.0, -160.0, 160.0, 0.05, 256.0);
        let day_time = inputs.day_time.rem_euclid(24000) as i32;
        let world_day = inputs.day_time.div_euclid(24000);
        let seconds = (now - self.start).as_secs_f32() % 3600.0;

        let value = |name: &str| -> Value {
            let v3 = |v: Vec3| Value::f(&v.to_array());
            let m4 = |m: Mat4| Value::f(&m.to_cols_array());
            match name {
                "cameraPosition" => v3(camera),
                "previousCameraPosition" => v3(previous_camera),
                "eyeAltitude" => Value::f(&[frame.camera.y as f32]),
                "upPosition" => v3(to_view(Vec3::new(0.0, 100.0, 0.0))),
                "sunPosition" => v3(sun),
                "moonPosition" => v3(moon),
                "shadowLightPosition" => v3(if day { sun } else { moon }),
                "sunAngle" => Value::f(&[sun_angle]),
                "shadowAngle" => Value::f(&[if day { sun_angle } else { sun_angle - 0.5 }]),
                "worldTime" => Value::i(&[day_time]),
                "worldDay" => Value::i(&[world_day as i32]),
                "moonPhase" => Value::i(&[world_day.rem_euclid(8) as i32]),
                "frameCounter" => Value::i(&[self.frame as i32]),
                "shadowModelView" => m4(shadow_view),
                "shadowModelViewInverse" => m4(shadow_view.inverse()),
                "shadowProjection" => m4(shadow_projection),
                "shadowProjectionInverse" => m4(shadow_projection.inverse()),
                "atlasSize" => Value::i(&[self.atlas[0] as i32, self.atlas[1] as i32]),
                "frameTime" => Value::f(&[dt]),
                "frameTimeCounter" => Value::f(&[seconds]),
                "viewWidth" => Value::f(&[size[0] as f32]),
                "viewHeight" => Value::f(&[size[1] as f32]),
                "aspectRatio" => Value::f(&[aspect]),
                "near" => Value::f(&[0.05]),
                "far" => Value::f(&[frame.distance]),
                "rainStrength" => Value::f(&[inputs.rain]),
                "wetness" => Value::f(&[self.wetness]),
                "thunderStrength" => Value::f(&[inputs.thunder]),
                "fogColor" => Value::f(&fog_color),
                "skyColor" => Value::f(&frame.sky.sky_color),
                // GL_LINEAR: vanilla's fog is linear.
                "fogMode" => Value::i(&[9729]),
                "fogStart" => Value::f(&[fog_start]),
                "fogEnd" => Value::f(&[fog_end]),
                "fogDensity" => Value::f(&[1.0]),
                "fogShape" => Value::i(&[0]),
                "isEyeInWater" => Value::i(&[inputs.eye_in]),
                "blindness" => Value::f(&[inputs.blindness]),
                "nightVision" => Value::f(&[frame.night_vision]),
                "darknessFactor" => Value::f(&[frame.darkness.0]),
                "darknessLightFactor" => Value::f(&[frame.darkness.1]),
                "screenBrightness" => Value::f(&[inputs.brightness]),
                "hideGUI" => Value::i(&[inputs.hide_gui as i32]),
                "eyeBrightness" => Value::i(&eye.map(|v| v as i32)),
                "biome" => Value::i(&[inputs.biome]),
                "biome_category" => Value::i(&[inputs.biome_category]),
                "biome_precipitation" => Value::i(&[inputs.biome_precipitation]),
                "temperature" => Value::f(&[inputs.temperature]),
                "rainfall" => Value::f(&[inputs.rainfall]),
                "eyeBrightnessSmooth" => Value::i(&self.eye_smooth.map(|v| v.round() as i32)),
                "gbufferModelView" => m4(view),
                "gbufferModelViewInverse" => m4(view.inverse()),
                "gbufferProjection" => m4(projection),
                "gbufferProjectionInverse" => m4(projection.inverse()),
                "gbufferPreviousModelView" => m4(previous_view),
                "gbufferPreviousProjection" => m4(previous_projection),
                // Held items wait for `item.properties` (5.3: 0 without the file); renderStage
                // is NONE outside gbuffers draws.
                _ => Value::i(&[0]),
            }
        };
        self.values = NAMES.iter().map(|n| value(n)).collect();
        self.frame = (self.frame + 1) % 720_720;
    }

    /// `hz_fog_color` and `hz_fog` of the translator's block (the compatibility `gl_Fog`).
    pub fn fog(&self) -> [Vec4; 2] {
        let get = |name| match index(name).and_then(|i| self.values.get(i)) {
            Some(Value::F(f)) => *f,
            _ => [0.0; 16],
        };
        let (color, start, end) = (get("fogColor"), get("fogStart")[0], get("fogEnd")[0]);
        [
            Vec4::new(color[0], color[1], color[2], 1.0),
            Vec4::new(1.0, start, end, 1.0 / (end - start).max(1e-3)),
        ]
    }
}

impl FrameUniforms {
    /// Evaluates the pack's custom uniforms with this frame's values and puts them after
    /// `NAMES`' values, in `custom.uniforms()` order.
    pub fn eval_custom(&mut self, custom: &mut horizon_shaders::Custom) {
        self.values.truncate(NAMES.len());
        let values = &self.values;
        let builtin = |name: &str| {
            let v = values.get(index(name)?)?;
            let n = components(name);
            Some(match *v {
                Value::F(f) => horizon_shaders::Val::vector(&f.map(f64::from)[..n]),
                Value::I(i) => horizon_shaders::Val::vector(&i.map(f64::from)[..n.min(3)]),
            })
        };
        let dt = match index("frameTime").and_then(|i| values.get(i)) {
            Some(Value::F(f)) => f64::from(f[0]),
            _ => 0.0,
        };
        let out = custom.eval(dt, &builtin);
        self.values
            .extend(out.iter().map(|v| Value::f(&v.v.map(|x| x as f32)[..v.n])));
    }
}

/// How many components the uniform `name` has (16 for a matrix).
fn components(name: &str) -> usize {
    match name {
        "cameraPosition"
        | "previousCameraPosition"
        | "upPosition"
        | "sunPosition"
        | "moonPosition"
        | "shadowLightPosition"
        | "fogColor"
        | "skyColor" => 3,
        "eyeBrightness" | "eyeBrightnessSmooth" | "atlasSize" => 2,
        n if n.starts_with("gbuffer") || n.starts_with("shadow") && n != "shadowAngle" => 16,
        _ => 1,
    }
}

/// The position of `name` in [`NAMES`].
pub fn index(name: &str) -> Option<usize> {
    NAMES.iter().position(|n| *n == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_follow_the_declared_type() {
        let mut out = vec![0u8; 64];
        Value::i(&[7]).write("float", &mut out, 0);
        assert_eq!(f32::from_ne_bytes(out[0..4].try_into().unwrap()), 7.0);
        Value::f(&[1.5, 2.5]).write("ivec2", &mut out, 4);
        assert_eq!(i32::from_ne_bytes(out[8..12].try_into().unwrap()), 2);
        Value::i(&[1]).write("bool", &mut out, 16);
        assert_eq!(i32::from_ne_bytes(out[16..20].try_into().unwrap()), 1);
        Value::f(&[1.0; 16]).write("mat4", &mut out, 60);
        assert_eq!(out[60..64], [0; 4], "a value past the block is dropped");
    }

    #[test]
    fn noon_is_straight_up_and_the_sun_sets_in_the_west() {
        assert!(body(0.0).abs_diff_eq(Vec3::new(0.0, 100.0, 0.0), 1e-3));
        assert!(body(90.0).abs_diff_eq(Vec3::new(-100.0, 0.0, 0.0), 1e-3));
    }
}
