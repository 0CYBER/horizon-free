//! A visibility buffer for solid terrain, opt-in with `HORIZON_VISBUFFER=1` (read once, at
//! startup): pass 1 rasterises the solid flat draws (near sections and far tiles, phase 1 and,
//! with occlusion culling, phase 2) writing only depth and the ID of the quad each pixel shows
//! (`visbuffer.wgsl`'s `vis_id`: arena page and quad); cutout and fading
//! quads keep their alpha test and dither there, the rest never discard. Pass 2 is one
//! full-screen draw with up to `PAGE_SLOTS` arena pages bound at once (more pages, more draws)
//! that reads the quad back from its page and its section's origin (through a copy of every
//! drawn page's block table in one buffer), and runs terrain.wgsl's
//! vertex work and solid shading once per pixel with analytic screen gradients. Every solid
//! pixel is shaded once, however many quads were drawn over it and however small they are.
//! A draw per page instead made each GPU thread group run the shading once for every page
//! among its pixels, with most lanes idle: the pages' sections lie all over the screen.
//!
//! It runs only in frames drawn flat (`flat`: Metal past `flat::MIN_SECTIONS`, or
//! `HORIZON_FLAT_DRAWS=1`) and falls back to the normal solid passes in frames with a shader
//! pack drawing terrain, one of Horizon's own packs with a material stage or a shadow pass, a
//! pack's surface target, and on the title screen. The water mirror's own pass stays as it is.
use super::arena::Arena;
use super::flat::Flat;
use super::pack_timing::PassTiming;

/// Arena pages one resolve draw reads (`visbuffer.wgsl`'s `vis_page_0`...).
pub const PAGE_SLOTS: u32 = 16;
/// Arena pages an ID names (`visbuffer.wgsl`'s `vis_id`): frames with more draw without it.
pub const MAX_PAGES: u32 = 64;
/// Maximum number of `PAGE_SLOTS`-page chunks addressable by `MAX_PAGES`.
const MAX_CHUNKS: usize = (MAX_PAGES / PAGE_SLOTS) as usize;
/// The resolve binds that many pages, the block tables and the origins by section id.
const STORAGE_BUFFERS: u32 = PAGE_SLOTS + 2;
pub(super) const IDS: wgpu::TextureFormat = wgpu::TextureFormat::R32Uint;

/// Tracks which `Flat` block-table generation was last copied into `Visbuffer::blocks` for each
/// arena page, skipping redundant multi-megabyte `copy_buffer_to_buffer` transfers in steady state.
struct PageTableSync {
    copied_generations: [u64; MAX_PAGES as usize],
}

impl PageTableSync {
    fn new() -> Self {
        Self {
            copied_generations: [0; MAX_PAGES as usize],
        }
    }

    fn needs_copy(&self, page: u32, generation: u64) -> bool {
        let idx = page as usize;
        idx < MAX_PAGES as usize && (generation == 0 || self.copied_generations[idx] != generation)
    }

    fn mark_copied(&mut self, page: u32, generation: u64) {
        if let Some(slot) = self.copied_generations.get_mut(page as usize) {
            *slot = generation;
        }
    }

    fn clear(&mut self) {
        self.copied_generations = [0; MAX_PAGES as usize];
    }

    fn trim(&mut self, live: impl Fn(u32) -> bool) {
        for (page, slot) in self.copied_generations.iter_mut().enumerate() {
            if !live(page as u32) {
                *slot = 0;
            }
        }
    }
}

/// Resource identity key for a cached resolve-pass bind group covering one `PAGE_SLOTS` chunk.
struct ResolveGroupKey<B> {
    ids_generation: u64,
    blocks: B,
    origins: B,
    pages: [B; PAGE_SLOTS as usize],
}

impl<B: PartialEq> ResolveGroupKey<B> {
    fn matches(
        &self,
        ids_generation: u64,
        blocks: &B,
        origins: &B,
        pages: &[&B; PAGE_SLOTS as usize],
    ) -> bool {
        self.ids_generation == ids_generation
            && self.blocks == *blocks
            && self.origins == *origins
            && self.pages.iter().zip(pages.iter()).all(|(a, b)| *a == **b)
    }
}

struct CachedResolveGroup<B, G> {
    key: ResolveGroupKey<B>,
    bound_pages: [u32; PAGE_SLOTS as usize],
    group: G,
}

/// Per-chunk cache for pass-2 resolve bind groups, avoiding per-frame `device.create_bind_group`
/// calls and temporary `Vec` allocations when arena/origin/ID buffers are unchanged.
struct ResolveGroupCache<B, G> {
    slots: [Option<CachedResolveGroup<B, G>>; MAX_CHUNKS],
}

impl<B, G> ResolveGroupCache<B, G> {
    fn new() -> Self {
        Self {
            slots: [const { None }; MAX_CHUNKS],
        }
    }

