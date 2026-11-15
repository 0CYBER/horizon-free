//! Shader API 0.2's kept resources (#16011 item 4): every manifest rule of `[kept]`,
//! `[[resource]]`, a kept `output` and options' `resets`.
use horizon_shader_api::kept::{Element, Format, Keep, Shape};
use horizon_shader_api::{MANIFEST, Manifest, Pack, check};

const HEAD: &str = "schema = 1\nid = \"p\"\nname = \"P\"\nversion = \"1\"\napi = \"0.2\"\n";
/// The pass that draws the picture, after every other entry.
const LAST: &str = "[[post]]\nfile = \"last.wgsl\"\n";
/// A fixed texture and a buffer.
const RES: &str = "[[resource]]\nname = \"tex\"\nformat = \"r32float\"\nsize = [4, 4]\nkeep = \"view\"\n\
                   [[resource]]\nname = \"buf\"\nbuffer = 64\nelement = \"u32\"\nkeep = \"view\"\n";

fn parse(extra: &str) -> Result<Manifest, String> {
    Manifest::parse(&format!("{HEAD}{extra}{LAST}"))
}

fn refused(extra: &str, needle: &str) {
    let e = parse(extra).expect_err(extra);
    assert!(e.contains(needle), "{needle:?} not in {e:?}");
}

fn resource(name: &str, shape: &str) -> String {
    format!("[[resource]]\nname = \"{name}\"\n{shape}\nkeep = \"view\"\n")
}

/// The design's resources: a world cache, a 1×1 exposure and two histories, one written as an
/// `output`.
const EXAMPLE: &str = r#"
[kept]
reach = 256

[[resource]]
name = "cache"
buffer = 16777216
element = "vec4<u32>"
keep = "world"

[[resource]]
name = "exposure"
format = "r32float"
size = [1, 1]
keep = "view"

[[resource]]
name = "ray_length"
format = "r32float"
scale = 1
keep = "view"

[[resource]]
name = "radiance"
format = "rgba16float"
scale = 1
keep = "view"

[[post]]
file = "denoise.wgsl"
output = "radiance"

[[options]]
id = "bounces"
name = "Bounces"
type = "int"
default = 1
min = 1
max = 4
resets = "world"
"#;

#[test]
fn the_design_s_resources_read_as_declared() {
    let m = parse(EXAMPLE).unwrap();
    let k = &m.kept;
    assert_eq!(k.reach, Some(256));
    let shapes: Vec<_> = (k.resources.iter())
        .map(|r| (r.name.as_str(), r.shape, r.keep))
        .collect();
    let (buffer, fixed, scaled) = (
        Shape::Buffer {
            bytes: 16 << 20,
            element: Element::Vec4U32,
        },
        Shape::Fixed {
            format: Format::R32Float,
            size: [1, 1],
        },
        |format| Shape::Scaled { format, scale: 1.0 },
    );
    assert_eq!(
        shapes,
        [
            ("cache", buffer, Keep::World),
            ("exposure", fixed, Keep::View),
            ("ray_length", scaled(Format::R32Float), Keep::View),
            ("radiance", scaled(Format::Rgba16Float), Keep::View),
        ]
    );
    assert_eq!(k.resets, [("bounces".to_owned(), Keep::World)]);
    assert!(m.computes());
    // The kept output is no post target of its own: the two R-relative textures take all four.
    assert!(m.post_targets().is_empty());
    assert!(!parse("").unwrap().computes());
}

/// The check takes them (the game refuses them: `native_shaders.rs`).
#[test]
fn a_pack_with_kept_resources_passes_the_check() {
    let post = "override bounces: i32 = 1;\n\
                fn post(input: PostInput) -> vec4<f32> { return input.color * f32(bounces); }\n";
    let files = [
        (MANIFEST, format!("{HEAD}{EXAMPLE}{LAST}")),
        ("denoise.wgsl", post.to_owned()),
        ("last.wgsl", post.to_owned()),
    ];
    let pack = Pack::from_files(
        (files.into_iter())
            .map(|(n, t)| (n.to_owned(), t.into_bytes()))
            .collect(),
    )
    .unwrap();
    check(&pack).unwrap();
}

