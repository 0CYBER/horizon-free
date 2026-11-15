//! `[motion]`: the motion guide's names (`scene_motion`, `motion_at` and the rest) in every 0.2
//! post pass of a pack that sets it, and in nothing of a pack that doesn't.
use horizon_shader_api::{MANIFEST, Manifest, Pack, Problem, Report, StageKind, check, compose};

/// A pack on the line `api` with `[motion]` when `motion`, and a post pass per file of `posts`.
fn pack(api: &str, motion: bool, posts: &[&str]) -> Pack {
    let mut manifest =
        format!("schema = 1\nid = \"p\"\nname = \"P\"\nversion = \"1\"\napi = \"{api}\"\n");
    let mut files = std::collections::BTreeMap::new();
    for (i, post) in posts.iter().enumerate() {
        manifest += &format!("[[post]]\nfile = \"post{i}.wgsl\"\n");
        files.insert(format!("post{i}.wgsl"), post.as_bytes().to_vec());
    }
    if motion {
        manifest += "[motion]\n";
    }
    files.insert(MANIFEST.to_owned(), manifest.into_bytes());
    Pack::from_files(files).unwrap()
}

fn checked(motion: bool, post: &str) -> Result<Report, Vec<Problem>> {
    check(&pack("0.2", motion, &[post]))
}

const PLAIN: &str = "fn post(input: PostInput) -> vec4<f32> { return input.color; }\n";

const READER: &str = "fn post(input: PostInput) -> vec4<f32> {\n\
                      let m = motion_at(vec2<i32>(input.pixel));\n\
                      return select(input.color, vec4<f32>(1.0), m.kind == MOTION_OBJECT);\n}\n";

/// The 0.2 post stage `PLAIN` composed to before `[motion]` (`origin/main` at a51b2d4105).
const STAGE_SHA: &str = "d9bed86dce9bd8e2417382774349cac05b85ab9e1b411eb577bf0af62fb244d1";

#[test]
fn a_pack_without_motion_composes_as_before() {
    let composed = compose(&pack("0.2", false, &[PLAIN]), StageKind::Post, "post0.wgsl");
    let sha = horizon_shader_api::package::sha256_hex(composed.text.as_bytes());
    assert_eq!(sha, STAGE_SHA);
    // The names stay the pack's own.
    let own = "fn motion_at(t: vec2<i32>) -> f32 { return 1.0; }\n\
               fn post(input: PostInput) -> vec4<f32> { return input.color * motion_at(vec2<i32>(0)); }\n";
    checked(false, own).unwrap();
}

#[test]
fn every_post_pass_of_a_motion_pack_reads_the_guide() {
    let pack = pack("0.2", true, &[READER, PLAIN]);
    for file in ["post0.wgsl", "post1.wgsl"] {
        let text = compose(&pack, StageKind::Post, file).text;
        assert!(text.contains("@group(3) @binding(14) var scene_motion: texture_2d<f32>;"));
    }
    let report = check(&pack).unwrap();
    let reads: Vec<bool> = (report.stages.iter())
        .map(|s| s.reads("scene_motion"))
        .collect();
    assert_eq!(reads, [true, false]);
    // Directly, without the helper.
    let direct = "fn post(input: PostInput) -> vec4<f32> {\n\
                  return textureLoad(scene_motion, vec2<i32>(input.pixel), 0);\n}\n";
    assert!(checked(true, direct).unwrap().stages[0].reads("scene_motion"));
}

/// The sky (depth 0) takes the engine's texel, written from the unbobbed views, before anything
/// reads `w` as an object with nothing to match. M2b's GPU tests hold the numbers.
#[test]
fn the_sky_reads_the_engine_s_motion() {
    let text = compose(&pack("0.2", true, &[READER]), StageKind::Post, "post0.wgsl").text;
    let at = |needle: &str| text.find(needle).unwrap_or_else(|| panic!("{needle}"));
    let sky = at("if depth == 0.0 {");
    let camera = at("return MotionData(object.xy, MOTION_CAMERA);");
    assert!(sky < camera && camera < at("return MotionData(vec2<f32>(0.0), MOTION_NEW);"));
}

#[test]
fn the_guide_without_motion_says_to_set_it() {
    let problems = checked(false, READER).expect_err("should fail");
    assert!(
        problems
            .iter()
            .any(|p| p.message.contains("`motion_at`") && p.message.contains("need `[motion]`")),
        "{problems:#?}"
    );
}

#[test]
fn motion_is_0_2_with_a_post_pass() {
    let manifest = |api: &str, stage: &str| {
        format!(
            "schema = 1\nid = \"p\"\nname = \"P\"\nversion = \"1\"\napi = \"{api}\"\n\
             {stage}\nfile = \"a.wgsl\"\n[motion]\n"
        )
    };
    assert!(
        Manifest::parse(&manifest("0.2", "[[post]]"))
            .unwrap()
            .motion
            .is_some()
    );
    for (text, needle) in [
        (manifest("0.1", "[[post]]"), "[motion] is shader API 0.2"),
        (manifest("0.2", "[sky]"), "[motion] needs a [[post]] pass"),
        (manifest("0.2", "[[post]]") + "on = true\n", "unknown field"),
    ] {
        let e = Manifest::parse(&text).unwrap_err().to_string();
        assert!(e.contains(needle), "{needle:?} not in {e:?}");
    }
}

#[test]
fn motion_data_is_horizon_s() {
    let post = "fn post(input: PostInput) -> vec4<f32> {\n\
                let m = MotionData(vec2<f32>(0.0), MOTION_CAMERA);\n\
                return input.color + vec4<f32>(m.delta, 0.0, 0.0);\n}\n";
    let problems = checked(true, post).expect_err("should fail");
    assert!(
        (problems.iter()).any(|p| p.message.contains("constructs `MotionData`")),
        "{problems:#?}"
    );
}
