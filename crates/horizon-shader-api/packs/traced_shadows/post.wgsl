// The sun's shadows, one ray a pixel: where terrain stands between a point and the sun, the
// point is darkened. It shows how `trace_any` is called, not how shadows should look.

// What a point in the sun's shadow keeps of its light.
const SHADE: f32 = 0.5;
// How far a ray looks for something in its way, in blocks: past the scene from anywhere in it.
const REACH: f32 = 512.0;

fn post(input: PostInput) -> vec4<f32> {
    let sun = frame.sun_direction;
    // The sky, the night, and a device that doesn't trace: the picture as it is. So are the
    // Nether and the End: the frame has a sun direction there too, and every ray would end in
    // rock.
    let sunless = frame.dimension != 0 || sun.y <= 0.0;
    if frame.has_ray_tracing == 0u || input.depth == 0.0 || sunless {
        return input.color;
    }
    let position = view_position(input.uv, input.depth);
    // The scene holds the terrain near the camera: past it nothing is there to cast a shadow.
    if max(abs(position.x), abs(position.z)) > frame.ray_distance {
        return input.color;
    }
    // The ray starts a little toward the camera, off the surface it would otherwise hit at once.
    let away = length(position);
    let origin = position * (1.0 - 0.002 - 0.02 / max(away, 0.02));
    if trace_any(origin, sun, REACH) {
        return vec4<f32>(input.color.rgb * SHADE, input.color.a);
    }
    return input.color;
}
