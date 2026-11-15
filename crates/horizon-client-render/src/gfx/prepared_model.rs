use super::{EntityPipelineOptions, entity_pipeline_factory, entity_shader};
use crate::Result;
use glam::{DVec3, Mat3, Mat4, Vec3};

/// One corner of a generation-owned prepared face. The instance supplies the pose, tint and light.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PreparedModelVertex {
    pub pos: [f32; 4],
    pub uv: [f32; 4],
    pub normal: [f32; 4],
    /// x is 1 when the face takes the instance tint; y is the face emission level.
    pub flags: [u32; 4],
}

/// One world copy of a prepared model. The pose is camera-relative; tint and light stay in vanilla's space.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PreparedModelInstance {
    pub model: [[f32; 4]; 4],
    pub normal: [[f32; 4]; 3],
    pub tint: [f32; 4],
    pub light: u32,
    pub _pad: [u32; 3],
}

impl PreparedModelInstance {
    /// `local` is the block-corner pose. The camera translation is subtracted in f64.
    pub fn new(local: Mat4, origin: DVec3, camera: DVec3, tint: [f32; 4], light: u32) -> Self {
        let model = camera_matrix(local, origin, camera);
        Self::from_model(model.to_cols_array_2d(), normal_matrix(model), tint, light)
    }

    #[inline]
    fn from_model(model: [[f32; 4]; 4], normal: [[f32; 4]; 3], tint: [f32; 4], light: u32) -> Self {
        Self {
            model,
            normal,
            tint,
            light,
            _pad: [0; 3],
        }
    }

    /// The camera-relative pose stored for this copy.
    pub fn matrix(&self) -> Mat4 {
        Mat4::from_cols_array_2d(&self.model)
    }
}

#[inline]
fn normal_matrix(model: Mat4) -> [[f32; 4]; 3] {
    let normal = Mat3::from_mat4(model).inverse().transpose();
    let cols = normal.to_cols_array_2d();
    let mut rows = [[0.0; 4]; 3];
    for column in 0..3 {
        rows[column][..3].copy_from_slice(&cols[column]);
    }
    rows
}

/// Reuse the inverse transpose for consecutive copies with the same linear pose.
/// This is a batch-local cache, not retained across views or resource generations.
#[derive(Default)]
pub struct PreparedInstanceBuilder {
    previous: Option<PreparedNormal>,
}

struct PreparedNormal {
    basis: [[f32; 4]; 3],
    normal: [[f32; 4]; 3],
}

impl PreparedInstanceBuilder {
    /// Construct the same instance as `PreparedModelInstance::new`, including singular poses.
    #[inline]
    pub fn instance(
        &mut self,
        local: Mat4,
        origin: DVec3,
        camera: DVec3,
        tint: [f32; 4],
        light: u32,
    ) -> PreparedModelInstance {
        let model = camera_matrix(local, origin, camera);
        let columns = model.to_cols_array_2d();
        let basis = [columns[0], columns[1], columns[2]];
        let previous = self.previous.get_or_insert_with(|| PreparedNormal {
            basis,
            normal: normal_matrix(model),
        });
        // Byte equality preserves signed zero and NaN payloads as well as finite poses.
        if bytemuck::bytes_of(&previous.basis) != bytemuck::bytes_of(&basis) {
            previous.basis = basis;
            previous.normal = normal_matrix(model);
        }
        PreparedModelInstance::from_model(columns, previous.normal, tint, light)
    }
}

/// A prepared copy drawn at `cpu_vertices` in the entity stream, before the CPU vertices that follow it.
#[derive(Clone, Copy)]
pub struct PreparedModelDraw {
    pub cpu_vertices: u32,
    pub generation: u64,
    pub slot: u32,
    pub instance: PreparedModelInstance,
}

