//! Hardware ray queries, only on request (#13780, after the #1180 spike): the shader pack picked
//! at startup asks the device for wgpu's `EXPERIMENTAL_RAY_QUERY` with `[ray_tracing]`, when the
//! GPU has it (spec #16011 item 5); `HORIZON_RAY_TRACING` overrides the pack (`Switch`). Without
//! either the device is opened exactly as before. Every build has it: it is a render capability
//! for shader packs (spec #15624), not a Horizon extra.

/// The ray query feature.
const RAY_QUERY: wgpu::Features = wgpu::Features::EXPERIMENTAL_RAY_QUERY;

/// Whether `device` was opened with ray queries.
pub(in crate::gfx) fn on(device: &wgpu::Device) -> bool {
    device.features().contains(RAY_QUERY)
}

impl crate::gfx::Renderer {
    /// Whether the device traces rays (`on`): a pack's ray-only data is worth building.
    pub fn ray_queries(&self) -> bool {
        on(&self.device)
    }
}

/// What buffers that BLAS builds read (the quad indices) need on top of their own usage:
/// nothing without ray queries, so the off path's buffers are main's.
pub(in crate::gfx) fn blas_input(device: &wgpu::Device) -> wgpu::BufferUsages {
    if on(device) {
        wgpu::BufferUsages::BLAS_INPUT
    } else {
        wgpu::BufferUsages::empty()
    }
}

/// What the arena's vertex pages need on top of their own usage: the unpack pass reads every
/// section with a BLAS from them (`scene`). Nothing without ray queries, so the off path's
/// pages are main's.
pub(in crate::gfx) fn arena_input(device: &wgpu::Device) -> wgpu::BufferUsages {
    if on(device) {
        wgpu::BufferUsages::STORAGE
    } else {
        wgpu::BufferUsages::empty()
    }
}

/// The label those buffers carry in wgpu's messages, and the one the crash report shows.
const QUAD_INDICES: &str = "Quad indices";

/// The index buffer of `quads` quads (triangles 0 1 2 and 0 2 3), which the terrain draws
/// and BLAS builds both read. `None` when the device would not take it: a lost device, and
/// one out of memory, answer the creation with a buffer that is already invalid, and
/// wgpu's `DeviceExt::create_buffer_init` panics on the mapped range of one (#15464). The
/// caller leaves the draw it was for out, or reports the loss, rather than the render
/// thread dying.
pub(in crate::gfx) fn quad_indices(device: &wgpu::Device, quads: u32) -> Option<wgpu::Buffer> {
    let indices = quad_index_data(quads);
    // The scopes hold what the device made of the creation: outside them wgpu reports an
    // invalid buffer as an uncaptured error, which panics.
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    let buffer = filled(
        device,
        QUAD_INDICES,
        bytemuck::cast_slice(&indices),
        wgpu::BufferUsages::INDEX | blas_input(device),
    );
    let error = pollster::block_on(memory.pop()).or_else(|| pollster::block_on(validation.pop()));
    match error {
        None => buffer,
        Some(e) => {
            log::warn!("Horizon renderer: no {QUAD_INDICES} buffer ({e})");
            None
        }
    }
}

/// `quad_indices` for a frame that grows them (#13624): made unmapped and written through
/// `queue`, so `frame_targets::made` sees what the device made of it — a buffer mapped at
/// creation fails before a scope can — and the frame is given up where there was no room.
pub(in crate::gfx) fn written_quad_indices(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    quads: u32,
) -> wgpu::Buffer {
    let indices = quad_index_data(quads);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(QUAD_INDICES),
        size: std::mem::size_of_val(indices.as_slice()) as u64,
        usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST | blas_input(device),
        mapped_at_creation: false,
    });
    queue.write_buffer(&buffer, 0, bytemuck::cast_slice(&indices));
    buffer
}

fn quad_index_data(quads: u32) -> Vec<u32> {
    (0..quads)
        .flat_map(|q| {
            let b = q * 4;
            [b, b + 1, b + 2, b, b + 2, b + 3]
        })
        .collect()
}

/// `contents` in a buffer of `usage`, made as `DeviceExt::create_buffer_init` makes it: the
/// size rounded up to `COPY_BUFFER_ALIGNMENT`, the data written through the mapping the
/// creation takes. `None` when the device refuses the buffer, which `create_buffer_init`
/// reports by panicking on the mapped range instead.
fn filled(
    device: &wgpu::Device,
    label: &str,
    contents: &[u8],
    usage: wgpu::BufferUsages,
) -> Option<wgpu::Buffer> {
    let unpadded_size = contents.len() as wgpu::BufferAddress;
    if unpadded_size == 0 {
        return Some(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: 0,
            usage,
            mapped_at_creation: false,
        }));
    }
    // Valid Vulkan usage: a size that is a multiple of `COPY_BUFFER_ALIGNMENT`, above zero.
    let align_mask = wgpu::COPY_BUFFER_ALIGNMENT - 1;
    let padded_size = (unpadded_size + align_mask) & !align_mask;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: padded_size,
        usage,
        mapped_at_creation: true,
    });
    let mut view = buffer.slice(..).get_mapped_range_mut().ok()?;
    view.slice(..unpadded_size as usize)
        .copy_from_slice(contents);
    drop(view);
    buffer.unmap();
    Some(buffer)
}
/// Who turns ray tracing on (`HORIZON_RAY_TRACING`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::gfx) enum Switch {
    /// Unset: a shader pack with `[ray_tracing]`.
    Pack,
    /// `1`: always, with a scene even without a pack (the tools' and proofs' switch).
    On,
    /// Any other value: nothing.
    Off,
}

