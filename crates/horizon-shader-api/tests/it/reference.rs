//! The stages' WGSL against the Rust definitions: the preludes compile, the frame's WGSL layout
//! is the uploaded struct's, the WGSL of a pack without `[ray_tracing]` is what it was before the
//! key, and docs/api/shaders-reference-0.2.md is up to date. Rewrite the reference with
//! `HORIZON_WRITE_DOCS=1 cargo test -p horizon-shader-api --test it reference::`.
use horizon_shader_api::{
    Frame, Pack, StageKind, compose, package::sha256_hex, reference, stages::Tracing,
};
use naga::valid::Capabilities;
use std::path::Path;

fn root() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn validate(text: &str) -> naga::Module {
    validate_for(text, Capabilities::default())
}

fn validate_for(text: &str, capabilities: Capabilities) -> naga::Module {
    let module = naga::front::wgsl::parse_str(text)
        .unwrap_or_else(|e| panic!("{}\n{text}", e.emit_to_string(text)));
    naga::valid::Validator::new(naga::valid::ValidationFlags::all(), capabilities)
        .validate(&module)
        .unwrap_or_else(|e| panic!("{}\n{text}", e.emit_to_string(text)));
    module
}

/// Vanilla's own entity colour (`entity_color` in the renderer's entity.wgsl), from the stage's
/// inputs: the reference prints the same function.
const VANILLA_ENTITY: &str = "\
fn entity(input: EntityInput) -> Surface {
    let base = input.albedo.rgb * input.tint.rgb * input.shade;
    // Vanilla lays the flash over the surface in its gamma (the 2.2 power), before the light.
    let gamma = vec3<f32>(1.0 / 2.2);
    let flashed = pow(mix(pow(base, gamma), pow(input.overlay.rgb, gamma), input.overlay.a), vec3<f32>(2.2));
    return Surface(vec4<f32>(flashed * input.lightmap, input.albedo.a * input.tint.a));
}
";

#[test]
fn the_reference_prints_vanilla_s_entity_function_as_it_is_checked() {
    let doc = reference::markdown();
    assert!(doc.contains(&format!("```wgsl\n{VANILLA_ENTITY}```")));
}

/// Vanilla's look, written as each stage.
fn vanilla(kind: StageKind) -> &'static str {
    match kind {
        StageKind::Sky => "fn sky(input: SkyInput) -> vec4<f32> { return input.color; }\n",
        StageKind::Material => {
            "fn material(input: MaterialInput) -> Surface {\n    \
             return Surface(input.albedo * input.tint * vec4<f32>(input.lightmap, 1.0));\n}\n"
        }
        StageKind::Vertex => {
            "fn vertex(input: VertexInput) -> vec3<f32> { return input.position; }\n"
        }
        StageKind::Weather => {
            "fn weather(input: WeatherInput) -> vec4<f32> { return input.color; }\n"
        }
        StageKind::Post => "fn post(input: PostInput) -> vec4<f32> { return input.color; }\n",
        StageKind::Tonemap => {
            "fn tonemap(input: TonemapInput) -> vec3<f32> { return input.color; }\n"
        }
        StageKind::Entity => VANILLA_ENTITY,
        StageKind::Compute | StageKind::BeforeWorld => {
            "fn compute(input: ComputeInput) {}
"
        }
    }
}

#[test]
fn every_stage_compiles_with_vanilla_s_function() {
    for kind in StageKind::ALL {
        let text = format!("{}{}{}", kind.prelude(), vanilla(kind), kind.entries());
        let module = validate(&text);
        assert!(
            module
                .entry_points
                .iter()
                .all(|e| e.name.starts_with("horizon_")),
            "{kind:?}"
        );
    }
}

