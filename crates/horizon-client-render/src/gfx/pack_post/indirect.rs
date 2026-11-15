//! Snapshot an SSBO's dispatch arguments, then enforce the device's work-group limit.
//!
//! Separate copy/compute/dispatch scopes let a program write the very SSBO from which its
//! arguments came. No buffer is simultaneously writable storage and an indirect argument.

#[derive(Clone)]
pub(super) struct Dispatch {
    pub(super) request: horizon_shaders::IndirectDispatch,
    snapshot: wgpu::Buffer,
    pub(super) arguments: wgpu::Buffer,
    pipeline: wgpu::ComputePipeline,
    group: wgpu::BindGroup,
}

impl Dispatch {
    pub(super) fn new(device: &wgpu::Device, request: horizon_shaders::IndirectDispatch) -> Self {
        let buffer = |label, usage| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: 12,
                usage,
                mapped_at_creation: false,
            })
        };
        let snapshot = buffer(
            "Indirect argument snapshot",
            wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::STORAGE,
        );
        let arguments = buffer(
            "Bounded indirect arguments",
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::INDIRECT,
        );
        let limit = device.limits().max_compute_workgroups_per_dimension;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Bound indirect compute dispatch"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "@group(0) @binding(0) var<storage,read> args_in: array<u32,3>;\n\
                 @group(0) @binding(1) var<storage,read_write> args_out: array<u32,3>;\n\
                 @compute @workgroup_size(1) fn main() {{\n\
                     let counts = vec3u(args_in[0],args_in[1],args_in[2]);\n\
                     let valid = all(counts <= vec3u({limit}u));\n\
                     let bounded = select(vec3u(0u),counts,valid);\n\
                     args_out[0]=bounded.x; args_out[1]=bounded.y; args_out[2]=bounded.z;\n\
                 }}"
                )
                .into(),
            ),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Bound indirect compute dispatch"),
            layout: None,
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Indirect argument bounds"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: snapshot.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: arguments.as_entire_binding(),
                },
            ],
        });
        Self {
            request,
            snapshot,
            arguments,
            pipeline,
            group,
        }
    }

    pub(super) fn prepare(&self, encoder: &mut wgpu::CommandEncoder, source: &wgpu::Buffer) {
        encoder.copy_buffer_to_buffer(source, self.request.offset, &self.snapshot, 0, 12);
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Bound indirect compute dispatch"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.group, &[]);
        pass.dispatch_workgroups(1, 1, 1);
    }
}