/// Effective switch combining `HORIZON_RAY_TRACING` (when set, it overrides the setting)
/// and the player's `Graphics::ray_tracing` setting.
pub(in crate::gfx) fn switch_with(mode: crate::graphics::RayTracingMode) -> Switch {
    resolve_with(std::env::var_os("HORIZON_RAY_TRACING").as_deref(), mode)
}

/// The switch `HORIZON_RAY_TRACING` set to `var` makes: only `1` forces it on, and a value that
/// is not `1` (`0`, empty, anything else) forces it off rather than leaving it to the pack.
fn resolve(var: Option<&std::ffi::OsStr>) -> Switch {
    match var {
        None => Switch::Pack,
        Some(v) if v == "1" => Switch::On,
        Some(_) => Switch::Off,
    }
}

fn resolve_with(var: Option<&std::ffi::OsStr>, mode: crate::graphics::RayTracingMode) -> Switch {
    if var.is_some() {
        return resolve(var);
    }
    match mode {
        crate::graphics::RayTracingMode::Auto => Switch::Pack,
        crate::graphics::RayTracingMode::On => Switch::On,
        crate::graphics::RayTracingMode::Off => Switch::Off,
    }
}

/// Whether `switch` wants ray tracing for a pack that declares `[ray_tracing]` (`traces`) or
/// not: the device's ray queries at startup, the scene once that pack loads.
pub(in crate::gfx) fn wanted(switch: Switch, traces: bool) -> bool {
    match switch {
        Switch::Pack => traces,
        Switch::On => true,
        Switch::Off => false,
    }
}

/// The features to request on `adapter`: ray queries, when `wanted` and offered.
pub(in crate::gfx) fn features(adapter: &wgpu::Adapter, wanted: bool) -> wgpu::Features {
    let features = pick(wanted, adapter.features());
    if wanted {
        let info = adapter.get_info();
        if features.is_empty() {
            log::warn!(
                "Horizon renderer: ray tracing asked for, but {} on {:?} has no hardware ray \
                 queries; it stays off",
                info.name,
                info.backend
            );
        } else {
            log::info!(
                "Horizon renderer: ray queries on ({} on {:?})",
                info.name,
                info.backend
            );
        }
    }
    features
}

pub(super) fn pick(requested: bool, offered: wgpu::Features) -> wgpu::Features {
    if requested {
        offered & RAY_QUERY
    } else {
        wgpu::Features::empty()
    }
}

/// The device's opt-in to `features`' experimental ones.
#[cfg(test)]
pub(super) fn experimental(features: wgpu::Features) -> wgpu::ExperimentalFeatures {
    if features.contains(RAY_QUERY) {
        horizon_hdr::experimental_features()
    } else {
        wgpu::ExperimentalFeatures::disabled()
    }
}

/// What ray tracing is on a renderer to a pack that declares `[ray_tracing]`: `offered`, its
/// adapter has ray queries; `enabled`, its device was opened with them; `until_asked`, its
/// adapter is on a compiler without them only because the start didn't ask for them
/// (`dx12_compiler::fxc_until_asked`).
fn status(
    switch: Switch,
    offered: bool,
    enabled: bool,
    until_asked: bool,
) -> crate::gfx::output::RayStatus {
    use crate::gfx::output::RayStatus;
    match (switch, offered, enabled) {
        (Switch::Off, ..) => RayStatus::Off,
        (_, _, true) => RayStatus::Ready,
        (_, false, _) if until_asked => RayStatus::Compiler,
        (_, false, _) => RayStatus::Unsupported,
        (_, true, false) => RayStatus::Restart,
    }
}

