//! What a pack's frame draws into: its images and storage buffers, its colour and depth buffers,
//! and `final`'s picture. A frame makes them where they are missing or made for another render
//! size (a window resized, the resolution scale), which is when video memory, full of terrain,
//! may have no room for them. Each is made as the frame's own targets are
//! (`frame_targets::made`, #13624): what failed is dropped, never kept, and the frame is given
//! up and tried again. A pack that finds no room frame after frame is turned off
//! (`out_of_room`) well before the frames given up would end the game.

use horizon_shaders::{BUFFERS, Group};

use super::{FinalTarget, Mips, PackPost, SINK, Targets};
use crate::gfx::frame_targets::{self, Kind, made};
use crate::gfx::pack_images;

/// Frames in a row a pack may give up for want of video memory before it is turned off
/// (`PackPost::out_of_room`): a quarter of what ends the game (`GIVEN_UP_LIMIT`, #13624), so
/// the frames after it have the time to show that the game draws without the pack, and several
/// times the frames the renderer takes to give back what it let go (`arena::GRAVE_FRAMES`), so
/// a pack short of room only while that lands keeps running. Half a second at 60 frames a
/// second.
pub(super) const OUT_OF_ROOM_FRAMES: u32 = 30;

impl PackPost {
    /// Makes what a frame at `size` in a window of `window` pixels draws into (`resize`,
    /// `fit_final`), and counts the frames in a row without the room for it.
    pub(super) fn fit(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        size: [u32; 2],
        window: [u32; 2],
    ) -> bool {
        let fits = self.resize(device, queue, size) && self.fit_final(device, size, window);
        self.given_up = if fits { 0 } else { self.given_up + 1 };
        fits
    }

    /// Video memory has had no room for what the pack draws into for `OUT_OF_ROOM_FRAMES`
    /// frames in a row: none is coming, and the game runs on only without the pack.
    pub(in crate::gfx) fn out_of_room(&self) -> bool {
        self.given_up >= OUT_OF_ROOM_FRAMES
    }

    /// Makes what a frame at `size` draws into, where it is missing or made for another render
    /// size. `false` where video memory had no room for some of it: that is dropped, and made
    /// again on the next frame.
    pub(super) fn resize(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        size: [u32; 2],
    ) -> bool {
        // Free what another render size makes again before making any of it: a pack with large
        // buffers can't hold both sets at once on a card it otherwise fits (a window resize ran
        // out of video memory), and an image or a storage buffer of the render size made while
        // the old targets were still there met nearly everything twice. The bind groups hold
        // the old views too, and the GPU may still be using them, so wait for it; a resize is
        // rare.
        // `final`'s picture of another size too, which `fit_final` would only drop after these
        // are made.
        let images = pack_images::release_resized(&mut self.images, size);
        let buffers = self.buffers.release_resized(size);
        let targets = self.targets.take_if(|t| t.size != size).is_some();
        let picture = self.final_target.take_if(|t| t.size != size).is_some();
        if images || buffers || targets || picture {
            self.forget_groups();
            let _ = device.poll(wgpu::PollType::wait_indefinitely());
        }
        let missing = self.missing();
        let images = pack_images::resize_images(device, &mut self.images, size);
        if images != Some(false) {
            self.forget_groups();
        }
        let buffers = images.is_some() && self.buffers.resize(device, queue, size);
        if frame_targets::screenshot_frame() {
            let made = self.missing();
            self.unchecked.0 |= missing.0 & !made.0;
            self.unchecked.1 |= missing.1 && !made.1;
        }
        if !buffers {
            return false;
        }
        if self.targets.as_ref().is_some_and(|t| t.size == size) {
            return true;
        }
        self.setup_pending = true;
        self.rgb_unset = true;
        self.targets = made(device, Kind::Pack, || self.targets(device, size));
        if self.targets.is_none() {
            return false;
        }
        self.water_mask_dirty = true;
        self.forget_groups();
        for pass in self.passes.iter().filter(|p| self.sizes_differ(p)) {
            log::info!(
                "shader pack: {}: writes buffers of different sizes at {}x{} (size.buffer); skipped",
                pass.name,
                size[0],
                size[1]
            );
        }
        true
    }