/// The CPU entity pipelines a prepared draw is ordered against.
pub struct EntityRangePass<'a> {
    pub pipeline: &'a wgpu::RenderPipeline,
    pub hdr_pipeline: &'a wgpu::RenderPipeline,
    pub vertices: &'a wgpu::Buffer,
    pub indices: &'a wgpu::Buffer,
    pub hdr: bool,
}

/// One slice of the ordinary entity stream, then the instanced copies at its end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OrderedSpan {
    pub start: u32,
    pub end: u32,
    pub run: Option<InstanceRun>,
}

/// Consecutive prepared copies drawn as one `draw_indexed`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstanceRun {
    pub first: u32,
    pub count: u32,
}

/// CPU ranges and instanced runs in stream order. Same cut, generation and slot coalesce.
pub fn fill_instance_ranges(
    vertices: u32,
    draws: &[PreparedModelDraw],
    cuts: &[u32],
    offset: u32,
    ranges: &mut Vec<OrderedSpan>,
) {
    ranges.clear();
    debug_assert_eq!(draws.len(), cuts.len());
    let mut cursor = 0u32;
    let mut index = 0usize;
    while index < draws.len() {
        let cut = cuts
            .get(index)
            .copied()
            .unwrap_or(0)
            .saturating_add(offset)
            .min(vertices);
        if cut > cursor {
            ranges.push(OrderedSpan {
                start: cursor,
                end: cut,
                run: None,
            });
        }
        let generation = draws[index].generation;
        let slot = draws[index].slot;
        let first = index;
        index += 1;
        while index < draws.len() {
            let next = cuts
                .get(index)
                .copied()
                .unwrap_or(0)
                .saturating_add(offset)
                .min(vertices);
            if next != cut || draws[index].generation != generation || draws[index].slot != slot {
                break;
            }
            index += 1;
        }
        ranges.push(OrderedSpan {
            start: cut,
            end: cut,
            run: Some(InstanceRun {
                first: first as u32,
                count: (index - first) as u32,
            }),
        });
        cursor = cut;
    }
    if vertices > cursor {
        ranges.push(OrderedSpan {
            start: cursor,
            end: vertices,
            run: None,
        });
    }
}

/// Draw entity ranges and prepared runs in stream order. An HDR mark follows its color draw.
pub fn draw_ordered<'a>(
    pass: &mut wgpu::RenderPass<'a>,
    entities: &EntityRangePass<'a>,
    models: Option<&'a PreparedModels>,
    instances: &'a wgpu::Buffer,
    draws: &[PreparedModelDraw],
    ranges: &[OrderedSpan],
) {
    for span in ranges {
        if span.end / 4 > span.start / 4 {
            let (first, last) = (span.start / 4 * 6, span.end / 4 * 6);
            pass.set_pipeline(entities.pipeline);
            pass.set_vertex_buffer(0, entities.vertices.slice(..));
            pass.set_index_buffer(entities.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(first..last, 0, 0..1);
            if entities.hdr {
                pass.set_pipeline(entities.hdr_pipeline);
                pass.draw_indexed(first..last, 0, 0..1);
            }
        }
        let Some(run) = span.run else {
            continue;
        };
        let Some(models) = models else {
            continue;
        };
        let Some(draw) = draws.get(run.first as usize) else {
            continue;
        };
        let Ok(range) = models.indices(draw.generation, draw.slot) else {
            continue;
        };
        models.draw_run(pass, false, instances, range, run);
        if entities.hdr {
            models.draw_run(pass, true, instances, range, run);
        }
    }
    pass.set_pipeline(entities.pipeline);
    pass.set_vertex_buffer(0, entities.vertices.slice(..));
    pass.set_index_buffer(entities.indices.slice(..), wgpu::IndexFormat::Uint32);
}

/// Camera-relative pose: subtract in f64, then convert the translation to f32.
pub fn camera_matrix(local: Mat4, origin: DVec3, camera: DVec3) -> Mat4 {
    let delta = origin - camera;
    Mat4::from_translation(Vec3::new(delta.x as f32, delta.y as f32, delta.z as f32)) * local
}

fn shader() -> String {
    entity_shader() + include_str!("../prepared_model.wgsl")
}

const VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 4] = wgpu::vertex_attr_array![
    0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Uint32x4
];
const INSTANCE_ATTRIBUTES: [wgpu::VertexAttribute; 9] = wgpu::vertex_attr_array![
    4 => Float32x4, 5 => Float32x4, 6 => Float32x4, 7 => Float32x4,
    8 => Float32x4, 9 => Float32x4, 10 => Float32x4, 11 => Float32x4, 12 => Uint32
];

