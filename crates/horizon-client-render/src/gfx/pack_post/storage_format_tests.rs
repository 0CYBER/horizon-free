//! A storage image whose `layout(...)` qualifier names another format with its texture's channels
//! and value class binds the texture's format: IterationRP's bloom declares `layout(rgba16)` over
//! colortex8 made `RGBA16F`, and its four bloom passes were refused (#14889). Original fixture, no
//! pack files.
use super::depth_test::{Gpu, SIZE, VERTEX, fixture_frame};
use super::*;
use crate::hud::HudBatch;
use wgpu::TextureFormat as T;

/// Only a qualifier that differs from its texture in being normalized rather than float binds it;
/// runs without a GPU, unlike the stores below and `shadow_image_tests`.
#[test]
fn a_qualifier_binds_another_texture_only_with_its_channels_width_and_value_class() {
    for (qualifier, texture, binds) in [
        ("rgba16", T::Rgba16Float, true),
        ("rgba16_snorm", T::Rgba16Float, true),
        ("rg16", T::Rg16Float, true),
        ("r11f_g11f_b10f", T::Rgba16Float, true),
        // Another width: main's shadowcolor0 case (#16900).
        ("rgba16f", T::Rgba8Unorm, false),
        ("rgba8", T::Rgba16Float, false),
        // Packed: the same texel size, other channel widths.
        ("rgb10_a2", T::Rgba8Unorm, false),
        ("rgba8", T::Rgb10a2Unorm, false),
        // Integer against float.
        ("rgba16i", T::Rgba16Float, false),
        ("rgba16f", T::Rgba16Uint, false),
        // Another channel count.
        ("rg16", T::Rgba16Float, false),
        ("rgba16", T::R16Float, false),
    ] {
        let declared = image_formats::image_format(qualifier).unwrap();
        assert_eq!(
            image_formats::binds_as(declared, texture),
            binds,
            "{qualifier} over {texture:?}"
        );
    }
}

#[test]
fn an_rgba16_image_over_an_rgba16f_texture_is_stored_into() {
    let Some(gpu) = Gpu::new() else { return };
    let files = vec![
        (
            "shaders.properties".into(),
            "image.imgA = texA RGBA RGBA16F HALF_FLOAT false false 4 4\n".into(),
        ),
        (
            "composite.csh".into(),
            "#version 430\nlayout(local_size_x = 1) in;\nconst ivec3 workGroups = ivec3(1, 1, 1);\n\
             layout(rgba16) writeonly uniform image2D imgA;\n\
             void main() { imageStore(imgA, ivec2(1, 1), vec4(0.25, 0.5, 0.75, 1.0)); }\n"
                .into(),
        ),
        ("final.vsh".into(), VERTEX.into()),
        (
            "final.fsh".into(),
            "#version 130\nuniform sampler2D texA;\nvarying vec2 uv;\n\
             void main() { gl_FragColor = vec4(texelFetch(texA, ivec2(1, 1), 0).rgb, 1.0); }\n"
                .into(),
        ),
    ];
    let color = gpu.texture(T::Rgba8Unorm);
    let depth = gpu.texture(T::Depth32Float);
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
        horizon_shaders::GameInputs::default(),
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
    let pixels = gpu.render(&mut post, &color, &depth, &frame, &mut |_, _, _, _| {});
    // (0.25, 0.5, 0.75) in eight bits.
    let p = pixels[0];
    assert!(
        p[0].abs_diff(64) <= 1 && p[1].abs_diff(128) <= 1 && p[2].abs_diff(191) <= 1,
        "{p:?}, notes {:?}",
        post.notes
    );
}
