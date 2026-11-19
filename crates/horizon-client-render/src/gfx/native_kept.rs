//! A shader pack's kept resources (#16011 item 4): the storage its `[[resource]]`s declare,
//! made per view when the pack is installed, the layout entries a pass binds them with, what a
//! device lacks for them, and the epochs that say when they start over; the post chain's passes
//! bind them, and each frame starts by swapping, fitting and clearing them (`KeptStore::begin`).
//! Compute `[[post]]` entries dispatch where they stand in the chain, and `[[before_world]]`
//! entries once a frame after the shadow map (`NativePack::before_world`). Under `[ray_tracing]`
//! a pass that traces binds the scene besides (`trace_scene`).
use horizon_shader_api::check::{CheckedStage, Report};
use horizon_shader_api::kept::{self, Dispatch, Format, Keep, Kept, Shape};
use horizon_shader_api::stages::{GROUP, KeptAccess, KeptBinding, kept_bindings};
use horizon_shader_api::{Manifest, StageKind};

use super::{NativePost, Writes};

/// One resource: a buffer, or a texture's two versions, which swap each frame.
pub(super) enum Held {
    Buffer(wgpu::Buffer),
    Pair([wgpu::TextureView; 2]),
}

/// A compute `[[post]]` entry, with the ordinary pass it runs before (`at`), or a
/// `[[before_world]]` one (`at` none): its pipeline and dispatch, and under `[ray_tracing]`,
/// where its file traces, its module against the scene until `trace_scene` takes it.
pub(super) struct ComputePass {
    pub(super) label: String,
    at: Option<usize>,
    pipeline: wgpu::ComputePipeline,
    workgroup: [u32; 3],
    dispatch: Dispatch,
    pub(super) scene: Option<naga::Module>,
}

/// A post pass's kept bindings, and its layouts: the chain's shared entries and those, which
/// `entries` holds for a layout with the scene besides (`trace_scene`).
pub(super) struct KeptPass {
    bindings: Vec<KeptBinding>,
    pub(super) entries: Vec<wgpu::BindGroupLayoutEntry>,
    pub(super) layout: wgpu::BindGroupLayout,
    pub(super) pipeline: wgpu::PipelineLayout,
}

/// A pack's kept resources for one view, in `[[resource]]` order: none made until `allocate`.
pub(super) struct KeptStore {
    kept: Kept,
    held: Vec<Held>,
    size: [u32; 2],
    /// An R the scaled textures had no room for (`fit`): not tried again until R changes.
    refused: Option<[u32; 2]>,
    /// This frame's view and world epochs and R, from frame prep (`NativePack::write`).
    next: ([u32; 2], [u32; 2]),
    /// Each pass's bindings and layouts (`chain`), the ordinary passes' then the compute
    /// ones', and the compute passes, `[[post]]`'s then `[[before_world]]`'s; a rebuild carries
    /// the rest, not these.
    pub(super) passes: Vec<KeptPass>,
    pub(super) computes: Vec<ComputePass>,
    /// The `[[before_world]]` passes' bind groups, a parity's after another, made once
    /// (`before_world`) and again after the textures or the pass they bind change, or the
    /// scene they hold (its generation, none without the scene).
    worlds: Vec<wgpu::BindGroup>,
    worlds_traced: Option<u64>,
    /// Which version of each texture this frame writes; the other is `_previous`.
    parity: usize,
    /// Real frames handed over (`frame`), and the one `begin` last started.
    serial: u32,
    begun: u32,
    /// The frames the chain and the `[[before_world]]` passes last ran in (`once`).
    ran: [u32; 2],
    /// Whether this run of the chain is its frame's first: the compute passes run once a
    /// frame, so an in-place write (`x[i] += 1`) counts once however often the chain runs.
    started: bool,
    /// The epochs the resources were last cleared for.
    cleared: [u32; 2],
    /// Moves on each time resources are made (`allocate`, `fit`): the material's groups hold
    /// them (`Reads`).
    made: u32,
}

impl KeptStore {
    /// Every resource of `kept`, zeros, the R-relative textures at `scale` of `size` (R).
    #[cfg(test)]
    pub(super) fn new(device: &wgpu::Device, kept: &Kept, size: [u32; 2]) -> Self {
        let mut store = Self::declared(kept, size);
        store.allocate(device, size).unwrap();
        store
    }

    /// The store of `kept` at `size`, nothing made yet.
    pub(super) fn declared(kept: &Kept, size: [u32; 2]) -> Self {
        Self {
            kept: kept.clone(),
            held: Vec::new(),
            size,
            refused: None,
            next: ([1, 1], size),
            passes: Vec::new(),
            computes: Vec::new(),
            worlds: Vec::new(),
            worlds_traced: None,
            parity: 0,
            serial: 0,
            ran: [0, 0],
            started: false,
            begun: 0,
            cleared: [1, 1],
            made: 0,
        }
    }

