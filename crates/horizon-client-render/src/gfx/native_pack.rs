//! One of Horizon's own shader packs while it runs (docs/shaders.md): the frame its stages
//! read, its sky, material and post stages, and the timer of their passes.

use horizon_shader_api::check::CheckedStage;
use horizon_shader_api::{Report, StageKind};

use super::displace::Displacement;
use super::native_material::{DisplacedShape, NativeMaterial, TerrainShape};
use super::native_post::{NativeFrame, NativePost};
use super::native_shadow::{NativeShadow, ShadowGrid};
use super::native_sky::NativeSky;
use super::pack_timing::PassTiming;

pub struct NativePack {
    pub frame: NativeFrame,
    pub sky: Option<NativeSky>,
    pub material: Option<NativeMaterial>,
    pub post: Option<NativePost>,
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
        let textures = super::native_post::pack_textures(device, queue, &report);
        // `[shadow]`, unless its option is off, at the size its options pick.
        let chosen = report
            .shadow
            .as_ref()
            .and_then(|s| chosen_shadow(s, &report.manifest.options, constants));
        let settings = chosen
            .as_ref()
            .map(|settings| super::native_shadow::fitted(device, settings));
        let api = report.manifest.api;
        let mut frame = NativeFrame::new(
            device,
            settings.as_ref(),
            textures,
            lights,
            terrain.pbr,
            (api, terrain.linear && api == horizon_shader_api::Api::V0_2),
        );
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
        let (post, others): (Vec<_>, Vec<_>) = report
            .stages
            .into_iter()
            .partition(|s| s.kind == StageKind::Post);
        let sky = others
            .iter()
            .find(|s| s.kind == StageKind::Sky)
            .map(|stage| NativeSky::new(device, globals, &frame, stage, constants, format))
            .transpose()?;
        let material = others
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
                    has_vertex,
                    displaced.as_ref(),
                )
            })
            .transpose()?;
        // A 0.2 pack always has one: its decode and tonemap run without a chain too.
        let tonemap = others.iter().find(|s| s.kind == StageKind::Tonemap);
        let post = (!post.is_empty() || api == horizon_shader_api::Api::V0_2)
            .then(|| {
                let manifest = &report.manifest;
                NativePost::new(device, post, manifest, constants, format, &frame, tonemap)
            })
            .transpose()?;
        let mut labels: Vec<String> = sky.iter().map(|s| s.label.clone()).collect();
        labels.extend(
            others
                .iter()
                .filter(|s| material.is_some() && s.kind == StageKind::Material)
                .map(|s| s.label.clone()),
        );
        labels.extend(weather_label);
        if let Some(post) = &post {
            labels.extend(post.labels().into_iter().map(str::to_owned));
        }
        let pack = Self {
            particle_lights: report.manifest.particle_lights.clone(),
            light_colors: report.manifest.lights.clone(),
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

    /// Whether the pack draws anything the game runs today.
    pub fn draws(&self) -> bool {
        self.sky.is_some()
            || self.material.is_some()
            || self.weather.is_some()
            || self.post.is_some()
    }

    /// Whether a stage reads the arena's solid quads as `TerrainVertex`es, so they upload
    /// wide (`Renderer::follow_packed_layout`): `[displacement]`'s copy of the quads. The
    /// material's and the shadow pass's solid entries unpack compact records as the engine's
    /// own pipelines do (`native_material::SECTION_CORNER`).
    pub fn reads_terrain_vertices(&self) -> bool {
        self.displacement.is_some()
    }
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
        let report = pack("0.1", "");
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

    /// A 0.2 pack whose stages read no terrain vertices (a sky, the post every 0.2 pack runs, a
    /// material and a shadow pass, whose solid entries read the flat draws' page) leaves the
    /// solid quads in the compact layout; one with `[displacement]`, which copies the quads'
    /// vertices, does not (#16127).
    #[test]
    fn only_a_pack_with_displacement_reads_terrain_vertices() {
        let instance = crate::gfx::test_gpu::instance();
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
            return;
        };
        let Ok((device, queue)) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                required_limits: adapter.limits(),
                ..Default::default()
            }))
        else {
            return;
        };
        let (globals, empty) = terrain_layouts(&device);
        // The flat draws' group 1 (`Flat::new`): block slots, origins and the page.
        let read = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let flat = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[read(0), read(1), read(super::super::flat::ARENA_BINDING)],
        });
        // Height maps, without which there is no `[displacement]`.
        let pbr = [0; 2].map(|_| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: 16,
                    height: 16,
                    depth_or_array_layers: 2,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
        });
        // The entities' atlas, which the shadow pass's entity draw samples.
        let atlas = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty,
            count: None,
        };
        let entities = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                atlas(
                    1,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
                atlas(
                    2,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                ),
            ],
        });
        let reads = |stages: &str| {
            let shape = TerrainShape {
                flat: Some(&flat),
                pbr: Some(&pbr),
                entities: &entities,
                ..terrain_shape(&globals, &empty)
            };
            let report = pack("0.2", stages);
            let (pack, _) =
                NativePack::new(&device, &queue, &shape, report, &[], Default::default()).unwrap();
            assert!(pack.post.is_some() && pack.sky.is_some());
            pack.reads_terrain_vertices()
        };
        let material = "[material]\nfile = \"material.wgsl\"\n";
        assert!(!reads(""));
        assert!(!reads(&format!("{material}[shadow]\n")));
        assert!(reads(&format!("{material}[displacement]\n")));
    }

    /// A pack of API `api` with a sky stage and the manifest's `stages` after it.
    fn pack(api: &str, stages: &str) -> horizon_shader_api::Report {
        use horizon_shader_api::{MANIFEST, Pack, check};
        let manifest = format!(
            "schema = 1\nid = \"t\"\nname = \"T\"\nversion = \"1\"\napi = \"{api}\"\n\
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
