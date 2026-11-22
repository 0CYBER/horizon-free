use super::*;

/// The wgpu instance options, without wgpu's validation of indirect draws
/// in release builds. That validation (meant for untrusted WebGPU content)
/// copies every indirect draw through a compute pass before the render pass
/// and costs about a tenth of `render`'s CPU time on Metal. The draws it checks
/// are the section draws `render` writes from resident sections, in
/// bounds by construction: index counts within `indices`, vertices within
/// the section's arena range, first instances below the origins written that
/// frame. Debug builds keep validating them; `WGPU_VALIDATION_INDIRECT_CALL=1`
/// turns it back on. DirectX 12 keeps it too: there wgpu-core's validation
/// pass is also what passes each draw's first instance to shaders reading
/// `instance_index`, and DirectX 12 is the fallback, where working matters
/// more than CPU time. GPU culling's count draws (`gpu_culling`) are written on
/// the GPU and never validated, in any build or backend: they are in bounds by
/// construction too (that module's doc), and no shader they run reads
/// `instance_index`.
/// `allow_nonconformant` offers the adapters wgpu hides for reporting no Vulkan
/// conformance version: Mesa's hasvk, which drives Ivy Bridge and Haswell,
/// reports 0.0.0.0 while it works. `open_device` asks for them only on a second
/// try at a backend that had no adapter at all. On DirectX 12, `dx12_compiler`
/// compiles the shaders (`gfx/dx12_compiler.rs`), unless `WGPU_DX12_COMPILER`
/// names another.
pub(crate) fn instance_descriptor(
    backends: wgpu::Backends,
    window: Option<&Arc<Window>>,
    allow_nonconformant: bool,
    dx12_compiler: wgpu::Dx12Compiler,
) -> wgpu::InstanceDescriptor {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    if let Some(window) = window {
        descriptor = descriptor.with_display_handle(Box::new(WindowDisplay(window.clone())));
    }
    descriptor.backends = backends.with_env();
    let dx12 = cfg!(windows) && descriptor.backends.contains(wgpu::Backends::DX12);
    // FXC spends about 2.6 s on every start compiling the pipelines (DXC about
    // 1 s); what it compiled once is kept in the game folder and read back
    // after that (third_party/wgpu-hal/src/dx12/disk_cache.rs).
    #[cfg(windows)]
    if dx12 {
        wgpu::hal::dx12::disk_cache::set_file(dx12_shader_cache(&crate::logging::game_home()));
    }
    if !cfg!(debug_assertions) && !dx12 {
        descriptor
            .flags
            .remove(wgpu::InstanceFlags::VALIDATION_INDIRECT_CALL);
    }
    descriptor.backend_options.dx12.shader_compiler = dx12_compiler;
    let mut descriptor = descriptor.with_env();
    if allow_nonconformant {
        descriptor
            .flags
            .insert(wgpu::InstanceFlags::ALLOW_UNDERLYING_NONCOMPLIANT_ADAPTER);
    }
    descriptor
}
/// Where DX12 keeps the shaders it compiled, inside a game folder.
pub fn dx12_shader_cache(home: &std::path::Path) -> std::path::PathBuf {
    home.join("cache").join("dx12-shaders.bin")
}

