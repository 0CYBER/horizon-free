//! Occlusion culling of solid section draws, in two phases.
//!
//! Frustum and cave culling leave many sections that terrain in front hides
//! entirely (about half of the solid vertices drawn from the ground at render
//! distance 16, more at 32), and drawing fewer vertices is what helps an
//! integrated GPU. Phase 1 draws the sections that the last test read back
//! saw. Then, in the same pass, the box of every other section (and of a
//! share of phase 1's, to notice when they go out of sight) is drawn against
//! phase 1's depth with no writes but to the test's results. Phase 2, in a
//! second pass, draws the sections whose box was seen; the others' draws have
//! an instance count of 0.
//!
//! The picture stays the one without culling: the boxes (grown by a block,
//! the farthest a block model reaches out of its cell) are drawn front faces
//! only, so a fragment is the box's nearest point on its pixel, and the
//! fragment shader runs only where the depth test passes (forced early), so a
//! section is left out only where all of its fragments would fail the depth
//! test against the final depth, which is only nearer. What the CPU knows of
//! the results only sorts sections between the phases: a stale result draws
//! a hidden section in phase 1, or tests a section that is in sight.
//!
//! Every solid draw is an entry of one of the two lists, so the CPU records
//! as many draws as without culling: drivers that unroll an indirect
//! multi-draw on the CPU (Mesa's on Intel Gen9) pay per entry. Phase 2's
//! counts come from the GPU, which needs a native multi-draw and forced
//! early depth tests: Vulkan. Translucent draws are not culled.
use std::num::NonZeroU64;
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};

const WORDS: u64 = 5 * 4;
/// Phase 1's sections are each tested every `RETEST` frames.
const RETEST: u32 = 8;
/// Culling runs while the last result read back hid at least this share of
/// the frame's solid indices: below it the test and the second pass cost
/// more than they save. Else one frame in `PROBE` tests every section.
const MIN_SHARE: f32 = 0.1;
const PROBE: u32 = 64;
/// Boxes over more than this share of the screen count as seen untested.
const BIG: f32 = 0.25;
/// Results read back at once, some frames late so the CPU never waits.
const READBACKS: usize = 3;
const FREE: u8 = 0;
const PENDING: u8 = 1;
const MAPPED: u8 = 2;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    stamp: u32,
    big: f32,
    pad: [u32; 2],
}

/// One frame's results on their way back.
struct Readback {
    buffer: wgpu::Buffer,
    capacity: u64,
    state: Arc<AtomicU8>,
    stamp: u32,
    /// The resident slot of each test, and its section's solid indices.
    slots: Vec<u32>,
    indices: Vec<u32>,
    /// The frame's solid indices.
    total: u64,
}

/// The bind group and the buffers from outside it was made with.
struct Group {
    origins: wgpu::Buffer,
    globals: wgpu::Buffer,
    group: wgpu::BindGroup,
}

pub struct Cull {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    group: Option<Group>,
    params: wgpu::Buffer,
    /// Phase 2's draws: the solid draws after phase 1's in the frame's list.
    pub phase2: wgpu::Buffer,
    phase2_capacity: u64,
    tests: wgpu::Buffer,
    tests_capacity: u64,
    /// This frame's tests: visible index, first phase 2 draw, phase 2 draws.
    test_data: Vec<[u32; 4]>,
    test_slots: Vec<u32>,
    test_indices: Vec<u32>,
    total: u64,
    /// The frame tests every section, as culling was off.
    probe: bool,
    results: wgpu::Buffer,
    readbacks: Vec<Readback>,
    next: usize,
    current: Option<usize>,
    /// Per resident slot: whether the last result read back saw it.
    seen: Vec<bool>,
    stamp: u32,
    /// Of the boxes the last test read back drew, how many it did not see
    /// and how many it drew, and the share of solid indices hidden.
    pub hidden: ([usize; 2], f32),
    /// Cull every frame, whatever the hidden share (A/B checks).
    pub always: bool,
}