/// The generation's prepared geometry and the instanced entity pipelines that draw it.
pub struct PreparedModels {
    generation: u64,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    ranges: Vec<Option<(u32, u32)>>,
    pipeline: wgpu::RenderPipeline,
    hdr_pipeline: wgpu::RenderPipeline,
}

impl PreparedModels {
    /// The resource generation these buffers were uploaded for.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The slot's index range. A different generation is refused rather than drawn.
    pub fn indices(&self, generation: u64, slot: u32) -> Result<(u32, u32)> {
        if generation != self.generation {
            return Err("stale prepared model geometry".into());
        }
        self.ranges
            .get(slot as usize)
            .copied()
            .flatten()
            .ok_or_else(|| "prepared model slot has no geometry".into())
    }

    pub fn create(
        device: &wgpu::Device,
        reuse: &super::reuse::Reuse,
        bindings: &wgpu::BindGroupLayout,
        format: wgpu::TextureFormat,
        generation: u64,
        vertices: &[PreparedModelVertex],
        ranges: Vec<Option<(u32, u32)>>,
    ) -> Result<Self> {
        // A failed check or allocation returns before either buffer is published.
        if take_forced_failure() {
            return Err("prepared model upload failed".into());
        }
        if vertices.is_empty() {
            return Err("prepared model geometry is empty".into());
        }
        let indices: Vec<u32> = (0..vertices.len() as u32 / 4)
            .flat_map(|face| {
                [
                    4 * face,
                    4 * face + 1,
                    4 * face + 2,
                    4 * face,
                    4 * face + 2,
                    4 * face + 3,
                ]
            })
            .collect();
        let limit = device.limits().max_buffer_size;
        let vertex_bytes = std::mem::size_of_val(vertices) as u64;
        let index_bytes = std::mem::size_of_val(indices.as_slice()) as u64;
        if vertex_bytes > limit || index_bytes > limit {
            return Err(format!(
                "prepared model buffer is {vertex_bytes} bytes (indices {index_bytes}), above the {limit}-byte limit"
            )
            .into());
        }
        let checks = super::GpuChecks::push(device);
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Prepared model geometry"),
            size: vertex_bytes,
            usage: wgpu::BufferUsages::VERTEX,
            mapped_at_creation: true,
        });
        let index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Prepared model geometry"),
            size: index_bytes,
            usage: wgpu::BufferUsages::INDEX,
            mapped_at_creation: true,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Prepared model instance"),
            source: wgpu::ShaderSource::Wgsl(shader().into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Prepared model instance"),
            bind_group_layouts: &[Some(bindings)],
            immediate_size: 0,
        });
        let buffers = [
            Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<PreparedModelVertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &VERTEX_ATTRIBUTES,
            }),
            Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<PreparedModelInstance>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &INSTANCE_ATTRIBUTES,
            }),
        ];
        let kept = reuse.on(device);
        let make = |label, blend, depth_write, bias, fragment, write_mask| {
            entity_pipeline_factory(&kept, &layout, &layout, &shader, &buffers, format, false)(
                EntityPipelineOptions {
                    label,
                    cull_mode: None,
                    depth_write,
                    compare: wgpu::CompareFunction::GreaterEqual,
                    bias,
                    blend,
                    clouds: false,
                    vertex: "vs_prepared_model",
                    fragment,
                    write_mask,
                },
            )
        };
        let pipeline = make(
            "Prepared model instance",
            wgpu::BlendState::ALPHA_BLENDING,
            true,
            wgpu::DepthBiasState {
                constant: 128,
                slope_scale: 0.0,
                clamp: 0.0,
            },
            "fs_entity",
            wgpu::ColorWrites::ALL,
        );
        // Bias 0, the same as the CPU entity mark (`hdr_mark_pipeline`). The colour
        // pass writes depth at bias 128, so this mark fails the depth test and the
        // world keeps no scene gain. A bias of 128 here made instanced copies brighter
        // in scRGB than the CPU route.
        let hdr_pipeline = make(
            "Prepared model HDR mark",
            wgpu::BlendState::REPLACE,
            false,
            wgpu::DepthBiasState {
                constant: 0,
                slope_scale: 0.0,
                clamp: 0.0,
            },
            "fs_hdr_mark",
            wgpu::ColorWrites::ALPHA,
        );
        let scope_error = checks.pop().map(|error| error.to_string());
        copy_after_scope(scope_error.clone(), || {
            map_bytes(&vertex_buffer, bytemuck::cast_slice(vertices))
        })?;
        copy_after_scope(scope_error, || {
            map_bytes(&index_buffer, bytemuck::cast_slice(&indices))
        })?;
        Ok(Self {
            generation,
            vertices: vertex_buffer,
            indices: index_buffer,
            ranges,
            pipeline,
            hdr_pipeline,
        })
    }

    /// Draw `run.count` copies starting at `run.first`. `hdr` is the alpha mark, not a second mesh.
    pub fn draw_run<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        hdr: bool,
        instance: &'a wgpu::Buffer,
        range: (u32, u32),
        run: InstanceRun,
    ) {
        if run.count == 0 {
            return;
        }
        pass.set_pipeline(if hdr {
            &self.hdr_pipeline
        } else {
            &self.pipeline
        });
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.set_vertex_buffer(1, instance.slice(..));
        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(
            range.0..range.0 + range.1,
            0,
            run.first..run.first.saturating_add(run.count),
        );
    }

    /// Draw one copy from `instance` at `instance_index`. `hdr` is the alpha mark, not a second mesh.
    pub fn draw_at<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        hdr: bool,
        instance: &'a wgpu::Buffer,
        range: (u32, u32),
        instance_index: u32,
    ) {
        self.draw_run(
            pass,
            hdr,
            instance,
            range,
            InstanceRun {
                first: instance_index,
                count: 1,
            },
        );
    }

    /// Draw the first copy. `hdr` selects the alpha mark; it is a second pass, not a different mesh.
    pub fn draw<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        hdr: bool,
        instance: &'a wgpu::Buffer,
        range: (u32, u32),
    ) {
        self.draw_at(pass, hdr, instance, range, 0);
    }
}

