use super::{EntityKind, EntityRuns, kind_bits, shader_source, surfaced};
use crate::mesher::Vertex;

/// Vanilla's own as a pack writes it (docs/api/shaders-reference-0.2.md): the flash over the
/// surface in vanilla's gamma, then the light.
pub const VANILLA: &str = "
fn entity(input: EntityInput) -> Surface {
    let base = input.albedo.rgb * input.tint.rgb * input.shade;
    let flashed = pow(mix(pow(base, vec3(1.0 / 2.2)), pow(input.overlay.rgb, vec3(1.0 / 2.2)),
        input.overlay.a), vec3(2.2));
    let light = min(input.lightmap + input.colored_light, vec3(1.0));
    return Surface(vec4(flashed * light, input.albedo.a * input.tint.a));
}
";

/// A pack's entity shader whose stage is `entity`.
pub fn shader(entity: &str) -> String {
    shader_with(entity, "")
}

/// `shader` for a manifest that ends with `more`.
fn shader_with(entity: &str, more: &str) -> String {
    use horizon_shader_api::{MANIFEST, Pack, check};
    let manifest = "schema = 1\nid = \"t\"\nname = \"T\"\nversion = \"1\"\napi = \"0.2\"\n\
        [entity]\nfile = \"entity.wgsl\"\n"
        .to_owned()
        + more;
    let files = [(MANIFEST, &*manifest), ("entity.wgsl", entity)]
        .map(|(n, t)| (n.to_owned(), t.as_bytes().to_vec()))
        .into();
    let report = check(&Pack::from_files(files).unwrap()).unwrap();
    let stage = &report.stages[0];
    shader_source(&stage.source, report.manifest.fog, stage.light_source())
}

/// A stage that fogs its own surfaces, as vanilla would, under `[frame] fog = "pack"`.
const OWN_FOG: &str = "[frame]\nfog = \"pack\"\n";

/// Every name a pack's code could take that Horizon's entity shaders or the entries around
/// the stage have had, in any case: a pack's own are checked only against lowercase `horizon_`.
const OWN_NAMES: &str = "const globals: f32 = 1.0;
fn entity_color(c: f32) -> f32 { return c; }
struct Out { a: f32 }
struct HorizonEntityOut { a: f32 }
const HORIZON_ENTITY_KIND_SHIFT: u32 = 1u;
const HORIZON_ENTITY_BLOCK_ENTITY: u32 = 2u;
";

/// The module-level names of `module`: its types, constants, overrides, variables, functions
/// and entry points.
fn names(module: &naga::Module) -> std::collections::BTreeSet<String> {
    let types = module.types.iter().filter_map(|(_, t)| t.name.clone());
    let constants = module.constants.iter().filter_map(|(_, c)| c.name.clone());
    let overrides = module.overrides.iter().filter_map(|(_, o)| o.name.clone());
    let variables = module
        .global_variables
        .iter()
        .filter_map(|(_, v)| v.name.clone());
    let functions = module.functions.iter().filter_map(|(_, f)| f.name.clone());
    let entries = module.entry_points.iter().map(|e| e.name.clone());
    types
        .chain(constants)
        .chain(overrides)
        .chain(variables)
        .chain(functions)
        .chain(entries)
        .collect()
}

/// What the renderer adds to a pack's stage is all in the names a pack's files may not
/// declare (`horizon_`, lowercase: horizon-shader-api's check), so no pack that passes the
/// check can meet a duplicate declaration here.
#[test]
fn the_renderers_names_are_reserved() {
    let parse = |source: &str| naga::front::wgsl::parse_str(source).unwrap();
    let source = shader(VANILLA);
    let stage = names(&parse(
        &source[..source.find("// Horizon's entities").unwrap()],
    ));
    let added: Vec<_> = names(&parse(&source))
        .into_iter()
        .filter(|name| !stage.contains(name) && !name.starts_with("horizon_"))
        .collect();
    assert_eq!(added, [] as [&str; 0]);
}

