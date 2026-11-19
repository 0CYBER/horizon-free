//! What a pack's WGSL reads: the frame uniform every stage sees, which the
//! renderer fills and uploads as the Rust struct below, and the stage input
//! and output structs. The WGSL of each is generated from these definitions
//! (`Struct::wgsl`), and so is the reference (docs/api/shaders-reference-0.2.md).

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
        @pad fog_gap: [f32; 2],
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
        /// docs/modding/src/how-to/native-mods.md): what they mean is that mod's, such as where and when
        /// something happened in the world. All 0 when no mod sets them.
        mods: [[f32; 4]; 8],
        /// Unit vector toward the body casting this frame's shadows: the sun by day, the moon
        /// by night, as `sun_direction` and `moon_direction` have them, also without a
        /// `[shadow]` stage.
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
        /// Changes when the pack's `keep = "view"` resources are cleared, to zeros (`reset_reasons`
        /// says why), and is never 0. Only a pack with `[[resource]]` has this field.
        view_epoch: u32,
        /// The same for its `keep = "world"` resources. `view_epoch` changes with it: what
        /// clears the world's clears the view's too. Only a pack with `[[resource]]` has this field.
        world_epoch: u32,
        /// Why an epoch changed this frame: the prelude's `REASON_*` bits ("Kept resources"); 0
        /// when neither did. Only a pack with `[[resource]]` has this field.
        reset_reasons: u32,
        /// Where the fog of where the camera is starts and where it is full (x, y), in blocks
        /// along the straight line from the camera: the dimension's and the biome's, thickened by
        /// rain, or that of water, lava, powder snow, blindness, darkness or a boss.
        fog_environment: [f32; 2],
        /// Where the render distance's fog starts and where it is full (x, y), in blocks along the
        /// larger of the horizontal distance and the height difference from the camera. Vanilla's
        /// fog is the larger of the two (`vanilla_fog`, under `[frame] fog = "pack"`).
        fog_render: [f32; 2],
        /// The moon's phase, as Horizon draws the moon: 0 full, 1 waning gibbous, 2 third
        /// quarter, 3 waning crescent, 4 new, 5 waxing crescent, 6 first quarter, 7 waxing
        /// gibbous. It moves on by one each day. Only a pack whose files name `moon_phase` has
        /// this field.
        moon_phase: u32,
        @pad padding: [u32; 1],
        /// Vanilla's light colour at full block and sky light (15 and 15) this frame, linear: what
        /// `MaterialInput.lightmap` and `EntityInput.lightmap` are where light is full. Its
        /// brightest channel is 1 unless something dims all light (the Darkness effect, a
        /// wither's bar). Only a pack whose files name `full_lightmap` has this field.
        full_lightmap: [f32; 3],
        @pad full_lightmap_padding: [u32; 1],
    }
}

impl Frame {
    /// Where `has_ray_tracing` and `ray_distance` are among the fields: in what was a gap of
    /// the layout before `luminance`, so no other field moves with or without them.
    pub const RAY_FIELDS: std::ops::Range<usize> = 53..55;

    /// Where `view_epoch`, `world_epoch` and `reset_reasons` are among the fields: after
    /// `luminance`, so a pack without `[[resource]]` declares a prefix and no other field moves.
    pub const KEPT_FIELDS: std::ops::Range<usize> = 56..59;

    /// Where `moon_phase` is among the fields: the last, in what was padding at the end, so the
    /// uploaded struct keeps its size and no field moves.
    pub const MOON_FIELDS: std::ops::Range<usize> = 61..62;

    /// Where `full_lightmap` is among the fields: the last, on 16 bytes of its own after the
    /// struct it had before, so no other field moves.
    pub const FULL_LIGHTMAP_FIELDS: std::ops::Range<usize> = 62..63;

    /// The fields the prelude declares, by index, for a pack that asks for the scene
    /// (`[ray_tracing]`) or not, keeps resources (`[[resource]]`) or not, and whose files name
    /// `moon_phase` or not, and `full_lightmap` or not.
    fn declared_at(
        ray_tracing: bool,
        kept: bool,
        moon_phase: bool,
        full_lightmap: bool,
    ) -> Vec<usize> {
        let skipped = |i: usize| {
            (!ray_tracing && Self::RAY_FIELDS.contains(&i))
                || (!kept && Self::KEPT_FIELDS.contains(&i))
                || (!moon_phase && Self::MOON_FIELDS.contains(&i))
                || (!full_lightmap && Self::FULL_LIGHTMAP_FIELDS.contains(&i))
        };
        (0..Self::WGSL.fields.len())
            .filter(|&i| !skipped(i))
            .collect()
    }