/// A pack with `[ray_tracing]`: every stage without the scene, on any device, and with it, on
/// one with ray queries.
#[test]
fn every_stage_compiles_under_ray_tracing() {
    let traces = "fn post(input: PostInput) -> vec4<f32> {\n    \
                  let hit = trace(view_position(input.uv, input.depth), frame.sun_direction, frame.ray_distance);\n    \
                  let maps = hit_normal_map(hit) * hit_specular_map(hit) * f32(block_pbr_maps()) + hit.tangent;\n    \
                  let water = trace_layers(hit.position, frame.sun_direction, 8.0, RAY_SOLID | RAY_TRANSLUCENT);\n    \
                  let wet = f32(trace_any_layers(hit.position, hit.normal, 8.0, RAY_TRANSLUCENT)) * water.albedo.a;\n    \
                  let own = maps * hit.emission * f32(hit.fluid) * f32(hit.translucent) + wet;\n    \
                  return select(input.color, hit.albedo * hit.tint + own, hit.distance >= 0.0 && frame.has_ray_tracing == 1u);\n}\n";
    let shaded = "fn material(input: MaterialInput) -> Surface {\n    \
                  let lit = f32(!trace_any(input.position, frame.sun_direction, frame.ray_distance))\n        \
                      * hit_specular_map(trace(input.position, input.normal, 8.0)).r;\n    \
                  return Surface(input.albedo * input.tint * vec4<f32>(input.lightmap * lit, 1.0));\n}\n";
    for kind in StageKind::ALL {
        let own = match kind {
            StageKind::Post => traces,
            StageKind::Material => shaded,
            _ => vanilla(kind),
        };
        let stub = kind.prelude_traced(Tracing::Stub);
        validate(&format!("{stub}{own}{}", kind.entries()));
        let scene = kind.prelude_traced(Tracing::Scene);
        let text = format!("{scene}{own}{}", kind.entries());
        validate_for(&text, Capabilities::default() | Capabilities::RAY_QUERY);
        // Only the post and material stages' modules need the device's ray queries.
        let rays = matches!(kind, StageKind::Post | StageKind::Material);
        assert_eq!(scene.contains("ray_query"), rays, "{kind:?}");
        assert!(!stub.contains("ray_query") && !stub.contains("acceleration_structure"));
    }
}

/// The SHA-256 of the material stage's prelude for a `[ray_tracing]` pack whose material
/// calls a trace function, without the scene and with it.
const PINNED_TRACED_MATERIAL: [(Tracing, &str); 2] = [
    (
        Tracing::Stub,
        "ed31d1755a1383dc0d6e1dfc70d9d3831180f4baf4094cd3e2ad7c7407651311",
    ),
    (
        Tracing::Scene,
        "9ec02b2d7ad21f591dbbbb5facdf0b802726b9d36bdda0b31d9d87083948f2ff",
    ),
];

#[test]
fn the_traced_material_prelude_is_unchanged() {
    for (tracing, prelude) in PINNED_TRACED_MATERIAL {
        let have = StageKind::Material.prelude_traced(tracing);
        assert_eq!(sha256_hex(have.as_bytes()), prelude, "{tracing:?}");
    }
}

/// The SHA-256 of the prelude and entries of each stage for a pack without `[ray_tracing]`, as
/// they were before the key: a pack that doesn't ask for the scene is built from the same WGSL.
const PINNED_0_2: [(StageKind, &str, &str); 6] = [
    (
        StageKind::Sky,
        "13320fd37a8b7de9011742b88709c820d7f89b2709e35313bd8e667c27eaf233",
        "54bac4d416d804d8a96e1e31a6d62460f77573634c1e7bec16670641355d678a",
    ),
    (
        StageKind::Vertex,
        "43afb61a66484dc1afe032322b9c35648ec5d93e4ea7b453e8ac1c5bb4c1d54c",
        "f30235a35a2ec15f502efcea364a4470227325fcef36328b5cd04528a916a8ba",
    ),
    (
        StageKind::Material,
        "44421906ba871e4bde6a5e89e659a376dd97e2b4debb82c8fae56fba280b3be3",
        "6716c28ae0e699d15695da89c0848ae9d242c0e146b8487aff463ccc19bff4d2",
    ),
    (
        StageKind::Weather,
        "6055a97e6f8df727508899bfd332374179a51f85e3377581b4b867b7cbef1aa2",
        "c2e4dabacd487692038c3fbf5708aed72a12d951c15184ceaf61ee1bb61b4982",
    ),
    (
        StageKind::Post,
        "2c9d8e0d342e5314ca4216780e94d26f0e5ea7eb8b779acd7c995ccb80cfe32a",
        "f1ffc3c9e0692e1d1f49de4a34ae90b93f7f2eda2eaa42ce936f630139cc4648",
    ),
    (
        StageKind::Tonemap,
        "1261f4b8f021ed0cea25333e87d8277043f226ac1ab5b3387390bb18fbf3144d",
        "2dcf7ee0874daeb033058ed55323b74c0da5de4a170cece2e6192500714c0b6a",
    ),
];

#[test]
fn the_wgsl_of_a_pack_without_ray_tracing_is_unchanged() {
    for (kind, prelude, entries) in PINNED_0_2 {
        let have = (
            sha256_hex(kind.prelude().as_bytes()),
            sha256_hex(kind.entries().as_bytes()),
        );
        assert_eq!(have, (prelude.to_owned(), entries.to_owned()), "{kind:?}");
    }
    // And a whole stage of a pack in the tree, as the game composes it.
    let dir = root().join("crates/horizon-shader-api/packs/golden_hour");
    let pack = Pack::read(&dir).unwrap();
    let glow = compose(&pack, StageKind::Post, "post/glow.wgsl");
    assert_eq!(
        // As the repository holds the files: a Windows checkout may give them CRLF.
        sha256_hex(glow.text.replace('\r', "").as_bytes()),
        "6cd2dbe4b62501369aaa01695da01dd973695f1e1ee7daea5da9dfcca25d1715"
    );
}

