// Light from the blocks that give it off (torches, lanterns, glowstone; not lava, which isn't
// listed), each source tested with a ray, so walls stop it and corners cast shadows. It adds to the picture, whose own block
// light stays: it shows how the light sources are read, not how block light should look.

// How bright a source's light is at its block, over the picture's own colour.
const GAIN: f32 = 1.5;

// How far along the unit `direction` from `origin` the ray enters the block whose centre is
// `center`: a full glowing cube (glowstone, a sea lantern) is in the ray scene itself, so a
// shadow ray aimed at its centre would be stopped by the source.
fn block_entry(origin: vec3<f32>, direction: vec3<f32>, center: vec3<f32>) -> f32 {
    // An axis the ray runs along never limits it: a tiny step instead of a division by 0.
    let tiny = vec3<f32>(1e-6);
    let inverse = 1.0 / select(direction, tiny, abs(direction) < tiny);
    let a = (center - 0.5 - origin) * inverse;
    let b = (center + 0.5 - origin) * inverse;
    let near = min(a, b);
    return max(max(near.x, near.y), near.z);
}

// The face a pixel's point lies on, from its neighbours' points, turned toward the camera.
fn surface_normal(uv: vec2<f32>, position: vec3<f32>) -> vec3<f32> {
    let texel = 1.0 / vec2<f32>(textureDimensions(scene_depth));
    let right = uv + vec2<f32>(texel.x, 0.0);
    let down = uv + vec2<f32>(0.0, texel.y);
    let along = view_position(right, depth_at(right)) - position;
    let across = view_position(down, depth_at(down)) - position;
    var normal = normalize(cross(along, across));
    if dot(normal, position) > 0.0 {
        normal = -normal;
    }
    return normal;
}

fn post(input: PostInput) -> vec4<f32> {
    // The sky, and a device that doesn't trace: the picture as it is (no sources are listed then).
    if frame.has_ray_tracing == 0u || input.depth == 0.0 {
        return input.color;
    }
    let position = view_position(input.uv, input.depth);
    let normal = surface_normal(input.uv, position);
    // The rays start a little off the surface, which they would otherwise hit at once.
    let origin = position + normal * 0.02;
    let near = ray_lights_near(position);
    var light = vec3<f32>(0.0);
    // Every source the cell lists (at most 24): they are ranked at the cell's nearest point, not
    // at this pixel, so testing only the first few would show the cell's edges.
    for (var i = 0u; i < near.y; i++) {
        let source = ray_light(ray_light_at(near.x + i));
        let to = source.position - origin;
        let distance = length(to);
        if distance >= source.level {
            continue;
        }
        let direction = to / distance;
        let facing = dot(normal, direction);
        if facing <= 0.0 {
            continue;
        }
        // The ray stops just short of the source's own block, which may be a solid one.
        let reach = block_entry(origin, direction, source.position) - 0.01;
        if reach > 0.0 && trace_any(origin, direction, reach) {
            continue;
        }
        // As block light falls off: strongest at the source, gone at its level.
        let falloff = 1.0 - distance / source.level;
        light += source.color * falloff * falloff * facing;
    }
    return vec4<f32>(input.color.rgb * (1.0 + light * GAIN), input.color.a);
}