fn buffer(
    device: &wgpu::Device,
    label: &str,
    size: u64,
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage,
        mapped_at_creation: false,
    })
}

const PHASE2_USAGE: wgpu::BufferUsages = wgpu::BufferUsages::INDIRECT
    .union(wgpu::BufferUsages::STORAGE)
    .union(wgpu::BufferUsages::COPY_DST);
const TESTS_USAGE: wgpu::BufferUsages =
    wgpu::BufferUsages::STORAGE.union(wgpu::BufferUsages::COPY_DST);
const RESULTS_USAGE: wgpu::BufferUsages =
    wgpu::BufferUsages::STORAGE.union(wgpu::BufferUsages::COPY_SRC);
const READBACK_USAGE: wgpu::BufferUsages =
    wgpu::BufferUsages::MAP_READ.union(wgpu::BufferUsages::COPY_DST);

impl Cull {
    /// The device feature culling needs, where the adapter has it.
    pub fn features(adapter: &wgpu::Adapter) -> wgpu::Features {
        adapter.features() & wgpu::Features::SHADER_EARLY_DEPTH_TEST
    }
    /// Whether the device draws an indirect multi-draw natively and runs
    /// fragment shaders that write storage only past an early depth test.
    pub fn supported(adapter: &wgpu::Adapter, device: &wgpu::Device) -> bool {
        adapter.get_info().backend == wgpu::Backend::Vulkan
            && device.features().contains(
                wgpu::Features::SHADER_EARLY_DEPTH_TEST | wgpu::Features::INDIRECT_FIRST_INSTANCE,
            )
            && adapter.get_downlevel_capabilities().flags.contains(
                wgpu::DownlevelFlags::VERTEX_STORAGE
                    | wgpu::DownlevelFlags::FRAGMENT_WRITABLE_STORAGE,
            )
    }
    /// `color` is the world pass's colour format.
    pub fn new(device: &wgpu::Device, color: wgpu::TextureFormat) -> Self {
        let entry = |binding, visibility, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty,
            count: None,
        };
        let storage = |read_only| wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        };
        let uniform = wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        };
        let both = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let vertex = wgpu::ShaderStages::VERTEX;
        let fragment = wgpu::ShaderStages::FRAGMENT;
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Section culling"),
            entries: &[
                entry(0, both, storage(true)),
                entry(1, fragment, storage(false)),
                entry(2, fragment, storage(false)),
                entry(3, vertex, storage(true)),
                entry(4, vertex, uniform),
                entry(5, both, uniform),
            ],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Section culling"),
            source: wgpu::ShaderSource::Wgsl(include_str!("cull.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Section culling"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Section culling"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_box"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            // The world pass's reversed depth test, without writes.
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_box"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color,
                    blend: None,
                    write_mask: wgpu::ColorWrites::empty(),
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        Self {
            pipeline,
            layout,
            group: None,
            params: buffer(
                device,
                "Section culling parameters",
                std::mem::size_of::<Params>() as u64,
                wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            ),
            phase2: buffer(device, "Section draws, phase 2", WORDS, PHASE2_USAGE),
            phase2_capacity: 1,
            tests: buffer(device, "Section tests", 16, TESTS_USAGE),
            tests_capacity: 1,
            test_data: Vec::new(),
            test_slots: Vec::new(),
            test_indices: Vec::new(),
            total: 0,
            probe: false,
            results: buffer(device, "Section test results", 4, RESULTS_USAGE),
            readbacks: (0..READBACKS)
                .map(|_| Readback {
                    buffer: buffer(device, "Section test readback", 4, READBACK_USAGE),
                    capacity: 1,
                    state: Arc::new(AtomicU8::new(FREE)),
                    stamp: 0,
                    slots: Vec::new(),
                    indices: Vec::new(),
                    total: 0,
                })
                .collect(),
            next: 0,
            current: None,
            seen: Vec::new(),
            stamp: 0,
            hidden: ([0; 2], 1.0),
            always: false,
        }
    }

    /// Start a frame: take in the results that came back, and tell whether
    /// the frame culls.
    pub fn begin(&mut self, device: &wgpu::Device) -> bool {
        let _ = device.poll(wgpu::PollType::Poll);
        for i in 0..READBACKS {
            let r = &mut self.readbacks[(self.next + i) % READBACKS];
            if r.state.load(Ordering::Acquire) != MAPPED {
                continue;
            }
            let mapped = r.buffer.slice(..r.slots.len() as u64 * 4);
            if let Ok(data) = mapped.get_mapped_range() {
                // The mapping isn't guaranteed to be 4-byte aligned (a Vega 11
                // on Windows panicked in cast_slice), so copy the words out.
                let results: Vec<u32> = bytemuck::pod_collect_to_vec(&data);
                let (mut hidden, mut indices) = (0, 0u64);
                for (&result, &n) in results.iter().zip(&r.indices) {
                    if result != r.stamp {
                        hidden += 1;
                        indices += u64::from(n);
                    }
                }
                let share = indices as f32 / r.total.max(1) as f32;
                self.hidden = ([hidden, results.len()], share);
                for (&slot, &result) in r.slots.iter().zip(&results) {
                    let slot = slot as usize;
                    if slot >= self.seen.len() {
                        self.seen.resize(slot + 1, false);
                    }
                    self.seen[slot] = result == r.stamp;
                }
            }
            r.buffer.unmap();
            r.state.store(FREE, Ordering::Release);
        }
        self.stamp = self.stamp.wrapping_add(1).max(1);
        self.test_data.clear();
        self.test_slots.clear();
        self.test_indices.clear();
        self.total = 0;
        self.probe = !self.always && self.hidden.1 < MIN_SHARE;
        !self.probe || self.stamp.is_multiple_of(PROBE)
    }

    /// Whether phase 1 draws the section in resident `slot`.
    pub fn first(&self, slot: u32) -> bool {
        self.seen.get(slot as usize).copied().unwrap_or(false)
    }

    /// A section phase 1 draws, `index` in the visible list, with `indices`
    /// solid indices: tested in turn.
    pub fn drawn(&mut self, index: usize, slot: u32, indices: u32) {
        self.total += u64::from(indices);
        if indices > 0 && (self.probe || slot.wrapping_add(self.stamp).is_multiple_of(RETEST)) {
            self.push([index as u32, 0, 0, 0], slot, indices);
        }
    }

    /// A section left to phase 2: its `draws` from phase 2's `first`.
    pub fn deferred(&mut self, index: usize, slot: u32, first: usize, draws: usize, indices: u32) {
        self.total += u64::from(indices);
        if draws > 0 {
            self.push([index as u32, first as u32, draws as u32, 0], slot, indices);
        }
    }

    fn push(&mut self, test: [u32; 4], slot: u32, indices: u32) {
        self.test_data.push(test);
        self.test_slots.push(slot);
        self.test_indices.push(indices);
    }

    /// Stage this frame's inputs: phase 2's draws, with instance counts of 0,
    /// and the tests, which read the frame's camera-relative `origins` and
    /// view projection (the start of `globals`).
    pub fn upload(
        &mut self,
        device: &wgpu::Device,
        uploads: &mut super::upload::Uploads,
        encoder: &mut wgpu::CommandEncoder,
        phase2: &[[u32; 5]],
        origins: &wgpu::Buffer,
        globals: &wgpu::Buffer,
    ) {
        let draws = phase2.len() as u64;
        if draws > self.phase2_capacity {
            self.phase2_capacity = draws.next_power_of_two();
            self.phase2 = buffer(
                device,
                "Section draws, phase 2",
                self.phase2_capacity * WORDS,
                PHASE2_USAGE,
            );
            self.group = None;
        }
        let tests = self.test_data.len() as u64;
        if tests > self.tests_capacity {
            self.tests_capacity = tests.next_power_of_two();
            self.tests = buffer(
                device,
                "Section tests",
                self.tests_capacity * 16,
                TESTS_USAGE,
            );
            self.results = buffer(
                device,
                "Section test results",
                self.tests_capacity * 4,
                RESULTS_USAGE,
            );
            self.group = None;
        }
        if draws > 0 {
            uploads.buffer(encoder, &self.phase2, 0, bytemuck::cast_slice(phase2));
        }
        if tests > 0 {
            uploads.buffer(
                encoder,
                &self.tests,
                0,
                bytemuck::cast_slice(&self.test_data),
            );
        }
        let params = Params {
            stamp: self.stamp,
            big: BIG,
            pad: [0; 2],
        };
        uploads.buffer(encoder, &self.params, 0, bytemuck::bytes_of(&params));
        if !self
            .group
            .as_ref()
            .is_some_and(|g| g.origins == *origins && g.globals == *globals)
        {
            self.group = Some(self.make_group(device, origins, globals));
        }
    }

    /// In the world pass after phase 1: draw the tested boxes against its
    /// depth. Leaves the pass's pipeline and group 0 changed.
    pub fn test(&self, pass: &mut wgpu::RenderPass<'_>) {
        let Some(Group { group, .. }) = &self.group else {
            return;
        };
        if self.test_data.is_empty() {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, group, &[]);
        pass.draw(0..36, 0..self.test_data.len() as u32);
    }

    /// After the world passes: copy the results for reading back, when a
    /// readback buffer is free.
    pub fn end(&mut self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder) {
        let tests = self.test_data.len() as u64;
        let r = &mut self.readbacks[self.next];
        if tests == 0 || r.state.load(Ordering::Acquire) != FREE {
            return;
        }
        if tests > r.capacity {
            r.capacity = tests.next_power_of_two();
            r.buffer = buffer(
                device,
                "Section test readback",
                r.capacity * 4,
                READBACK_USAGE,
            );
        }
        encoder.copy_buffer_to_buffer(&self.results, 0, &r.buffer, 0, tests * 4);
        r.stamp = self.stamp;
        r.slots.clear();
        r.slots.extend_from_slice(&self.test_slots);
        r.indices.clear();
        r.indices.extend_from_slice(&self.test_indices);
        r.total = self.total;
        self.current = Some(self.next);
    }

    /// After the frame's submission: start reading its results back.
    pub fn submitted(&mut self) {
        let Some(i) = self.current.take() else {
            return;
        };
        let r = &self.readbacks[i];
        let state = r.state.clone();
        state.store(PENDING, Ordering::Release);
        r.buffer
            .slice(..r.slots.len() as u64 * 4)
            .map_async(wgpu::MapMode::Read, move |result| {
                state.store(
                    if result.is_ok() { MAPPED } else { FREE },
                    Ordering::Release,
                );
            });
        self.next = (i + 1) % READBACKS;
    }

    fn make_group(
        &self,
        device: &wgpu::Device,
        origins: &wgpu::Buffer,
        globals: &wgpu::Buffer,
    ) -> Group {
        fn whole(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
            wgpu::BindGroupEntry {
                binding,
                resource: buffer.as_entire_binding(),
            }
        }
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Section culling"),
            layout: &self.layout,
            entries: &[
                whole(0, &self.tests),
                whole(1, &self.results),
                whole(2, &self.phase2),
                whole(3, origins),
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: globals,
                        offset: 0,
                        size: NonZeroU64::new(64),
                    }),
                },
                whole(5, &self.params),
            ],
        });
        Group {
            origins: origins.clone(),
            globals: globals.clone(),
            group,
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn shader_validates() {
        let module = naga::front::wgsl::parse_str(include_str!("cull.wgsl")).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
    }
}
