//! Original controls for spec revision 1, sections 3.2 and 2.6. No pack assets.
use super::destination_tests::Fixture;
use super::*;
use std::io::{Cursor, Write};

const VERTEX: &str = "#version 120\nvoid main() { gl_Position=ftransform(); }\n";

fn pack(properties: &str, shaders: &[(&str, &str)], final_body: &str) -> Post {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, source) in shaders.iter().copied().chain([
        ("shaders.properties", properties),
        ("final.vsh", VERTEX),
        ("final.fsh", final_body),
    ]) {
        zip.start_file(
            format!("shaders/{name}"),
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(source.as_bytes()).unwrap();
    }
    let pack = horizon_shaders::Pack::from_zip_bytes(
        "original-indirect-control",
        zip.finish().unwrap().into_inner(),
    )
    .unwrap();
    let mut macros = horizon_shaders::Macros::standard();
    let (properties, _) = horizon_shaders::read_properties(&pack, &mut macros);
    horizon_shaders::load_post(&pack, "", &properties, &macros, &Default::default())
}

#[test]
fn indirect_dispatch_reads_prior_writes_and_keeps_direct_dispatch() {
    let Some(gpu) = Fixture::new() else { return };
    assert!(
        gpu.adapter.get_downlevel_capabilities().flags.contains(
            wgpu::DownlevelFlags::COMPUTE_SHADERS | wgpu::DownlevelFlags::INDIRECT_EXECUTION
        ),
        "this control requires compute and indirect execution"
    );
    let post = pack(
        "bufferObject.2=4 true 1 1\nindirect.composite=2 4\nindirect.composite_b=2 4\n",
        &[
            (
                "begin.csh",
                "#version 430\nlayout(local_size_x=1) in;\nconst ivec3 workGroups=ivec3(1);\nlayout(std430,binding=2) buffer State { uint v[]; };\nvoid main(){v[1]=2u;v[2]=3u;v[3]=1u;v[8]=0u;v[9]=0u;v[10]=0u;v[11]=0u;}\n",
            ),
            (
                "composite.csh",
                "#version 430\nlayout(local_size_x=1) in;\nconst ivec3 workGroups=ivec3(70000,1,1);\nlayout(std430,binding=2) buffer State { uint v[]; };\nvoid main(){atomicAdd(v[8],1u);if(gl_NumWorkGroups==uvec3(2,3,1) && gl_GlobalInvocationID==gl_WorkGroupID) atomicAdd(v[9],1u);}\n",
            ),
            (
                "composite_a.csh",
                "#version 430\nlayout(local_size_x=1) in;\nconst ivec3 workGroups=ivec3(1);\nlayout(std430,binding=2) buffer State { uint v[]; };\nvoid main(){v[1]=1u;v[2]=2u;v[3]=1u;}\n",
            ),
            (
                "composite_b.csh",
                "#version 430\nlayout(local_size_x=1) in;\nconst ivec3 workGroups=ivec3(7,1,1);\nlayout(std430,binding=2) buffer State { uint v[]; };\nvoid main(){atomicAdd(v[10],1u);if(gl_NumWorkGroups==uvec3(1,2,1)) atomicAdd(v[11],1u);}\n",
            ),
        ],
        "#version 430 compatibility\nlayout(std430,binding=2) buffer State { uint v[]; };\nvoid main(){gl_FragColor=vec4(float(v[8])/8.0,float(v[10])/4.0,float(v[9]+v[11])/8.0,1);}\n",
    );
    assert!(post.notes.is_empty(), "{:?}", post.notes);
    let mut post = gpu.build(post);
    for size in [[8, 8], [8, 8], [11, 7]] {
        let pixels = gpu.render(&mut post, size);
        for p in pixels {
            assert!(
                p[0].abs_diff(191) <= 1 && p[1].abs_diff(128) <= 1 && p[2] == 255,
                "indirect work counts/builtins at {size:?}: {p:?}; {:?}",
                post.notes
            );
        }
    }
    post.reset_history(&gpu.device);
    let p = gpu.render(&mut post, [8, 8])[0];
    assert!(
        p[0].abs_diff(191) <= 1 && p[1].abs_diff(128) <= 1 && p[2] == 255,
        "indirect arguments after history reset: {p:?}; {:?}",
        post.notes
    );
}

