//! What a pack's WGSL reads: the frame uniform every stage sees, which the
//! renderer fills and uploads as the Rust struct below, and the stage input
//! and output structs. The WGSL of each is generated from these definitions
//! (`Struct::wgsl`), and so is the reference (docs/api/shaders-reference.md).

/// One field of a WGSL struct, for the prelude and the reference.
#[derive(Debug)]
pub struct Field {
    pub name: &'static str,
    pub ty: &'static str,
    /// Its meaning, unit and range.
    pub doc: &'static str,
}

/// A struct a pack's WGSL can name.
#[derive(Debug)]
pub struct Struct {
    pub name: &'static str,
    pub doc: &'static str,
    pub fields: &'static [Field],
}

impl Struct {
    pub fn wgsl(&self) -> String {
        let mut out = format!("struct {} {{\n", self.name);
        for f in self.fields {
            out += &format!("    {}: {},\n", f.name, f.ty);
        }
        out + "}\n"
    }
}

/// The WGSL type of a Rust field the renderer uploads.
pub trait Wgsl {
    const TYPE: &'static str;
}

macro_rules! wgsl_types {
    ($($rust:ty => $wgsl:literal),* $(,)?) => {
        $(impl Wgsl for $rust {
            const TYPE: &'static str = $wgsl;
        })*
    };
}

wgsl_types! {
    f32 => "f32",
    u32 => "u32",
    i32 => "i32",
    [f32; 2] => "vec2<f32>",
    [f32; 3] => "vec3<f32>",
    [f32; 4] => "vec4<f32>",
    [[f32; 4]; 4] => "mat4x4<f32>",
}

