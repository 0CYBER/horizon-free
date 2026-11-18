//! The stages' WGSL against the Rust definitions: the preludes compile, the frame's WGSL layout
//! is the uploaded struct's, the WGSL of a pack without `[ray_tracing]` is what it was before the
//! key, and docs/api/shaders-reference-0.2.md is up to date. Rewrite the reference with
//! `HORIZON_WRITE_DOCS=1 cargo test -p horizon-shader-api --test it reference::`.
use horizon_shader_api::{
    Frame, Pack, StageKind, compose,
    package::sha256_hex,
    reference,
    stages::{Tracing, shaded_entries},
};
use naga::valid::Capabilities;
use std::path::Path;

/// The stage's entry points for a pack that doesn't name `shaded_tint`.
fn entries_of(kind: StageKind) -> String {
    shaded_entries(kind.entries(), false)
}

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
        let text = format!("{}{}{}", kind.prelude(), vanilla(kind), entries_of(kind));
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
        validate(&format!("{stub}{own}{}", entries_of(kind)));
        let scene = kind.prelude_traced(Tracing::Scene);
        let text = format!("{scene}{own}{}", entries_of(kind));
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
        "023aaffa02bca0c7ce0d6a2b577b0b0af8495a411dd822a40ef4409c739210d6",
    ),
    (
        Tracing::Scene,
        "3a67f6454a3f339e10030dd27d896fba89d76db6b35d4db895fcd1209cf16773",
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
/// they were before the key but for what changed for every pack since (the frame's fog fields,
/// the post stage's `history`, the terrain's looks): a pack that doesn't ask for the scene is
/// built from the same WGSL.
const PINNED_0_2: [(StageKind, &str, &str); 6] = [
    (
        StageKind::Sky,
        "88322d6f72dbda324d890c2f7a2dfa4c963e027bf422b952851c48a5f0d2ae67",
        "54bac4d416d804d8a96e1e31a6d62460f77573634c1e7bec16670641355d678a",
    ),
    (
        StageKind::Vertex,
        "d1a57e56014b96daafd07e4fe14a99f4e80d706d2032d26bdbac74bbfee436a7",
        "c77929934ce1572c21697f1e3328434638701370f5a7a94a9f3268d52d378636",
    ),
    (
        StageKind::Material,
        "7f5fc7fbca82bcb3ac0bbe45e38be645012698d6102913cd38696b7fd40c762d",
        "8b69597d65e4cadc049206cf473c144c07b1e61902a3ee3afb0a4ca137c00033",
    ),
    (
        StageKind::Weather,
        "336a93c79c08005cad9246ac88aaea4dad1ecd5d72f391e95fb55cc8a94e401d",
        "c2e4dabacd487692038c3fbf5708aed72a12d951c15184ceaf61ee1bb61b4982",
    ),
    (
        StageKind::Post,
        "de8725d1bd09091439222978f5c5c8f73d16c18c61125f5be395cb1104105993",
        "f1ffc3c9e0692e1d1f49de4a34ae90b93f7f2eda2eaa42ce936f630139cc4648",
    ),
    (
        StageKind::Tonemap,
        "c17c9c25f183c3d49db5a819249b79a61252e7eb18006021f6e493d1c95e6070",
        "2dcf7ee0874daeb033058ed55323b74c0da5de4a170cece2e6192500714c0b6a",
    ),
];

#[test]
fn the_wgsl_of_a_pack_without_ray_tracing_is_unchanged() {
    for (kind, prelude, entries) in PINNED_0_2 {
        let have = (
            sha256_hex(kind.prelude().as_bytes()),
            sha256_hex(entries_of(kind).as_bytes()),
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
        "e0de176db0fbae5c564406678c7f0142b1d679e1a8dc443a969f7abf2cd7828f"
    );
}

/// The `Frame` struct a prelude declares, for a pack with `[ray_tracing]` or without, with
/// `[[resource]]` or without, and whose files name `moon_phase` or not, as (name, offset) and
/// its span.
fn frame_layout(ray_tracing: bool, kept: bool, moon_phase: bool) -> (Vec<(String, usize)>, usize) {
    let module = validate(&Frame::declared_wgsl(ray_tracing, kept, moon_phase));
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
            for moon_phase in [false, true] {
                let mut want = rust.clone();
                let left = |range: std::ops::Range<usize>, on: bool| (!on).then_some(range);
                for range in [
                    left(Frame::MOON_FIELDS, moon_phase),
                    left(Frame::KEPT_FIELDS, kept),
                    left(Frame::RAY_FIELDS, ray_tracing),
                ]
                .into_iter()
                .flatten()
                {
                    want.drain(range);
                }
                let keys =
                    format!("ray tracing {ray_tracing}, kept {kept}, moon phase {moon_phase}");
                let (wgsl, span) = frame_layout(ray_tracing, kept, moon_phase);
                assert_eq!(wgsl, want, "{keys}");
                // The fog's fields are every pack's, and the moon's phase is in the padding of
                // the last 16 bytes: the struct is the buffer's size either way.
                assert_eq!(span, size, "{keys}");
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
