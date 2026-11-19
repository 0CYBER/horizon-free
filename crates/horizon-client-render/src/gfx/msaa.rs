//! 4x terrain MSAA (`msaa4x`, docs/graphics.md): opaque and cutout terrain, near and far, draws
//! into four-sample targets seeded with the sky, which resolve into the world's colour, depth and
//! a native pack's surface target before the single-sample entities, translucency and post
//! passes. Cutout coverage goes out through the sample mask, never HDR's light marks in alpha.
//! Its pipelines come with a renderer made while the setting is on, so a start with it off
//! compiles none (`Renderer::take_msaa_rebuild`); its targets live only while it draws.

use super::{Renderer, SolidPipeline, native_material::SURFACE_FORMAT, reuse, texture_bytes};

#[cfg(test)]
#[path = "msaa_tests.rs"]
mod tests;

pub(super) const SAMPLES: u32 = 4;

/// A pipeline's multisample state at `count` samples.
pub(super) fn state(count: u32) -> wgpu::MultisampleState {
    wgpu::MultisampleState {
        count,
        ..Default::default()
    }
}

/// Whether the adapter draws and reads four samples of the frame's format, depth and a native
/// pack's surface target. Never on OpenGL: old drivers there offer multisampled integer textures
/// without the sample mask output, and its start stays as it was.
fn supported(adapter: &wgpu::Adapter, format: wgpu::TextureFormat) -> bool {
    if adapter.get_info().backend == wgpu::Backend::Gl {
        return false;
    }
    [format, wgpu::TextureFormat::Depth32Float, SURFACE_FORMAT]
        .into_iter()
        .all(|format| {
            let features = adapter.get_texture_format_features(format);
            features
                .flags
                .contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_X4)
                && features.allowed_usages.contains(
                    wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                )
        })
}

/// The renderer's 4x MSAA: the setting, and its passes where they are built.
pub(super) struct Msaa {
    supported: bool,
    requested: bool,
    /// The setting came on without the pipelines: the renderer is to be made again.
    rebuild: bool,
    /// The game makes the renderer again when `rebuild` asks (not split-screen's).
    rebuildable: bool,
    /// The last frame's terrain drew multisampled.
    active: bool,
    passes: Option<Passes>,
}

impl Msaa {
    /// The passes are built only where the adapter can draw them and the setting is on.
    pub(super) fn new(
        kept: &reuse::On<'_>,
        device: &wgpu::Device,
        adapter: &wgpu::Adapter,
        format: wgpu::TextureFormat,
        linear: bool,
        requested: bool,
    ) -> Self {
        let supported = supported(adapter, format);
        if requested && !supported {
            log::warn!("Horizon: 4x terrain MSAA isn't supported by this adapter's formats");
        }
        Self {
            supported,
            requested,
            rebuild: false,
            rebuildable: true,
            active: false,
            passes: (requested && supported).then(|| Passes::new(kept, device, format, linear)),
        }
    }

    /// The terrain's four-sample pipelines are wanted: this renderer was made with it on.
    pub(super) fn built(&self) -> bool {
        self.passes.is_some()
    }

    /// The adapter draws it, and its pipelines are here or a renderer made again brings them.
    fn offered(&self) -> bool {
        self.supported && (self.passes.is_some() || self.rebuildable)
    }

    /// It is on and can draw, unless something in the frame keeps terrain single-sampled.
    pub(super) fn wanted(&self) -> bool {
        self.requested && self.passes.is_some()
    }

    /// The targets at the size of `depth`, the scaled world's (the seed samples its colour,
    /// which the swapchain can't be), with a surface target where a native pack writes one
    /// (`surface`). `None`, or video memory running out, draws this frame single-sampled; `None`
    /// lets the targets go too (the title screen, a classic shader pack).
    pub(super) fn fit(
        &mut self,
        device: &wgpu::Device,
        depth: Option<&wgpu::TextureView>,
        surface: bool,
    ) {
        self.active = match (depth.map(wgpu::TextureView::texture), &mut self.passes) {
            (Some(depth), Some(passes)) => {
                passes.fit(device, [depth.width(), depth.height()], surface)
            }
            // The targets go; a failure stays, so the next frame at its size doesn't try again.
            (None, Some(passes)) => {
                passes.targets = None;
                false
            }
            (_, None) => false,
        };
    }