    /// The pack's buffers at `size`.
    fn targets(&self, device: &wgpu::Device, size: [u32; 2]) -> Targets {
        let sizes: [[u32; 2]; BUFFERS] =
            std::array::from_fn(|i| self.sizes[i].map_or(size, |s| s.resolve(size)));
        let full_chain = |size: [u32; 2]| 32 - size[0].max(size[1]).max(1).leading_zeros();
        let texture_sized = |label: &str, format, usage, levels, size: [u32; 2]| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: levels,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: usage | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
        };
        let texture = |label: &str, format, usage| texture_sized(label, format, usage, 1, size);
        let level = |t: &wgpu::Texture, level: u32| {
            t.create_view(&wgpu::TextureViewDescriptor {
                base_mip_level: level,
                mip_level_count: Some(1),
                ..Default::default()
            })
        };
        let mut mips: [Option<[Mips; 2]>; BUFFERS] = Default::default();
        let color = std::array::from_fn(|i| {
            (self.used & 1 << i != 0).then(|| {
                let label = format!("colortex{i}");
                let mipped = self.mipped & 1 << i != 0;
                let levels = if mipped { full_chain(sizes[i]) } else { 1 };
                let mut attachment = wgpu::TextureUsages::RENDER_ATTACHMENT;
                if self.storage & 1 << i != 0 {
                    attachment |= wgpu::TextureUsages::STORAGE_BINDING;
                    // Atomics on a texture need it made for them (Metal ignores them otherwise).
                    if device.features().contains(wgpu::Features::TEXTURE_ATOMIC)
                        && matches!(
                            self.formats[i],
                            wgpu::TextureFormat::R32Sint | wgpu::TextureFormat::R32Uint
                        )
                    {
                        attachment |= wgpu::TextureUsages::STORAGE_ATOMIC;
                    }
                }
                if self.copied & 1 << i != 0 {
                    attachment |= wgpu::TextureUsages::COPY_SRC;
                }
                // A buffer that isn't cleared starts each frame with B copied back to A.
                if i != 0 && self.clears[i].is_none() {
                    attachment |= wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST;
                }
                let side = || texture_sized(&label, self.formats[i], attachment, levels, sizes[i]);
                let a = side();
                let sides = [
                    a.clone(),
                    if self.doubled & 1 << i != 0 {
                        side()
                    } else {
                        a
                    },
                ];
                if mipped && let Some((_, layout)) = self.down.get(&self.formats[i]) {
                    mips[i] = Some(sides.each_ref().map(|t| {
                        Mips {
                            all: t.create_view(&Default::default()),
                            levels: (1..levels)
                                .map(|l| {
                                    let group =
                                        device.create_bind_group(&wgpu::BindGroupDescriptor {
                                            label: Some(&label),
                                            layout,
                                            entries: &[
                                                wgpu::BindGroupEntry {
                                                    binding: 0,
                                                    resource: wgpu::BindingResource::TextureView(
                                                        &level(t, l - 1),
                                                    ),
                                                },
                                                wgpu::BindGroupEntry {
                                                    binding: 2,
                                                    resource: wgpu::BindingResource::Sampler(
                                                        &self.linear,
                                                    ),
                                                },
                                            ],
                                        });
                                    (level(t, l), group)
                                })
                                .collect(),
                        }
                    }));
                }
                sides.map(|t| level(&t, 0))
            })
        });
        let depth32 = wgpu::TextureFormat::Depth32Float;
        let depth_texture = texture(
            "depthtex0",
            depth32,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let depth1 = self.depth1.then(|| {
            let t = texture("depthtex1", depth32, wgpu::TextureUsages::COPY_DST);
            let view = t.create_view(&Default::default());
            (t, view)
        });
        let depth2 = self.depth2.then(|| {
            let t = texture("depthtex2", depth32, wgpu::TextureUsages::COPY_DST);
            let view = t.create_view(&Default::default());
            (t, view)
        });
        let water_mask = self.water_mask.then(|| {
            let t = texture(
                "Shader pack water patches",
                depth32,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
            );
            let view = t.create_view(&Default::default());
            (t, view)
        });
        let scratch = std::array::from_fn(|i| {
            (self.copied & 1 << i != 0).then(|| {
                // A copy also read as an image before a dispatch.
                let image = if self.storage & 1 << i != 0 {
                    wgpu::TextureUsages::STORAGE_BINDING
                } else {
                    wgpu::TextureUsages::empty()
                };
                texture_sized(
                    &format!("colortex{i} copy"),
                    self.formats[i],
                    wgpu::TextureUsages::COPY_DST | image,
                    1,
                    sizes[i],
                )
                .create_view(&Default::default())
            })
        });
        let sink = texture(
            "Shader pack sink",
            SINK,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        )
        .create_view(&Default::default());
        Targets {
            size,
            sink,
            sizes,
            scratch,
            color,
            depth: depth_texture.create_view(&Default::default()),
            depth_texture,
            depth1,
            depth2,
            water_mask,
            mips,
        }
    }

    /// The images without textures (bit n: image n), and whether the storage buffers are.
    fn missing(&self) -> (u64, bool) {
        let images = self.images.iter().enumerate();
        let images = images.fold(0, |m, (i, image)| m | u64::from(image.view.is_none()) << i);
        (images, self.buffers.group.is_none())
    }

    /// A scaled screenshot's frame is checked: where its scopes found no room (`failed`), the
    /// images and storage buffers it made, which may be invalid, are dropped for the next frame
    /// to make again. What depends on the render size is made again at the window's size anyway,
    /// but not an image or buffer of a size of its own, which would stay invalid.
    pub(in crate::gfx) fn screenshot_checked(&mut self, failed: bool) {
        let (images, buffers) = std::mem::take(&mut self.unchecked);
        if !failed || (images == 0 && !buffers) {
            return;
        }
        for (i, image) in self.images.iter_mut().enumerate() {
            if images & 1 << i != 0 {
                image.reset_history();
            }
        }
        if buffers {
            self.buffers.reset_history();
        }
        self.forget_groups();
    }

    /// Makes the picture a pack's `final` draws at `size` while the window is another, where it
    /// is missing or of another size; `false` where video memory had no room for it.
    pub(super) fn fit_final(
        &mut self,
        device: &wgpu::Device,
        size: [u32; 2],
        window: [u32; 2],
    ) -> bool {
        // A resolution scale leaves the picture smaller (or larger) than the window: the pack's
        // `final` draws it at the picture's size, and one copy takes it to the window (#9291).
        if size == window || !self.passes.iter().any(|p| p.group == Group::Final) {
            self.final_target = None;
            return true;
        }
        if self.final_target.as_ref().is_none_or(|t| t.size != size) {
            self.final_target = made(device, Kind::Pack, || {
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("Shader pack final"),
                    size: wgpu::Extent3d {
                        width: size[0],
                        height: size[1],
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: self.screen,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                });
                FinalTarget {
                    size,
                    view: texture.create_view(&Default::default()),
                }
            });
        }
        self.final_target.is_some()
    }
}