    /// Makes every resource, zeros, the R-relative ones at R `size`, unless they are made; or
    /// none, and why, without the video memory for them.
    fn allocate(&mut self, device: &wgpu::Device, size: [u32; 2]) -> Result<(), String> {
        if self.held.is_empty() {
            (self.size, self.refused) = (size, None);
            self.worlds.clear();
            self.made = self.made.wrapping_add(1);
            let (resources, size) = (&self.kept.resources, self.size);
            self.held = made(device, resources, size).map_err(|e| {
                let bytes: u64 = resources.iter().map(|r| bytes(r.shape, size)).sum();
                let mib = bytes.div_ceil(1 << 20);
                format!("its kept resources ({mib} MiB) don't fit in video memory ({e})")
            })?;
        }
        Ok(())
    }

    /// Destroys every resource now, not when the GPU is done with the last frame.
    fn free(&mut self) {
        self.worlds.clear();
        for held in self.held.drain(..) {
            match held {
                Held::Buffer(buffer) => buffer.destroy(),
                Held::Pair(views) => views.iter().for_each(|v| v.texture().destroy()),
            }
        }
    }

    /// Takes this frame's `epochs` (view, world) and R, which it clears and fits by: a real
    /// frame, which `begin` starts once.
    pub(super) fn frame(&mut self, epochs: [u32; 2], size: [u32; 2]) {
        self.next = (epochs, size);
        self.serial = self.serial.wrapping_add(1);
    }

    /// Remakes the R-relative textures, as zeros, when R changes; whether it did. Without the
    /// video memory for all of them at the new R, they stay as they are, at the old one.
    pub(super) fn fit(&mut self, device: &wgpu::Device, size: [u32; 2]) -> bool {
        if size == self.size {
            // Back where they fit: a refused R may fit when it comes again.
            self.refused = None;
            return false;
        }
        if self.refused == Some(size) {
            return false;
        }
        let scaled = |r: &&kept::Resource| matches!(r.shape, Shape::Scaled { .. });
        let resources = self.kept.resources.iter();
        let mut remade = match made(device, resources.clone().filter(scaled), size) {
            Ok(remade) => remade.into_iter(),
            Err(e) => {
                let ([w, h], [was_w, was_h]) = (size, self.size);
                log::warn!(
                    "Horizon shader pack: no video memory for its kept textures at {w}×{h} ({e}); they stay at {was_w}×{was_h}"
                );
                self.refused = Some(size);
                return false;
            }
        };
        (self.size, self.refused) = (size, None);
        self.worlds.clear();
        self.made = self.made.wrapping_add(1);
        for (_, held) in resources.zip(&mut self.held).filter(|(r, _)| scaled(r)) {
            *held = remade.next().expect("one remade a scaled resource");
        }
        true
    }

    /// Starts a frame of the chain at R `size`: fits R, and once a real frame (however often
    /// it is called) swaps the textures' versions and clears, in `encoder`, the `keep = "view"`
    /// resources when the view epoch moved and the `keep = "world"` ones when the world's did.
    /// Whether textures were remade, so bind groups holding them go.
    pub(super) fn begin(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        size: [u32; 2],
    ) -> bool {
        let remade = self.fit(device, size);
        if self.begun == self.serial {
            return remade;
        }
        (self.begun, self.parity) = (self.serial, 1 - self.parity);
        let epochs = self.next.0;
        let moved = [0, 1].map(|i| epochs[i] != self.cleared[i]);
        self.cleared = epochs;
        for (r, held) in self.kept.resources.iter().zip(&self.held) {
            if !moved[usize::from(r.keep == Keep::World)] {
                continue;
            }
            match held {
                Held::Buffer(buffer) => encoder.clear_buffer(buffer, 0, None),
                // `CLEAR_TEXTURE` is optional: a render pass clears.
                Held::Pair(views) => {
                    for view in views {
                        drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("Kept resource clear"),
                            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                view,
                                resolve_target: None,
                                depth_slice: None,
                                ops: wgpu::Operations {
                                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                    store: wgpu::StoreOp::Store,
                                },
                            })],
                            ..Default::default()
                        }));
                    }
                }
            }
        }
        remade
    }

    /// Whether this is the frame's first run of the chain (0) or of the `[[before_world]]`
    /// passes (1); `begin` starts the frame on whichever runs first.
    fn once(&mut self, run: usize) -> bool {
        std::mem::replace(&mut self.ran[run], self.serial) != self.serial
    }
}

