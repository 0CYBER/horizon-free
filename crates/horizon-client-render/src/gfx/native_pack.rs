//! One of Horizon's own shader packs while it runs (docs/shaders.md): the frame its stages
//! read, its sky, material, entity and post stages, and the timer of their passes.

use horizon_shader_api::check::CheckedStage;
use horizon_shader_api::{Report, StageKind};

use super::displace::Displacement;
use super::external::{Capabilities, FrameStart, TimingSetup, WorldPost, WorldTargets};
use super::native_entity::NativeEntity;
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
/// `opaque_color`): only the material stage has them, for `material` and, under
/// `[displacement]`, `displacement`, and whatever those call.
fn reads_opaque(stages: &[CheckedStage]) -> bool {
    let reads = |s: &CheckedStage, function: naga::Handle<naga::Function>| {
        s.module
            .global_variables
            .iter()
            .filter(|(_, g)| matches!(g.name.as_deref(), Some("opaque_depth" | "opaque_color")))
            .any(|(global, _)| !s.info[function][global].is_empty())
    };
    stages
        .iter()
        .filter(|s| s.kind == StageKind::Material)
        .any(|s| {
            s.module
                .functions
                .iter()
                .filter(|(_, f)| matches!(f.name.as_deref(), Some("material" | "displacement")))
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
    /// its passes and material stage trace this frame's `scene` from, once it is built.
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
        self.frame.ray_distance = post.or(terrain);
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

    /// Joins the frame's group with this frame's `scene` from `camera` for a material stage
    /// that traces, and with the kept resources it reads: after the frame's group changed for
    /// this frame and the kept resources' frame started (`before_world`), before the terrain
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
        self.bind_kept(device);
    }

    /// Whether the pack's rays read the scene's entity layer: its files name `RAY_ENTITY`, and
    /// its stages trace on this device.
    pub(in crate::gfx) fn traces_entities(&self) -> bool {
        let material = self.material.as_ref().and_then(|m| m.scene.as_ref());
        self.post.traces_entities() || material.is_some_and(|t| t.traces_entities())
    }

    /// The pack's entity stage and the group 3 it draws with this frame.
    pub(in crate::gfx) fn entities(&self) -> Option<(&NativeEntity, &wgpu::BindGroup)> {
        let entity = self.entity.as_ref()?;
        let kept = entity
            .kept
            .as_ref()
            .and_then(|k| k.group(self.post.parity()));
        Some((entity, kept.unwrap_or(&self.frame.group)))
    }

    /// The pipelines the pack's terrain draws with this frame, and its group 3.
    pub(in crate::gfx) fn terrain(&self) -> Option<(&MaterialPipelines, &wgpu::BindGroup)> {
        Some((self.material.as_ref()?).drawn(&self.frame.group, self.post.parity()))
    }

    /// The pack of the checked stages `report` with the option values `constants`, drawing
    /// onto the world's targets as `terrain` describes; the sky reads the terrain's group 0.
    /// Also returns its stages' labels, in the order they draw. Errors name the stage wgpu
    /// refused.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        terrain: &TerrainShape<'_>,
        report: Report,
        constants: &[(String, f64)],
        lights: super::block_light::LightTable,
    ) -> Result<(Self, Vec<String>), String> {
        // A backend may refuse a layout or pipeline the pack asks for (a stage's bindings
        // past its sampled-texture limit, say, as OpenGL's 16 was before #10095): the pack
        // is refused with that error instead of a validation panic in the renderer.
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let built = Self::build(device, queue, terrain, report, constants, lights);
        let refused = pollster::block_on(scope.pop());
        match (built, refused) {
            (Ok(built), None) => Ok(built),
            (Ok(_), Some(e)) => Err(e.to_string()),
            (Err(e), _) => Err(e),
        }
    }

    fn build(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        terrain: &TerrainShape<'_>,
        report: Report,
        constants: &[(String, f64)],
        lights: super::block_light::LightTable,
    ) -> Result<(Self, Vec<String>), String> {
        let (globals, format) = (terrain.globals, terrain.format);
        let kept = super::native_post::kept::Signature::of(&report, constants);
        let textures = super::native_post::pack_textures(device, queue, &report);
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
        let reach = report.manifest.kept.reach;
        frame.epochs = (kept.as_ref()).map(|_| super::native_post::kept::Epochs::new(reach));
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
        frame.reads_opaque = reads_opaque(&report.stages);
        let fog = report.material_fog;
        let coverage = report.manifest.material_coverage;
        let keeps = |key: fn(&horizon_shader_api::Stage) -> Option<bool>| {
            report.sky.as_ref().and_then(key).unwrap_or(true)
        };
        let vanilla_clouds = keeps(|s| s.clouds);
        let vanilla_sun = keeps(|s| s.sun);
        let vanilla_moon = keeps(|s| s.moon);
        let vanilla_stars = keeps(|s| s.stars);
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
        let entity = others
            .iter()
            .find(|s| s.kind == StageKind::Entity)
            .map(|stage| {
                let groups = [terrain.entities, terrain.retained_entities];
                let manifest = &report.manifest;
                let listed = manifest.entity.as_ref();
                let kept = StageReads::new(device, manifest, listed, &frame);
                let stage = (stage, kept);
                NativeEntity::new(device, format, groups, &frame, stage, constants, manifest)
            })
            .transpose()?;
        // The decode and the tonemap run without a chain too.
        let tonemap = others.iter().find(|s| s.kind == StageKind::Tonemap);
        // A `[ray_tracing]` pack's post passes as the check built them against the scene; its
        // compute passes keep theirs (`ComputePass::scene`). Whether any of them, compute and
        // before-world passes included, reads the entity layer.
        let entities = post.iter().any(CheckedStage::traces_entities);
        let scenes: Vec<(String, naga::Module)> = (post.iter_mut())
            .filter(|s| s.kind == StageKind::Post)
            .filter_map(|s| Some((s.label.clone(), s.scene.take()?)))
            .collect();
        let manifest = &report.manifest;
        let mut post = NativePost::new(device, post, manifest, constants, format, &frame, tonemap)?;
        // One scene for every stage, as `frame.has_ray_tracing` says: where the material stage
        // traces and the device refuses its pipelines, the post passes keep missing too.
        let traced = |m: &NativeMaterial| m.scene.is_some();
        let wanted = |s: &horizon_shader_api::check::CheckedStage| s.scene_source.is_some();
        let refused = others.iter().any(wanted)
            && super::rt::query::on(device)
            && !material.as_ref().is_some_and(traced);
        let computes = post.computes_trace();
        if (!scenes.is_empty() || computes) && !refused {
            let shape = super::native_post::trace::TraceShape::of(terrain, entities);
            if !post.trace_scene(device, scenes, constants, &shape, &frame.pbr)
                && let Some(m) = material.as_mut()
            {
                m.scene = None;
            }
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
            output,
            lights,
            light_revision,
            sky_visible,
            scene,
        } = start;
        let scene = scene.and_then(|s| s.traced(frame.camera));
        self.write(queue, frame, size, output.display(), scene.as_ref());
        self.frame
            .block_light
            .update(queue, frame.camera, lights, light_revision);
        let Some(t) = &mut self.timing else {
            return false;
        };
        t.begin(device, queue);
        self.sky.is_some() && t.recording() && sky_visible && frame.skybox
    }

    /// Binds the water mirror the renderer drew this frame, and outside the title screen
    /// fits the material stage's copies of the world and the surface target to its targets.
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
        if self.material.is_some() {
            self.frame.fit_opaque(device, depth, color);
        }
        let writes = self.material.as_ref().is_some_and(|m| m.writes_surface);
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
    /// world's scaled target, the last onto the screen.
    pub fn run_post(&mut self, post: WorldPost<'_>) {
        let WorldPost {
            device,
            encoder,
            world: (color, depth),
            screen,
            size,
            scene,
        } = post;
        let scene = scene.and_then(|(scene, camera)| scene.traced(camera));
        self.post.run(
            device,
            encoder,
            (color, depth, &self.frame.surface_view),
            screen,
            size,
            self.timing.as_mut(),
            scene.as_ref(),
        );
    }

    /// What the pack does in the renderer's place, fixed while it runs: the renderer keeps it
    /// from the pack's install on (`replace_executor`).
    pub fn capabilities(&self) -> Capabilities {
        Capabilities {
            shadow: self.shadow.is_some(),
            reflection: self.frame.reflects,
            // GPU culling doesn't list the sections `[displacement]`'s grid takes quads from.
            displacement: self.displacement.is_some(),
            material: self.material.is_some(),
            hides_clouds: !self.vanilla_clouds,
            // `[sky]`'s sun, moon and stars are left out only where its stage draws its own.
            hides_sun: self.sky.is_some() && !self.vanilla_sun,
            hides_moon: self.sky.is_some() && !self.vanilla_moon,
            hides_stars: self.sky.is_some() && !self.vanilla_stars,
            tone_curve: self.post.tonemaps(),
            ray_scene: self.traces,
            entities: self.entity.is_some(),
            ray_entities: self.traces_entities(),
            looks: self.looks,
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

#[cfg(test)]
pub(in crate::gfx) mod tests {
    use super::{Capabilities, NativePack, TerrainShape, chosen_shadow, reads_opaque};
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
            reads_opaque(&check(&Pack::from_files(files).unwrap()).unwrap().stages)
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
        assert_eq!(does(""), Capabilities::default());
        let hides = Capabilities {
            hides_clouds: true,
            hides_sun: true,
            hides_moon: true,
            hides_stars: true,
            ..Capabilities::default()
        };
        let all = "clouds = false\nsun = false\nmoon = false\nstars = false\n";
        assert_eq!(does(all), hides);
        let terrain = Capabilities {
            material: true,
            ..Capabilities::default()
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
            ..Capabilities::default()
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
        let draws: [Draw<'_>; 4] = [
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