#[cfg(feature = "test-support")]
static SLOT_CAP: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(u64::MAX);

#[cfg(feature = "test-support")]
pub fn cap_prepared_instance_slots(slots: u64) {
    SLOT_CAP.store(slots, std::sync::atomic::Ordering::Relaxed);
}

/// `u64::MAX` means the test has not named an uploaded generation.
#[cfg(feature = "test-support")]
static UPLOADED_GENERATION: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(u64::MAX);

/// Pretend prepared geometry for `generation` is resident. `None` clears it.
#[cfg(feature = "test-support")]
pub fn set_test_prepared_generation(generation: Option<u64>) {
    UPLOADED_GENERATION.store(
        generation.unwrap_or(u64::MAX),
        std::sync::atomic::Ordering::Relaxed,
    );
}

/// The generation `set_test_prepared_generation` published, if any.
#[cfg(feature = "test-support")]
pub fn test_prepared_generation() -> Option<u64> {
    let generation = UPLOADED_GENERATION.load(std::sync::atomic::Ordering::Relaxed);
    (generation != u64::MAX).then_some(generation)
}

pub fn prepared_instance_room_for(already: u64, incoming: usize, max_buffer: u64) -> usize {
    let stride = std::mem::size_of::<PreparedModelInstance>() as u64;
    let device = max_instance_slots(stride, max_buffer);
    #[cfg(feature = "test-support")]
    let slots = device.min(SLOT_CAP.load(std::sync::atomic::Ordering::Relaxed));
    #[cfg(not(feature = "test-support"))]
    let slots = device;
    instance_room(already, incoming, slots)
}

