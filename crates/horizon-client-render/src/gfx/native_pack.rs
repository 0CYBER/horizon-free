//! One of Horizon's own shader packs while it runs (docs/shaders.md): the frame its stages
//! read, its sky, material and post stages, and the timer of their passes.

use horizon_shader_api::check::CheckedStage;
use horizon_shader_api::{Report, StageKind};

use super::displace::Displacement;
use super::native_material::{DisplacedShape, MaterialPipelines, NativeMaterial, TerrainShape};
use super::native_post::{NativeFrame, NativePost};
use super::native_shadow::{NativeShadow, ShadowGrid};
use super::native_sky::NativeSky;
use super::output::EffectiveOutput;
use super::pack_timing::PassTiming;
use super::surface_setup::ray_query::ray_scene::RayScene;
use super::{Frame, SectionKey};
use crate::fast_hash::FxHashMap;

pub struct NativePack {
    pub frame: NativeFrame,
    pub sky: Option<NativeSky>,
    pub material: Option<NativeMaterial>,
    /// The post chain (none without `[[post]]`), the decode before it and the tonemap after.
    pub post: NativePost,
    /// With a sky stage, what of Minecraft's sky still draws over it (`[sky]`).
    pub vanilla_clouds: bool,
    pub vanilla_sun: bool,
    pub vanilla_stars: bool,
    pub weather: Option<wgpu::RenderPipeline>,
    /// The draw of the shadow map the stages read, if the pack asks for one.
    pub shadow: Option<NativeShadow>,
    /// `[displacement]`'s grid, while it is on and the resource packs have height maps.
    pub displacement: Option<Displacement>,
    /// `[[particle_lights]]`.
    pub particle_lights: Vec<horizon_shader_api::manifest::ParticleLight>,
    /// `[[lights]]`, for the colour of held and dropped items' light.
    pub light_colors: Vec<horizon_shader_api::manifest::LightColor>,
    /// GPU time of each pass, while the renderer asks for it (`pack_timing.rs`).
    pub timing: Option<PassTiming>,
    /// It declares `[ray_tracing]`: the renderer keeps a scene for it (`replace_native`).
    pub traces: bool,
    /// With kept resources, what lets a rebuild keep them (`carry`).
    kept: Option<super::native_post::kept::Signature>,
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

impl NativePack {
    /// Writes this frame's inputs (`NativeFrame::write`), and for a `[ray_tracing]` pack where
    /// its post passes and material stage trace `scene` from, once it is built.
    pub(in crate::gfx) fn write(
        &mut self,
        queue: &wgpu::Queue,
        frame: &super::Frame,
        size: [u32; 2],
        display: super::output::DisplayValues,
        scene: Option<&RayScene>,
    ) {
        let scene = scene.and_then(|s| s.traced(frame.camera));
        let post = self.post.trace(queue, scene.as_ref());
        let material = self.material.as_ref();
        let terrain = material.and_then(|m| m.trace(queue, scene.as_ref()));
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

    /// Joins the frame's group with this frame's `scene` from `camera` for a material stage
    /// that traces: after the frame's group changed for this frame, before the terrain draws.
    pub(in crate::gfx) fn bind_scene(
        &mut self,
        device: &wgpu::Device,
        scene: Option<&RayScene>,
        camera: glam::DVec3,
    ) {
        if let Some(trace) = self.material.as_mut().and_then(|m| m.scene.as_mut()) {
            let scene = scene.and_then(|s| s.traced(camera));
            trace.bind(device, &self.frame, scene.as_ref());
        }
    }

    /// The pipelines the pack's terrain draws with this frame, and its group 3.
    pub(in crate::gfx) fn terrain(&self) -> Option<(&MaterialPipelines, &wgpu::BindGroup)> {
        Some(self.material.as_ref()?.drawn(&self.frame.group))
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
        let coverage = report.manifest.coverage;
        let keeps = |key: fn(&horizon_shader_api::Stage) -> Option<bool>| {
            report.sky.as_ref().and_then(key).unwrap_or(true)
        };
        let vanilla_clouds = keeps(|s| s.clouds);
        let vanilla_sun = keeps(|s| s.sun);
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
        let (mut post, others): (Vec<_>, Vec<_>) = report
            .stages
            .into_iter()
            .partition(|s| s.kind == StageKind::Post);
        let sky = others
            .iter()
            .find(|s| s.kind == StageKind::Sky)
            .map(|stage| NativeSky::new(device, globals, &frame, stage, constants, format))
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
                )
            })
            .transpose()?;
        // The decode and the tonemap run without a chain too.
        let tonemap = others.iter().find(|s| s.kind == StageKind::Tonemap);
        // A `[ray_tracing]` pack's passes as the check built them against the scene.
        let scenes: Vec<(String, naga::Module)> = post
            .iter_mut()
            .filter_map(|s| Some((s.label.clone(), s.scene.take()?)))
            .collect();
        let manifest = &report.manifest;
        let mut post = NativePost::new(device, post, manifest, constants, format, &frame, tonemap)?;
        // One scene for every stage, as `frame.has_ray_tracing` says: where the material stage
        // traces and the device refuses its pipelines, the post passes keep missing too.
        let traced = |m: &NativeMaterial| m.scene.is_some();
        let wanted = |s: &horizon_shader_api::check::CheckedStage| s.scene_source.is_some();
        let refused = others.iter().any(wanted)
            && super::surface_setup::ray_query::on(device)
            && !material.as_ref().is_some_and(traced);
        if !scenes.is_empty() && !refused {
            let shape = super::native_post::trace::TraceShape::of(terrain);
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
                .filter(|s| material.is_some() && s.kind == StageKind::Material)
                .map(|s| s.label.clone()),
        );
        labels.extend(weather_label);
        labels.extend(post.labels().into_iter().map(str::to_owned));
        let pack = Self {
            particle_lights: report.manifest.particle_lights.clone(),
            light_colors: report.manifest.lights.clone(),
            traces: report.manifest.ray_tracing.is_some(),
            kept,
            frame,
            sky,
            material,
            post,
            vanilla_clouds,
            vanilla_sun,
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
        self.write(queue, frame, size, output.display(), scene);
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
}

/// What the renderer hands the pack before its world passes (`NativePack::start_frame`).
pub struct FrameStart<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub frame: &'a Frame<'a>,
    /// The world's size this frame: the scaled target's, else the window's.
    pub size: [u32; 2],
    /// This frame's output, whose display values a 0.2 pack reads.
    pub output: &'a EffectiveOutput,
    /// The light sources the section meshes list, and their revision (`BlockLight::update`).
    pub lights: &'a FxHashMap<SectionKey, Vec<([i32; 3], u32)>>,
    pub light_revision: u64,
    /// The sky is open this frame.
    pub sky_visible: bool,
    /// The ray scene, when the game traces rays (`NativePack::write`).
    pub scene: Option<&'a RayScene>,
}

