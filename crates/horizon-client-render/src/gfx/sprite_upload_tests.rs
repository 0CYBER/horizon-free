use super::*;
#[test]
fn real_rail_mips_do_not_change_with_larger_array_layers() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/horizon-client")
        .join(horizon_core::VANILLA_VERSION);
    if !root
        .join("assets/minecraft/textures/block/rail.png")
        .exists()
    {
        return;
    }
    let mut sprites = crate::sprites::Sprites::new(
        crate::resource_source::ResourceSource::directory(root.clone()),
    );
    let layers: Vec<_> = [
        "rail",
        "powered_rail",
        "powered_rail_on",
        "detector_rail",
        "activator_rail",
    ]
    .map(|name| sprites.insert(&format!("minecraft:block/{name}")).unwrap())
    .into();
    sprites.finish();
    assert_eq!(sprites.size, 16);
    let native: Vec<_> = layers
        .iter()
        .map(|&layer| {
            let start = layer as usize * 16 * 16 * 4;
            mip_chain(
                &sprites.pixels[start..start + 16 * 16 * 4],
                16,
                &sprites.mips,
                layer,
                1,
            )
            .0
        })
        .collect();
    sprites.insert("minecraft:block/oak_sign").unwrap();
    sprites.finish();
    assert_eq!(sprites.size, 32);
    for (&layer, expected) in layers.iter().zip(native) {
        let start = layer as usize * 32 * 32 * 4;
        let (actual, _) = mip_chain(
            &sprites.pixels[start..start + 32 * 32 * 4],
            32,
            &sprites.mips,
            layer,
            1,
        );
        assert_eq!(&actual[32 * 32 * 4..], expected, "rail layer {layer}");
    }
}
#[test]
fn prepared_texture_uploads_preserve_layer_order_and_settings() {
    let size = 4;
    let layers = 5;
    let pixels: Vec<u8> = (0..size * size * layers)
        .flat_map(|i| [i as u8, (i * 3) as u8, 99, if i % 3 == 0 { 0 } else { 255 }])
        .collect();
    let settings: Vec<_> = (0..layers)
        .map(|i| crate::mipmap::Settings {
            strategy: if i % 2 == 0 {
                crate::mipmap::Strategy::Mean
            } else {
                crate::mipmap::Strategy::StrictCutout
            },
            ..Default::default()
        })
        .collect();
    let border = [20, 30, 40, 128].repeat(16);
    let expected = mip_chain(&pixels, size, &settings, 0, layers);
    // A partial final array and an exact boundary exercise the global
    // layer offset used to select each sprite's mipmap settings.
    for split in [1, 2, 5, 8] {
        let prepared =
            PreparedTextures::with_split(&pixels, size, layers, &settings, (&border, size), split);
        assert_eq!(prepared.sprites.len(), layers.div_ceil(split) as usize);
        let actual: Vec<_> = prepared
            .sprites
            .iter()
            .flat_map(|(bytes, levels)| {
                assert_eq!(*levels, expected.1);
                bytes.iter().copied()
            })
            .collect();
        assert_eq!(actual, expected.0);
        assert_eq!(prepared.forcefield, mip_chain(&border, size, &[], 0, 1));
        // Chains cut before the adapter was known, cut again for it.
        for to in [1, 2, 3, 5, 8] {
            let direct =
                PreparedTextures::with_split(&pixels, size, layers, &settings, (&border, size), to);
            let resplit = PreparedTextures::with_split(
                &pixels,
                size,
                layers,
                &settings,
                (&border, size),
                split,
            )
            .resplit(layers, to);
            assert_eq!(resplit.split, to);
            assert_eq!(resplit.sprites, direct.sprites, "{split} -> {to}");
        }
    }
}

/// The chains are made a chunk of layers at a time, each on its own thread. Their
/// bytes must be the ones one thread making every layer in turn gives, in layer
/// order, whichever layer a chunk starts at.
#[test]
fn mip_chains_match_one_thread_making_every_layer() {
    let size: u32 = 8;
    let layers: u32 = 40;
    let pixels: Vec<u8> = (0..size * size * layers)
        .flat_map(|i| [i as u8, (i * 7) as u8, 33, if i % 5 == 0 { 0 } else { 255 }])
        .collect();
    let settings: Vec<_> = (0..layers)
        .map(|i| crate::mipmap::Settings {
            strategy: match i % 3 {
                0 => crate::mipmap::Strategy::Mean,
                1 => crate::mipmap::Strategy::Cutout,
                _ => crate::mipmap::Strategy::DarkCutout,
            },
            alpha_cutoff_bias: (i % 4) as f32 / 4.0,
            item: i % 7 == 0,
            ..Default::default()
        })
        .collect();
    let layer_bytes = (size * size * 4) as usize;
    let levels = size.trailing_zeros();
    let chain_bytes: usize = (0..=levels).map(|l| layer_bytes >> (2 * l)).sum();
    let mut expected: Vec<u8> = Vec::new();
    for (layer, s) in settings.iter().copied().enumerate() {
        let base = crate::mipmap::Image {
            width: size,
            height: size,
            pixels: pixels[layer * layer_bytes..(layer + 1) * layer_bytes]
                .as_chunks::<4>()
                .0
                .to_vec(),
        };
        let has_transparent = s.has_transparent || base.pixels.iter().any(|p| p[3] == 0);
        for image in crate::mipmap::generate(
            base,
            levels,
            crate::mipmap::Settings {
                has_transparent,
                ..s
            },
        ) {
            expected.extend(image.pixels.iter().flatten());
        }
    }
    assert_eq!(expected.len(), chain_bytes * layers as usize);
    assert_eq!(mip_chain(&pixels, size, &settings, 0, layers).0, expected);
    // From the middle: the chunk's settings still come from the global layer index.
    let from = (layers / 2) as usize;
    let chain = mip_chain(
        &pixels[from * layer_bytes..],
        size,
        &settings,
        from as u32,
        layers / 2,
    );
    assert_eq!(chain.0, expected[from * chain_bytes..]);
}