/// The shader the renderer builds around a pack's stage is valid WGSL with both of its
/// pipelines' entries, whatever names the pack's own code takes: Horizon's are prefixed.
#[test]
fn the_packs_entity_shader_validates() {
    let entities = [VANILLA.to_owned(), OWN_NAMES.to_owned() + VANILLA];
    let surface = "surface = true\n";
    let fogs = [("", false), (OWN_FOG, false), (surface, true)];
    for (entity, (more, surfaces)) in entities.iter().flat_map(|e| fogs.map(|f| (e, f))) {
        let source = shader_with(entity, more);
        let source = if surfaces { surfaced(&source) } else { source };
        let module = naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
        let entries: Vec<_> = module.entry_points.iter().map(|e| &*e.name).collect();
        for entry in [
            "horizon_entity_vs",
            "horizon_entity_vs_static",
            "horizon_entity_fs",
        ] {
            assert!(entries.contains(&entry), "{entry} in {entries:?}");
        }
    }
}

/// What Horizon writes around the stage of a pack that leaves fog to it, after its prefixed
/// entity.wgsl (which changes with the engine): the SHA-256 it had before `[frame] fog`, so
/// such a pack's entity shader is the same WGSL. Under the key only the glint's fade differs.
#[test]
fn the_entries_around_a_stage_horizon_fogs_are_unchanged() {
    use sha2::{Digest, Sha256};
    let entries = |more| {
        let source = shader_with(VANILLA, more).replace('\r', "");
        let at = source.find("const horizon_entity_kind_shift").unwrap();
        source[at..].to_owned()
    };
    let engine = entries("");
    assert_eq!(
        format!("{:x}", Sha256::digest(&engine)),
        "ebd9010e458b0a778bb0ba6852736233788906a4f7890e5160cc287375f51c5a"
    );
    assert!(!engine.contains("vanilla_fog"));
    let own = entries(OWN_FOG);
    assert!(own.contains("    let f = vanilla_fog(position);\n"));
    assert!(!own.contains("horizon_vanilla_globals.fog"));
}

