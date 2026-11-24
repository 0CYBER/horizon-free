//! Which compiler turns DirectX 12's shaders into bytecode: DXC on integrated
//! GPUs and on a discrete GPU asked for ray queries or mesh shaders, FXC
//! otherwise (#8513).
//!
//! Measured on one laptop at render distance 32 (2560x1440, the same world and
//! view): DXC took Intel Iris Xe from 44 to 65 FPS (22.1 to 14.7 ms of GPU
//! time) and an NVIDIA RTX 3050 Ti from 159 down to 150 FPS. An RX 9070 gained
//! about 9 % of GPU time at render distance 32 and lost 0.03 ms at 8. So
//! discrete GPUs keep FXC (`d3dcompiler_47.dll`, part of Windows), and only
//! integrated ones load DXC (`dxcompiler.dll`, which the Windows x64 zip puts
//! beside the game: `tools/ci/fetch_dxc.py`). The exception is a discrete GPU
//! asked for ray queries or mesh shaders: those need Shader Model 6.5, which
//! FXC (Shader Model 5.1) can't compile, so it loads DXC too. Its ordinary
//! play, without either, keeps FXC.
//!
//! wgpu chooses the compiler for a whole instance, before any adapter is
//! known. `open_backend` therefore asks for the adapter on an FXC instance and,
//! when that adapter is an integrated GPU on DirectX 12 or a discrete one asked
//! for Shader Model 6.5 (`wants_dxc`), asks again on an instance with DXC. That happens once, when the device is opened; nothing
//! here runs per frame. Without the DLL, or when it does not load, the game
//! keeps the FXC instance and logs one line. `WGPU_DX12_COMPILER` (`fxc`,
//! `dxc`, `auto`) overrides the choice: wgpu applies it to every instance.

use std::path::PathBuf;

/// The DLL's name, as the Windows zip ships it beside `horizon-client.exe`.
pub(super) const DLL: &str = "dxcompiler.dll";

/// Whether `info`, opened on an FXC instance, should be opened again with DXC:
/// an integrated GPU on DirectX 12, or a discrete one when the device will be
/// asked for Shader Model 6.5 features (`sm_6_5`: ray queries or mesh shaders),
/// unless `forced` (`WGPU_DX12_COMPILER` is set, so its compiler is already the
/// one every instance uses).
pub(super) fn wants_dxc(info: &wgpu::AdapterInfo, forced: bool, sm_6_5: bool) -> bool {
    !forced
        && info.backend == wgpu::Backend::Dx12
        && match info.device_type {
            wgpu::DeviceType::IntegratedGpu => true,
            wgpu::DeviceType::DiscreteGpu => sm_6_5,
            _ => false,
        }
}

/// Whether this start opened `info` on FXC only because nothing asked for Shader Model 6.5
/// (`sm_6_5`), with the DLL beside the game (`dll`): a start that asks for it gets DXC.
pub(super) fn dxc_on_request(
    info: &wgpu::AdapterInfo,
    forced: bool,
    sm_6_5: bool,
    dll: bool,
) -> bool {
    dll && !wants_dxc(info, forced, sm_6_5) && wants_dxc(info, forced, true)
}

/// `dxc_on_request` for the device opened last: its adapter offers no ray queries on FXC, but
/// the next start, asked for them, may (`rt::query`'s status tells the two apart).
static FXC_UNTIL_ASKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Records `dxc_on_request` for the device being opened.
pub(super) fn note_fxc_until_asked(until_asked: bool) {
    FXC_UNTIL_ASKED.store(until_asked, std::sync::atomic::Ordering::Relaxed);
}

/// Whether the device opened last kept FXC only because nothing asked for Shader Model 6.5.
pub(in crate::gfx) fn fxc_until_asked() -> bool {
    FXC_UNTIL_ASKED.load(std::sync::atomic::Ordering::Relaxed)
}

/// Whether `WGPU_DX12_COMPILER` names the compiler.
pub(super) fn forced() -> bool {
    std::env::var_os("WGPU_DX12_COMPILER").is_some()
}

/// `dxcompiler.dll` beside the running executable, if it is there. An absolute
/// path, so a copy elsewhere on `PATH` (an older Windows SDK's) is never the
/// one loaded.
pub(super) fn bundled_dll() -> Option<PathBuf> {
    let dll = std::env::current_exe().ok()?.parent()?.join(DLL);
    dll.is_file().then_some(dll)
}

/// The compiler setting that loads `dll`, and fails the instance (rather
/// than falling back to FXC on its own) when it doesn't load: `open_backend`
/// then keeps the FXC instance it already has.
pub(super) fn dxc(dll: &std::path::Path) -> wgpu::Dx12Compiler {
    wgpu::Dx12Compiler::DynamicDxc {
        dxc_path: dll.to_string_lossy().into_owned(),
    }
}

