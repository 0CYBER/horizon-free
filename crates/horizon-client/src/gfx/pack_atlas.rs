//! The block atlas of shader packs (spec 5.1, `atlasSize` in 5.2): packs sample one 2D atlas
//! with atlas coordinates, Horizon keeps block sprites in texture arrays. With a pack, every
//! layer is copied into a grid, row by row from the top left, and an animated layer again
//! whenever its frame changes.

#[expect(dead_code, reason = "the gbuffers pass binds it (next slice)")]
pub(super) struct PackAtlas {
    texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    /// Sprites per row.
    pub columns: u32,
    /// A sprite's edge in pixels, at the first level.
    pub sprite: u32,
    /// Layers per sprite array.
    split: u32,
}

impl PackAtlas {
    /// Copies `layers` layers of `arrays` (`split` per array, `sprite` pixels wide, `levels`
    /// mip levels); `None` when the grid is larger than the device allows.
    pub fn new(
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        arrays: &[wgpu::Texture],
        split: u32,
        layers: u32,
        sprite: u32,
    ) -> Option<Self> {
        let layers = layers.max(1);
        let columns = (f64::from(layers).sqrt().ceil() as u32).max(1);
        let rows = layers.div_ceil(columns);
        let max = device.limits().max_texture_dimension_2d;
        if columns * sprite > max || rows * sprite > max {
            return None;
        }
        // Levels past a sprite's single pixel would mix neighbours: stop there.
        let levels = arrays[0].mip_level_count().min(sprite.ilog2() + 1);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Shader pack block atlas"),
            size: wgpu::Extent3d {
                width: columns * sprite,
                height: rows * sprite,
                depth_or_array_layers: 1,
            },
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let atlas = Self {
            texture,
            view,
            columns,
            sprite,
            split,
        };
        atlas.copy(encoder, arrays, 0..layers);
        Some(atlas)
    }

    /// The atlas's size in pixels (`atlasSize`).
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "the gbuffers pass reads it (next slice)")
    )]
    pub fn size(&self) -> [u32; 2] {
        [self.texture.width(), self.texture.height()]
    }

    /// Copies `layers` from the sprite arrays again, every level.
    pub fn copy(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        arrays: &[wgpu::Texture],
        layers: impl IntoIterator<Item = u32>,
    ) {
        for layer in layers {
            let Some(array) = arrays.get((layer / self.split) as usize) else {
                continue;
            };
            let (column, row) = (layer % self.columns, layer / self.columns);
            for level in 0..self.texture.mip_level_count() {
                let edge = self.sprite >> level;
                encoder.copy_texture_to_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: array,
                        mip_level: level,
                        origin: wgpu::Origin3d {
                            x: 0,
                            y: 0,
                            z: layer % self.split,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::TexelCopyTextureInfo {
                        texture: &self.texture,
                        mip_level: level,
                        origin: wgpu::Origin3d {
                            x: column * edge,
                            y: row * edge,
                            z: 0,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::Extent3d {
                        width: edge,
                        height: edge,
                        depth_or_array_layers: 1,
                    },
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two arrays of 2x2 sprites, one layer split; each layer a colour of its own.
    #[test]
    fn layers_land_in_their_cells() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
            return;
        };
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default())).unwrap();
        let (split, layers, sprite) = (3u32, 5u32, 2u32);
        let arrays: Vec<wgpu::Texture> = (0..2)
            .map(|a| {
                let count = (layers - a * split).min(split);
                let pixels: Vec<u8> = (0..count)
                    .flat_map(|l| [(a * split + l) as u8 * 40, 0, 0, 255].repeat(4))
                    .collect();
                wgpu::util::DeviceExt::create_texture_with_data(
                    &device,
                    &queue,
                    &wgpu::TextureDescriptor {
                        label: None,
                        size: wgpu::Extent3d {
                            width: sprite,
                            height: sprite,
                            depth_or_array_layers: count,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
                        view_formats: &[],
                    },
                    wgpu::util::TextureDataOrder::LayerMajor,
                    &pixels,
                )
            })
            .collect();
        let mut encoder = device.create_command_encoder(&Default::default());
        let atlas = PackAtlas::new(&device, &mut encoder, &arrays, split, layers, sprite).unwrap();
        assert_eq!((atlas.columns, atlas.size()), (3, [6, 4]));
        let read = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 256 * 4,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            atlas.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &read,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width: 6,
                height: 4,
                depth_or_array_layers: 1,
            },
        );
        queue.submit([encoder.finish()]);
        read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let data = read.slice(..).get_mapped_range().unwrap();
        let red = |x: usize, y: usize| data[y * 256 + x * 4];
        // Layer 4 (second array, its second layer) is in row 1, column 1.
        assert_eq!([red(0, 0), red(2, 0), red(4, 0)], [0, 40, 80]);
        assert_eq!([red(1, 3), red(3, 2), red(5, 2)], [120, 160, 0]);
    }
}