/// What the renderer hands the pack after the water mirror (`NativePack::prepare_world_targets`).
pub struct WorldTargets<'a> {
    pub device: &'a wgpu::Device,
    pub encoder: &'a mut wgpu::CommandEncoder,
    /// The water mirror's colour, when one was drawn this frame.
    pub reflection: Option<wgpu::TextureView>,
    pub title: bool,
    /// The world's depth, and its colour where it can be copied (the scaled target, not the
    /// window's own).
    pub depth: &'a wgpu::Texture,
    pub color: Option<&'a wgpu::Texture>,
}

#[cfg(test)]
mod tests {
    use super::{NativePack, TerrainShape, chosen_shadow, reads_opaque};
    use horizon_shader_api::manifest::{OptionKind, PackOption, Shadow};

    /// The world before translucent terrain is copied only for a pack whose material, or the
    /// functions it calls, or its `displacement`, reads `opaque_depth` or `opaque_color` (#16124).
    #[test]
    fn the_opaque_copies_are_made_only_for_a_pack_that_reads_them() {
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
        let files = files
            .into_iter()
            .map(|(n, t)| (n.to_owned(), t.as_bytes().to_vec()))
            .collect();
        check(&Pack::from_files(files).unwrap()).unwrap()
    }

    /// The terrain's group 0 as the renderer makes it (`Renderer::new`), and an empty layout
    /// for the other groups.
    fn terrain_layouts(device: &wgpu::Device) -> (wgpu::BindGroupLayout, wgpu::BindGroupLayout) {
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

    fn terrain_shape<'a>(
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
            weather_groups: [empty, empty],
            pbr: None,
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
