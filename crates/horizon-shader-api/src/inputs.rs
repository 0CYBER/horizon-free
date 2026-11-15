//! What a pack's WGSL reads: the frame uniform every stage sees, which the
//! renderer fills and uploads as the Rust struct below, and the stage input
//! and output structs. The WGSL of each is generated from these definitions
//! (`Struct::wgsl`), and so is the reference (docs/api/shaders-reference.md).

/// One field of a WGSL struct, for the prelude and the reference.
#[derive(Clone, Copy, Debug)]
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
    [[f32; 4]; 8] => "array<vec4<f32>, 8>",
}

/// A uniform struct: the Rust type the renderer uploads, and its WGSL. `@pad` after a field
/// names a field in Rust only, which fills what WGSL's layout leaves empty there (before a
/// `vec3`, aligned to 16 bytes, and up to the struct's size, a multiple of 16), since `Pod`
/// allows no implicit padding.
macro_rules! uniform {
    ($(#[doc = $doc:literal])* $name:ident {
        $($(#[doc = $fdoc:literal])* $field:ident: $ty:ty, $(@pad $pad:ident: $pad_ty:ty,)?)*
    }) => {
        $(#[doc = $doc])*
        #[repr(C)]
        #[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
        pub struct $name {
            $($(#[doc = $fdoc])* pub $field: $ty,
            $(/// Not in the WGSL: zeros where its layout leaves a gap.
            pub $pad: $pad_ty,)?)*
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
        /// What falls in the biome at the camera while it rains: 0 nothing, 1 rain, 2 snow.
        precipitation: u32,
        /// The biome at the camera's temperature (vanilla's, -0.7..2.0; it snows below 0.15).
        temperature: f32,
        /// The biome at the camera's downfall, 0..1.
        downfall: f32,
        /// `rain`, smoothed: rises over about 30 seconds and dries over about 10, 0..1.
        wetness: f32,
        /// Last frame's camera-relative world to clip space (its `projection * view`); a point
        /// at camera-relative `p` now was at `p + camera_motion` relative to last frame's camera.
        /// This frame's on the first frame and after a gap of over half a second (a pause, a
        /// world load), when last frame's camera says nothing about this one.
        previous_view_projection: [[f32; 4]; 4],
        /// How far the camera moved since last frame, in blocks (0 after such a gap).
        camera_motion: [f32; 3],
        /// Seconds since last frame (0 on the first and after such a gap).
        delta_time: f32,
        /// `eye_light` eased with a half-life of 2 seconds: it follows the light levels' steps
        /// without stepping itself (for exposure and indoor fog).
        eye_light_smooth: [f32; 2],
        /// 1 while the camera is in water (`eye_in` 1), eased with a half-life of a quarter
        /// second in and out.
        in_water: f32,
        /// The same for lava (`eye_in` 2).
        in_lava: f32,
        /// The y of the horizontal water plane `reflection` mirrors the world about, in world
        /// blocks, while `has_reflection` is 1.
        water_plane: f32,
        /// 1 while the material stage's `reflection` holds this frame's water mirror, else 0.
        has_reflection: u32,
        /// The size of `reflection`, in pixels: half the world's picture.
        reflection_resolution: [f32; 2],
        /// Eight values a native mod hands its own pack each frame (`native_shader_params`,
        /// docs/native-mods.md): what they mean is that mod's, such as where and when
        /// something happened in the world. All 0 when no mod sets them.
        mods: [[f32; 4]; 8],
        /// Unit vector toward the body casting this frame's shadows: the sun by day, the moon
        /// by night.
        light_direction: [f32; 3],
        /// 1 while `shadow_map` holds this frame's shadow map, else 0.
        has_shadow: u32,
        /// Camera-relative world to the shadow camera's clip space, before the distortion
        /// (orthographic, looking along `-light_direction`; z 0 at the light, 1 away from it).
        shadow_view_proj: [[f32; 4]; 4],
        /// Half the side of the square the shadow map covers, in blocks (the manifest's
        /// `[shadow] distance`); 0 without a shadow map.
        shadow_distance: f32,
        /// How strongly the shadow map is distorted toward its centre, 0..1: a point at
        /// shadow clip `xy` is stored at `xy / (length(xy) * k + 1 - k)`, so texels near the
        /// camera cover less ground (`shadow_position` applies it).
        shadow_distortion: f32,
        /// The shadow map's side, in texels; 0 without a shadow map.
        shadow_resolution: f32,
        /// The ground one shadow texel covers at the map's centre, in blocks (for a bias that
        /// scales with it); 0 without a shadow map.
        shadow_center_texel: f32,
        /// 1 while the output is HDR, else 0.
        hdr: u32,
        /// The peak over paper white: the brightest value the display shows (Apple's EDR
        /// headroom, Android's HDR/SDR ratio). 1 in SDR. It changes while the game runs (the
        /// system's SDR white, a calibration, another display), so read it every frame.
        headroom: f32,
        /// Paper white in nits; a nominal 80 in SDR.
        paper_white: f32,
        /// The peak brightness in nits; a nominal 80 in SDR, as `paper_white`.
        peak: f32,
        /// The display's black in nits (HDR's Black Level); 0 in SDR.
        black_level: f32,
        /// The primaries of this pack's colours (`[frame] primaries`): 0 BT.709, 1 BT.2020.
        primaries: u32,
        /// 1 while `trace` and `trace_any` read this frame's scene, else 0 (a GPU without ray
        /// queries, ray tracing off, no world): every ray then misses. Only a pack with
        /// `[ray_tracing]` has this field.
        has_ray_tracing: u32,
        /// How far from the camera along x and along z, in blocks, the scene is sure to reach;
        /// 0 while `has_ray_tracing` is 0. Only a pack with `[ray_tracing]` has this field.
        ray_distance: f32,
        /// The weights that give a colour's luminance in those primaries, `dot(c, frame.luminance)`:
        /// (0.2126, 0.7152, 0.0722) for BT.709, (0.2627, 0.6780, 0.0593) for BT.2020.
        luminance: [f32; 3],
        @pad padding: u32,
    }
}

impl Frame {
    /// The fields 0.1's prelude declares: the struct up to `shadow_center_texel`, a prefix of
    /// the buffer the renderer uploads.
    pub const FIELDS_0_1: usize = 49;

    /// Where `has_ray_tracing` and `ray_distance` are among the fields: in what was a gap of
    /// the layout before `luminance`, so no other field moves with or without them.
    pub const RAY_FIELDS: std::ops::Range<usize> = 55..57;

    /// The fields without `RAY_FIELDS`: what a 0.2 pack without `[ray_tracing]` declares.
    const UNTRACED: [Field; 56] = {
        let all = Self::WGSL.fields;
        let mut out = [all[0]; 56];
        let (mut from, mut to) = (0, 0);
        while from < all.len() {
            if from < Self::RAY_FIELDS.start || from >= Self::RAY_FIELDS.end {
                out[to] = all[from];
                to += 1;
            }
            from += 1;
        }
        out
    };

    /// `Frame` as the line's prelude declares it for a pack without `[ray_tracing]`.
    pub fn line(api: crate::Api) -> Struct {
        Self::declared(api, false)
    }

    /// `Frame` as the line's prelude declares it, for a pack that asks for the scene
    /// (`[ray_tracing]`, 0.2) or not.
    pub fn declared(api: crate::Api, ray_tracing: bool) -> Struct {
        let fields: &'static [Field] = match (api, ray_tracing) {
            (crate::Api::V0_1, _) => &Self::WGSL.fields[..Self::FIELDS_0_1],
            (crate::Api::V0_2, false) => &Self::UNTRACED,
            (crate::Api::V0_2, true) => Self::WGSL.fields,
        };
        Struct {
            fields,
            ..Self::WGSL
        }
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

/// A compute pass's input (0.2): one invocation of its dispatch.
pub const COMPUTE_INPUT: Struct = Struct {
    name: "ComputeInput",
    doc: "One invocation of a dispatch, for `fn compute(input: ComputeInput)`.",
    fields: fields! {
        cell: "vec3<u32>" = "The invocation's cell: where it is in the whole dispatch.",
        local: "u32" = "Its index in its workgroup, from 0 to the workgroup's invocations less 1.",
        size: "vec3<u32>" = "The dispatch's cells: its `dispatch`, or the render size and 1 for `\"pixels\"`.",
        in_bounds: "bool" = "Whether `cell` is within `size`. Every invocation of every workgroup runs: one beyond `size` touches no resource, but still reaches every `workgroupBarrier()`.",
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
        tags: "u32" = "The pack's sprite tags this surface's sprite has: bit i for the manifest's i-th `[[tags]]`.",
        material: "u32" = "The `[[materials]]` id of this surface's sprite (0 when no entry names it).",
        screen: "vec2<f32>" = "Where the point is on the screen, 0..1 from the top left (`opaque_depth` and `opaque_color` at `screen * frame.resolution`).",
        face: "u32" = "For a fluid, which face of the block the point is on, `Direction` order: 0 down, 1 up, 2 north, 3 south, 4 west, 5 east. 0 elsewhere.",
        fluid: "u32" = "0 not a fluid, 1 water, 2 lava.",
        emission: "f32" = "How strongly the block itself shines, 0..1, by its kind of light: a redstone torch or a candle 7/15, a torch or lava 14/15, glowstone 1; 0 for most blocks.",
        normal: "vec3<f32>" = "The surface's geometric normal, a unit vector in world axes, facing the camera. A normal within about 3.6 degrees of an axis is that axis exactly, so distant block faces do not shimmer; flowing water's slopes are steeper and keep theirs.",
        tangent: "vec4<f32>" = "The surface's tangent: xyz the unit direction in world axes along which `uv.x` grows, w the sign (1 or -1) that makes `cross(normal, tangent.xyz) * tangent.w` the direction `uv.y` grows along (down the texture). With `normal` it turns a normal map's normals into world axes.",
        colored_light: "vec3<f32>" = "The frame's coloured lights at this point (`[[particle_lights]]`, held and dropped items in their `[[lights]]` colour while dynamic lights are on, and mods' visual lights), 0..1, to light the surface with as the pack likes; 0 where none reaches.",
        displaced: "f32" = "How much of the relief `[displacement]` gave this point as geometry, 0..1: 1 near the camera, where the surface already lies where `displacement` put it (`position` is that point; a parallax effect would count its depth twice), easing to 0 at `[displacement] distance`; always 0 without `[displacement]`.",
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

/// The entity stage's input (0.2): one point of a surface that is no block terrain.
pub const ENTITY_INPUT: Struct = Struct {
    name: "EntityInput",
    doc: "One point of the surface of an entity, a block entity, an item or a particle, for `fn entity(input: EntityInput) -> Surface`.",
    fields: fields! {
        albedo: "vec4<f32>" = "The texture's colour here.",
        tint: "vec4<f32>" = "The vertex colour without vanilla's directional shade (dyes, particle colours, fades).",
        overlay: "vec4<f32>" = "The flash vanilla lays over the surface (hurt, TNT, a creeper's swelling): rgb its colour, alpha how much of the surface it replaces. Alpha 0 without one.",
        lightmap: "vec3<f32>" = "Vanilla's light colour for `light` this frame.",
        shade: "f32" = "Vanilla's directional shade for this face as a factor on linear light, 0..1 (vanilla's value decoded with the 2.2 power, as every 0.2 colour is). 1 where vanilla applies none.",
        position: "vec3<f32>" = "The point, camera-relative, in blocks.",
        emission: "f32" = "1 on a layer vanilla draws at full brightness (spider and enderman eyes, the warden's glow), else 0. There `lightmap` is full light; `shade` stays vanilla's, which shades the warden's glow and not the eyes.",
        normal: "vec3<f32>" = "The surface's geometric normal, a unit vector in world axes, facing the camera, from the position's screen derivatives (as `MaterialInput.normal`, without its snap to the axes).",
        kind: "u32" = "What is drawn: 0 an entity (mobs, players, armour, vehicles), 1 a block entity (chests, signs, banners, beds), 2 a dropped or framed item, 3 reserved for the first-person hand, 4 a particle.",
        colored_light: "vec3<f32>" = "As `MaterialInput.colored_light`: the frame's coloured lights at this point, 0..1; 0 where none reaches.",
        translucent: "bool" = "Drawn with blending.",
        light: "vec2<f32>" = "Block and sky light, 0..1.",
        uv: "vec2<f32>" = "The texture coordinate.",
        screen: "vec2<f32>" = "Where the point is on the screen, 0..1 from the top left (as `MaterialInput.screen`).",
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

/// The input of the material stage's `displacement`, under `[displacement]`.
pub const DISPLACEMENT_INPUT: Struct = Struct {
    name: "DisplacementInput",
    doc: "One corner of the grid a block face near the camera is cut into, for `fn displacement(input: DisplacementInput) -> f32` (`[displacement]`), which returns how deep below the face, in blocks, the surface lies at `height`.",
    fields: fields! {
        position: "vec3<f32>" = "The corner on the flat face, camera-relative, in blocks.",
        normal: "vec3<f32>" = "The face's normal, a unit vector in world axes.",
        uv: "vec2<f32>" = "The texture coordinate inside the block's sprite (as `MaterialInput.uv`).",
        material: "u32" = "The `[[materials]]` id of the block texture (0 when none matches).",
        height: "f32" = "A LabPBR height, 0..1 (1 the top of the relief): the normal map's alpha here, or the texture's average height (what the face's edges are pinned to).",
    },
};

/// The vertex stage's input: one corner of a block's surface.
pub const VERTEX_INPUT: Struct = Struct {
    name: "VertexInput",
    doc: "One corner of a block's surface, for `fn vertex(input: VertexInput) -> vec3<f32>`, which returns where it is drawn (camera-relative, in blocks; `input.position` where it stays).",
    fields: fields! {
        position: "vec3<f32>" = "The corner, camera-relative, in blocks (`+ frame.camera_position` for the world's).",
        uv: "vec2<f32>" = "The texture coordinate inside the block's sprite.",
        material: "u32" = "The `[[materials]]` id of the block's sprite (0 when none matches).",
        light: "vec2<f32>" = "Block and sky light, 0..1.",
        tint: "vec4<f32>" = "The vertex colour (biome tint, face shade, ambient occlusion); alpha 1.",
        face: "u32" = "For a fluid, the face of the block the corner is on (as `MaterialInput.face`), else 0.",
        fluid: "u32" = "1 on a fluid's surface (water or lava), else 0.",
    },
};

/// The weather stage's input: one point of a rain or snow streak.
pub const WEATHER_INPUT: Struct = Struct {
    name: "WeatherInput",
    doc: "One point of a rain or snow streak, for `fn weather(input: WeatherInput) -> vec4<f32>`, \
          which returns the colour blended over the world (alpha its opacity).",
    fields: fields! {
        color: "vec4<f32>" = "The rain or snow texture's colour here times the streak's vertex colour (alpha 0.1..1).",
        light: "vec2<f32>" = "Block and sky light, 0..1.",
        position: "vec3<f32>" = "The point, camera-relative, in blocks.",
        snow: "bool" = "A snow streak (rain otherwise).",
    },
};

/// What `trace` and `trace_layers` return (0.2, `[ray_tracing]`): the nearest surface along a ray.
pub const RAY_HIT: Struct = Struct {
    name: "RayHit",
    doc: "The nearest surface along a ray, from `trace` or `trace_layers`. Its `albedo`, `tint`, `light`, `uv`, `material`, `tags`, `emission`, `fluid`, `translucent` and `tangent` are what `MaterialInput` gives the material stage for the same point, so one function can shade both. On a miss `distance` is negative and every other field is 0 (`translucent` false).",
    fields: fields! {
        distance: "f32" = "How far along the ray the surface is, in blocks; negative on a miss.",
        position: "vec3<f32>" = "The point, camera-relative, in blocks (as `view_position`).",
        normal: "vec3<f32>" = "The face's normal, a unit vector in world axes, turned toward the ray's origin.",
        uv: "vec2<f32>" = "The texture coordinate inside the block's sprite (as `MaterialInput.uv`).",
        albedo: "vec4<f32>" = "The block texture's colour at the point, at full size (mip level 0), linear light; alpha the texture's.",
        tint: "vec4<f32>" = "The vertex colour: biome tint, face shade and ambient occlusion, linear light; alpha 1.",
        light: "vec2<f32>" = "Block and sky light, 0..1, smoothed over the face.",
        material: "u32" = "The `[[materials]]` id of the surface's sprite (0 when no entry names it).",
        tags: "u32" = "The pack's sprite tags the surface's sprite has: bit i for the manifest's i-th `[[tags]]`.",
        emission: "f32" = "How strongly the block itself shines, 0..1, by its kind of light: a redstone torch or a candle 7/15, a torch or lava 14/15, glowstone 1; 0 for most blocks (as `MaterialInput.emission`).",
        fluid: "u32" = "0 not a fluid, 1 water, 2 lava (as `MaterialInput.fluid`). Water is in the `RAY_TRANSLUCENT` layer, lava in `RAY_SOLID`.",
        translucent: "bool" = "A surface of the translucent pass (water, stained glass, ice), as `MaterialInput.translucent`: a hit in the `RAY_TRANSLUCENT` layer, so false for every hit of `trace`.",
        tangent: "vec4<f32>" = "The surface's tangent, as `MaterialInput.tangent`: xyz the unit direction in world axes along which `uv.x` grows, w the sign (1 or -1) that makes `cross(normal, tangent.xyz) * tangent.w` the direction `uv.y` grows along (down the texture). It follows `normal`, which faces the ray's origin. With `normal` it turns `hit_normal_map`'s normals into world axes.",
        horizon_sprite: "u32" = "Horizon's own: where the hit's sprite is kept, for `hit_normal_map` and `hit_specular_map`. Not for packs to read; it changes with the renderer.",
    },
};

/// The tonemap stage's input (0.2): one pixel of the window.
pub const TONEMAP_INPUT: Struct = Struct {
    name: "TonemapInput",
    doc: "One pixel of an output (the window), for `fn tonemap(input: TonemapInput) -> vec3<f32>`, which returns its colour for the output `frame` describes: linear light, 1.0 at paper white, from 0 to `frame.headroom` (Horizon clamps it there).",
    fields: fields! {
        uv: "vec2<f32>" = "Where the pixel is, 0..1 from the top left.",
        pixel: "vec2<f32>" = "The pixel's centre, in pixels from the top left of the output (the window).",
        color: "vec3<f32>" = "The post chain's picture here (the world's without one), linear light, never negative or NaN.",
    },
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_fields_carry_their_docs() {
        let wgsl = Frame::WGSL;
        assert_eq!(wgsl.fields.len(), 58);
        assert!(wgsl.fields.iter().all(|f| !f.doc.trim().is_empty()));
        // 0.1's 49 fields, then 0.2's nine: `luminance` is a `vec3`, on the next 16 bytes.
        assert_eq!(
            std::mem::size_of::<Frame>(),
            6 * 64 + 15 * 16 + 8 * 16 + 3 * 16
        );
        assert_eq!(Frame::OFFSETS[57], std::mem::size_of::<Frame>() - 16);
        // The scene's two fields are only a `[ray_tracing]` pack's.
        let names = |s: Struct| s.fields.iter().map(|f| f.name).collect::<Vec<_>>();
        let traced = names(Frame::declared(crate::Api::V0_2, true));
        assert_eq!(
            traced[Frame::RAY_FIELDS],
            ["has_ray_tracing", "ray_distance"]
        );
        let untraced = names(Frame::line(crate::Api::V0_2));
        assert_eq!(untraced.len(), 56);
        assert!(!untraced.iter().any(|n| n.contains("ray")));
        assert_eq!(Frame::OFFSETS[Frame::FIELDS_0_1], 6 * 64 + 15 * 16 + 8 * 16);
        assert_eq!(
            Frame::line(crate::Api::V0_1).fields.len(),
            Frame::FIELDS_0_1
        );
        assert!(
            wgsl.wgsl()
                .contains("    camera_position: vec3<f32>,\n    time: f32,\n")
        );
    }
}
