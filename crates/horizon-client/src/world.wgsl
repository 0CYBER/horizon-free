struct Camera {
    view_projection: mat4x4<f32>,
    position: vec4<f32>,
    fog: vec4<f32>,
}
@group(0) @binding(0) var<uniform> camera: Camera;
@group(0) @binding(1) var sprites: texture_2d_array<f32>;
@group(0) @binding(2) var sprite_sampler: sampler;
struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) layer: u32,
    @location(2) shade: f32,
    @location(3) world_position: vec3<f32>,
}
@vertex fn vs_main(@builtin(vertex_index) vertex: u32,
    @location(0) origin: vec3<f32>, @location(1) extent: vec2<f32>,
    @location(2) face: u32, @location(3) layer: u32) -> VertexOut {
    let corners = array<vec2<f32>,4>(vec2(0.0,0.0),vec2(1.0,0.0),vec2(1.0,1.0),vec2(0.0,1.0));
    let normal_order = array<u32,6>(0u,1u,2u,0u,2u,3u);
    let reversed_order = array<u32,6>(0u,2u,1u,0u,3u,2u);
    let flipped = face==1u || face==2u || face==5u;
    let corner = corners[select(normal_order[vertex],reversed_order[vertex],flipped)];
    let offset = corner*extent;
    var position = origin;
    if face<2u { position+=vec3(offset.x,0.0,offset.y); }
    else if face<4u { position+=vec3(offset.x,offset.y,0.0); }
    else { position+=vec3(0.0,offset.y,offset.x); }
    var output: VertexOut;
    output.clip=camera.view_projection*vec4(position,1.0);
    output.uv=vec2(offset.x,extent.y-offset.y);
    output.layer=layer;
    output.shade=select(select(0.8,0.6,face>=4u),select(0.5,1.0,face==1u),face<2u);
    output.world_position=position;
    return output;
}
@fragment fn fs_main(input:VertexOut)->@location(0) vec4<f32> {
    let color=textureSample(sprites,sprite_sampler,input.uv,i32(input.layer));
    let distance=length(input.world_position-camera.position.xyz);
    let fog_amount=smoothstep(camera.position.w*0.65,camera.position.w,distance);
    return vec4(mix(color.rgb*input.shade,camera.fog.rgb,fog_amount),1.0);
}

@vertex fn vs_model(@builtin(vertex_index) vertex: u32,
    @location(0) p0: vec3<f32>, @location(1) p1: vec3<f32>,
    @location(2) p2: vec3<f32>, @location(3) p3: vec3<f32>,
    @location(4) uv0: vec2<f32>, @location(5) uv1: vec2<f32>,
    @location(6) uv2: vec2<f32>, @location(7) uv3: vec2<f32>,
    @location(8) layer: u32, @location(9) shade: f32) -> VertexOut {
    let order=array<u32,6>(0u,1u,2u,0u,2u,3u);
    let positions=array<vec3<f32>,4>(p0,p1,p2,p3);
    let uvs=array<vec2<f32>,4>(uv0,uv1,uv2,uv3);
    let corner=order[vertex];
    var output: VertexOut;
    output.clip=camera.view_projection*vec4(positions[corner],1.0);
    output.uv=uvs[corner];
    output.layer=layer;
    output.shade=shade;
    output.world_position=positions[corner];
    return output;
}
@fragment fn fs_model(input:VertexOut)->@location(0) vec4<f32> {
    let color=textureSample(sprites,sprite_sampler,input.uv,i32(input.layer));
    // Inspection sprites are validated to contain only zero or full alpha.
    if color.a<0.5 { discard; }
    let distance=length(input.world_position-camera.position.xyz);
    let fog_amount=smoothstep(camera.position.w*0.65,camera.position.w,distance);
    return vec4(mix(color.rgb*input.shade,camera.fog.rgb,fog_amount),1.0);
}