    /// Its targets ran out of video memory at `size` with `surface` (`fit`): such a frame draws
    /// single-sampled until the size changes or the setting goes off and on.
    pub(super) fn failed_at(&self, size: [u32; 2], surface: bool) -> bool {
        (self.passes.as_ref()).is_some_and(|p| p.failed == Some((size, surface)))
    }

    /// The passes, when this frame's terrain draws multisampled (`fit`).
    pub(super) fn passes(&self) -> Option<&Passes> {
        self.passes.as_ref().filter(|_| self.active)
    }

    fn set(&mut self, on: bool) {
        if self.requested == on {
            return;
        }
        self.requested = on;
        if !on {
            // Switched off before the renderer was made again: nothing left to make.
            self.rebuild = false;
            self.active = false;
            if let Some(passes) = &mut self.passes {
                passes.targets = None;
                passes.failed = None;
            }
        } else if !self.supported {
            log::warn!("Horizon: 4x terrain MSAA isn't supported by this adapter's formats");
        } else if self.passes.is_none() {
            self.rebuild = true;
        }
    }

    /// Bytes of the targets (`gpu_census`).
    pub(super) fn bytes(&self) -> u64 {
        let targets = self.passes.as_ref().and_then(|p| p.targets.as_ref());
        targets.map_or(0, |t| t.textures.iter().map(texture_bytes).sum())
    }
}

impl Renderer {
    /// 4x MSAA on or off (`Graphics::msaa4x`). Off drops its targets at once. On, in a renderer
    /// made with it off, asks for the renderer to be made again (`take_msaa_rebuild`), which
    /// compiles its pipelines and keeps the others.
    pub fn set_msaa4x(&mut self, on: bool) {
        self.msaa.set(on);
    }

    /// Whether the renderer is to be made again for 4x MSAA's pipelines; true once.
    pub fn take_msaa_rebuild(&mut self) -> bool {
        std::mem::take(&mut self.msaa.rebuild)
    }

    /// The samples of the last frame's terrain: 4 while 4x MSAA drew it, else 1.
    pub fn msaa_samples(&self) -> u32 {
        if self.msaa.active { SAMPLES } else { 1 }
    }

    /// The 4x MSAA setting as the renderer has it.
    pub fn msaa_requested(&self) -> bool {
        self.msaa.requested
    }

    /// Whether the Anti-aliasing button offers 4x MSAA: the adapter draws it, and this renderer
    /// has its pipelines or will be made again for them.
    pub(super) fn msaa_offered(&self) -> bool {
        self.msaa.offered()
    }

    /// Whether the game makes this renderer again for 4x MSAA's pipelines when it comes on
    /// (`take_msaa_rebuild`). Split-screen doesn't, so there it is offered only where the
    /// renderer was made with it on.
    pub fn set_msaa_rebuildable(&mut self, rebuildable: bool) {
        self.msaa.rebuildable = rebuildable;
    }
}

impl SolidPipeline {
    /// The four-sample pipeline while 4x MSAA draws (`msaa`), else `get`'s.
    pub(super) fn antialiased(&self, hdr: bool, msaa: bool) -> &wgpu::RenderPipeline {
        match &self.msaa {
            Some(multisampled) if msaa => multisampled.get(hdr),
            _ => self.get(hdr),
        }
    }
}

struct Targets {
    size: [u32; 2],
    surface: bool,
    textures: [wgpu::Texture; 3],
    color: wgpu::TextureView,
    depth: wgpu::TextureView,
    metadata: wgpu::TextureView,
    group: wgpu::BindGroup,
}