    /// The fields the prelude declares (`declared_at`).
    pub fn declared(
        ray_tracing: bool,
        kept: bool,
        moon_phase: bool,
        full_lightmap: bool,
    ) -> Vec<Field> {
        let at = Self::declared_at(ray_tracing, kept, moon_phase, full_lightmap).into_iter();
        at.map(|i| Self::WGSL.fields[i]).collect()
    }

    /// `declared` as WGSL, each field at the offset the uploaded struct has it: a field followed
    /// by bytes WGSL's layout wouldn't leave (a removed field's, or the epochs' without
    /// `[[resource]]`) keeps them (`@size`). Without `full_lightmap` the struct ends where it did
    /// before it, 16 bytes short of the uploaded one.
    pub fn declared_wgsl(
        ray_tracing: bool,
        kept: bool,
        moon_phase: bool,
        full_lightmap: bool,
    ) -> String {
        let mut out = format!("struct {} {{\n", Self::WGSL.name);
        let at = Self::declared_at(ray_tracing, kept, moon_phase, full_lightmap);
        for (n, &i) in at.iter().enumerate() {
            let f = Self::WGSL.fields[i];
            out += "    ";
            if let Some(&next) = at.get(n + 1) {
                let (size, _) = layout(f.ty);
                let (_, align) = layout(Self::WGSL.fields[next].ty);
                let placed = (Self::OFFSETS[i] + size).next_multiple_of(align);
                if placed != Self::OFFSETS[next] {
                    out += &format!("@size({}) ", Self::OFFSETS[next] - Self::OFFSETS[i]);
                }
            }
            out += &format!("{}: {},\n", f.name, f.ty);
        }
        out + "}\n"
    }
}

/// The size and alignment WGSL's uniform layout gives a `Frame` field's type.
fn layout(ty: &str) -> (usize, usize) {
    match ty {
        "f32" | "u32" | "i32" => (4, 4),
        "vec2<f32>" => (8, 8),
        "vec3<f32>" => (12, 16),
        "vec4<f32>" => (16, 16),
        "mat4x4<f32>" => (64, 16),
        "array<vec4<f32>, 8>" => (128, 16),
        _ => panic!("no layout for {ty}"),
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

/// A compute pass's input: one invocation of its dispatch.
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
        tint: "vec4<f32>" = "The colour vanilla multiplies the block's texture by: the biome's tint (grass, leaves, water), a model's own colour, else white; alpha 1. Without the face's shade and ambient occlusion (`shade`, `ao`).",
        shade: "f32" = "Vanilla's directional shade for this face as a factor on linear light, 0..1 (vanilla's value decoded with the 2.2 power, as `EntityInput.shade`): the shade the face was given, 1 on top, 0.8 north and south, 0.6 east and west, 0.5 below, 0.9 above and below in the Nether. A face with `shade_direction_override` is shaded as that face (a torch is 1 all round, 0.9 in the Nether), one vanilla doesn't shade (a model element with `\"shade\": false`) is 1, and a fluid's side has the side's shade times the top's.",
        ao: "f32" = "Vanilla's ambient occlusion here as a factor on linear light, 0..1 (decoded with the 2.2 power), smoothed over the face: how much the blocks around each corner darken it. 1 where vanilla applies none: Smooth Lighting off, a block that gives off light, a model without ambient occlusion, fluids, distant terrain.",
        light: "vec2<f32>" = "Block and sky light, 0..1, smoothed over the face: vanilla's smooth light, without ambient occlusion (`ao`), and Horizon's dynamic block light.",
        lightmap: "vec3<f32>" = "Vanilla's light colour for that light this frame.",
        position: "vec3<f32>" = "The point, camera-relative, in blocks.",
        uv: "vec2<f32>" = "The texture coordinate inside the block atlas.",
        translucent: "bool" = "Drawn in the translucent pass (water, stained glass, ice).",
        tags: "u32" = "The pack's sprite tags this surface's sprite has: bit i for the manifest's i-th `[[tags]]`.",
        material: "u32" = "The `[[materials]]` id of this surface's sprite (0 when no entry names it).",
        screen: "vec2<f32>" = "Where the point is on the screen, 0..1 from the top left (`opaque_depth` and `opaque_color` at `screen * frame.resolution`).",
        face: "u32" = "Which way the surface faces, `Direction` order: 0 down, 1 up, 2 north, 3 south, 4 west, 5 east (a model face's own direction; a fluid's face of its block).",
        fluid: "u32" = "0 not a fluid, 1 water, 2 lava.",
        emission: "f32" = "How much light the surface gives off, 0..1: its block's light level over 15 (a torch 14/15; a lantern, glowstone, a froglight or lava 1; a redstone torch or glow lichen 7/15; one lit candle 3/15), 0 for a block that gives none. The same on every face of the block: a model face's own light level (`light_emission`, as the open eyeblossom's petals) doesn't count: it raises that face's block light in `light` (and `lightmap` with it), as vanilla does. A fluid's surface has the fluid's own (lava 1, water 0, also around a waterlogged lantern). It says how much light the block gives, not which of its texels glow: the resource packs' LabPBR maps say that where they ship (`block_specular_map(uv).a`).",
        normal: "vec3<f32>" = "The surface's geometric normal, a unit vector in world axes, facing the camera. A normal within about 3.6 degrees of an axis is that axis exactly, so distant block faces do not shimmer; flowing water's slopes are steeper and keep theirs.",
        tangent: "vec4<f32>" = "The surface's tangent: xyz the unit direction in world axes along which `uv.x` grows, w the sign (1 or -1) that makes `cross(normal, tangent.xyz) * tangent.w` the direction `uv.y` grows along (down the texture). With `normal` it turns a normal map's normals into world axes.",
        colored_light: "vec3<f32>" = "The frame's coloured lights at this point (`[[particle_lights]]`, held and dropped items in their `[[lights]]` colour while dynamic lights are on, and mods' visual lights), 0..1, to light the surface with as the pack likes; 0 where none reaches.",
        displaced: "f32" = "How much of the relief `[displacement]` gave this point as geometry, 0..1: 1 near the camera, where the surface already lies where `displacement` put it (`position` is that point; a parallax effect would count its depth twice), easing to 0 at `[displacement] distance`; always 0 without `[displacement]`.",
    },
};

