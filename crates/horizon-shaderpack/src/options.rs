//! A pack's options, found in its source the way Iris finds them
//! (`OptionAnnotatedSource`, `ParsedString`, `OptionSet`), and the player's
//! values written back into the lines they came from.
//!
//! - `#define NAME` / `//#define NAME` (optionally `// comment`): a boolean,
//!   on unless commented out, and an option only if some `#ifdef NAME` /
//!   `#ifndef NAME` in the same include component refers to it.
//! - `#define NAME VALUE // comment [A B C]`: a value with its allowed list.
//! - `const int|float|bool NAME = VALUE; // [A B C]`: only for the constants
//!   Iris lets players configure ([`CONST_OPTIONS`]).

use std::collections::{BTreeMap, BTreeSet};

/// The `const` names Iris configures (`VALID_CONST_OPTION_NAMES`), apart from
/// the per-buffer ones [`is_const_option`] adds.
pub const CONST_OPTIONS: &[&str] = &[
    "shadowMapResolution",
    "shadowDistance",
    "voxelDistance",
    "shadowDistanceRenderMul",
    "entityShadowDistanceMul",
    "shadowIntervalSize",
    "generateShadowMipmap",
    "generateShadowColorMipmap",
    "shadowHardwareFiltering",
    "shadowtex0Mipmap",
    "shadowtexMipmap",
    "shadowtex1Mipmap",
    "shadowtex0Nearest",
    "shadowtexNearest",
    "shadow0MinMagNearest",
    "shadowtex1Nearest",
    "shadow1MinMagNearest",
    "wetnessHalflife",
    "drynessHalflife",
    "eyeBrightnessHalflife",
    "centerDepthHalflife",
    "sunPathRotation",
    "ambientOcclusionLevel",
    "superSamplingLevel",
    "noiseTextureResolution",
];

/// Iris's shadow colour buffer count (`MAX_SHADOW_COLOR_BUFFERS_IRIS`).
const SHADOW_COLOR_BUFFERS: u32 = 8;