/// Whether a layout entry of the post chain's group is one a `[[before_world]]` pass binds: the
/// API's resources of that stage (`StageKind::resources`) and the pack's `[[textures]]`, which
/// every stage reads.
fn before_world_binds(e: &wgpu::BindGroupLayoutEntry) -> bool {
    use horizon_shader_api::stages::{POST_TARGET_BINDING, TEXTURE_BINDING};
    let own = StageKind::BeforeWorld.resources().iter();
    own.map(|r| r.binding).any(|b| b == e.binding)
        || (TEXTURE_BINDING..POST_TARGET_BINDING).contains(&e.binding)
}

/// The kept resources and compute passes of the post chain `stages` (the `[[post]]` entries of
/// `manifest`, in order, then its `[[before_world]]` ones), and its ordinary passes: each pass's
/// layouts hold the group's `shared` entries (a `[[before_world]]` pass's of them, as
/// `before_world_binds`) and its kept ones. None without `[[resource]]`: a compute pass would
/// have nothing to write, so it doesn't run. An error where the device can't run or make them
/// (`unmet`) or refuses a compute pipeline.
pub(super) fn chain(
    device: &wgpu::Device,
    manifest: &Manifest,
    stages: Vec<CheckedStage>,
    shared: &[wgpu::BindGroupLayoutEntry],
    constants: &[(String, f64)],
) -> Result<(Vec<CheckedStage>, Option<KeptStore>), String> {
    let kept = &manifest.kept;
    let computing = |s: &CheckedStage| s.kind != StageKind::Post;
    if kept.resources.is_empty() {
        return Ok((stages.into_iter().filter(|s| !computing(s)).collect(), None));
    }
    if let Some(why) = unmet(device, manifest, &stages, shared) {
        return Err(why);
    }
    let (computes, ordinary): (Vec<_>, Vec<_>) = stages.into_iter().partition(computing);
    let mut store = KeptStore::declared(kept, [1, 1]);
    // The ordinary passes' layouts first, so a pass's index is its index in the chain.
    let (plain, compute): (Vec<_>, Vec<_>) = manifest.post.iter().partition(|p| !p.compute);
    let plain = plain.into_iter().map(|p| (p, false));
    let compute = compute.into_iter().map(|p| (p, false));
    let world = kept.before_world.iter().map(|p| (p, true));
    store.passes = (plain.chain(compute).chain(world))
        .map(|(entry, before)| {
            let visibility = match entry.compute || before {
                true => wgpu::ShaderStages::COMPUTE,
                false => wgpu::ShaderStages::FRAGMENT,
            };
            let bindings = kept_bindings(manifest, entry);
            let mut entries: Vec<_> = (shared.iter())
                .filter(|e| !before || before_world_binds(e))
                .map(|e| wgpu::BindGroupLayoutEntry { visibility, ..*e })
                .collect();
            entries.extend(layout_entries(kept, &bindings, visibility));
            let label = Some("Native shader pack post, kept");
            let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label,
                entries: &entries,
            });
            let mut groups: [Option<&wgpu::BindGroupLayout>; GROUP as usize + 1] =
                Default::default();
            groups[GROUP as usize] = Some(&layout);
            let pipeline = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label,
                bind_group_layouts: &groups,
                ..Default::default()
            });
            KeptPass {
                bindings,
                entries,
                layout,
                pipeline,
            }
        })
        .collect();
    // A compute entry runs before the ordinary pass after it in the list (`at`); the
    // `[[before_world]]` ones before the world (none). The check lists each kind in order.
    let (worlds, computes): (Vec<_>, Vec<_>) =
        (computes.into_iter()).partition(|s| s.kind == StageKind::BeforeWorld);
    let mut at = 0;
    let post = (manifest.post.iter()).filter_map(|entry| {
        at += usize::from(!entry.compute);
        entry.compute.then_some((entry, Some(at)))
    });
    let world = kept.before_world.iter().map(|entry| (entry, None));
    let (mut computes, mut worlds) = (computes.into_iter(), worlds.into_iter());
    for (entry, at) in post.chain(world) {
        let stage = match at {
            Some(_) => computes.next(),
            None => worlds.next(),
        };
        let Some(stage) = stage else {
            return Err(format!("{}: no checked compute stage", entry.file));
        };
        let layout = &store.passes[ordinary.len() + store.computes.len()].pipeline;
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let pipeline = compute_pipeline(device, &stage.label, layout, stage.module, constants);
        if let Some(e) = pollster::block_on(scope.pop()) {
            return Err(format!("{}: {e}", stage.file));
        }
        store.computes.push(ComputePass {
            label: stage.label,
            at,
            pipeline,
            workgroup: entry.workgroup.unwrap_or([1, 1, 1]),
            dispatch: entry.dispatch.unwrap_or(Dispatch::Pixels),
            scene: stage.scene,
        });
    }
    Ok((ordinary, Some(store)))
}