#[cfg(feature = "test-support")]
pub fn capped_prepared_room(already: u64, incoming: usize) -> Option<usize> {
    let cap = SLOT_CAP.load(std::sync::atomic::Ordering::Relaxed);
    (cap != u64::MAX).then(|| prepared_instance_room_for(already, incoming, u64::MAX))
}

/// How many instances of `stride` bytes fit in `limit`.
pub fn max_instance_slots(stride: u64, limit: u64) -> u64 {
    if stride == 0 || limit < stride {
        0
    } else {
        limit / stride
    }
}

/// How many of `incoming` copies still fit after `already` are staged.
pub fn instance_room(already: u64, incoming: usize, slots: u64) -> usize {
    usize::try_from(slots.saturating_sub(already).min(incoming as u64)).unwrap_or(incoming)
}

/// Slots to allocate for `count` instances. The size stays within `limit`, and `count` itself must fit.
pub fn instance_slot_capacity(count: u64, stride: u64, limit: u64) -> Result<u64> {
    if count == 0 {
        return Ok(0);
    }
    let max_slots = max_instance_slots(stride, limit);
    if max_slots == 0 {
        return Err(format!(
            "prepared instance of {stride} bytes does not fit the {limit}-byte limit"
        )
        .into());
    }
    if count > max_slots {
        return Err(format!(
            "prepared instances need {count} slots of {stride} bytes, above the {limit}-byte limit ({max_slots} slots)"
        )
        .into());
    }
    Ok(count.next_power_of_two().min(max_slots))
}

/// Prepared copies are drawn into the entity pass, whose target is the frame format.
pub fn prepared_attachment(frame: wgpu::TextureFormat) -> wgpu::TextureFormat {
    frame
}

/// Drop this frame's staged copies. The allocation stays for the next frame.
pub fn finish_staged<T>(draws: &mut Vec<T>) {
    draws.clear();
}

fn copy_after_scope(scope_error: Option<String>, copy: impl FnOnce() -> Result<()>) -> Result<()> {
    if let Some(error) = scope_error {
        return Err(error.into());
    }
    copy()
}

fn map_bytes(buffer: &wgpu::Buffer, contents: &[u8]) -> Result<()> {
    let mut view = buffer
        .get_mapped_range_mut(..)
        .map_err(|error| error.to_string())?;
    view.copy_from_slice(contents);
    drop(view);
    buffer.unmap();
    Ok(())
}

fn take_forced_failure() -> bool {
    #[cfg(feature = "test-support")]
    {
        FAIL_UPLOAD.with(|flag| flag.replace(false))
    }
    #[cfg(not(feature = "test-support"))]
    {
        false
    }
}

/// The next [`PreparedModels::create`] on this thread returns an error and uploads nothing.
/// Another thread's upload is left alone, so parallel pixel tests cannot consume each other's failure.
#[cfg(feature = "test-support")]
pub fn fail_next_prepared_upload() {
    FAIL_UPLOAD.with(|flag| flag.set(true));
}

