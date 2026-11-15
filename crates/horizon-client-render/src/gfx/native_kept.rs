//! A shader pack's kept resources (#16011 item 4): the storage its `[[resource]]`s declare,
//! made per view when the pack is installed, the layout entries a pass binds them with, what a
//! device lacks for them, and the epochs that say when they start over. Nothing binds or clears
//! them yet; the game still refuses such a pack.
use horizon_shader_api::check::{CheckedStage, Report};
use horizon_shader_api::kept::{self, Dispatch, Format, Keep, Kept, Shape};
use horizon_shader_api::stages::{KeptAccess, KeptBinding, kept_bindings};
use horizon_shader_api::{Manifest, StageKind};

/// One resource: a buffer, or a texture's two versions, which swap each frame.
pub(super) enum Held {
    Buffer(wgpu::Buffer),
    Pair([wgpu::Texture; 2]),
}

/// A pack's kept resources for one view, in `[[resource]]` order: none made until `allocate`.
pub(super) struct KeptStore {
    kept: Kept,
    held: Vec<Held>,
    size: [u32; 2],
    /// This frame's view and world epochs and R, from frame prep (`NativePack::write`), which
    /// it clears and fits by from P4a on.
    next: ([u32; 2], [u32; 2]),
}

impl KeptStore {
    /// Every resource of `kept`, zeros, the R-relative textures at `scale` of `size` (R).
    #[cfg(test)]
    pub(super) fn new(device: &wgpu::Device, kept: &Kept, size: [u32; 2]) -> Self {
        let mut store = Self::declared(kept, size);
        store.allocate(device);
        store
    }

    /// The store of `kept` at `size`, nothing made yet.
    pub(super) fn declared(kept: &Kept, size: [u32; 2]) -> Self {
        Self {
            kept: kept.clone(),
            held: Vec::new(),
            size,
            next: ([1, 1], size),
        }
    }

    /// Makes every resource, zeros, unless they are made.
    fn allocate(&mut self, device: &wgpu::Device) {
        if self.held.is_empty() {
            let (resources, size) = (&self.kept.resources, self.size);
            self.held = (resources.iter())
                .map(|r| make(device, &r.name, r.shape, size))
                .collect();
        }
    }

    /// Destroys every resource now, not when the GPU is done with the last frame.
    fn free(&mut self) {
        for held in self.held.drain(..) {
            match held {
                Held::Buffer(buffer) => buffer.destroy(),
                Held::Pair(textures) => textures.iter().for_each(wgpu::Texture::destroy),
            }
        }
    }

    /// Takes this frame's `epochs` (view, world) and R, which it clears and fits by.
    pub(super) fn frame(&mut self, epochs: [u32; 2], size: [u32; 2]) {
        self.next = (epochs, size);
    }

    /// Remakes the R-relative textures, as zeros, when R changes.
    pub(super) fn fit(&mut self, device: &wgpu::Device, size: [u32; 2]) {
        if size == self.size {
            return;
        }
        self.size = size;
        for (r, held) in self.kept.resources.iter().zip(&mut self.held) {
            if matches!(r.shape, Shape::Scaled { .. }) {
                *held = make(device, &r.name, r.shape, size);
            }
        }
    }
}

/// Installs the kept resources of `new`, the post stage of the pack that runs from now on,
/// where `old`'s ran: a store `new` carried over (`NativePack::carry`, before) stays; else
/// `old`'s is destroyed before `new`'s is made, so a reload never holds two stores, and a pack
/// that fails to build never makes one.
pub(in crate::gfx) fn install(
    device: &wgpu::Device,
    new: Option<&mut super::NativePost>,
    old: Option<&mut super::NativePost>,
) {
    if let Some(store) = old.and_then(|p| p.kept.as_mut()) {
        store.free();
    }
    if let Some(store) = new.and_then(|p| p.kept.as_mut()) {
        store.allocate(device);
    }
}

impl super::NativePost {
    /// Takes the kept resources of `old`, the pack this one rebuilds (`NativePack::carry`).
    pub(in crate::gfx) fn carry(&mut self, old: &mut Self) {
        self.kept = old.kept.take().or(self.kept.take());
    }

    /// Hands the kept resources this frame's `epochs` (view, world) and R (`NativePack::write`).
    pub(in crate::gfx) fn kept_frame(&mut self, epochs: [u32; 2], size: [u32; 2]) {
        if let Some(kept) = &mut self.kept {
            kept.frame(epochs, size);
        }
    }
}

impl super::NativeFrame {
    /// Takes the epochs of `old`, the pack this one rebuilds (`NativePack::carry`), and its last
    /// frame: a pause or a teleport across the rebuild still resets the carried resources.
    pub(in crate::gfx) fn carry(&mut self, old: &mut Self) {
        self.epochs = old.epochs.take();
        self.last = old.last;
    }
}

