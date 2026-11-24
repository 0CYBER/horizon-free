//! Horizon's own shader packs in the game (docs/shaders.md): the pick in `shaderpacks/` (a
//! folder with `horizon-shaders.toml`, or a `.hshader`; `game/interface/shader_packs.rs`) runs once it
//! passes `horizon_shader_api::check`. The pack is looked at twice a second: a saved file (or
//! saved option values) is checked and the pack rebuilt, and a
//! pack that fails is refused with its problems in the chat. Where a stage fails, the last
//! good version keeps drawing, on the frame it has: the pick keeps asking for it
//! (`remember_look`). A pack whose manifest is refused (`manifest_refused`) stops drawing
//! instead: the manifest decides the frame, the one that ran was built for another, and the
//! pick asks for its frame as no pack does (`picked_pack_shape`).

use std::{
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use horizon_client::gfx::native_entity::{EntityKind, EntityRuns};
use horizon_client::text::Text;
use horizon_shader_api::{Manifest, Pack, Primaries, check};

use super::Game;

const LOOK_EVERY: Duration = Duration::from_millis(500);
const RED: u32 = 0xFF5555;

thread_local! {
    // Loader outcomes survive renderer replacement; rejected files must not cause a rebuild loop.
    // Each is the pack's files (`fingerprint`) and whether a version of the pack draws since:
    // those files, or the last good ones before them (`remember_look`).
    static OUTCOMES: std::cell::RefCell<std::collections::HashMap<PathBuf, (Option<u64>, bool)>> =
        std::cell::RefCell::default();
}

/// Whether a version of the pack at `path` draws since it was last looked at: `None` before
/// that.
pub(super) fn outcome(path: &Path) -> Option<bool> {
    OUTCOMES.with_borrow(|packs| packs.get(path).map(|(_, draws)| *draws))
}

pub(super) fn remember(path: &Path, stamp: Option<u64>, draws: bool) {
    OUTCOMES.with_borrow_mut(|packs| packs.insert(path.to_owned(), (stamp, draws)));
}

/// Remembers a look at one of Horizon's own packs: its files were `taken`, or they failed
/// while a version of the pack was `running`. A stage that fails leaves that version drawing,
/// so the pick keeps asking for its frame and the renderer isn't made again; a refused
/// manifest stops it (`manifest_refused`), and a pack that never ran has nothing to keep.
pub(super) fn remember_look(
    path: &Path,
    stamp: Option<u64>,
    taken: bool,
    manifest_refused: bool,
    running: bool,
) {
    remember(path, stamp, taken || (running && !manifest_refused));
}

pub(super) fn refused(path: &Path, stamp: Option<u64>) -> bool {
    OUTCOMES.with_borrow(|packs| packs.get(path) == Some(&(stamp, false)))
}

#[derive(Default)]
pub(super) struct NativeShaders {
    /// The resolved pack path, of either kind (`game/interface/shader_packs.rs`); empty: none.
    pub picked: String,
    /// The pack's files when it was last read (`fingerprint`).
    stamp: Option<u64>,
    looked: Option<Instant>,
    selection: String,
    resolved: PathBuf,
    resolved_at: Option<Instant>,
    /// The runs of this frame's entity geometry that the pack's `[entity]` stage lights
    /// (`Game::lit_entity`), up to the renderer taking them (`Renderer::take_entity_runs`).
    pub entity_runs: std::cell::RefCell<EntityRuns>,
    /// The same runs' vertices posed on the GPU, counted as `SkinFrame::vertices` counts them.
    pub posed_runs: std::cell::RefCell<EntityRuns>,
    /// While the ray scene has the entity layer: the camera-relative box of its reach this frame
    /// (`Renderer::ray_entity_reach`), and the models of the entities within it that the camera
    /// doesn't draw, with their runs (`Game::ray_only`), up to the renderer taking them.
    pub ray_reach: Option<horizon_client::gfx::Frustum>,
    pub ray_only: std::cell::RefCell<Vec<horizon_client::mesher::Vertex>>,
    pub ray_runs: std::cell::RefCell<EntityRuns>,
    /// Set while an entity of the layer's kinds is culled (`Game::culled_for_rays`): occlusion
    /// then moves its model to `ray_only` instead of dropping it.
    pub ray_keeps: std::cell::Cell<bool>,
}

impl NativeShaders {
    /// Gives `renderer` the frame's entity runs and its ray-only entities.
    pub fn hand_over(&mut self, renderer: &mut horizon_client::gfx::Renderer) {
        renderer.take_entity_runs(self.entity_runs.get_mut(), self.posed_runs.get_mut());
        renderer.take_ray_only(self.ray_only.get_mut(), self.ray_runs.get_mut());
    }

    pub fn resolve_path(&mut self, dir: &Path, picked: &str, filesystem: bool) -> PathBuf {
        if self.selection != picked || self.resolved_at.is_none_or(|t| t.elapsed() >= LOOK_EVERY) {
            let resolved = if filesystem {
                dir.join(picked)
            } else {
                horizon_client::shader_pack_list::resolve(dir, picked).1
            };
            // A manifest being edited may be temporarily invalid. Keep its path so the
            // reload reports the error.
            let unresolved =
                !filesystem && picked.starts_with("horizon:") && resolved == dir.join(picked);
            let keep_path = unresolved
                && self.selection == picked
                && self.resolved_at.is_some()
                && Pack::read_manifest(&self.resolved).is_err();
            if !keep_path {
                self.resolved = resolved;
            }
            self.selection = picked.to_owned();
            self.resolved_at = Some(Instant::now());
        }
        self.resolved.clone()
    }

    /// No pick: the next `resolve_path` looks again, as one asked with no pick would, without
    /// the disk now (`follow_frame` asks every frame; title screens don't call `resolve_path`).
    pub fn forget_selection(&mut self) {
        if !self.selection.is_empty() {
            self.selection.clear();
            self.resolved_at = None;
        }
    }

    pub fn select(&mut self, name: &str) {
        self.picked = name.to_owned();
        self.stamp = None;
        self.looked = None;
    }

    /// Forces the next `tick_native_shader_pack` to check the pack's files immediately instead of
    /// waiting for `LOOK_EVERY` (used after explicit option Apply/save).
    pub fn check_now(&mut self) {
        self.looked = None;
    }

    /// Reconciles a due fingerprint scan for `path` against the last recorded stamp.
    /// Records `stamp` on cached refusals as well as new builds so reverting to a previously
    /// refused revision while another revision is building, then restoring the cancelled revision,
    /// triggers a fresh build instead of hitting `stamp == self.stamp`.
    fn reconcile_stamp(&mut self, path: &Path, stamp: Option<u64>, now: Instant) -> ScanAction {
        self.looked = Some(now);
        if stamp == self.stamp {
            return ScanAction::Poll {
                empty_stamp: stamp.is_none(),
            };
        }
        if refused(path, stamp) {
            self.stamp = stamp;
            return ScanAction::CachedRefusal;
        }
        let reload = self.stamp.is_some();
        self.stamp = stamp;
        ScanAction::Build { stamp, reload }
    }

    pub fn picking(name: &str) -> Self {
        Self {
            picked: name.to_owned(),
            ..Self::default()
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScanAction {
    /// Fingerprint unchanged since the last scan: keep polling any in-flight build.
    Poll { empty_stamp: bool },
    /// The requested revision is already in the refusal cache: cancel any superseded build.
    CachedRefusal,
    /// A new unrefused revision should be submitted for compilation.
    Build { stamp: Option<u64>, reload: bool },
}

impl Game<'_> {
    /// The pack's `[entity]` stage lights vertices `run` of the entity geometry as `kind`. A
    /// run is named once it is drawn, and with its end: what no caller names stays Horizon's
    /// own draw (items, particles, fire, beams, ropes, text, and whatever is added without a
    /// word here), so a run ends before any of those. Within a run, quads of another batch
    /// material (a translucent item in a hand, an energy swirl) and unlit overlays (the beam
    /// that heals the dragon, an End portal) stay Horizon's all the same: `EntityBatch` names
    /// only its ordinary quads, and the stage's shader passes the unlit ones on.
    /// The models the run posed on the GPU (`entity_skin`) are the vertices it drew there
    /// from `posed` on (`SkinFrame::vertices` when it began): the stage lights them alike.
    /// Kept only while the pack in use has the stage, or its rays read the entity layer
    /// (`names_entity_runs`).
    fn lit_run(&self, run: std::ops::Range<usize>, posed: u32, kind: EntityKind) {
        let stage = self.renderer.as_ref();
        if stage.is_some_and(|r| r.names_entity_runs()) {
            self.native_shaders.entity_runs.borrow_mut().push(run, kind);
            let posed = posed as usize..self.skin_frame.borrow().vertices as usize;
            self.native_shaders
                .posed_runs
                .borrow_mut()
                .push(posed, kind);
        }
    }

    /// Where the next run's posed vertices start (`lit_posed_entity`).
    pub(super) fn posed_vertices(&self) -> u32 {
        self.skin_frame.borrow().vertices
    }

    /// Vertices `run` are an entity's model: a mob's or a player's with what it holds and
    /// wears, a vehicle's, a projectile's.
    pub(super) fn lit_entity(&self, run: std::ops::Range<usize>) {
        self.lit_run(run, self.posed_vertices(), EntityKind::Entity);
    }

    /// [`Self::lit_entity`] of a run that may have posed models on the GPU since its posed
    /// vertices numbered `posed` (`posed_vertices`).
    pub(super) fn lit_posed_entity(&self, run: std::ops::Range<usize>, posed: u32) {
        self.lit_run(run, posed, EntityKind::Entity);
    }

    /// Vertices `run` are block entities' models.
    pub(super) fn lit_block_entity(&self, run: std::ops::Range<usize>) {
        self.lit_run(run, self.posed_vertices(), EntityKind::BlockEntity);
    }

    /// What `draw` adds to `out` is entities' models (`lit_entity`), on the GPU too.
    pub(super) fn lit(
        &mut self,
        out: &mut Vec<horizon_client::mesher::Vertex>,
        draw: impl FnOnce(&mut Self, &mut Vec<horizon_client::mesher::Vertex>),
    ) {
        let (start, posed) = (out.len(), self.posed_vertices());
        draw(self, out);
        self.lit_posed_entity(start..out.len(), posed);
    }

    /// Polls any completed background native shader pack build and installs/reports its outcome.
    /// Returns `true` if a finished build was discarded as stale (renderer state changed
    /// mid-compile) and needs a rebuild.
    pub(in crate::game) fn poll_native_shader_pack(&mut self) -> bool {
        if self.screenshot_sequence.is_some() {
            return false;
        }
        let outcome = self
            .renderer
            .as_mut()
            .and_then(|r| r.poll_native_shader_pack::<NativePackJobContext, NativePackBuiltMeta>());
        match outcome {
            Some(outcome) => self.apply_native_pack_outcome(outcome),
            None => false,
        }
    }

    /// Blocks until any in-flight background native shader pack build completes and applies its
    /// outcome. Returns `true` if the finished build was discarded as stale (renderer state
    /// changed mid-compile) and needs a rebuild. Used by deterministic capture paths
    /// (`HORIZON_STILL`, `screenshot-sequence`, `horizon-ctl screenshot`) and tests.
    pub(in crate::game) fn wait_native_shader_pack(&mut self) -> bool {
        let outcome = self
            .renderer
            .as_mut()
            .and_then(|r| r.wait_native_shader_pack::<NativePackJobContext, NativePackBuiltMeta>());
        match outcome {
            Some(outcome) => self.apply_native_pack_outcome(outcome),
            None => false,
        }
    }

    /// Reconciles the currently selected native shader pack before arming a deterministic capture
    /// (`horizon-ctl screenshot`, `screenshot-sequence`): resolves any newly picked pack or saved
    /// option values, starts its build immediately, and waits for it to finish and install.
    pub(in crate::game) fn settle_native_shader_pack(&mut self) {
        let selected = super::shader_packs::picked_pack(&self.settings).to_owned();
        let path = self.native_shaders.resolve_path(
            &Self::shader_packs_dir(),
            &selected,
            super::shader_packs::dev_pack().is_some(),
        );
        let file = path.file_name().unwrap_or_default().to_string_lossy();
        let picked = if selected.is_empty()
            || super::shader_packs::active_pack(&file, &self.settings.mods).is_empty()
        {
            String::new()
        } else {
            path.to_string_lossy().into_owned()
        };
        if self.native_shaders.picked != picked {
            self.native_shaders.select(&picked);
            self.set_translucent_layers(Vec::new());
            self.pack_ids = None;
            if let Some(renderer) = &mut self.renderer {
                renderer.drop_shader_packs();
            }
        }
        // Ensure the renderer's frame format matches the newly selected pack before compiling it.
        self.follow_frame();
        let native = !picked.is_empty()
            && ((super::shader_packs::dev_pack().is_none() && selected.starts_with("horizon:"))
                || horizon_shader_api::package::is_pack(&path));
        if native {
            for _ in 0..2 {
                self.native_shaders.check_now();
                let mut stale = self.tick_native_shader_pack(&path);
                if !stale
                    && self
                        .renderer
                        .as_ref()
                        .is_some_and(|r| r.native_pack_loading())
                {
                    stale = self.wait_native_shader_pack();
                }
                if !stale {
                    break;
                }
            }
        } else if self
            .renderer
            .as_ref()
            .is_some_and(|r| r.native_pack_loading())
        {
            self.wait_native_shader_pack();
        }
    }

    /// Builds the pack at `path` on a background worker thread when its files changed since it was
    /// last read, while the currently running pack keeps drawing. Under `HORIZON_STILL`, waits for
    /// the build synchronously so every frame stays deterministic. Returns `true` if a finished
    /// build was discarded as stale and needs a rebuild.
    pub(in crate::game) fn tick_native_shader_pack(&mut self, path: &Path) -> bool {
        // A reload would replace the pack a `screenshot-sequence` draws: it waits for the reply.
        if self.screenshot_sequence.is_some() {
            return false;
        }
        let now = Instant::now();
        let scan_due = self
            .native_shaders
            .looked
            .is_none_or(|t| now - t >= LOOK_EVERY);
        // Reconcile any due fingerprint scan BEFORE polling completions so applying a newer
        // revision supersedes and discards a just-finished build of an older revision rather than
        // installing it for one frame and resetting TAA history needlessly.
        if !scan_due {
            return self.poll_native_shader_pack();
        }
        let stamp = fingerprint(path);
        let (stamp, reload) = match self.native_shaders.reconcile_stamp(path, stamp, now) {
            ScanAction::Poll { empty_stamp } => {
                let stale = self.poll_native_shader_pack();
                // A pack gone (no stamp) is never read: a renderer made again holds the world for
                // nothing (`release_world`, which leaves a running pack's alone).
                if empty_stamp
                    && let Some(renderer) = &mut self.renderer
                    && !renderer.native_pack_loading()
                {
                    renderer.release_world();
                }
                return stale;
            }
            ScanAction::CachedRefusal => {
                // The requested revision is already known to fail: cancel any in-flight build for
                // an older revision so it cannot install over this refusal.
                if let Some(renderer) = &mut self.renderer {
                    renderer.cancel_native_shader_pack();
                    renderer.release_world();
                }
                return false;
            }
            ScanAction::Build { stamp, reload } => (stamp, reload),
        };
        if self.renderer.is_none() {
            return false;
        }
        let manifest_refused = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let context = NativePackJobContext {
            path: path.to_path_buf(),
            stamp,
            reload,
            started: now,
            manifest_refused: manifest_refused.clone(),
        };
        let path_owned = path.to_path_buf();
        let render = self.pack.render.clone();
        let assets = self.pack.assets.clone();
        let layer_names = self.pack.layer_names.clone();
        let res = self.res.clone();
        let ray_queries = self
            .renderer
            .as_ref()
            .is_some_and(|renderer| renderer.ray_queries());
        let submit = self
            .renderer
            .as_mut()
            .expect("renderer checked above")
            .load_native_shader_pack(context, move |cancel| {
                use std::sync::atomic::Ordering;
                if cancel.load(Ordering::Relaxed) {
                    return Err(Vec::new());
                }
                // Validate manifest compatibility on the exact `Pack` snapshot read by the worker
                // so a concurrent manifest edit cannot bypass `not_drawn` checks (no TOCTOU).
                let pack = match validate_pack_snapshot(&path_owned, Pack::read(&path_owned)) {
                    Ok(pack) => pack,
                    Err((why, refused_manifest)) => {
                        manifest_refused.store(refused_manifest, Ordering::Release);
                        return Err(vec![why]);
                    }
                };
                if cancel.load(Ordering::Relaxed) {
                    return Err(Vec::new());
                }
                let report = check(&pack)
                    .map_err(|p| p.iter().map(ToString::to_string).collect::<Vec<String>>())?;
                if cancel.load(Ordering::Relaxed) {
                    return Err(Vec::new());
                }
                // The player's values (the pack's Shader Options page), else the defaults.
                let options = &pack.manifest.options;
                let values_pack = horizon_client::shader_pack_list::native_values_pack(
                    &path_owned,
                    &pack.manifest.id,
                );
                let values = horizon_client::shader_pack_list::load_values(&values_pack, options);
                let constants = options.iter().map(|o| o.id.clone()).zip(values).collect();
                let sprites = || {
                    let layers = render.sprite_layers.iter();
                    layers.map(|(name, layer)| (name.as_str(), *layer))
                };
                let tags = pack.manifest.sprite_tags(sprites());
                let mut materials = pack.manifest.sprite_materials(sprites());
                // `blocks` lists: the sprites their models draw with, where no pattern named them.
                let listed = pack.manifest.material_blocks();
                if !listed.is_empty() {
                    let macros = horizon_shaders::Macros::standard();
                    let properties = horizon_shaders::Properties::parse(&listed, &macros);
                    let ids = horizon_shaders::BlockIds::parse(&properties);
                    let table = super::shader_packs::block_id_table_from(
                        &render,
                        &assets,
                        &layer_names,
                        &ids,
                    );
                    for (layer, id) in table.iter().step_by(2).enumerate() {
                        let layer = layer as u32;
                        if *id > 0 && !materials.iter().any(|m| m.0 == layer) {
                            materials.push((layer, *id as u32));
                        }
                    }
                    materials.sort_unstable();
                }
                let layers = sprites().map(|(_, l)| l as usize + 1).max().unwrap_or(0);
                let translucent = pack.manifest.translucent_layers(&materials, layers);
                // Each light-giving block state's `[[lights]]` colour, with its level.
                let mut lights = horizon_client::gfx::block_light::LightTable::default();
                if !pack.manifest.lights.is_empty() {
                    for (id, state) in res.facts().states.iter().enumerate() {
                        if state.emission == 0 {
                            continue;
                        }
                        if let Some(color) = pack.manifest.light_of(&state.name) {
                            lights.insert(id as u32, (color, state.emission));
                        }
                    }
                }
                // Each light source a `[ray_tracing] lights = true` pack's rays read: its
                // `[[lights]]` colour, its level and its model's `[[materials]]` id (lava left
                // out), where a stage reads them and the device traces.
                let mut sources = horizon_client::gfx::ray_lights::Sources::default();
                let traces_lights = report.stages.iter().any(|s| s.traces_lights());
                if traces_lights && ray_queries {
                    let catalog = &render.model_catalog;
                    let origin = horizon_core::position::BlockPos::new(0, 0, 0);
                    for (id, state) in res.facts().states.iter().enumerate() {
                        if state.emission == 0 {
                            continue;
                        }
                        let state_id = horizon_world::registry::BlockStateId(id as u32);
                        let models = catalog.collect(state_id, origin).unwrap_or_default();
                        let layers = models
                            .iter()
                            .flat_map(|(_, m)| m.faces.iter().map(|f| f.layer));
                        let color = pack.manifest.light_of(&state.name);
                        let light = (state.emission, state.fluid);
                        let source = horizon_client::gfx::ray_lights::source;
                        if let Some(s) = source(light, color, layers, &materials) {
                            sources.insert(id as u32, s);
                        }
                    }
                }
                if cancel.load(Ordering::Relaxed) {
                    return Err(Vec::new());
                }
                Ok((
                    (report, constants, tags, materials, (lights, sources)),
                    NativePackBuiltMeta {
                        manifest: pack.manifest,
                        translucent,
                    },
                ))
            });
        if let Err((context, why)) = submit {
            self.apply_native_pack_refusal(context, vec![why], false);
            return false;
        }
        if self.clock.still() {
            return self.wait_native_shader_pack();
        }
        false
    }

    fn apply_native_pack_outcome(
        &mut self,
        outcome: horizon_client::gfx::NativePackOutcome<NativePackJobContext, NativePackBuiltMeta>,
    ) -> bool {
        match outcome {
            horizon_client::gfx::NativePackOutcome::Cancelled => false,
            horizon_client::gfx::NativePackOutcome::Stale => {
                // Renderer state (e.g. sprite-view mipmaps or frame format) changed while this
                // build was compiling: clear our stamp so the next tick rebuilds cleanly against
                // the current renderer state.
                self.native_shaders.stamp = None;
                self.native_shaders.check_now();
                true
            }
            horizon_client::gfx::NativePackOutcome::Installed {
                context,
                labels,
                meta,
            } => {
                remember_look(&context.path, context.stamp, true, false, true);
                self.sync_native_material_maps(meta.manifest.material.is_some());
                self.set_translucent_layers(meta.translucent);
                log::info!(
                    "shader pack {} {}: {} in {} ms",
                    meta.manifest.id,
                    meta.manifest.version,
                    labels.join(", "),
                    context.started.elapsed().as_millis()
                );
                if context.reload {
                    self.system_message(Text::literal(format!(
                        "Shader pack {} reloaded",
                        meta.manifest.name
                    )));
                }
                false
            }
            horizon_client::gfx::NativePackOutcome::Refused { context, problems } => {
                let manifest_refused = context
                    .manifest_refused
                    .load(std::sync::atomic::Ordering::Acquire);
                self.apply_native_pack_refusal(context, problems, manifest_refused);
                false
            }
        }
    }

    fn apply_native_pack_refusal(
        &mut self,
        context: NativePackJobContext,
        problems: Vec<String>,
        manifest_refused: bool,
    ) {
        let running = self
            .renderer
            .as_ref()
            .is_some_and(|r| r.shader_pack_kind() == Some("native"));
        remember_look(
            &context.path,
            context.stamp,
            false,
            manifest_refused,
            running,
        );
        // A refused manifest stops its preceding pack; other failed hot reloads keep it.
        // Only a pack that still runs can retain its preceding material-map demand.
        let material_maps = !manifest_refused && running && horizon_client::sprites::labpbr_maps();
        if manifest_refused {
            if let Some(renderer) = &mut self.renderer {
                renderer.drop_shader_packs();
            }
            self.set_translucent_layers(Vec::new());
        } else if let Some(renderer) = &mut self.renderer {
            renderer.release_world();
        }
        self.sync_native_material_maps(material_maps);
        let name = context
            .path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        log::warn!("shader pack {name} refused: {}", problems.join("; "));
        self.system_message(Text::colored(format!("Shader pack {name} refused:"), RED));
        for p in problems {
            self.system_message(Text::colored(p, RED));
        }
    }

    fn sync_native_material_maps(&mut self, material_maps: bool) {
        if material_maps != horizon_client::sprites::labpbr_maps() {
            if let Some(renderer) = &mut self.renderer {
                renderer.drop_shader_packs();
            }
            // An existing resource load resets this pack's stamp when it applies. Let it
            // finish before changing what its workers read; the next build rechecks demand.
            if self.rest_loader.is_none() && self.pack_reload.is_none() && !self.pending_pack_reload
            {
                horizon_client::sprites::set_labpbr_maps(material_maps);
                log::info!("Horizon: reloading resources for native pack material-map demand");
                self.reload_packs();
            }
        }
    }
}

struct NativePackJobContext {
    path: PathBuf,
    stamp: Option<u64>,
    reload: bool,
    started: Instant,
    manifest_refused: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

struct NativePackBuiltMeta {
    manifest: Manifest,
    translucent: Vec<bool>,
}

/// Validates a loaded pack snapshot against renderer compatibility (`not_drawn`) without a
/// separate manifest read on another thread. On error, returns `(reason, manifest_refused)`:
/// `true` when the manifest itself failed to read or asks for an unserved feature (stops any
/// previously running pack), `false` when only a stage file could not be read.
fn validate_pack_snapshot(
    path: &Path,
    read: std::result::Result<Pack, String>,
) -> std::result::Result<Pack, (String, bool)> {
    match read {
        Ok(pack) => match not_drawn(&pack.manifest) {
            Some(why) => Err((why.into(), true)),
            None => Ok(pack),
        },
        Err(err) => {
            let refused_manifest = match Pack::read_manifest(path) {
                Ok(manifest) => not_drawn(&manifest).is_some(),
                Err(_) => true,
            };
            Err((err, refused_manifest))
        }
    }
}

/// Why the manifest of the pack at `path` is refused, if it is: it doesn't read (a shader API
/// line this build doesn't serve, a mistake in it), or it asks for what isn't drawn yet
/// (`not_drawn`). A pack that ran before stops then; one whose stages fail keeps its last good
/// version drawing.
#[cfg(test)]
fn manifest_refused(path: &Path) -> Option<String> {
    match Pack::read_manifest(path) {
        Ok(manifest) => not_drawn(&manifest).map(Into::into),
        Err(e) => Some(e),
    }
}

/// Whether the served native material stage can read resource-pack companion maps.
pub(super) fn needs_material_maps(manifest: &Manifest) -> bool {
    manifest.material.is_some() && not_drawn(manifest).is_none()
}

/// Why the game doesn't draw a pack the check may still pass: `[frame] primaries = "bt2020"`
/// isn't served yet (#14584), and neither is `[motion]`. Serving `"bt2020"` must also keep HDR's
/// Color Boost (`gfx::hdr::gamut`) off for that pack: it stretches BT.709 colours, and a BT.2020
/// pack's are already where it meant them.
fn not_drawn(m: &Manifest) -> Option<&'static str> {
    if m.primaries == Primaries::Bt2020 {
        return Some(
            "[frame] primaries = \"bt2020\" isn't served yet: leave it out, or set \"bt709\"",
        );
    }
    m.motion.is_some().then_some(
        "[motion] needs a later Horizon: this one checks the motion guide but doesn't draw it",
    )
}

/// A hash of the names, sizes and modification times of the pack's files and of its saved
/// option values (`None` when the pack is unreadable).
pub(super) fn fingerprint(path: &Path) -> Option<u64> {
    fn walk(path: &Path, hasher: &mut std::hash::DefaultHasher) -> Option<()> {
        let meta = std::fs::metadata(path).ok()?;
        path.hash(hasher);
        if meta.is_dir() {
            let mut entries: Vec<_> = std::fs::read_dir(path)
                .ok()?
                .flatten()
                .map(|e| e.path())
                .collect();
            entries.sort();
            for entry in entries {
                walk(&entry, hasher)?;
            }
        } else {
            meta.len().hash(hasher);
            meta.modified().ok()?.hash(hasher);
        }
        Some(())
    }
    let mut hasher = std::hash::DefaultHasher::new();
    walk(path, &mut hasher)?;
    // The player's option values, when there are any.
    if let Ok(manifest) = Pack::read_manifest(path) {
        let values_pack = horizon_client::shader_pack_list::native_values_pack(path, &manifest.id);
        let values = horizon_client::shader_pack_list::values_path(&values_pack);
        let _ = walk(&values, &mut hasher);
    }
    Some(hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::not_drawn;
    use horizon_shader_api::Manifest;

    #[test]
    fn outcomes_and_refusals_survive_renderer_resets() {
        use super::{NativeShaders, fingerprint, outcome, refused, remember};
        let root = std::env::temp_dir().join(format!("horizon-acceptance-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("post.wgsl");
        std::fs::write(&file, "first files").unwrap();
        let stamp = fingerprint(&root);
        assert_eq!(outcome(&root), None);
        remember(&root, stamp, true);
        assert_eq!(outcome(&root), Some(true));
        // Policy, SDK and GPU refusals all reach this same outcome before any resource reload.
        remember(&root, stamp, false);
        for _ in 0..3 {
            let mut reset = NativeShaders::picking(&root.to_string_lossy());
            reset.select(&root.to_string_lossy());
            assert_eq!(outcome(&root), Some(false));
            assert!(refused(&root, fingerprint(&root)));
        }
        std::fs::write(&file, "repaired files with a different size").unwrap();
        assert!(!refused(&root, fingerprint(&root)));
        remember(&root, fingerprint(&root), true);
        assert_eq!(outcome(&root), Some(true));
        std::fs::remove_dir_all(root).unwrap();
    }

    fn manifest(extra: &str) -> Manifest {
        Manifest::parse(&format!(
            "schema = 1\nid = \"p\"\nname = \"P\"\nversion = \"1\"\napi = \"0.2\"\n\
             [[post]]\nfile = \"post.wgsl\"\n{extra}"
        ))
        .unwrap()
    }

    #[test]
    fn stable_options_are_in_the_reload_signature_and_override_keeps_its_path() {
        use super::{NativeShaders, fingerprint};
        let dir =
            std::env::temp_dir().join(format!("horizon-native-identity-{}", std::process::id()));
        let pack = dir.join("native-test_mod");
        std::fs::create_dir_all(&pack).unwrap();
        std::fs::write(pack.join(horizon_shader_api::MANIFEST),
            "schema = 1\nid = \"p\"\nname = \"P\"\nversion = \"1\"\napi = \"0.2\"\n[[post]]\nfile = \"post.wgsl\"\n").unwrap();
        std::fs::write(pack.join("post.wgsl"), "fixture").unwrap();
        let mut state = NativeShaders::default();
        assert_eq!(state.resolve_path(&dir, "horizon:p", false), pack);
        let first = fingerprint(&pack).unwrap();
        let manifest_path = pack.join(horizon_shader_api::MANIFEST);
        let manifest = std::fs::read_to_string(&manifest_path).unwrap();
        std::fs::write(&manifest_path, "temporarily invalid").unwrap();
        state.resolved_at = Some(std::time::Instant::now() - super::LOOK_EVERY);
        assert_eq!(state.resolve_path(&dir, "horizon:p", false), pack);
        assert_ne!(fingerprint(&pack).unwrap(), first);
        std::fs::write(&manifest_path, manifest).unwrap();
        let first = fingerprint(&pack).unwrap();
        let values = horizon_client::shader_pack_list::native_values_pack(&pack, "p");
        let path = horizon_client::shader_pack_list::values_path(&values);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "strength=2\n").unwrap();
        assert_ne!(fingerprint(&pack).unwrap(), first);
        std::fs::rename(&pack, dir.join("renamed")).unwrap();
        state.resolved_at = None;
        assert_eq!(
            state.resolve_path(&dir, "horizon:p", false),
            dir.join("renamed")
        );
        let manifest_path = dir.join("renamed").join(horizon_shader_api::MANIFEST);
        let text = std::fs::read_to_string(&manifest_path)
            .unwrap()
            .replace("id = \"p\"", "id = \"other\"");
        std::fs::write(manifest_path, text).unwrap();
        state.resolved_at = Some(std::time::Instant::now() - super::LOOK_EVERY);
        assert_eq!(
            state.resolve_path(&dir, "horizon:p", false),
            dir.join("horizon:p"),
            "a valid different manifest ID must not replace the selected identity"
        );
        let mut override_state = NativeShaders::default();
        assert_eq!(
            override_state.resolve_path(&dir, "native-test_mod", true),
            pack,
            "an explicit missing filesystem override must not switch to another installed pack"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Every way a manifest is refused stops the pack that ran: another API line, what isn't
    /// drawn yet, a mistake in it. A stage that fails is not one of them: its last good
    /// version keeps drawing.
    #[test]
    fn a_refused_manifest_stops_the_pack_and_a_failing_stage_does_not() {
        use super::manifest_refused;
        let pack = std::env::temp_dir().join(format!("horizon-refused-{}", std::process::id()));
        std::fs::create_dir_all(&pack).unwrap();
        let write = |api: &str, extra: &str| {
            let text = format!(
                "schema = 1\nid = \"p\"\nname = \"P\"\nversion = \"1\"\napi = \"{api}\"\n\
                 [[post]]\nfile = \"post.wgsl\"\n{extra}"
            );
            std::fs::write(pack.join(horizon_shader_api::MANIFEST), text).unwrap();
        };
        // A stage the check refuses: the manifest itself is taken.
        std::fs::write(pack.join("post.wgsl"), "not wgsl").unwrap();
        write("0.2", "");
        assert_eq!(manifest_refused(&pack), None);
        let snapshot =
            super::validate_pack_snapshot(&pack, horizon_shader_api::Pack::read(&pack)).unwrap();
        assert!(horizon_shader_api::check(&snapshot).is_err());
        for (api, extra, why) in [
            ("0.1", "", "needs shader API 0.1; this Horizon serves 0.2"),
            (
                "0.2",
                "[frame]\nblend = \"linear\"\nprimaries = \"bt2020\"\n",
                "primaries = \"bt2020\" isn't served yet",
            ),
            ("0.2", "[frame\n", "horizon-shaders.toml"),
        ] {
            write(api, extra);
            let refused = manifest_refused(&pack).unwrap_or_default();
            assert!(refused.contains(why), "{api} {extra}: {refused}");
            let (err, refused_manifest) =
                super::validate_pack_snapshot(&pack, horizon_shader_api::Pack::read(&pack))
                    .expect_err("manifest refusal");
            assert!(refused_manifest, "{api} {extra} must stop running pack");
            assert!(err.contains(why), "{api} {extra}: {err}");
        }
        std::fs::remove_dir_all(pack).unwrap();
    }

    #[test]
    fn cached_refusal_updates_stamp_so_restoring_cancelled_revision_rebuilds() {
        use super::{NativeShaders, ScanAction, remember};
        let path =
            std::env::temp_dir().join(format!("horizon-cached-refusal-{}", std::process::id()));
        let now = std::time::Instant::now();
        let stamp_a = Some(1_u64);
        let stamp_b = Some(2_u64);
        // Revision B was previously refused.
        remember(&path, stamp_b, false);
        let mut state = NativeShaders::picking(&path.to_string_lossy());
        // Revision A starts building.
        assert_eq!(
            state.reconcile_stamp(&path, stamp_a, now),
            ScanAction::Build {
                stamp: stamp_a,
                reload: false,
            }
        );
        // Reverting to cached-refused revision B cancels A and records B's stamp.
        assert_eq!(
            state.reconcile_stamp(&path, stamp_b, now),
            ScanAction::CachedRefusal
        );
        // Remaining on B does not loop rebuilds or cancellations.
        assert_eq!(
            state.reconcile_stamp(&path, stamp_b, now),
            ScanAction::Poll { empty_stamp: false }
        );
        // Restoring revision A must trigger a fresh build rather than hitting the cancelled A stamp.
        assert_eq!(
            state.reconcile_stamp(&path, stamp_a, now),
            ScanAction::Build {
                stamp: stamp_a,
                reload: true,
            }
        );
    }

    #[test]
    fn a_pack_is_drawn_but_for_what_isnt_served_yet() {
        // Either blend: `"linear"` is drawn as `"gamma"` is. The entity stage is drawn too.
        for frame in [
            "",
            "[frame]\nblend = \"linear\"\n",
            "[frame]\nprimaries = \"bt709\"\n",
            "[entity]\nfile = \"e.wgsl\"\n",
        ] {
            assert_eq!(not_drawn(&manifest(frame)), None, "{frame}");
        }
        let wide = "[frame]\nblend = \"linear\"\nprimaries = \"bt2020\"\n";
        let why = not_drawn(&manifest(wide)).unwrap();
        assert!(
            why.contains("primaries = \"bt2020\" isn't served yet"),
            "{why}"
        );
        let why = not_drawn(&manifest("[motion]\n")).unwrap();
        assert!(why.contains("[motion] needs a later Horizon"), "{why}");
        let kept =
            "[[resource]]\nname = \"cache\"\nbuffer = 64\nelement = \"u32\"\nkeep = \"view\"\n";
        assert_eq!(not_drawn(&manifest(kept)), None);
        let traced = format!("{kept}[ray_tracing]\n");
        assert_eq!(not_drawn(&manifest(&traced)), None);
    }
}