/// The window's display connection, so the OpenGL backend creates its EGL
/// display from the X11 or Wayland connection the window lives on. Without
/// one, wgpu-hal falls back to `EGL_MESA_platform_surfaceless`, whose configs
/// cannot present to a window; the backend then reports the surface as
/// incompatible and the game has no OpenGL to fall back to (`gl not
/// compatible with provided surface`).
#[derive(Debug, Clone)]
struct WindowDisplay(Arc<Window>);
impl wgpu::rwh::HasDisplayHandle for WindowDisplay {
    fn display_handle(
        &self,
    ) -> std::result::Result<wgpu::rwh::DisplayHandle<'_>, wgpu::rwh::HandleError> {
        wgpu::rwh::HasDisplayHandle::display_handle(&*self.0)
    }
}
/// The backends to open a device on, each tried in turn until one gives a
/// device. `api` is the player's choice (`GraphicsApi::with_env` applied);
/// `os` is `std::env::consts::OS`; `arch` the machine's (`horizon_system::machine_arch`), not the
/// build's; `wgpu_backend`: whether `WGPU_BACKEND` is set, which picks the backends itself.
/// Auto tries DirectX 12 first on Windows ARM64, the x64 build emulated there included,
/// then Vulkan, based on the device losses seen there (#7513). Other
/// Windows systems try Vulkan, then DirectX 12, then OpenGL; Linux tries Vulkan then
/// OpenGL. Each is tried when the one before has no surface, adapter or device. A native
/// crash in the driver can't be caught in the game; the launcher switches the setting to
/// DirectX 12 after one at the renderer's start on Vulkan (#746) or, on Windows, OpenGL
/// (#18291). macOS has Metal only.
pub(crate) fn backend_order(
    api: GraphicsApi,
    os: &str,
    arch: &str,
    wgpu_backend: bool,
) -> Vec<wgpu::Backends> {
    use wgpu::Backends;
    if os == "macos" || wgpu_backend {
        return vec![Backends::default()];
    }
    let windows = os == "windows";
    match api {
        GraphicsApi::Auto if windows && arch == "aarch64" => {
            vec![Backends::DX12, Backends::VULKAN, Backends::GL]
        }
        GraphicsApi::Auto if windows => vec![Backends::VULKAN, Backends::DX12, Backends::GL],
        GraphicsApi::Auto => vec![Backends::VULKAN, Backends::GL],
        GraphicsApi::Dx12 if windows => vec![Backends::DX12],
        GraphicsApi::Vulkan | GraphicsApi::Dx12 => vec![Backends::VULKAN],
        GraphicsApi::OpenGl => vec![Backends::GL],
    }
}
fn backends_name(backends: wgpu::Backends) -> &'static str {
    match backends {
        wgpu::Backends::VULKAN => "Vulkan",
        wgpu::Backends::DX12 => "DirectX 12",
        wgpu::Backends::GL => "OpenGL",
        _ => "graphics",
    }
}
/// Whether an adapter renders on the CPU (WARP, llvmpipe).
pub(crate) fn is_software(kind: wgpu::DeviceType) -> bool {
    kind == wgpu::DeviceType::Cpu
}
/// A device and what it came from.
pub(crate) struct Opened {
    pub(crate) instance: wgpu::Instance,
    pub(crate) surface: Option<wgpu::Surface<'static>>,
    pub(crate) adapter: wgpu::Adapter,
    pub(crate) device: wgpu::Device,
    pub(crate) queue: wgpu::Queue,
    pub(crate) device_lost: Arc<std::sync::atomic::AtomicBool>,
    /// The window surface's configuration, set on `device` during the search, and the
    /// surface's capabilities on `adapter`. None offscreen.
    pub(crate) configured: Option<(wgpu::SurfaceConfiguration, wgpu::SurfaceCapabilities)>,
}
/// Logs the adapter and records it for crash reports.
#[derive(Clone, Debug, Default)]
pub struct Adapter {
    pub name: String,
    pub vendor: String,
    /// `integrated`, `discrete`, `virtual`, `cpu` or `other`.
    pub kind: String,
    pub driver: String,
    pub backend: String,
}

static ADAPTER: std::sync::Mutex<Option<Adapter>> = std::sync::Mutex::new(None);

/// The adapter, once the renderer has one.
pub fn adapter() -> Option<Adapter> {
    ADAPTER.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// wgpu's device type in the words reports use.
pub fn gpu_kind(device_type: &str) -> &'static str {
    match device_type {
        "IntegratedGpu" => "integrated",
        "DiscreteGpu" => "discrete",
        "VirtualGpu" => "virtual",
        "Cpu" => "cpu",
        _ => "other",
    }
}

