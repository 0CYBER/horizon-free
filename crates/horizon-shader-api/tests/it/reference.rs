//! The stages' WGSL against the Rust definitions: the preludes of both lines compile, the
//! frame's WGSL layout is the uploaded struct's, 0.1's WGSL is what it always was, 0.2's is
//! what it was before `[ray_tracing]` for a pack without the key, and
//! docs/api/shaders-reference.md and shaders-reference-0.2.md are up to date. Rewrite the
//! references with `HORIZON_WRITE_DOCS=1 cargo test -p horizon-shader-api --test it reference::`.
use horizon_shader_api::{
    Api, Frame, Pack, StageKind, compose, package::sha256_hex, reference, stages::Tracing,
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
    let doc = reference::markdown(Api::V0_2);
    assert!(doc.contains(&format!("```wgsl\n{VANILLA_ENTITY}```")));
    assert!(!reference::markdown(Api::V0_1).contains("EntityInput"));
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
    for api in Api::SERVED {
        for &kind in api.stages() {
            let text = format!(
                "{}{}{}",
                kind.prelude(api),
                vanilla(kind),
                kind.entries(api)
            );
            let module = validate(&text);
            assert!(
                module
                    .entry_points
                    .iter()
                    .all(|e| e.name.starts_with("horizon_")),
                "{api} {kind:?}"
            );
        }
    }
    assert_eq!(Api::V0_1.stages().len(), 5);
    assert!(!Api::V0_1.stages().contains(&StageKind::Tonemap));
    assert!(!Api::V0_1.stages().contains(&StageKind::Entity));
}

/// A pack with `[ray_tracing]`: every stage without the scene, on any device, and with it, on
/// one with ray queries.
#[test]
fn every_0_2_stage_compiles_under_ray_tracing() {
    let api = Api::V0_2;
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
    for &kind in api.stages() {
        let own = match kind {
            StageKind::Post => traces,
            StageKind::Material => shaded,
            _ => vanilla(kind),
        };
        let stub = kind.prelude_traced(api, Tracing::Stub);
        validate(&format!("{stub}{own}{}", kind.entries(api)));
        let scene = kind.prelude_traced(api, Tracing::Scene);
        let text = format!("{scene}{own}{}", kind.entries(api));
        validate_for(&text, Capabilities::default() | Capabilities::RAY_QUERY);
        // Only the post and material stages' modules need the device's ray queries.
        let rays = matches!(kind, StageKind::Post | StageKind::Material);
        assert_eq!(scene.contains("ray_query"), rays, "{kind:?}");
        assert!(!stub.contains("ray_query") && !stub.contains("acceleration_structure"));
    }
}

/// The SHA-256 of the material stage's 0.2 prelude for a `[ray_tracing]` pack whose material
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
        let have = StageKind::Material.prelude_traced(Api::V0_2, tracing);
        assert_eq!(sha256_hex(have.as_bytes()), prelude, "{tracing:?}");
    }
}

/// The SHA-256 of 0.1's prelude and entries of each stage, as they were before 0.2: the line
/// packs run on today doesn't move.
const PINNED_0_1: [(StageKind, &str, &str); 5] = [
    (
        StageKind::Sky,
        "5a12b39ea8b56ad8deead04aa848671ceebde05cdd35af029d1bca5fe1e4cb51",
        "8fa92b1f28b48f70f7704cd1db183d5720484c1c92f779542a5805cd8d540fbc",
    ),
    (
        StageKind::Vertex,
        "1ba03e6acd07fc26e267f58fe65aaa607976faab1834d16f55c694e15eacf885",
        "0c512329404218b074bd5140c2dcb4268721ad2b280d5d516a8066ffc2a78f97",
    ),
    (
        StageKind::Material,
        "1f165861a0f74fa58bab41d9770798fd80ccbc12b61fa8ede7bb4804bcbea24e",
        "9dfaf3b191390a268a43bb3ecb8cbc1739e083b4e7c1783b58c76b332afe88cc",
    ),
    (
        StageKind::Weather,
        "174ad066203226f4b28848d6cc2037df75c9ea68481bbff8b73eca469d9b5bb7",
        "6fd42fe9ab23d5aca1ca0bb61d9021cad149e795574d892fcd8178a3dae15032",
    ),
    (
        StageKind::Post,
        "4b1b7b6c7b1149657965a178a879535975ed76d09524c294d4350c6c06db83ab",
        "a1d88de49274e1d9aed65d2a7fd500e9e385a7a7c9fbb6f9064d4bdbd3a8d6da",
    ),
];

