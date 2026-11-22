// Native mods' world primitives (gfx/world_primitives.rs): quads from the shared atlas and lines
// with a width in pixels, camera-relative, at the end of the world pass.

struct Globals {
    // Camera-relative world to clip space, with TAA's jitter (what the world was drawn with).
    view_proj: mat4x4<f32>,
    // The frame's size in pixels (line widths are in its pixels).
    viewport: vec4<f32>,
}
@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var atlas: texture_2d<f32>;
@group(0) @binding(2) var atlas_sampler: sampler;

struct QuadOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    // A beam's sprite v range (start, length), else 0.
    @location(2) @interpolate(flat) wrap: vec2<f32>,
}

// `Vertex` (mesher.rs): position, atlas UV, layer, RGBA colour, light.
@vertex fn vs_quad(@location(0) pos: vec3<f32>, @location(1) uv: vec2<f32>,
    @location(2) layer: u32, @location(3) color: vec4<f32>, @location(4) light: u32) -> QuadOut {
    var out: QuadOut;
    out.clip = globals.view_proj * vec4(pos, 1.0);
    out.uv = uv;
    out.color = color;
    // Layer 6 is a beacon beam's: its v repeats inside the sprite, whose range rides in the
    // light bytes (block_entity_render::beam).
    if (layer & 0xffffu) == 6u {
        out.wrap = vec2(f32(light & 0xffffu), f32(light >> 16u)) / 65535.0;
    } else {
        out.wrap = vec2(0.0);
    }
    return out;
}

@fragment fn fs_quad(in: QuadOut) -> @location(0) vec4<f32> {
    var uv = in.uv;
    if in.wrap.y > 0.0 { uv.y = in.wrap.x + fract(in.uv.y) * in.wrap.y; }
    let color = textureSample(atlas, atlas_sampler, uv) * in.color;
    if color.a <= 0.0 { discard; }
    return frame_stored(color);
}

struct LineOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
}

// One instance per line (gizmos::camera_relative): its ends, ARGB colour and width in pixels,
// a strip of 4 vertices across it, as terrain.wgsl's `vs_debug_line`.
@vertex fn vs_line(@builtin(vertex_index) vertex: u32,
    @location(0) start: vec3<f32>, @location(1) end: vec3<f32>,
    @location(2) argb: vec4<f32>, @location(3) width: f32) -> LineOut {
    var out: LineOut;
    // ARGB in a little-endian u32 reads as B, G, R, A.
    out.color = argb.zyxw;

    let view_shrink = 1.0 - 1.0 / 256.0;
    let p0 = globals.view_proj * vec4(start * view_shrink, 1.0);
    let p1 = globals.view_proj * vec4(end * view_shrink, 1.0);

    // An end behind the camera has no screen position: cut the line where it reaches the
    // near plane (in clip space, so the cut is exact), and drop it when both ends are behind.
    let near = 0.01;
    if p0.w < near && p1.w < near {
        out.clip = vec4(0.0, 0.0, -2.0, 1.0);
        return out;
    }
    var c0 = p0;
    var c1 = p1;
    if c0.w < near {
        c0 = mix(c0, c1, (near - c0.w) / (c1.w - c0.w));
    } else if c1.w < near {
        c1 = mix(c1, c0, (near - c1.w) / (c0.w - c1.w));
    }

    let ndc0 = c0.xyz / c0.w;
    let ndc1 = c1.xyz / c1.w;
    let screen = globals.viewport.xy;
    let delta = (ndc1.xy - ndc0.xy) * screen;
    let len = length(delta);
    let direction = select(vec2(1.0, 0.0), delta / len, len > 0.001);
    let offset = vec2(-direction.y, direction.x) * width / screen;

    // Vertices 0 and 1 sit at the start, 2 and 3 at the end; odd ones on the far side.
    let at_end = vertex >= 2u;
    let base = select(ndc0, ndc1, at_end);
    let w = select(c0.w, c1.w, at_end);
    let side = select(-1.0, 1.0, vertex % 2u != 0u);
    out.clip = vec4((base + vec3(side * offset, 0.0)) * w, w);
    return out;
}

@fragment fn fs_line(in: LineOut) -> @location(0) vec4<f32> {
    return frame_stored(in.color);
}