/// Logs the adapter and records it ([`adapter`]) for crash reports and telemetry.
pub(crate) fn describe_adapter(adapter: &wgpu::Adapter) {
    let info = adapter.get_info();
    log::info!(
        "Horizon renderer: {} ({:?}, {:?})",
        info.name,
        info.backend,
        info.device_type
    );
    *ADAPTER.lock().unwrap_or_else(|e| e.into_inner()) = Some(Adapter {
        name: info.name.clone(),
        vendor: horizon_system::gpu_vendor(&info.name, info.vendor).to_owned(),
        kind: gpu_kind(&format!("{:?}", info.device_type)).to_owned(),
        driver: format!("{} {}", info.driver, info.driver_info)
            .trim()
            .to_owned(),
        backend: format!("{:?}", info.backend),
    });
    crate::crash::set_detail(
        "Graphics Adapter",
        format!(
            "{} ({:?}, {:?}), driver {} {}",
            info.name, info.backend, info.device_type, info.driver, info.driver_info
        ),
    );
}
/// Whether an adapter's texture arrays hold the block textures Horizon puts
/// in one array: the arrays terrain.wgsl binds for them, and the layers per array.
pub(crate) fn check_layers(adapter: &wgpu::Adapter, layers: u32) -> Result<(u32, u32)> {
    let info = adapter.get_info();
    let limits = adapter.limits();
    let max = limits.max_texture_array_layers;
    let gl = info.backend == wgpu::Backend::Gl;
    let requested = adapter_requested_split(&info, requested_split(), layers);
    // A pack past the arrays terrain.wgsl binds takes more, where the GPU samples enough.
    match super::layer_split(max, gl, requested, layers) {
        Ok(split) => Ok((super::SPRITE_ARRAYS, split)),
        Err(_) if super::sprite_arrays_for(&limits) == super::MORE_SPRITE_ARRAYS => {
            let more = super::MORE_SPRITE_ARRAYS;
            Ok((
                more,
                super::layer_split_in(max, gl, requested, layers, more)?,
            ))
        }
        Err(e) => Err(e.into()),
    }
}
/// Whether the swapchain can take a window of `width` x `height`: each side from 1 to the
/// device's `max_texture_dimension_2d` (`max`), else `Surface::configure` panics. 0 is a
/// minimized window. Past `max`: Windows lets a program size a window up to 65535 pixels
/// (`SetWindowPos`), a 984x65496 client area, when an RTX 5080 takes 32768. Vanilla's
/// `Window.onFramebufferResize` likewise keeps the last size for one that is not positive.
pub(crate) fn drawable_size(width: u32, height: u32, max: u32) -> bool {
    (1..=max).contains(&width) && (1..=max).contains(&height)
}
/// The swapchain format: a non-sRGB target, because vanilla blends and tints in gamma
/// space, and 8-bit, because wgpu gives `Rgba16Float` the linear `ExtendedSrgb` color
/// space: the compositor would read these gamma-encoded values as linear light and wash
/// every colour out.
pub(crate) fn surface_format(
    formats: &[wgpu::TextureFormat],
    default: wgpu::TextureFormat,
) -> wgpu::TextureFormat {
    [
        wgpu::TextureFormat::Bgra8Unorm,
        wgpu::TextureFormat::Rgba8Unorm,
    ]
    .into_iter()
    .find(|f| formats.contains(f))
    .or_else(|| formats.iter().copied().find(|f| !f.is_srgb()))
    .unwrap_or(default)
}
/// Whether a format with the adapter's `format` features takes every usage and flag the
/// passes take of the frame.
pub(crate) fn draws_frame(format: wgpu::TextureFormatFeatures) -> bool {
    format.allowed_usages.contains(
        wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
    ) && format.flags.contains(
        wgpu::TextureFormatFeatureFlags::BLENDABLE | wgpu::TextureFormatFeatureFlags::FILTERABLE,
    )
}

/// The frame under a shader API 0.2 pack, or with HDR on and no pack (hdr-linear-light.md):
/// float, so values over 1 survive from the world to the tonemap and the HDR pass.
pub(crate) const LINEAR_FRAME: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// How long choosing a GPU may take before the game gives up: a driver that stopped
/// answering inside Vulkan's instance creation, DXGI's adapter enumeration or device
/// creation would hold the event loop on the boot screen forever without this (#16769).
/// The search is adapter enumeration, seconds even on a slow machine; shader compilation,
/// which can take minutes on an old GPU, comes after it, so a search this long means the
/// driver hung, not that it is still working. `HORIZON_DEVICE_OPEN_TIMEOUT` (seconds) raises
/// it for a machine whose driver is slow but alive.
pub(super) const DEVICE_OPEN_TIMEOUT: Duration = Duration::from_secs(30);
/// How long the timeout's message has before it ends the game: the boot screen repaints the
/// log's last lines, and a search that finishes in this window was slow, not hung.
const DEVICE_OPEN_GRACE: Duration = Duration::from_secs(1);
/// `HORIZON_DEVICE_OPEN_TIMEOUT` in seconds when it is a positive whole number, else
/// `DEVICE_OPEN_TIMEOUT`.
fn device_open_timeout() -> Duration {
    parse_device_open_timeout(std::env::var("HORIZON_DEVICE_OPEN_TIMEOUT").ok().as_deref())
}
pub(super) fn parse_device_open_timeout(seconds: Option<&str>) -> Duration {
    seconds
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|&seconds| seconds > 0)
        .map_or(DEVICE_OPEN_TIMEOUT, Duration::from_secs)
}

