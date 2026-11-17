//! Hardware mesh shaders, only on request (#17243 step 1): when the player or setting enables
//! `HORIZON_MESH_SHADER=1` and the GPU supports `EXPERIMENTAL_MESH_SHADER`.
//! Without either, the device is opened with the exact same features and limits as `main`.
//! Every build has it: it is an engine render capability for terrain geometry.

/// The mesh shader feature in wgpu.
pub(in crate::gfx) const MESH_SHADER: wgpu::Features = wgpu::Features::EXPERIMENTAL_MESH_SHADER;

/// Whether `device` was opened with mesh shaders.
#[allow(dead_code)]
pub(in crate::gfx) fn on(device: &wgpu::Device) -> bool {
    device.features().contains(MESH_SHADER)
}

/// The environment variable name that controls the mesh shader pipeline.
pub const ENV_VAR: &str = "HORIZON_MESH_SHADER";

/// Who turns mesh shaders on (`HORIZON_MESH_SHADER`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::gfx) enum Switch {
    /// `1`: force on when offered by the adapter.
    On,
    /// Unset or any other value: off (main's path).
    Off,
}

/// Resolves the switch from an optional environment variable string.
pub(in crate::gfx) fn resolve(var: Option<&std::ffi::OsStr>) -> Switch {
    match var {
        Some(v) if v == "1" => Switch::On,
        _ => Switch::Off,
    }
}

/// Reads the current process's switch for mesh shaders.
pub(in crate::gfx) fn switch() -> Switch {
    resolve(std::env::var_os(ENV_VAR).as_deref())
}

/// Features to request from `adapter` when wanted and offered.
pub(super) fn features(adapter: &wgpu::Adapter) -> wgpu::Features {
    let wanted = switch() == Switch::On;
    let picked = pick(wanted, adapter.features());
    if wanted {
        let info = adapter.get_info();
        if picked.is_empty() {
            log::warn!(
                "Horizon renderer: mesh shaders asked for ({ENV_VAR}=1), but {} on {:?} has no \
                 EXPERIMENTAL_MESH_SHADER support; it stays off",
                info.name,
                info.backend
            );
        } else {
            log::info!(
                "Horizon renderer: hardware mesh shaders on ({} on {:?})",
                info.name,
                info.backend
            );
        }
    }
    picked
}

/// Returns the mesh shader feature if requested and offered, or empty otherwise.
pub(super) fn pick(requested: bool, offered: wgpu::Features) -> wgpu::Features {
    if requested {
        offered & MESH_SHADER
    } else {
        wgpu::Features::empty()
    }
}

/// The device's opt-in to `features`' experimental ones.
pub(super) fn experimental(features: wgpu::Features) -> wgpu::ExperimentalFeatures {
    if features.contains(MESH_SHADER) || features.contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY) {
        horizon_hdr::experimental_features()
    } else {
        wgpu::ExperimentalFeatures::disabled()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mesh_shader_switch_resolution() {
        let os = |v: &str| Some(std::ffi::OsString::from(v));
        assert_eq!(resolve(None), Switch::Off);
        assert_eq!(resolve(os("1").as_deref()), Switch::On);
        assert_eq!(resolve(os("0").as_deref()), Switch::Off);
        assert_eq!(resolve(os("true").as_deref()), Switch::Off);
        assert_eq!(resolve(os("").as_deref()), Switch::Off);
        assert_eq!(resolve(os(" 1").as_deref()), Switch::Off);
    }

    #[test]
    fn mesh_shader_only_when_asked_and_offered() {
        let all = wgpu::Features::all();
        assert_eq!(pick(true, all), MESH_SHADER);
        assert!(pick(false, all).is_empty());
        assert!(pick(true, all - MESH_SHADER).is_empty());
        assert!(!experimental(wgpu::Features::empty()).is_enabled());
        assert!(experimental(MESH_SHADER).is_enabled());
    }
}
