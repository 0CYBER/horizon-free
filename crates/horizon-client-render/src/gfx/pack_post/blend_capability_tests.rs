//! Original required-capability and independent attachment controls (spec 3.2).
use super::destination_tests::Fixture;
use super::*;
use std::io::{Cursor, Write};
use wgpu::util::DeviceExt;

#[test]
fn required_blending_refuses_only_when_independent_blend_is_missing() {
    use wgpu::DownlevelFlags as D;
    for flags in [
        D::empty(),
        D::COMPUTE_SHADERS,
        D::INDEPENDENT_BLEND,
        D::all(),
    ] {
        assert!(blend_capability::check(false, flags).is_ok());
        let result = blend_capability::check(true, flags);
        assert_eq!(result.is_ok(), flags.contains(D::INDEPENDENT_BLEND));
        if let Err(refused) = result {
            assert!(refused.reason.contains("PER_BUFFER_BLENDING"));
            assert!(!refused.is_cancelled());
        }
    }
}

fn pack(requirement: &str) -> Post {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let properties = format!(
        "iris.features.{requirement}=per_buffer_blending\n\
         alphaTest.gbuffers_terrain=off\n\
         blend.gbuffers_terrain=SRC_ALPHA ONE_MINUS_SRC_ALPHA ONE ZERO\n\
         blend.gbuffers_terrain.colortex5=off\n\
         blend.gbuffers_terrain.colortex7=ZERO ONE ZERO ONE\n"
    );
    for (name, source) in [
        ("shaders.properties", properties.as_str()),
        (
            "gbuffers_terrain.vsh",
            "#version 120\nvoid main() { gl_Position=gl_Vertex; }\n",
        ),
        (
            "gbuffers_terrain.fsh",
            "#version 330 compatibility\n/* RENDERTARGETS: 2,5,7 */\n\
             const vec4 colortex2ClearColor=vec4(0.2,0.3,0.4,0.6);\n\
             const vec4 colortex5ClearColor=vec4(0.2,0.3,0.4,0.6);\n\
             const vec4 colortex7ClearColor=vec4(0.2,0.3,0.4,0.6);\n\
             void main() {\n\
             gl_FragData[0]=vec4(0.8,0.4,0.2,0.5);\n\
             gl_FragData[1]=vec4(0.8,0.4,0.2,0.5);\n\
             gl_FragData[2]=vec4(0.8,0.4,0.2,0.5);\n}\n",
        ),
        (
            "final.vsh",
            "#version 120\nvoid main() { gl_Position=ftransform(); }\n",
        ),
        (
            "final.fsh",
            "#version 330 compatibility\nuniform sampler2D colortex2;\n\
             uniform sampler2D colortex5;\nuniform sampler2D colortex7;\n\
             void main() {\nivec2 p=ivec2(gl_FragCoord.xy);\n\
             gl_FragColor=vec4(texelFetch(colortex2,p,0).r,\n\
             texelFetch(colortex5,p,0).r,texelFetch(colortex7,p,0).r,1.0);\n}\n",
        ),
    ] {
        zip.start_file(
            format!("shaders/{name}"),
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(source.as_bytes()).unwrap();
    }
    let pack = horizon_shaders::Pack::from_zip_bytes(
        "original-required-attachment-blending",
        zip.finish().unwrap().into_inner(),
    )
    .unwrap();
    let mut macros = horizon_shaders::Macros::standard();
    let (properties, notes) = horizon_shaders::read_properties(&pack, &mut macros);
    assert!(notes.is_empty(), "{notes:?}");
    let inputs = horizon_shaders::GameInputs {
        terrain: Some(horizon_shaders::GbuffersInputs {
            declarations: "layout(location=0) in vec2 fixture_position;\n".into(),
            // The near plane passes against any valid native depth in the reused fixture.
            body: "hz_Vertex=vec4(fixture_position,-1.0,1.0);\n".into(),
            ..Default::default()
        }),
        ..Default::default()
    };
    let post = horizon_shaders::load_post(&pack, "", &properties, &macros, &inputs);
    assert_eq!(post.per_buffer_blending_required, requirement == "required");
    assert!(post.terrain.is_some(), "{:?}", post.notes);
    post
}

#[test]
fn required_blending_loads_on_capable_adapters_and_preserves_attachment_states() {
    let Some(gpu) = Fixture::new() else {
        eprintln!("required blending GPU control skipped: no adapter");
        return;
    };
    let flags = gpu.adapter.get_downlevel_capabilities().flags;
    if !flags.contains(wgpu::DownlevelFlags::INDEPENDENT_BLEND) {
        let result = gpu.try_build(pack("required"));
        let Err(refused) = result else {
            panic!("a required unsupported capability must refuse the whole pack");
        };
        assert!(refused.reason.contains("PER_BUFFER_BLENDING"));
        eprintln!("attachment pixel control skipped: adapter lacks independent blending");
        return;
    }
    const ATTRIBUTES: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![0 => Float32x2];
    let layouts = [Some(wgpu::VertexBufferLayout {
        array_stride: 8,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &ATTRIBUTES,
    })];
    let positions = [[-1.0f32, -1.0], [3.0, -1.0], [-1.0, 3.0]];
    let vertices = gpu
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("original blend fixture triangle"),
            contents: bytemuck::cast_slice(&positions),
            usage: wgpu::BufferUsages::VERTEX,
        });
    for requirement in ["required", "optional"] {
        let geometry = Some(Geometry {
            buffers: &layouts,
            table: None,
            depth_write: false,
            atlas: None,
            material_maps: false,
            ids: None,
            cull: false,
        });
        let mut post = gpu.build_with_terrain(pack(requirement), geometry);
        assert!(post.terrain.is_some(), "{:?}", post.notes);
        let mut draws = 0;
        let pixels = gpu.render_with_terrain(&mut post, [4, 4], &mut |pass, list, _, _| {
            if matches!(list, TerrainList::Solid) {
                draws += 1;
                pass.set_vertex_buffer(0, vertices.slice(..));
                pass.draw(0..3, 0..1);
            }
        });
        assert_eq!(draws, 1, "fixture must submit its solid triangle once");
        // colortex2 inherits program alpha blending, colortex5 disables blending,
        // and colortex7 keeps its destination through its own ZERO/ONE factors.
        for pixel in pixels {
            assert!(
                pixel
                    .iter()
                    .zip([128, 204, 51, 255])
                    .all(|(got, want)| got.abs_diff(want) <= 1),
                "{requirement}: {pixel:?}"
            );
        }
    }
}
