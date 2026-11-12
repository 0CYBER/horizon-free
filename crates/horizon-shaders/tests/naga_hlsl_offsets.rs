//! HLSL sampling offsets have one signed component per texture axis, not per array layer.

#[test]
fn sampling_offset_casts_preserve_their_dimension() {
    for (texture, sample, cast) in [
        (
            "texture_1d<f32>",
            "textureSampleLevel(image, image_sampler, 0.5, 0.0, 1)",
            ", int(",
        ),
        (
            "texture_2d<f32>",
            "textureSample(image, image_sampler, vec2<f32>(0.5), vec2<i32>(0, 1))",
            ", int2(",
        ),
        (
            "texture_2d_array<f32>",
            "textureSample(image, image_sampler, vec2<f32>(0.5), 0, vec2<i32>(0, 1))",
            ", int2(",
        ),
        (
            "texture_3d<f32>",
            "textureSample(image, image_sampler, vec3<f32>(0.5), vec3<i32>(0, 1, 2))",
            ", int3(",
        ),
    ] {
        let source = format!(
            "@group(0) @binding(0) var image: {texture};\n\
             @group(0) @binding(1) var image_sampler: sampler;\n\
             @fragment fn main() -> @location(0) vec4<f32> {{ return {sample}; }}"
        );
        let module = naga::front::wgsl::parse_str(&source).unwrap();
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        let mut output = String::new();
        naga::back::hlsl::Writer::new(
            &mut output,
            &naga::back::hlsl::Options::default(),
            &naga::back::hlsl::PipelineOptions {
                entry_point: Some((naga::ShaderStage::Fragment, "main".into())),
            },
        )
        .write(&module, &info, None)
        .unwrap();
        let call = output
            .lines()
            .find(|line| line.contains(".Sample"))
            .unwrap();
        assert!(call.contains(cast), "{texture}: {call}");
    }
}
