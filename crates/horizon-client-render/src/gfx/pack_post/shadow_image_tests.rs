//! Original controls for spec 2.6, 5 images, 6.1.6 and 6.3: storage images name the
//! current shadow colour buffer, including across shadowcomp flips and later programs.
use super::destination_tests::Fixture;
use super::*;
use std::io::{Cursor, Write};

const VERTEX: &str = "#version 120\nvoid main() { gl_Position=ftransform(); }\n";

fn load(files: &[(&str, String)]) -> Post {
    load_with_inputs(files, &Default::default())
}

fn load_with_inputs(files: &[(&str, String)], inputs: &horizon_shaders::GameInputs) -> Post {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, source) in files {
        zip.start_file(
            format!("shaders/{name}"),
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(source.as_bytes()).unwrap();
    }
    let pack = horizon_shaders::Pack::from_zip_bytes(
        "original-shadow-image-control",
        zip.finish().unwrap().into_inner(),
    )
    .unwrap();
    let mut macros = horizon_shaders::Macros::standard();
    let (properties, _) = horizon_shaders::read_properties(&pack, &mut macros);
    horizon_shaders::load_post(&pack, "", &properties, &macros, inputs)
}

fn coherence(format: &str, qualifier: &str, integer: bool) -> Post {
    let vector = if integer { "uvec4" } else { "vec4" };
    let image = if integer { "uimage2D" } else { "image2D" };
    let sampler = if integer { "usampler2D" } else { "sampler2D" };
    let value = |n: u32| {
        if integer {
            n.to_string()
        } else {
            format!("{}", n as f32 / 8.0)
        }
    };
    let shader = |body: String| {
        format!(
            "#version 430\nlayout(local_size_x=1) in;\nconst ivec3 workGroups=ivec3(4,4,1);\n{body}\n"
        )
    };
    let decl =
        |n: usize| format!("layout({qualifier}) writeonly uniform {image} shadowcolorimg{n};\n");
    let scale = if integer { " / 8.0" } else { "" };
    let outputs = format!(
        "layout(location=0) out {vector} firstColor;\nlayout(location=1) out {vector} secondColor;\n"
    );
    let one_output = format!("layout(location=0) out {vector} firstColor;\n");
    let first = format!("{vector}({},0,0,1)", value(1));
    let second = format!("{vector}(0,{},0,1)", value(2));
    let flipped = format!(
        "texelFetch(shadowcolor0,ivec2(0),0)+{vector}({},0,0,0)",
        value(1)
    );
    load(&[
        ("shadowcomp.vsh", VERTEX.into()),
        (
            "shadowcomp.fsh",
            format!(
                "#version 430\n/* RENDERTARGETS:0,1 */\n/* const int shadowcolor0Format={format};\nconst int shadowcolor1Format={format}; */\n{outputs}void main() {{ firstColor={first}; secondColor={second}; }}"
            ),
        ),
        // Samplers snapshot the preceding draw; image stores update the same current sides.
        (
            "shadowcomp1.csh",
            shader(format!(
                "{}{}uniform {sampler} shadowcolor0;\nuniform {sampler} shadowcolor1;\nvoid main() {{ ivec2 p=ivec2(gl_GlobalInvocationID.xy); {vector} a=texelFetch(shadowcolor0,p,0); {vector} b=texelFetch(shadowcolor1,p,0); imageStore(shadowcolorimg0,p,a+{vector}({},0,0,0)); imageStore(shadowcolorimg1,p,b+{vector}(0,{},0,0)); }}",
                decl(0),
                decl(1),
                value(2),
                value(2)
            )),
        ),
        ("shadowcomp1.vsh", VERTEX.into()),
        (
            "shadowcomp1.fsh",
            format!(
                "#version 430\n/* RENDERTARGETS:0 */\nuniform {sampler} shadowcolor0;\n{one_output}void main() {{ firstColor={flipped}; }}"
            ),
        ),
        ("shadowcomp2.vsh", VERTEX.into()),
        // No colour output: storage-only shadow fragments need a shadow-sized sink.
        (
            "shadowcomp2.fsh",
            format!(
                "#version 430\n{}uniform {sampler} shadowcolor1;\nvoid main() {{ ivec2 p=ivec2(gl_FragCoord.xy); imageStore(shadowcolorimg1,p,texelFetch(shadowcolor1,p,0)+{vector}(0,{},0,0)); }}",
                decl(1),
                value(1)
            ),
        ),
        ("shadowcomp3.vsh", VERTEX.into()),
        // An odd number of buffer0 flips alternates its physical main side each frame.
        (
            "shadowcomp3.fsh",
            format!(
                "#version 430\n/* RENDERTARGETS:0 */\nuniform {sampler} shadowcolor0;\n{one_output}void main() {{ firstColor=texelFetch(shadowcolor0,ivec2(0),0); }}"
            ),
        ),
        // This runs after the render flip. A compute-only program must not flip again.
        (
            "prepare.csh",
            shader(format!(
                "{}layout({qualifier}) readonly uniform {image} shadowcolorimg1;\nvoid main() {{ ivec2 p=ivec2(gl_GlobalInvocationID.xy); {vector} a=imageLoad(shadowcolorimg1,p); imageStore(shadowcolorimg0,p,{vector}(a.g,0,0,1)); }}",
                decl(0)
            )),
        ),
        ("composite.vsh", VERTEX.into()),
        // Fragment storage shares a sampler: the sampler must see a pre-pass copy.
        (
            "composite.fsh",
            format!(
                "#version 430\n{}uniform {sampler} shadowcolor1;\nvoid main() {{ ivec2 p=ivec2(gl_FragCoord.xy); {vector} a=texelFetch(shadowcolor1,p,0); imageStore(shadowcolorimg1,p,a+{vector}(0,{},0,0)); }}",
                decl(1),
                value(1)
            ),
        ),
        ("final.vsh", VERTEX.into()),
        (
            "final.fsh",
            format!(
                "#version 430\nconst int shadowMapResolution=4;\nlayout({qualifier}) readonly uniform {image} shadowcolorimg0;\nuniform {sampler} shadowcolor1;\nvoid main() {{ gl_FragColor=vec4(float(imageLoad(shadowcolorimg0,ivec2(0)).r){scale},float(texelFetch(shadowcolor1,ivec2(0),0).g){scale},0,1); }}"
            ),
        ),
    ])
}