    fn ensure(
        &mut self,
        chunk: usize,
        ids_generation: u64,
        blocks: &B,
        origins: &B,
        pages: &[&B; PAGE_SLOTS as usize],
        bound_pages: [u32; PAGE_SLOTS as usize],
        build: impl FnOnce() -> G,
    ) where
        B: Clone + PartialEq,
    {
        if self.slots[chunk]
            .as_ref()
            .is_some_and(|entry| entry.key.matches(ids_generation, blocks, origins, pages))
        {
            return;
        }
        let key = ResolveGroupKey {
            ids_generation,
            blocks: blocks.clone(),
            origins: origins.clone(),
            pages: std::array::from_fn(|i| pages[i].clone()),
        };
        self.slots[chunk] = Some(CachedResolveGroup {
            key,
            bound_pages,
            group: build(),
        });
    }

    fn get(&self, chunk: usize) -> Option<&G> {
        self.slots[chunk].as_ref().map(|entry| &entry.group)
    }

    fn clear(&mut self) {
        self.slots = [const { None }; MAX_CHUNKS];
    }

    fn trim(&mut self, live: impl Fn(u32) -> bool) {
        for slot in &mut self.slots {
            if slot
                .as_ref()
                .is_some_and(|entry| entry.bound_pages.iter().any(|&p| !live(p)))
            {
                *slot = None;
            }
        }
    }
}

fn collect_sorted_pages(
    pages: impl IntoIterator<Item = u32>,
    out: &mut [u32; MAX_PAGES as usize],
) -> usize {
    let mut mask = 0u64;
    for page in pages {
        if page < MAX_PAGES {
            mask |= 1u64 << page;
        }
    }
    let mut len = 0usize;
    while mask != 0 {
        out[len] = mask.trailing_zeros();
        len += 1;
        mask &= mask - 1;
    }
    len
}

/// Whether the visibility buffer is asked for, and the device binds storage buffers enough.
pub fn wanted(device: &wgpu::Device) -> bool {
    std::env::var("HORIZON_VISBUFFER").as_deref() == Ok("1")
        && device.limits().max_storage_buffers_per_shader_stage >= STORAGE_BUFFERS
}

pub struct Visbuffer {
    /// Pass 1 over the quads `fs_opaque` draws (no discard) and the rest.
    opaque: wgpu::RenderPipeline,
    cutout: wgpu::RenderPipeline,
    resolve: wgpu::RenderPipeline,
    /// Pass 2's group 1: the block tables (binding 0), the origins by section id (1),
    /// `PAGE_SLOTS` pages' vertices (from 2), the IDs (after them).
    page_layout: wgpu::BindGroupLayout,
    /// Every page's block table, page after page (`Flat::blocks_per_page` entries each),
    /// synced lazily from the drawn pages' own whenever their generation changes, and its size
    /// in pages.
    blocks: Option<(wgpu::Buffer, u32)>,
    blocks_per_page: u64,
    /// The IDs, the size of the world's depth.
    ids: Option<(wgpu::TextureView, [u32; 2])>,
    /// Generation bumped whenever `ids` is replaced so cached resolve bind groups rebind it.
    ids_generation: u64,
    /// Per-page copied block-table generations for `blocks`.
    page_sync: PageTableSync,
    /// Cached pass-2 resolve bind groups keyed by `(ids_generation, blocks, origins, pages)`.
    resolve_cache: ResolveGroupCache<wgpu::Buffer, wgpu::BindGroup>,
    /// The size the IDs or block tables had no room at (#13624).
    no_room: Option<[u32; 2]>,
}