pub fn is_const_option(name: &str) -> bool {
    CONST_OPTIONS.contains(&name)
        || (0..SHADOW_COLOR_BUFFERS).any(|i| {
            [
                format!("shadowcolor{i}Mipmap"),
                format!("shadowColor{i}Mipmap"),
                format!("shadowcolor{i}Nearest"),
                format!("shadowColor{i}Nearest"),
                format!("shadowcolor{i}MinMagNearest"),
                format!("shadowColor{i}MinMagNearest"),
                format!("shadowHardwareFiltering{i}"),
            ]
            .iter()
            .any(|n| n == name)
        })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptionKind {
    Define,
    Const,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BoolOption {
    pub kind: OptionKind,
    pub name: String,
    pub comment: Option<String>,
    pub default: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StringOption {
    pub kind: OptionKind,
    pub name: String,
    pub comment: Option<String>,
    pub default: String,
    /// The bracketed list, with the default added at the end when missing.
    pub allowed: Vec<String>,
}

impl StringOption {
    /// `StringOption.create`: `None` without a `[...]` list in the comment.
    fn create(kind: OptionKind, name: &str, comment: Option<&str>, default: &str) -> Option<Self> {
        let comment = comment?;
        let open = comment.find('[')?;
        let close = open + comment[open..].find(']')?;
        // Java's split(" "): empty strings between doubled spaces stay,
        // trailing ones go.
        let mut allowed: Vec<String> = comment[open + 1..close]
            .split(' ')
            .map(str::to_owned)
            .collect();
        while allowed.last().is_some_and(String::is_empty) {
            allowed.pop();
        }
        if !allowed.iter().any(|v| v == default) {
            allowed.push(default.to_owned());
        }
        let rest = format!("{}{}", &comment[..open], &comment[close + 1..]);
        Some(Self {
            kind,
            name: name.to_owned(),
            comment: non_empty(&rest),
            default: default.to_owned(),
            allowed,
        })
    }
}

/// An option's comment; empty is none (`BaseOption`).
fn non_empty(comment: &str) -> Option<String> {
    let c = comment.trim();
    (!c.is_empty()).then(|| c.to_owned())
}

/// Iris's `ParsedString`: a cursor over a trimmed line.
struct Cursor(Vec<char>);

impl Cursor {
    fn new(s: &str) -> Self {
        Self(s.chars().collect())
    }
    fn literal(&mut self, token: &str) -> bool {
        let t: Vec<char> = token.chars().collect();
        if self.0.starts_with(&t) {
            self.0.drain(..t.len());
            true
        } else {
            false
        }
    }
    /// Whitespace first, then trims both ends (as Iris does).
    fn whitespace(&mut self) -> bool {
        if !self.0.first().is_some_and(|c| c.is_whitespace()) {
            return false;
        }
        let s: String = self.0.iter().collect();
        self.0 = s.trim().chars().collect();
        true
    }
    /// `//` and any further slashes.
    fn comments(&mut self) -> bool {
        if !self.literal("//") {
            return false;
        }
        while self.literal("/") {}
        true
    }
    fn contains(&self, s: &str) -> bool {
        self.rest().contains(s)
    }
    fn is_end(&self) -> bool {
        self.0.is_empty()
    }
    fn rest(&self) -> String {
        self.0.iter().collect()
    }
    fn take(&mut self, n: usize) -> String {
        self.0.drain(..n).collect()
    }
    fn word(&mut self) -> Option<String> {
        let n = self
            .0
            .iter()
            .take_while(|c| c.is_alphanumeric() || **c == '_')
            .count();
        (n > 0).then(|| self.take(n))
    }
    /// Iris's `takeNumber`: digits and any character next to a digit, an `f`
    /// suffix, and the whole must parse as a Java float.
    fn number(&mut self) -> Option<String> {
        let c = &self.0;
        let mut n = 0;
        while n < c.len() {
            if n + 1 < c.len() {
                if !c[n].is_ascii_digit() && !c[n + 1].is_ascii_digit() {
                    break;
                }
            } else if !c[n].is_ascii_digit() {
                break;
            }
            n += 1;
        }
        if n > 0 && n + 1 < c.len() && (c[n] == 'f' || c[n] == 'F') {
            n += 1;
        }
        let text: String = c[..n].iter().collect();
        java_float(&text).then(|| self.take(n))
    }
    fn word_or_number(&mut self) -> Option<String> {
        self.number().or_else(|| self.word())
    }
}

/// Whether Java's `Float.parseFloat` accepts `s` (decimal forms; a trailing
/// `f`/`d` is allowed).
fn java_float(s: &str) -> bool {
    let s = s.trim();
    let s = s.strip_suffix(['f', 'F', 'd', 'D']).unwrap_or(s);
    !s.is_empty() && s.parse::<f64>().is_ok()
}

/// The options and `#ifdef` references of one file, by line index.
#[derive(Default, Debug)]
pub struct Annotated {
    pub bools: BTreeMap<usize, BoolOption>,
    pub strings: BTreeMap<usize, StringOption>,
    /// Names tested by a bare `#ifdef NAME` / `#ifndef NAME` line.
    pub ifdef_refs: BTreeSet<String>,
    /// Why a line that looks like an option isn't one.
    pub diagnostics: BTreeMap<usize, String>,
}

impl Annotated {
    pub fn parse(lines: &[String]) -> Self {
        let mut out = Self::default();
        for (i, line) in lines.iter().enumerate() {
            if !["#define", "const", "#ifdef", "#ifndef"]
                .iter()
                .any(|k| line.contains(k))
            {
                continue;
            }
            let mut c = Cursor::new(line.trim());
            if c.literal("#ifdef") || c.literal("#ifndef") {
                if !c.whitespace() {
                    continue;
                }
                let name = c.word();
                c.whitespace();
                if let (Some(name), true) = (name, c.is_end()) {
                    out.ifdef_refs.insert(name);
                }
            } else if c.literal("const") {
                if let Err(why) = out.parse_const(i, &mut c) {
                    out.diagnostics.insert(i, why.into());
                }
            } else if c.contains("#define") {
                if let Err(why) = out.parse_define(i, &mut c) {
                    out.diagnostics.insert(i, why.into());
                }
            }
        }
        out
    }

    fn parse_const(&mut self, i: usize, c: &mut Cursor) -> Result<(), &'static str> {
        if !c.whitespace() {
            return Err("expected whitespace after const");
        }
        let is_bool = if c.literal("int") || c.literal("float") {
            false
        } else if c.literal("bool") {
            true
        } else {
            return Err("only int, float and bool consts are options");
        };
        if !c.whitespace() {
            return Err("expected whitespace after the type");
        }
        let name = c.word().ok_or("expected a name after the type")?;
        c.whitespace();
        if !c.literal("=") {
            return Err("expected = after the name");
        }
        c.whitespace();
        let value = c
            .word_or_number()
            .ok_or("expected a word or number after =")?;
        c.whitespace();
        if !c.literal(";") {
            return Err("expected ; after the value");
        }
        c.whitespace();
        let comment = if c.comments() {
            non_empty(&c.rest())
        } else if !c.is_end() {
            return Err("only a comment may follow the ;");
        } else {
            None
        };
        if !is_const_option(&name) {
            return Err("not one of the configurable consts");
        }
        if is_bool {
            let default = match value.as_str() {
                "true" => true,
                "false" => false,
                _ => return Err("a bool const option must be true or false"),
            };
            self.bools.insert(
                i,
                BoolOption {
                    kind: OptionKind::Const,
                    name,
                    comment,
                    default,
                },
            );
            return Ok(());
        }
        let option = StringOption::create(OptionKind::Const, &name, comment.as_deref(), &value)
            .ok_or("a const option needs a [...] list of values in its comment")?;
        self.strings.insert(i, option);
        Ok(())
    }

    fn parse_define(&mut self, i: usize, c: &mut Cursor) -> Result<(), &'static str> {
        let commented = c.comments();
        c.whitespace();
        if !c.literal("#define") {
            return Err("#define isn't where an option's would be");
        }
        if !c.whitespace() {
            return Err("expected whitespace after #define");
        }
        let name = c.word().ok_or("expected a name after #define")?;
        let mut spaced = c.whitespace();
        if c.is_end() {
            self.bools.insert(
                i,
                BoolOption {
                    kind: OptionKind::Define,
                    name,
                    comment: None,
                    default: !commented,
                },
            );
            return Ok(());
        }
        if c.comments() {
            let comment = non_empty(&c.rest());
            self.bools.insert(
                i,
                BoolOption {
                    kind: OptionKind::Define,
                    name,
                    comment,
                    default: !commented,
                },
            );
            return Ok(());
        }
        if !spaced {
            return Err("only letters, digits and _ may be in an option's name");
        }
        if commented {
            return Err("only boolean #define options may be commented out");
        }
        let value = c
            .word_or_number()
            .ok_or("the value is neither a number nor a word")?;
        spaced = c.whitespace();
        if c.is_end() {
            return Err("a value #define needs a comment with a [...] list of values");
        }
        if !c.comments() {
            return Err(if spaced {
                "only a comment may follow the value"
            } else {
                "unexpected characters after the value"
            });
        }
        let comment = c.rest().trim().to_owned();
        let option = StringOption::create(OptionKind::Define, &name, Some(&comment), &value)
            .ok_or("a value #define needs a [...] list of values in its comment")?;
        self.strings.insert(i, option);
        Ok(())
    }

    /// Line `i` (text `line`) with the player's values applied
    /// (`OptionAnnotatedSource.edit`).
    pub fn edit(&self, values: &OptionValues, i: usize, line: &str) -> String {
        if let Some(o) = self.bools.get(&i) {
            let value = values.bools.get(&o.name).copied();
            return match o.kind {
                OptionKind::Define => {
                    let on = value.unwrap_or(o.default);
                    let commented = line.trim().starts_with("//");
                    if commented && on {
                        let mut c = Cursor::new(line);
                        c.whitespace();
                        c.comments();
                        c.rest()
                    } else if !on {
                        format!("//{line}")
                    } else {
                        line.to_owned()
                    }
                }
                OptionKind::Const => match value {
                    Some(v) => edit_const(line, &o.default.to_string(), &v.to_string()),
                    None => line.to_owned(),
                },
            };
        }
        if let Some(o) = self.strings.get(&i) {
            if let Some(v) = values.strings.get(&o.name) {
                return match o.kind {
                    OptionKind::Define => format!(
                        "#define {} {v} // OptionAnnotatedSource: Changed option",
                        o.name
                    ),
                    OptionKind::Const => edit_const(line, &o.default, v),
                };
            }
        }
        line.to_owned()
    }
}

/// The first `current` after the `=` replaced by `new`.
fn edit_const(line: &str, current: &str, new: &str) -> String {
    match line.find('=') {
        Some(eq) => format!("{}{}", &line[..eq], line[eq..].replacen(current, new, 1)),
        None => line.to_owned(),
    }
}

/// Every option of a pack by name. Two definitions with different defaults
/// make the name ambiguous: Iris drops it.
#[derive(Default, Debug)]
pub struct OptionSet {
    pub bools: BTreeMap<String, BoolOption>,
    pub strings: BTreeMap<String, StringOption>,
}

impl OptionSet {
    pub fn add_bool(&mut self, o: &BoolOption) {
        match self.bools.get_mut(&o.name) {
            Some(existing) if existing.default != o.default => {
                self.bools.remove(&o.name);
            }
            Some(existing) => {
                if existing.comment.is_none() {
                    *existing = o.clone();
                }
            }
            None => {
                self.bools.insert(o.name.clone(), o.clone());
            }
        }
    }

    pub fn add_string(&mut self, o: &StringOption) {
        match self.strings.get_mut(&o.name) {
            Some(existing) if existing.default != o.default => {
                self.strings.remove(&o.name);
            }
            Some(existing) => {
                if existing.comment.is_none() {
                    *existing = o.clone();
                }
            }
            None => {
                self.strings.insert(o.name.clone(), o.clone());
            }
        }
    }
}

/// The player's values that differ from the defaults (`MutableOptionValues`).
#[derive(Default, Debug, Clone, PartialEq)]
pub struct OptionValues {
    pub bools: BTreeMap<String, bool>,
    pub strings: BTreeMap<String, String>,
}

impl OptionValues {
    /// From saved `name=value` pairs. A boolean other than `true`/`false` is
    /// ignored; a value outside an option's allowed list is kept (profiles and
    /// hand-edited files may set one, as in OptiFine).
    pub fn new(options: &OptionSet, saved: &BTreeMap<String, String>) -> Self {
        let mut out = Self::default();
        for (name, o) in &options.bools {
            let value = match saved.get(name).map(String::as_str) {
                Some("true") => true,
                Some("false") => false,
                _ => continue,
            };
            if value != o.default {
                out.bools.insert(name.clone(), value);
            }
        }
        // A saved value the option doesn't list (an old or hand-edited
        // file) is dropped, as OptiFine's `ShaderOption.setValue` refuses it.
        for (name, o) in &options.strings {
            if let Some(v) = saved.get(name)
                && *v != o.default
                && o.allowed.contains(v)
            {
                out.strings.insert(name.clone(), v.clone());
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(line: &str) -> Annotated {
        Annotated::parse(&[line.to_owned()])
    }

    fn string(line: &str) -> Option<(String, String, Vec<String>)> {
        one(line)
            .strings
            .into_values()
            .next()
            .map(|o| (o.name, o.default, o.allowed))
    }

    #[test]
    fn value_defines_need_an_allowed_list() {
        // Iris's OptionDiscoverTest cases.
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(
            string("#define SECRET 0 //[0]"),
            Some(("SECRET".into(), "0".into(), s(&["0"])))
        );
        assert_eq!(
            string("#define SECRET 0//[0]"),
            Some(("SECRET".into(), "0".into(), s(&["0"])))
        );
        assert_eq!(
            string("#define OPT 1 //[0]"),
            Some(("OPT".into(), "1".into(), s(&["0", "1"])))
        );
        for none in [
            "#define PI 3.14 // It's PI.",
            "#define PI 3.14",
            "const int noiseTextureResolution = 512;",
            "const int noiseTextureResolution = 512; // Default noise texture size [",
        ] {
            let a = one(none);
            assert!(a.strings.is_empty() && a.bools.is_empty(), "{none}");
        }
        assert_eq!(
            string("  #define SHADOW_QUALITY 2.0f // Shadows [1.0f 2.0f 3.0f]"),
            Some((
                "SHADOW_QUALITY".into(),
                "2.0f".into(),
                s(&["1.0f", "2.0f", "3.0f"])
            ))
        );
        assert_eq!(
            string("#define SKY_STYLE SKY_REALISTIC // [SKY_REALISTIC SKY_STYLISED]")
                .unwrap()
                .1,
            "SKY_REALISTIC"
        );
        assert_eq!(
            string("const int shadowMapResolution = 2048; // [1024 2048 4096]")
                .unwrap()
                .0,
            "shadowMapResolution"
        );
        assert!(string("const int myResolution = 2048; // [1024 2048 4096]").is_none());
        assert!(string("//#define SHADOW_QUALITY 2 // [1 2 3]").is_none());
    }

    #[test]
    fn boolean_defines_and_references() {
        let a = Annotated::parse(&[
            "#define BLOOM // Bloom".into(),
            "//#define DOF".into(),
            "// #define MOTION_BLUR // Blur".into(),
            "#define FOG(x) x".into(),
            "#ifdef BLOOM".into(),
            "#ifndef DOF // trailing text: not a reference".into(),
            "#if defined MOTION_BLUR".into(),
            "const bool shadowHardwareFiltering = true; // Filtering".into(),
        ]);
        let b: Vec<(&str, bool, Option<&str>)> = a
            .bools
            .values()
            .map(|o| (o.name.as_str(), o.default, o.comment.as_deref()))
            .collect();
        assert_eq!(
            b,
            [
                ("BLOOM", true, Some("Bloom")),
                ("DOF", false, None),
                ("MOTION_BLUR", false, Some("Blur")),
                ("shadowHardwareFiltering", true, Some("Filtering"))
            ]
        );
        assert_eq!(a.ifdef_refs.iter().collect::<Vec<_>>(), ["BLOOM"]);
    }

    #[test]
    fn values_are_written_into_the_lines() {
        let lines: Vec<String> = [
            "#define BLOOM",
            "//#define DOF",
            "#define QUALITY 2 // [1 2 3]",
            "const float sunPathRotation = -40.0; // [-40.0 0.0 40.0]",
            "const bool shadowHardwareFiltering = false;",
        ]
        .map(String::from)
        .into();
        let a = Annotated::parse(&lines);
        let mut set = OptionSet::default();
        a.bools.values().for_each(|o| set.add_bool(o));
        a.strings.values().for_each(|o| set.add_string(o));
        let saved: BTreeMap<String, String> = [
            ("BLOOM", "false"),
            ("DOF", "true"),
            ("QUALITY", "3"),
            ("sunPathRotation", "0.0"),
            ("shadowHardwareFiltering", "yes"),
        ]
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .into();
        let values = OptionValues::new(&set, &saved);
        assert!(
            !values.bools.contains_key("shadowHardwareFiltering"),
            "not a boolean: ignored"
        );
        let stale = [("QUALITY", "abc"), ("sunPathRotation", "")]
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .into();
        assert!(
            OptionValues::new(&set, &stale).strings.is_empty(),
            "values the options don't list: ignored"
        );
        let edited: Vec<String> = lines
            .iter()
            .enumerate()
            .map(|(i, l)| a.edit(&values, i, l))
            .collect();
        assert_eq!(
            edited,
            [
                "//#define BLOOM",
                "#define DOF",
                "#define QUALITY 3 // OptionAnnotatedSource: Changed option",
                "const float sunPathRotation = 0.0; // [-40.0 0.0 40.0]",
                "const bool shadowHardwareFiltering = false;",
            ]
        );
    }

    #[test]
    fn conflicting_defaults_drop_the_option() {
        let mut set = OptionSet::default();
        let a = Annotated::parse(&["#define Q 1 // [1 2]".into()]);
        let b = Annotated::parse(&["#define Q 2 // Quality [1 2]".into()]);
        let c = Annotated::parse(&["#define Q 1 // Quality [1 2]".into()]);
        set.add_string(&a.strings[&0]);
        set.add_string(&c.strings[&0]);
        assert_eq!(
            set.strings["Q"].comment.as_deref(),
            Some("Quality"),
            "the commented one wins"
        );
        set.add_string(&b.strings[&0]);
        assert!(set.strings.is_empty());
    }
}