#[test]
fn reserved_and_taken_names_are_refused() {
    // `distance` and `active` are WGSL's: the API says `ray_length` and `in_bounds`.
    for name in [
        "distance",
        "active",
        "length",
        "frame",
        "previous",
        "compute",
        "Cache",
        "horizon_x",
    ] {
        refused(
            &resource(name, "format = \"r32float\"\nsize = [1, 1]"),
            "resource",
        );
    }
    let buffer = resource("trace", "buffer = 4\nelement = \"u32\"");
    refused(&format!("[ray_tracing]\n{buffer}"), "not a free WGSL name");
    parse(&buffer).unwrap();
    let previous = resource("tex_previous", "buffer = 4\nelement = \"u32\"");
    refused(&format!("{RES}{previous}"), "another's _previous");
    let twice = resource("buf", "buffer = 4\nelement = \"u32\"");
    refused(&format!("{RES}{twice}"), "declared twice");
    // An `output` by a reserved word was refused by naga before; now by the manifest.
    refused(
        "[[post]]\nfile = \"a.wgsl\"\noutput = \"active\"\n",
        "not a free WGSL name",
    );
}

#[test]
fn a_resource_has_one_shape() {
    let cases = [
        ("buffer = 4", "needs an `element`"),
        (
            "buffer = 4\nelement = \"vec3<f32>\"",
            "element \"vec3<f32>\"",
        ),
        ("buffer = 6\nelement = \"u32\"", "a whole number of u32s"),
        ("buffer = 0\nelement = \"u32\"", "a whole number"),
        (
            "buffer = 4\nelement = \"u32\"\nscale = 1",
            "a buffer has no `size` or `scale`",
        ),
        (
            "format = \"rgba8unorm\"\nsize = [1, 1]",
            "format \"rgba8unorm\"",
        ),
        (
            "format = \"r32float\"",
            "a texture has a `size` or a `scale`",
        ),
        (
            "format = \"r32float\"\nsize = [1, 1]\nscale = 1",
            "a texture has a `size` or a `scale`",
        ),
        ("format = \"r32float\"\nsize = [0, 1]", "at least 1 texel"),
        ("format = \"r32float\"\nscale = 0.3", "scale 0.3"),
        (
            "format = \"r32float\"\nsize = [1, 1]\nelement = \"u32\"",
            "`element` is a buffer's",
        ),
        (
            "buffer = 4\nformat = \"r32float\"",
            "either `buffer` (bytes) or `format`",
        ),
        ("", "either `buffer` (bytes) or `format`"),
    ];
    for (shape, needle) in cases {
        refused(&resource("r", shape), needle);
    }
    refused(
        "[[resource]]\nname = \"r\"\nbuffer = 4\nelement = \"u32\"\nkeep = \"frame\"\n",
        "keep \"frame\"",
    );
    for element in ["u32", "atomic<u32>", "f32", "vec4<u32>", "vec4<f32>"] {
        parse(&resource(
            "r",
            &format!("buffer = 64\nelement = \"{element}\""),
        ))
        .unwrap();
    }
    for format in ["rgba16float", "r32float", "rgba32float", "rgba32uint"] {
        parse(&resource(
            "r",
            &format!("format = \"{format}\"\nscale = 0.5"),
        ))
        .unwrap();
    }
}

#[test]
fn world_resources_are_fixed_and_have_a_reach() {
    let world = "[[resource]]\nname = \"w\"\nformat = \"r32float\"\nkeep = \"world\"\n";
    refused(
        &format!("[kept]\nreach = 64\n{world}scale = 1\n"),
        "keep = \"world\" with `scale`",
    );
    refused(&format!("{world}size = [8, 8]\n"), "needs [kept] reach");
    refused(
        &format!("[kept]\nreach = 0\n{world}size = [8, 8]\n"),
        "at least 1 block",
    );
    refused(
        &format!("[kept]\nreach = 64\n{RES}"),
        "no keep = \"world\" resource",
    );
    parse(&format!("[kept]\nreach = 64\n{world}size = [8, 8]\n")).unwrap();
}