/// A pack whose files name `light_source_tier` reads, in `EntityInput`, the tier entity.wgsl's
/// look reads (its `tier`, from the quad's layer), for every tier a quad carries, through the
/// stage's own vertex (`horizon_entity_vertex`) and fragment (`horizon_entity_fs`).
#[test]
fn a_pack_reads_the_light_source_tier_the_engine_s_look_reads() {
    let _gpu = crate::gfx::test_gpu::lock();
    let instance = crate::gfx::test_gpu::instance();
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
        return;
    };
    let Ok((device, queue)) = pollster::block_on(adapter.request_device(&Default::default()))
    else {
        return;
    };
    let entity = "fn entity(input: EntityInput) -> Surface {\n    \
        return Surface(vec4<f32>(f32(input.light_source_tier) * 0.125, 0.0, 0.0, 1.0));\n}\n";
    let tiers = [0u32, 1, 2, 3, 4];
    let n = tiers.len();
    let list: Vec<String> = tiers.iter().map(|t| format!("{t}u")).collect();
    // One point a tier, at its own texel, lit as a mob's quad.
    let source = shader(entity)
        + &format!(
            "
@vertex fn test_vs(@builtin(vertex_index) v: u32) -> horizon_entity_out {{
    var tiers = array<u32, {n}>({list});
    let layer = (tiers[v] << 16u) | {kind}u;
    var out = horizon_entity_vertex(vec3(0.0, 0.0, -2.0), vec2(0.5), layer, vec4(1.0),
        vec4(240u), vec4(0.0, 1.0, 0.0, 1.0), vec2(0.0), (layer >> horizon_entity_kind_shift) & 7u);
    out.clip = vec4((f32(v) + 0.5) / 8.0 - 1.0, 0.0, 0.5, 1.0);
    return out;
}}
@fragment fn test_engine(in: horizon_entity_out) -> @location(0) vec4<f32> {{
    return vec4(f32(horizon_entity_own(in).tier) * 0.125, 0.0, 0.0, 1.0);
}}
",
            list = list.join(", "),
            kind = kind_bits(EntityKind::Entity),
        );
    let entities = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &super::entity_entries(),
    });
    let frame = super::super::native_post::NativeFrame::new(
        &device,
        None,
        Vec::new(),
        Default::default(),
        None,
        false,
    );
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: 4,
            height: 4,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    let sampler = device.create_sampler(&Default::default());
    let uniform = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 16384,
        usage: wgpu::BufferUsages::UNIFORM,
        mapped_at_creation: false,
    });
    let entries: Vec<_> = (0..=7)
        .map(|binding| wgpu::BindGroupEntry {
            binding,
            resource: match binding {
                0 => uniform.as_entire_binding(),
                2 | 4 | 6 => wgpu::BindingResource::Sampler(&sampler),
                _ => wgpu::BindingResource::TextureView(&view),
            },
        })
        .collect();
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &entities,
        entries: &entries,
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[Some(&entities), None, None, Some(&frame.layout)],
        ..Default::default()
    });
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: None,
        source: wgpu::ShaderSource::Wgsl(source.as_str().into()),
    });
    // The stage's alpha decides (`[entity] coverage = "pack"`): the blank atlas has none.
    let constants = [
        ("horizon_vanilla_HDR_WRITE_LIGHT", 0.0),
        ("horizon_vanilla_FRAME_LINEAR", 0.0),
        ("horizon_entity_coverage_pack", 1.0),
    ];
    let format = wgpu::TextureFormat::Rgba32Float;
    let drawn = |fragment: &str| -> Vec<[f32; 4]> {
        let options = wgpu::PipelineCompilationOptions {
            constants: &constants,
            ..Default::default()
        };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("test_vs"),
                compilation_options: options.clone(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::PointList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some(fragment),
                compilation_options: options,
                targets: &[Some(format.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        // 16 texels of 16 bytes: the 256 bytes a copy's rows align to.
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 16,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());
        let read = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 256,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: -1.0,
                        g: 0.0,
                        b: 0.0,
                        a: 0.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.set_bind_group(3, &frame.group, &[]);
        pass.draw(0..n as u32, 0..1);
        drop(pass);
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &read,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: None,
                },
            },
            target.size(),
        );
        queue.submit([encoder.finish()]);
        read.slice(..)
            .map_async(wgpu::MapMode::Read, |r| r.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        bytemuck::pod_collect_to_vec(&read.slice(..).get_mapped_range().unwrap())
    };
    // The stage's colour comes out encoded for a frame of vanilla's gamma. A drawn pixel's alpha
    // is 1; the clear's is 0, so a pixel the stage skipped can't pass as tier 0.
    let pack: Vec<f32> = drawn("horizon_entity_fs")[..n]
        .iter()
        .map(|t| {
            assert_eq!(t[3], 1.0, "the pack drew every pixel");
            (t[0].max(0.0).powf(2.2) / 0.125).round()
        })
        .collect();
    let engine: Vec<f32> = (drawn("test_engine")[..n].iter())
        .map(|t| (t[0] / 0.125).round())
        .collect();
    let want = tiers.map(|t| t as f32);
    assert_eq!(engine, want, "the engine's tiers");
    assert_eq!(pack, want, "the pack's tiers");
}

/// The kind is written above every bit the entity shaders read of a layer (the overlay, the
/// HDR tier, `OIT_ENTITY`), so a quad drawn by Horizon's own pipelines reads as before.
#[test]
fn a_kind_keeps_clear_of_the_layers_other_bits() {
    let kinds = [EntityKind::Entity, EntityKind::BlockEntity];
    for kind in kinds {
        let bits = kind_bits(kind);
        assert_ne!(bits, 0);
        assert_eq!(bits & (super::super::entity_batch::OIT_ENTITY * 2 - 1), 0);
        assert_eq!((bits >> super::KIND_SHIFT) & 7, kind as u32 + 1);
    }
}

fn quad(layer: u32) -> [Vertex; 4] {
    [Vertex {
        layer,
        ..bytemuck::Zeroable::zeroed()
    }; 4]
}