/// Resets that clear the world resources too.
const WORLD: u32 =
    kept::REASON_LOAD | kept::REASON_RESET | kept::REASON_WORLD | kept::REASON_TELEPORT;

/// When a pack's kept resources start over (`frame.view_epoch`, `world_epoch`) and why
/// (`reset_reasons`). An epoch moves on once in a frame that resets it, and is never 0, so a
/// cell cleared to zeros never matches one.
pub(in crate::gfx) struct Epochs {
    view: u32,
    world: u32,
    reasons: u32,
    moves: [bool; 2],
    /// A teleport further than this, in blocks, resets both.
    reach: f64,
    /// Last frame's R and whether it was HDR.
    seen: Option<([u32; 2], bool)>,
}

impl Epochs {
    /// A fresh store's: both 1, loaded. Without `[kept] reach` (no world resources), view
    /// resources clear on a teleport of 256 blocks (#16011 item 4's example).
    pub(in crate::gfx) fn new(reach: Option<u32>) -> Self {
        let reach = f64::from(reach.unwrap_or(256));
        let (moves, seen) = ([false; 2], None);
        let (view, world, reasons) = (1, 1, kept::REASON_LOAD);
        Self {
            view,
            world,
            reasons,
            moves,
            reach,
            seen,
        }
    }

    /// A reset for `bits` (`kept::REASON_*`): the view's on any, the world's too on a load,
    /// reset, world change or teleport.
    pub(in crate::gfx) fn note(&mut self, bits: u32) {
        self.reasons |= bits;
        self.moves[0] = true;
        self.moves[1] |= bits & WORLD != 0;
    }

    /// A changed option marked `resets = keep`.
    fn option(&mut self, keep: Keep) {
        self.note(kept::REASON_OPTION);
        self.moves[1] |= keep == Keep::World;
    }

    /// This frame's `[view_epoch, world_epoch, reset_reasons]`, at R `size`, HDR or not,
    /// after a pause or not, the camera `moved` blocks since last frame. The reasons are this
    /// frame's only.
    pub(super) fn advance(
        &mut self,
        size: [u32; 2],
        hdr: bool,
        paused: bool,
        moved: f64,
    ) -> [u32; 3] {
        let seen = self.seen.replace((size, hdr));
        for (bit, reset) in [
            (kept::REASON_PAUSE, paused),
            (kept::REASON_SIZE, seen.is_some_and(|s| s.0 != size)),
            (kept::REASON_DISPLAY, seen.is_some_and(|s| s.1 != hdr)),
            (kept::REASON_TELEPORT, moved > self.reach),
        ] {
            if reset {
                self.note(bit);
            }
        }
        let next = |e: u32, moves| {
            if moves {
                e.checked_add(1).unwrap_or(1)
            } else {
                e
            }
        };
        (self.view, self.world) = (
            next(self.view, self.moves[0]),
            next(self.world, self.moves[1]),
        );
        self.moves = [false; 2];
        [self.view, self.world, std::mem::take(&mut self.reasons)]
    }

    /// The epochs (view, world) last advanced to.
    pub(in crate::gfx) fn current(&self) -> [u32; 2] {
        [self.view, self.world]
    }
}

/// What lets a rebuilt pack keep the kept resources of the pack before (`NativePack::carry`):
/// the same declaration and composed stages, with its option values and their `resets`.
pub(in crate::gfx) struct Signature {
    hash: u64,
    values: Vec<(String, f64)>,
    resets: Vec<(String, Keep)>,
}

impl Signature {
    /// `report`'s with the option values `constants`, if it declares kept resources.
    pub(in crate::gfx) fn of(report: &Report, constants: &[(String, f64)]) -> Option<Self> {
        use std::hash::{Hash, Hasher};
        let kept = &report.manifest.kept;
        if kept.resources.is_empty() {
            return None;
        }
        let mut hasher = std::hash::DefaultHasher::new();
        format!("{kept:?}").hash(&mut hasher);
        for s in &report.stages {
            (format!("{:?}", s.kind), &s.file, &s.source, &s.scene_source).hash(&mut hasher);
        }
        Some(Self {
            hash: hasher.finish(),
            values: constants.to_vec(),
            resets: kept.resets.clone(),
        })
    }

    /// Notes on `epochs` the resets of the options changed since `old`, if the store of `old`
    /// carries over; else false.
    pub(in crate::gfx) fn carry(&self, old: &Self, epochs: &mut Epochs) -> bool {
        if self.hash != old.hash {
            return false;
        }
        let value = |values: &[(String, f64)], id: &str| {
            values.iter().find(|(k, _)| k == id).map(|(_, v)| *v)
        };
        for (id, keep) in &self.resets {
            if value(&self.values, id) != value(&old.values, id) {
                epochs.option(*keep);
            }
        }
        true
    }
}