/// What the material stage returns.
pub const SURFACE: Struct = Struct {
    name: "Surface",
    doc: "What `material` returns. Vanilla's own is `albedo * vec4(lit_tint, 1.0)`, its \
          vertex colour lit at each corner; `albedo * vec4(shaded_tint.rgb * lightmap, 1.0)` \
          costs nothing and is close, but smooths colour and light apart, so it differs where \
          both change across a face (an occluded corner beside a torch). Horizon then applies the alpha test (cutout and translucent; on the texture's alpha \
          unless `[material] coverage = \"pack\"`) and fog (unless `[frame] fog = \"pack\"`).",
    fields: fields! {
        color: "vec4<f32>" = "The lit colour: rgb linear light, 0 or more and unbounded (more than 1 is brighter than white); alpha translucent terrain's opacity, and what the alpha test reads under `[material] coverage = \"pack\"`.",
    },
};

/// The entity stage's input: one point of a surface that is no block terrain.
pub const ENTITY_INPUT: Struct = Struct {
    name: "EntityInput",
    doc: "One point of the surface of an entity, a block entity, an item or a particle, for `fn entity(input: EntityInput) -> Surface`.",
    fields: fields! {
        albedo: "vec4<f32>" = "The texture's colour here.",
        tint: "vec4<f32>" = "The vertex colour without vanilla's directional shade (dyes, particle colours, fades). A block drawn as an entity (a falling block, a block in a minecart, a block display, a block item) keeps its faces' shade in it, and will: its `shade` is 1.",
        overlay: "vec4<f32>" = "The flash vanilla lays over the surface (hurt, TNT, a creeper's swelling): rgb its colour, alpha how much of the surface it replaces. Alpha 0 without one.",
        lightmap: "vec3<f32>" = "Vanilla's light colour for `light` this frame.",
        shade: "f32" = "Vanilla's directional shade for this face as a factor on linear light, 0..1 (vanilla's value decoded with the 2.2 power, as every 0.2 colour is). 1 where vanilla applies none.",
        position: "vec3<f32>" = "The point, camera-relative, in blocks.",
        full_light: "bool" = "The surface's light is full, block and sky light both 15 as the vertices carry them (before dynamic light): so are the layers vanilla draws at full brightness (spider and enderman eyes, the warden's glow), and any surface standing in full block and sky light (a blaze in daylight). There `lightmap` is full light; `shade` stays vanilla's, which shades the warden's glow and not the eyes. Ender chests and other block entities have no terrain `emission`.",
        normal: "vec3<f32>" = "The surface's geometric normal, a unit vector in world axes, facing the camera, from the position's screen derivatives (as `MaterialInput.normal`, without its snap to the axes).",
        kind: "u32" = "What is drawn: 0 an entity (mobs, players, armour, vehicles), 1 a block entity (chests, banners, bells, shulker boxes), 2 a dropped or framed item, 3 reserved for the first-person hand, 4 a particle.",
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
        tint: "vec4<f32>" = "As `MaterialInput.tint`: the colour vanilla multiplies the texture by (biome tint, a model's colour), without the shade and ambient occlusion; alpha 1.",
        shade: "f32" = "As `MaterialInput.shade`.",
        ao: "f32" = "As `MaterialInput.ao`, at this corner.",
        face: "u32" = "As `MaterialInput.face`.",
        fluid: "u32" = "As `MaterialInput.fluid`: 0 not a fluid, 1 water, 2 lava.",
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

/// What `trace` and `trace_layers` return (`[ray_tracing]`): the nearest surface along a ray.
pub const RAY_HIT: Struct = Struct {
    name: "RayHit",
    doc: "The nearest surface along a ray, from `trace` or `trace_layers`. Its `albedo`, `uv`, `material`, `tags`, `fluid`, `translucent` and `tangent` are what `MaterialInput` gives for the same point. Its `light` excludes dynamic light. On a terrain hit with a look record, `tint`, `shade` and `ao` follow `MaterialInput` in API 0.2. Terrain `emission` comes from the vertex and gives the block's light level over 15. A stage that reads a hit's `tint`, `shade` or `ao` has the scene keep its terrain's looks, eight bytes a face in video memory for the faces within its reach and as many in memory for every loaded section, from each section's next mesh on: when a pack starts reading these fields, loaded sections are queued for remeshing. With no usable record, including while the replacement mesh or GPU allocation is pending, hits use the baked vertex colour for `tint` and 1 for `shade` and `ao`. The scene holds the faces flat: under `[displacement]` a fragment shades from its displaced point, a hit from the face. `shaded_tint` stays the baked vertex colour on terrain and uses `tint.rgb * shade` on entity hits. On a miss, and in every stage while the scene isn't there (`frame.has_ray_tracing` 0), `distance` is negative and every other field is 0 (the `bool`s false).",
    fields: fields! {
        distance: "f32" = "How far along the ray the surface is, in blocks; negative on a miss.",
        position: "vec3<f32>" = "The point, camera-relative, in blocks (as `view_position`).",
        normal: "vec3<f32>" = "The face's normal, a unit vector in world axes, turned toward the ray's origin.",
        uv: "vec2<f32>" = "The texture coordinate inside the block's sprite (as `MaterialInput.uv`).",
        albedo: "vec4<f32>" = "The block texture's colour at the point, at full size (mip level 0), linear light; alpha the texture's.",
        tint: "vec4<f32>" = "The colour vanilla multiplies the texture by, linear light; alpha 1. On a terrain hit with a look record, as `MaterialInput.tint`: without the face's shade and ambient occlusion (`shade`, `ao`). Without a record, the baked vertex colour. An entity hit's is its vertices' (`entity`).",
        shade: "f32" = "The face's directional shade, linear: `MaterialInput.shade` on a terrain hit with a look record, 1 without one, and vanilla's entity shade on an entity hit (`entity`).",
        ao: "f32" = "Ambient occlusion at the point, linear: `MaterialInput.ao` smoothed over a terrain face with a look record, and 1 on other hits.",
        light: "vec2<f32>" = "Block and sky light, 0..1, smoothed over the face, as the world stores it: without the dynamic light (a held torch, a burning entity) that `MaterialInput.light` adds.",
        material: "u32" = "The `[[materials]]` id of the surface's sprite (0 when no entry names it).",
        tags: "u32" = "The pack's sprite tags the surface's sprite has: bit i for the manifest's i-th `[[tags]]`.",
        emission: "f32" = "How much light the surface gives off, as `MaterialInput.emission`: its block's light level over 15 (a lantern or lava 1, one lit candle 3/15), 0 for a block that gives none.",
        fluid: "u32" = "0 not a fluid, 1 water, 2 lava (as `MaterialInput.fluid`). Water is in the `RAY_TRANSLUCENT` layer, lava in `RAY_SOLID`.",
        translucent: "bool" = "A surface of the translucent pass (water, stained glass, ice), as `MaterialInput.translucent`: a hit in the `RAY_TRANSLUCENT` layer, so false for every hit of `trace`.",
        back: "bool" = "The ray met the face from behind, against the way it faces (its normal before `normal` is turned toward the ray's origin): every hit from inside a full block, none from outside. A face drawn on both sides, as a cross-model plant's, is two quads in one place facing opposite ways, met at the same distance; which one a ray takes is the GPU's choice, so `back` there says nothing of the side (on the GPUs tried it takes the same quad from both sides, and one of the two directions reads back).",
        entity: "bool" = "Only in a pack whose files name `RAY_ENTITY`: a hit in that layer, an entity's surface. Its `albedo` is the entity texture's texel and `tint`, `light` and `uv` are its vertices' (the colour vanilla multiplies in, alpha included; the atlas position); `shade` is vanilla's entity shade for the face's normal (as `EntityInput.shade`), `ao` 1; `material`, `tags`, `emission` and `fluid` are 0, `hit_normal_map` and `hit_specular_map` give their defaults.",
        tangent: "vec4<f32>" = "The surface's tangent, as `MaterialInput.tangent`: xyz the unit direction in world axes along which `uv.x` grows, w the sign (1 or -1) that makes `cross(normal, tangent.xyz) * tangent.w` the direction `uv.y` grows along (down the texture). It follows `normal`, which faces the ray's origin. With `normal` it turns `hit_normal_map`'s normals into world axes.",
        horizon_sprite: "u32" = "Horizon's own: where the hit's sprite is kept, for `hit_normal_map` and `hit_specular_map`. Not for packs to read; it changes with the renderer.",
    },
};

/// Where `RayHit.entity` is: a pack that doesn't name `RAY_ENTITY` has `RayHit` without it.
const RAY_HIT_ENTITY: usize = 15;

/// `RAY_HIT` without `entity`.
const RAY_HIT_PLAIN: [Field; 17] = {
    let all = RAY_HIT.fields;
    let mut out = [all[0]; 17];
    let mut i = 0;
    while i < out.len() {
        out[i] = all[i + (i >= RAY_HIT_ENTITY) as usize];
        i += 1;
    }
    out
};

/// `RayHit` as the prelude declares it, for a pack whose files name `RAY_ENTITY` (`entities`)
/// or not.
pub fn ray_hit(entities: bool) -> Struct {
    let fields: &'static [Field] = match entities {
        false => &RAY_HIT_PLAIN,
        true => RAY_HIT.fields,
    };
    Struct { fields, ..RAY_HIT }
}

/// `shaded_tint`, the last field of `MaterialInput`, `VertexInput` and `RayHit` in a pack whose
/// files name it (`stages::names_shaded_tint`): the vertex holds it, so it costs nothing.
pub const SHADED_TINT: Field = Field {
    name: "shaded_tint",
    ty: "vec4<f32>",
    doc: "Only in a pack whose files name `shaded_tint`: the colour vanilla multiplies the texture by with the face's shade and ambient occlusion in it, `tint.rgb * shade * ao` as vanilla's vertex carries it (vanilla multiplies them at each corner in 8-bit integers, so it is within two steps of 255 of that product); alpha 1. Draw vanilla's colour with it; `tint`, `shade` and `ao` are for lighting faces yourself, and reading them from `MaterialInput` or `VertexInput` makes the world load again when the pack starts and terrain take a third again its memory. For the cost of reading those fields on a terrain hit, see `RayHit`. On `RayHit` it is linear light. Terrain hits interpolate the baked vertex colour, entity hits use `tint.rgb * shade` and keep their `tint`'s alpha, and misses return 0.",
};

/// `input` with `SHADED_TINT` last where `shaded`.
pub fn shaded(input: &Struct, shaded: bool) -> String {
    declared(input, false, false, shaded)
}

/// `lit_tint`, in `MaterialInput` after its light-source fields and before `shaded_tint`, in a
/// pack whose files name it (`stages::names_lit_tint`): the renderer hands it down in a varying
/// of its own, the last one the terrain has on the weakest GPUs.
pub const LIT_TINT: Field = Field {
    name: "lit_tint",
    ty: "vec3<f32>",
    doc: "Only in a pack whose files name `lit_tint`: vanilla's lit vertex colour, `shaded_tint.rgb * lightmap` multiplied at each corner and then smoothed over the face, as Horizon's own terrain draws blocks, with the frame's coloured lights added under the vertex colour at each corner (`min(shaded_tint.rgb * (lightmap + colored_light), 1)` there, wherever any reaches). Those products are taken in vanilla's gamma at each corner, on the corner's own values; the smoothed result is then decoded with the 2.2 power, as `shaded_tint` and `lightmap` are, so the formula doesn't hold for the decoded fields a stage gets. So `albedo.rgb * lit_tint` is vanilla's lit colour, where multiplying `shaded_tint` by `lightmap`, each smoothed apart, differs from it where both change across a face (an occluded corner beside a torch). `[displacement]`'s grid has it at each of its points between the face's four corners' products, as it has the vertex colour, each lit where the mesh puts the corner (before `[vertex]` moves it).",
};

/// What a pack whose files name one of their names (`stages::names_light_source`) has after
/// `MaterialInput`'s other fields, before `shaded_tint`: Horizon's own look's light sources,
/// which the vertex and the sprite already hold, so they cost nothing.
pub const MATERIAL_LIGHT_SOURCE: [Field; 3] = [
    Field {
        name: "light_source_tier",
        ty: "u32",
        doc: "Only in a pack whose files name `light_source_tier`, `light_source_whole` or `light_source_threshold`: how Horizon's own HDR look ranks the surface's block as a light source: 0 none, 1 low (magma, a redstone torch, candles, froglights), 2 incandescent (torches, lanterns, fire, lava), 3 bright (glowstone, a beacon, campfires), 4 very bright (the sun and lightning; no block has it). It comes from the block, which `emission` doesn't tell (glow lichen gives light and is 0, a froglight gives 15 and is 1), the same on every face except a model face that gives its own light, which ranks at least by that light (an open eyeblossom's petals are 2); a fluid's surface has the fluid's (lava 2), and distant terrain keeps its block's (with `light_source_whole`), so far glowstone is 3.",
    },
    Field {
        name: "light_source_whole",
        ty: "bool",
        doc: "Only in a pack whose files name `light_source_tier`, `light_source_whole` or `light_source_threshold`: the whole sprite shines in Horizon's look (froglights, fire, lava, campfires), whatever a texel's brightness; otherwise only its texels as bright as `light_source_threshold` do (a torch's flame, not its handle). False where `light_source_tier` is 0.",
    },
    Field {
        name: "light_source_threshold",
        ty: "f32",
        doc: "Only in a pack whose files name `light_source_tier`, `light_source_whole` or `light_source_threshold`: the brightness a texel of the surface's sprite reaches where Horizon's look lets it shine (unless `light_source_whole`), 0..1. A texel's brightness is the mean of the luminance and the brightest channel of its colour in linear light: of `c = albedo.rgb`, `0.5 * (dot(c, vec3(0.2126, 0.7152, 0.0722)) + max(c.r, max(c.g, c.b)))`. It is measured from the sprite itself, a set share of the way from its darkest to its brightest opaque texel, when HDR output turns on: 0 for every sprite until the renderer drawing it has measured it (every texel passes), so a renderer made while HDR output is off, as turning it off makes one, reads 0; and 1 for a sprite without an opaque texel.",
    },
];

/// What a pack whose files name one of `MATERIAL_LIGHT_SOURCE`'s names has last in `EntityInput`.
pub const ENTITY_LIGHT_SOURCE: [Field; 1] = [Field {
    name: "light_source_tier",
    ty: "u32",
    doc: "Only in a pack whose files name `light_source_tier`, `light_source_whole` or `light_source_threshold`: how Horizon's own HDR look ranks the surface as a light source, 0..4 as `MaterialInput.light_source_tier` (a blaze, spider and enderman eyes 3; a magma cube, a glow squid, glowing sign text 1). Horizon's look has no threshold or whole sprite for these: every texel shines by its own brightness, `albedo * tint`.",
}];

/// The fields `input` has after its own, before `shaded_tint`, in a pack whose files name one of
/// `MATERIAL_LIGHT_SOURCE`'s names: `MaterialInput`'s three and `EntityInput`'s tier.
pub fn light_source(input: &Struct) -> &'static [Field] {
    match input.name {
        "MaterialInput" => &MATERIAL_LIGHT_SOURCE,
        "EntityInput" => &ENTITY_LIGHT_SOURCE,
        _ => &[],
    }
}

