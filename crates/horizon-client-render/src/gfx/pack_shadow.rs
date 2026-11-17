//! A shader pack's shadow buffers (spec 6.3): the shadow map's depth (`shadowtex0`, and
//! `shadowtex1` without translucent geometry), and the colour buffers `shadowcolor0` to `7`, each
//! with two sides that `shadowcomp` programs flip as composite programs flip colortex (6.1.6).
//! Square, `shadowMapResolution` pixels.

use horizon_shaders::SHADOW_COLORS;

type Ops<T> = wgpu::Operations<T>;
type Color = wgpu::Color;

/// Loaded and stored.
pub(super) fn load<T>() -> Ops<T> {
    wgpu::Operations {
        load: wgpu::LoadOp::Load,
        store: wgpu::StoreOp::Store,
    }
}

fn clear(c: [f32; 4]) -> Ops<Color> {
    wgpu::Operations {
        load: wgpu::LoadOp::Clear(wgpu::Color {
            r: c[0].into(),
            g: c[1].into(),
            b: c[2].into(),
            a: c[3].into(),
        }),
        store: wgpu::StoreOp::Store,
    }
}

/// What the shadow pass writes and the programs read.
pub(super) struct ShadowTargets {
    pub size: u32,
    /// `shadowtex0`, written by the shadow pass.
    pub depth: (wgpu::Texture, wgpu::TextureView),
    /// `shadowtex1`: a copy of `shadowtex0` taken before translucent terrain, when read.
    pub depth1: Option<(wgpu::Texture, wgpu::TextureView)>,
    /// Both sides of each shadowcolor buffer used; one texture for both when no `shadowcomp`
    /// program draws into or flips the buffer.
    pub color: [Option<[wgpu::TextureView; 2]>; SHADOW_COLORS],
    /// A shadow-sized attachment for fragment programs that write only storage images.
    pub sink: Option<wgpu::TextureView>,
    /// Pre-pass copies used by samplers and lowered image loads sharing a storage image.
    pub scratch: [Option<wgpu::TextureView>; SHADOW_COLORS],
    /// Buffers accessed through storage; initialize them before any program can use them.
    storage: u16,
    /// The clear colour of each buffer cleared every frame.
    pub clears: [Option<[f32; 4]>; SHADOW_COLORS],
    /// Bit n set: B is shadowcolor n's "main".
    pub main: u16,
    /// The depth/ordinary attachments have not begun this frame. Their first draw clears
    /// them; storage-backed colours may have initialized before the early programs.
    fresh: bool,
    /// Bit n set: shadowcolor n needs no further frame clear.
    cleared: u8,
    /// Depth comparison for `sampler2DShadow` (hardware filtering): linear, as GL filters it.
    pub comparison: wgpu::Sampler,
}

fn target(
    device: &wgpu::Device,
    label: &str,
    size: u32,
    format: wgpu::TextureFormat,
    copy: wgpu::TextureUsages,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT | copy,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    (texture, view)
}

/// The depth stand-in is initialized to no shadow on the next pack render.
pub(super) fn stand_in(device: &wgpu::Device) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("Shader pack shadow stand-in"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&Default::default())
}

impl ShadowTargets {
    pub fn reset_history(&mut self, device: &wgpu::Device, shadow: &horizon_shaders::Shadow) {
        let mut colors = 0;
        let mut doubled = 0;
        let formats = std::array::from_fn(|n| match &self.color[n] {
            Some([a, b]) => {
                colors |= 1 << n;
                if a.texture() != b.texture() {
                    doubled |= 1 << n;
                }
                a.texture().format()
            }
            None => wgpu::TextureFormat::Rgba8Unorm,
        });
        *self = Self::new(
            device,
            shadow,
            formats,
            colors,
            doubled,
            self.depth1.is_some(),
            self.storage,
            self.scratch.iter().enumerate().fold(0, |mask, (n, view)| {
                mask | if view.is_some() { 1 << n } else { 0 }
            }),
            self.sink.is_some(),
        );
    }