/// The kinds follow the game's runs through the sorted batch: an ordinary quad inside a run
/// takes its kind; one before, between or after the runs takes none, whatever was drawn
/// before it (an arrow after the last mob, a dragon after everything), and neither do the
/// other materials' quads, in a run or not (a translucent item in a mob's hand). The frame
/// after has none unless the game names its runs again.
#[test]
fn quads_take_the_kind_of_their_run() {
    // Quads 0, 1, 3, 4 and 5 are ordinary; quad 2 is another material's (overlay 32, a
    // translucent item's), sorted out of them though it lies in the entity's run.
    let vertices: Vec<Vertex> = [0, 5, 32, 0, 7, 0].into_iter().flat_map(quad).collect();
    let mut batch = super::super::entity_batch::EntityBatch::default();
    batch.kinds.push(4..12, EntityKind::Entity);
    batch.kinds.push(12..16, EntityKind::BlockEntity);
    batch.kinds.push(20..24, EntityKind::Entity);
    let kinds = |batch: &mut super::super::entity_batch::EntityBatch| -> Vec<u32> {
        batch.update(&vertices, &[], &[], &[], false);
        let quads = batch.combined.chunks(4);
        quads.map(|q| q[3].layer >> super::KIND_SHIFT & 7).collect()
    };
    // The ordinary quads lead the batch: 0, 1, 3, 4 and 5, then quad 2. Quad 4 follows the
    // block entity's run and is in none.
    assert_eq!(kinds(&mut batch), [0, 1, 2, 0, 1, 0]);
    assert_eq!(batch.combined[4].layer & 0xffff, 5);
    assert_eq!(batch.combined[20].layer, 32);
    assert_eq!(kinds(&mut batch), [0; 6]);
}

/// The ray scene's entity layer takes the quads the runs name, of the ordinary material and of
/// improved transparency's, wherever the batch sorts them, and nothing else: not a particle
/// drawn between two mobs, nor an additive quad, nor another material's quad in a run. Its two
/// ranges are where the batch put each material.
#[test]
fn the_entity_layer_s_quads_are_the_runs_through_the_sorting() {
    use super::super::entity_batch::{EntityBatch, OIT_ENTITY};
    use horizon_shader_api::stages::RAY_ENTITY_QUAD;
    // A mob (quads 0 and 1, one of improved transparency's), a particle (2), a translucent
    // item the mob holds (3), a block entity (4, improved transparency's), a particle (5).
    let layers = [0, OIT_ENTITY, 0, 32, OIT_ENTITY, 0];
    let vertices: Vec<Vertex> = layers.into_iter().flat_map(quad).collect();
    let additive: Vec<Vertex> = quad(2 | OIT_ENTITY).into();
    let mut batch = EntityBatch::default();
    batch.kinds.push(0..8, EntityKind::Entity);
    batch.kinds.push(12..20, EntityKind::Entity);
    batch.kinds.push(16..20, EntityKind::BlockEntity);
    batch.update(&vertices, &additive, &[], &[], true);
    // Sorted: the ordinary quads 0, 2, 5, the item 3, improved transparency's 1, 4 and the
    // additive quad it took.
    let marked: Vec<bool> = (batch.combined.chunks(4))
        .map(|q| q.iter().all(|v| v.layer & RAY_ENTITY_QUAD != 0))
        .collect();
    assert_eq!(marked, [true, false, false, false, true, true, false]);
    assert_eq!(batch.combined[12].layer & 0xffff, 32);
    assert_eq!(batch.ray_ranges, [(0, 3), (4, 3), (7, 0)]);
    // The bit keeps clear of the kind and of every bit the entity shaders read.
    let kind = batch.combined[20].layer >> super::KIND_SHIFT & 7;
    assert_eq!(kind, EntityKind::BlockEntity as u32 + 1);
    assert_eq!(
        RAY_ENTITY_QUAD & (7 << super::KIND_SHIFT | (OIT_ENTITY * 2 - 1)),
        0
    );
    // Without runs nothing is marked.
    batch.update(&vertices, &additive, &[], &[], true);
    assert!(
        batch
            .combined
            .iter()
            .all(|v| v.layer & RAY_ENTITY_QUAD == 0)
    );
}

