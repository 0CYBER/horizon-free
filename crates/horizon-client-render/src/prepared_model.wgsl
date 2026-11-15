// One instance of generation-owned prepared geometry. The model matrix is camera-relative:
// its translation was subtracted in f64 before the f32 cast. Normals use the CPU inverse-transpose.
@vertex fn vs_prepared_model(
    @location(0) position: vec4<f32>,
    @location(1) uv: vec4<f32>,
    @location(2) normal: vec4<f32>,
    @location(3) flags: vec4<u32>,
    @location(4) col0: vec4<f32>,
    @location(5) col1: vec4<f32>,
    @location(6) col2: vec4<f32>,
    @location(7) col3: vec4<f32>,
    @location(8) nrow0: vec4<f32>,
    @location(9) nrow1: vec4<f32>,
    @location(10) nrow2: vec4<f32>,
    @location(11) tint: vec4<f32>,
    @location(12) light: u32,
) -> Out {
    let model = mat4x4(col0, col1, col2, col3);
    let nmat = mat3x3(nrow0.xyz, nrow1.xyz, nrow2.xyz);
    let world = (model * vec4(position.xyz, 1.0)).xyz;
    let n = nmat * normal.xyz;
    let len = length(n);
    let unit = select(vec3(0.0), n / len, len > 0.0);
    let q = trunc(clamp(unit, vec3(-1.0), vec3(1.0)) * 127.0);
    let sn = vec4(q / 127.0, 1.0);
    let color = select(vec4(1.0), tint, flags.x != 0u);
    let sky = light & 0xffu;
    let block = max((light >> 8u) & 0xffu, flags.y * 16u);
    return entity_vertex(world, uv.xy, 0u, color, vec4<u32>(sky, block, 0u, 0u), sn);
}