/// Opens a device on the first of `backend_order`'s backends that gives one.
/// Offscreen rendering needs no surface, so it can use a GPU the window's
/// display cannot present from (a headless compositor).
pub(super) async fn open_device(
    window: &Arc<Window>,
    offscreen: bool,
    api: GraphicsApi,
    needs: surface_setup::DeviceNeeds,
) -> Result<Opened> {
    let wgpu_backend = std::env::var_os("WGPU_BACKEND").is_some();
    // A search that hangs does so inside the driver, where nothing in this process can
    // interrupt it: the search stays on this thread (a window's surface can only be made
    // on it), and a watchdog ends the start instead of leaving the boot screen forever.
    let timeout = device_open_timeout();
    let searched = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let effective_api = api.with_env();
    let active_api = Arc::new(std::sync::Mutex::new(effective_api));
    let active_for_timeout = active_api.clone();
    let searched_for_timeout = searched.clone();
    let (disarm, _watchdog) = arm_search_watchdog(
        timeout,
        DEVICE_OPEN_GRACE,
        searched.clone(),
        move || {
            let hung = active_for_timeout.lock().map_or(effective_api, |g| *g);
            super::safe_start::mark_watchdog_hang(&searched_for_timeout, hung);
            log::error!("{}", device_search_timed_out(timeout));
            log::logger().flush();
        },
        move || {
            let message = device_search_timed_out(timeout);
            // The panic hook reports and ends the game (a start stuck inside a driver call
            // never dies on its own). The watchdog's thread name takes the hook's worker-thread
            // path: a crash report, then exit code 101.
            panic!("{message}");
        },
    );
    let opened = open_device_search(
        window.clone(),
        offscreen,
        wgpu_backend,
        api,
        needs,
        &active_api,
    )
    .await;
    searched.store(true, std::sync::atomic::Ordering::Release);
    if opened.is_ok() {
        super::safe_start::clear_watchdog_hang();
    }
    drop(disarm);
    opened
}
/// `open_device`'s tries in turn, each a set of backends and whether a software adapter
/// (WARP, llvmpipe) is allowed.
async fn open_device_search(
    window: Arc<Window>,
    offscreen: bool,
    wgpu_backend: bool,
    api: GraphicsApi,
    needs: surface_setup::DeviceNeeds,
    active_api: &std::sync::Mutex<GraphicsApi>,
) -> Result<Opened> {
    let tries = device_search(
        api.with_env(),
        std::env::consts::OS,
        // The machine's: the x64 build emulated on a Windows ARM64 PC has its GPU too (#7513).
        horizon_system::machine_arch(),
        wgpu_backend,
    );
    let mut first = None;
    for (order, allow_cpu) in tries {
        match open_device_with(&window, offscreen, &order, needs, allow_cpu, active_api).await {
            Ok(opened) => {
                if let Some(first) = &first {
                    log::warn!(
                        "Horizon renderer: the chosen graphics API gave no GPU ({first}); {}",
                        if allow_cpu {
                            "rendering on the CPU, which is slow"
                        } else {
                            "using the GPU through another one"
                        }
                    );
                }
                return Ok(opened);
            }
            Err(e) => {
                first.get_or_insert(e);
            }
        }
    }
    Err(first.unwrap_or_else(|| NO_GPU_DRIVER.into()))
}
/// Arms the device search's watchdog (#16769): the search blocks inside the graphics
/// driver, where nothing in this process can interrupt it, so when it outlives its timeout
/// the driver hung and waiting longer cannot end. It waits one `grace` more for the log's
/// line to reach the boot screen and for `searched`, which the start sets when the search
/// returns: one that answers inside that window was slow, not hung, and runs on. `on_timeout`
/// reports the timeout; `on_fire` ends the game if the search is still stuck. A sent value or the sender's drop disarms it, as a finished
/// or failed search does.
pub(super) fn arm_search_watchdog(
    timeout: Duration,
    grace: Duration,
    searched: Arc<std::sync::atomic::AtomicBool>,
    on_timeout: impl FnOnce() + Send + 'static,
    on_fire: impl FnOnce() + Send + 'static,
) -> (
    std::sync::mpsc::Sender<()>,
    Option<std::thread::JoinHandle<()>>,
) {
    let (sender, receiver) = std::sync::mpsc::channel();
    let handle = std::thread::Builder::new()
        .name("horizon-worker-gpu-watchdog".into())
        .spawn(move || {
            if receiver.recv_timeout(timeout) != Err(std::sync::mpsc::RecvTimeoutError::Timeout) {
                return;
            }
            on_timeout();
            std::thread::sleep(grace);
            if !searched.load(std::sync::atomic::Ordering::Acquire) {
                on_fire();
            }
        })
        .ok();
    (sender, handle)
}
/// What the player is told when the graphics driver stops answering while the game is
/// picking a GPU: the start can't continue, and waiting longer only keeps the boot screen
/// up (#16769).
pub(super) fn device_search_timed_out(waited: Duration) -> String {
    format!(
        "Horizon could not get a graphics device: the graphics driver stopped answering while \
         the game was picking a GPU ({waited:?}). The last \"Horizon renderer:\" line in the \
         log names the graphics API it was asking. Update your GPU driver and start again; to \
         try another API meanwhile, set graphicsApi to opengl, dx12 or vulkan in options.txt, \
         or start the game with HORIZON_GRAPHICS_API=opengl (or dx12, or vulkan)"
    )
}
/// The tries `open_device` makes, in turn: the chosen API's backends (`backend_order`) on a
/// real GPU; when the player chose one API, the other backends `Auto` would try, on a real
/// GPU; and last the chosen backends with a software adapter (WARP, llvmpipe) allowed.
/// Software rendering takes most of a weak CPU even on the title screen, so Auto and the
/// other choices try every GPU first: an Intel HD 2500 has DirectX 12 only through WARP and
/// its GPU through OpenGL (#5185), and a Fermi GPU has no Vulkan or DirectX 12 driver but
/// OpenGL (#3978). DirectX 12 tries nothing else: the launcher chooses it after Vulkan or
/// OpenGL crashed in the driver at the renderer's start (#746, #18291), which the game can't
/// catch, so going back to either would crash again; WARP is slow but runs.
pub(crate) fn device_search(
    api: GraphicsApi,
    os: &str,
    arch: &str,
    wgpu_backend: bool,
) -> Vec<(Vec<wgpu::Backends>, bool)> {
    let chosen = backend_order(api, os, arch, wgpu_backend);
    let mut tries = vec![(chosen.clone(), false)];
    if chosen != [wgpu::Backends::DX12] {
        let others: Vec<_> = backend_order(GraphicsApi::Auto, os, arch, wgpu_backend)
            .into_iter()
            .filter(|b| !chosen.contains(b))
            .collect();
        if !others.is_empty() {
            tries.push((others, false));
        }
    }
    tries.push((chosen, true));
    tries
}
fn api_for_backends(backends: wgpu::Backends, fallback: GraphicsApi) -> GraphicsApi {
    if backends == wgpu::Backends::VULKAN {
        GraphicsApi::Vulkan
    } else if backends == wgpu::Backends::DX12 {
        GraphicsApi::Dx12
    } else if backends == wgpu::Backends::GL {
        GraphicsApi::OpenGl
    } else {
        fallback
    }
}

