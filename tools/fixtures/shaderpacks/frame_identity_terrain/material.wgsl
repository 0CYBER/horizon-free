// Terrain lit by vanilla's light plus the sun (the moon by night) through the shadow map, on the
// LabPBR normal map's normals; under [displacement] the normal map's heights are geometry. Each part
// changes the picture plainly, so a change in how terrain reaches this stage shows.

// The relief's full depth, in blocks: a height of 0 lies this far below the face.
const RELIEF: f32 = 0.25;

fn displacement(input: DisplacementInput) -> f32 {
    return (1.0 - input.height) * RELIEF;
}

// 1 where the light reaches `position`, 0 in shadow; nudged off the face by about two shadow
// texels so a face doesn't shadow itself.
fn lit(position: vec3<f32>, normal: vec3<f32>) -> f32 {
    if frame.has_shadow == 0u {
        return 1.0;
    }
    let p = shadow_position(position + normal * frame.shadow_center_texel * 2.0);
    if any(p.xy < vec2<f32>(0.0)) || any(p.xy > vec2<f32>(1.0)) {
        return 1.0;
    }
    return textureSampleCompareLevel(shadow_map, shadow_sampler, p.xy, p.z);
}

fn material(input: MaterialInput) -> Surface {
    // The LabPBR normal map's normal, in world axes (rg along the tangent and down the texture).
    let map = block_normal_map(input.uv);
    let xy = map.rg * 2.0 - 1.0;
    let down = cross(input.normal, input.tangent.xyz) * input.tangent.w;
    let normal = normalize(input.tangent.xyz * xy.x + down * xy.y
        + input.normal * sqrt(max(1.0 - dot(xy, xy), 0.0)));
    // The sun by day, a little of the moon by night, in the Overworld, where the sky reaches.
    let day = smoothstep(-0.05, 0.15, frame.sun_direction.y);
    let body = select(0.0, mix(0.15, 1.0, day), frame.dimension == 0) * input.light.y;
    let direct = max(dot(normal, frame.light_direction), 0.0) * lit(input.position, input.normal);
    // Vanilla's light at a third, plus the direct light; the map's ambient occlusion on both.
    let light = (input.lightmap * 0.35 + vec3<f32>(1.2, 1.1, 0.9) * body * direct) * map.b;
    return Surface(vec4<f32>(input.albedo.rgb * input.tint.rgb * light, input.albedo.a * input.tint.a));
}
