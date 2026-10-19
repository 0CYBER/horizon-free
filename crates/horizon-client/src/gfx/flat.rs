//! Flat section draws: where indirect multi-draw is emulated, as on Metal
//! (wgpu replays it as one draw call per indirect entry), the frame's section
//! draws are copied by a compute pass into one index list, in draw order, and
//! drawn with one draw call per run of sections in the same arena page.
//!
//! The picture is the one the per-section draws give: the same triangles
//! from the same vertices in the same order, through the same vertex and
//! fragment code. Only where a vertex finds its section's origin changes:
//! from the instance attribute to a table. A section's vertices fill whole
//! blocks of `GRAIN` vertices of its page (the arena rounds every range up),
//! so each block names one section: its id here, which indexes the frame's
//! camera-relative origins (computed on the CPU exactly as for the instance
//! attribute).
use super::arena::GRAIN;

/// One section draw for `flatten.wgsl`: index count, first sorted index (or
/// `QUADS` for a solid draw), base vertex, first index in the list.
type Range = [u32; 4];
const QUADS: u32 = u32::MAX;
const GROUP_LIMIT: u32 = 65535;

struct PageTable {
    /// Section id of every block of the page.
    blocks: wgpu::Buffer,
    group: Option<wgpu::BindGroup>,
}

/// A run of the frame's index list drawn from one arena page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Call {
    pub page: u32,
    pub first: u32,
    pub count: u32,
}

pub struct Flat {
    pub solid: wgpu::RenderPipeline,
    pub translucent: wgpu::RenderPipeline,
    flatten: wgpu::ComputePipeline,
    compute_layout: wgpu::BindGroupLayout,
    draw_layout: wgpu::BindGroupLayout,
    blocks_per_page: u64,
    pages: Vec<Option<PageTable>>,
    free_ids: Vec<u32>,
    next_id: u32,
    origins: wgpu::Buffer,
    origin_capacity: u64,
    origin_data: Vec<[f32; 4]>,
    /// The frame's index list.
    pub indices: wgpu::Buffer,
    index_capacity: u64,
    ranges: wgpu::Buffer,
    range_capacity: u64,
    range_data: Vec<Range>,
    /// Segments of `range_data` by index source: (sorted index page or
    /// `None` for solid draws, first range, ranges).
    segments: Vec<(Option<u32>, u64, u64)>,
    empty: wgpu::Buffer,
    /// This frame's draw calls: solid ones, then translucent ones.
    pub calls: Vec<Call>,
    pub solid_calls: usize,
    /// Frames with fewer section draws keep the per-section draws.
    pub min_draws: usize,
    range_align: u64,
}

/// Section draws from which a frame is drawn flat. Below it the index list
/// costs the GPU more than the draw calls cost the CPU: on an M4 Pro at
/// render distance 16 (2,900 draws) the copy and the list add about 0.4 ms
/// of GPU time for 0.5–0.8 ms less CPU time; at 32 (9,300 draws) the CPU
/// saves 1.3 ms.
const MIN_DRAWS: usize = 4096;

/// Whether flat draws apply: the backend emulates multi-draw, and vertex
/// shaders can read storage buffers large enough for an index page.
/// `HORIZON_FLAT_DRAWS=1` draws every frame flat, `0` none.
pub fn wanted(adapter: &wgpu::Adapter, device: &wgpu::Device) -> bool {
    match std::env::var("HORIZON_FLAT_DRAWS").as_deref() {
        Ok("0") => return false,
        Ok("1") => return supported(adapter, device),
        _ => {}
    }
    adapter.get_info().backend == wgpu::Backend::Metal && supported(adapter, device)
}

fn supported(adapter: &wgpu::Adapter, device: &wgpu::Device) -> bool {
    let flags = adapter.get_downlevel_capabilities().flags;
    let limits = device.limits();
    flags.contains(wgpu::DownlevelFlags::VERTEX_STORAGE | wgpu::DownlevelFlags::COMPUTE_SHADERS)
        && limits.max_storage_buffers_per_shader_stage >= 3
        && limits.max_storage_buffer_binding_size >= super::arena::PAGE_BYTES
}