fn format(format: Format) -> wgpu::TextureFormat {
    match format {
        Format::Rgba16Float => wgpu::TextureFormat::Rgba16Float,
        Format::R32Float => wgpu::TextureFormat::R32Float,
        Format::Rgba32Float => wgpu::TextureFormat::Rgba32Float,
        Format::Rgba32Uint => wgpu::TextureFormat::Rgba32Uint,
    }
}

/// The usages a kept texture is made with: written by compute passes, read by any pass,
/// cleared by a render pass (`CLEAR_TEXTURE` is optional).
const TEXTURE_USAGES: wgpu::TextureUsages = wgpu::TextureUsages::STORAGE_BINDING
    .union(wgpu::TextureUsages::TEXTURE_BINDING)
    .union(wgpu::TextureUsages::RENDER_ATTACHMENT);

#[cfg(test)]
thread_local! {
    /// Usages the probe asks for besides `TEXTURE_USAGES`: a test's stand-in for a device
    /// that can't make a format so.
    pub(super) static PROBE_EXTRA: std::cell::Cell<wgpu::TextureUsages> =
        const { std::cell::Cell::new(wgpu::TextureUsages::empty()) };
}

/// The first kept texture whose format the device can't make with `TEXTURE_USAGES` (OpenGL ES
/// without `EXT_color_buffer_float` renders to no 32-bit float format), if any: a 1×1 texture
/// of each format, made and dropped under an error scope of its own, while the pack builds, so
/// the pack is refused and the running one keeps its store.
fn unmade(device: &wgpu::Device, kept: &Kept) -> Option<String> {
    #[cfg(not(test))]
    let usage = TEXTURE_USAGES;
    #[cfg(test)]
    let usage = TEXTURE_USAGES | PROBE_EXTRA.get();
    let mut seen = Vec::new();
    for r in &kept.resources {
        let Some(f) = r.format().filter(|f| !seen.contains(f)) else {
            continue;
        };
        seen.push(f);
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("Kept resource probe"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: format(f),
                usage,
                view_formats: &[],
            })
            .destroy();
        if let Some(e) = pollster::block_on(scope.pop()) {
            return Some(format!(
                "{}: the device can't make an {} texture to keep: {e}",
                r.name,
                f.wgsl()
            ));
        }
    }
    None
}

/// `refusal` for `device`, then whether it makes every kept texture's format (`unmade`).
pub(super) fn unmet(
    device: &wgpu::Device,
    manifest: &Manifest,
    stages: &[CheckedStage],
    shared: &[wgpu::BindGroupLayoutEntry],
) -> Option<String> {
    refusal(&device.limits(), manifest, stages, shared).or_else(|| unmade(device, &manifest.kept))
}

fn make(device: &wgpu::Device, name: &str, shape: Shape, size: [u32; 2]) -> Held {
    let pair = |f, [width, height]: [u32; 2]| {
        [0, 1].map(|_| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(name),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: format(f),
                usage: TEXTURE_USAGES,
                view_formats: &[],
            })
        })
    };
    match shape {
        Shape::Buffer { bytes, .. } => {
            Held::Buffer(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(name),
                size: bytes,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }))
        }
        Shape::Fixed { format, size } => Held::Pair(pair(format, size)),
        Shape::Scaled { format, scale } => Held::Pair(pair(format, super::scaled(size, scale))),
    }
}