/// A compute pass's pipeline from its checked `module` (`horizon_cs_compute`), with the option
/// values `constants`.
pub(super) fn compute_pipeline(
    device: &wgpu::Device,
    label: &str,
    layout: &wgpu::PipelineLayout,
    module: naga::Module,
    constants: &[(String, f64)],
) -> wgpu::ComputePipeline {
    let present = super::present_constants(&module, constants);
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(&super::super::dx12_compiler::module_label(label)),
        source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(module)),
    });
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        module: &module,
        entry_point: Some("horizon_cs_compute"),
        compilation_options: wgpu::PipelineCompilationOptions {
            constants: &present,
            ..Default::default()
        },
        cache: None,
    })
}

/// The bind groups a parity of the chain makes, in order: each ordinary pass's (index, its own
/// index), then each compute `[[post]]` pass's (index, the ordinary pass it runs before, whose
/// `previous` it reads too).
pub(super) fn order(
    kept: &Option<KeptStore>,
    ordinary: usize,
) -> impl Iterator<Item = (usize, usize)> + '_ {
    let computes = kept.iter().flat_map(|k| &k.computes).enumerate();
    (0..ordinary)
        .map(|i| (i, i))
        .chain(computes.filter_map(move |(c, p)| Some((ordinary + c, p.at?))))
}

/// Dispatches, in `encoder`, the compute passes that run before ordinary pass `at`, with this
/// frame's `groups` (`order`'s) at R `size`, on the frame's first run of the chain only
/// (`begin`): `"pixels"` covers R in whole workgroups, the prelude's `in_bounds` false past it.
/// With `traced` (a frame with the scene), a pass that traces runs its pipeline against it.
/// An indirect dispatch (#16545's follow-up) would take its workgroups from a buffer here.
pub(super) fn dispatch(
    kept: &Option<KeptStore>,
    encoder: &mut wgpu::CommandEncoder,
    (groups, traced): (&[wgpu::BindGroup], Option<&[Option<wgpu::ComputePipeline>]>),
    at: usize,
    size: [u32; 2],
    mut timing: Option<&mut crate::gfx::pack_timing::PassTiming>,
) {
    let Some(k) = kept.as_ref().filter(|k| k.started) else {
        return;
    };
    let ordinary = k.passes.len() - k.computes.len();
    for (c, p) in k.computes.iter().enumerate() {
        if p.at == Some(at) {
            let pipeline = traced.and_then(|t| t[c].as_ref()).unwrap_or(&p.pipeline);
            let timing = timing.as_deref_mut();
            record(p, encoder, (&groups[ordinary + c], pipeline), size, timing);
        }
    }
}

/// Records compute pass `p` in `encoder` with its `group` and `pipeline` at R `size`, timed
/// under its name in a timed frame.
fn record(
    p: &ComputePass,
    encoder: &mut wgpu::CommandEncoder,
    (group, pipeline): (&wgpu::BindGroup, &wgpu::ComputePipeline),
    size: [u32; 2],
    timing: Option<&mut crate::gfx::pack_timing::PassTiming>,
) {
    let cells = match p.dispatch {
        Dispatch::Cells(cells) => cells,
        Dispatch::Pixels => [size[0], size[1], 1],
    };
    let [x, y, z] = [0, 1, 2].map(|i| cells[i].div_ceil(p.workgroup[i]));
    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
        label: Some(&p.label),
        timestamp_writes: timing.and_then(|t| t.compute_writes(&p.label)),
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(GROUP, group, &[]);
    pass.dispatch_workgroups(x, y, z);
}

/// Where each ordinary `[[post]]` entry's `output` draws: one of the pack's `targets`
/// (`Manifest::post_targets`), a kept texture, or nowhere of its own (the chain).
pub(super) fn outputs(manifest: &Manifest, targets: &[(&str, f32)]) -> Vec<Option<Writes>> {
    let kept = |name| (manifest.kept.resources.iter()).position(|r| r.name == name);
    (manifest.post.iter().filter(|p| !p.compute))
        .map(|p| {
            let name = p.output.as_deref()?;
            let target = targets.iter().position(|t| t.0 == name).map(Writes::Target);
            target.or_else(|| kept(name).map(Writes::Kept))
        })
        .collect()
}