/// A run has its end, and one named where geometry was cut back replaces what was named
/// there: no vertex is ever in a run its caller didn't name it in.
#[test]
fn runs_end_and_give_way_where_the_geometry_was_cut_back() {
    let bits = |runs: &EntityRuns| -> Vec<u32> {
        let quads = runs.bits((0..6).map(|quad| quad * 4));
        quads.map(|bits| bits >> super::KIND_SHIFT).collect()
    };
    let mut runs = EntityRuns::default();
    // Nothing drawn names nothing.
    runs.push(0..0, EntityKind::Entity);
    assert_eq!(runs, EntityRuns::default());
    runs.push(4..8, EntityKind::Entity);
    runs.push(8..12, EntityKind::Entity);
    runs.push(12..20, EntityKind::BlockEntity);
    assert_eq!(bits(&runs), [0, 1, 1, 2, 2, 0]);
    // The geometry was cut back to vertex 8 and a block entity drawn there: the entity's
    // run ends at it, and the old run from 12 is gone with its vertices.
    runs.push(8..12, EntityKind::BlockEntity);
    assert_eq!(bits(&runs), [0, 1, 2, 0, 0, 0]);
    // Cut back to before every run, with nothing named after.
    runs.push(2..2, EntityKind::Entity);
    assert_eq!(bits(&runs), [0; 6]);
}