/// A uniform struct: the Rust type the renderer uploads, and its WGSL.
macro_rules! uniform {
    ($(#[doc = $doc:literal])* $name:ident {
        $($(#[doc = $fdoc:literal])* $field:ident: $ty:ty,)*
    }) => {
        $(#[doc = $doc])*
        #[repr(C)]
        #[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
        pub struct $name {
            $($(#[doc = $fdoc])* pub $field: $ty,)*
        }

        impl $name {
            pub const WGSL: Struct = Struct {
                name: stringify!($name),
                doc: concat!($($doc, "\n"),*),
                fields: &[$(Field {
                    name: stringify!($field),
                    ty: <$ty as Wgsl>::TYPE,
                    doc: concat!($($fdoc),*),
                }),*],
            };
            /// Each field's byte offset, in `WGSL.fields`' order.
            pub const OFFSETS: &'static [usize] = &[$(std::mem::offset_of!($name, $field)),*];
        }
    };
}

uniform! {
    /// The frame, as `frame` in every stage. Positions and directions are
    /// in world axes relative to the camera (x east, y up, z south), so they
    /// stay precise far from the world's origin; depth is reversed (1 near,
    /// 0 at the far plane and in the sky).
    Frame {
        /// Camera-relative world to view space: the camera's rotation.
        view: [[f32; 4]; 4],
        /// View to clip space (reversed depth, wgpu's clip space: z 0..1).
        projection: [[f32; 4]; 4],
        /// View to camera-relative world space.
        view_inverse: [[f32; 4]; 4],
        /// Clip to view space.
        projection_inverse: [[f32; 4]; 4],
        /// The camera's world position, in blocks (about a millimetre of precision 10 000 blocks out).
        camera_position: [f32; 3],
        /// Seconds since the pack was loaded, wrapping to 0 every hour.
        time: f32,
        /// Unit vector toward the sun (below the horizon at night).
        sun_direction: [f32; 3],
        /// The time of day, 0..1: 0 sunrise, 0.25 noon, 0.5 sunset, 0.75 midnight.
        day_time: f32,
        /// Unit vector toward the moon.
        moon_direction: [f32; 3],
        /// Vanilla's GameTime: (game time % 24000 + partial tick) / 24000, 0..1.
        game_time: f32,
        /// Vanilla's sky colour this frame, 0..1.
        sky_color: [f32; 3],
        /// Rain strength, 0..1.
        rain: f32,
        /// Vanilla's fog colour this frame (also the clear colour), 0..1.
        fog_color: [f32; 3],
        /// Thunder strength, 0..1.
        thunder: f32,
        /// The size of the world's picture, in pixels (the resolution scale applied).
        resolution: [f32; 2],
        /// Block and sky light at the camera, 0..1 (level / 15).
        eye_light: [f32; 2],
        /// Where vanilla's fog starts, in blocks from the camera.
        fog_start: f32,
        /// Where vanilla's fog is full, in blocks.
        fog_end: f32,
        /// The render distance, in blocks.
        render_distance: f32,
        /// The far plane, in blocks.
        far: f32,
        /// Night vision strength, 0..1.
        night_vision: f32,
        /// Blindness strength, 0..1.
        blindness: f32,
        /// The darkness effect's strength, 0..1.
        darkness: f32,
        /// The Brightness option, 0..1 (Moody to Bright).
        brightness: f32,
        /// What the camera is in: 0 air, 1 water, 2 lava, 3 powder snow.
        eye_in: u32,
        /// 0 the Overworld, -1 the Nether, 1 the End, 2 any other dimension.
        dimension: i32,
        /// Frames drawn since the pack was loaded (wraps).
        frame_index: u32,
        /// 1 while F1 hides the HUD, else 0.
        hide_gui: u32,
    }
}

macro_rules! fields {
    ($($name:ident: $ty:literal = $doc:literal,)*) => {
        &[$(Field { name: stringify!($name), ty: $ty, doc: $doc }),*]
    };
}

/// The post stage's input: one pixel of the picture.
pub const POST_INPUT: Struct = Struct {
    name: "PostInput",
    doc: "One pixel of the picture, for `fn post(input: PostInput) -> vec4<f32>`.",
    fields: fields! {
        uv: "vec2<f32>" = "Where the pixel is, 0..1 from the top left.",
        pixel: "vec2<f32>" = "The pixel's centre, in pixels from the top left of what the pass draws (the world's size; the window's for the last pass).",
        color: "vec4<f32>" = "The previous pass's colour here (the world's picture for the first pass).",
        depth: "f32" = "The world's depth here, reversed: 0 is the sky.",
    },
};

/// The material stage's input: one point of a block's surface.
pub const MATERIAL_INPUT: Struct = Struct {
    name: "MaterialInput",
    doc: "One point of a block's surface, for `fn material(input: MaterialInput) -> Surface`.",
    fields: fields! {
        albedo: "vec4<f32>" = "The block texture's colour here (alpha 0..1).",
        tint: "vec4<f32>" = "The vertex colour: biome tint, face shade and ambient occlusion.",
        light: "vec2<f32>" = "Block and sky light, 0..1, smoothed over the face.",
        lightmap: "vec3<f32>" = "Vanilla's light colour for that light this frame.",
        position: "vec3<f32>" = "The point, camera-relative, in blocks.",
        uv: "vec2<f32>" = "The texture coordinate inside the block atlas.",
        translucent: "bool" = "Drawn in the translucent pass (water, stained glass, ice).",
    },
};

/// What the material stage returns.
pub const SURFACE: Struct = Struct {
    name: "Surface",
    doc: "What `material` returns. Vanilla's own is `albedo * tint * vec4(lightmap, 1.0)`; \
          Horizon then applies the alpha test (cutout and translucent) and fog.",
    fields: fields! {
        color: "vec4<f32>" = "The lit colour, 0..1, with its alpha.",
    },
};

/// The sky stage's input: one direction of the sky.
pub const SKY_INPUT: Struct = Struct {
    name: "SkyInput",
    doc: "One direction of the sky, for `fn sky(input: SkyInput) -> vec4<f32>`. The sun, moon, \
          stars and clouds are drawn over the result.",
    fields: fields! {
        direction: "vec3<f32>" = "Unit view ray, camera-relative world axes.",
        color: "vec4<f32>" = "Vanilla's sky colour along the ray (sky, fog and sunrise).",
    },
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_fields_carry_their_docs() {
        let wgsl = Frame::WGSL;
        assert_eq!(wgsl.fields.len(), 28);
        assert!(wgsl.fields.iter().all(|f| !f.doc.trim().is_empty()));
        assert_eq!(std::mem::size_of::<Frame>(), 4 * 64 + 9 * 16);
        assert!(
            wgsl.wgsl()
                .contains("    camera_position: vec3<f32>,\n    time: f32,\n")
        );
    }
}
