//! One of Horizon's own shader packs while it runs (docs/shaders.md): the frame its stages
//! read, its sky, material, entity and post stages, and the timer of their passes.

use std::borrow::Cow;
use std::sync::atomic::{AtomicBool, Ordering};

use glam::Mat4;
use horizon_shader_api::check::CheckedStage;
use horizon_shader_api::{Jitter, Report, StageKind};

/// Failure to build a `NativePack`: either cancelled because a newer build superseded it, or
/// refused with the reason to show the player.
#[derive(Debug)]
pub enum BuildError {
    Cancelled,
    Refused(String),
}

impl From<String> for BuildError {
    fn from(reason: String) -> Self {
        Self::Refused(reason)
    }
}

fn check_cancelled(cancel: &AtomicBool) -> Result<(), BuildError> {
    if cancel.load(Ordering::Relaxed) {
        Err(BuildError::Cancelled)
    } else {
        Ok(())
    }
}

use crate::mesher::Vertex;

use super::displace::Displacement;
use super::external::{
    Capabilities, FrameStart, TimingSetup, WorldOutput, WorldPost, WorldTargets,
};
use super::native_entity::{EntityPipelines, NativeEntity};
use super::native_material::{DisplacedShape, MaterialPipelines, NativeMaterial, TerrainShape};
use super::native_post::kept::StageReads;
use super::native_post::{NativeFrame, NativePost};
use super::native_shadow::{NativeShadow, ShadowGrid};
use super::native_sky::NativeSky;
use super::pack_timing::PassTiming;
use super::rt::scene::{RayScene, Traced};

pub struct NativePack {
    pub frame: NativeFrame,
    pub sky: Option<NativeSky>,
    pub material: Option<NativeMaterial>,
    pub entity: Option<NativeEntity>,
    /// The post chain (none without `[[post]]`), the decode before it and the tonemap after.
    pub post: NativePost,
    /// With a sky stage, what of Minecraft's sky still draws over it (`[sky]`).
    pub vanilla_clouds: bool,
    pub vanilla_sun: bool,
    pub vanilla_moon: bool,
    pub vanilla_stars: bool,
    /// `[sky] sun_path_rotation`, degrees: also turns the sun, moon and stars Minecraft draws.
    pub sun_path_rotation: f32,
    pub weather: Option<wgpu::RenderPipeline>,
    /// The draw of the shadow map the stages read, if the pack asks for one.
    pub shadow: Option<NativeShadow>,
    /// `[displacement]`'s grid, while it is on and the resource packs have height maps.
    pub displacement: Option<Displacement>,
    /// GPU time of each pass, while the renderer asks for it (`pack_timing.rs`).
    pub timing: Option<PassTiming>,
    /// It declares `[ray_tracing]`: the renderer keeps a scene for it (`replace_executor`).
    pub traces: bool,
    /// Its terrain pipelines bind the arenas' quad looks (`looks::wanted`): sections carry them.
    pub looks: bool,
    /// `[frame] jitter`: it asks for the world's sub-pixel offset under its own post chain.
    pub jitter: bool,
    /// With kept resources, what lets a rebuild keep them (`carry`).
    kept: Option<super::native_post::kept::Signature>,
}

/// What the game reads of the running pack's manifest beside its executor
/// (`Renderer::native_pack_name`, `light_color`, `particle_lights`).
pub struct Metadata {
    /// The manifest's `name`.
    pub name: String,
    /// `[[particle_lights]]`.
    pub particle_lights: Vec<horizon_shader_api::manifest::ParticleLight>,
    /// `[[lights]]`, for the colour of held and dropped items' light.
    pub light_colors: Vec<horizon_shader_api::manifest::LightColor>,
}

impl Metadata {
    pub fn of(report: &Report) -> Self {
        Self {
            name: report.manifest.name.clone(),
            particle_lights: report.manifest.particle_lights.clone(),
            light_colors: report.manifest.lights.clone(),
        }
    }
}

/// Whether the option `id` (a bool, or a number above 0) is on, or there is none to ask.
fn option_on(constants: &[(String, f64)], id: Option<&String>) -> bool {
    id.is_none_or(|id| {
        constants
            .iter()
            .find(|(k, _)| k == id)
            .is_none_or(|(_, v)| *v != 0.0)
    })
}

/// Whether a stage of the pack reads the world before translucent terrain (`opaque_depth`,
/// `opaque_color`), including reads through functions called by a stage.
fn reads_opaque(stages: &[CheckedStage], post_opaque: bool) -> bool {
    let reads = |s: &CheckedStage, function: naga::Handle<naga::Function>| {
        s.module
            .global_variables
            .iter()
            .filter(|(_, g)| matches!(g.name.as_deref(), Some("opaque_depth" | "opaque_color")))
            .any(|(global, _)| !s.info[function][global].is_empty())
    };
    stages
        .iter()
        .filter(|s| {
            s.kind == StageKind::Material
                || (post_opaque && matches!(s.kind, StageKind::Post | StageKind::Compute))
        })
        .any(|s| {
            s.module
                .functions
                .iter()
                .filter(|(_, f)| {
                    f.name.as_deref() == Some(s.kind.function())
                        || (s.kind == StageKind::Material
                            && f.name.as_deref() == Some("displacement"))
                })
                .any(|(function, _)| reads(s, function))
        })
}

/// `[displacement]`'s cells: the value of its `cells_option`, if any.
fn chosen_cells(
    d: &horizon_shader_api::manifest::Displacement,
    options: &[horizon_shader_api::manifest::PackOption],
    constants: &[(String, f64)],
) -> u32 {
    let chosen = d.cells_option.as_ref().and_then(|id| {
        let option = options.iter().find(|o| o.id == *id)?;
        let horizon_shader_api::manifest::OptionKind::Choice { values, .. } = &option.kind else {
            return None;
        };
        let (_, index) = constants.iter().find(|(k, _)| k == id)?;
        values.get(*index as usize)?.parse().ok()
    });
    chosen.unwrap_or(d.cells)
}

/// `[shadow]` with the option values `constants`: none while its `option` is off, else the
/// resolution and distance its `resolution_option` and `distance_option` pick.
fn chosen_shadow(
    s: &horizon_shader_api::manifest::Shadow,
    options: &[horizon_shader_api::manifest::PackOption],
    constants: &[(String, f64)],
) -> Option<horizon_shader_api::manifest::Shadow> {
    let number = |id: Option<&String>| {
        let option = options.iter().find(|o| Some(&o.id) == id)?;
        let (_, value) = constants.iter().find(|(k, _)| *k == option.id)?;
        option.kind.number(*value)
    };
    option_on(constants, s.option.as_ref()).then(|| horizon_shader_api::manifest::Shadow {
        resolution: number(s.resolution_option.as_ref()).map_or(s.resolution, |r| r as u32),
        distance: number(s.distance_option.as_ref()).map_or(s.distance, |d| d as f32),
        ..s.clone()
    })
}

/// Installs the kept resources of `new`, the pack that runs from now on, where `old` ran
/// (`native_post::kept::install`), at the R of its frames `size`. Without the video memory for
/// them, why, and whether `old` can keep running: its own are made again, as at its load, unless
/// there is no room for those either.
pub(in crate::gfx) fn install_kept(
    device: &wgpu::Device,
    new: Option<&mut NativePack>,
    mut old: Option<&mut NativePack>,
    size: [u32; 2],
) -> Result<(), (String, bool)> {
    let posts = (
        new.map(|p| &mut p.post),
        old.as_deref_mut().map(|p| &mut p.post),
    );
    let Err(refused) = super::native_post::kept::install(device, posts.0, posts.1, size) else {
        return Ok(());
    };
    let runs = old.is_none_or(|old| {
        let made = old.reload_kept(device);
        if let Err(e) = &made {
            log::warn!("Horizon shader pack: the running pack stops: {e}");
        }
        made.is_ok()
    });
    Err((refused, runs))
}

impl NativePack {
    /// Writes this frame's inputs (`NativeFrame::write`), and for a `[ray_tracing]` pack where
    /// its passes and material and entity stages trace this frame's `scene` from, once it is built.
    pub(in crate::gfx) fn write(
        &mut self,
        queue: &wgpu::Queue,
        frame: &super::Frame,
        size: [u32; 2],
        display: super::output::DisplayValues,
        scene: Option<&Traced<'_>>,
    ) {
        let post = self.post.trace(queue, scene);
        let material = self.material.as_ref();
        let terrain = material.and_then(|m| m.trace(queue, scene));
        let entity = self.entity.as_ref().and_then(|e| e.scene.as_ref());
        let entities = entity.and_then(|e| e.trace(queue, scene));
        self.frame.ray_distance = post.or(terrain).or(entities);
        self.frame.write(queue, frame, size, display);
        if let Some(epochs) = &self.frame.epochs {
            self.post.kept_frame(epochs.current(), size);
        }
    }