#[test]
fn sprite_split_follows_the_adapters_layer_limit() {
    // The Discord report: 512 layers per array, 1291 block textures (3 arrays).
    assert_eq!(layer_split(512, false, None, 1291), Ok(512));
    assert_eq!(layer_split(512, true, None, 1291), Ok(512));
    assert_eq!(layer_split(2048, false, None, 1291), Ok(SPRITE_SPLIT));
    assert_eq!(layer_split(8192, false, None, 1291), Ok(SPRITE_SPLIT));
    // HORIZON_SPRITE_SPLIT only lowers it.
    assert_eq!(layer_split(2048, false, Some(512), 1291), Ok(512));
    assert_eq!(layer_split(256, false, Some(512), 1291), Ok(256));
    // OpenGL pads a multiple of 6 by a layer: a full array must still fit.
    assert_eq!(layer_split(600, true, None, 1291), Ok(599));
    assert_eq!(layer_split(600, false, None, 1291), Ok(600));
    assert_eq!(array_layers(599, true), 599);
    // Past the arrays terrain.wgsl binds.
    assert_eq!(layer_split(256, false, None, 256 * SPRITE_ARRAYS), Ok(256));
    let err = layer_split(128, false, None, 1291).unwrap_err();
    assert!(err.contains("128 texture layers per array"), "{err}");
    assert!(layer_split(512, false, None, 512 * SPRITE_ARRAYS + 1).is_err());
}

#[test]
fn metal_uses_a_conservative_default_split_but_respects_overrides() {
    let mut info = wgpu::AdapterInfo {
        name: "test adapter".into(),
        vendor: 0,
        device: 0,
        device_type: wgpu::DeviceType::IntegratedGpu,
        device_pci_bus_id: String::new(),
        driver: String::new(),
        driver_info: String::new(),
        backend: wgpu::Backend::Vulkan,
        subgroup_min_size: 0,
        subgroup_max_size: 0,
        transient_saves_memory: None,
        limit_bucket: None,
    };
    info.backend = wgpu::Backend::Metal;
    assert_eq!(adapter_requested_split(&info, None, 1200), Some(512));
    assert_eq!(
        adapter_requested_split(&info, None, 512 * SPRITE_ARRAYS),
        Some(512)
    );
    assert_eq!(adapter_requested_split(&info, Some(1024), 1200), Some(1024));
    // More sprites than the arrays hold at 512: the arrays grow, up to the GPU's layers.
    let split = adapter_requested_split(&info, None, 10_271);
    assert_eq!(split, Some(1284));
    assert_eq!(layer_split(2048, false, split, 10_271), Ok(1284));
    let split = adapter_requested_split(&info, None, 2048 * SPRITE_ARRAYS + 1);
    assert!(layer_split(2048, false, split, 2048 * SPRITE_ARRAYS + 1).is_err());

    info.backend = wgpu::Backend::Vulkan;
    assert_eq!(adapter_requested_split(&info, None, 10_271), None);
}

#[test]
fn broken_haswell_vulkan_detects_haswell_and_leaves_ivy_bridge() {
    let mut info = wgpu::AdapterInfo {
        name: "Intel(R) HD Graphics 4400 (HSW GT2)".into(),
        vendor: 0x8086,
        device: 0x0a16,
        device_type: wgpu::DeviceType::IntegratedGpu,
        device_pci_bus_id: String::new(),
        driver: "Mesa 26.2.5".into(),
        driver_info: "Mesa 26.2.5".into(),
        backend: wgpu::Backend::Vulkan,
        subgroup_min_size: 0,
        subgroup_max_size: 0,
        transient_saves_memory: None,
        limit_bucket: None,
    };
    assert!(is_broken_haswell_vulkan(&info));

    // Ivy Bridge keeps Vulkan (#4059)
    info.name = "Intel(R) HD Graphics 4000 (IVB GT2)".into();
    info.device = 0x0166;
    assert!(!is_broken_haswell_vulkan(&info));

    // Haswell on OpenGL is stable (crocus)
    info.name = "Intel(R) HD Graphics 4400 (HSW GT2)".into();
    info.device = 0x0a16;
    info.backend = wgpu::Backend::Gl;
    assert!(!is_broken_haswell_vulkan(&info));
}

#[test]
fn mip_chain_allocates_exactly_its_size() {
    for size in [16, 64] {
        let animation = crate::sprites::test_animation(size, 0);
        let (chain, _) = mip_chain(animation.pixels(0), size, &[], 0, 1);
        assert_eq!(chain.capacity(), chain.len());
    }
}

#[test]
fn collect_animated_layers_masks_opaque_layer() {
    use crate::mesher::{self, SectionMesh, TerrainVertex};
    let opaque_vertex =
        TerrainVertex::new([0.0, 0.0, 0.0], [0.0, 0.0], 7 | mesher::OPAQUE_LAYER, 0, 0);
    let translucent_vertex = TerrainVertex::new([0.0, 0.0, 0.0], [0.0, 0.0], 12, 0, 0);
    let mut mesh = SectionMesh {
        solid: vec![opaque_vertex; 4],
        translucent: vec![translucent_vertex; 4],
        ..Default::default()
    };
    mesh.note_sprite_layer(7 | mesher::OPAQUE_LAYER);
    mesh.note_sprite_layer(12);
    mesh.finish_sprite_layers();

    let animated = super::collect_animated_layers(&mesh, |layer| layer == 7 || layer == 12);
    assert_eq!(&*animated, &[7, 12]);
}