/// The format a pass that `writes` off the screen draws in: a kept texture's own, else the
/// chain's.
pub(super) fn target(kept: &Option<KeptStore>, writes: Writes) -> wgpu::TextureFormat {
    match (kept, writes) {
        (Some(k), Writes::Kept(r)) => match k.kept.resources[r].shape {
            Shape::Fixed { format: f, .. } | Shape::Scaled { format: f, .. } => format(f),
            Shape::Buffer { .. } => unreachable!("the check takes only a texture as an output"),
        },
        _ => super::INTERMEDIATE,
    }
}

/// Post pass `pass`'s kept bindings and layouts, if the chain has kept resources (the tonemap,
/// `None`, has none).
pub(super) fn pass(kept: &Option<KeptStore>, pass: Option<usize>) -> Option<&KeptPass> {
    Some(&kept.as_ref()?.passes[pass?])
}

/// Post pass `pass`'s kept group entries for the frames whose written version is `parity`.
pub(super) fn entries(
    kept: &Option<KeptStore>,
    pass: Option<usize>,
    parity: usize,
) -> Vec<wgpu::BindGroupEntry<'_>> {
    let (Some(k), Some(p)) = (kept, self::pass(kept, pass)) else {
        return Vec::new();
    };
    bound(k, &p.bindings, parity)
}

/// The group entries of `bindings` of `k`'s resources for the frames whose written version is
/// `parity`.
fn bound<'a>(
    k: &'a KeptStore,
    bindings: &[KeptBinding],
    parity: usize,
) -> Vec<wgpu::BindGroupEntry<'a>> {
    (bindings.iter())
        .map(|b| wgpu::BindGroupEntry {
            binding: b.binding,
            resource: match (&k.held[b.resource], b.access) {
                (Held::Buffer(buffer), _) => buffer.as_entire_binding(),
                (Held::Pair(v), KeptAccess::Previous) => {
                    wgpu::BindingResource::TextureView(&v[1 - parity])
                }
                (Held::Pair(v), _) => wgpu::BindingResource::TextureView(&v[parity]),
            },
        })
        .collect()
}

/// What the terrain reads of a pack's kept resources (`[material] reads`): its `bindings`.
#[derive(Clone, Copy)]
pub(in crate::gfx) struct Reads<'a> {
    store: &'a KeptStore,
    bindings: &'a [KeptBinding],
}

impl<'a> Reads<'a> {
    /// Which resources the groups were made with (`KeptStore::made`).
    pub(in crate::gfx) fn made(&self) -> u32 {
        self.store.made
    }

    /// The group entries for the frames whose written version is `parity`: this frame's version
    /// what a `[[before_world]]` pass wrote, `_previous` the other.
    pub(in crate::gfx) fn entries(&self, parity: usize) -> Vec<wgpu::BindGroupEntry<'a>> {
        bound(self.store, self.bindings, parity)
    }
}

impl NativePost {
    /// What a material of `bindings` reads of this pack's kept resources, if it has any.
    pub(in crate::gfx) fn reads<'a>(&'a self, bindings: &'a [KeptBinding]) -> Option<Reads<'a>> {
        let store = self.kept.as_ref()?;
        Some(Reads { store, bindings })
    }

    /// The version of the kept textures this frame writes (`KeptStore::parity`); 0 without.
    pub(in crate::gfx) fn parity(&self) -> usize {
        self.kept.as_ref().map_or(0, |k| k.parity)
    }
}

/// This frame's set of the cached `groups`, one a parity with kept resources.
pub(super) fn current<'a>(
    kept: &Option<KeptStore>,
    groups: &'a [wgpu::BindGroup],
) -> &'a [wgpu::BindGroup] {
    let n = groups.len() / (1 + usize::from(kept.is_some()));
    &groups[kept.as_ref().map_or(0, |k| k.parity) * n..][..n]
}

/// This frame's version of kept texture `r`, a pass's output.
pub(super) fn output(kept: &Option<KeptStore>, r: usize) -> &wgpu::TextureView {
    match kept.as_ref().map(|k| (&k.held[r], k.parity)) {
        Some((Held::Pair(v), parity)) => &v[parity],
        _ => unreachable!("an output is a texture of the chain's store"),
    }
}

/// Installs the kept resources of `new`, the post stage of the pack that runs from now on,
/// where `old`'s ran: a store `new` carried over (`NativePack::carry`, before) stays; else
/// `old`'s is destroyed before `new`'s is made, so a reload never holds two stores, and a pack
/// that fails to build never makes one. `new`'s R-relative textures are made at R `size`, the
/// frame's. Why `new`'s couldn't be made, without the video memory for it: then it holds none.
pub(in crate::gfx) fn install(
    device: &wgpu::Device,
    new: Option<&mut NativePost>,
    old: Option<&mut NativePost>,
    size: [u32; 2],
) -> Result<(), String> {
    if let Some(store) = old.and_then(|p| p.kept.as_mut()) {
        store.free();
    }
    if let Some(store) = new.and_then(|p| p.kept.as_mut()) {
        return store.allocate(device, size);
    }
    Ok(())
}