    /// Takes the kept resources and epochs of `old`, the pack before, when this one declares
    /// the same and composes the same stages (an option change rebuilds a pack): then the
    /// changed options' `resets` clear theirs. Else this pack's stay fresh, loaded.
    pub(in crate::gfx) fn carry(&mut self, old: &mut NativePack) {
        let (Some(new), Some(was)) = (&self.kept, &old.kept) else {
            return;
        };
        let Some(epochs) = &mut old.frame.epochs else {
            return;
        };
        if new.carry(was, epochs) {
            self.post.carry(&mut old.post);
            self.frame.carry(&mut old.frame);
        }
    }

    /// Makes this pack's kept resources again, as zeros, after the pack that was to replace it
    /// had no room for its own (`install_kept`), at R `size`: it sees a load.
    fn reload_kept(&mut self, device: &wgpu::Device) -> Result<(), String> {
        if let Some(epochs) = &mut self.frame.epochs {
            epochs.note(horizon_shader_api::kept::REASON_LOAD);
        }
        super::native_post::kept::reinstall(device, &mut self.post)
    }

    /// Joins the frame's group with this frame's `scene` from `camera` for material and entity stages
    /// that trace, and with the kept resources they read: after the frame's group changed for
    /// this frame and the kept resources' frame started (`before_world`), before the world
    /// draws.
    pub(in crate::gfx) fn bind_scene(
        &mut self,
        device: &wgpu::Device,
        scene: Option<&RayScene>,
        camera: glam::DVec3,
    ) {
        let scene = scene.and_then(|s| s.traced(camera));
        self.bind_terrain(device, scene.as_ref());
    }

    /// `bind_scene` with this frame's `scene` as the camera traces it.
    pub(in crate::gfx) fn bind_terrain(
        &mut self,
        device: &wgpu::Device,
        scene: Option<&Traced<'_>>,
    ) {
        if let Some(m) = &mut self.material {
            m.bind(device, &self.frame, scene, &self.post);
        }
        if let Some(entity) = &mut self.entity
            && let Some(trace) = &mut entity.scene
        {
            let reads = entity.kept.as_ref().and_then(|k| k.reads(&self.post));
            trace.bind(device, &self.frame, scene, reads);
        }
        self.bind_kept(device);
    }

    /// Whether the pack's rays read the scene's entity layer: its files name `RAY_ENTITY`, and
    /// its stages trace on this device.
    pub(in crate::gfx) fn traces_entities(&self) -> bool {
        let material = self.material.as_ref().and_then(|m| m.scene.as_ref());
        let entity = self.entity.as_ref().and_then(|e| e.scene.as_ref());
        self.post.traces_entities()
            || material.is_some_and(|t| t.traces_entities())
            || entity.is_some_and(|t| t.traces_entities())
    }

    /// Whether the pack's rays read the terrain's looks (`Report::ray_looks`), and its stages
    /// trace on this device.
    pub(in crate::gfx) fn traces_looks(&self) -> bool {
        let material = self.material.as_ref().and_then(|m| m.scene.as_ref());
        let entity = self.entity.as_ref().and_then(|e| e.scene.as_ref());
        self.post.traces_looks()
            || material.is_some_and(|t| t.traces_looks())
            || entity.is_some_and(|t| t.traces_looks())
    }

    /// The pack's entity stage and the group 3 it draws with this frame.
    pub(in crate::gfx) fn entities(&self) -> Option<(&EntityPipelines, &wgpu::BindGroup)> {
        let entity = self.entity.as_ref()?;
        let kept = entity
            .kept
            .as_ref()
            .and_then(|k| k.group(self.post.parity()));
        Some(entity.drawn(kept.unwrap_or(&self.frame.group), self.post.parity()))
    }

    /// The pipelines the pack's terrain draws with this frame, and its group 3.
    pub(in crate::gfx) fn terrain(&self) -> Option<(&MaterialPipelines, &wgpu::BindGroup)> {
        Some((self.material.as_ref()?).drawn(&self.frame.group, self.post.parity()))
    }

