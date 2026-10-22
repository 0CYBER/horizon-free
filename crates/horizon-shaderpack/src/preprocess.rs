//! The C preprocessor pass: Iris's standard macros (`StandardMacros`,
//! `IrisDefines`) defined, then every directive and macro expanded by pp-rs,
//! the preprocessor naga's GLSL front end uses. The output keeps the inlined
//! source's line numbers, so an error later on still names `file:line`.

use crate::include::Inlined;
use pp_rs::{
    pp::Preprocessor,
    token::{PreprocessorError, Punct, TokenValue},
};
use std::collections::HashMap;

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

/// Whether trimmed `line` is an `#if` or `#elif` directive.
pub(crate) fn is_condition(line: &str) -> bool {
    line.strip_prefix('#')
        .map(str::trim_start)
        .and_then(|d| d.strip_prefix("elif").or_else(|| d.strip_prefix("if")))
        .is_some_and(|r| !r.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_'))
}

/// Whether `word` is a float literal.
pub(crate) fn is_float(word: &str) -> bool {
    word.starts_with(|c: char| c.is_ascii_digit() || c == '.')
        && !word.starts_with("0x")
        && !word.starts_with("0X")
        && word.contains(['.', 'e', 'E'])
}

/// What JCPP's `#if` makes of a number (`Preprocessor.expr` takes
/// `NumericValue.longValue`): a float counts as its integer part, so
/// `0.75` is 0 and `1.5e1` is 10. `None` for a non-number.
pub(crate) fn jcpp_integer(number: &str) -> Option<i64> {
    let number = number.trim_end_matches(['f', 'F', 'l', 'L', 'u', 'U']);
    let (mantissa, exponent) = match number.find(['e', 'E']) {
        Some(i) => (&number[..i], number[i + 1..].parse::<i32>().ok()?),
        None => (number, 0),
    };
    let integer = mantissa.split('.').next()?;
    let value = if integer.is_empty() {
        0
    } else {
        integer.parse::<i64>().ok()?
    };
    Some((value as f64 * 10f64.powi(exponent)) as i64)
}

/// `line` with each word (identifier or number, with an exponent's sign:
/// `1e-3`) replaced by what `f` returns for it, if anything. The directive's
/// name and the names after `defined` are kept.
pub(crate) fn rewrite_condition(line: &str, f: impl Fn(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(line.len());
    let (mut rest, mut first, mut after_defined) = (line, true, false);
    while let Some(c) = rest.chars().next() {
        if !(c.is_ascii_alphanumeric() || c == '_' || c == '.') {
            if !c.is_whitespace() && c != '(' && c != '#' {
                after_defined = false;
            }
            out.push(c);
            rest = &rest[c.len_utf8()..];
            continue;
        }
        let bytes = rest.as_bytes();
        let mut len = 0;
        while let Some(&b) = bytes.get(len) {
            let sign = (b == b'+' || b == b'-')
                && bytes[0].is_ascii_digit()
                && matches!(bytes[len - 1], b'e' | b'E');
            if !(b.is_ascii_alphanumeric() || b == b'_' || b == b'.' || sign) {
                break;
            }
            len += 1;
        }
        let word = &rest[..len];
        let kept = first || after_defined;
        match (!kept).then(|| f(word)).flatten() {
            Some(replacement) => out.push_str(&replacement),
            None => out.push_str(word),
        }
        after_defined = word == "defined";
        first = false;
        rest = &rest[len..];
    }
    out
}

/// Make pp-rs read `text` as JCPP, Iris's preprocessor, does:
/// - a `#define` replaces an earlier macro (`Preprocessor.addMacro`; Bloop
///   and Steadfast redefine theirs), where pp-rs refuses a different
///   redefinition: an `#undef` line goes before every `#define`;
/// - in `#if`/`#elif`, a float counts as its integer part, and so does a
///   macro always defined as the same float (an option: `#define LEVEL 0.5`);
///   a macro always defined empty (Iris's `MC_GL_VENDOR_OTHER`, Bloop's
///   `#define TAA_USE_YCOCG`) counts as 0: JCPP reports the missing operand
///   and goes on with 0. pp-rs stops on both;
/// - `#error "..."` (Photon's guard in programs it disables) lexes: pp-rs
///   has no string tokens and fails on the quote, even in a skipped block.
///
/// Returns the text and the added lines' 1-based numbers in it.
fn jcpp_compatible(text: &str, env: &Environment) -> (String, Vec<u32>) {
    // A `#define` line's name and body.
    let define = |l: &str| {
        let rest = l
            .trim_start()
            .strip_prefix('#')?
            .trim_start()
            .strip_prefix("define")
            .filter(|rest| rest.starts_with([' ', '\t']))?
            .trim_start();
        let end = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(rest.len());
        let (name, body) = rest.split_at(end);
        let body = body.split("//").next().unwrap_or("").trim();
        (!name.is_empty()).then(|| (name.to_owned(), body.to_owned()))
    };
    // Each macro's body, if every definition has the same one.
    let mut bodies: HashMap<String, Option<String>> = HashMap::new();
    let definitions = env
        .defines
        .iter()
        .cloned()
        .chain(text.lines().filter_map(define));
    for (name, body) in definitions {
        bodies
            .entry(name)
            .and_modify(|b| {
                if b.as_ref() != Some(&body) {
                    *b = None;
                }
            })
            .or_insert(Some(body));
    }
    let number = |word: &str| -> Option<String> {
        let word = match bodies.get(word) {
            Some(Some(body)) if body.is_empty() => return Some("0".into()),
            Some(Some(body)) => body.as_str(),
            Some(None) => return None,
            None => word,
        };
        is_float(word)
            .then(|| jcpp_integer(word))
            .flatten()
            .map(|n| n.to_string())
    };
    let mut out = String::with_capacity(text.len() + text.len() / 16);
    let mut added = Vec::new();
    let (mut line, mut continued) = (1, false);
    for l in text.split_inclusive('\n') {
        if !continued && let Some((name, _)) = define(l) {
            out.push_str(&format!("#undef {name}\n"));
            added.push(line);
            line += 1;
        }
        let directive = l.trim_start().strip_prefix('#').map(str::trim_start);
        if !continued
            && directive.is_some_and(|d| d.starts_with("error") || d.starts_with("warning"))
        {
            out.push_str(&l.replace(['"', '\''], ""));
        } else if !continued && is_condition(l.trim_start()) {
            out.push_str(&rewrite_condition(l, number));
        } else {
            out.push_str(l);
        }
        continued = l.trim_end().ends_with('\\');
        line += 1;
    }
    (out, added)
}

/// Expand `source` with `env`'s macros. Errors name the file and line.
pub fn preprocess(source: &Inlined, env: &Environment) -> Result<Preprocessed, String> {
    let (marked, comments) = mark_comments(&source.text);
    let (text, added) = jcpp_compatible(&marked, env);
    let text = &text;
    let line_starts: Vec<usize> = std::iter::once(0)
        .chain(text.match_indices('\n').map(|(i, _)| i + 1))
        .collect();
    // Lines of the source: the added `#undef` lines don't count.
    let line_of = |offset: u32| {
        let line = line_starts.partition_point(|&s| s <= offset as usize) as u32;
        line - added.partition_point(|&a| a < line) as u32
    };
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
        let token = match item {
            // JCPP lets an `#if` run to the end of the input (Bloop's
            // taa.fsh never closes its `#ifdef TAA`); pp-rs says so there,
            // after the last token.
            Err((PreprocessorError::UnfinishedBlock, _)) => break,
            item => {
                item.map_err(|(e, at)| format!("{}: {e:?}", source.locate(line_of(at.start))))?
            }
        };
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
