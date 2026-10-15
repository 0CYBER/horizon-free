//! The C preprocessor pass: Iris's standard macros (`StandardMacros`,
//! `IrisDefines`) defined, then every directive and macro expanded by pp-rs,
//! the preprocessor naga's GLSL front end uses. The output keeps the inlined
//! source's line numbers, so an error later on still names `file:line`.

use crate::include::Inlined;
use pp_rs::{
    pp::Preprocessor,
    token::{Punct, TokenValue},
};

/// The version of Minecraft packs are told they run on (`MC_VERSION`,
/// Iris's `formatVersionString("26.3")`).
pub const MC_VERSION: &str = "260300";
/// The Iris release whose behaviour the translation follows (`IRIS_VERSION`, 1.10.0).
pub const IRIS_VERSION: &str = "11000";

/// Iris's `WorldRenderingPhase`, in order: `MC_RENDER_STAGE_<name>` is the index.
pub const RENDER_STAGES: &[&str] = &[
    "NONE",
    "SKY",
    "SUNSET",
    "CUSTOM_SKY",
    "SUN",
    "MOON",
    "STARS",
    "VOID",
    "TERRAIN_SOLID",
    "TERRAIN_CUTOUT_MIPPED",
    "TERRAIN_CUTOUT",
    "ENTITIES",
    "BLOCK_ENTITIES",
    "DESTROY",
    "OUTLINE",
    "DEBUG",
    "HAND_SOLID",
    "TERRAIN_TRANSLUCENT",
    "TRIPWIRE",
    "PARTICLES",
    "CLOUDS",
    "RAIN_SNOW",
    "WORLD_BORDER",
    "HAND_TRANSLUCENT",
];

/// Iris's `BiomeCategories`, in order: `CAT_<name>` is the index.
pub const BIOME_CATEGORIES: &[&str] = &[
    "NONE",
    "TAIGA",
    "EXTREME_HILLS",
    "JUNGLE",
    "MESA",
    "PLAINS",
    "SAVANNA",
    "ICY",
    "THE_END",
    "BEACH",
    "FOREST",
    "OCEAN",
    "DESERT",
    "RIVER",
    "SWAMP",
    "MUSHROOM",
    "NETHER",
    "MOUNTAIN",
    "UNDERGROUND",
];

/// Distant Horizons block kinds Iris always defines (`DH_BLOCK_<name>`).
const DH_BLOCKS: &[&str] = &[
    "UNKNOWN",
    "LEAVES",
    "STONE",
    "WOOD",
    "METAL",
    "DIRT",
    "LAVA",
    "DEEPSLATE",
    "SNOW",
    "SAND",
    "TERRACOTTA",
    "NETHER_STONE",
    "WATER",
    "GRASS",
    "AIR",
    "ILLUMINATED",
];

/// The macros every program is preprocessed with.
#[derive(Clone, Debug)]
pub struct Environment {
    pub defines: Vec<(String, String)>,
}

impl Environment {
    /// What Iris defines (`IrisDefines.createIrisReplacements`) for a GPU
    /// whose adapter reports `vendor` and `renderer`, without any GL
    /// extension (`MC_GL_*`) or mod macro: packs then take their portable
    /// paths. Biomes (`BIOME_*`) and the texture format come from the game:
    /// add them with [`Environment::define`].
    pub fn iris(vendor: &str, renderer: &str, mipmap_levels: u32) -> Self {
        let mut env = Self {
            defines: Vec::new(),
        };
        env.define("MC_VERSION", MC_VERSION);
        env.define("MC_MIPMAP_LEVEL", &mipmap_levels.to_string());
        env.define("IRIS_VERSION", IRIS_VERSION);
        // Horizon's renderer stands in for GL 4.6 / GLSL 4.60.
        env.define("MC_GL_VERSION", "460");
        env.define("MC_GLSL_VERSION", "460");
        env.define(os_macro(), "");
        env.define(vendor_macro(vendor), "");
        env.define(renderer_macro(renderer), "");
        for flag in [
            "IS_IRIS",
            "IRIS_REQUIRES_SEPARATE_ENTITY_DRAWS",
            "IRIS_HAS_TRANSLUCENCY_SORTING",
        ] {
            env.define(flag, "");
        }
        env.define("MAX_COLOR_BUFFERS", "32");
        env.define("IRIS_TAG_SUPPORT", "2");
        for (i, name) in DH_BLOCKS.iter().enumerate() {
            env.define(&format!("DH_BLOCK_{name}"), &i.to_string());
        }
        env.define("MC_NORMAL_MAP", "");
        env.define("MC_SPECULAR_MAP", "");
        env.define("MC_RENDER_QUALITY", "1.0");
        env.define("MC_SHADOW_QUALITY", "1.0");
        env.define("MC_HAND_DEPTH", "0.125");
        for (i, name) in RENDER_STAGES.iter().enumerate() {
            env.define(&format!("MC_RENDER_STAGE_{name}"), &i.to_string());
        }
        for (i, name) in BIOME_CATEGORIES.iter().enumerate() {
            env.define(&format!("CAT_{name}"), &i.to_string());
        }
        env.define("PPT_NONE", "0");
        env.define("PPT_RAIN", "1");
        env.define("PPT_SNOW", "2");
        env
    }