#[test]
fn shadow_images_follow_rendered_buffers_flips_and_ordered_storage_passes() {
    let Some(gpu) = Fixture::new() else { return };
    for (format, qualifier, integer) in [
        ("RGBA8", "rgba8", false),
        ("RGBA16F", "rgba16f", false),
        ("RGBA32UI", "rgba32ui", true),
    ] {
        let mut post = gpu
            .try_build(coherence(format, qualifier, integer))
            .expect("shadow image pack builds");
        for size in [[4, 4], [4, 4], [8, 8]] {
            let pixels = gpu.render(&mut post, size);
            let got = pixels[0];
            assert!(
                got[0].abs_diff(159) <= 2
                    && got[1].abs_diff(191) <= 2
                    && got[2] == 0
                    && got[3] == 255,
                "{format} {size:?}: {got:?}; {:?}",
                post.notes
            );
        }
        assert!(
            post.notes
                .iter()
                .all(|note| note.starts_with("shadow map ")),
            "{:?}",
            post.notes
        );
        post.reset_history(&gpu.device);
        let got = gpu.render(&mut post, [4, 4])[0];
        assert!(
            got[0].abs_diff(159) <= 2 && got[1].abs_diff(191) <= 2,
            "reset {format}: {got:?}"
        );
    }
}

#[test]
fn unavailable_shadow_images_are_diagnosed_instead_of_binding_stand_ins() {
    let Some(gpu) = Fixture::new() else { return };
    for (properties, qualifier, expected) in [
        (
            "shadow.enabled=false\n",
            "rgba8",
            "needs enabled shadow buffers",
        ),
        (
            "shadow.enabled=true\n",
            "rgba16f",
            "but its texture is Rgba8Unorm",
        ),
    ] {
        let post = load(&[
            ("shaders.properties", properties.into()),
            (
                "composite.csh",
                format!(
                    "#version 430\nlayout(local_size_x=1) in;\nlayout({qualifier}) writeonly uniform image2D shadowcolorimg0;\nvoid main() {{ imageStore(shadowcolorimg0,ivec2(0),vec4(1)); }}"
                ),
            ),
            ("final.vsh", VERTEX.into()),
            (
                "final.fsh",
                "#version 120\nvoid main() { gl_FragColor=vec4(0,0,0,1); }".into(),
            ),
        ]);
        let post = gpu.try_build(post).unwrap();
        assert!(
            post.notes.iter().any(|note| note.contains(expected)),
            "{:?}",
            post.notes
        );
        assert!(!post.passes.iter().any(|p| p.name == "composite"));
        assert!(post.stand_ins.is_empty());
    }
}