    /// The pack of the checked stages `report` with the option values `constants`, drawing
    /// onto the world's targets as `terrain` describes; the sky reads the terrain's group 0.
    /// Also returns its stages' labels, in the order they draw. Errors name the stage wgpu
    /// refused. `lights` holds the `[[lights]]` colours of the block states they name,
    /// `sources` each listed light source's (`ray_lights::source`), for a pack whose rays read
    /// them (`[ray_tracing] lights = true`).
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        terrain: &TerrainShape<'_>,
        report: Report,
        constants: &[(String, f64)],
        (lights, sources): (super::block_light::LightTable, super::ray_lights::Sources),
    ) -> Result<(Self, Vec<String>), String> {
        let cancel = AtomicBool::new(false);
        Self::new_cancellable(
            device,
            queue,
            terrain,
            report,
            constants,
            (lights, sources),
            &cancel,
        )
        .map_err(|e| match e {
            BuildError::Refused(reason) => reason,
            BuildError::Cancelled => "cancelled".into(),
        })
    }

    /// As [`Self::new`], checking `cancel` between stages and passes so a superseded background
    /// build stops early and drops partial GPU pipelines on the worker thread.
    pub fn new_cancellable(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        terrain: &TerrainShape<'_>,
        report: Report,
        constants: &[(String, f64)],
        (lights, sources): (super::block_light::LightTable, super::ray_lights::Sources),
        cancel: &AtomicBool,
    ) -> Result<(Self, Vec<String>), BuildError> {
        check_cancelled(cancel)?;
        // A backend may refuse a layout or pipeline the pack asks for (a stage's bindings
        // past its sampled-texture limit, say, as OpenGL's 16 was before #10095): the pack
        // is refused with that error instead of a validation panic in the renderer. A stage
        // the backend can't translate (OpenGL and a depth `textureLoad`, #18349) is an
        // internal error, which a validation scope lets through: each scope takes both.
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
        let built = Self::build(
            device,
            queue,
            terrain,
            report,
            constants,
            (lights, sources),
            cancel,
        );
        let internal = pollster::block_on(internal.pop());
        let refused = internal.or(pollster::block_on(validation.pop()));
        if cancel.load(Ordering::Relaxed) {
            return Err(BuildError::Cancelled);
        }
        match (built, refused) {
            (Ok(built), None) => Ok(built),
            (Ok(_), Some(e)) => Err(BuildError::Refused(e.to_string())),
            (Err(e), _) => Err(e),
        }
    }

    fn build(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        terrain: &TerrainShape<'_>,
        report: Report,
        constants: &[(String, f64)],
        (lights, sources): (super::block_light::LightTable, super::ray_lights::Sources),
        cancel: &AtomicBool,
    ) -> Result<(Self, Vec<String>), BuildError> {
        check_cancelled(cancel)?;
        let (globals, format) = (terrain.globals, terrain.format);
        let kept = super::native_post::kept::Signature::of(&report, constants);
        let textures = super::native_post::pack_textures(device, queue, &report);
        check_cancelled(cancel)?;
        // `[shadow]`, unless its option is off, at the size its options pick.
        let chosen = report
            .shadow
            .as_ref()
            .and_then(|s| chosen_shadow(s, &report.manifest.options, constants));
        let settings = chosen
            .as_ref()
            .map(|settings| super::native_shadow::fitted(device, settings));
        let mut frame = NativeFrame::new(
            device,
            settings.as_ref(),
            textures,
            lights,
            terrain.pbr,
            terrain.linear,
        );
        let sun_path_rotation = report
            .sky
            .as_ref()
            .and_then(|s| s.sun_path_rotation)
            .unwrap_or(0.0);
        frame.set_sun_path_rotation(sun_path_rotation);
        // One buffer of light sources for every stage of a `[ray_tracing] lights = true` pack
        // whose files name them, on a device that traces: the passes' bind groups take it as
        // they are made. Its sources and worker wait for a pass that reads them (below).
        if report.stages.iter().any(CheckedStage::traces_lights) && super::rt::query::on(device) {
            frame.ray_lights = Some(super::ray_lights::RayLights::new(device));
        }
        let reach = report.manifest.kept.reach;
        frame.epochs = (kept.as_ref()).map(|_| super::native_post::kept::Epochs::new(reach));
        check_cancelled(cancel)?;
        // `[displacement]`, unless its option is off; nothing to displace without heights.
        let displacement = report
            .manifest
            .displacement
            .as_ref()
            .filter(|d| option_on(constants, d.option.as_ref()))
            .filter(|_| terrain.pbr.is_some())
            .map(|d| {
                // No more cells than the height maps have texels.
                let texels = terrain.pbr.map_or(16, |p| p[0].width());
                let cells = chosen_cells(d, &report.manifest.options, constants)
                    .min(texels)
                    .max(1);
                let steps =
                    d.steps_option.is_some() && option_on(constants, d.steps_option.as_ref());
                Displacement::new(
                    device,
                    d.distance,
                    cells,
                    steps,
                    &frame.pbr[0],
                    report.shadow.is_some(),
                    terrain.looks,
                )
            });
        check_cancelled(cancel)?;
        let shadow = chosen
            .as_ref()
            .map(|s| {
                let vertex = report.stages.iter().find(|s| s.kind == StageKind::Vertex);
                // `[displacement]`'s grid casts too, from the material's module.
                let material = report.stages.iter().find(|s| s.kind == StageKind::Material);
                let grid = displacement.as_ref().and_then(|d| {
                    let picks = d.shadow.as_ref()?;
                    Some(ShadowGrid {
                        distance: d.distance,
                        cells: picks.cells,
                        steps: picks.steps,
                        material: material?,
                        layout: &d.draw_layout,
                        normals: &frame.pbr[0],
                    })
                });
                NativeShadow::new(
                    device,
                    terrain,
                    &frame,
                    &s.solid_materials,
                    vertex,
                    constants,
                    grid,
                )
            })
            .transpose()?;
        // The mirror is drawn for a material that reads it, while `[reflection] option` is on.
        let option = report.reflection.as_ref().and_then(|r| r.option.as_ref());
        frame.reflects = report
            .stages
            .iter()
            .find(|s| s.kind == StageKind::Material)
            .is_some_and(|s| s.reads("reflection"))
            && option_on(constants, option);
        frame.reads_opaque = reads_opaque(&report.stages, report.manifest.post_opaque);
        let fog = report.material_fog;
        let coverage = report.manifest.material_coverage;
        let keeps = |key: fn(&horizon_shader_api::Stage) -> Option<bool>| {
            report.sky.as_ref().and_then(key).unwrap_or(true)
        };
        let vanilla_clouds = keeps(|s| s.clouds);
        let vanilla_sun = keeps(|s| s.sun);
        let vanilla_moon = keeps(|s| s.moon);
        let vanilla_stars = keeps(|s| s.stars);
        check_cancelled(cancel)?;
        let weather = report
            .stages
            .iter()
            .find(|s| s.kind == StageKind::Weather)
            .map(|stage| {
                super::weather::native_pipeline(
                    device,
                    format,
                    terrain.weather_groups[0],
                    terrain.weather_groups[1],
                    &frame,
                    stage,
                    constants,
                )
            })
            .transpose()?;
        let weather_label = report
            .stages
            .iter()
            .find(|s| s.kind == StageKind::Weather)
            .map(|s| s.label.clone());
        let has_vertex = report.stages.iter().any(|s| s.kind == StageKind::Vertex);
        let (mut post, others): (Vec<_>, Vec<_>) = report.stages.into_iter().partition(|s| {
            matches!(
                s.kind,
                StageKind::Post | StageKind::Compute | StageKind::BeforeWorld
            )
        });
        check_cancelled(cancel)?;
        let sky = others
            .iter()
            .find(|s| s.kind == StageKind::Sky)
            .map(|stage| {
                let kept = StageReads::new(
                    device,
                    &report.manifest,
                    report.manifest.sky.as_ref(),
                    &frame,
                );
                NativeSky::new(device, globals, (&frame, kept), stage, constants, format)
            })
            .transpose()?;
        check_cancelled(cancel)?;
        let mut material = others
            .iter()
            .find(|s| s.kind == StageKind::Material)
            .map(|stage| {
                let displaced = displacement.as_ref().map(|d| DisplacedShape {
                    distance: d.distance,
                    cells: d.view.cells,
                    steps: d.view.steps,
                    layout: &d.draw_layout,
                });
                let listed = report.manifest.material.as_ref();
                let kept = StageReads::new(device, &report.manifest, listed, &frame);
                NativeMaterial::new(
                    device,
                    terrain,
                    &frame,
                    stage,
                    constants,
                    fog,
                    coverage,
                    has_vertex,
                    displaced.as_ref(),
                    kept,
                )
            })
            .transpose()?;
        check_cancelled(cancel)?;
        let mut entity = others
            .iter()
            .find(|s| s.kind == StageKind::Entity)
            .map(|stage| {
                let groups = [terrain.entities, terrain.retained_entities];
                let manifest = &report.manifest;
                let listed = manifest.entity.as_ref();
                let kept = StageReads::new(device, manifest, listed, &frame);
                let shape = super::native_post::trace::TraceShape::of(terrain, false, None);
                let stage = (stage, kept, &shape);
                NativeEntity::new(device, format, groups, &frame, stage, constants, manifest)
            })
            .transpose()?;
        check_cancelled(cancel)?;
        // The decode and the tonemap run without a chain too.
        let tonemap = others.iter().find(|s| s.kind == StageKind::Tonemap);
        // A `[ray_tracing]` pack's post passes as the check built them against the scene; its
        // compute passes keep theirs (`ComputePass::scene`). Whether any of them, compute and
        // before-world passes included, reads the entity layer.
        let entities = post.iter().any(CheckedStage::traces_entities);
        // The light sources, once for the pack: each pass that reads them binds them.
        let lights = frame.ray_lights.as_ref().map(|l| l.buffer.clone());
        let scenes: Vec<(String, naga::Module)> = (post.iter_mut())
            .filter(|s| s.kind == StageKind::Post)
            .filter_map(|s| Some((s.label.clone(), s.scene.take()?)))
            .collect();
        let manifest = &report.manifest;
        let mut post = NativePost::new_cancellable(
            device, post, manifest, constants, format, &frame, tonemap, cancel,
        )?;
        check_cancelled(cancel)?;
        // One scene for every stage, as `frame.has_ray_tracing` says: where the material or the
        // entity stage traces and the device refuses its pipelines, the other stages keep
        // missing too.
        let wanted = |kind| {
            let wants = |s: &CheckedStage| s.kind == kind && s.scene_source.is_some();
            others.iter().any(wants)
        };
        let lacks = |kind, built: bool| wanted(kind) && !built;
        let refused = super::rt::query::on(device)
            && (lacks(
                StageKind::Material,
                material.as_ref().is_some_and(|m| m.scene.is_some()),
            ) || lacks(
                StageKind::Entity,
                entity.as_ref().is_some_and(|e| e.scene.is_some()),
            ));
        let tracing = !scenes.is_empty() || post.computes_trace();
        let posted = tracing && !refused && {
            let shape = super::native_post::trace::TraceShape::of(terrain, entities, lights);
            post.trace_scene_cancellable(device, scenes, constants, &shape, &frame.pbr, cancel)?
        };
        if refused || (tracing && !posted) {
            material.iter_mut().for_each(|m| m.scene = None);
            entity.iter_mut().for_each(|e| e.scene = None);
        }
        check_cancelled(cancel)?;
        // Nothing is built against the scene that reads them (refused, or past the device's
        // limits): no list to keep.
        let terrain_lights = material.as_ref().and_then(|m| m.scene.as_ref());
        let entity_lights = entity.as_ref().and_then(|e| e.scene.as_ref());
        if !post.traces_lights()
            && !terrain_lights.is_some_and(|t| t.traces_lights())
            && !entity_lights.is_some_and(|t| t.traces_lights())
        {
            frame.ray_lights = None;
        }
        if let Some(r) = &mut frame.ray_lights {
            r.start(sources);
        }
        let mut labels: Vec<String> = sky.iter().map(|s| s.label.clone()).collect();
        labels.extend(
            others
                .iter()
                .filter(|s| match s.kind {
                    StageKind::Material => material.is_some(),
                    kind => kind == StageKind::Entity,
                })
                .map(|s| s.label.clone()),
        );
        labels.extend(weather_label);
        labels.extend(post.labels().into_iter().map(str::to_owned));
        let pack = Self {
            traces: report.manifest.ray_tracing.is_some(),
            looks: terrain.looks.is_some(),
            jitter: match &report.manifest.jitter {
                Jitter::Off => false,
                Jitter::On => true,
                Jitter::While(id) => option_on(constants, Some(id)),
            },
            kept,
            frame,
            sky,
            material,
            entity,
            post,
            vanilla_clouds,
            vanilla_sun,
            vanilla_moon,
            vanilla_stars,
            sun_path_rotation,
            weather,
            shadow,
            displacement,
            timing: None,
        };
        Ok((pack, labels))
    }

    /// Writes this frame's inputs, once a frame, and starts timing its passes. Returns
    /// whether its sky draws in a pass of its own: in a timed frame, so that it has a time of
    /// its own.
    pub fn start_frame(&mut self, start: FrameStart<'_>) -> bool {
        let FrameStart {
            device,
            queue,
            frame,
            size,
            projection,
            output,
            lights,
            light_revision,
            min_y,
            sky_visible,
            scene,
            lightmap,
            jitter,
        } = start;
        let scene = scene.and_then(|s| s.traced(frame.camera));
        self.frame.full_lightmap = super::native_post::full_lightmap(lightmap);
        self.frame.jitter = jitter;
        self.frame.lens = projection;
        self.write(queue, frame, size, output.display(), scene.as_ref());
        self.frame
            .block_light
            .update(queue, frame.camera, lights, light_revision);
        // Kept only while the passes trace: in a frame without the scene they miss anyway.
        if let (Some(r), Some(_)) = (&mut self.frame.ray_lights, &scene) {
            r.update(queue, frame.camera, (lights, min_y), light_revision);
        }
        let Some(t) = &mut self.timing else {
            return false;
        };
        t.begin(device, queue);
        self.sky.is_some() && t.recording() && sky_visible && frame.skybox
    }

    /// Binds the water mirror the renderer drew this frame, and outside the title screen
    /// fits the opaque copies, and the surface target (the material's or the entity stage's), to
    /// its targets.
    pub fn prepare_world_targets(&mut self, world: WorldTargets<'_>) {
        let WorldTargets {
            device,
            encoder,
            reflection,
            title,
            depth,
            color,
        } = world;
        let reflection = reflection.map(|view| self.frame.linear_mirror(device, encoder, view));
        self.frame.set_reflection(device, reflection.as_ref());
        if title {
            return;
        }
        if self.frame.fit_opaque(device, depth, color) {
            self.post.refresh_opaque(&self.frame);
        }
        let writes = self.material.as_ref().is_some_and(|m| m.writes_surface)
            || self.entity.as_ref().is_some_and(|e| e.writes_surface);
        let size = if writes {
            [depth.width(), depth.height()]
        } else {
            [1, 1]
        };
        self.frame.fit_surface(device, size);
    }

    /// The rain and snow through the pack's weather stage, in the world pass after translucent
    /// terrain and clouds. False when the pack has none, for the renderer's own.
    pub fn draw_weather(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        weather: &super::weather::Weather,
        entities: &wgpu::BindGroup,
    ) -> bool {
        let Some(pipeline) = &self.weather else {
            return false;
        };
        weather.draw_bound(pass, entities, |pass| {
            pass.set_pipeline(pipeline);
            pass.set_bind_group(horizon_shader_api::stages::GROUP, &self.frame.group, &[]);
        });
        true
    }

    /// The post stage in place of the renderer's own finish of the world: its passes over the
    /// world's scaled target, the last onto its picture of the window's size.
    pub fn run_post(&mut self, post: WorldPost<'_>) {
        let WorldPost {
            device,
            encoder,
            world: (color, depth),
            size,
            display,
            window,
            scene,
        } = post;
        let scene = scene.and_then(|(scene, camera)| scene.traced(camera));
        self.post.run_chain(
            device,
            encoder,
            (color, depth, &self.frame.surface_view),
            size,
            display,
            window,
            self.timing.as_mut(),
            scene.as_ref(),
        );
    }

    /// The post stage's output after `run_post`: its tonemap onto the screen.
    pub fn run_output(&mut self, output: WorldOutput<'_>) {
        let WorldOutput {
            device,
            encoder,
            screen,
            resolved,
        } = output;
        self.post
            .run_display(device, encoder, screen, resolved, self.timing.as_mut());
    }

    /// What the pack does in the renderer's place, fixed while it runs: the renderer keeps it
    /// from the pack's install on (`replace_executor`).
    pub fn capabilities(&self) -> Capabilities {
        let engine_resolve = !self.post.chain() && !self.frame.linear;
        Capabilities {
            shadow: self.shadow.is_some(),
            reflection: self.frame.reflects,
            // GPU culling doesn't list the sections `[displacement]`'s grid takes quads from.
            displacement: self.displacement.is_some(),
            material: self.material.is_some(),
            replaces_sky: self.sky.is_some(),
            hides_clouds: !self.vanilla_clouds,
            // `[sky]`'s sun, moon and stars are left out only where its stage draws its own.
            hides_sun: self.sky.is_some() && !self.vanilla_sun,
            hides_moon: self.sky.is_some() && !self.vanilla_moon,
            hides_stars: self.sky.is_some() && !self.vanilla_stars,
            tone_curve: self.post.tonemaps(),
            ray_scene: self.traces,
            entities: self.entity.is_some(),
            ray_entities: self.traces_entities(),
            ray_looks: self.traces_looks(),
            looks: self.looks,
            engine_resolve,
            engine_upscale: engine_resolve && self.post.upscalable(),
            // Where the renderer resolves the world, its TAA jitters it already.
            jitter: self.jitter && !engine_resolve,
        }
    }

    /// Times the pack's passes, or stops, as the renderer's settings say, before its world
    /// passes.
    pub fn set_up_timing(&mut self, setup: TimingSetup<'_>) {
        if self.timing.is_some() != setup.on {
            self.timing = setup
                .on
                .then(|| PassTiming::new(setup.device, setup.queue))
                .flatten();
        }
        if let Some(t) = &mut self.timing {
            t.gap = setup.gap;
            t.keep_frames(setup.keep_frames);
        }
    }

    /// The frame with the pack's timed passes was submitted.
    pub fn timing_submitted(&mut self, queue: &wgpu::Queue) {
        if let Some(timer) = &mut self.timing {
            timer.submitted(queue);
        }
    }
}

