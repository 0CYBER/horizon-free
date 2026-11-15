// The pack's own sky: vanilla's colour tinted, and the clouds, stars and sun the manifest takes
// over from the renderer, each unlike vanilla's, so a sky that falls back to the renderer's shows.

// A hash of an integer cell, the same on every run and every GPU.
fn cell_hash(cell: vec3<i32>) -> u32 {
    let h = bitcast<vec3<u32>>(cell) * vec3<u32>(73856093u, 19349663u, 83492791u);
    var x = h.x ^ h.y ^ h.z;
    x = (x ^ (x >> 16u)) * 0x7feb352du;
    x = (x ^ (x >> 15u)) * 0x846ca68bu;
    return x ^ (x >> 16u);
}

fn sky(input: SkyInput) -> vec4<f32> {
    let d = input.direction;
    let day = smoothstep(-0.1, 0.1, frame.sun_direction.y);
    var color = input.color.rgb * vec3<f32>(0.8, 0.9, 1.2);
    // Clouds: square cells on a plane above the camera, one in four filled, by day.
    if d.y > 0.05 {
        let cell = vec2<i32>(floor(d.xz / d.y * 3.0));
        let filled = select(0.0, 1.0, (cell_hash(vec3<i32>(cell, 0)) & 3u) == 0u);
        color = mix(color, vec3<f32>(0.95, 0.9, 0.8) * day, filled * 0.6 * smoothstep(0.05, 0.25, d.y));
    }
    // Stars: one cell of the direction in a hundred lit, by night.
    let star = select(0.0, 1.0, cell_hash(vec3<i32>(floor(d * 150.0))) % 100u == 0u);
    color += vec3<f32>(1.0, 0.8, 0.5) * star * (1.0 - day);
    // The sun: a hard-edged orange disc, round where vanilla's is square.
    let sun = select(0.0, 1.0, dot(d, frame.sun_direction) > 0.996);
    color = mix(color, vec3<f32>(1.0, 0.45, 0.1), sun);
    return vec4<f32>(color, input.color.a);
}