#[test]
fn shadow_geometry_images_use_independent_buffers_and_reject_attachment_aliases() {
    let Some(gpu) = Fixture::new() else { return };
    for n in [1, 0] {
        let post = load_with_inputs(&[
            ("shadow.vsh", "#version 430\nvoid main() { vec2 p[3]=vec2[3](vec2(-1,-1),vec2(3,-1),vec2(-1,3)); gl_Position=vec4(p[gl_VertexID],0.5,1); }".into()),
            ("shadow.fsh", format!("#version 430\n/* RENDERTARGETS:0 */\nlayout(rgba8) writeonly uniform image2D shadowcolorimg{n};\nvoid main() {{ imageStore(shadowcolorimg{n},ivec2(gl_FragCoord.xy),vec4(0.25,0,0,1)); gl_FragData[0]=vec4(0,0,0.75,1); }}")),
            ("final.vsh", VERTEX.into()),
            ("final.fsh", format!("#version 430\nconst int shadowMapResolution=4;\nlayout(rgba8) readonly uniform image2D shadowcolorimg{n};\nuniform sampler2D shadowcolor0;\nvoid main() {{ gl_FragColor=vec4(imageLoad(shadowcolorimg{n},ivec2(0)).r,texelFetch(shadowcolor0,ivec2(0),0).b,0,1); }}")),
        ], &horizon_shaders::GameInputs { terrain: Some(Default::default()), ..Default::default() });
        assert!(post.shadow.is_some(), "{:?}", post.notes);
        let mut post = gpu.build_with_terrain(
            post,
            Some(Geometry {
                buffers: &[],
                table: None,
                depth_write: true,
                atlas: None,
                material_maps: false,
                ids: None,
                cull: false,
            }),
        );
        if n == 0 {
            assert!(post.shadow_terrain.is_none());
            assert!(
                post.notes
                    .iter()
                    .any(|note| note.contains("shadow image aliases a render attachment")),
                "{:?}",
                post.notes
            );
        } else {
            assert!(post.shadow_terrain.is_some(), "{:?}", post.notes);
            let got = gpu.render_with_terrain(&mut post, [4, 4], &mut |pass, list, _, _| {
                if list == TerrainList::ShadowSolid {
                    pass.draw(0..3, 0..1);
                }
            })[0];
            assert!(
                got[0].abs_diff(64) <= 1
                    && got[1].abs_diff(191) <= 1
                    && got[2] == 0
                    && got[3] == 255,
                "shadow geometry: {got:?}; {:?}",
                post.notes
            );
        }
    }
}

#[test]
fn image_only_shadow_demand_uses_shadow_resolution_and_separate_color_buffers() {
    let Some(gpu) = Fixture::new() else { return };
    for n in [0, 1] {
        let post = load(&[
            (
                "prepare.csh",
                format!(
                    "#version 430\nlayout(local_size_x=1) in;\nconst ivec3 workGroups=ivec3(1);\nlayout(rgba8) writeonly uniform image2D shadowcolorimg{n};\nlayout(rgba8) writeonly uniform image2D colorimg2;\nvoid main() {{ imageStore(shadowcolorimg{n},ivec2(3,3),vec4(0.25,0,0,1)); imageStore(colorimg2,ivec2(0),vec4(0,0,0.75,1)); }}"
                ),
            ),
            ("final.vsh", VERTEX.into()),
            (
                "final.fsh",
                format!(
                    "#version 430\nconst int shadowMapResolution=4;\nlayout(rgba8) readonly uniform image2D shadowcolorimg{n};\nuniform sampler2D colortex2;\nvoid main() {{ gl_FragColor=vec4(imageLoad(shadowcolorimg{n},ivec2(3,3)).r,float(imageSize(shadowcolorimg{n}).x)/8.0,texelFetch(colortex2,ivec2(0),0).b,1); }}"
                ),
            ),
        ]);
        let mut post = gpu.try_build(post).expect("image-only shadow pack builds");
        let got = gpu.render(&mut post, [8, 8])[0];
        assert!(
            got[0].abs_diff(64) <= 1
                && got[1].abs_diff(128) <= 1
                && got[2].abs_diff(191) <= 1
                && got[3] == 255,
            "shadowcolorimg{n}: {got:?}; {:?}",
            post.notes
        );
        assert!(
            post.notes
                .iter()
                .all(|note| note.starts_with("shadow map ")),
            "{:?}",
            post.notes
        );
    }
}