/// The sky's seed and the resolve, with the targets they draw.
pub(super) struct Passes {
    format: wgpu::TextureFormat,
    seed_layout: wgpu::BindGroupLayout,
    resolve_layout: wgpu::BindGroupLayout,
    seed: wgpu::RenderPipeline,
    resolve: [wgpu::RenderPipeline; 2],
    empty: wgpu::BindGroup,
    targets: Option<Targets>,
    /// The size and surface whose targets ran out of memory: not tried again until they change.
    failed: Option<([u32; 2], bool)>,
}

impl Passes {
    /// `linear`: the frame holds linear light, which resolves tone-weighted.
    fn new(
        kept: &reuse::On<'_>,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        linear: bool,
    ) -> Self {
        let texture = |binding, sample_type, multisampled| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled,
            },
            count: None,
        };
        let float = wgpu::TextureSampleType::Float { filterable: false };
        let seed_layout = kept.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("MSAA sky input"),
            entries: &[texture(0, float, false)],
        });
        let resolve_layout = kept.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("MSAA resolve inputs"),
            entries: &[
                texture(0, float, true),
                // As world_post's depth binding: an unfilterable float view also
                // translates textureLoad to GLSL, unlike a depth-typed binding.
                texture(1, float, true),
                texture(2, wgpu::TextureSampleType::Uint, true),
            ],
        });
        let empty_layout = kept.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("MSAA unused group"),
            entries: &[],
        });
        let empty = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("MSAA unused group"),
            layout: &empty_layout,
            entries: &[],
        });
        let module = kept.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Terrain MSAA seed and resolve"),
            source: wgpu::ShaderSource::Wgsl(include_str!("msaa.wgsl").into()),
        });
        let constants = [("LINEAR", f64::from(u8::from(linear)))];
        let pipeline = |entry, groups: &[Option<&wgpu::BindGroupLayout>], count, surface| {
            let layout = kept.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Terrain MSAA"),
                bind_group_layouts: groups,
                ..Default::default()
            });
            let targets = [
                Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                }),
                Some(wgpu::ColorTargetState {
                    format: SURFACE_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                }),
            ];
            kept.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vertex"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::Always),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: state(count),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: wgpu::PipelineCompilationOptions {
                        constants: &constants,
                        ..Default::default()
                    },
                    targets: &targets[..if surface { 2 } else { 1 }],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let resolve_groups = [Some(&empty_layout), Some(&resolve_layout)];
        Self {
            format,
            seed: pipeline("seed", &[Some(&seed_layout)], SAMPLES, false),
            resolve: [
                pipeline("resolve_color", &resolve_groups, 1, false),
                pipeline("resolve_surface", &resolve_groups, 1, true),
            ],
            seed_layout,
            resolve_layout,
            empty,
            targets: None,
            failed: None,
        }
    }

    /// Whether the targets are there at `size`, made again where its size or `surface` changed.
    fn fit(&mut self, device: &wgpu::Device, size: [u32; 2], surface: bool) -> bool {
        if self
            .targets
            .as_ref()
            .is_some_and(|t| t.size == size && t.surface == surface)
        {
            return true;
        }
        if self.failed == Some((size, surface)) {
            return false;
        }
        // The old targets go first: a resize would otherwise hold both, as world_post's would.
        self.targets = None;
        // Errors raised are fatal, so catch running out of memory, and the invalid views of a
        // texture that failed.
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let targets = self.build(device, size, surface);
        let error = pollster::block_on(memory.pop()).or(pollster::block_on(validation.pop()));
        if let Some(error) = error {
            log::warn!(
                "Horizon renderer: no video memory for 4x MSAA at {size:?}; terrain draws single-sampled ({error})"
            );
            self.failed = Some((size, surface));
            return false;
        }
        self.failed = None;
        self.targets = Some(targets);
        true
    }

    fn build(&self, device: &wgpu::Device, size: [u32; 2], surface: bool) -> Targets {
        let texture = |label, format, size: [u32; 2]| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: SAMPLES,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
        };
        let textures = [
            texture("MSAA terrain colour", self.format, size),
            texture(
                "MSAA terrain depth",
                wgpu::TextureFormat::Depth32Float,
                size,
            ),
            texture(
                "MSAA terrain surface",
                SURFACE_FORMAT,
                if surface { size } else { [1, 1] },
            ),
        ];
        let [color, depth, metadata] = textures
            .each_ref()
            .map(|texture| texture.create_view(&Default::default()));
        let entries = [&color, &depth, &metadata]
            .into_iter()
            .enumerate()
            .map(|(binding, view)| wgpu::BindGroupEntry {
                binding: binding as u32,
                resource: wgpu::BindingResource::TextureView(view),
            })
            .collect::<Vec<_>>();
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("MSAA resolve inputs"),
            layout: &self.resolve_layout,
            entries: &entries,
        });
        Targets {
            size,
            surface,
            textures,
            color,
            depth,
            metadata,
            group,
        }
    }

    /// Every sample starts as the sky already drawn in `color`, at the far depth.
    pub(super) fn seed(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        color: &wgpu::TextureView,
    ) {
        let t = self.targets.as_ref().expect("MSAA targets fitted");
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("MSAA sky input"),
            layout: &self.seed_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(color),
            }],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Seed terrain MSAA"),
            color_attachments: &[Some(attachment(
                &t.color,
                wgpu::LoadOp::Clear(wgpu::Color::BLACK),
            ))],
            depth_stencil_attachment: Some(depth_attachment(&t.depth, wgpu::LoadOp::Clear(0.0))),
            ..Default::default()
        });
        pass.set_pipeline(&self.seed);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
    }

    /// The terrain's pass over the seeded samples.
    pub(super) fn begin<'a>(&self, encoder: &'a mut wgpu::CommandEncoder) -> wgpu::RenderPass<'a> {
        let t = self.targets.as_ref().expect("MSAA targets fitted");
        let colors = [
            Some(attachment(&t.color, wgpu::LoadOp::Load)),
            Some(attachment(
                &t.metadata,
                wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
            )),
        ];
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("4x MSAA terrain"),
            color_attachments: &colors[..if t.surface { 2 } else { 1 }],
            depth_stencil_attachment: Some(depth_attachment(&t.depth, wgpu::LoadOp::Load)),
            ..Default::default()
        })
    }

    /// The samples into the world's single-sample `color`, `depth` and `surface` targets.
    pub(super) fn resolve(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        color: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        surface: Option<&wgpu::TextureView>,
    ) {
        let t = self.targets.as_ref().expect("MSAA targets fitted");
        let colors = [
            Some(attachment(color, wgpu::LoadOp::Load)),
            surface.map(|v| attachment(v, wgpu::LoadOp::Load)),
        ];
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Resolve terrain MSAA"),
            color_attachments: &colors[..if surface.is_some() { 2 } else { 1 }],
            depth_stencil_attachment: Some(depth_attachment(depth, wgpu::LoadOp::Clear(0.0))),
            ..Default::default()
        });
        pass.set_pipeline(&self.resolve[usize::from(surface.is_some())]);
        pass.set_bind_group(0, &self.empty, &[]);
        pass.set_bind_group(1, &t.group, &[]);
        pass.draw(0..3, 0..1);
    }
}

fn attachment(
    view: &wgpu::TextureView,
    load: wgpu::LoadOp<wgpu::Color>,
) -> wgpu::RenderPassColorAttachment<'_> {
    wgpu::RenderPassColorAttachment {
        view,
        resolve_target: None,
        depth_slice: None,
        ops: wgpu::Operations {
            load,
            store: wgpu::StoreOp::Store,
        },
    }
}

fn depth_attachment(
    view: &wgpu::TextureView,
    load: wgpu::LoadOp<f32>,
) -> wgpu::RenderPassDepthStencilAttachment<'_> {
    wgpu::RenderPassDepthStencilAttachment {
        view,
        depth_ops: Some(wgpu::Operations {
            load,
            store: wgpu::StoreOp::Store,
        }),
        stencil_ops: None,
    }
}