#[test]
fn indirect_counts_and_ranges_are_bounded_without_shader_ssbo_bindings() {
    let Some(gpu) = Fixture::new() else { return };
    let limit = gpu.device.limits().max_compute_workgroups_per_dimension;
    for (counts, offset, expected) in [
        ([1, 1, 1], 4, [0, 255, 0, 255]),
        ([0, 1, 1], 4, [0, 0, 0, 255]),
        ([1, 0, 1], 4, [0, 0, 0, 255]),
        ([1, 1, 0], 4, [0, 0, 0, 255]),
        ([limit + 1, 1, 1], 4, [0, 0, 0, 255]),
        ([1, limit + 1, 1], 4, [0, 0, 0, 255]),
        ([1, 1, limit + 1], 4, [0, 0, 0, 255]),
        ([u32::MAX, 1, 1], 4, [0, 0, 0, 255]),
        ([1, 1, 1], 8, [0, 0, 0, 255]),
    ] {
        let setup = format!(
            "#version 430\nlayout(local_size_x=1) in;\nconst ivec3 workGroups=ivec3(1);\nlayout(std430,binding=2) buffer State {{ uint v[]; }};\nvoid main(){{v[1]={}u;v[2]={}u;v[3]={}u;}}\n",
            counts[0], counts[1], counts[2]
        );
        let post = pack(
            &format!("bufferObject.2=16\nindirect.composite=2 {offset}\n"),
            &[
                ("begin.csh", &setup),
                (
                    "composite.csh",
                    "#version 430\nlayout(local_size_x=1) in;\nconst ivec3 workGroups=ivec3(1);\nlayout(rgba8) writeonly uniform image2D colorimg1;\nvoid main(){imageStore(colorimg1,ivec2(0),vec4(0,1,0,1));}\n",
                ),
            ],
            "#version 430 compatibility\nuniform sampler2D colortex1;\nconst vec4 colortex1ClearColor=vec4(0,0,0,1);\nvoid main(){gl_FragColor=texelFetch(colortex1,ivec2(0),0);}\n",
        );
        let mut post = gpu.build(post);
        let pixels = gpu.render(&mut post, [8, 8]);
        assert_eq!(
            pixels[0], expected,
            "counts {counts:?}, offset {offset}; {:?}",
            post.notes
        );
    }
}

#[test]
fn an_unavailable_indirect_buffer_skips_only_its_compute() {
    let Some(gpu) = Fixture::new() else { return };
    let post = pack(
        "indirect.composite=7 0\n",
        &[(
            "composite.csh",
            "#version 430\nlayout(local_size_x=1) in;\nlayout(rgba8) writeonly uniform image2D colorimg1;\nvoid main(){imageStore(colorimg1,ivec2(0),vec4(1,0,0,1));}\n",
        )],
        "#version 430 compatibility\nvoid main(){gl_FragColor=vec4(0,1,0,1);}\n",
    );
    let mut post = gpu.try_build(post).expect("final still runs");
    assert!(
        post.notes.iter().any(|n| n.contains("indirect buffer 7")),
        "{:?}",
        post.notes
    );
    assert!(!post.passes.iter().any(|p| p.name == "composite"));
    assert_eq!(gpu.render(&mut post, [8, 8])[0], [0, 255, 0, 255]);
}

