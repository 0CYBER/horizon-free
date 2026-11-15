//! Hardware ray queries, only on request (#13780, after the #1180 spike): the shader pack picked
//! at startup asks the device for wgpu's `EXPERIMENTAL_RAY_QUERY` with `[ray_tracing]`, when the
//! GPU has it (spec #16011 item 5); `HORIZON_RAY_TRACING` overrides the pack (`Switch`). Without
//! either the device is opened exactly as before. Every build has it: it is a render capability
//! for shader packs (spec #15624), not a Horizon extra.

/// The terrain's acceleration structures (step 2).
#[path = "ray_scene.rs"]
pub(in crate::gfx) mod ray_scene;

/// The ray query feature.
const RAY_QUERY: wgpu::Features = wgpu::Features::EXPERIMENTAL_RAY_QUERY;

/// Whether `device` was opened with ray queries.
pub(in crate::gfx) fn on(device: &wgpu::Device) -> bool {
    device.features().contains(RAY_QUERY)
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
/// section with a BLAS from them (`ray_scene`). Nothing without ray queries, so the off path's
/// pages are main's.
pub(in crate::gfx) fn arena_input(device: &wgpu::Device) -> wgpu::BufferUsages {
    if on(device) {
        wgpu::BufferUsages::STORAGE
    } else {
        wgpu::BufferUsages::empty()
    }
}

/// The index buffer of `quads` quads (triangles 0 1 2 and 0 2 3), which the terrain draws
/// and BLAS builds both read.
pub(in crate::gfx) fn quad_indices(device: &wgpu::Device, quads: u32) -> wgpu::Buffer {
    use wgpu::util::DeviceExt;
    let indices: Vec<u32> = (0..quads)
        .flat_map(|q| {
            let b = q * 4;
            [b, b + 1, b + 2, b, b + 2, b + 3]
        })
        .collect();
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Quad indices"),
        contents: bytemuck::cast_slice(&indices),
        usage: wgpu::BufferUsages::INDEX | blas_input(device),
    })
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

/// This process's switch: the one place the environment is read.
pub(in crate::gfx) fn switch() -> Switch {
    resolve(std::env::var_os("HORIZON_RAY_TRACING").as_deref())
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

/// Whether `switch` wants ray tracing for a pack that declares `[ray_tracing]` (`traces`) or
/// not: the device's ray queries at startup, the scene once that pack loads.
pub(in crate::gfx) fn wanted(switch: Switch, traces: bool) -> bool {
    match switch {
        Switch::Pack => traces,
        Switch::On => true,
        Switch::Off => false,
    }
}

/// The features to request on `adapter`: ray queries, when wanted and offered. `traces`: the
/// shader pack picked at startup declares `[ray_tracing]` (the device can't add them later).
pub(super) fn features(adapter: &wgpu::Adapter, traces: bool) -> wgpu::Features {
    let wanted = wanted(switch(), traces);
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

fn pick(requested: bool, offered: wgpu::Features) -> wgpu::Features {
    if requested {
        offered & RAY_QUERY
    } else {
        wgpu::Features::empty()
    }
}

/// The device's opt-in to `features`' experimental ones.
pub(super) fn experimental(features: wgpu::Features) -> wgpu::ExperimentalFeatures {
    if features.contains(RAY_QUERY) {
        horizon_hdr::experimental_features()
    } else {
        wgpu::ExperimentalFeatures::disabled()
    }
}

/// What ray tracing is on a renderer to a pack that declares `[ray_tracing]`: `offered`, its
/// adapter has ray queries; `enabled`, its device was opened with them.
fn status(switch: Switch, offered: bool, enabled: bool) -> crate::gfx::output::RayStatus {
    use crate::gfx::output::RayStatus;
    match (switch, offered, enabled) {
        (Switch::Off, ..) => RayStatus::Off,
        (_, _, true) => RayStatus::Ready,
        (_, false, _) => RayStatus::Unsupported,
        (_, true, false) => RayStatus::Restart,
    }
}

impl crate::gfx::Renderer {
    /// Its output under the running shader pack (`EffectiveOutput::with_pack`), with what its
    /// ray tracing is to a pack that asks for it.
    pub(in crate::gfx) fn output_with_rays(&self) -> crate::gfx::output::EffectiveOutput {
        let native = self.native.as_ref().map(|n| n.frame.api);
        let output = (self.output).with_pack(self.shader_pack_kind() == Some("iris"), native);
        let offered = self.adapter.features().contains(RAY_QUERY);
        crate::gfx::output::EffectiveOutput {
            rays: Some(status(switch(), offered, on(&self.device))),
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
        use crate::gfx::output::RayStatus::{Off, Ready, Restart, Unsupported};
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
        assert_eq!(status(Switch::Pack, true, true), Ready);
        assert_eq!(status(Switch::Pack, true, false), Restart);
        assert_eq!(status(Switch::Pack, false, false), Unsupported);
        assert_eq!(status(Switch::On, false, false), Unsupported);
        assert_eq!(status(Switch::Off, true, false), Off);
    }

    #[test]
    fn ray_queries_only_when_asked_and_offered() {
        let all = wgpu::Features::all();
        assert_eq!(pick(true, all), RAY_QUERY);
        assert!(pick(false, all).is_empty());
        assert!(pick(true, all - RAY_QUERY).is_empty());
        assert!(!experimental(wgpu::Features::empty()).is_enabled());
    }
}