/// `input` with its `light_source` fields where `light_source`, `LIT_TINT` where `lit` (only
/// `MaterialInput` has it), then `SHADED_TINT` where `shaded`.
pub fn declared(input: &Struct, light_source: bool, lit: bool, shaded: bool) -> String {
    let mut out = input.wgsl();
    out.truncate(out.len() - 2);
    let more = (self::light_source(input).iter()).filter(|_| light_source);
    let lit = (lit && input.name == MATERIAL_INPUT.name).then_some(&LIT_TINT);
    for f in more.chain(lit).chain(shaded.then_some(&SHADED_TINT)) {
        out += &format!("    {}: {},\n", f.name, f.ty);
    }
    out + "}\n"
}

/// The tonemap stage's input: one pixel of the window.
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
        assert_eq!(wgsl.fields.len(), 63);
        assert!(wgsl.fields.iter().all(|f| !f.doc.trim().is_empty()));
        // `luminance` is a `vec3`, on the next 16 bytes, the epochs after it, then the fog's
        // two pairs and the moon's phase, to the end of the next 16; the full light's colour on
        // the 16 after them.
        let size = std::mem::size_of::<Frame>();
        assert_eq!(size, 6 * 64 + 15 * 16 + 8 * 16 + 6 * 16);
        assert_eq!(
            Frame::OFFSETS[55..],
            [
                size - 64,
                size - 52,
                size - 48,
                size - 44,
                size - 40,
                size - 32,
                size - 24,
                size - 16
            ]
        );
        // The scene's two fields are only a `[ray_tracing]` pack's, the epochs only a
        // `[[resource]]` pack's; the fog's every pack's; the moon's phase, last, only a pack's
        // that names it.
        let names = |s: Vec<Field>| s.iter().map(|f| f.name).collect::<Vec<_>>();
        let traced = names(Frame::declared(true, false, false, false));
        assert_eq!(
            traced[Frame::RAY_FIELDS],
            ["has_ray_tracing", "ray_distance"]
        );
        let untraced = names(Frame::declared(false, false, false, false));
        assert_eq!(untraced.len(), 56);
        assert!(!untraced.iter().any(|n| n.contains("ray")));
        assert!(
            !untraced
                .iter()
                .any(|n| *n == "fog_start" || *n == "fog_end")
        );
        let epochs = ["view_epoch", "world_epoch", "reset_reasons"];
        let fog = ["fog_environment", "fog_render"];
        for ray_tracing in [false, true] {
            let kept = names(Frame::declared(ray_tracing, true, false, false));
            assert_eq!(kept[kept.len() - 5..kept.len() - 2], epochs);
            let plain = names(Frame::declared(ray_tracing, false, false, false));
            assert_eq!(plain[plain.len() - 3..], ["luminance", fog[0], fog[1]]);
            let mooned = names(Frame::declared(ray_tracing, false, true, false));
            assert_eq!(mooned[mooned.len() - 3..], [fog[0], fog[1], "moon_phase"]);
            let full = names(Frame::declared(ray_tracing, false, false, true));
            assert_eq!(full[full.len() - 3..], [fog[0], fog[1], "full_lightmap"]);
        }
        assert_eq!(
            names(Frame::WGSL.fields.to_vec())[Frame::FULL_LIGHTMAP_FIELDS],
            ["full_lightmap"]
        );
        assert_eq!(
            names(Frame::WGSL.fields.to_vec())[Frame::KEPT_FIELDS],
            epochs
        );
        assert_eq!(
            names(Frame::WGSL.fields.to_vec())[Frame::MOON_FIELDS],
            ["moon_phase"]
        );
        assert!(wgsl.wgsl().contains(
            "    camera_position: vec3<f32>,
    time: f32,
"
        ));
    }
}