/// Under `[entity] surface = true` an entity's point is `present` to the post stage's
/// `surface_at`: the depth its fragment writes to the surface target is the one the depth
/// test kept within `surface_at`'s tolerance, through the entity pipeline's depth bias, at
/// near and far depths alike. A pixel no entity covers keeps the clear.
#[test]
fn an_entity_s_surface_is_present_where_its_depth_is() {
    let _gpu = crate::gfx::test_gpu::lock();
    let instance = crate::gfx::test_gpu::instance();
    // Every adapter: whether the fragment's depth holds the bias is the driver's.
    for adapter in pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all())) {
        let name = format!(
            "{} ({:?})",
            adapter.get_info().name,
            adapter.get_info().backend
        );
        let Ok((device, queue)) = pollster::block_on(adapter.request_device(&Default::default()))
        else {
            continue;
        };
        let entity = "fn entity(input: EntityInput) -> Surface {\n    \
        return Surface(vec4<f32>(1.0, 0.0, 0.0, 1.0));\n}\n";
        // Reversed depth: 1 is the near plane.
        let depths = [0.9f32, 0.5, 0.1, 0.01, 1e-3, 1e-4, 1e-5];
        let n = depths.len();
        let list: Vec<String> = depths.iter().map(|d| format!("{d:e}")).collect();
        // A flat triangle a depth that covers only its own texel's centre (64 texels a row).
        let source = surfaced(&shader_with(entity, "surface = true\n"))
            + &format!(
                "
@vertex fn test_vs(@builtin(vertex_index) v: u32) -> horizon_entity_out {{
    var depths = array<f32, {n}>({list});
    let i = v / 3u;
    let corner = v % 3u;
    let layer = {kind}u;
    var out = horizon_entity_vertex(vec3(0.0, 0.0, -2.0), vec2(0.5), layer, vec4(1.0),
        vec4(240u), vec4(0.0, 1.0, 0.0, 1.0), vec2(0.0), (layer >> horizon_entity_kind_shift) & 7u);
    let w = 2.0 / 64.0;
    let left = f32(i) * w - 1.0;
    let x = select(left, left + 1.6 * w, corner == 1u);
    let y = select(-1.0, 3.0, corner == 2u);
    out.clip = vec4(x, y, depths[i], 1.0);
    out.proj = vec4(f32(corner == 1u), f32(corner == 2u), 0.0, 1.0);
    return out;
}}
",
                list = list.join(", "),
                kind = kind_bits(EntityKind::Entity),
            );
        let entities = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &super::entity_entries(),
        });
        let frame = super::super::native_post::NativeFrame::new(
            &device,
            None,
            Vec::new(),
            Default::default(),
            None,
            false,
        );
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 4,
                height: 4,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let sampler = device.create_sampler(&Default::default());
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 16384,
            usage: wgpu::BufferUsages::UNIFORM,
            mapped_at_creation: false,
        });
        let entries: Vec<_> = (0..=7)
            .map(|binding| wgpu::BindGroupEntry {
                binding,
                resource: match binding {
                    0 => uniform.as_entire_binding(),
                    2 | 4 | 6 => wgpu::BindingResource::Sampler(&sampler),
                    _ => wgpu::BindingResource::TextureView(&view),
                },
            })
            .collect();
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &entities,
            entries: &entries,
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&entities), None, None, Some(&frame.layout)],
            ..Default::default()
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(source.as_str().into()),
        });
        let constants = [
            ("horizon_vanilla_HDR_WRITE_LIGHT", 0.0),
            ("horizon_vanilla_FRAME_LINEAR", 0.0),
            ("horizon_entity_coverage_pack", 1.0),
        ];
        let options = wgpu::PipelineCompilationOptions {
            constants: &constants,
            ..Default::default()
        };
        let surface_format = super::super::native_material::SURFACE_FORMAT;
        // The entity pipeline's (`NativeEntity::pipelines`): its depth test and its bias.
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("test_vs"),
                compilation_options: options.clone(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil: Default::default(),
                bias: wgpu::DepthBiasState {
                    constant: 128,
                    ..Default::default()
                },
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("horizon_entity_fs"),
                compilation_options: options,
                targets: &[
                    Some(wgpu::TextureFormat::Rgba8Unorm.into()),
                    Some(surface_format.into()),
                ],
            }),
            multiview_mask: None,
            cache: None,
        });
        let target = |format, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: 64,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: usage | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        };
        let attach = wgpu::TextureUsages::RENDER_ATTACHMENT;
        let color = target(wgpu::TextureFormat::Rgba8Unorm, attach);
        let surface = target(surface_format, attach);
        let depth = target(wgpu::TextureFormat::Depth32Float, attach);
        let views = [&color, &surface, &depth].map(|t| t.create_view(&Default::default()));
        // 64 texels of 16 bytes (the surface), then 64 of 4 (the depth).
        let read = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 1024 + 256,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        let clear = |view| {
            Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[clear(&views[0]), clear(&views[1])],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &views[2],
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.set_bind_group(3, &frame.group, &[]);
        pass.draw(0..3 * n as u32, 0..1);
        drop(pass);
        let all = wgpu::TextureAspect::All;
        let depth_only = wgpu::TextureAspect::DepthOnly;
        for (texture, aspect, offset, bytes_per_row) in
            [(&surface, all, 0, 1024), (&depth, depth_only, 1024, 256)]
        {
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    aspect,
                    ..texture.as_image_copy()
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &read,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset,
                        bytes_per_row: Some(bytes_per_row),
                        rows_per_image: None,
                    },
                },
                texture.size(),
            );
        }
        queue.submit([encoder.finish()]);
        read.slice(..)
            .map_async(wgpu::MapMode::Read, |r| r.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let bytes = read.slice(..).get_mapped_range().unwrap().to_vec();
        let surfaces: Vec<[f32; 4]> = bytemuck::pod_collect_to_vec(&bytes[..1024]);
        let kept: Vec<f32> = bytemuck::pod_collect_to_vec(&bytes[1024..]);
        for (i, want) in depths.iter().enumerate() {
            let (w, depth) = (surfaces[i][3], kept[i]);
            assert!(
                depth >= *want,
                "texel {i}: the bias brings the entity nearer"
            );
            // `surface_at`'s `present` (horizon-shader-api's post prelude).
            assert!(
                w > 0.0 && (w - depth).abs() <= w * 2e-5,
                "{name}, texel {i}: the surface's depth {w:e} for the kept {depth:e} (drawn at {want:e})"
            );
        }
        assert_eq!(surfaces[n], [0.0; 4], "{name}: a texel no entity covers");
        eprintln!("{name}: every depth present");
    }
}
