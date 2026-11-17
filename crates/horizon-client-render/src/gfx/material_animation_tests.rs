use super::material_animate::MaterialAnimations;
use crate::sprites::test_material_frames;

#[test]
fn animated_materials_keep_the_initial_upload_mip_semantics() {
    // Each 2x2 cell has mean 112, including alpha (native displacement height).
    let varied: Vec<u8> = (0..16)
        .flat_map(|i| {
            let v = [0, 64, 128, 255][(i / 4 % 2) * 2 + i % 2];
            [v, v, v, v]
        })
        .collect();
    let still = [8, 16, 24, 32].repeat(16);
    let a = test_material_frames(
        vec![vec![0; 64]; 2],
        r#"{"frames":[0,1]}"#,
        [
            Some(vec![still.clone(), varied.clone()]),
            Some(vec![still, varied.clone()]),
        ],
    );
    let mut expected = varied.clone();
    expected.extend([112; 16]);
    expected.extend([112; 4]);
    let prepared = super::PreparedTextures::new(&[0; 64], 4, 1, &[], (&[0; 64], 4))
        .with_pbr(&varied, &varied, 4, 1);
    let initial = prepared.pbr.unwrap();
    assert_eq!(initial.0, expected, "initial normal upload is the control");
    assert_eq!(
        initial.1, expected,
        "initial specular upload is the control"
    );
    let mut animator = MaterialAnimations::new(std::slice::from_ref(&a));
    let mut written = [Vec::new(), Vec::new()];
    let dirty = animator.animate([(0, 1)], 4, 3, |layer, kind, _, _, pixels| {
        assert_eq!(layer, 0);
        written[kind].extend_from_slice(pixels);
    });
    assert_eq!(dirty, [vec![0], vec![0]]);
    assert_eq!(
        written[0], expected,
        "animated normal/height mips match initial upload"
    );
    assert_eq!(
        written[1], expected,
        "animated data mips match initial upload"
    );
    assert_uploaded_mips(&a, &expected);
}

#[test]
fn material_uploads_skip_static_maps_and_unchanged_selected_frames() {
    let zero = [10, 20, 30, 40].repeat(4);
    let one = [50, 60, 70, 80].repeat(4);
    let a = test_material_frames(
        vec![vec![0; 16]; 2],
        r#"{"frames":[1,0],"frametime":4,"interpolate":true}"#,
        [
            Some(vec![zero.clone(), one.clone()]),
            Some(vec![[3, 4, 5, 6].repeat(4)]),
        ],
    );
    let mut animator = MaterialAnimations::new(&[a]);
    let mut writes = 0;
    let dirty = animator.animate([(0, 1)], 2, 2, |_, _, _, _, _| writes += 1);
    assert_eq!(
        writes, 0,
        "initial upload already holds declared first frame"
    );
    assert_eq!(dirty, [Vec::<u32>::new(), Vec::<u32>::new()]);
    // A mode may select frame 0 while the clip clock still selects frame 1.
    let mut written = Vec::new();
    let dirty = animator.animate([(0, 0)], 2, 2, |_, kind, _, _, pixels| {
        assert_eq!(kind, 0, "single-frame specular companion stays untouched");
        written.extend_from_slice(pixels);
    });
    let mut expected = zero;
    expected.extend([10, 20, 30, 40]);
    assert_eq!(written, expected);
    assert_eq!(dirty, [vec![0], vec![]]);
    let dirty = animator.animate([(0, 0)], 2, 2, |_, _, _, _, _| {
        panic!("interpolation does not change categorical material frames")
    });
    assert_eq!(dirty, [Vec::<u32>::new(), Vec::<u32>::new()]);
    // An inactive colour animation contributes no selected layer.
    let dirty = animator.animate([], 2, 2, |_, _, _, _, _| panic!("inactive"));
    assert_eq!(dirty, [Vec::<u32>::new(), Vec::<u32>::new()]);
    let mut written = Vec::new();
    animator.animate([(0, 1)], 2, 2, |_, kind, _, _, pixels| {
        assert_eq!(kind, 0);
        written.extend_from_slice(pixels);
    });
    let mut expected = one;
    expected.extend([50, 60, 70, 80]);
    assert_eq!(written, expected, "reactivation uses selected frame");
}

// Exercise the same upload callback against a real texture, without a window or game.
fn assert_uploaded_mips(animation: &crate::sprites::SpriteAnimation, expected: &[u8]) {
    let instance = super::test_gpu::instance();
    let adapter = match pollster::block_on(instance.request_adapter(&Default::default())) {
        Ok(adapter) => adapter,
        // No adapter: CI's Linux runners ask for DirectX 12 so that GPU tests find none.
        Err(error) => {
            eprintln!("material mip readback setup skipped: no adapter ({error})");
            return;
        }
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    let maps: [_; 2] = std::array::from_fn(|_| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("original material animation control"),
            size: wgpu::Extent3d {
                width: 4,
                height: 4,
                depth_or_array_layers: 1,
            },
            mip_level_count: 3,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    });
    let mut animator = MaterialAnimations::new(std::slice::from_ref(animation));
    let mut uploads = super::upload::Uploads::new(&device);
    let mut encoder = device.create_command_encoder(&Default::default());
    animator.animate([(0, 1)], 4, 3, |layer, kind, level, side, pixels| {
        uploads.texture(
            &mut encoder,
            wgpu::TexelCopyTextureInfo {
                texture: &maps[kind],
                mip_level: level,
                origin: wgpu::Origin3d {
                    x: 0,
                    y: 0,
                    z: layer,
                },
                aspect: wgpu::TextureAspect::All,
            },
            side,
            side,
            pixels,
        );
    });
    uploads.finish(&encoder);
    queue.submit([encoder.finish()]);
    for map in &maps {
        let mut actual = Vec::new();
        for level in 0..3 {
            let side = 4 >> level;
            let stride = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("original material mip readback"),
                size: u64::from(stride * side),
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: map,
                    mip_level: level,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(stride),
                        rows_per_image: Some(side),
                    },
                },
                wgpu::Extent3d {
                    width: side,
                    height: side,
                    depth_or_array_layers: 1,
                },
            );
            queue.submit([encoder.finish()]);
            buffer
                .slice(..)
                .map_async(wgpu::MapMode::Read, |result| result.unwrap());
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            let bytes = buffer.slice(..).get_mapped_range().unwrap();
            for row in bytes.chunks_exact(stride as usize) {
                actual.extend_from_slice(&row[..(side * 4) as usize]);
            }
        }
        assert_eq!(actual, expected, "uploaded data/height bytes at every mip");
    }
}