    /// Define `name` (replacing an earlier value).
    pub fn define(&mut self, name: &str, value: &str) {
        self.defines.retain(|(n, _)| n != name);
        self.defines.push((name.to_owned(), value.to_owned()));
    }
}

fn os_macro() -> &'static str {
    if cfg!(target_os = "macos") {
        "MC_OS_MAC"
    } else if cfg!(windows) {
        "MC_OS_WINDOWS"
    } else {
        "MC_OS_LINUX"
    }
}

/// `StandardMacros.getVendor`.
pub fn vendor_macro(vendor: &str) -> &'static str {
    let v = vendor.to_lowercase();
    const TABLE: &[(&str, &str)] = &[
        ("ati", "MC_GL_VENDOR_ATI"),
        ("intel", "MC_GL_VENDOR_INTEL"),
        ("nvidia", "MC_GL_VENDOR_NVIDIA"),
        ("amd", "MC_GL_VENDOR_AMD"),
        ("x.org", "MC_GL_VENDOR_XORG"),
    ];
    TABLE
        .iter()
        .find(|(prefix, _)| v.starts_with(prefix))
        .map_or("MC_GL_VENDOR_OTHER", |(_, m)| m)
}

/// `StandardMacros.getRenderer`.
pub fn renderer_macro(renderer: &str) -> &'static str {
    let r = renderer.to_lowercase();
    const TABLE: &[(&str, &str)] = &[
        ("amd", "MC_GL_RENDERER_RADEON"),
        ("ati", "MC_GL_RENDERER_RADEON"),
        ("radeon", "MC_GL_RENDERER_RADEON"),
        ("gallium", "MC_GL_RENDERER_GALLIUM"),
        ("intel", "MC_GL_RENDERER_INTEL"),
        ("geforce", "MC_GL_RENDERER_GEFORCE"),
        ("nvidia", "MC_GL_RENDERER_GEFORCE"),
        ("quadro", "MC_GL_RENDERER_QUADRO"),
        ("nvs", "MC_GL_RENDERER_QUADRO"),
        ("mesa", "MC_GL_RENDERER_MESA"),
        ("apple", "MC_GL_RENDERER_APPLE"),
    ];
    TABLE
        .iter()
        .find(|(prefix, _)| r.starts_with(prefix))
        .map_or("MC_GL_RENDERER_OTHER", |(_, m)| m)
}

/// A program after the preprocessor: plain GLSL with no directives.
#[derive(Debug)]
pub struct Preprocessed {
    /// The source, one output line per inlined line (macro expansions joined).
    pub text: String,
    /// The `#version` line's words (`["330", "compatibility"]`), if any.
    pub version: Vec<String>,
    /// `#extension` lines' words (`["GL_ARB_gpu_shader5", ":", "enable"]`).
    pub extensions: Vec<Vec<String>>,
    /// The comments Iris reads directives from (`/* DRAWBUFFERS:01 */`,
    /// `const int colortex0Format = RGBA16F;`) that are outside `#if`'d out
    /// blocks, with their output line. Iris keeps comments through its
    /// preprocessor; pp-rs drops them.
    pub comments: Vec<(u32, String)>,
}

impl Preprocessed {
    /// The text with its kept comments put back at the start of their
    /// lines: what Iris's directive parsers read.
    pub fn with_comments(&self) -> String {
        let mut comments = self.comments.iter().peekable();
        let mut out = String::new();
        for (i, line) in self.text.lines().enumerate() {
            while let Some((_, comment)) = comments.next_if(|(at, _)| *at as usize <= i + 1) {
                out.push_str(comment);
                out.push(' ');
            }
            out.push_str(line);
            out.push('\n');
        }
        out
    }
}

/// What a comment must hold for Iris to read a directive from it.
const DIRECTIVE_WORDS: &[&str] = &["DRAWBUFFERS:", "RENDERTARGETS:", "const"];
/// The identifier a kept comment is replaced by while pp-rs runs.
const COMMENT_MARK: &str = "iris_comment_";

/// Replace the comments holding [`DIRECTIVE_WORDS`] with an identifier
/// (and their line breaks), so the ones in `#if`'d out blocks disappear with
/// them. Comments on directive lines are left alone: there the identifier
/// would end up in a macro or an `#if`.
fn mark_comments(text: &str) -> (String, Vec<String>) {
    let mut out = String::with_capacity(text.len());
    let mut comments = Vec::new();
    let (mut directive, mut line_start, mut continued) = (false, true, false);
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        let comment_len = if rest.starts_with("//") {
            rest.find('\n').unwrap_or(rest.len())
        } else if let Some(body) = rest.strip_prefix("/*") {
            body.find("*/").map_or(rest.len(), |i| i + 4)
        } else {
            0
        };
        if comment_len > 0 {
            let comment = &rest[..comment_len];
            if !directive && DIRECTIVE_WORDS.iter().any(|w| comment.contains(w)) {
                out.push_str(&format!(" {COMMENT_MARK}{} ", comments.len()));
                out.extend(std::iter::repeat_n('\n', comment.matches('\n').count()));
                comments.push(comment.to_owned());
            } else {
                out.push_str(comment);
            }
            if comment.contains('\n') {
                // A comment spanning lines ends a directive like a line break.
                directive = false;
            }
            line_start = false;
            rest = &rest[comment_len..];
            continue;
        }
        match c {
            '\n' => {
                directive = directive && continued;
                line_start = !directive;
            }
            '#' if line_start => directive = true,
            c if c.is_whitespace() => {}
            _ => line_start = false,
        }
        if c != '\r' && !c.is_whitespace() || c == '\n' {
            continued = c == '\\';
        }
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    (out, comments)
}