/// Whether the DXC instance offered the adapter the FXC instance had: on a
/// laptop with two GPUs, the same request should pick the same one, and the
/// game must not switch GPUs because the compiler changed.
pub(super) fn same_adapter(fxc: &wgpu::AdapterInfo, dxc: &wgpu::AdapterInfo) -> bool {
    fxc.backend == dxc.backend
        && fxc.vendor == dxc.vendor
        && fxc.device == dxc.device
        && fxc.device_type == dxc.device_type
        && fxc.device_pci_bus_id == dxc.device_pci_bus_id
}

/// `text` from a shader pack as a shader module's label DXC can take. wgpu hands
/// the label to DXC as the source's file name, its first argument: DXC fails to
/// read a name with a colon, and takes one starting with `-` or `/` for an
/// option (#16258). Those become `_`; FXC, Vulkan and Metal take any label.
pub(super) fn module_label(text: &str) -> String {
    text.chars()
        .enumerate()
        .map(|(i, c)| match c {
            ':' => '_',
            '-' | '/' if i == 0 => '_',
            c => c,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adapter(backend: wgpu::Backend, device_type: wgpu::DeviceType) -> wgpu::AdapterInfo {
        wgpu::AdapterInfo {
            name: "Intel(R) Iris(R) Xe Graphics".into(),
            vendor: 0x8086,
            device: 0x9a49,
            device_type,
            device_pci_bus_id: "0000:00:02.0".into(),
            driver: String::new(),
            driver_info: String::new(),
            backend,
            subgroup_min_size: 0,
            subgroup_max_size: 0,
            transient_saves_memory: None,
            limit_bucket: None,
        }
    }

    #[test]
    fn integrated_gpus_and_discrete_ones_asked_for_shader_model_6_5_get_dxc() {
        use wgpu::{Backend, DeviceType};
        for sm_6_5 in [false, true] {
            assert!(wants_dxc(
                &adapter(Backend::Dx12, DeviceType::IntegratedGpu),
                false,
                sm_6_5
            ));
            for kind in [DeviceType::VirtualGpu, DeviceType::Cpu, DeviceType::Other] {
                assert!(
                    !wants_dxc(&adapter(Backend::Dx12, kind), false, sm_6_5),
                    "{kind:?}"
                );
            }
            // Other backends have no shader compiler to choose.
            for backend in [Backend::Vulkan, Backend::Gl, Backend::Metal] {
                for kind in [DeviceType::IntegratedGpu, DeviceType::DiscreteGpu] {
                    assert!(
                        !wants_dxc(&adapter(backend, kind), false, sm_6_5),
                        "{backend:?} {kind:?}"
                    );
                }
            }
        }
        // A discrete GPU keeps FXC (#8513) unless ray queries or mesh shaders need DXC.
        let dgpu = adapter(Backend::Dx12, DeviceType::DiscreteGpu);
        assert!(!wants_dxc(&dgpu, false, false));
        assert!(wants_dxc(&dgpu, false, true));
        // Such a start kept FXC only because nothing asked: one that asks gets DXC, if it ships.
        assert!(dxc_on_request(&dgpu, false, false, true));
        assert!(!dxc_on_request(&dgpu, false, true, true));
        assert!(!dxc_on_request(&dgpu, false, false, false));
        let igpu = adapter(Backend::Dx12, DeviceType::IntegratedGpu);
        assert!(!dxc_on_request(&igpu, false, false, true));
    }

    #[test]
    fn wgpu_dx12_compiler_overrides_the_choice() {
        let igpu = adapter(wgpu::Backend::Dx12, wgpu::DeviceType::IntegratedGpu);
        assert!(!wants_dxc(&igpu, true, false));
        let dgpu = adapter(wgpu::Backend::Dx12, wgpu::DeviceType::DiscreteGpu);
        assert!(!wants_dxc(&dgpu, true, true));
    }

    #[test]
    fn the_dxc_instance_must_offer_the_same_gpu() {
        let igpu = adapter(wgpu::Backend::Dx12, wgpu::DeviceType::IntegratedGpu);
        assert!(same_adapter(&igpu, &igpu.clone()));
        let dgpu = wgpu::AdapterInfo {
            name: "NVIDIA GeForce RTX 3050 Ti Laptop GPU".into(),
            vendor: 0x10de,
            device: 0x25a0,
            device_type: wgpu::DeviceType::DiscreteGpu,
            device_pci_bus_id: "0000:01:00.0".into(),
            ..igpu.clone()
        };
        assert!(!same_adapter(&igpu, &dgpu));
    }

    #[test]
    fn module_labels_keep_dxc_from_reading_a_file_or_an_option() {
        for (text, label) in [
            ("Bloom: blur", "Bloom_ blur"),
            ("a:b:c", "a_b_c"),
            ("C:sky", "C_sky"),
            ("-sky", "_sky"),
            ("-", "_"),
            ("/E", "_E"),
            // Elsewhere DXC takes them, and the rest of what a pack may write.
            ("Bloom - blur/fast", "Bloom - blur/fast"),
            (" -sky", " -sky"),
            (
                "Ciel étoilé… \\ * ? \" < > | %",
                "Ciel étoilé… \\ * ? \" < > | %",
            ),
            ("", ""),
        ] {
            assert_eq!(module_label(text), label, "{text:?}");
        }
    }
}