#[test]
fn the_0_1_wgsl_is_unchanged() {
    for (kind, prelude, entries) in PINNED_0_1 {
        let have = (
            sha256_hex(kind.prelude(Api::V0_1).as_bytes()),
            sha256_hex(kind.entries(Api::V0_1).as_bytes()),
        );
        assert_eq!(have, (prelude.to_owned(), entries.to_owned()), "{kind:?}");
    }
}

/// The SHA-256 of 0.2's prelude and entries of each stage for a pack without `[ray_tracing]`, as
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
fn the_0_2_wgsl_of_a_pack_without_ray_tracing_is_unchanged() {
    for (kind, prelude, entries) in PINNED_0_2 {
        let have = (
            sha256_hex(kind.prelude(Api::V0_2).as_bytes()),
            sha256_hex(kind.entries(Api::V0_2).as_bytes()),
        );
        assert_eq!(have, (prelude.to_owned(), entries.to_owned()), "{kind:?}");
    }
    // And a whole stage of a pack in the tree, as the game composes it.
    let dir = root().join("crates/horizon-shader-api/packs/golden_hour_0_2");
    let pack = Pack::read(&dir).unwrap();
    let glow = compose(&pack, StageKind::Post, "post/glow.wgsl");
    assert_eq!(
        // As the repository holds the files: a Windows checkout may give them CRLF.
        sha256_hex(glow.text.replace('\r', "").as_bytes()),
        "ba23f7e523ae846f9122cb2d5b1560adadc02279b344b5b331ba7ed5a0e699d7"
    );
}

/// The `Frame` struct the line's sky prelude declares (for a pack with `[ray_tracing]` or
/// without; with `kept`, as one with `[[resource]]` has it), as (name, offset) and its span.
fn frame_layout(api: Api, ray_tracing: bool, kept: bool) -> (Vec<(String, usize)>, usize) {
    let tracing = if ray_tracing {
        Tracing::Stub
    } else {
        Tracing::None
    };
    // The prelude is the pack's (`tests/names.rs` has a kept one's): the struct alone here.
    let text = match kept {
        true => Frame::declared(api, ray_tracing, true).wgsl(),
        false => {
            let prelude = StageKind::Sky.prelude_traced(api, tracing);
            format!("{prelude}{}", vanilla(StageKind::Sky))
        }
    };
    let module = validate(&text);
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
    let (wgsl, span) = frame_layout(Api::V0_2, true, true);
    assert_eq!(span, size);
    assert_eq!(wgsl, rust);
    // Without `[ray_tracing]` the scene's two fields are a gap: the others stay where they are.
    let (wgsl, span) = frame_layout(Api::V0_2, false, true);
    assert_eq!(span, size);
    let mut untraced = rust.clone();
    untraced.drain(Frame::RAY_FIELDS);
    assert_eq!(wgsl, untraced);
    // Without `[[resource]]` the struct ends before the epochs, a prefix of the same buffer.
    let unkept = Frame::KEPT_FIELDS.len();
    let (wgsl, span) = frame_layout(Api::V0_2, true, false);
    assert_eq!(
        (span, wgsl),
        (size - 16, rust[..rust.len() - unkept].to_vec())
    );
    let (wgsl, span) = frame_layout(Api::V0_2, false, false);
    let untraced = &untraced[..untraced.len() - unkept];
    assert_eq!((span, wgsl), (size - 16, untraced.to_vec()));
    // 0.1 declares the start of the same buffer.
    let (wgsl, span) = frame_layout(Api::V0_1, false, false);
    assert_eq!(wgsl, rust[..Frame::FIELDS_0_1]);
    assert_eq!(span, rust[Frame::FIELDS_0_1].1);
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
fn the_references_are_up_to_date() {
    for api in Api::SERVED {
        let doc = reference::path(api);
        let path = root().join(doc);
        let want = reference::markdown(api);
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
}