/// The layout entries of `bindings` (`kept_bindings`) for `visibility`: a buffer read-only
/// where the pass doesn't write it, a written texture write-only storage, a read one filterable
/// only as `rgba16float` (the 32-bit ones are read with `textureLoad`), `rgba32uint` read as
/// `u32`.
pub(super) fn layout_entries(
    kept: &Kept,
    bindings: &[KeptBinding],
    visibility: wgpu::ShaderStages,
) -> Vec<wgpu::BindGroupLayoutEntry> {
    let entry = |b: &KeptBinding| {
        let ty = match (kept.resources[b.resource].shape, b.access) {
            (Shape::Buffer { .. }, access) => wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage {
                    read_only: access != KeptAccess::Write,
                },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            (Shape::Fixed { format: f, .. } | Shape::Scaled { format: f, .. }, access) => {
                match access {
                    KeptAccess::Write => wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: format(f),
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    _ => wgpu::BindingType::Texture {
                        sample_type: match f {
                            Format::Rgba32Uint => wgpu::TextureSampleType::Uint,
                            f => wgpu::TextureSampleType::Float {
                                filterable: f == Format::Rgba16Float,
                            },
                        },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                }
            }
        };
        wgpu::BindGroupLayoutEntry {
            binding: b.binding,
            visibility,
            ty,
            count: None,
        }
    };
    bindings.iter().map(entry).collect()
}

/// What a device of `limits` lacks for the kept resources and compute passes of `manifest`, if
/// anything: `stages` are passes of its `[[post]]` and `[[before_world]]` lists, in order, each
/// binding the group-3 entries `shared` besides its kept ones. A device without compute shaders
/// has no compute limits (wgpu gives them as 0), so those say it.
pub(super) fn refusal(
    limits: &wgpu::Limits,
    manifest: &Manifest,
    stages: &[CheckedStage],
    shared: &[wgpu::BindGroupLayoutEntry],
) -> Option<String> {
    let computes = |s: &CheckedStage| matches!(s.kind, StageKind::Compute | StageKind::BeforeWorld);
    if stages.iter().any(computes) && limits.max_compute_invocations_per_workgroup == 0 {
        return Some("the device runs no compute shaders".into());
    }
    let kept = &manifest.kept;
    let largest = (limits.max_storage_buffer_binding_size).min(limits.max_buffer_size);
    let side = limits.max_texture_dimension_2d;
    for r in &kept.resources {
        if let Shape::Fixed { size: [w, h], .. } = r.shape
            && w.max(h) > side
        {
            return Some(format!(
                "{}: {w}×{h} texels, the device takes {side}",
                r.name
            ));
        }
        if let Shape::Buffer { bytes, .. } = r.shape
            && bytes > largest
        {
            return Some(format!(
                "{}: {bytes} bytes, the device takes {largest}",
                r.name
            ));
        }
    }
    // Each kind's checked passes are its manifest entries in order.
    let ordinary: Vec<_> = manifest.post.iter().filter(|s| !s.compute).collect();
    let compute: Vec<_> = manifest.post.iter().filter(|s| s.compute).collect();
    let before: Vec<_> = kept.before_world.iter().collect();
    let mut next = [0; 3];
    for stage in stages {
        let (i, list, visibility) = match stage.kind {
            StageKind::Post => (0, &ordinary, wgpu::ShaderStages::FRAGMENT),
            StageKind::Compute => (1, &compute, wgpu::ShaderStages::COMPUTE),
            StageKind::BeforeWorld => (2, &before, wgpu::ShaderStages::COMPUTE),
            _ => continue,
        };
        let Some(entry) = list.get(next[i]) else {
            continue;
        };
        next[i] += 1;
        let why = |why: String| Some(format!("{}: {why}", stage.file));
        let mut entries: Vec<_> = (shared.iter())
            .map(|e| wgpu::BindGroupLayoutEntry { visibility, ..*e })
            .collect();
        entries.extend(layout_entries(
            kept,
            &kept_bindings(manifest, entry),
            visibility,
        ));
        if let Some(over) = super::trace::over_limits(&entries, &[], limits, visibility) {
            return why(over);
        }
        let Some(workgroup) = entry.workgroup else {
            continue;
        };
        let most = [
            limits.max_compute_workgroup_size_x,
            limits.max_compute_workgroup_size_y,
            limits.max_compute_workgroup_size_z,
        ];
        let invocations = workgroup.iter().product::<u32>();
        if workgroup.iter().zip(most).any(|(w, m)| *w > m)
            || invocations > limits.max_compute_invocations_per_workgroup
        {
            return why(format!(
                "a workgroup of {workgroup:?}, the device takes {most:?} and {} invocations",
                limits.max_compute_invocations_per_workgroup
            ));
        }
        let shared_bytes = workgroup_bytes(stage);
        if shared_bytes > limits.max_compute_workgroup_storage_size {
            return why(format!(
                "{shared_bytes} bytes of var<workgroup>, the device takes {}",
                limits.max_compute_workgroup_storage_size
            ));
        }
        if let Some(Dispatch::Cells(cells)) = entry.dispatch {
            let groups = cells.iter().zip(workgroup).map(|(c, w)| c.div_ceil(w));
            let most = limits.max_compute_workgroups_per_dimension;
            if let Some(n) = groups.max().filter(|n| *n > most) {
                return why(format!(
                    "{n} workgroups along an axis, the device takes {most}"
                ));
            }
        }
    }
    None
}

/// The bytes of the `var<workgroup>`s a compute stage's entry point (its only one) uses, itself
/// or through what it calls, as the validator found: not those the module declares for others.
fn workgroup_bytes(stage: &CheckedStage) -> u32 {
    let (module, uses) = (&stage.module, stage.info.get_entry_point(0));
    let mut layouter = naga::proc::Layouter::default();
    if layouter.update(module.to_ctx()).is_err() {
        return u32::MAX;
    }
    (module.global_variables.iter())
        .filter(|(h, g)| g.space == naga::AddressSpace::WorkGroup && !uses[*h].is_empty())
        .map(|(_, g)| layouter[g.ty].size)
        .sum()
}

#[cfg(test)]
#[path = "native_kept_tests.rs"]
mod tests;