#[test]
fn early_shadow_image_writes_survive_clear_order_and_cache_hits() {
    use super::depth_test::{Gpu, SIZE, fixture_frame};
    use crate::hud::HudBatch;
    let Some(gpu) = Gpu::new() else {
        eprintln!("Shadow early-write/cache control SKIPPED: no GPU adapter");
        return;
    };
    for clear in [true, false] {
        let files = vec![
            ("begin.csh".into(), "#version 430\nlayout(local_size_x=1) in;\nconst ivec3 workGroups=ivec3(1);\nlayout(r32f) uniform image2D shadowcolorimg1;\nvoid main() { vec4 before=imageLoad(shadowcolorimg1,ivec2(0)); imageStore(shadowcolorimg1,ivec2(0),before+vec4(0.25,0,0,0)); }".into()),
            ("shadowcomp.vsh".into(), VERTEX.into()),
            ("shadowcomp.fsh".into(), "#version 430\n/* RENDERTARGETS:0 */\nlayout(r32f) readonly uniform image2D shadowcolorimg1;\nvoid main() { gl_FragData[0]=vec4(imageLoad(shadowcolorimg1,ivec2(0)).r,0,0,1); }".into()),
            ("final.vsh".into(), VERTEX.into()),
            ("final.fsh".into(), format!("#version 430\nconst int shadowMapResolution=4;\n/* const int shadowcolor1Format=R32F; */\nconst bool shadowcolor1Clear={clear};\nconst vec4 shadowcolor1ClearColor=vec4(0);\nlayout(r32f) readonly uniform image2D shadowcolorimg1;\nuniform sampler2D shadowcolor0;\nvoid main() {{ gl_FragColor=vec4(texelFetch(shadowcolor0,ivec2(0),0).r,imageLoad(shadowcolorimg1,ivec2(0)).r,0,1); }}")),
        ];
        let color = gpu.texture(wgpu::TextureFormat::Rgba8Unorm);
        let depth = gpu.texture(wgpu::TextureFormat::Depth32Float);
        gpu.clear(&color, &depth, 0.0);
        let mut post = gpu.load(
            files,
            World {
                atlas: None,
                material_maps: None,
                lightmap: color.clone(),
                terrain: None,
                block_ids: &[],
                sprite_rects: &[],
                celestial: None,
                gui_size: SIZE,
                clouds: None,
                weather: false,
                hand: false,
            },
            Default::default(),
        );
        let hud = HudBatch::new(
            SIZE,
            crate::hud::GuiScale {
                requested: 0,
                unicode: false,
            },
        );
        let mut frame = fixture_frame(&hud);
        frame.title = false;
        frame.shadow_cache = true;
        for (step, expected) in [
            [64, 64],
            [64, 128],
            if clear { [64, 64] } else { [191, 191] },
        ]
        .into_iter()
        .enumerate()
        {
            if step == 2 {
                frame.shadow_scene_revision.0 += 1;
            }
            let got = gpu.render(&mut post, &color, &depth, &frame, &mut |_, _, _, _| {})[0];
            assert!(
                got[0].abs_diff(expected[0]) <= 2 && got[1].abs_diff(expected[1]) <= 2,
                "clear={clear}, step={step}: {got:?}, expected {expected:?}; {:?}",
                post.notes
            );
        }
    }
}
