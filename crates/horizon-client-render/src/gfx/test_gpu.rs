//! The GPU tests' `wgpu::Instance` and adapters.

/// A new instance for one GPU test. On Windows it opens Vulkan and DirectX 12,
/// not OpenGL: WGL instances made and dropped from many test threads at once
/// deadlocked NVIDIA's driver, and the render tests hung most runs (#5238).
/// The game runs there on Vulkan or DirectX 12 unless neither works.
/// `WGPU_BACKEND` and the other `WGPU_*` variables still apply, as in the game.
pub(crate) fn instance() -> wgpu::Instance {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    if cfg!(windows) {
        descriptor.backends = wgpu::Backends::PRIMARY;
    }
    wgpu::Instance::new(descriptor.with_env())
}

/// The adapters players' shaders compile for, those this machine has: Vulkan,
/// and DirectX 12 with each of its compilers, with names for the tests' output.
/// Players' DirectX 12 compiles with FXC, part of Windows, and with DXC
/// (`dxcompiler.dll`, beside the game in the Windows x64 zip) on integrated
/// GPUs (`gfx/dx12_compiler.rs`); the two compile some shaders differently.
pub(crate) fn each_adapter() -> impl Iterator<Item = (wgpu::Adapter, &'static str)> {
    [
        (wgpu::Backends::VULKAN, Default::default(), "Vulkan"),
        (
            wgpu::Backends::DX12,
            wgpu::Dx12Compiler::Fxc,
            "DirectX 12, FXC",
        ),
        (
            wgpu::Backends::DX12,
            wgpu::Dx12Compiler::default_dynamic_dxc(),
            "DirectX 12, DXC",
        ),
    ]
    .into_iter()
    .filter_map(|(backends, compiler, name)| {
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
        desc.backends = backends;
        desc.backend_options.dx12.shader_compiler = compiler;
        // Optimized as in a release build: wgpu's `DEBUG`, on in debug builds,
        // compiles DirectX 12 shaders without optimization, and hid FXC's
        // dropped test of #16168.
        desc.flags.remove(wgpu::InstanceFlags::DEBUG);
        let instance = wgpu::Instance::new(desc);
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
        Some((adapter, name))
    })
}