/// Makes `post`'s store again, as zeros, after `install` freed it for a pack that was refused:
/// at the R its textures were last made at, not this frame's, and still not trying an R they
/// had no room for (`KeptStore::fit`).
pub(in crate::gfx) fn reinstall(
    device: &wgpu::Device,
    post: &mut NativePost,
) -> Result<(), String> {
    let Some(store) = &mut post.kept else {
        return Ok(());
    };
    // Its passes' groups bind the freed views.
    post.groups = None;
    let (size, refused) = (store.size, store.refused);
    store.allocate(device, size)?;
    store.refused = refused;
    Ok(())
}

impl NativePost {
    /// Takes the kept resources of `old`, the pack this one rebuilds (`NativePack::carry`),
    /// keeping its own passes' layouts, which its pipelines were made with.
    pub(in crate::gfx) fn carry(&mut self, old: &mut Self) {
        if let (Some(new), Some(was)) = (&mut self.kept, &mut old.kept) {
            std::mem::swap(&mut new.passes, &mut was.passes);
            std::mem::swap(&mut new.computes, &mut was.computes);
            std::mem::swap(new, was);
            // The old pack's groups bind its frame and shadow maps, not this one's.
            new.worlds.clear();
        }
    }

    /// Starts this frame's use of the kept resources at R `size` (`KeptStore::begin`).
    pub(super) fn begin_kept(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        size: [u32; 2],
    ) {
        let begun = |k: &mut KeptStore| {
            k.started = k.once(0);
            k.begin(device, encoder, size)
        };
        if (self.kept.as_mut()).is_some_and(begun) {
            self.groups = None;
        }
    }

    /// Starts the frame of the kept resources (`KeptStore::begin`, at the frame's R) and
    /// dispatches the `[[before_world]]` passes, once a frame (the first call after
    /// `kept_frame`), before the world draws: the terrain (`[material] reads`) and the passes
    /// after read what they wrote as this frame's. With `scene`, this frame's (built
    /// before them), the passes that trace run against it. With `timing`, each is timed.
    pub(in crate::gfx) fn before_world(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        scene: Option<&super::trace::Traced<'_>>,
        mut timing: Option<&mut crate::gfx::pack_timing::PassTiming>,
    ) {
        let Some(k) = &mut self.kept else {
            return;
        };
        if !k.once(1) {
            return;
        }
        if k.begin(device, encoder, k.next.1) {
            self.groups = None;
        }
        let worlds = k.computes.iter().filter(|p| p.at.is_none()).count();
        if worlds == 0 {
            return;
        }
        let traced = self.trace.as_ref().zip(scene);
        let generation = traced.map(|(_, scene)| scene.generation);
        let Some(k) = &self.kept else {
            return;
        };
        if k.worlds.is_empty() || k.worlds_traced != generation {
            let (post, passes) = (&*self, k.passes.len());
            let groups = (0..2).flat_map(|parity| {
                (passes - worlds..passes)
                    .map(move |i| post.world_group(k, device, (i, parity), traced))
            });
            let groups: Vec<_> = groups.collect();
            if let Some(k) = &mut self.kept {
                (k.worlds, k.worlds_traced) = (groups, generation);
            }
        }
        let Some(k) = &self.kept else {
            return;
        };
        let first = k.computes.len() - worlds;
        let pipelines = traced.map(|(trace, _)| &trace.computes[first..]);
        for (w, p) in k.computes[first..].iter().enumerate() {
            let pipeline = pipelines.and_then(|t| t[w].as_ref()).unwrap_or(&p.pipeline);
            let group = &k.worlds[k.parity * worlds + w];
            record(
                p,
                encoder,
                (group, pipeline),
                k.next.1,
                timing.as_deref_mut(),
            );
        }
    }