#[cfg(feature = "test-support")]
thread_local! {
    static FAIL_UPLOAD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytemuck::Zeroable;

    #[test]
    fn batch_normal_reuse_matches_independent_instances() {
        let mut builder = PreparedInstanceBuilder::default();
        let camera = DVec3::new(30_000_000.25, 72.5, -30_000_000.75);
        for local in [
            Mat4::IDENTITY,
            Mat4::from_rotation_y(0.6),
            Mat4::from_scale(Vec3::new(-0.5, 1.25, 2.0)),
            Mat4::from_scale(Vec3::ZERO),
        ] {
            for offset in [0.0, 1.5, 100.25, f64::MAX] {
                let origin = camera + DVec3::splat(offset);
                let tint = [0.5, 0.25, 1.0, 0.125];
                let expected = PreparedModelInstance::new(local, origin, camera, tint, 0xF030);
                let cached = builder.instance(local, origin, camera, tint, 0xF030);
                assert_eq!(bytemuck::bytes_of(&cached), bytemuck::bytes_of(&expected));
            }
        }
    }

    #[test]
    fn camera_translation_keeps_a_fraction_past_f32_precision() {
        let local = Mat4::from_scale(Vec3::new(1.2, 0.5, 1.7));
        let origin = DVec3::new(9_007_199_254.0, 64.0, -9_007_199_254.5);
        let camera = origin + DVec3::new(0.25, -1.5, 2.0);
        let matrix = camera_matrix(local, origin, camera);
        let translation = matrix.w_axis;
        assert!((translation.x + 0.25).abs() < 1.0e-5, "{translation:?}");
        assert!((translation.y - 1.5).abs() < 1.0e-5, "{translation:?}");
        assert!((translation.z + 2.0).abs() < 1.0e-5, "{translation:?}");
        let lost = (origin.as_vec3() - camera.as_vec3()).x;
        assert_eq!(lost, 0.0, "f32 subtraction of this origin is {lost}");
    }

    #[test]
    fn instance_growth_stays_inside_the_buffer_limit() {
        let stride = std::mem::size_of::<PreparedModelInstance>() as u64;
        let slots = instance_slot_capacity(65, stride, 16_384).unwrap();
        assert!(slots >= 65 && slots * stride <= 16_384, "{slots}");
        assert!(instance_slot_capacity(114, stride, 16_384).is_err());
        let wide = 256 << 20;
        let slots = instance_slot_capacity(1_048_577, stride, wide).unwrap();
        assert!(slots >= 1_048_577 && slots * stride <= wide, "{slots}");
    }

    #[test]
    fn prepared_attachment_is_the_frame_not_the_swapchain() {
        let frame = wgpu::TextureFormat::Rgba8UnormSrgb.remove_srgb_suffix();
        assert_eq!(prepared_attachment(frame), wgpu::TextureFormat::Rgba8Unorm);
        assert_ne!(
            prepared_attachment(frame),
            wgpu::TextureFormat::Rgba8UnormSrgb
        );
        assert_eq!(
            prepared_attachment(wgpu::TextureFormat::Rgba16Unorm),
            wgpu::TextureFormat::Rgba16Unorm
        );
    }

    #[test]
    fn finishing_a_frame_drops_staged_draws_and_keeps_capacity() {
        let mut draws = vec![PreparedModelDraw {
            cpu_vertices: 0,
            generation: 1,
            slot: 0,
            instance: PreparedModelInstance::zeroed(),
        }];
        let capacity = draws.capacity();
        finish_staged(&mut draws);
        assert!(draws.is_empty());
        assert!(draws.capacity() >= capacity);
    }

    fn draw_at(slot: u32, generation: u64, cut: u32) -> PreparedModelDraw {
        PreparedModelDraw {
            cpu_vertices: cut,
            generation,
            slot,
            instance: PreparedModelInstance::zeroed(),
        }
    }

    fn cpu(start: u32, end: u32) -> OrderedSpan {
        OrderedSpan {
            start,
            end,
            run: None,
        }
    }

    fn run(at: u32, first: u32, count: u32) -> OrderedSpan {
        OrderedSpan {
            start: at,
            end: at,
            run: Some(InstanceRun { first, count }),
        }
    }

    #[test]
    fn prepared_draws_keep_the_entity_stream_order() {
        let draws = [draw_at(0, 1, 4), draw_at(0, 1, 8)];
        let mut ranges = Vec::new();
        fill_instance_ranges(16, &draws, &[4, 8], 0, &mut ranges);
        assert_eq!(
            ranges,
            vec![cpu(0, 4), run(4, 0, 1), cpu(4, 8), run(8, 1, 1), cpu(8, 16)]
        );
    }

    #[test]
    fn the_same_model_at_one_cut_is_one_run() {
        let draws = [
            draw_at(3, 7, 0),
            draw_at(3, 7, 0),
            draw_at(3, 7, 0),
            draw_at(4, 7, 0),
            draw_at(3, 8, 0),
            draw_at(3, 7, 12),
            draw_at(3, 7, 12),
        ];
        let cuts = [0, 0, 0, 0, 0, 12, 12];
        let mut ranges = Vec::new();
        fill_instance_ranges(20, &draws, &cuts, 0, &mut ranges);
        let runs: Vec<_> = ranges.iter().filter_map(|span| span.run).collect();
        assert_eq!(
            runs,
            vec![
                InstanceRun { first: 0, count: 3 },
                InstanceRun { first: 3, count: 1 },
                InstanceRun { first: 4, count: 1 },
                InstanceRun { first: 5, count: 2 },
            ]
        );
        assert!(
            ranges
                .iter()
                .any(|span| span.run.is_none() && span.end > span.start)
        );
    }

    #[test]
    #[ignore = "daily"]
    fn oversized_prepared_upload_returns_an_error() {
        let _gpu = crate::gfx::test_gpu::lock();
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = wgpu::Backends::VULKAN;
        let instance = wgpu::Instance::new(descriptor);
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            force_fallback_adapter: true,
            ..Default::default()
        }))
        .expect("software Vulkan adapter");
        assert_eq!(
            adapter.get_info().device_type,
            wgpu::DeviceType::Cpu,
            "no software Vulkan adapter"
        );
        let limits = wgpu::Limits {
            max_buffer_size: 16_384,
            ..wgpu::Limits::default()
        };
        let (device, _) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: limits,
            ..Default::default()
        }))
        .expect("software Vulkan device");
        let bindings = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &super::super::entity_binding_entries(),
        });
        let vertices = vec![PreparedModelVertex::zeroed(); 260];
        let Err(error) = PreparedModels::create(
            &device,
            &super::super::reuse::Reuse::default(),
            &bindings,
            wgpu::TextureFormat::Rgba8Unorm,
            1,
            &vertices,
            vec![Some((0, 6))],
        ) else {
            panic!("an oversized prepared upload must return an error");
        };
        let message = error.to_string();
        assert!(
            message.contains("16_640") || message.contains("16640"),
            "{message}"
        );
    }

    #[test]
    fn mapped_copy_runs_only_after_a_clean_scope() {
        let mut mapped = false;
        let error = copy_after_scope(Some("allocation failed".into()), || {
            mapped = true;
            Ok(())
        });
        assert!(
            error.is_err(),
            "a failed allocation must not be treated as uploaded"
        );
        assert!(
            !mapped,
            "mapped memory was touched before the scope was clean"
        );
        copy_after_scope(None, || {
            mapped = true;
            Ok(())
        })
        .unwrap();
        assert!(mapped);
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn forced_upload_failure_stays_on_the_thread_that_set_it() {
        fail_next_prepared_upload();
        let stolen = std::thread::scope(|scope| scope.spawn(take_forced_failure).join().unwrap());
        assert!(!stolen, "another thread consumed the forced upload failure");
        assert!(take_forced_failure());
        assert!(!take_forced_failure());
    }
}