/// `open_device` on `order`'s backends in turn, taking an adapter of the CPU type
/// (software rendering) only when `allow_cpu`.
async fn open_device_with(
    window: &Arc<Window>,
    offscreen: bool,
    order: &[wgpu::Backends],
    needs: surface_setup::DeviceNeeds,
    allow_cpu: bool,
    active_api: &std::sync::Mutex<GraphicsApi>,
) -> Result<Opened> {
    let mut failures = Vec::new();
    for (attempt, backends) in order.iter().copied().enumerate() {
        if let Ok(mut guard) = active_api.lock() {
            *guard = api_for_backends(backends, *guard);
        }
        let name = backends_name(backends);
        if attempt > 0 {
            log::warn!("Horizon renderer: trying {name} ({})", failures.join("; "));
        } else {
            // The search's first driver call is this backend's own, and a hanging driver
            // blocks inside it: with nothing after this line, the log names the backend
            // whose driver hung (#16769).
            startup_step(format_args!("asking {name} for a graphics device"));
        }
        match open_with_retry(backends, window, offscreen, needs, allow_cpu).await {
            Ok(opened) => return Ok(opened),
            Err(e) => failures.push(e),
        }
    }
    Err(format!("{NO_GPU_DRIVER} ({})", failures.join("; ")).into())
}
/// Why a backend gave no device: no adapter at all, which a second try with the
/// adapters wgpu hides can still find, or a failure after an adapter.
enum NoDevice {
    NoAdapter(String),
    Failed(String),
}
/// Opens a device on one backend, and on a backend whose adapters wgpu all hid
/// for reporting no Vulkan conformance version tries it again with them shown:
/// Mesa's hasvk drives Ivy Bridge and Haswell, works, and reports 0.0.0.0, so
/// without this the player is told they have no graphics driver while Vulkan
/// runs (a `gl not compatible with provided surface` fallback, or nothing, is
/// all they get). Returns why the backend gave no device.
async fn open_with_retry(
    backends: wgpu::Backends,
    window: &Arc<Window>,
    offscreen: bool,
    needs: surface_setup::DeviceNeeds,
    allow_cpu: bool,
) -> std::result::Result<Opened, String> {
    match open_backend(backends, window, offscreen, false, needs, allow_cpu).await {
        Err(NoDevice::NoAdapter(hidden)) => {
            log::warn!(
                "Horizon renderer: {} has no conformant adapter ({hidden}); asking again for the \
                 ones wgpu hides as non-conformant",
                backends_name(backends)
            );
            match open_backend(backends, window, offscreen, true, needs, allow_cpu).await {
                Ok(opened) => {
                    let info = opened.adapter.get_info();
                    if is_broken_haswell_vulkan(&info) {
                        log::warn!(
                            "Horizon renderer: skipping {} on Vulkan because Mesa hasvk suffers from \
                             vertex corruption on Haswell; falling back to next backend",
                            info.name
                        );
                        return Err(format!(
                            "Mesa hasvk on Haswell ({}) has broken vertex buffers under Vulkan; falling back",
                            info.name
                        ));
                    }
                    log::warn!(
                        "Horizon renderer: using {} although its driver reports no Vulkan \
                         conformance version",
                        info.name
                    );
                    Ok(opened)
                }
                Err(NoDevice::NoAdapter(e)) | Err(NoDevice::Failed(e)) => Err(e),
            }
        }
        Err(NoDevice::Failed(e)) => Err(e),
        Ok(opened) => Ok(opened),
    }
}
/// Intel Haswell (Gen 7.5) Mesa Vulkan driver (`hasvk`) reports 0.0.0.0 and suffers
/// from severe vertex buffer corruption and polygon tearing (#9912), but its OpenGL driver
/// (`crocus`) is fully conformant and stable. Ivy Bridge (`IVB`) keeps Vulkan (#4059).
pub(crate) fn is_broken_haswell_vulkan(info: &wgpu::AdapterInfo) -> bool {
    info.backend == wgpu::Backend::Vulkan
        && info.vendor == 0x8086
        && (matches!(
            info.device,
            0x0400..=0x042f // Haswell Desktop / Mobile
                | 0x0a00..=0x0a2f // Haswell ULT (Ultrabook)
                | 0x0d00..=0x0d2f // Haswell CRW (Crystal Well)
        ) || ["HSW", "Haswell"].iter().any(|tag| info.name.contains(tag)))
}
/// Opens a device on `backends` alone: its surface, adapter and device, or what
/// stopped it. `allow_nonconformant` is `open_with_retry`'s second try; a
/// software adapter is taken only when `allow_cpu`.
async fn open_backend(
    backends: wgpu::Backends,
    window: &Arc<Window>,
    offscreen: bool,
    allow_nonconformant: bool,
    needs: surface_setup::DeviceNeeds,
    allow_cpu: bool,
) -> std::result::Result<Opened, NoDevice> {
    let name = backends_name(backends);
    let open = |dx12_compiler| {
        wgpu::Instance::new(instance_descriptor(
            backends,
            Some(window),
            allow_nonconformant,
            dx12_compiler,
        ))
    };
    let instance = open(wgpu::Dx12Compiler::Fxc);
    let (surface, adapter) = match surface_and_adapter(&instance, window, offscreen).await {
        Ok(found) => found,
        Err(NoDevice::Failed(e)) => return Err(NoDevice::Failed(format!("{name} surface: {e}"))),
        Err(NoDevice::NoAdapter(e)) => return Err(NoDevice::NoAdapter(format!("{name}: {e}"))),
    };
    // NoAdapter, so `open_with_retry` asks again: with the non-conformant
    // adapters shown, wgpu ranks a real GPU (hasvk on Ivy Bridge or Haswell)
    // above a software one. The second try's error is final either way.
    if !allow_cpu && is_software(adapter.get_info().device_type) {
        return Err(NoDevice::NoAdapter(format!(
            "{name}: only a software adapter ({})",
            adapter.get_info().name
        )));
    }
    // An integrated GPU on DirectX 12 compiles with DXC, and so does a discrete
    // one asked for ray queries or mesh shaders, which FXC can't compile: the
    // same request again, on an instance that loads the DLL beside the game.
    // Anything that goes wrong keeps the FXC instance.
    let forced = dx12_compiler::forced();
    let sm_6_5 = needs.shader_model_6_5();
    let dxc = if dx12_compiler::wants_dxc(&adapter.get_info(), forced, sm_6_5) {
        open_dxc(&adapter.get_info(), window, offscreen, open).await
    } else {
        None
    };
    let dxc_found = dxc.is_some();
    let (instance, surface, adapter) = dxc.unwrap_or((instance, surface, adapter));
    if adapter.get_info().backend == wgpu::Backend::Dx12 {
        crate::crash::set_detail(
            "DirectX 12 shader compiler",
            match (forced, dxc_found) {
                (true, _) => "WGPU_DX12_COMPILER",
                (false, true) => "DXC",
                (false, false) => "FXC",
            },
        );
    }
    // An adapter that gives way (`gives_way`): the next one of this backend that presents is
    // tried before the next backend (#13493).
    let mut tried = Vec::new();
    let mut adapter = adapter;
    loop {
        let failure = match open_adapter(name, window, surface.as_ref(), &adapter, needs).await {
            Ok(adapted) => {
                return Ok(Opened {
                    instance,
                    surface,
                    adapter,
                    device: adapted.device,
                    queue: adapted.queue,
                    device_lost: adapted.device_lost,
                    configured: adapted.configured,
                });
            }
            Err(failure) => failure,
        };
        let gives_way = gives_way(&failure, allow_cpu);
        let failed_to_open = matches!(failure, AdapterFailure::Failed(_));
        let (what, why) = match failure {
            AdapterFailure::CannotPresent(why) => ("it can't present to the window", why),
            AdapterFailure::Failed(e) if !gives_way => return Err(NoDevice::Failed(e)),
            AdapterFailure::Failed(e) => ("its device failed to open", e),
        };
        let info = adapter.get_info();
        tried.push(info.clone());
        let others = instance.enumerate_adapters(wgpu::Backends::all()).await;
        let choices: Vec<_> = others
            .iter()
            .map(|other| AdapterChoice {
                kind: other.get_info().device_type,
                presents: surface
                    .as_ref()
                    .is_some_and(|s| !s.get_capabilities(other).formats.is_empty()),
                tried: tried.contains(&other.get_info()),
            })
            .collect();
        let Some(next) = pick_adapter(&choices, allow_cpu) else {
            log::warn!(
                "Horizon renderer: skipping {} on {name}: {what} ({why}), and no other {name} \
                 adapter can stand in",
                info.name
            );
            return Err(NoDevice::Failed(match failed_to_open {
                true => why,
                false => format!("{name}: {} can't present to the window ({why})", info.name),
            }));
        };
        adapter = others[next].clone();
        log::warn!(
            "Horizon renderer: skipping {} on {name}: {what} ({why}); trying {}",
            info.name,
            adapter.get_info().name
        );
    }
}
/// Whether `open_backend` tries another adapter of the backend after `failure`; otherwise the
/// failure ends the backend. One that can't present to the window gives way (#13493). One whose
/// device failed to open gives way only on the search's last, software-allowed try:
/// `request_adapter` hands back the same GPU there, and DirectX 12 tries no other backend
/// (`device_search`), so without this WARP never stands in for a GPU whose DirectX 12 device
/// fails to open (#18291).
fn gives_way(failure: &AdapterFailure, allow_cpu: bool) -> bool {
    match failure {
        AdapterFailure::CannotPresent(_) => true,
        AdapterFailure::Failed(_) => allow_cpu,
    }
}
/// A surface for `window` (none `offscreen`) and the adapter `open_backend`
/// asks `instance` for. `Failed` is the surface's error, `NoAdapter` the
/// request's.
async fn surface_and_adapter(
    instance: &wgpu::Instance,
    window: &Arc<Window>,
    offscreen: bool,
) -> std::result::Result<(Option<wgpu::Surface<'static>>, wgpu::Adapter), NoDevice> {
    let surface = if offscreen {
        None
    } else {
        Some(
            instance
                .create_surface(window.clone())
                .map_err(|e| NoDevice::Failed(e.to_string()))?,
        )
    };
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: surface.as_ref(),
            force_fallback_adapter: false,
            ..Default::default()
        })
        .await
        .map_err(|e| NoDevice::NoAdapter(e.to_string()))?;
    Ok((surface, adapter))
}
/// The DirectX 12 instance, surface and adapter with DXC for the integrated
/// GPU `fxc` (`gfx/dx12_compiler.rs`), `open` making the instance; `None`, with
/// one line in the log, when the DLL is missing, doesn't load, or the instance
/// offers another GPU: the FXC instance stays.
async fn open_dxc(
    fxc: &wgpu::AdapterInfo,
    window: &Arc<Window>,
    offscreen: bool,
    open: impl FnOnce(wgpu::Dx12Compiler) -> wgpu::Instance,
) -> Option<(
    wgpu::Instance,
    Option<wgpu::Surface<'static>>,
    wgpu::Adapter,
)> {
    let Some(dll) = dx12_compiler::bundled_dll() else {
        log::info!(
            "Horizon renderer: no {} beside the game; DirectX 12 compiles shaders with FXC",
            dx12_compiler::DLL
        );
        return None;
    };
    let instance = open(dx12_compiler::dxc(&dll));
    let why = match surface_and_adapter(&instance, window, offscreen).await {
        Ok((surface, adapter)) if dx12_compiler::same_adapter(fxc, &adapter.get_info()) => {
            log::info!(
                "Horizon renderer: DirectX 12 compiles shaders with DXC ({}) on this {:?}",
                dll.display(),
                fxc.device_type
            );
            return Some((instance, surface, adapter));
        }
        Ok((_, adapter)) => format!("it offered another GPU, {}", adapter.get_info().name),
        Err(NoDevice::Failed(e) | NoDevice::NoAdapter(e)) => e,
    };
    log::warn!(
        "Horizon renderer: DXC from {} didn't load ({why}); DirectX 12 compiles shaders with FXC",
        dll.display()
    );
    None
}
/// What to tell a player whose system offers no usable graphics adapter.
/// Vulkan is loaded at run time (the game links no graphics library), so a
/// missing driver shows here rather than as a missing-DLL dialog.
#[cfg(target_os = "windows")]
const NO_GPU_DRIVER: &str = "No graphics driver with Vulkan, DirectX 12 or OpenGL found: Horizon needs \
     a GPU driver with one of them. Install the latest driver for your NVIDIA, AMD or Intel GPU and \
     start the game again";
#[cfg(target_os = "macos")]
const NO_GPU_DRIVER: &str = "No Metal graphics device found";
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
const NO_GPU_DRIVER: &str = "No Vulkan or OpenGL graphics driver found: Horizon needs the Vulkan loader \
     (libvulkan.so.1) and a Vulkan driver for your GPU (Mesa's, or NVIDIA's), or OpenGL through EGL";
#[cfg(test)]
mod tests {
    use super::{AdapterFailure, gives_way};
    /// #18291: on DirectX 12's software-allowed last try a GPU whose device fails to open gives way
    /// to another adapter (WARP); on the GPU tries it ends the backend as before. One that can't
    /// present always gives way (#13493).
    #[test]
    fn a_device_that_fails_to_open_gives_way_only_on_the_software_try() {
        let failed = || AdapterFailure::Failed("E_INVALIDARG".into());
        assert!(gives_way(&failed(), true));
        assert!(!gives_way(&failed(), false));
        let cannot_present = AdapterFailure::CannotPresent("no formats".into());
        assert!(gives_way(&cannot_present, false));
    }
}