#[test]
fn changing_indirect_shadow_arguments_bypasses_only_the_indirect_shadow_cache() {
    use crate::hud::HudBatch;
    let Some(gpu) = Fixture::new() else { return };
    for (indirect, expected) in [(true, 6u32), (false, 2u32)] {
        let properties = format!(
            "shadow.enabled=true\nbufferObject.0=32\n{}",
            if indirect {
                "indirect.shadowcomp=0 0\n"
            } else {
                ""
            }
        );
        let post = pack(
            &properties,
            &[
                (
                    "begin.csh",
                    "#version 430\nlayout(local_size_x=1) in;\nconst ivec3 workGroups=ivec3(1);\nlayout(std430,binding=0) buffer State { uint v[]; };\nvoid main(){v[0]+=1u;v[1]=1u;v[2]=1u;}\n",
                ),
                (
                    "shadowcomp.csh",
                    "#version 430\nlayout(local_size_x=1) in;\nconst ivec3 workGroups=ivec3(1);\nlayout(std430,binding=0) buffer State { uint v[]; };\nvoid main(){atomicAdd(v[3],1u);}\n",
                ),
            ],
            "#version 430 compatibility\nuniform sampler2D shadowtex0;\nconst int shadowMapResolution=16;\nvoid main(){gl_FragColor=vec4(texelFetch(shadowtex0,ivec2(0),0).r,1,0,1);}\n",
        );
        let mut post = gpu.build(post);
        gpu.render(&mut post, [8, 8]);
        assert!(post.shadow_targets.is_some());
        let hud = HudBatch::scratch([8, 8]);
        let mut frame = super::depth_test::fixture_frame(&hud);
        frame.shadow_cache = true;
        let mut passes = std::mem::take(&mut post.passes);
        let begin = passes.iter_mut().find(|p| p.name == "begin").unwrap();
        for _ in 0..2 {
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            post.dispatch(&gpu.device, &gpu.queue, &mut encoder, begin, [8, 8]);
            post.run_shadow(
                &gpu.device,
                &gpu.queue,
                &mut encoder,
                &mut |_, _, _, _| {},
                &frame,
            );
            gpu.queue.submit([encoder.finish()]);
        }
        post.passes = passes;
        let read = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Original shadow dispatch counter"),
            size: 4,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(
            post.buffers.indirect_source(0, 12).unwrap(),
            12,
            &read,
            0,
            4,
        );
        gpu.queue.submit([encoder.finish()]);
        read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        let bytes = read.slice(..).get_mapped_range().unwrap();
        assert_eq!(
            u32::from_le_bytes(bytes[..4].try_into().unwrap()),
            expected,
            "indirect={indirect}"
        );
    }
}

/// Run explicitly with HORIZON_SHADER_PACK_WITHOUT=indirect as a separate adapter control.
#[test]
#[ignore = "requires an explicit capability mask"]
fn an_unsupported_indirect_device_keeps_the_direct_and_render_passes() {
    assert_eq!(
        std::env::var("HORIZON_SHADER_PACK_WITHOUT").as_deref(),
        Ok("indirect")
    );
    let Some(gpu) = Fixture::new() else {
        panic!("adapter required for capability control")
    };
    let post = pack(
        "bufferObject.0=16\nindirect.composite=0 0\n",
        &[
            (
                "composite.csh",
                "#version 430\nlayout(local_size_x=1) in;\nvoid main(){}\n",
            ),
            (
                "composite_a.csh",
                "#version 430\nlayout(local_size_x=1) in;\nconst ivec3 workGroups=ivec3(1);\nlayout(rgba8) writeonly uniform image2D colorimg1;\nvoid main(){imageStore(colorimg1,ivec2(0),vec4(0,1,0,1));}\n",
            ),
        ],
        "#version 430 compatibility\nuniform sampler2D colortex1;\nvoid main(){gl_FragColor=texelFetch(colortex1,ivec2(0),0);}\n",
    );
    let mut post = gpu
        .try_build(post)
        .expect("direct compute/final remain supported");
    assert!(
        post.notes
            .iter()
            .any(|n| n.contains("cannot dispatch indirect compute")),
        "{:?}",
        post.notes
    );
    assert_eq!(gpu.render(&mut post, [8, 8])[0], [0, 255, 0, 255]);
}