impl crate::gfx::Renderer {
    /// Its output under the running shader pack (`EffectiveOutput::with_pack`), with what its
    /// ray tracing is to a pack that asks for it.
    pub(in crate::gfx) fn output_with_rays(&self) -> crate::gfx::output::EffectiveOutput {
        let output = (self.output).with_pack(self.shader_pack_kind().is_some());
        let offered = self.adapter.features().contains(RAY_QUERY);
        crate::gfx::output::EffectiveOutput {
            rays: Some(status(
                switch_with(self.ray_tracing_mode),
                offered,
                on(&self.device),
                crate::gfx::dx12_compiler::fxc_until_asked(),
            )),
            ..output
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spec #16011 item 5: unset, the pack decides; `1` forces it on, with a scene without a
    /// pack; any other value forces it off.
    #[test]
    fn the_pack_turns_ray_tracing_on_unless_the_variable_says_otherwise() {
        use crate::gfx::output::RayStatus::{Compiler, Off, Ready, Restart, Unsupported};
        let os = |v: &str| Some(std::ffi::OsString::from(v));
        let cases = [
            (None, Switch::Pack),
            (os("1"), Switch::On),
            (os("0"), Switch::Off),
            (os(""), Switch::Off),
            (os("true"), Switch::Off),
            (os(" 1"), Switch::Off),
        ];
        for (var, switch) in cases {
            assert_eq!(resolve(var.as_deref()), switch, "{var:?}");
        }
        // Wanted: for a pack that declares `[ray_tracing]`, unless overridden.
        assert!(wanted(Switch::Pack, true) && !wanted(Switch::Pack, false));
        assert!(wanted(Switch::On, false) && !wanted(Switch::Off, true));
        // (switch, offered, enabled) -> what the shader pack screen says
        assert_eq!(status(Switch::Pack, true, true, false), Ready);
        assert_eq!(status(Switch::Pack, true, false, false), Restart);
        assert_eq!(status(Switch::Pack, false, false, false), Unsupported);
        assert_eq!(status(Switch::On, false, false, false), Unsupported);
        assert_eq!(status(Switch::Off, true, false, false), Off);
        // A start on DirectX 12's FXC that didn't ask: the next one, asked, compiles with DXC.
        assert_eq!(status(Switch::On, false, false, true), Compiler);
        assert_eq!(status(Switch::Off, false, false, true), Off);
    }

    #[test]
    fn graphics_ray_tracing_setting_respects_env_override_and_modes() {
        use crate::graphics::RayTracingMode;
        let os = |v: &str| Some(std::ffi::OsString::from(v));
        assert_eq!(resolve_with(None, RayTracingMode::Auto), Switch::Pack);
        assert_eq!(resolve_with(None, RayTracingMode::On), Switch::On);
        assert_eq!(resolve_with(None, RayTracingMode::Off), Switch::Off);
        // Environment override always takes precedence over the persisted setting.
        assert_eq!(
            resolve_with(os("1").as_deref(), RayTracingMode::Off),
            Switch::On
        );
        assert_eq!(
            resolve_with(os("0").as_deref(), RayTracingMode::On),
            Switch::Off
        );
        assert_eq!(
            resolve_with(os("0").as_deref(), RayTracingMode::Auto),
            Switch::Off
        );
    }

    #[test]
    fn ray_queries_only_when_asked_and_offered() {
        let all = wgpu::Features::all();
        assert_eq!(pick(true, all), RAY_QUERY);
        assert!(pick(false, all).is_empty());
        assert!(pick(true, all - RAY_QUERY).is_empty());
        assert!(!experimental(wgpu::Features::empty()).is_enabled());
    }

    /// #15464: 65,536 quads is the terrain's first index buffer, 1.5 MiB (the one the crash
    /// report names). A device that will not take it — a lost one and one out of memory
    /// answer the same way — makes `quad_indices` report it, where
    /// `DeviceExt::create_buffer_init` panics on the mapped range of the invalid buffer, on
    /// the render thread.
    #[test]
    fn a_quad_index_buffer_the_device_will_not_take_is_reported_not_panicked_on() {
        let _gpu = crate::gfx::test_gpu::lock();
        let instance = crate::gfx::test_gpu::instance();
        let Some(adapter) = pollster::block_on(instance.request_adapter(&Default::default())).ok()
        else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        // Every limit the adapter has, but buffers of 1 MiB: the creation below is refused
        // the way a lost device refuses it, with an invalid buffer rather than an error.
        let mut limits = adapter.limits();
        limits.max_buffer_size = 1 << 20;
        limits.max_storage_buffer_binding_size =
            limits.max_storage_buffer_binding_size.min(1 << 20);
        let desc = wgpu::DeviceDescriptor {
            label: Some("Quad indices test"),
            required_limits: limits,
            ..Default::default()
        };
        let Ok((device, _queue)) = pollster::block_on(adapter.request_device(&desc)) else {
            eprintln!("no GPU device: skipped");
            return;
        };
        // 65,536 quads at 6 indices of 4 bytes each: 1.5 MiB, past the device's 1 MiB.
        assert!(quad_indices(&device, 1 << 16).is_none());
        // A buffer that fits is still made: the refusal above is the device's, not a call
        // that never works.
        assert!(quad_indices(&device, 4).is_some());
    }
}