/// The turn a native pack's `[sky] sun_path_rotation` (`tilt`, degrees) gives the stars, sun and
/// moon the renderer draws (none under the End sky), as `PackSky::prepare` gives them for
/// `shaders/` packs.
pub(super) fn sky_turn(tilt: f32, end_sky: bool) -> Mat4 {
    super::pack_uniforms::tilt_matrix(if end_sky { 0.0 } else { tilt })
}

/// The bytes of the frame's first `count` sun and moon vertices, turned by [`sky_turn`].
pub(super) fn celestial_bytes<'a>(
    tilt: f32,
    frame: &super::Frame<'a>,
    count: usize,
) -> Cow<'a, [u8]> {
    turned(sky_turn(tilt, frame.end_sky), &frame.celestial[..count])
}

fn turned(turn: Mat4, bodies: &[Vertex]) -> Cow<'_, [u8]> {
    if turn == Mat4::IDENTITY {
        return Cow::Borrowed(bytemuck::cast_slice(bodies));
    }
    let bodies: Vec<Vertex> = bodies
        .iter()
        .map(|v| Vertex {
            pos: turn.transform_vector3(v.pos.into()).into(),
            ..*v
        })
        .collect();
    Cow::Owned(bytemuck::cast_slice(&bodies).to_vec())
}

#[cfg(test)]
pub(in crate::gfx) mod tests {
    use super::{
        Capabilities, NativePack, TerrainShape, chosen_shadow, reads_opaque, sky_turn, turned,
    };
    use horizon_shader_api::manifest::{OptionKind, PackOption, Shadow};

