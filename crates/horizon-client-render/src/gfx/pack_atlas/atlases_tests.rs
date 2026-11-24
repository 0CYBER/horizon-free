//! `frame_targets::NO_ROOM` stands in for a device out of video memory: a `made` that would
//! make something then reports no room.
use super::*;
use crate::gfx::frame_targets::NO_ROOM;

/// One array of three 4 px layers (sprites of 2, 4 and 1 px), or of 2 px material maps.
fn array(device: &wgpu::Device, queue: &wgpu::Queue, side: u32) -> wgpu::Texture {
    wgpu::util::DeviceExt::create_texture_with_data(
        device,
        queue,
        &wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: side,
                height: side,
                depth_or_array_layers: 3,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        &vec![255; (side * side * 3 * 4) as usize],
    )
}

struct Fixture {
    device: wgpu::Device,
    queue: wgpu::Queue,
    arrays: [wgpu::Texture; 1],
    maps: [[wgpu::Texture; 2]; 1],
}

impl Fixture {
    /// `None` without an adapter (a CI runner without a GPU), as the atlas's other tests.
    fn new() -> Option<Self> {
        let instance = crate::gfx::test_gpu::instance();
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default())).unwrap();
        let arrays = [array(&device, &queue, 4)];
        let maps = [[array(&device, &queue, 2), array(&device, &queue, 2)]];
        Some(Self {
            device,
            queue,
            arrays,
            maps,
        })
    }

    /// Flat draws' atlases at the Mipmap Levels option `mip_option`.
    fn ready(&self, atlases: &mut Atlases, mip_option: u32) -> bool {
        let sprites = Sprites {
            arrays: &self.arrays,
            split: 3,
            native: &[2, 4, 1],
            maps: Some(&self.maps),
            flat: true,
            mip_option,
            anisotropy_bit: 0,
        };
        atlases.ready(&self.device, &self.queue, &sprites)
    }
}

fn textures(atlases: &Atlases) -> Vec<wgpu::Texture> {
    let materials = atlases.materials.iter().flatten();
    (atlases.colour.iter().chain(materials))
        .map(|a| a.texture.clone())
        .collect()
}

/// The failure this guards against: a pack's option changed with video memory full, the pack
/// loaded again, and atlases made again beside the old ones found no room.
#[test]
fn a_pack_loaded_again_finds_its_atlases() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(gpu) = Fixture::new() else { return };
    let mut atlases = Atlases::default();
    assert!(gpu.ready(&mut atlases, 0));
    let made = textures(&atlases);
    assert_eq!(made.len(), 3);
    // No room for anything new: nothing new is made.
    NO_ROOM.set(1);
    assert!(gpu.ready(&mut atlases, 0));
    assert_eq!(NO_ROOM.replace(0), 1, "an atlas was made again");
    assert_eq!(textures(&atlases), made);
}

#[test]
fn atlases_without_room_are_not_kept_and_the_next_load_makes_them() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(gpu) = Fixture::new() else { return };
    let mut atlases = Atlases::default();
    NO_ROOM.set(1);
    assert!(!gpu.ready(&mut atlases, 0));
    assert_eq!(NO_ROOM.replace(0), 0, "nothing was made that could fail");
    assert!(textures(&atlases).is_empty());
    assert!(gpu.ready(&mut atlases, 0));
    let made = textures(&atlases);
    assert_eq!(made.len(), 3);
    // Another padding makes them again. The old ones are let go first, so without room for
    // the new ones none is left, and the next load starts from nothing.
    NO_ROOM.set(1);
    assert!(!gpu.ready(&mut atlases, 1));
    assert_eq!(NO_ROOM.replace(0), 0, "another padding kept the atlases");
    assert!(textures(&atlases).is_empty());
    assert!(gpu.ready(&mut atlases, 1));
    assert_eq!(textures(&atlases).len(), 3);
    assert!(textures(&atlases).iter().all(|t| !made.contains(t)));
}