fn storage(binding: u32, stage: wgpu::ShaderStages, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: stage,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn buffer(
    device: &wgpu::Device,
    label: &str,
    size: u64,
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: size.max(16),
        usage: usage | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// Lays out the frame's index list: each draw's range (destinations in draw
/// order) grouped by index source into `ranges`, every group padded to
/// `align` entries for binding, with `segments` naming each group; and the
/// draw calls, one per run of draws in the same vertex page, solid and
/// translucent apart. Returns the first translucent call and the list's
/// length.
fn plan(
    draws: &[[u32; 5]],
    pages: &[(u32, Option<u32>)],
    solid_draws: usize,
    align: u64,
    calls: &mut Vec<Call>,
    ranges: &mut Vec<Range>,
    segments: &mut Vec<(Option<u32>, u64, u64)>,
) -> (usize, u64) {
    calls.clear();
    ranges.clear();
    segments.clear();
    let mut solid_calls = None;
    let mut total = 0u64;
    let mut by_source: Vec<(Option<u32>, Vec<Range>)> = Vec::new();
    for (i, (d, &(page, sorted))) in draws.iter().zip(pages).enumerate() {
        let first = if sorted.is_some() { d[2] } else { QUADS };
        let range = [d[0], first, d[3], total as u32];
        match by_source.iter_mut().find(|(s, _)| *s == sorted) {
            Some((_, list)) => list.push(range),
            None => by_source.push((sorted, vec![range])),
        }
        if i == solid_draws {
            solid_calls = Some(calls.len());
        }
        match calls.last_mut() {
            Some(last) if last.page == page && i != solid_draws => last.count += d[0],
            _ => calls.push(Call {
                page,
                first: total as u32,
                count: d[0],
            }),
        }
        total += u64::from(d[0]);
    }
    for (source, list) in by_source {
        segments.push((source, ranges.len() as u64, list.len() as u64));
        ranges.extend(list);
        let padded = (ranges.len() as u64).next_multiple_of(align);
        ranges.resize(padded as usize, [0; 4]);
    }
    (solid_calls.unwrap_or(calls.len()), total)
}

impl Flat {
    /// `pipeline(layout, label, vertex entry, fragment entry, blend)` builds a
    /// terrain pipeline as the per-section ones, without the origin buffer.
    pub fn new(
        device: &wgpu::Device,
        terrain_layout: &wgpu::BindGroupLayout,
        page_vertices: u64,
        pipeline: impl Fn(
            &wgpu::PipelineLayout,
            &str,
            &str,
            Option<wgpu::BlendState>,
        ) -> wgpu::RenderPipeline,
    ) -> Self {
        let vertex = wgpu::ShaderStages::VERTEX;
        let compute = wgpu::ShaderStages::COMPUTE;
        let draw_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Flat section draws"),
            entries: &[storage(0, vertex, true), storage(1, vertex, true)],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Flat terrain layout"),
            bind_group_layouts: &[Some(terrain_layout), Some(&draw_layout)],
            ..Default::default()
        });
        let compute_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Flatten section draws"),
            entries: &[
                storage(0, compute, true),
                storage(1, compute, true),
                storage(2, compute, false),
            ],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Flatten section draws"),
            source: wgpu::ShaderSource::Wgsl(include_str!("flatten.wgsl").into()),
        });
        let flatten = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Flatten section draws"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("Flatten section draws"),
                    bind_group_layouts: &[Some(&compute_layout)],
                    ..Default::default()
                }),
            ),
            module: &module,
            entry_point: Some("flatten"),
            compilation_options: Default::default(),
            cache: None,
        });
        let storage_usage = wgpu::BufferUsages::STORAGE;
        Self {
            solid: pipeline(&layout, "Terrain solid (flat)", "fs_solid", None),
            translucent: pipeline(
                &layout,
                "Terrain translucent (flat)",
                "fs_translucent",
                Some(wgpu::BlendState::ALPHA_BLENDING),
            ),
            flatten,
            compute_layout,
            draw_layout,
            blocks_per_page: page_vertices.div_ceil(GRAIN),
            pages: Vec::new(),
            free_ids: Vec::new(),
            next_id: 0,
            origins: buffer(device, "Section origins by id", 16, storage_usage),
            origin_capacity: 1,
            origin_data: Vec::new(),
            indices: buffer(
                device,
                "Flat section indices",
                16,
                wgpu::BufferUsages::INDEX | storage_usage,
            ),
            index_capacity: 4,
            ranges: buffer(device, "Flat section ranges", 16, storage_usage),
            range_capacity: 1,
            range_data: Vec::new(),
            segments: Vec::new(),
            empty: buffer(device, "No sorted indices", 16, storage_usage),
            calls: Vec::new(),
            solid_calls: 0,
            min_draws: if std::env::var("HORIZON_FLAT_DRAWS").as_deref() == Ok("1") {
                0
            } else {
                MIN_DRAWS
            },
            range_align: (u64::from(device.limits().min_storage_buffer_offset_alignment) / 16)
                .max(1),
        }
    }
    /// A section uploaded to `vertices` vertices from `start` of `page`: its
    /// id, which its blocks now name (written through `uploads` in `encoder`).
    pub fn add(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        uploads: &mut super::upload::Uploads,
        page: u32,
        start: u64,
        vertices: u64,
    ) -> u32 {
        let id = self.free_ids.pop().unwrap_or_else(|| {
            self.next_id += 1;
            self.next_id - 1
        });
        let page = page as usize;
        if self.pages.len() <= page {
            self.pages.resize_with(page + 1, || None);
        }
        let table = self.pages[page].get_or_insert_with(|| PageTable {
            blocks: buffer(
                device,
                "Section ids by block",
                self.blocks_per_page * 4,
                wgpu::BufferUsages::STORAGE,
            ),
            group: None,
        });
        let first = start / GRAIN;
        let ids = vec![id; vertices.div_ceil(GRAIN) as usize];
        uploads.buffer(
            encoder,
            &table.blocks,
            first * 4,
            bytemuck::cast_slice(&ids),
        );
        id
    }
    pub fn remove(&mut self, id: u32) {
        self.free_ids.push(id);
    }
    pub fn clear(&mut self) {
        self.pages.clear();
        self.free_ids.clear();
        self.next_id = 0;
    }
    /// Forgets the tables of pages the arena released.
    pub fn trim(&mut self, live: impl Fn(u32) -> bool) {
        for (i, page) in self.pages.iter_mut().enumerate() {
            if page.is_some() && !live(i as u32) {
                *page = None;
            }
        }
    }
    /// Builds the frame's index list from `draws` (`DrawIndexedIndirectArgs`:
    /// index count, instances, first index, base vertex, first instance),
    /// solid ones first, with their (vertex page, sorted index page) and
    /// the origins of the sections they draw by id. Records the copy into
    /// `encoder`; `calls` then lists the frame's draw calls. False when the
    /// frame cannot be drawn flat (the per-section draws then apply).
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        uploads: &mut super::upload::Uploads,
        draws: &[[u32; 5]],
        pages: &[(u32, Option<u32>)],
        solid_draws: usize,
        origins: impl Iterator<Item = (u32, [f32; 4])>,
        sorted_page: impl Fn(u32) -> wgpu::Buffer,
    ) -> bool {
        // Origins by section id.
        self.origin_data.clear();
        for (id, o) in origins {
            let id = id as usize;
            if self.origin_data.len() <= id {
                self.origin_data.resize(id + 1, [0.0; 4]);
            }
            self.origin_data[id] = o;
        }
        if self.origin_data.len() as u64 > self.origin_capacity {
            self.origin_capacity = (self.origin_data.len() as u64).next_power_of_two();
            self.origins = buffer(
                device,
                "Section origins by id",
                self.origin_capacity * 16,
                wgpu::BufferUsages::STORAGE,
            );
            for page in self.pages.iter_mut().flatten() {
                page.group = None;
            }
        }
        if !self.origin_data.is_empty() {
            uploads.buffer(
                encoder,
                &self.origins,
                0,
                bytemuck::cast_slice(&self.origin_data),
            );
        }
        let (solid_calls, total) = plan(
            draws,
            pages,
            solid_draws,
            self.range_align,
            &mut self.calls,
            &mut self.range_data,
            &mut self.segments,
        );
        self.solid_calls = solid_calls;
        if total == 0 || total * 4 > device.limits().max_storage_buffer_binding_size {
            self.calls.clear();
            self.solid_calls = 0;
            return false;
        }
        if self.range_data.len() as u64 > self.range_capacity {
            self.range_capacity = (self.range_data.len() as u64).next_power_of_two();
            self.ranges = buffer(
                device,
                "Flat section ranges",
                self.range_capacity * 16,
                wgpu::BufferUsages::STORAGE,
            );
        }
        if total > self.index_capacity {
            self.index_capacity = total.next_power_of_two();
            self.indices = buffer(
                device,
                "Flat section indices",
                self.index_capacity * 4,
                wgpu::BufferUsages::INDEX | wgpu::BufferUsages::STORAGE,
            );
        }
        uploads.buffer(
            encoder,
            &self.ranges,
            0,
            bytemuck::cast_slice(&self.range_data),
        );
        let groups: Vec<(wgpu::BindGroup, u32)> = self
            .segments
            .iter()
            .map(|&(source, first, count)| {
                let sorted = source.map(&sorted_page);
                let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Flatten section draws"),
                    layout: &self.compute_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                buffer: &self.ranges,
                                offset: first * 16,
                                size: wgpu::BufferSize::new(count * 16),
                            }),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: sorted.as_ref().unwrap_or(&self.empty).as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: self.indices.as_entire_binding(),
                        },
                    ],
                });
                (group, count as u32)
            })
            .collect();
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Flatten section draws"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.flatten);
        for (group, count) in &groups {
            pass.set_bind_group(0, group, &[]);
            pass.dispatch_workgroups((*count).min(GROUP_LIMIT), count.div_ceil(GROUP_LIMIT), 1);
        }
        drop(pass);
        for page in self.calls.iter().map(|c| c.page) {
            let table = self.pages[page as usize]
                .as_mut()
                .expect("a drawn page has its block table");
            if table.group.is_none() {
                table.group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Flat section draws"),
                    layout: &self.draw_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: table.blocks.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: self.origins.as_entire_binding(),
                        },
                    ],
                }));
            }
        }
        true
    }
    /// Draws `calls` with a flat pipeline set and `vertices` giving each
    /// page's vertex buffer.
    pub fn draw<'a>(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        calls: &[Call],
        vertices: impl Fn(u32) -> &'a wgpu::Buffer,
    ) {
        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
        for call in calls {
            let table = self.pages[call.page as usize]
                .as_ref()
                .and_then(|t| t.group.as_ref())
                .expect("prepared page");
            pass.set_vertex_buffer(0, vertices(call.page).slice(..));
            pass.set_bind_group(1, table, &[]);
            pass.draw_indexed(call.first..call.first + call.count, 0, 0..1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Draws as `render` writes them: solid (page, quads at a base vertex)
    /// first, then translucent (page, sorted page, count, first sorted index,
    /// base vertex).
    fn sample() -> (Vec<[u32; 5]>, Vec<(u32, Option<u32>)>, usize) {
        let draws = vec![
            [6 * 3, 1, 0, 0, 0],
            [6 * 2, 1, 0, 512, 1],
            [6, 1, 0, 256, 2],
            [6 * 4, 1, 0, 1024, 3],
            [6 * 2, 1, 6, 768, 1],
            [6, 1, 0, 40, 0],
        ];
        let pages = vec![
            (0, None),
            (0, None),
            (1, None),
            (0, None),
            (0, Some(0)),
            (0, Some(1)),
        ];
        (draws, pages, 4)
    }

    #[test]
    fn plan_draws_runs_of_a_page_in_order() {
        let (draws, pages, solid) = sample();
        let (mut calls, mut ranges, mut segments) = (Vec::new(), Vec::new(), Vec::new());
        let (solid_calls, total) = plan(
            &draws,
            &pages,
            solid,
            4,
            &mut calls,
            &mut ranges,
            &mut segments,
        );
        assert_eq!(total, 6 * 13);
        let call = |page, first, count| Call { page, first, count };
        // Pages 0, 0 | 1 | 0, then translucent 0, 0 (never merged with solid).
        assert_eq!(
            calls,
            [
                call(0, 0, 30),
                call(1, 30, 6),
                call(0, 36, 24),
                call(0, 60, 18)
            ]
        );
        assert_eq!(solid_calls, 3);
        // Solid ranges, then each sorted page's, every group padded to 4.
        assert_eq!(segments, [(None, 0, 4), (Some(0), 4, 1), (Some(1), 8, 1)]);
        assert_eq!(ranges[3], [24, QUADS, 1024, 36]);
        assert_eq!(ranges[4], [12, 6, 768, 60]);
        assert_eq!(ranges[8], [6, 0, 40, 72]);
        assert_eq!(ranges.len(), 12);
    }

    /// The compute pass writes exactly the indices the per-section draws
    /// read: quads 0 1 2 0 2 3 for solid draws, sorted indices otherwise,
    /// plus the base vertex.
    #[test]
    fn flatten_writes_the_indices_of_the_per_section_draws() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let Ok((device, queue)) = pollster::block_on(adapter.request_device(&Default::default()))
        else {
            eprintln!("no GPU device: skipped");
            return;
        };
        use wgpu::util::DeviceExt;
        let (draws, pages, solid) = sample();
        let align = (u64::from(device.limits().min_storage_buffer_offset_alignment) / 16).max(1);
        let (mut calls, mut ranges, mut segments) = (Vec::new(), Vec::new(), Vec::new());
        let (_, total) = plan(
            &draws,
            &pages,
            solid,
            align,
            &mut calls,
            &mut ranges,
            &mut segments,
        );
        let sorted: [Vec<u32>; 2] = [
            (0..32).map(|i| (i * 7) % 32).collect(),
            (0..8).rev().collect(),
        ];
        let mut expected = Vec::new();
        for (d, (_, page)) in draws.iter().zip(&pages) {
            for k in 0..d[0] {
                let i = match page {
                    None => (k / 6) * 4 + [0, 1, 2, 0, 2, 3][(k % 6) as usize],
                    Some(p) => sorted[*p as usize][(d[2] + k) as usize],
                };
                expected.push(d[3] + i);
            }
        }
        let storage = |entries: &[u32]| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: bytemuck::cast_slice(entries),
                usage: wgpu::BufferUsages::STORAGE,
            })
        };
        let range_buffer = storage(bytemuck::cast_slice(&ranges));
        let sorted_buffers = [storage(&sorted[0]), storage(&sorted[1]), storage(&[0; 4])];
        let out = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: total * 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let read = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: total * 4,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(include_str!("flatten.wgsl").into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: None,
            layout: None,
            module: &module,
            entry_point: Some("flatten"),
            compilation_options: Default::default(),
            cache: None,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            for &(source, first, count) in &segments {
                let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: None,
                    layout: &pipeline.get_bind_group_layout(0),
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                buffer: &range_buffer,
                                offset: first * 16,
                                size: wgpu::BufferSize::new(count * 16),
                            }),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: sorted_buffers[source.map_or(2, |p| p as usize)]
                                .as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: out.as_entire_binding(),
                        },
                    ],
                });
                pass.set_bind_group(0, &group, &[]);
                pass.dispatch_workgroups(count as u32, 1, 1);
            }
        }
        encoder.copy_buffer_to_buffer(&out, 0, &read, 0, total * 4);
        queue.submit([encoder.finish()]);
        read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let got: Vec<u32> =
            bytemuck::cast_slice(&read.slice(..).get_mapped_range().unwrap()).to_vec();
        assert_eq!(got, expected);
    }
}