    /// The world before translucent terrain is copied only for a pack whose material, or the
    /// functions it calls, or its `displacement`, reads `opaque_depth` or `opaque_color` (#16124).
    #[test]
    fn the_opaque_copies_are_made_only_for_a_pack_that_reads_them() {
        let _gpu = crate::gfx::test_gpu::lock();
        use horizon_shader_api::{MANIFEST, Pack, check};
        let reads = |material: &str, displacement: &str| {
            let manifest = "schema = 1
id = \"t\"
name = \"T\"
version = \"1\"
                api = \"0.2\"
[material]
file = \"material.wgsl\"
[displacement]
";
            let text = format!(
                "fn material(input: MaterialInput) -> Surface {{
    {material}
}}
                fn displacement(input: DisplacementInput) -> f32 {{
    {displacement}
}}
                fn behind(t: vec2<i32>) -> vec4<f32> {{ return textureLoad(opaque_color, t, 0); }}
"
            );
            let files = [(MANIFEST, manifest), ("material.wgsl", text.as_str())]
                .map(|(n, t)| (n.to_owned(), t.as_bytes().to_vec()))
                .into();
            reads_opaque(
                &check(&Pack::from_files(files).unwrap()).unwrap().stages,
                false,
            )
        };
        let plain = "return Surface(input.albedo);";
        let flat = "return 0.0;";
        assert!(!reads(plain, flat), "a helper nothing calls reads nothing");
        assert!(reads(
            "return Surface(input.albedo + behind(vec2<i32>(input.screen)));",
            flat
        ));
        assert!(reads(
            "return Surface(input.albedo * textureLoad(opaque_depth, vec2<i32>(0), 0));",
            flat
        ));
        assert!(reads(
            plain,
            "return textureLoad(opaque_depth, vec2<i32>(input.uv), 0);"
        ));
        let post_reads = |body: &str| {
            let manifest = "schema = 1\nid = \"t\"\nname = \"T\"\nversion = \"1\"\napi = \"0.2\"\n\
                            post_opaque = true\n[[post]]\nfile = \"post.wgsl\"\n";
            let shader = format!(
                "fn behind() -> f32 {{ return textureLoad(opaque_depth, vec2<i32>(0), 0); }}\n\
                 fn post(input: PostInput) -> vec4<f32> {{ {body} }}"
            );
            let files = [(MANIFEST, manifest), ("post.wgsl", &shader)]
                .map(|(name, text)| (name.to_owned(), text.as_bytes().to_vec()))
                .into();
            reads_opaque(
                &check(&Pack::from_files(files).unwrap()).unwrap().stages,
                true,
            )
        };
        assert!(!post_reads("return input.color;"));
        assert!(post_reads("return vec4<f32>(behind());"));
    }

    /// Without a pack the sky is not turned and the sun and moon go up as they are; a turn
    /// moves each vertex and keeps the rest of it.
    #[test]
    fn the_sun_and_moon_turn_only_with_a_tilt() {
        use crate::mesher::Vertex;
        use std::borrow::Cow;
        assert_eq!(sky_turn(0.0, false), glam::Mat4::IDENTITY);
        let mut sun = <Vertex as bytemuck::Zeroable>::zeroed();
        sun.pos = [0.0, 1.0, 0.0];
        sun.color = 7;
        let bodies = [sun];
        assert!(matches!(
            turned(glam::Mat4::IDENTITY, &bodies),
            Cow::Borrowed(_)
        ));
        let tilted = turned(super::super::pack_uniforms::tilt_matrix(90.0), &bodies);
        let moved: Vec<Vertex> = bytemuck::pod_collect_to_vec(&tilted);
        assert!((moved[0].pos[1]).abs() < 1e-6 && (moved[0].pos[2].abs() - 1.0).abs() < 1e-6);
        assert_eq!(moved[0].color, 7);
    }

    /// A device with OpenGL's minimum 16 sampled textures per stage refuses a pack whose
    /// sky adds the terrain's 10 to the frame's 9: that is the pack's error to report, not
    /// a validation panic (#10095).
    #[test]
    fn a_pack_the_backend_refuses_is_an_error_not_a_panic() {
        let _gpu = crate::gfx::test_gpu::lock();
        let instance = crate::gfx::test_gpu::instance();
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
            return;
        };
        let mut limits = adapter.limits();
        limits.max_sampled_textures_per_shader_stage = 16;
        let Ok((device, queue)) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                required_limits: limits,
                ..Default::default()
            }))
        else {
            return;
        };
        let (globals, empty) = terrain_layouts(&device);
        let report = pack("");
        let built = NativePack::new(
            &device,
            &queue,
            &terrain_shape(&globals, &empty),
            report,
            &[],
            Default::default(),
        );
        assert!(built.is_err(), "the pack should be refused");
    }

    /// OpenGL can't translate a stage that loads from a depth texture (naga's GLSL writer has
    /// no `textureLoad` for them): a material that reads `opaque_depth`, and any post stage
    /// (its entry reads `scene_depth` through `depth_at`), refuse the pack instead of reaching
    /// the device's panicking handler (#18349).
    #[test]
    fn a_stage_opengl_cannot_translate_refuses_the_pack() {
        use horizon_shader_api::{MANIFEST, Pack, check};
        // OpenGL, which `test_gpu::instance` leaves out on Windows: WGL instances made on many
        // test threads at once deadlocked NVIDIA's driver (#5238). The lock keeps this one alone.
        // CI's `WGPU_BACKEND=dx12` keeps GPU tests off Mesa's software drivers (ci.yml); honour it.
        if wgpu::Backends::from_env().is_some_and(|b| !b.contains(wgpu::Backends::GL)) {
            eprintln!("skipped: WGPU_BACKEND leaves out OpenGL");
            return;
        }
        let _gpu = crate::gfx::test_gpu::lock();
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
        desc.backends = wgpu::Backends::GL;
        let instance = wgpu::Instance::new(desc);
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
            eprintln!("skipped: no OpenGL adapter");
            return;
        };
        let Ok((device, queue)) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                required_limits: adapter.limits(),
                ..Default::default()
            }))
        else {
            eprintln!("skipped: no OpenGL device");
            return;
        };
        eprintln!("OpenGL: {}", adapter.get_info().name);
        let (globals, empty) = terrain_layouts(&device);
        let sky = "fn sky(input: SkyInput) -> vec4<f32> { return input.color; }\n";
        let material = "fn material(input: MaterialInput) -> Surface {
    return Surface(input.albedo * textureLoad(opaque_depth, vec2<i32>(0), 0));
}\n";
        let post = "fn post(input: PostInput) -> vec4<f32> { return input.color; }\n";
        for (stages, file, text) in [
            ("[[post]]\nfile = \"post.wgsl\"\n", "post.wgsl", post),
            (
                "[material]\nfile = \"material.wgsl\"\n",
                "material.wgsl",
                material,
            ),
        ] {
            let manifest = format!(
                "schema = 1\nid = \"t\"\nname = \"T\"\nversion = \"1\"\napi = \"0.2\"\n\
                 [sky]\nfile = \"sky.wgsl\"\n{stages}"
            );
            let files = [
                (MANIFEST, manifest.as_str()),
                ("sky.wgsl", sky),
                (file, text),
            ]
            .map(|(n, t)| (n.to_owned(), t.as_bytes().to_vec()))
            .into();
            let report = check(&Pack::from_files(files).unwrap()).unwrap();
            let shape = terrain_shape(&globals, &empty);
            let built = NativePack::new(&device, &queue, &shape, report, &[], Default::default());
            let Err(refused) = built else {
                panic!("{file}: the pack should be refused");
            };
            let refused = refused.to_string();
            assert!(
                refused.contains("textureLoad"),
                "{file}: refused for another reason: {refused}"
            );
        }
    }

    /// What the renderer keeps of a pack from its install on: each answer as its manifest and
    /// the renderer's resources give it.
    #[test]
    fn a_pack_tells_the_renderer_what_it_draws_in_its_place() {
        let _gpu = crate::gfx::test_gpu::lock();
        let instance = crate::gfx::test_gpu::instance();
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
            return;
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .unwrap();
        let (globals, empty) = terrain_layouts(&device);
        let does = |stages: &str| {
            let shape = terrain_shape(&globals, &empty);
            let built = NativePack::new(
                &device,
                &queue,
                &shape,
                pack(stages),
                &[],
                Default::default(),
            );
            built.expect("the pack builds").0.capabilities()
        };
        let material = "[material]\nfile = \"material.wgsl\"\n";
        // Without post passes, a pack whose frame holds gamma light keeps the player's FXAA,
        // and TAA's upscaling under Horizon's tonemap.
        let plain = Capabilities {
            replaces_sky: true,
            engine_resolve: true,
            engine_upscale: true,
            ..Capabilities::default()
        };
        assert_eq!(does(""), plain);
        // FXAA's blend is made for gamma light, not the linear light of `blend = "linear"`.
        let linear = TerrainShape {
            linear: true,
            ..terrain_shape(&globals, &empty)
        };
        let linear_does = |stages: &str| {
            let built = NativePack::new(
                &device,
                &queue,
                &linear,
                pack(stages),
                &[],
                Default::default(),
            );
            built.expect("the pack builds").0.capabilities()
        };
        let sky = Capabilities {
            replaces_sky: true,
            ..Capabilities::default()
        };
        assert_eq!(linear_does(""), sky);
        // `[frame] jitter`: a pack that resolves its own world gets TAA's offset alone; where
        // the renderer resolves it, its TAA jitters already.
        let jitter = "[frame]\njitter = true\n";
        let jittered = Capabilities {
            jitter: true,
            ..sky
        };
        assert_eq!(linear_does(jitter), jittered);
        assert_eq!(linear_does("[frame]\njitter = false\n"), sky);
        assert_eq!(does(jitter), plain);
        let hides = Capabilities {
            hides_clouds: true,
            hides_sun: true,
            hides_moon: true,
            hides_stars: true,
            ..plain
        };
        let all = "clouds = false\nsun = false\nmoon = false\nstars = false\n";
        assert_eq!(does(all), hides);
        let terrain = Capabilities {
            material: true,
            ..plain
        };
        assert_eq!(does(material), terrain);
        // No resource pack's height maps here (`TerrainShape::pbr`): nothing to displace.
        assert_eq!(does(&format!("{material}[displacement]\n")), terrain);
        let traces = Capabilities {
            ray_scene: true,
            ..terrain
        };
        assert_eq!(does(&format!("{material}[ray_tracing]\n")), traces);
        let tonemap = Capabilities {
            tone_curve: true,
            ..plain
        };
        assert_eq!(does("[tonemap]\nfile = \"tonemap.wgsl\"\n"), tonemap);
    }

    /// `[sky] moon = false` reaches the renderer as `hides_moon`, and the celestial pass then
    /// draws the sun's quad without the moon's; the same sky without the key keeps both.
    #[test]
    fn a_pack_sky_without_minecraft_s_moon_leaves_it_out_of_the_celestial_pass() {
        use crate::mesher::{LOW, VERY_BRIGHT, Vertex};
        let _gpu = crate::gfx::test_gpu::lock();
        let instance = crate::gfx::test_gpu::instance();
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
            return;
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .unwrap();
        let (globals, empty) = terrain_layouts(&device);
        let quad = |tier: u8| {
            let mut v: Vertex = bytemuck::Zeroable::zeroed();
            v.layer = 2 | u32::from(tier) << 16;
            [v; 4]
        };
        let sun_and_moon = [quad(VERY_BRIGHT), quad(LOW)].concat();
        let drawn = |stages: &str| {
            let shape = terrain_shape(&globals, &empty);
            let built = NativePack::new(
                &device,
                &queue,
                &shape,
                pack(stages),
                &[],
                Default::default(),
            );
            let c = built.expect("the pack builds").0.capabilities();
            (
                c.hides_moon,
                super::super::celestial::bodies(&sun_and_moon, c.hides_sun, c.hides_moon),
            )
        };
        assert_eq!(drawn("moon = false\n"), (true, 0..6));
        assert_eq!(drawn(""), (false, 0..12));
    }

    /// A pack with a sky stage and the manifest's `stages` after it.
    fn pack(stages: &str) -> horizon_shader_api::Report {
        use horizon_shader_api::{MANIFEST, Pack, check};
        let manifest = format!(
            "schema = 1\nid = \"t\"\nname = \"T\"\nversion = \"1\"\napi = \"0.2\"\n\
             [sky]\nfile = \"sky.wgsl\"\n{stages}"
        );
        let mut files = vec![
            (MANIFEST, manifest.as_str()),
            (
                "sky.wgsl",
                "fn sky(input: SkyInput) -> vec4<f32> { return input.color; }\n",
            ),
        ];
        // `[displacement]` takes its function from the material's file.
        let material = format!(
            "fn material(input: MaterialInput) -> Surface {{ return Surface(input.albedo); }}\n{}",
            if stages.contains("[displacement]") {
                "fn displacement(input: DisplacementInput) -> f32 { return 0.0; }\n"
            } else {
                ""
            }
        );
        if stages.contains("[material]") {
            files.push(("material.wgsl", &material));
        }
        if stages.contains("[tonemap]") {
            files.push((
                "tonemap.wgsl",
                "fn tonemap(input: TonemapInput) -> vec3<f32> { return input.color; }\n",
            ));
        }
        let files = files
            .into_iter()
            .map(|(n, t)| (n.to_owned(), t.as_bytes().to_vec()))
            .collect();
        check(&Pack::from_files(files).unwrap()).unwrap()
    }

    /// The terrain's group 0 as the renderer makes it (`Renderer::new`), and an empty layout
    /// for the other groups.
    pub(in crate::gfx) fn terrain_layouts(
        device: &wgpu::Device,
    ) -> (wgpu::BindGroupLayout, wgpu::BindGroupLayout) {
        // Its uniform, the 10 sampled textures (_terrain.wgsl_'s 8 sprite arrays, the
        // lightmap and the emission thresholds) and two samplers.
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty,
            count: None,
        };
        let texture = |view_dimension| wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension,
            multisampled: false,
        };
        let sampler = wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering);
        let uniform = wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        };
        let array = texture(wgpu::TextureViewDimension::D2Array);
        let plain = texture(wgpu::TextureViewDimension::D2);
        let entries = [
            (0, uniform),
            (1, array),
            (2, sampler),
            (3, plain),
            (4, sampler),
        ]
        .into_iter()
        .chain((5..=11).map(|binding| (binding, array)))
        .chain([(12, plain)])
        .map(|(binding, ty)| entry(binding, ty))
        .collect::<Vec<_>>();
        let globals = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &entries,
        });
        let empty = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[],
        });
        (globals, empty)
    }

    pub(in crate::gfx) fn terrain_shape<'a>(
        globals: &'a wgpu::BindGroupLayout,
        empty: &'a wgpu::BindGroupLayout,
    ) -> TerrainShape<'a> {
        TerrainShape {
            globals,
            flat: None,
            sprites: [1.0, 2048.0],
            sprite_views: &[],
            format: wgpu::TextureFormat::Rgba8Unorm,
            linear: false,
            msaa: false,
            sprite_tags: &[],
            sprite_materials: &[],
            entities: empty,
            retained_entities: empty,
            weather_groups: [empty, empty],
            pbr: None,
            looks: None,
        }
    }

    /// A pack whose terrain takes the quads' looks (`TerrainShape::looks`): its material,
    /// shadow and `[displacement]` grid pipelines build with the looks' group, and its material
    /// and shadow terrain draw with a page's looks bound, all under wgpu's validation.
    #[test]
    fn a_pack_taking_the_looks_builds_and_draws_its_terrain_with_them() {
        use wgpu::util::DeviceExt;
        let _gpu = crate::gfx::test_gpu::lock();
        let Some(adapter) = crate::gfx::test_gpu::adapter() else {
            return;
        };
        // The adapter's own limits, as the game asks for: a sky and terrain take more sampled
        // textures than the defaults' 16.
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .unwrap();
        let flags = adapter.get_downlevel_capabilities().flags;
        if !flags.contains(wgpu::DownlevelFlags::VERTEX_STORAGE) {
            eprintln!("no vertex storage: skipped");
            return;
        }
        let (globals, empty) = terrain_layouts(&device);
        let looks = super::super::looks::layout(&device);
        let texture = |format, layers, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: 4,
                    height: 4,
                    depth_or_array_layers: layers,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let sampled = wgpu::TextureUsages::TEXTURE_BINDING;
        let rgba = wgpu::TextureFormat::Rgba8Unorm;
        let pbr = [texture(rgba, 2, sampled), texture(rgba, 2, sampled)];
        // The shadow pass's entity draw reads the entities' atlas.
        let entities = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &super::super::entity_binding_entries(),
        });
        let shape = TerrainShape {
            pbr: Some(&pbr),
            looks: Some(&looks),
            entities: &entities,
            ..terrain_shape(&globals, &empty)
        };
        let report = pack("[material]\nfile = \"material.wgsl\"\n[shadow]\n[displacement]\n");
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let (pack, _) =
            NativePack::new(&device, &queue, &shape, report, &[], Default::default()).unwrap();
        assert!(pack.looks);
        let layers = pack.terrain_layers().unwrap();
        let shadow = pack.shadow.as_ref().unwrap();
        assert!(layers.displaced.is_some() && shadow.displaced.is_some());
        // The terrain's group 0 (`terrain_layouts`) of blank resources.
        let array = texture(rgba, 2, sampled).create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let plain = texture(rgba, 1, sampled).create_view(&Default::default());
        let sampler = device.create_sampler(&Default::default());
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 16384,
            usage: wgpu::BufferUsages::UNIFORM,
            mapped_at_creation: false,
        });
        let resource = |binding: u32| match binding {
            0 => uniform.as_entire_binding(),
            2 | 4 => wgpu::BindingResource::Sampler(&sampler),
            3 | 12 => wgpu::BindingResource::TextureView(&plain),
            _ => wgpu::BindingResource::TextureView(&array),
        };
        let entries: Vec<_> = (0..=12)
            .map(|binding| wgpu::BindGroupEntry {
                binding,
                resource: resource(binding),
            })
            .collect();
        let terrain = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &globals,
            entries: &entries,
        });
        // A page's looks: one quad's, at its four vertices' slots.
        let look = crate::mesher::QuadLook::new(0x80_c0ff, 0.8, 2, [1.0, 0.8, 0.6, 1.0]);
        let page_looks = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&[look; 4]),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let looks_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &looks,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: page_looks.as_entire_binding(),
            }],
        });
        let corner = |x: f32, y: f32| {
            crate::mesher::TerrainVertex::new([x, y, -2.0], [x, y], 0, u32::MAX, 0xf0f0)
        };
        let vertices = [corner(0.0, 0.0), corner(1.0, 0.0), corner(1.0, 1.0)];
        let buffer = |contents: &[u8]| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents,
                usage: wgpu::BufferUsages::VERTEX,
            })
        };
        let vertices = buffer(bytemuck::cast_slice(&vertices));
        let origins = buffer(bytemuck::cast_slice(&[0.0f32, 0.0, 0.0, 1.0]));
        let attachment = wgpu::TextureUsages::RENDER_ATTACHMENT;
        let view = |t: wgpu::Texture| t.create_view(&Default::default());
        let color = view(texture(rgba, 1, attachment));
        let shadow_color = view(texture(
            super::super::native_post::INTERMEDIATE,
            1,
            attachment,
        ));
        let depth = view(texture(wgpu::TextureFormat::Depth32Float, 1, attachment));
        let mut encoder = device.create_command_encoder(&Default::default());
        // Each terrain pipeline in a pass of its targets, the page's looks bound.
        // The shadow's frame group, or the material's (`TerrainLayers::bind`).
        type Draw<'a> = (
            &'a wgpu::RenderPipeline,
            Option<&'a wgpu::TextureView>,
            Option<&'a wgpu::BindGroup>,
        );
        // The solid terrain's depth pre-pass, which `[material] coverage = engine` has, before
        // the solid entry that tests equal against it.
        let depth_pass = layers
            .depth
            .expect("the engine's coverage has a depth pre-pass");
        let draws: [Draw<'_>; 5] = [
            (depth_pass.solid, Some(&color), None),
            (layers.solid, Some(&color), None),
            (layers.translucent, Some(&color), None),
            (&shadow.solid, None, Some(&shadow.group)),
            (
                &shadow.translucent,
                Some(&shadow_color),
                Some(&shadow.group),
            ),
        ];
        for (pipeline, target, frame) in draws {
            let colour: Vec<_> = (target.into_iter())
                .map(|view| {
                    Some(wgpu::RenderPassColorAttachment {
                        view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: Default::default(),
                    })
                })
                .collect();
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: colour.as_slice(),
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth,
                    depth_ops: Some(Default::default()),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &terrain, &[]);
            pass.set_bind_group(super::super::looks::GROUP, &looks_group, &[]);
            match frame {
                Some(group) => pass.set_bind_group(horizon_shader_api::stages::GROUP, group, &[]),
                None => layers.bind(&mut pass),
            }
            pass.set_vertex_buffer(0, vertices.slice(..));
            pass.set_vertex_buffer(1, origins.slice(..));
            pass.draw(0..3, 0..1);
        }
        queue.submit([encoder.finish()]);
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        if let Some(e) = pollster::block_on(scope.pop()) {
            panic!("{e}");
        }
    }

    /// The material stage's own `MaterialInput` of a quad with a look, drawn and read back,
    /// against the ray scene's oracle of it (`rt::scene::looks_tests::material`, which its GPU test
    /// holds a hit to): a pack that shows three of the fields draws what a pack showing the
    /// oracle's values as constants draws, through the same wrapper and frame. The corners
    /// have different ambient occlusion values, so the expected values vary by sample point.
    #[test]
    fn the_material_s_looks_are_the_ray_oracle_s() {
        use wgpu::util::DeviceExt;
        let _gpu = crate::gfx::test_gpu::lock();
        let Some(adapter) = crate::gfx::test_gpu::adapter() else {
            return;
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .unwrap();
        if !(adapter.get_downlevel_capabilities().flags)
            .contains(wgpu::DownlevelFlags::VERTEX_STORAGE)
        {
            return;
        }
        let (globals, empty) = terrain_layouts(&device);
        let looks = super::super::looks::layout(&device);
        let texture = |format, layers, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: 4,
                    height: 4,
                    depth_or_array_layers: layers,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let rgba = wgpu::TextureFormat::Rgba8Unorm;
        let sampled = wgpu::TextureUsages::TEXTURE_BINDING;
        let shape = TerrainShape {
            looks: Some(&looks),
            ..terrain_shape(&globals, &empty)
        };
        // Group 0 with `view_proj` the identity (its first field), the rest blank.
        let array = texture(rgba, 2, sampled).create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let plain = texture(rgba, 1, sampled).create_view(&Default::default());
        let sampler = device.create_sampler(&Default::default());
        let mut words = vec![0.0f32; 4096];
        for i in 0..4 {
            words[i * 5] = 1.0;
        }
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&words),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let resource = |binding: u32| match binding {
            0 => uniform.as_entire_binding(),
            2 | 4 => wgpu::BindingResource::Sampler(&sampler),
            3 | 12 => wgpu::BindingResource::TextureView(&plain),
            _ => wgpu::BindingResource::TextureView(&array),
        };
        let entries: Vec<_> = (0..=12)
            .map(|binding| wgpu::BindGroupEntry {
                binding,
                resource: resource(binding),
            })
            .collect();
        let terrain = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &globals,
            entries: &entries,
        });
        // A tinted look with uneven corner AO, and the fallback to vertex colour and unit
        // shade and AO. Both draws use the light level in the same vertices.
        let looked = crate::mesher::QuadLook::new(0x80_c0ff, 0.8, 4, [1.0, 0.2, 0.3, 0.6]);
        let groups = [looked, crate::mesher::QuadLook::default()].map(|look| {
            let page_looks = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: bytemuck::cast_slice(&[look; 4]),
                usage: wgpu::BufferUsages::STORAGE,
            });
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &looks,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: page_looks.as_entire_binding(),
                }],
            });
            (look, group)
        });
        // One quad over the whole target, both its triangles (corners 0, 1, 2 and 0, 2, 3, the
        // corner `vertex_index & 3`), each corner its own colour (low byte R) and AO: a pixel
        // read back tells blending before decoding from after, and each corner's byte.
        let corner = |x: f32, y: f32, color: u32| {
            // Over opaque texels (bit 15): the blank sprites pass no alpha test.
            crate::mesher::TerrainVertex::new([x, y, 0.5], [0.0, 0.0], 0x8000, color, 0xf0f0)
        };
        let corners = [
            corner(-1.0, -1.0, 0x3f40_80c0),
            corner(1.0, -1.0, 0x3f20_4010),
            corner(1.0, 1.0, 0x3f10_f030),
            corner(-1.0, 1.0, 0x3fc0_2080),
        ];
        let buffer = |contents: &[u8], usage| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents,
                usage,
            })
        };
        let vertex = wgpu::BufferUsages::VERTEX;
        let vertices = buffer(bytemuck::cast_slice(&corners), vertex);
        let origins = buffer(bytemuck::cast_slice(&[0.0f32, 0.0, 0.0, 1.0]), vertex);
        let indices = buffer(
            bytemuck::cast_slice(&[0u16, 1, 2, 0, 2, 3]),
            wgpu::BufferUsages::INDEX,
        );
        // Pixels (column, row) off the diagonal, two on each triangle, and the corners' weights
        // at their centres.
        let pixels = [(0, 0), (1, 0), (3, 3), (3, 2)];
        let weights = |(i, j): (usize, usize)| {
            let (x, y) = ((i as f32 + 0.5) / 2.0 - 1.0, 1.0 - (j as f32 + 0.5) / 2.0);
            if y < x {
                let b2 = (y + 1.0) / 2.0;
                let b1 = (x + 1.0) / 2.0 - b2;
                [1.0 - b1 - b2, b1, b2, 0.0]
            } else {
                let b2 = (x + 1.0) / 2.0;
                let b3 = (y + 1.0) / 2.0 - b2;
                [1.0 - b2 - b3, 0.0, b2, b3]
            }
        };
        // What a pack showing `shown` draws over the quad with `looks`, at `pixels`.
        let draw = |shown: &str, looks: &wgpu::BindGroup| -> Vec<[u8; 4]> {
            let material = format!(
                "fn material(input: MaterialInput) -> Surface {{\n    \
                 let k = input.shaded_tint;\n    return Surface(vec4<f32>({shown}, 1.0));\n}}\n"
            );
            let manifest = "schema = 1\nid = \"t\"\nname = \"T\"\nversion = \"1\"\napi = \"0.2\"\n\
                            [material]\nfile = \"material.wgsl\"\n";
            let files = [
                (horizon_shader_api::MANIFEST, manifest),
                ("material.wgsl", &material),
            ]
            .map(|(n, t)| (n.to_owned(), t.as_bytes().to_vec()))
            .into();
            let pack = horizon_shader_api::Pack::from_files(files).unwrap();
            let report = horizon_shader_api::check(&pack).unwrap();
            let (pack, _) =
                NativePack::new(&device, &queue, &shape, report, &[], Default::default()).unwrap();
            let layers = pack.terrain_layers().unwrap();
            let target = texture(
                rgba,
                1,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            );
            let depth = texture(
                wgpu::TextureFormat::Depth32Float,
                1,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
            );
            let (view, depth) = (
                target.create_view(&Default::default()),
                depth.create_view(&Default::default()),
            );
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: Default::default(),
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &depth,
                        depth_ops: Some(Default::default()),
                        stencil_ops: None,
                    }),
                    ..Default::default()
                });
                pass.set_bind_group(0, &terrain, &[]);
                pass.set_bind_group(super::super::looks::GROUP, looks, &[]);
                layers.bind(&mut pass);
                pass.set_vertex_buffer(0, vertices.slice(..));
                pass.set_vertex_buffer(1, origins.slice(..));
                pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint16);
                // As the world pass draws it: the depth pre-pass, then the shading that tests
                // equal against it, which must still cover every pixel.
                let depth_pass = layers
                    .depth
                    .expect("the engine's coverage has a depth pre-pass");
                for pipeline in [depth_pass.solid, layers.solid] {
                    pass.set_pipeline(pipeline);
                    pass.draw_indexed(0..6, 0, 0..1);
                }
            }
            let buffer = crate::gfx::copy_frame(&device, &mut encoder, &target, [4, 4]);
            queue.submit([encoder.finish()]);
            buffer
                .slice(..)
                .map_async(wgpu::MapMode::Read, |r| r.unwrap());
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            let frame = buffer.slice(..).get_mapped_range().unwrap();
            let row = crate::gfx::padded_row(4) as usize;
            (pixels.iter())
                .map(|&(i, j)| frame[j * row + i * 4..][..4].try_into().unwrap())
                .collect()
        };
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        type Want = [[f32; 4]; 3];
        let fields: [(&str, fn(&Want) -> [f32; 3]); 2] = [
            ("input.tint.r, input.tint.g, input.shade", |w| {
                [w[0][0], w[0][1], w[0][3]]
            }),
            ("input.ao, input.emission, k.r", |w| {
                [w[1][3], w[2][0], w[1][0]]
            }),
        ];
        let material = crate::gfx::rt::scene::looks_tests::material;
        for (look, group) in &groups {
            for (shown, of) in fields {
                let got = draw(shown, group);
                for (k, &pixel) in pixels.iter().enumerate() {
                    let [a, b, c] = of(&material(&corners, *look, weights(pixel)));
                    let oracle = draw(&format!("{a:?}, {b:?}, {c:?}"), group)[k];
                    assert_eq!([got[k][3], oracle[3]], [255; 2], "uncovered {pixel:?}");
                    assert!(
                        got[k].iter().zip(oracle).all(|(g, o)| g.abs_diff(o) <= 1),
                        "{look:?}, {shown} at {pixel:?}: {:?}, the oracle's {oracle:?}",
                        got[k]
                    );
                }
            }
        }
        if let Some(e) = pollster::block_on(scope.pop()) {
            panic!("{e}");
        }
    }

    /// On a device with ray queries, an entity stage that traces is built a second time against
    /// the scene, beside the entities' own groups, with the entity layer's bindings where the
    /// pack names `RAY_ENTITY`; one that doesn't trace is built once, as before.
    #[test]
    fn an_entity_stage_that_traces_is_built_against_the_scene() {
        use horizon_shader_api::{MANIFEST, Pack, check};
        let ray_query = crate::gfx::rt::scene::entity_tests::ray_device;
        let Some((device, queue)) = ray_query() else {
            eprintln!("no GPU with ray queries: skipped");
            return;
        };
        let layout = |entries: &[wgpu::BindGroupLayoutEntry]| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: None,
                entries,
            })
        };
        let entities = layout(&super::super::native_entity::entity_entries());
        // The retained block entities' place, read by their vertex shader only.
        let retained = layout(&[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }]);
        let empty = layout(&[]);
        let sprites = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let views: Vec<_> = (0..8)
            .map(|_| {
                sprites.create_view(&wgpu::TextureViewDescriptor {
                    dimension: Some(wgpu::TextureViewDimension::D2Array),
                    ..Default::default()
                })
            })
            .collect();
        let terrain = TerrainShape {
            globals: &empty,
            flat: None,
            sprites: [8.0, 1.0],
            sprite_views: &views,
            format: wgpu::TextureFormat::Rgba16Float,
            linear: true,
            msaa: false,
            sprite_tags: &[],
            sprite_materials: &[],
            entities: &entities,
            retained_entities: &retained,
            weather_groups: [&empty, &empty],
            pbr: None,
            looks: None,
        };
        let build = |entity: &str| {
            let manifest = "schema = 1\nid = \"t\"\nname = \"T\"\nversion = \"1\"\napi = \"0.2\"\n\
                [entity]\nfile = \"entity.wgsl\"\n[ray_tracing]\n";
            let files = [(MANIFEST, manifest), ("entity.wgsl", entity)]
                .map(|(n, t)| (n.to_owned(), t.as_bytes().to_vec()))
                .into();
            let report = check(&Pack::from_files(files).unwrap()).unwrap();
            let lights = Default::default();
            NativePack::new(&device, &queue, &terrain, report, &[], lights)
                .unwrap()
                .0
        };
        let sun = "fn entity(input: EntityInput) -> Surface {\n    \
            let start = input.position + input.normal * 0.05;\n    \
            let shade = select(1.0, 0.4, trace_any_layers(start, frame.sun_direction, 64.0, LAYERS));\n    \
            return Surface(vec4<f32>(input.albedo.rgb * shade, input.albedo.a));\n}\n";
        let solid = build(&format!("const LAYERS: u32 = RAY_SOLID;\n{sun}"));
        let stage = solid.entity.as_ref().unwrap();
        assert!(stage.scene.as_ref().is_some_and(|s| !s.traces_entities()));
        assert!(!solid.traces_entities());
        let mobs = build(&format!(
            "const LAYERS: u32 = RAY_SOLID | RAY_ENTITY;\n{sun}"
        ));
        assert!(mobs.entity.as_ref().unwrap().scene.is_some());
        assert!(mobs.traces_entities());
        let looks = build(
            "fn entity(input: EntityInput) -> Surface {
                let hit = trace(input.position, input.normal, 16.0);
                return Surface(vec4<f32>(hit.tint.rgb * hit_normal_map(hit).b, 1.0));
            }",
        );
        assert!(looks.traces_looks());
        assert!(!solid.traces_looks());
        let quiet =
            build("fn entity(input: EntityInput) -> Surface { return Surface(input.albedo); }\n");
        assert!(quiet.entity.as_ref().unwrap().scene.is_none());
    }

    fn option(id: &str, kind: OptionKind) -> PackOption {
        PackOption {
            id: id.into(),
            name: id.into(),
            description: String::new(),
            kind,
        }
    }

    #[test]
    fn the_shadow_map_follows_its_options() {
        let _gpu = crate::gfx::test_gpu::lock();
        let options = [
            option("shadows", OptionKind::Bool { default: true }),
            option(
                "quality",
                OptionKind::Choice {
                    values: vec!["1024".into(), "4096".into()],
                    default: 0,
                },
            ),
            option(
                "reach",
                OptionKind::Int {
                    default: 96,
                    min: 32,
                    max: 512,
                },
            ),
        ];
        let shadow = Shadow {
            option: Some("shadows".into()),
            resolution_option: Some("quality".into()),
            distance_option: Some("reach".into()),
            ..Shadow::default()
        };
        let values = |on: f64, quality: f64, reach: f64| {
            [("shadows", on), ("quality", quality), ("reach", reach)].map(|(k, v)| (k.into(), v))
        };
        let size = |values: &[(String, f64)]| {
            chosen_shadow(&shadow, &options, values).map(|s| (s.resolution, s.distance))
        };
        assert_eq!(size(&values(1.0, 1.0, 200.0)), Some((4096, 200.0)));
        assert_eq!(size(&values(1.0, 0.0, 64.0)), Some((1024, 64.0)));
        assert_eq!(size(&values(0.0, 1.0, 200.0)), None);
        // Without the options' keys: the manifest's own numbers, always drawn.
        assert_eq!(
            chosen_shadow(&Shadow::default(), &options, &values(0.0, 1.0, 200.0))
                .map(|s| (s.resolution, s.distance)),
            Some((2048, 128.0))
        );
    }
}
