//! `[entity] reads` and `[sky] reads` (#16011 item 4, after `[material] reads`): the entity and
//! sky stages read kept resources by `[material]`'s rules, this frame's version of a texture a
//! `[[before_world]]` pass wrote, any texture's `_previous` and a buffer, declared in their
//! modules as a pass's are; they write none.
use horizon_shader_api::stages::{KEPT_BINDING, KeptAccess, kept_bindings};
use horizon_shader_api::{MANIFEST, Manifest, Pack, StageKind, check, compose};

const HEAD: &str = "schema = 1\nid = \"p\"\nname = \"P\"\nversion = \"1\"\napi = \"0.2\"\n";
/// A fixed texture and a buffer, both written by a `[[before_world]]` pass.
const KEPT: &str = "[[resource]]\nname = \"tex\"\nformat = \"r32float\"\nsize = [4, 4]\nkeep = \"view\"\n\
                    [[resource]]\nname = \"buf\"\nbuffer = 64\nelement = \"u32\"\nkeep = \"view\"\n";
const WORLD: &str = "[[before_world]]\nfile = \"w.wgsl\"\ndispatch = [4, 4, 1]\nworkgroup = [4, 4, 1]\n\
                     writes = [\"tex\", \"buf\"]\n";
const WRITE: &str = "fn compute(input: ComputeInput) {\n    if input.in_bounds {\n        \
                     textureStore(tex, vec2<i32>(input.cell.xy), vec4<f32>(1.0));\n        \
                     buf[input.cell.x] = 1u;\n    }\n}\n";
/// What each stage's function reads: this frame's texture, last frame's and the buffer.
const SUM: &str = "fn kept_sum() -> f32 {\n    \
                   return textureLoad(tex, vec2<i32>(0), 0).x\n        \
                   + textureLoad(tex_previous, vec2<i32>(0), 0).x + f32(buf[0]);\n}\n";
const ENTITY: &str =
    "fn entity(input: EntityInput) -> Surface { return Surface(input.albedo * kept_sum()); }\n";
const SKY: &str = "fn sky(input: SkyInput) -> vec4<f32> { return input.color * kept_sum(); }\n";
const ALL: &str = "reads = [\"tex\", \"tex_previous\", \"buf\"]\n";

/// The two stages, as `[name]`, their file and its source.
const STAGES: [(&str, &str, &str); 2] = [("entity", "e.wgsl", ENTITY), ("sky", "s.wgsl", SKY)];

fn stage(name: &str, reads: &str) -> String {
    format!(
        "{KEPT}{WORLD}[{name}]\nfile = \"{}.wgsl\"\n{reads}",
        &name[..1]
    )
}

fn parse(extra: &str) -> Result<Manifest, String> {
    Manifest::parse(&format!("{HEAD}{extra}"))
}

fn refused(extra: &str, needle: &str) {
    let e = parse(extra).expect_err(extra);
    assert!(e.contains(needle), "{needle:?} not in {e:?}");
}

fn pack(manifest: &str, files: &[(&str, &str)]) -> Pack {
    let mut all: std::collections::BTreeMap<String, Vec<u8>> = files
        .iter()
        .map(|(n, t)| ((*n).to_owned(), t.as_bytes().to_vec()))
        .collect();
    all.insert(MANIFEST.into(), format!("{HEAD}{manifest}").into_bytes());
    Pack::from_files(all).unwrap()
}

fn kind(name: &str) -> StageKind {
    match name {
        "entity" => StageKind::Entity,
        _ => StageKind::Sky,
    }
}

#[test]
fn the_entity_and_sky_stages_read_what_they_list() {
    for (name, file, _) in STAGES {
        let m = parse(&stage(name, ALL)).unwrap();
        let listed = if name == "entity" { m.entity } else { m.sky };
        assert_eq!(listed.unwrap().reads, ["tex", "tex_previous", "buf"]);
        // This frame's texture needs a `[[before_world]]` writer, as the material's does: a post
        // pass writes it after the stage draws. Last frame's and a buffer need none.
        let late = format!(
            "{KEPT}[{name}]\nfile = \"{file}\"\nreads = [\"tex_previous\", \"buf\"]\n\
             [[post]]\nfile = \"c.wgsl\"\ncompute = true\ndispatch = [4, 4, 1]\n\
             workgroup = [4, 4, 1]\nwrites = [\"tex\", \"buf\"]\n[[post]]\nfile = \"last.wgsl\"\n"
        );
        parse(&late).unwrap();
        let draws = if name == "entity" {
            "the entities draw"
        } else {
            "the sky draws"
        };
        refused(
            &late.replace("\"tex_previous\"", "\"tex\""),
            &format!(
                "{file}: reads \"tex\", this frame's, which no [[before_world]] pass writes \
                 before {draws}: read tex_previous"
            ),
        );
        let only_buf = WORLD.replace("\"tex\", ", "");
        refused(
            &format!("{KEPT}{only_buf}[{name}]\nfile = \"{file}\"\nreads = [\"tex\"]\n"),
            "read tex_previous",
        );
    }
}

