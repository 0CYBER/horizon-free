//! Naga's HLSL writer names explicit level-zero samples by the methods HLSL provides. GLSL
//! samples outside a fragment shader become explicit level-zero operations in Horizon's
//! translator: ordinary textures use `SampleLevel(..., 0.0)`, while depth comparisons use
//! `SampleCmpLevelZero`.

use horizon_shaders::{Macros, Pack, Target, preprocess, translate_composite};

fn hlsl(declaration: &str, sample: &str) -> String {
    let vertex =
        format!("#version 450\n{declaration}\nvoid main() {{ gl_Position = {sample}; }}\n");
    let fragment = "#version 450\nvoid main() { gl_FragData[0] = vec4(1.0); }\n";
    let zip = {
        use std::io::Write;
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (path, source) in [
            ("shaders/composite.vsh", vertex.as_str()),
            ("shaders/composite.fsh", fragment),
        ] {
            zip.start_file(path, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(source.as_bytes()).unwrap();
        }
        zip.finish().unwrap().into_inner()
    };
    let pack = Pack::from_zip_bytes("zero-level.zip", zip).unwrap();
    let macros = Macros::standard();
    let program = translate_composite(
        &preprocess(&pack, "composite.vsh", &macros),
        &preprocess(&pack, "composite.fsh", &macros),
        Target::Buffers,
    )
    .unwrap();
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&program.vertex)
    .unwrap();
    let mut out = String::new();
    naga::back::hlsl::Writer::new(
        &mut out,
        &naga::back::hlsl::Options::default(),
        &naga::back::hlsl::PipelineOptions {
            entry_point: Some((naga::ShaderStage::Vertex, "main".into())),
        },
    )
    .write(&program.vertex, &info, None)
    .unwrap();
    out
}

#[test]
fn level_zero_samples_use_methods_hlsl_has() {
    let ordinary = hlsl("uniform sampler2D image;", "texture(image, vec2(0.5))");
    let ordinary_call = ordinary
        .lines()
        .find(|line| line.contains("image.Sample"))
        .expect("the authored sample reaches HLSL");
    assert!(ordinary_call.contains(".SampleLevel("), "{ordinary_call}");
    assert!(ordinary_call.contains(", 0.0)"), "{ordinary_call}");
    assert!(
        !ordinary_call.contains("SampleLevelZero"),
        "{ordinary_call}"
    );

    let comparison = hlsl(
        "uniform sampler2DShadow image;",
        "vec4(texture(image, vec3(0.5)))",
    );
    let comparison_call = comparison
        .lines()
        .find(|line| line.contains("image.Sample"))
        .expect("the authored comparison reaches HLSL");
    assert!(
        comparison_call.contains(".SampleCmpLevelZero("),
        "{comparison_call}"
    );
}