impl Visbuffer {
    /// `terrain_layout`: terrain.wgsl's group 0; `flat_layout`: the flat draws' group 1;
    /// `color`: the world pass's colour format; `constants`: the sprite arrays' pipeline
    /// constants.
    pub fn new(
        device: &wgpu::Device,
        terrain_layout: &wgpu::BindGroupLayout,
        flat: &Flat,
        (color, linear): (wgpu::TextureFormat, bool),
        constants: &[(&str, f64)],
    ) -> Self {
        let flat_layout = &flat.draw_layout;
        let blocks_per_page = flat.blocks_per_page();
        let resolve_constants: Vec<(&str, f64)> = constants
            .iter()
            .copied()
            .chain([("VIS_BLOCKS_PER_PAGE", blocks_per_page as f64)])
            .chain(super::linear_blend::frame_constants(color, linear))
            .collect();
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Horizon visibility buffer"),
            source: wgpu::ShaderSource::Wgsl(
                (super::terrain_shader() + include_str!("../visbuffer.wgsl")).into(),
            ),
        });
        let fragment = wgpu::ShaderStages::FRAGMENT;
        let storage = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: fragment,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let entries: Vec<_> = (0..2 + PAGE_SLOTS)
            .map(storage)
            .chain([wgpu::BindGroupLayoutEntry {
                binding: 2 + PAGE_SLOTS,
                visibility: fragment,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Uint,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }])
            .collect();
        let page_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Visibility buffer pages"),
            entries: &entries,
        });
        let id_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Visibility buffer IDs"),
            bind_group_layouts: &[Some(terrain_layout), Some(flat_layout)],
            ..Default::default()
        });
        let resolve_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Visibility buffer resolve"),
            bind_group_layouts: &[Some(terrain_layout), Some(&page_layout)],
            ..Default::default()
        });
        let ids = |label: &'static str, vertex: &'static str, fragment: &'static str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&id_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some(vertex),
                    compilation_options: Default::default(),
                    // `section_corner` reads the arena page (`flat::ARENA_BINDING`).
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    cull_mode: Some(wgpu::Face::Back),
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(fragment),
                    compilation_options: wgpu::PipelineCompilationOptions {
                        constants,
                        ..Default::default()
                    },
                    targets: &[Some(wgpu::ColorTargetState {
                        format: IDS,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let opaque = ids("Visibility buffer opaque", "vs_vis_opaque", "fs_vis_id");
        let cutout = ids("Visibility buffer cutout", "vs_vis_cutout", "fs_vis_cutout");
        let resolve = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Visibility buffer resolve"),
            layout: Some(&resolve_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_vis_resolve"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_vis_resolve"),
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants: &resolve_constants,
                    ..Default::default()
                },
                targets: &[Some(wgpu::ColorTargetState {
                    format: color,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        Self {
            opaque,
            cutout,
            resolve,
            page_layout,
            blocks: None,
            blocks_per_page,
            ids: None,
            ids_generation: 0,
            page_sync: PageTableSync::new(),
            resolve_cache: ResolveGroupCache::new(),
            no_room: None,
        }
    }

    /// Makes the IDs the world depth's `size`, and the block tables hold the pages `flat`
    /// draws this frame: `false` where there was no room for them (#13624), and the frame
    /// draws solid terrain without the visibility buffer. Nothing that failed is kept, the old
    /// one goes first, and that size is not tried again until it changes.
    pub fn fit(&mut self, device: &wgpu::Device, size: [u32; 2], flat: &Flat) -> bool {
        if self.no_room == Some(size) {
            return false;
        }
        let pages = flat.calls[..flat.solid_calls]
            .iter()
            .map(|c| c.page + 1)
            .max()
            .unwrap_or(1);
        self.fit_page_count(device, size, pages)
    }

    /// Makes the IDs the world depth's `size`, and the block tables hold at least `pages`.
    pub fn fit_pages(&mut self, device: &wgpu::Device, size: [u32; 2], pages: &[u32]) -> bool {
        if self.no_room == Some(size) {
            return false;
        }
        let count = pages.iter().map(|&p| p + 1).max().unwrap_or(1);
        self.fit_page_count(device, size, count)
    }

    /// Drops cached resolve bind groups and copied block-table generations when the world resets.
    pub fn clear(&mut self) {
        self.page_sync.clear();
        self.resolve_cache.clear();
    }

    /// Forgets copied block-table generations and cached resolve bind groups after an abandoned
    /// or discarded frame whose encoder never ran on the GPU, forcing the next frame to re-copy
    /// dirty page tables and rebuild any bind groups.
    pub fn forget(&mut self) {
        self.clear();
    }

    /// Drops cached entries referencing arena pages that `live` reports as retired.
    pub fn trim(&mut self, live: impl Fn(u32) -> bool) {
        self.page_sync.trim(&live);
        self.resolve_cache.trim(&live);
    }

    fn fit_page_count(&mut self, device: &wgpu::Device, size: [u32; 2], pages: u32) -> bool {
        use super::frame_targets::{Kind, made};
        if self.blocks.as_ref().is_none_or(|(_, n)| *n < pages) {
            let pages = pages.next_power_of_two();
            self.blocks = None;
            self.page_sync.clear();
            self.resolve_cache.clear();
            self.blocks = made(device, Kind::Visibility, || {
                let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Visibility buffer block tables"),
                    size: u64::from(pages) * self.blocks_per_page * 4,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                (buffer, pages)
            });
            if self.blocks.is_none() {
                self.no_room = Some(size);
                return false;
            }
        }
        if self.ids.as_ref().is_some_and(|(_, s)| *s == size) {
            return true;
        }
        self.ids = None;
        self.ids_generation = self.ids_generation.wrapping_add(1);
        self.resolve_cache.clear();
        self.ids = made(device, Kind::Visibility, || Self::ids(device, size));
        self.no_room = self.ids.is_none().then_some(size);
        self.ids.is_some()
    }

    fn ids(device: &wgpu::Device, size: [u32; 2]) -> (wgpu::TextureView, [u32; 2]) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Visibility buffer IDs"),
            size: wgpu::Extent3d {
                width: size[0].max(1),
                height: size[1].max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: IDS,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        (texture.create_view(&Default::default()), size)
    }

    /// Pass 1's render pass over the world's `depth`: the IDs cleared (`clear`, phase 1) or
    /// kept (phase 2 of occlusion culling).
    pub fn id_pass<'e>(
        &self,
        encoder: &'e mut wgpu::CommandEncoder,
        depth: &wgpu::TextureView,
        clear: bool,
        mut timing: Option<&mut PassTiming>,
    ) -> super::pack_timing::CountedPass<'e> {
        let (ids, _) = self.ids.as_ref().expect("fitted before the world pass");
        let name = if clear {
            "Visibility IDs"
        } else {
            "Visibility IDs after culling"
        };
        let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Horizon visibility buffer"),
            timestamp_writes: timing.as_deref_mut().and_then(|t| t.writes(name)),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: ids,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: if clear {
                        wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                    } else {
                        wgpu::LoadOp::Load
                    },
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        super::pack_timing::counted(timing, pass, super::pixel_count(depth), name)
    }

    /// Phase 1's solid calls into the IDs, as the normal passes draw them: the opaque
    /// pipeline over `calls[..pass_calls[0]]`, the cutout one over
    /// `calls[pass_calls[1]..phase1_calls]` (each leaves out the other's quads).
    pub fn draw_ids<'a>(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        flat: &Flat,
        vertices: impl Fn(u32) -> wgpu::BufferSlice<'a> + Copy,
    ) {
        let [opaque, cutout, _] = flat.pass_calls;
        pass.set_pipeline(&self.opaque);
        flat.draw_paged(pass, &flat.calls[..opaque], vertices);
        pass.set_pipeline(&self.cutout);
        flat.draw_paged(pass, &flat.calls[cutout..flat.phase1_calls], vertices);
    }

    /// Phase 2's calls as `Flat::flatten_phase2` left them (their first instance is their
    /// page: `Flat::page_instances`), through both pipelines.
    pub fn draw_ids_phase2<'a>(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        flat: &Flat,
        vertices: impl Fn(u32) -> wgpu::BufferSlice<'a> + Copy,
    ) {
        // Phase 2 lists each section's opaque runs first, its cutout runs after.
        let split = flat.pass_calls[2] - flat.phase1_calls;
        let total = flat.solid_calls - flat.phase1_calls;
        pass.set_pipeline(&self.opaque);
        flat.draw_phase2(pass, 0..split, vertices);
        pass.set_pipeline(&self.cutout);
        flat.draw_phase2(pass, split..total, vertices);
    }

    /// Bytes of its block tables and IDs.
    pub fn bytes(&self) -> u64 {
        let blocks = self.blocks.as_ref().map_or(0, |(b, _)| b.size());
        let ids = self
            .ids
            .as_ref()
            .map_or(0, |(_, [w, h])| u64::from(*w) * u64::from(*h) * 4);
        blocks + ids
    }

    /// Pass 2: shades the solid terrain the IDs show into `target`, the world's colour,
    /// over the sky drawn before. `terrain`: terrain.wgsl's group 0.
    #[allow(clippy::too_many_arguments)]
    pub fn resolve(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        terrain: &wgpu::BindGroup,
        flat: &Flat,
        arena: &Arena,
        timing: Option<&mut PassTiming>,
    ) {
        let mut pages = [0u32; MAX_PAGES as usize];
        let len = collect_sorted_pages(
            flat.calls[..flat.solid_calls].iter().map(|c| c.page),
            &mut pages,
        );
        self.resolve_pages(
            device,
            encoder,
            target,
            terrain,
            flat,
            arena,
            &pages[..len],
            timing,
        );
    }

    /// Shades the visibility buffer IDs for `pages` into `target`.
    pub fn resolve_pages(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        terrain: &wgpu::BindGroup,
        flat: &Flat,
        arena: &Arena,
        pages: &[u32],
        timing: Option<&mut PassTiming>,
    ) {
        let (Some((ids, _)), Some((blocks, _))) = (&self.ids, &self.blocks) else {
            return;
        };
        let Some(&any) = pages.iter().find(|&&p| p < MAX_PAGES) else {
            return;
        };
        let table = self.blocks_per_page * 4;
        let mut active_chunks = [false; MAX_CHUNKS];
        for &page in pages {
            if page >= MAX_PAGES {
                continue;
            }
            active_chunks[(page / PAGE_SLOTS) as usize] = true;
            if let (Some(from), Some(generation)) =
                (flat.block_table(page), flat.block_table_generation(page))
                && self.page_sync.needs_copy(page, generation)
            {
                encoder.copy_buffer_to_buffer(from, 0, blocks, u64::from(page) * table, table);
                self.page_sync.mark_copied(page, generation);
            }
        }
        // Pages in groups of `PAGE_SLOTS` by number: a page's slot is its number's remainder.
        // Binding resident pages (falling back to the lowest resident page for empty slots)
        // keeps cached bind groups stable even as individual pages enter and leave the frustum.
        let fallback = (0..MAX_PAGES).find(|&p| arena.has_page(p)).unwrap_or(any);
        let origins = flat.origins();
        for (chunk, &active) in active_chunks.iter().enumerate() {
            if !active {
                continue;
            }
            let base_page = chunk as u32 * PAGE_SLOTS;
            let bound_pages: [u32; PAGE_SLOTS as usize] = std::array::from_fn(|slot| {
                let page = base_page + slot as u32;
                if arena.has_page(page) { page } else { fallback }
            });
            let page_buffers: [&wgpu::Buffer; PAGE_SLOTS as usize] =
                std::array::from_fn(|slot| arena.buffer(bound_pages[slot]));
            self.resolve_cache.ensure(
                chunk,
                self.ids_generation,
                blocks,
                origins,
                &page_buffers,
                bound_pages,
                || {
                    let entries: [wgpu::BindGroupEntry<'_>; STORAGE_BUFFERS as usize + 1] =
                        std::array::from_fn(|i| match i {
                            0 => wgpu::BindGroupEntry {
                                binding: 0,
                                resource: blocks.as_entire_binding(),
                            },
                            1 => wgpu::BindGroupEntry {
                                binding: 1,
                                resource: origins.as_entire_binding(),
                            },
                            idx if idx < 2 + PAGE_SLOTS as usize => wgpu::BindGroupEntry {
                                binding: idx as u32,
                                resource: page_buffers[idx - 2].as_entire_binding(),
                            },
                            _ => wgpu::BindGroupEntry {
                                binding: 2 + PAGE_SLOTS,
                                resource: wgpu::BindingResource::TextureView(ids),
                            },
                        });
                    device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("Visibility buffer pages"),
                        layout: &self.page_layout,
                        entries: &entries,
                    })
                },
            );
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Horizon visibility buffer resolve"),
            timestamp_writes: timing.and_then(|t| t.writes("Visibility resolve")),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            ..Default::default()
        });
        pass.set_pipeline(&self.resolve);
        pass.set_bind_group(0, terrain, &[]);
        // The group of pages goes as the instance: each draw shades the pixels whose quad is
        // in one of its pages (one draw, up to `PAGE_SLOTS` pages).
        for (chunk, &active) in active_chunks.iter().enumerate() {
            if !active {
                continue;
            }
            let group = self
                .resolve_cache
                .get(chunk)
                .expect("active resolve bind group populated");
            pass.set_bind_group(1, group, &[]);
            let chunk_u32 = chunk as u32;
            pass.draw(0..3, chunk_u32..chunk_u32 + 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visbuffer_shader_validates() {
        let source = super::super::terrain_shader() + include_str!("../visbuffer.wgsl");
        let module = naga::front::wgsl::parse_str(&source).unwrap_or_else(|e| {
            panic!(
                "visbuffer WGSL parse failed:\n{}",
                e.emit_to_string(&source)
            )
        });
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("visbuffer naga validation failed: {e}"));
    }

    #[test]
    fn page_table_sync_skips_clean_pages_and_invalidates_on_change_trim_and_clear() {
        let mut sync = PageTableSync::new();
        assert!(sync.needs_copy(0, 1));
        assert!(sync.needs_copy(0, 0));
        assert!(!sync.needs_copy(MAX_PAGES, 1));

        sync.mark_copied(0, 1);
        sync.mark_copied(3, 2);
        // Steady state: unchanged generations skip copies.
        assert!(!sync.needs_copy(0, 1));
        assert!(!sync.needs_copy(3, 2));
        // Uncopied page or bumped generation forces a copy.
        assert!(sync.needs_copy(1, 5));
        assert!(sync.needs_copy(0, 4));
        // Generation 0 sentinel always requests a copy.
        assert!(sync.needs_copy(0, 0));

        // Trimming retired page 3 invalidates only page 3.
        sync.trim(|p| p != 3);
        assert!(!sync.needs_copy(0, 1));
        assert!(sync.needs_copy(3, 2));

        // Clearing invalidates every page.
        sync.clear();
        assert!(sync.needs_copy(0, 1));
        // Out-of-bounds page mark_copied is a safe no-op.
        sync.mark_copied(MAX_PAGES + 7, 9);
    }

    #[test]
    fn resolve_group_cache_reuses_and_invalidates_on_every_binding_key_change() {
        let mut cache: ResolveGroupCache<u32, usize> = ResolveGroupCache::new();
        let mut builds = 0usize;
        let bound_chunk0: [u32; PAGE_SLOTS as usize] = std::array::from_fn(|i| i as u32);
        let bound_chunk1: [u32; PAGE_SLOTS as usize] =
            std::array::from_fn(|i| PAGE_SLOTS + i as u32);
        let mut page_ids_0: [u32; PAGE_SLOTS as usize] = std::array::from_fn(|i| 100 + i as u32);
        let page_ids_1: [u32; PAGE_SLOTS as usize] = std::array::from_fn(|i| 200 + i as u32);

        fn refs_of(ids: &[u32; PAGE_SLOTS as usize]) -> [&u32; PAGE_SLOTS as usize] {
            std::array::from_fn(|i| &ids[i])
        }

        // Initial populate for chunk 0.
        let p0 = refs_of(&page_ids_0);
        cache.ensure(0, 1, &10, &20, &p0, bound_chunk0, || {
            builds += 1;
            builds
        });
        assert_eq!(builds, 1);
        assert_eq!(cache.get(0), Some(&1));

        // Identical inputs hit the cache without rebuilding.
        cache.ensure(0, 1, &10, &20, &p0, bound_chunk0, || {
            builds += 1;
            builds
        });
        assert_eq!(builds, 1);

        // Chunk 1 caches independently without evicting chunk 0.
        let p1 = refs_of(&page_ids_1);
        cache.ensure(1, 1, &10, &20, &p1, bound_chunk1, || {
            builds += 1;
            builds
        });
        assert_eq!(builds, 2);
        assert_eq!(cache.get(0), Some(&1));
        assert_eq!(cache.get(1), Some(&2));

        // Changing ids_generation invalidates.
        cache.ensure(0, 2, &10, &20, &p0, bound_chunk0, || {
            builds += 1;
            builds
        });
        assert_eq!(builds, 3);

        // Changing blocks buffer identity invalidates.
        cache.ensure(0, 2, &11, &20, &p0, bound_chunk0, || {
            builds += 1;
            builds
        });
        assert_eq!(builds, 4);

        // Changing origins buffer identity invalidates.
        cache.ensure(0, 2, &11, &21, &p0, bound_chunk0, || {
            builds += 1;
            builds
        });
        assert_eq!(builds, 5);

        // Changing a single arena page buffer invalidates.
        page_ids_0[7] = 999;
        let p0_changed = refs_of(&page_ids_0);
        cache.ensure(0, 2, &11, &21, &p0_changed, bound_chunk0, || {
            builds += 1;
            builds
        });
        assert_eq!(builds, 6);
        assert_eq!(cache.get(0), Some(&6));

        // Trimming a retired page drops only the chunk binding it.
        cache.trim(|p| p != 7);
        assert_eq!(cache.get(0), None);
        assert_eq!(cache.get(1), Some(&2));

        // Clear drops all cached chunks.
        cache.clear();
        assert_eq!(cache.get(0), None);
        assert_eq!(cache.get(1), None);
    }

    #[test]
    fn collect_sorted_pages_deduplicates_sorts_and_ignores_overflow() {
        let mut out = [0u32; MAX_PAGES as usize];
        assert_eq!(collect_sorted_pages([], &mut out), 0);

        let len = collect_sorted_pages([17, 2, 17, 0, MAX_PAGES, 63, 2, 100], &mut out);
        assert_eq!(&out[..len], &[0, 2, 17, 63]);
    }

    #[test]
    fn discarded_frame_invalidates_page_sync_and_retries_block_table_copy_on_next_frame() {
        use crate::gfx::arena::GRAIN;
        use crate::gfx::upload::Uploads;
        use crate::mesher::{OPAQUE_LAYER, TerrainVertex, UV_STEPS};

        let _gpu = crate::gfx::test_gpu::lock();
        let instance = crate::gfx::test_gpu::instance();
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        if adapter.limits().max_storage_buffers_per_shader_stage < STORAGE_BUFFERS {
            eprintln!("adapter lacks {STORAGE_BUFFERS} storage buffers: skipped");
            return;
        }
        let desc = wgpu::DeviceDescriptor {
            label: Some("Visbuffer discard recovery test"),
            required_limits: adapter.limits(),
            ..Default::default()
        };
        let Ok((device, queue)) = pollster::block_on(adapter.request_device(&desc)) else {
            eprintln!("no GPU device: skipped");
            return;
        };

        let terrain_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Visbuffer test terrain layout"),
            entries: &super::super::native_material::terrain_entries(),
        });
        let dummy_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(
                "@vertex fn vs() -> @builtin(position) vec4<f32> { return vec4<f32>(0.0); }
                 @fragment fn fs() -> @location(0) vec4<f32> { return vec4<f32>(0.0); }"
                    .into(),
            ),
        });
        let dummy_pipeline =
            |layout: &wgpu::PipelineLayout, label: &str, _: &str, _: &str, _, _, _| {
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(label),
                    layout: Some(layout),
                    vertex: wgpu::VertexState {
                        module: &dummy_module,
                        entry_point: Some("vs"),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    primitive: Default::default(),
                    depth_stencil: None,
                    multisample: Default::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: &dummy_module,
                        entry_point: Some("fs"),
                        compilation_options: Default::default(),
                        targets: &[Some(wgpu::TextureFormat::Rgba8Unorm.into())],
                    }),
                    multiview_mask: None,
                    cache: None,
                })
            };
        let dummy_pipeline_ref = &dummy_pipeline;
        let dummy_solid = |layout: &wgpu::PipelineLayout, label: &str, _: &str, _: &str, _| {
            super::super::SolidPipeline {
                plain: dummy_pipeline_ref(layout, label, "", "", None, None, true),
                hdr: None,
                msaa: None,
            }
        };

        let page_vertices = 4 * GRAIN;
        let mut flat = Flat::new(
            &device,
            &terrain_layout,
            page_vertices,
            0,
            false,
            dummy_pipeline_ref,
            dummy_solid,
        );
        let stride = std::mem::size_of::<TerrainVertex>() as u64;
        let mut arena = Arena::new(&device, stride, page_vertices, true);
        let mut vis = Visbuffer::new(
            &device,
            &terrain_layout,
            &flat,
            (wgpu::TextureFormat::Rgba8Unorm, false),
            &[("SPRITE_ARRAYS", 1.0), ("SPRITE_SPLIT", 2048.0)],
        );
        let mut uploads = Uploads::new(&device);

        // Camera at origin looking -Y with -Z up, FOV 90 deg.
        let proj = crate::gfx::world_projection(90.0, 1.0, 256.0);
        let view = glam::Mat4::look_at_rh(glam::Vec3::ZERO, glam::Vec3::NEG_Y, glam::Vec3::NEG_Z);
        let mut globals: crate::gfx::Globals = bytemuck::Zeroable::zeroed();
        globals.view_proj = (proj * view).to_cols_array_2d();
        globals.fog = [1000.0, 2000.0, 1000.0, 2000.0];
        globals.filtering = [0.0, 16.0, 0.0, 0.0];
        globals.world_view = [1.0, 0.0, 0.0, 0.0];
        let globals_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Visbuffer test globals"),
            size: std::mem::size_of::<crate::gfx::Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&globals_buf, 0, bytemuck::bytes_of(&globals));

        let create_white_tex = |label: &str, layers: u32| -> wgpu::TextureView {
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: layers,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            for layer in 0..layers {
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &tex,
                        mip_level: 0,
                        origin: wgpu::Origin3d {
                            x: 0,
                            y: 0,
                            z: layer,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    &[255u8, 255, 255, 255],
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(4),
                        rows_per_image: Some(1),
                    },
                    wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                );
            }
            tex.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(if layers > 1 {
                    wgpu::TextureViewDimension::D2Array
                } else {
                    wgpu::TextureViewDimension::D2
                }),
                ..Default::default()
            })
        };
        let sprite_view = create_white_tex("Visbuffer test sprites", 2);
        let sprite_views: [wgpu::TextureView; 8] = std::array::from_fn(|_| sprite_view.clone());
        let lightmap_view = create_white_tex("Visbuffer test lightmap", 1);
        let emission_view = create_white_tex("Visbuffer test emission", 1);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor::default());
        let group0 = crate::gfx::terrain_group(
            &device,
            "Visbuffer test group0",
            &terrain_layout,
            &globals_buf,
            &sprite_views,
            &sampler,
            &lightmap_view,
            &sampler,
            &emission_view,
        );

        // Single +Y quad spanning x in [0, 8], z in [0, 6] at y = 2: RED on x = 0, BLUE on x = 8.
        // origin_0 places x_local = 1 (RED) at screen center; origin_1 places x_local = 7 (BLUE)
        // at screen center. Both origins cover the center pixel, so a stale GPU block table in
        // `vis.blocks` visibly resolves the wrong origin color.
        let v = |x, z, color| TerrainVertex {
            pos: [x, 2.0, z],
            uv: [0, 0],
            layer: OPAQUE_LAYER,
            light: 0xF0F0,
            color,
        };
        let quad = [
            v(0.0, 0.0, 0xFF00_00FF),
            v(0.0, 6.0, 0xFF00_00FF),
            v(8.0, 6.0, 0xFFFF_0000),
            v(8.0, 0.0, 0xFFFF_0000),
        ];
        let index_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Visbuffer test indices"),
            size: 24,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&index_buf, 0, bytemuck::cast_slice(&[0u32, 1, 2, 0, 2, 3]));

        let origin_0 = [-1.0f32, -4.0, -3.0, 1.0 / UV_STEPS];
        let origin_1 = [-7.0f32, -4.0, -3.0, 1.0 / UV_STEPS];
        let mut setup = device.create_command_encoder(&Default::default());
        let (page, start) = arena
            .alloc(&device, &mut setup, GRAIN)
            .expect("arena page 0");
        assert_eq!((page, start), (0, 0));
        uploads.buffer(
            &mut setup,
            arena.buffer(page),
            start * stride,
            bytemuck::cast_slice(&quad),
        );
        let id0 = flat.add(&device, &mut setup, &mut uploads, 0, 0, GRAIN);
        let id1 = flat.add(&device, &mut setup, &mut uploads, 0, GRAIN, GRAIN);
        assert!(flat.write_origins(
            &device,
            &mut setup,
            &mut uploads,
            [(id0, origin_0), (id1, origin_1)].into_iter(),
        ));
        flat.bind_pages(&device, |p| arena.has_page(p).then(|| arena.buffer(p)));
        uploads.finish(&setup);
        queue.submit([setup.finish()]);

        let size = 64u32;
        assert!(vis.fit_pages(&device, [size, size], &[0]));
        let color_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Visbuffer test color"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let depth_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Visbuffer test depth"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let color_view = color_tex.create_view(&Default::default());
        let depth_view = depth_tex.create_view(&Default::default());
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Visbuffer test readback"),
            size: u64::from(size * size * 4),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let encode_vis_frame =
            |vis: &mut Visbuffer, flat: &Flat, encoder: &mut wgpu::CommandEncoder| {
                {
                    let _clear = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("Visbuffer test clear"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &color_view,
                            resolve_target: None,
                            depth_slice: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                            view: &depth_view,
                            depth_ops: Some(wgpu::Operations {
                                load: wgpu::LoadOp::Clear(0.0),
                                store: wgpu::StoreOp::Store,
                            }),
                            stencil_ops: None,
                        }),
                        ..Default::default()
                    });
                }
                {
                    let mut ids = vis.id_pass(encoder, &depth_view, true, None);
                    ids.set_pipeline(&vis.opaque);
                    ids.set_bind_group(0, &group0, &[]);
                    ids.set_bind_group(1, flat.page_group(0).expect("page 0 bound"), &[]);
                    ids.set_index_buffer(index_buf.slice(..), wgpu::IndexFormat::Uint32);
                    ids.draw_indexed(0..6, 0, 0..1);
                }
                vis.resolve_pages(
                    &device,
                    encoder,
                    &color_view,
                    &group0,
                    flat,
                    &arena,
                    &[0],
                    None,
                );
            };

        let read_center_pixel = |mut encoder: wgpu::CommandEncoder| -> [u8; 4] {
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &color_tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(size * 4),
                        rows_per_image: Some(size),
                    },
                },
                wgpu::Extent3d {
                    width: size,
                    height: size,
                    depth_or_array_layers: 1,
                },
            );
            queue.submit([encoder.finish()]);
            readback.map_async(wgpu::MapMode::Read, .., |r| r.unwrap());
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            let view = readback.get_mapped_range(..).expect("mapped readback");
            let idx = ((32 * size + 32) * 4) as usize;
            let px = [view[idx], view[idx + 1], view[idx + 2], view[idx + 3]];
            drop(view);
            readback.unmap();
            px
        };

        // Frame 1 (submitted): block 0 points to id0 (origin_0 -> RED at screen center).
        let gen1 = flat.block_table_generation(0).expect("page 0 generation");
        let mut enc1 = device.create_command_encoder(&Default::default());
        encode_vis_frame(&mut vis, &flat, &mut enc1);
        assert!(!vis.page_sync.needs_copy(0, gen1));
        let px1 = read_center_pixel(enc1);
        assert!(
            px1[0] > 180 && px1[2] < 60,
            "frame 1 should resolve origin_0 (red): {px1:?}"
        );

        // Remap block 0 to id1 (origin_1 -> BLUE at screen center), advancing the generation.
        let mut update = device.create_command_encoder(&Default::default());
        flat.place(&device, &mut update, &mut uploads, id1, 0, 0, GRAIN);
        uploads.finish(&update);
        queue.submit([update.finish()]);
        let gen2 = flat.block_table_generation(0).expect("updated generation");
        assert_ne!(gen2, gen1);
        assert!(vis.page_sync.needs_copy(0, gen2));

        // Frame 2 (recorded then discarded): resolve_pages records the block-table copy and marks
        // gen2 synced, then the encoder is dropped without submission.
        let mut discarded = device.create_command_encoder(&Default::default());
        encode_vis_frame(&mut vis, &flat, &mut discarded);
        assert!(!vis.page_sync.needs_copy(0, gen2));
        drop(discarded);

        // Recovery via `Visbuffer::forget()` invalidates cached copy records and bind groups.
        vis.forget();
        assert!(vis.page_sync.needs_copy(0, gen2));
        assert!(vis.resolve_cache.get(0).is_none());

        // Frame 3 (submitted): resolve_pages must re-copy page 0's updated block table so the
        // GPU resolves origin_1 (BLUE), not the stale origin_0 (RED) left from Frame 1.
        let mut enc3 = device.create_command_encoder(&Default::default());
        encode_vis_frame(&mut vis, &flat, &mut enc3);
        assert!(!vis.page_sync.needs_copy(0, gen2));
        let px3 = read_center_pixel(enc3);
        assert!(
            px3[2] > 180 && px3[0] < 60,
            "frame 3 after discarded frame must re-copy block table and resolve origin_1 (blue): {px3:?}"
        );
    }
}