#[test]
fn the_entity_and_sky_stages_name_declared_resources_once_and_write_none() {
    let keys = "and `reads` of [material], [entity] and [sky]: they draw with kept resources, \
                compute passes write them";
    for (name, file, _) in STAGES {
        for (reads, needle) in [
            (
                "reads = [\"nothing\"]",
                format!("{file}: reads \"nothing\", no [[resource]]"),
            ),
            (
                "reads = [\"buf_previous\"]",
                "no [[resource]] or texture's _previous".to_owned(),
            ),
            (
                "reads = [\"buf\", \"buf\"]",
                "reads \"buf\" twice".to_owned(),
            ),
        ] {
            refused(&stage(name, reads), &needle);
        }
        for key in [
            "writes = [\"buf\"]",
            "compute = true",
            "dispatch = [1, 1, 1]",
            "workgroup = [1, 1, 1]",
        ] {
            refused(&stage(name, &format!("{key}\n")), keys);
        }
    }
}

/// Each stage's module declares what it lists at the pass's bindings, as `kept_bindings` gives
/// them, and nothing it doesn't list; a file naming a resource its `reads` doesn't list is
/// refused.
#[test]
fn the_entity_and_sky_modules_declare_their_reads() {
    for (name, file, source) in STAGES {
        let p = pack(
            &stage(name, ALL),
            &[("w.wgsl", WRITE), (file, &format!("{SUM}{source}"))],
        );
        let report = check(&p).unwrap();
        let checked = (report.stages.iter())
            .find(|s| s.kind == kind(name))
            .unwrap();
        let listed = if name == "entity" {
            p.manifest.entity.as_ref()
        } else {
            p.manifest.sky.as_ref()
        };
        let want: Vec<_> = (kept_bindings(&p.manifest, listed.unwrap()).iter())
            .map(|b| (b.binding, b.access))
            .collect();
        assert_eq!(
            want,
            [
                (KEPT_BINDING, KeptAccess::Read),
                (KEPT_BINDING + 1, KeptAccess::Previous),
                (KEPT_BINDING + 2, KeptAccess::Read)
            ]
        );
        let text = compose(&p, kind(name), file).text;
        for line in [
            "@group(3) @binding(64) var tex: texture_2d<f32>;",
            "@group(3) @binding(65) var tex_previous: texture_2d<f32>;",
            "@group(3) @binding(66) var<storage, read> buf: array<u32>;",
        ] {
            assert!(text.contains(line), "{name}: {line}");
        }
        let bound: Vec<_> = (checked.module.global_variables.iter())
            .filter_map(|(_, g)| g.binding.as_ref().filter(|b| b.binding >= KEPT_BINDING))
            .map(|b| b.binding)
            .collect();
        assert_eq!(bound, [64, 65, 66], "{name}");
        // Listing the buffer alone: `tex` is undeclared there.
        let p = pack(
            &stage(name, "reads = [\"buf\"]\n"),
            &[("w.wgsl", WRITE), (file, &format!("{SUM}{source}"))],
        );
        let e = check(&p).expect_err(name);
        let e: Vec<String> = e.iter().map(ToString::to_string).collect();
        let needle = format!("`tex` is a kept resource the {name} stage doesn't list in `reads`");
        assert!(
            e.iter()
                .any(|p| p.starts_with(&format!("{file}:2:")) && p.contains(&needle)),
            "{e:#?}"
        );
    }
}

/// An entity or sky stage without `reads` in a kept pack composes as before `[entity] reads`
/// and `[sky] reads`: nothing at the kept bindings, byte for byte (stages.rs is unchanged, so
/// these are the hashes before them too).
#[test]
fn entity_and_sky_stages_without_reads_compose_as_before() {
    let plain = [
        "fn entity(input: EntityInput) -> Surface { return Surface(input.albedo); }\n",
        "fn sky(input: SkyInput) -> vec4<f32> { return input.color; }\n",
    ];
    let mut hashes = Vec::new();
    for ((name, file, _), source) in STAGES.into_iter().zip(plain) {
        let p = pack(&stage(name, ""), &[("w.wgsl", WRITE), (file, source)]);
        check(&p).unwrap();
        let text = compose(&p, kind(name), file).text;
        assert!(!text.contains("@binding(64)"), "{name}");
        hashes.push(horizon_shader_api::package::sha256_hex(text.as_bytes()));
    }
    assert_eq!(
        hashes,
        [
            "bd61824c6b2e78d5597e660d3bf613709605efe9bdc7ef97196c51df16b9ef27",
            "c117b03fc94824440ac38aebad9481acdb4471407e41d96108c2779e811b3c3d",
        ]
    );
}
