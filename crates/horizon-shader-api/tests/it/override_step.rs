//! `check` takes a stage through naga's override step as the game's pipelines do: the game joins
//! a material, vertex or entity stage to its own shader, whose overrides make wgpu re-evaluate
//! every function of the stage, and naga 30 refuses some valid WGSL there (gfx-rs/wgpu#7793,
//! #9002). A stage the game builds alone (a post pass) meets that only with an option of its own.
use horizon_shader_api::{MANIFEST, Pack, Problem, check};

fn material_pack(
    material: &str,
    options: &str,
) -> Result<horizon_shader_api::Report, Vec<Problem>> {
    let manifest = format!(
        "schema = 1\nid = \"p\"\nname = \"P\"\nversion = \"1\"\napi = \"0.2\"\n\
         [material]\nfile = \"material.wgsl\"\n{options}"
    );
    let files = [(MANIFEST, manifest.as_str()), ("material.wgsl", material)]
        .map(|(n, t)| (n.to_owned(), t.as_bytes().to_vec()))
        .into();
    check(&Pack::from_files(files).unwrap())
}

const TABLE: &str = "const TABLE: array<vec4<f32>, 2> = array<vec4<f32>, 2>(
    vec4<f32>(1.0, 0.5, 0.25, 1.0), vec4<f32>(0.5, 1.0, 0.25, 1.0));\n";
const MATERIAL: &str = "fn material(input: MaterialInput) -> Surface { return Surface(input.albedo * noted_light()); }\n";
/// gfx-rs/wgpu#7793: the components of a `let` of a constant vector, read in a later statement.
const COMPONENTS: &str =
    "fn noted_light() -> f32 { let named = TABLE[1]; return max(named.x, named.y); }\n";
/// gfx-rs/wgpu#9002: a `let` folded like a constant, into a builtin's domain rule.
const FOLDED: &str =
    "fn noted_light() -> f32 { let named = TABLE[1]; return sqrt(named.x - 2.0); }\n";

#[test]
fn a_stage_the_game_joins_to_its_shader_is_taken_through_the_override_step() {
    for (body, cause) in [
        (COMPONENTS, "'noted_light' is invalid: Expression ["),
        (
            FOLDED,
            "sqrt built-in function argument is out of valid range",
        ),
    ] {
        let own = [TABLE, body, MATERIAL].concat();
        let problems = material_pack(&own, "").expect_err("the game's Terrain pipeline refuses it");
        let p = &problems[0];
        assert_eq!(p.file, "material.wgsl", "{p}");
        assert!(p.message.contains("the override step refuses it"), "{p}");
        assert!(p.message.contains(cause), "{p}");
        // As a stage with an option of its own already was.
        let option = "[[options]]\nid = \"gain\"\nname = \"Gain\"\ntype = \"float\"\n\
                      default = 1.0\nmin = 0.0\nmax = 2.0\nstep = 0.1\n";
        let own = own.replace("noted_light());", "noted_light() * gain);");
        let with_option = material_pack(&format!("override gain: f32 = 1.0;\n{own}"), option)
            .expect_err("an override of the pack's own");
        assert!(
            with_option[0]
                .message
                .contains("the override step refuses it")
        );
    }
    // What the reference says to write instead.
    for body in [
        "fn noted_light() -> f32 { const named = TABLE[1]; return max(named.x, named.y); }\n",
        "fn noted_light() -> f32 { return max(TABLE[1].x, TABLE[1].y); }\n",
        "fn noted_light() -> f32 { var named = TABLE[1]; return max(named.x, named.y); }\n",
    ] {
        material_pack(&[TABLE, body, MATERIAL].concat(), "").unwrap();
    }
    // A post pass without options is built alone, in the game as here: it runs today.
    let post = [TABLE, COMPONENTS].concat()
        + "fn post(input: PostInput) -> vec4<f32> { return input.color * noted_light(); }
";
    let manifest = "schema = 1
id = \"p\"
name = \"P\"
version = \"1\"
api = \"0.2\"
                    [[post]]
file = \"post.wgsl\"
";
    let files = [(MANIFEST, manifest), ("post.wgsl", post.as_str())]
        .map(|(n, t)| (n.to_owned(), t.as_bytes().to_vec()))
        .into();
    check(&Pack::from_files(files).unwrap()).unwrap();
}