    /// The buffers of `shadow` the pack uses: `colors` (bit n: shadowcolor n), `depth1` when
    /// some program reads `shadowtex1`, `doubled` (bit n: shadowcolor n needs a B texture). The
    /// resolution is kept within the device's limit.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        device: &wgpu::Device,
        shadow: &horizon_shaders::Shadow,
        formats: [wgpu::TextureFormat; SHADOW_COLORS],
        colors: u16,
        doubled: u16,
        depth1: bool,
        storage: u16,
        copied: u16,
        image_sink: bool,
    ) -> Self {
        let size = shadow
            .resolution
            .clamp(1, device.limits().max_texture_dimension_2d);
        let copy = wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST;
        let depth = wgpu::TextureFormat::Depth32Float;
        let color = std::array::from_fn(|n| {
            (colors & 1 << n != 0).then(|| {
                let side = |side: &str| {
                    let label = format!("Shader pack shadowcolor{n} {side}");
                    target(
                        device,
                        &label,
                        size,
                        formats[n],
                        if storage & 1 << n != 0 {
                            wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC
                        } else {
                            wgpu::TextureUsages::empty()
                        },
                    )
                    .1
                };
                let a = side("A");
                let b = if doubled & 1 << n != 0 {
                    side("B")
                } else {
                    a.clone()
                };
                [a, b]
            })
        });
        Self {
            size,
            depth: target(device, "Shader pack shadowtex0", size, depth, copy),
            depth1: depth1.then(|| target(device, "Shader pack shadowtex1", size, depth, copy)),
            color,
            storage,
            sink: image_sink.then(|| {
                target(
                    device,
                    "Shader pack shadow sink",
                    size,
                    wgpu::TextureFormat::R8Unorm,
                    wgpu::TextureUsages::empty(),
                )
                .1
            }),
            scratch: std::array::from_fn(|n| {
                (copied & 1 << n != 0).then(|| {
                    target(
                        device,
                        "Shader pack shadow image copy",
                        size,
                        formats[n],
                        wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_DST,
                    )
                    .1
                })
            }),
            clears: std::array::from_fn(|n| {
                let b = &shadow.colors[n];
                b.clear.then(|| b.clear_color.unwrap_or([1.0; 4]))
            }),
            main: 0,
            fresh: false,
            cleared: 0,
            comparison: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("Shader pack shadow comparison"),
                compare: Some(wgpu::CompareFunction::LessEqual),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
        }
    }

    /// Side `alt` (of "main" and "alt") of shadowcolor `n`.
    pub fn color(&self, n: usize, alt: bool) -> Option<&wgpu::TextureView> {
        let side = usize::from(self.main & 1 << n != 0) ^ usize::from(alt);
        self.color.get(n)?.as_ref().map(|c| &c[side])
    }

    /// Snapshot the current main side before the pass binds it for storage.
    pub fn copy_colors(&self, encoder: &mut wgpu::CommandEncoder, mask: u16) {
        for n in (0..SHADOW_COLORS).filter(|n| mask & 1 << n != 0) {
            if let (Some(source), Some(copy)) = (self.color(n, false), &self.scratch[n]) {
                encoder.copy_texture_to_texture(
                    source.texture().as_image_copy(),
                    copy.texture().as_image_copy(),
                    source.texture().size(),
                );
            }
        }
    }

    /// Starts frame state before setup/begin. Storage-backed colours follow their authored
    /// clears here; other colours/depth stay deferred to the shadow draw or `clear_rest`.
    pub fn begin_frame(&mut self, encoder: &mut wgpu::CommandEncoder) {
        self.fresh = true;
        self.cleared = 0;
        let storage = self.storage;
        for n in (0..SHADOW_COLORS).filter(|n| storage & 1 << n != 0) {
            let Some(color) = self.clears[n] else {
                continue;
            };
            {
                let Some(view) = self.color(n, false) else {
                    continue;
                };
                encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Shader pack shadow image clear"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: clear(color),
                    })],
                    ..Default::default()
                });
            }
            self.cleared |= 1 << n;
        }
    }

    /// The load operations of a pass into the shadow map and shadowcolor buffers `writes`:
    /// clears in the frame's first one, loads after.
    pub fn clear_ops(
        &mut self,
        writes: impl Iterator<Item = usize>,
    ) -> (Ops<f32>, [Ops<Color>; SHADOW_COLORS]) {
        if !std::mem::take(&mut self.fresh) {
            return (load(), [load(); SHADOW_COLORS]);
        }
        let mut colors = [load(); SHADOW_COLORS];
        for n in writes.filter(|&n| n < SHADOW_COLORS) {
            if let Some(c) = self.clears[n]
                && self.cleared & 1 << n == 0
            {
                colors[n] = clear(c);
                self.cleared |= 1 << n;
            }
        }
        let depth = wgpu::Operations {
            load: wgpu::LoadOp::Clear(1.0),
            store: wgpu::StoreOp::Store,
        };
        (depth, colors)
    }

    /// Clears what no pass has cleared this frame: all of it when no shadow program drew, the
    /// cleared shadowcolor buffers the first one did not write. One buffer a pass: eight wide
    /// formats together would pass the device's bytes-per-sample limit.
    pub fn clear_rest(&mut self, encoder: &mut wgpu::CommandEncoder) {
        let mut depth = std::mem::take(&mut self.fresh);
        let cleared = std::mem::replace(&mut self.cleared, u8::MAX);
        let attachments = (0..SHADOW_COLORS)
            .filter(|n| cleared & 1 << n == 0)
            .filter_map(|n| {
                Some(Some(wgpu::RenderPassColorAttachment {
                    view: self.color(n, false)?,
                    depth_slice: None,
                    resolve_target: None,
                    ops: clear(self.clears[n]?),
                }))
            });
        let passes: Vec<Vec<_>> = attachments.map(|a| vec![a]).collect();
        let passes = if passes.is_empty() && depth {
            vec![Vec::new()]
        } else {
            passes
        };
        for attachments in &passes {
            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Shader pack shadow clear"),
                color_attachments: attachments,
                depth_stencil_attachment: std::mem::take(&mut depth).then_some(
                    wgpu::RenderPassDepthStencilAttachment {
                        view: &self.depth.1,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    },
                ),
                ..Default::default()
            });
        }
    }

    /// `shadowtex1`: the shadow map as it is now (before translucent geometry).
    pub fn copy_depth1(&self, encoder: &mut wgpu::CommandEncoder) {
        if let Some((copy, _)) = &self.depth1 {
            encoder.copy_texture_to_texture(
                self.depth.0.as_image_copy(),
                copy.as_image_copy(),
                self.depth.0.size(),
            );
        }
    }
}