#[test]
fn counts_and_memory_are_bounded() {
    let many = |n: usize, shape: &str| {
        (0..n)
            .map(|i| resource(&format!("r{i}"), shape))
            .collect::<String>()
    };
    let small = "buffer = 4\nelement = \"u32\"";
    parse(&many(8, small)).unwrap();
    refused(
        &many(9, small),
        "9 [[resource]] entries; a pack may declare 8",
    );
    // Two 16 MiB caches fit, and nothing more.
    let half = "buffer = 16777216\nelement = \"vec4<u32>\"";
    parse(&many(2, half)).unwrap();
    refused(
        &format!("{}{}", many(2, half), resource("more", small)),
        "buffers take 33554436 bytes",
    );
    // 1 MiB of fixed textures, both versions counted: two 128² of 16-byte texels, not one 256².
    for format in ["rgba32float", "rgba32uint"] {
        parse(&many(
            2,
            &format!("format = \"{format}\"\nsize = [128, 128]"),
        ))
        .unwrap();
        refused(
            &many(1, &format!("format = \"{format}\"\nsize = [256, 256]")),
            "fixed-size [[resource]] textures take 2097152 bytes",
        );
    }
    // R-relative textures take two of the four post targets each.
    let scaled = "format = \"rgba16float\"\nscale = 0.5";
    parse(&many(2, scaled)).unwrap();
    refused(
        &many(3, scaled),
        "and 6 for R-relative [[resource]] textures",
    );
    let output = "[[post]]\nfile = \"a.wgsl\"\noutput = \"glow\"\nscale = 0.5\n";
    refused(
        &format!("{}{output}", many(2, scaled)),
        "1 [[post]] outputs and 4 for R-relative",
    );
    parse(&format!("{}{output}", many(1, scaled))).unwrap();
}

/// Sizes past `u64` are refused as over budget, never wrapped or panicking.
#[test]
fn extreme_sizes_are_over_budget() {
    let huge = "format = \"rgba16float\"\nsize = [1073741824, 1073741824]";
    refused(
        &resource("r", huge),
        "textures take over 2^64 bytes with both versions",
    );
    let side = "format = \"rgba32float\"\nsize = [4294967295, 4294967295]";
    refused(&resource("r", side), "textures take over 2^64 bytes");
    // One buffer at TOML's largest whole number of u32s, then three whose sum passes u64.
    let most = "buffer = 9223372036854775804\nelement = \"u32\"";
    refused(
        &resource("r", most),
        "buffers take 9223372036854775804 bytes",
    );
    let three = (0..3)
        .map(|i| resource(&format!("r{i}"), most))
        .collect::<String>();
    refused(&three, "buffers take over 2^64 bytes");
}

#[test]
fn an_output_writes_a_kept_texture_at_its_size() {
    let output = |keys: &str| format!("{RES}[[post]]\nfile = \"a.wgsl\"\n{keys}\n");
    parse(&output("output = \"tex\"")).unwrap();
    refused(&output("output = \"tex\"\nscale = 0.5"), "with no `scale`");
    refused(&output("output = \"buf\""), "is a kept buffer");
    let uint = resource("ints", "format = \"rgba32uint\"\nsize = [4, 4]");
    refused(
        &format!("{uint}[[post]]\nfile = \"a.wgsl\"\noutput = \"ints\"\n"),
        "is an rgba32uint texture: a pass draws floats, so only a compute pass writes one",
    );
    refused(
        &output("output = \"tex_previous\""),
        "is last frame's: nothing writes it",
    );
    let last = Manifest::parse(&format!(
        "{HEAD}{RES}[[post]]\nfile = \"z.wgsl\"\noutput = \"tex\"\n"
    ));
    assert!(
        last.unwrap_err()
            .contains("the last [[post]] pass draws the picture")
    );
}

#[test]
fn options_say_what_they_reset() {
    let option = |resets: &str| {
        format!(
            "[[options]]\nid = \"o\"\nname = \"O\"\ntype = \"bool\"\ndefault = true\nresets = \"{resets}\"\n"
        )
    };
    let view = parse(&format!("{RES}{}", option("view"))).unwrap();
    assert_eq!(view.kept.resets, [("o".to_owned(), Keep::View)]);
    refused(
        &format!("{RES}{}", option("always")),
        "resets \"always\": view or world",
    );
    refused(&option("view"), "resets, with no [[resource]] to clear");
}

#[test]
fn shader_api_0_1_has_none_of_it() {
    let old = |extra: &str| {
        Manifest::parse(&format!("{}{extra}{LAST}", HEAD.replace("0.2", "0.1"))).unwrap_err()
    };
    assert!(old(RES).contains("[[resource]] is shader API 0.2"));
    assert!(old("[kept]\nreach = 1\n").contains("[kept] is shader API 0.2"));
    let option = "[[options]]\nid = \"o\"\nname = \"O\"\ntype = \"bool\"\ndefault = true\nresets = \"view\"\n";
    assert!(old(option).contains("an option's `resets` is shader API 0.2"));
}