    /// The bind group of `[[before_world]]` pass `pass` (its index in `KeptStore::passes`) for
    /// `parity`: what the API gives that stage (`before_world_binds`) and its kept resources,
    /// and with `traced` where the pass traces, the scene.
    fn world_group(
        &self,
        k: &KeptStore,
        device: &wgpu::Device,
        (pass, parity): (usize, usize),
        traced: Option<(&super::trace::NativeTrace, &super::trace::Traced<'_>)>,
    ) -> wgpu::BindGroup {
        let view = wgpu::BindingResource::TextureView;
        let mut entries = vec![
            (0, self.frame.as_entire_binding()),
            (4, wgpu::BindingResource::Sampler(&self.linear)),
            (5, wgpu::BindingResource::Sampler(&self.nearest)),
            (6, view(&self.shadow.0)),
            (7, wgpu::BindingResource::Sampler(&self.shadow.1)),
            (9, view(&self.shadow.2)),
            (10, view(&self.shadow.3)),
        ]
        .into_iter()
        .map(|(binding, resource)| wgpu::BindGroupEntry { binding, resource })
        .collect::<Vec<_>>();
        entries.extend(super::pack_texture_group_entries(&self.textures));
        entries.extend(self::entries(&self.kept, Some(pass), parity));
        let layout =
            match traced.and_then(|(trace, scene)| Some((trace.layout(pass)?, trace, scene))) {
                Some((layout, trace, scene)) => {
                    entries.extend(trace.entries(pass, scene));
                    layout
                }
                None => &k.passes[pass].layout,
            };
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Native shader pack before the world"),
            layout,
            entries: &entries,
        })
    }

    /// Whether a compute pass of the chain was built against the scene (`ComputePass::scene`).
    pub(in crate::gfx) fn computes_trace(&self) -> bool {
        let computes = self.kept.iter().flat_map(|k| &k.computes);
        computes.into_iter().any(|p| p.scene.is_some())
    }

    /// Hands the kept resources this frame's `epochs` (view, world) and R (`NativePack::write`).
    pub(in crate::gfx) fn kept_frame(&mut self, epochs: [u32; 2], size: [u32; 2]) {
        if let Some(kept) = &mut self.kept {
            kept.frame(epochs, size);
        }
    }
}

impl super::super::native_pack::NativePack {
    /// The pack's `[[before_world]]` passes (`NativePost::before_world`), in `encoder` after its
    /// shadow passes, which they read, and before the world draws, against this frame's
    /// `scene` where they trace.
    pub fn before_world(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        scene: Option<&super::trace::Traced<'_>>,
    ) {
        self.post
            .before_world(device, encoder, scene, self.timing.as_mut());
        // The sky may draw next, in a pass of its own (`draw_sky_pass`).
        self.bind_kept(device);
    }