/// The `Frame` struct a prelude declares, for a pack with `[ray_tracing]` or without, with
/// `[[resource]]` or without, with `[frame] fog = "pack"` or without, and whose files name
/// `moon_phase` or not, as (name, offset) and its span.
fn frame_layout(
    ray_tracing: bool,
    kept: bool,
    fog: bool,
    moon_phase: bool,
) -> (Vec<(String, usize)>, usize) {
    let module = validate(&Frame::declared_wgsl(ray_tracing, kept, fog, moon_phase));
    let (_, ty) = module
        .types
        .iter()
        .find(|(_, t)| t.name.as_deref() == Some("Frame"))
        .expect("Frame in the prelude");
    let naga::TypeInner::Struct { members, span } = &ty.inner else {
        panic!("Frame is a struct")
    };
    let members = members
        .iter()
        .map(|m| (m.name.clone().unwrap(), m.offset as usize))
        .collect();
    (members, *span as usize)
}

#[test]
fn the_frame_s_wgsl_layout_is_the_uploaded_struct_s() {
    let rust: Vec<_> = Frame::WGSL
        .fields
        .iter()
        .zip(Frame::OFFSETS)
        .map(|(f, &o)| (f.name.to_owned(), o))
        .collect();
    let size = std::mem::size_of::<Frame>();
    // Every field a pack may declare sits where the uploaded struct has it, whichever of the
    // keys bringing fields it sets: a field it doesn't declare is a gap, or the end.
    for ray_tracing in [false, true] {
        for kept in [false, true] {
            for fog in [false, true] {
                for moon_phase in [false, true] {
                    let mut want = rust.clone();
                    let left = |range: std::ops::Range<usize>, on: bool| (!on).then_some(range);
                    for range in [
                        left(Frame::MOON_FIELDS, moon_phase),
                        left(Frame::FOG_FIELDS, fog),
                        left(Frame::KEPT_FIELDS, kept),
                        left(Frame::RAY_FIELDS, ray_tracing),
                    ]
                    .into_iter()
                    .flatten()
                    {
                        want.drain(range);
                    }
                    let keys = format!(
                        "ray tracing {ray_tracing}, kept {kept}, fog {fog}, moon phase {moon_phase}"
                    );
                    let (wgsl, span) = frame_layout(ray_tracing, kept, fog, moon_phase);
                    assert_eq!(wgsl, want, "{keys}");
                    // Without the fog's fields and the moon's phase the struct ends before them,
                    // a prefix of the buffer; without the epochs either, before those.
                    let end = match (kept, fog || moon_phase) {
                        (_, true) => size,
                        (true, false) => size - 16,
                        (false, false) => size - 32,
                    };
                    assert_eq!(span, end, "{keys}");
                }
            }
        }
    }
}

#[test]
fn the_sample_pack_s_stages_compile() {
    let dir = root().join("crates/horizon-shader-api/tests/samples/schema-1");
    let pack = Pack::read(&dir).unwrap();
    let m = &pack.manifest;
    let stages = [
        (StageKind::Sky, m.sky.iter().collect::<Vec<_>>()),
        (StageKind::Material, m.material.iter().collect()),
        (StageKind::Post, m.post.iter().collect()),
    ];
    for (kind, list) in stages {
        for stage in list {
            let composed = compose(&pack, kind, &stage.file);
            validate(&composed.text);
            // Errors point into the pack's own file.
            let part = composed.parts.iter().find(|(f, _)| *f == stage.file);
            let part = part.unwrap().1.clone();
            let needle = format!("fn {}(", kind.function());
            let at = part.start + composed.text[part].find(&needle).unwrap();
            let source = pack.source(&stage.file).unwrap();
            let line = source.lines().position(|l| l.starts_with(&needle)).unwrap() + 1;
            assert_eq!(composed.locate(at), (stage.file.as_str(), line, 1));
        }
    }
}

#[test]
fn the_reference_is_up_to_date() {
    let doc = reference::PATH;
    let path = root().join(doc);
    let want = reference::markdown();
    if std::env::var_os("HORIZON_WRITE_DOCS").is_some() {
        std::fs::write(&path, &want).unwrap();
    }
    let have = std::fs::read_to_string(&path)
        .unwrap_or_default()
        .replace("\r\n", "\n");
    assert!(
        have == want,
        "{doc} is out of date: HORIZON_WRITE_DOCS=1 cargo test -p horizon-shader-api --test it reference::"
    );
}