/// Expand `source` with `env`'s macros. Errors name the file and line.
pub fn preprocess(source: &Inlined, env: &Environment) -> Result<Preprocessed, String> {
    let (marked, comments) = mark_comments(&source.text);
    let text = &marked;
    let line_starts: Vec<usize> = std::iter::once(0)
        .chain(text.match_indices('\n').map(|(i, _)| i + 1))
        .collect();
    let line_of = |offset: u32| line_starts.partition_point(|&s| s <= offset as usize) as u32;
    let mut pp = Preprocessor::new(text);
    for (name, value) in &env.defines {
        pp.add_define(name, value)
            .map_err(|e| format!("macro {name}: {:?}", e.0))?;
    }
    let mut out = Preprocessed {
        text: String::new(),
        version: Vec::new(),
        extensions: Vec::new(),
        comments: Vec::new(),
    };
    let mut line = 1;
    for item in pp {
        let token =
            item.map_err(|(e, at)| format!("{}: {e:?}", source.locate(line_of(at.start))))?;
        let at = line_of(token.location.start);
        let piece = match token.value {
            TokenValue::Version(v) => {
                out.version = v.tokens.into_iter().map(|t| spelling(t.value)).collect();
                continue;
            }
            TokenValue::Extension(e) => {
                out.extensions
                    .push(e.tokens.into_iter().map(|t| spelling(t.value)).collect());
                continue;
            }
            TokenValue::Pragma(_) => continue,
            TokenValue::Ident(ref name) if name.starts_with(COMMENT_MARK) => {
                if let Some(comment) = name[COMMENT_MARK.len()..]
                    .parse::<usize>()
                    .ok()
                    .and_then(|i| comments.get(i))
                {
                    out.comments.push((at, comment.clone()));
                }
                continue;
            }
            value => spelling(value),
        };
        if at > line {
            out.text
                .extend(std::iter::repeat_n('\n', (at - line) as usize));
            line = at;
        } else if !out.text.is_empty() {
            out.text.push(' ');
        }
        out.text.push_str(&piece);
    }
    out.text.push('\n');
    Ok(out)
}

/// A token as GLSL source.
fn spelling(value: TokenValue) -> String {
    match value {
        TokenValue::Ident(s) => s,
        TokenValue::Integer(i) => {
            let suffix = if i.signed { "" } else { "u" };
            // Bit patterns such as 0xFFFFFFFF stay hexadecimal: as decimals
            // they would overflow a signed int.
            if i.value > i32::MAX as u64 {
                format!("{:#X}{suffix}", i.value)
            } else {
                format!("{}{suffix}", i.value)
            }
        }
        TokenValue::Float(f) => {
            let mut s = format!("{:?}", f.value);
            if !s.contains(['.', 'e', 'i', 'N']) {
                s.push_str(".0");
            }
            s
        }
        TokenValue::Punct(p) => punct(p).to_owned(),
        TokenValue::Version(_) | TokenValue::Extension(_) | TokenValue::Pragma(_) => String::new(),
    }
}

fn punct(p: Punct) -> &'static str {
    use Punct::*;
    match p {
        AddAssign => "+=",
        SubAssign => "-=",
        MulAssign => "*=",
        DivAssign => "/=",
        ModAssign => "%=",
        LeftShiftAssign => "<<=",
        RightShiftAssign => ">>=",
        AndAssign => "&=",
        XorAssign => "^=",
        OrAssign => "|=",
        Increment => "++",
        Decrement => "--",
        LogicalAnd => "&&",
        LogicalOr => "||",
        LogicalXor => "^^",
        LessEqual => "<=",
        GreaterEqual => ">=",
        EqualEqual => "==",
        NotEqual => "!=",
        LeftShift => "<<",
        RightShift => ">>",
        LeftBrace => "{",
        RightBrace => "}",
        LeftParen => "(",
        RightParen => ")",
        LeftBracket => "[",
        RightBracket => "]",
        LeftAngle => "<",
        RightAngle => ">",
        Semicolon => ";",
        Comma => ",",
        Colon => ":",
        Dot => ".",
        Equal => "=",
        Bang => "!",
        Minus => "-",
        Tilde => "~",
        Plus => "+",
        Star => "*",
        Slash => "/",
        Percent => "%",
        Pipe => "|",
        Caret => "^",
        Ampersand => "&",
        Question => "?",
    }
}