    /// Makes the groups the entity and sky stages read the kept resources with, for the
    /// frame's group as it is now: once its frame started (`before_world`), and again where the
    /// frame's group changes before the world draws (`bind_scene`).
    pub(in crate::gfx) fn bind_kept(&mut self, device: &wgpu::Device) {
        let entity = self.entity.as_mut().and_then(|e| e.kept.as_mut());
        let sky = self.sky.as_mut().and_then(|s| s.kept.as_mut());
        for k in entity.into_iter().chain(sky) {
            k.bind(device, &self.frame, &self.post);
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
        Format::R32Uint => wgpu::TextureFormat::R32Uint,
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
    /// How many of the store's next allocations (`made`) report no room for them: a test's
    /// stand-in for a device out of video memory, which no test can make safely on a shared GPU.
    pub(super) static NO_ROOM: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
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

/// What `make` makes of each of `resources` at R `size`, or the error that left no room for
/// them: one raised outside a scope is fatal. A texture that failed is invalid, and so is its
/// view, so validation errors are caught too (any of `make`'s, reported as no room, with
/// wgpu's message); all of them are dropped then.
fn made<'a>(
    device: &wgpu::Device,
    resources: impl IntoIterator<Item = &'a kept::Resource>,
    size: [u32; 2],
) -> Result<Vec<Held>, String> {
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    let held: Vec<_> = (resources.into_iter())
        .map(|r| make(device, &r.name, r.shape, size))
        .collect();
    // Native error scopes resolve at once.
    let error = pollster::block_on(memory.pop()).or(pollster::block_on(validation.pop()));
    let error = error.map(|e| e.to_string());
    #[cfg(test)]
    let error = error.or_else(|| {
        let left = NO_ROOM.get().checked_sub(1)?;
        NO_ROOM.set(left);
        Some("no room (test)".to_owned())
    });
    match error {
        None => Ok(held),
        Some(e) => Err(e),
    }
}

/// The video memory resource `shape` takes at R `size`: both versions of a texture.
fn bytes(shape: Shape, size: [u32; 2]) -> u64 {
    let texels =
        |format: Format, [w, h]: [u32; 2]| 2 * u64::from(w) * u64::from(h) * format.bytes();
    match shape {
        Shape::Buffer { bytes, .. } => bytes,
        Shape::Fixed { format, size } => texels(format, size),
        Shape::Scaled { format, scale } => texels(format, super::scaled(size, scale)),
    }
}

fn make(device: &wgpu::Device, name: &str, shape: Shape, size: [u32; 2]) -> Held {
    let pair = |f, [width, height]: [u32; 2]| {
        [0, 1].map(|_| {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
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
            });
            texture.create_view(&Default::default())
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

/// The kept resources a stage that draws the world reads (`reads` of `[material]`, `[entity]`,
/// `[sky]`), in its group 3 from binding 64 (`kept_bindings`), for the fragment stage only: the
/// material's vertex stage and `displacement` also draw the shadow, which reads none.
pub(in crate::gfx) struct StageReads {
    bindings: Vec<KeptBinding>,
    /// Their layout entries (`layout_entries`).
    pub(in crate::gfx) entries: Vec<wgpu::BindGroupLayoutEntry>,
    /// The frame's entries and theirs, the stage's group 3 where it doesn't trace.
    pub(in crate::gfx) layout: wgpu::BindGroupLayout,
    /// The group a parity (`KeptStore::parity`), made for the frame's group `.0` and the store's
    /// resources `.1` (`Reads::made`).
    groups: Option<(wgpu::BindGroup, u32, [wgpu::BindGroup; 2])>,
}

impl StageReads {
    /// What `stage` of `manifest` reads, joined with `frame`'s entries; none where it reads none.
    pub(in crate::gfx) fn new(
        device: &wgpu::Device,
        manifest: &Manifest,
        stage: Option<&horizon_shader_api::Stage>,
        frame: &super::NativeFrame,
    ) -> Option<Self> {
        let bindings = kept_bindings(manifest, stage?);
        let fragment = wgpu::ShaderStages::FRAGMENT;
        let entries = layout_entries(&manifest.kept, &bindings, fragment);
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Native shader pack stage, kept"),
            entries: &[frame.entries(), &entries].concat(),
        });
        (!bindings.is_empty()).then_some(Self {
            bindings,
            entries,
            layout,
            groups: None,
        })
    }

    /// Why the fragment stage, group 3 with these and the stage's `others` groups' entries,
    /// is over the device's `limits`, if it is.
    pub(in crate::gfx) fn over(
        &self,
        frame: &super::NativeFrame,
        others: &[wgpu::BindGroupLayoutEntry],
        limits: &wgpu::Limits,
    ) -> Option<String> {
        let group = [frame.entries(), &self.entries].concat();
        super::trace::over_limits(&group, others, limits, wgpu::ShaderStages::FRAGMENT)
    }

    /// What it reads of `post`'s store.
    pub(in crate::gfx) fn reads<'a>(&'a self, post: &'a NativePost) -> Option<Reads<'a>> {
        post.reads(&self.bindings)
    }

    /// Makes the groups for the frame's group as it is now and the store's resources, unless
    /// they are made.
    pub(in crate::gfx) fn bind(
        &mut self,
        device: &wgpu::Device,
        frame: &super::NativeFrame,
        post: &NativePost,
    ) {
        let Some(reads) = post.reads(&self.bindings) else {
            return;
        };
        let made = |(group, made, _): &(wgpu::BindGroup, u32, [wgpu::BindGroup; 2])| {
            *group == frame.group && *made == reads.made()
        };
        if !self.groups.as_ref().is_some_and(made) {
            let group = |parity| frame.joined(device, &self.layout, &reads.entries(parity));
            self.groups = Some((frame.group.clone(), reads.made(), [0, 1].map(group)));
        }
    }

    /// The group of the frames whose written version is `parity`, once made (`bind`).
    pub(in crate::gfx) fn group(&self, parity: usize) -> Option<&wgpu::BindGroup> {
        self.groups.as_ref().map(|(_, _, groups)| &groups[parity])
    }
}

/// The layout entries of `bindings` (`kept_bindings`) for `visibility`: a buffer read-only
/// where the pass doesn't write it, a written texture write-only storage, a read one filterable
/// only as `rgba16float` (the 32-bit ones are read with `textureLoad`), the integer ones
/// (`rgba32uint`, `r32uint`) read as `u32`.
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
                            f if f.is_uint() => wgpu::TextureSampleType::Uint,
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
            .filter(|e| stage.kind != StageKind::BeforeWorld || before_world_binds(e))
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

#[cfg(test)]
#[path = "native_kept_trace_tests.rs"]
mod trace_tests;

#[cfg(test)]
#[path = "native_material_kept_tests.rs"]
mod material_tests;

#[cfg(test)]
#[path = "native_entity_sky_kept_tests.rs"]
mod entity_sky_tests;
