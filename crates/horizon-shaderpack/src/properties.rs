//! `shaders.properties`, read the way Iris reads it (`ShaderProperties`,
//! `PropertiesPreprocessor`): the option screens, sliders, profiles and
//! feature flags come from the file as written; everything else from the file
//! after the C preprocessor, run with Iris's macros and the player's options
//! (an enabled boolean option is defined as `1`, a value option as its value),
//! so `#if` blocks pick the directives that apply.

use crate::options::{OptionSet, OptionValues};
use crate::preprocess::{Environment, is_condition, is_float, jcpp_integer, rewrite_condition};
use pp_rs::{pp::Preprocessor, token::TokenValue};
use std::collections::BTreeMap;

/// A Java `.properties` file: keys in the order they first appear, the last
/// value of a repeated key kept (Iris's `OrderBackedProperties`).
#[derive(Debug, Default, Clone)]
pub struct Properties(Vec<(String, String)>);

impl Properties {
    /// `java.util.Properties.load`: `#`/`!` comment lines, `key=value`,
    /// `key:value` or `key value`, backslash continuations and escapes.
    pub fn parse(text: &str) -> Self {
        let mut out = Self::default();
        let mut lines = text.lines();
        while let Some(first) = lines.next() {
            let mut line = first.trim_start().to_owned();
            if line.is_empty() || line.starts_with('#') || line.starts_with('!') {
                continue;
            }
            while ends_with_odd_backslashes(&line) {
                line.pop();
                match lines.next() {
                    Some(next) => line.push_str(next.trim_start()),
                    None => break,
                }
            }
            let (key, value) = split_entry(&line);
            out.set(unescape(&key), unescape(&value));
        }
        out
    }

    fn set(&mut self, key: String, value: String) {
        match self.0.iter_mut().find(|(k, _)| *k == key) {
            Some(entry) => entry.1 = value,
            None => self.0.push((key, value)),
        }
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }
}

fn ends_with_odd_backslashes(s: &str) -> bool {
    s.chars().rev().take_while(|&c| c == '\\').count() % 2 == 1
}

/// The key ends at the first unescaped `=`, `:` or whitespace; then
/// whitespace and at most one `=` or `:` separate it from the value.
fn split_entry(line: &str) -> (String, String) {
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 2,
            '=' | ':' => break,
            c if c.is_whitespace() => break,
            _ => i += 1,
        }
    }
    let i = i.min(chars.len());
    let key: String = chars[..i].iter().collect();
    let mut j = i;
    while j < chars.len() && chars[j].is_whitespace() {
        j += 1;
    }
    if j < chars.len() && (chars[j] == '=' || chars[j] == ':') {
        j += 1;
    }
    while j < chars.len() && chars[j].is_whitespace() {
        j += 1;
    }
    (key, chars[j..].iter().collect())
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('f') => out.push('\u{c}'),
            Some('u') => {
                let hex: String = chars.by_ref().take(4).collect();
                out.extend(u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32));
            }
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

/// The directives the C preprocessor knows: other `#` lines are comments.
const DIRECTIVES: &[&str] = &[
    "#define", "#undef", "#ifdef", "#ifndef", "#if", "#elif", "#else", "#endif", "#error",
    "#warning", "#pragma", "#line", "#include",
];

/// An `#if`/`#elif` line with its floats as JCPP reads them (Steadfast tests
/// `#if ATMOSPHERE_FOG_STRENGTH_NOON == 0.75` with the option at 0.5, which
/// Iris reads as `0 == 0`): float literals and the options set to a float
/// become their integer part.
fn jcpp_condition(line: &str, options: &OptionSet, values: &OptionValues) -> String {
    rewrite_condition(line, |word| {
        let value = match options.strings.get(word) {
            Some(o) => values.strings.get(word).unwrap_or(&o.default).as_str(),
            None => word,
        };
        is_float(value)
            .then(|| jcpp_integer(value))
            .flatten()
            .map(|n| n.to_string())
    })
}

/// What backslashes are while the macros expand, as in Iris: the
/// preprocessor must not join a `\`-continued value with the directive on
/// the next line (Photon ends an `#if`'d value with `) \` before `#else`).
const BACKSLASH: &str = "IRIS_PASSTHROUGHBACKSLASH";

/// `PropertiesPreprocessor.preprocessSource`: comment lines dropped, other
/// `#`s removed, then the macros expanded with the backslashes hidden.
/// Tokens keep their source spelling and spacing (`0.50` stays `0.50`,
/// `<empty>` stays one word).
pub fn preprocess(
    text: &str,
    env: &Environment,
    options: &OptionSet,
    values: &OptionValues,
) -> Result<String, String> {
    let filtered: String = text
        .lines()
        .map(str::trim)
        .map(|l| match l.starts_with('#') {
            // JCPP lets a #define replace a macro (Complementary sets
            // COLORED_LIGHTING 0 on macOS); pp-rs refuses, so undefine first.
            true if l.starts_with("#define") => {
                match l["#define".len()..].split_whitespace().next() {
                    Some(name) => format!("#undef {}\n{l}", name.split('(').next().unwrap_or(name)),
                    None => l.to_owned(),
                }
            }
            true if is_condition(l) => jcpp_condition(l, options, values),
            true if DIRECTIVES.iter().any(|d| l.starts_with(d)) => l.to_owned(),
            true => String::new(),
            false => l.replace('#', ""),
        })
        .map(|l| l.replace('\\', BACKSLASH) + "\n")
        .collect();
    let mut pp = Preprocessor::new(&filtered);
    let mut define = |name: &str, value: &str| {
        pp.add_define(name, value)
            .map_err(|e| format!("{name}: {:?}", e.0))
    };
    for (name, o) in &options.bools {
        if values.bools.get(name).copied().unwrap_or(o.default) {
            define(name, "1")?;
        }
    }
    for (name, value) in &env.defines {
        define(name, if value.is_empty() { "1" } else { value })?;
    }
    for (name, o) in &options.strings {
        define(name, values.strings.get(name).unwrap_or(&o.default))?;
    }
    let line_starts: Vec<usize> = std::iter::once(0)
        .chain(filtered.match_indices('\n').map(|(i, _)| i + 1))
        .collect();
    let line_of = |offset: u32| line_starts.partition_point(|&s| s <= offset as usize);
    let mut out = String::new();
    let (mut line, mut last_end) = (0, None::<u32>);
    for item in pp {
        let token =
            item.map_err(|(e, at)| format!("shaders.properties:{}: {e:?}", line_of(at.start)))?;
        let (start, end) = (token.location.start, token.location.end);
        let at = line_of(start);
        if at > line {
            if line > 0 {
                out.push('\n');
            }
            line = at;
        } else if last_end != Some(start) {
            out.push(' ');
        }
        last_end = Some(end);
        let source = filtered.get(start as usize..end as usize).unwrap_or("");
        out.push_str(&match token.value {
            TokenValue::Ident(s) => s,
            TokenValue::Integer(i) if source.parse::<u64>().ok() == Some(i.value) => {
                source.to_owned()
            }
            TokenValue::Integer(i) => i.value.to_string(),
            TokenValue::Float(f)
                if source.trim_end_matches(['f', 'F']).parse::<f32>().ok() == Some(f.value) =>
            {
                source.to_owned()
            }
            TokenValue::Float(f) => f.value.to_string(),
            TokenValue::Punct(_) => source.to_owned(),
            TokenValue::Version(_) | TokenValue::Extension(_) | TokenValue::Pragma(_) => continue,
        });
    }
    out.push('\n');
    Ok(out.replace(BACKSLASH, "\\"))
}

/// One element of an option screen (`OptionMenuElement`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuElement {
    /// `<empty>`: a blank cell.
    Empty,
    /// `<profile>`: the profile button (only when the pack has profiles).
    Profile,
    /// `*`: every option no screen lists.
    Rest,
    /// `[NAME]`: a button to sub-screen `screen.NAME`.
    Screen(String),
    /// An option, by name. A name the source doesn't define is shown as an
    /// empty cell, as Iris does (packs keep removed options on screens).
    Option(String),
}

impl MenuElement {
    fn parse(word: &str) -> Self {
        match word {
            "<empty>" => Self::Empty,
            "<profile>" => Self::Profile,
            "*" => Self::Rest,
            w if w.starts_with('[') && w.ends_with(']') && w.len() > 1 => {
                Self::Screen(w[1..w.len() - 1].to_owned())
            }
            w => Self::Option(w.to_owned()),
        }
    }
}

/// A named set of option values (`profile.LOW = SHADOWS:1 !BLOOM`).
#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    pub name: String,
    pub values: BTreeMap<String, String>,
    pub disabled_programs: Vec<String>,
}

impl Profile {
    /// Whether the current values are this profile's (`Profile.matches`).
    pub fn matches(&self, options: &OptionSet, values: &OptionValues) -> bool {
        self.values.iter().all(|(name, value)| {
            if let Some(o) = options.bools.get(name) {
                let on = values.bools.get(name).copied().unwrap_or(o.default);
                return on.to_string() == *value;
            }
            if let Some(o) = options.strings.get(name) {
                return values.strings.get(name).unwrap_or(&o.default) == value;
            }
            true
        })
    }
}

/// A blend override: `off`, or the four GL factors (source and
/// destination colour, then alpha), as written (`SRC_ALPHA`, ...).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Blend {
    Off,
    Factors([String; 4]),
}

/// The GL blend factors packs may name (`BlendModeFunction`).
pub const BLEND_FACTORS: &[&str] = &[
    "ZERO",
    "ONE",
    "SRC_COLOR",
    "ONE_MINUS_SRC_COLOR",
    "DST_COLOR",
    "ONE_MINUS_DST_COLOR",
    "SRC_ALPHA",
    "ONE_MINUS_SRC_ALPHA",
    "DST_ALPHA",
    "ONE_MINUS_DST_ALPHA",
    "SRC_ALPHA_SATURATE",
];

/// Colour targets by their pre-`colortex` names (`gcolor` is `colortex0`).
pub const LEGACY_TARGETS: &[&str] = &[
    "gcolor",
    "gdepth",
    "gnormal",
    "composite",
    "gaux1",
    "gaux2",
    "gaux3",
    "gaux4",
];

fn parse_blend(value: &str) -> Result<Blend, String> {
    if value == "off" {
        return Ok(Blend::Off);
    }
    let words: Vec<&str> = value.split(' ').collect();
    match words.as_slice() {
        [a, b, c, d] if words.iter().all(|w| BLEND_FACTORS.contains(w)) => {
            Ok(Blend::Factors([a, b, c, d].map(|w| w.to_string())))
        }
        _ => Err(format!("invalid blend mode `{value}`")),
    }
}

/// `clouds = off | fast | fancy`: replaces the player's cloud setting while
/// the pack runs (`ShaderProperties.cloudSetting`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloudSetting {
    Off,
    Fast,
    Fancy,
}

/// The vanilla sky the pack keeps: Iris draws no clouds with `clouds = off`,
/// and no sun or moon with `sun = false` / `moon = false`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SkyDirectives {
    pub clouds: Option<CloudSetting>,
    pub sun: bool,
    pub moon: bool,
}

impl Default for SkyDirectives {
    fn default() -> Self {
        Self {
            clouds: None,
            sun: true,
            moon: true,
        }
    }
}

/// A pack's `shaders.properties`.
#[derive(Debug, Default)]
pub struct ShaderProperties {
    /// As written: screens, sliders, profiles, feature flags.
    pub original: Properties,
    /// After the preprocessor: every other directive.
    pub preprocessed: Properties,
}

impl ShaderProperties {
    pub fn new(
        text: &str,
        env: &Environment,
        options: &OptionSet,
        values: &OptionValues,
    ) -> Result<Self, String> {
        Ok(Self {
            original: Properties::parse(text),
            preprocessed: Properties::parse(&preprocess(text, env, options, values)?),
        })
    }

    /// The main screen (`screen`, `None`) or sub-screen `screen.NAME`; the
    /// main screen defaults to `*`.
    pub fn screen(&self, name: Option<&str>) -> Option<Vec<MenuElement>> {
        let value = match name {
            None => self.original.get("screen").or(Some("*")),
            Some(n) => self.original.get(&format!("screen.{n}")),
        }?;
        Some(value.split_whitespace().map(MenuElement::parse).collect())
    }

    /// The sub-screens, by name.
    pub fn screens(&self) -> Vec<&str> {
        self.original
            .iter()
            .filter_map(|(k, _)| k.strip_prefix("screen."))
            .filter(|n| !n.ends_with(".columns") && *n != "columns")
            .collect()
    }

    /// A screen's column count (`screen.columns`, `screen.NAME.columns`).
    pub fn columns(&self, name: Option<&str>) -> Option<u32> {
        let key = name.map_or("screen.columns".to_owned(), |n| {
            format!("screen.{n}.columns")
        });
        self.original.get(&key)?.trim().parse().ok()
    }

    /// Value options shown as sliders rather than buttons.
    pub fn sliders(&self) -> Vec<&str> {
        self.original
            .get("sliders")
            .map(|v| v.split_whitespace().collect())
            .unwrap_or_default()
    }

    /// `iris.features.required` and `iris.features.optional`.
    pub fn features(&self) -> (Vec<&str>, Vec<&str>) {
        let list = |k: &str| {
            self.original
                .get(k)
                .map(|v| v.split_whitespace().collect())
                .unwrap_or_default()
        };
        (
            list("iris.features.required"),
            list("iris.features.optional"),
        )
    }

    /// The profiles in file order, `profile.OTHER` references expanded
    /// (`ProfileSet.fromTree`).
    pub fn profiles(&self, options: &OptionSet) -> Result<Vec<Profile>, String> {
        let tree: Vec<(&str, Vec<&str>)> = self
            .original
            .iter()
            .filter_map(|(k, v)| {
                Some((k.strip_prefix("profile.")?, v.split_whitespace().collect()))
            })
            .collect();
        fn build(
            name: &str,
            parents: &mut Vec<String>,
            tree: &[(&str, Vec<&str>)],
            options: &OptionSet,
            out: &mut Profile,
        ) -> Result<(), String> {
            let words = &tree
                .iter()
                .find(|(n, _)| *n == name)
                .ok_or(format!("profile {name} does not exist"))?
                .1;
            for &w in words {
                if let Some(program) = w.strip_prefix("!program.") {
                    out.disabled_programs.push(program.to_owned());
                } else if let Some(other) = w.strip_prefix("profile.") {
                    if parents.iter().any(|p| p == other) {
                        return Err(format!(
                            "profile {name} includes itself through {}",
                            parents.join(", ")
                        ));
                    }
                    parents.push(other.to_owned());
                    build(other, parents, tree, options, out)?;
                } else if let Some(option) = w.strip_prefix('!') {
                    out.values.insert(option.to_owned(), "false".to_owned());
                } else if let Some((option, value)) =
                    w.split_once('=').or_else(|| w.split_once(':'))
                {
                    out.values.insert(option.to_owned(), value.to_owned());
                } else if options.bools.contains_key(w) {
                    out.values.insert(w.to_owned(), "true".to_owned());
                }
            }
            Ok(())
        }
        tree.iter()
            .map(|(name, _)| {
                let mut p = Profile {
                    name: name.to_string(),
                    values: BTreeMap::new(),
                    disabled_programs: Vec::new(),
                };
                build(name, &mut Vec::new(), &tree, options, &mut p)?;
                Ok(p)
            })
            .collect()
    }

    /// The profile the values match, trying those that set the most options
    /// first (`ProfileSet.scan`).
    pub fn current_profile<'p>(
        profiles: &'p [Profile],
        options: &OptionSet,
        values: &OptionValues,
    ) -> Option<&'p Profile> {
        let mut sorted: Vec<&Profile> = profiles.iter().collect();
        sorted.sort_by_key(|p| std::cmp::Reverse(p.values.len()));
        sorted.into_iter().find(|p| p.matches(options, values))
    }

    /// Whether `program` (`composite1`, `world-1/shadowcomp`) runs: not
    /// turned off by `program.<name>.enabled` or by the current profile.
    pub fn program_enabled(
        &self,
        program: &str,
        options: &OptionSet,
        values: &OptionValues,
    ) -> bool {
        if let Ok(profiles) = self.profiles(options) {
            if let Some(p) = Self::current_profile(&profiles, options, values) {
                if p.disabled_programs.iter().any(|d| d == program) {
                    return false;
                }
            }
        }
        match self.preprocessed.get(&format!("program.{program}.enabled")) {
            Some(expr) => boolean_expression(expr, options, values),
            None => true,
        }
    }

    /// `image.<name> = <sampler> <format> <internal format> ...`: each
    /// custom image's storage format (`rgba16f`), for [`crate::translate::Link`].
    pub fn image_formats(&self) -> BTreeMap<String, String> {
        self.preprocessed
            .iter()
            .filter_map(|(k, v)| {
                Some((
                    k.strip_prefix("image.")?.to_owned(),
                    v.split(' ').nth(2)?.to_lowercase(),
                ))
            })
            .collect()
    }

    /// `uniform.*` and `variable.*`: the pack's custom uniforms.
    pub fn custom_uniforms(&self) -> crate::uniforms::CustomUniforms {
        crate::uniforms::CustomUniforms::parse(self.preprocessed.iter())
    }

    /// `clouds`, `sun` and `moon`: what the pack does with the vanilla sky.
    pub fn sky(&self) -> SkyDirectives {
        let boolean = |key| {
            !matches!(
                self.preprocessed.get(key).map(str::trim),
                Some("false" | "0")
            )
        };
        SkyDirectives {
            clouds: match self.preprocessed.get("clouds").map(str::trim) {
                Some("off") => Some(CloudSetting::Off),
                Some("fast") => Some(CloudSetting::Fast),
                Some("fancy") => Some(CloudSetting::Fancy),
                _ => None,
            },
            sun: boolean("sun"),
            moon: boolean("moon"),
        }
    }

    /// `blend.<program>`: the program's blend override.
    pub fn blend(&self, program: &str) -> Option<Result<Blend, String>> {
        self.preprocessed
            .get(&format!("blend.{program}"))
            .map(parse_blend)
    }

    /// `blend.<program>.<buffer>`: per colour target overrides, by target index.
    pub fn buffer_blends(&self, program: &str) -> Vec<(u32, Result<Blend, String>)> {
        let prefix = format!("blend.{program}.");
        self.preprocessed
            .iter()
            .filter_map(|(k, v)| {
                let buffer = k.strip_prefix(&prefix)?;
                let index = match LEGACY_TARGETS.iter().position(|t| *t == buffer) {
                    Some(i) => i as u32,
                    None => buffer.strip_prefix("colortex")?.parse().ok()?,
                };
                Some((index, parse_blend(v)))
            })
            .collect()
    }
}

/// Iris's `BooleanParser`: options joined by `!`, `&&`, `||` and brackets,
/// `&&` binding tighter. `true`/`1` and `false`/`0` are literals; a name that
/// isn't a boolean option is true; a malformed expression is true.
pub fn boolean_expression(expr: &str, options: &OptionSet, values: &OptionValues) -> bool {
    #[derive(Clone, Copy, PartialEq)]
    enum Op {
        And,
        Or,
        Not,
        Open,
    }
    let value_of = |name: &str| match name {
        "true" | "1" => true,
        "false" | "0" => false,
        n => match options.bools.get(n) {
            Some(o) => values.bools.get(n).copied().unwrap_or(o.default),
            None => true,
        },
    };
    fn push_value(name: &mut String, ops: &mut Vec<Op>, vals: &mut Vec<bool>, value: bool) {
        name.clear();
        if ops.last() == Some(&Op::Not) {
            ops.pop();
            vals.push(!value);
        } else {
            vals.push(value);
        }
    }
    fn evaluate(ops: &mut Vec<Op>, vals: &mut Vec<bool>, bracket: bool) -> Option<()> {
        let mut value = vals.pop()?;
        while let Some(&op) = ops.last() {
            if bracket && op == Op::Open {
                break;
            }
            ops.pop();
            value = match op {
                Op::And => vals.pop()? && value,
                Op::Or => vals.pop()? || value,
                Op::Not => !value,
                Op::Open => value,
            };
        }
        if ops.last() == Some(&Op::Open) {
            ops.pop();
            if ops.last() == Some(&Op::Not) {
                ops.pop();
                value = !value;
            }
        }
        vals.push(value);
        Some(())
    }
    let run = || -> Option<bool> {
        let (mut ops, mut vals, mut name) = (Vec::new(), Vec::new(), String::new());
        let chars: Vec<char> = expr.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            match chars[i] {
                '!' => ops.push(Op::Not),
                c @ ('&' | '|') => {
                    if !name.is_empty() {
                        let v = value_of(&name);
                        push_value(&mut name, &mut ops, &mut vals, v);
                    }
                    if c == '&' {
                        if ops.last() != Some(&Op::And) {
                            ops.push(Op::Open);
                        }
                        ops.push(Op::And);
                    } else {
                        if ops.last() == Some(&Op::And) {
                            evaluate(&mut ops, &mut vals, true)?;
                        }
                        ops.push(Op::Or);
                    }
                    i += 1;
                }
                '(' => ops.push(Op::Open),
                ')' => {
                    if !name.is_empty() {
                        let v = value_of(&name);
                        push_value(&mut name, &mut ops, &mut vals, v);
                    }
                    if ops.last() == Some(&Op::And) {
                        evaluate(&mut ops, &mut vals, true)?;
                    }
                    evaluate(&mut ops, &mut vals, true)?;
                }
                ' ' => {}
                c => name.push(c),
            }
            i += 1;
        }
        if !name.is_empty() {
            let v = value_of(&name);
            push_value(&mut name, &mut ops, &mut vals, v);
        }
        evaluate(&mut ops, &mut vals, false)?;
        let result = vals.pop()?;
        (vals.is_empty() && ops.is_empty()).then_some(result)
    };
    run().unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::{Annotated, OptionSet};

    fn options(lines: &[&str]) -> OptionSet {
        let lines: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
        let a = Annotated::parse(&lines);
        let mut set = OptionSet::default();
        a.bools.values().for_each(|o| set.add_bool(o));
        a.strings.values().for_each(|o| set.add_string(o));
        set
    }

    #[test]
    fn java_properties_syntax() {
        let p = Properties::parse(
            "# comment\n! also\n  a = 1\nb:2\nc 3\nlong = x \\\n    y\nd=\\u0041\\\\z\na=last\nempty\n",
        );
        let all: Vec<(&str, &str)> = p.iter().collect();
        assert_eq!(
            all,
            [
                ("a", "last"),
                ("b", "2"),
                ("c", "3"),
                ("long", "x y"),
                ("d", "A\\z"),
                ("empty", "")
            ]
        );
    }

    #[test]
    fn floats_in_conditions_count_as_their_integer_part() {
        assert_eq!(
            [".5", "0.75", "1.0", "2.5f", "1.5e1", "7", "0x10"].map(jcpp_integer),
            [Some(0), Some(0), Some(1), Some(2), Some(10), Some(7), None]
        );
        // Steadfast's fog strength: the option is 0.5, so Iris takes the
        // 0.75 branch (0 == 0).
        let set = options(&["#define FOG 0.5 // [0.0 0.5 0.75 1.0]"]);
        let text = "\
#if FOG == 1.0
a = 1
#elif FOG == 0.75
a = 0.75
#else
a = 0.5
#endif
#if(FOG<1e1)
b = 1
#endif
";
        let env = Environment {
            defines: Vec::new(),
        };
        let values = OptionValues::default();
        let out = preprocess(text, &env, &set, &values).unwrap();
        assert_eq!(out, "a = 0.75\nb = 1\n");
        assert_eq!(
            jcpp_condition("#elif FOG == 0.75 && X > 2.5", &set, &values),
            "#elif 0 == 0 && X > 2"
        );
    }

    #[test]
    fn boolean_expressions() {
        let set = options(&["#define A", "//#define B", "#define C"]);
        let values = OptionValues::default();
        let eval = |e: &str| boolean_expression(e, &set, &values);
        assert!(eval("A") && !eval("B") && !eval("!A") && eval("!B"));
        assert!(!eval("A && B") && eval("A || B") && eval("B || C"));
        assert!(eval("A && B || C"), "&& binds tighter");
        assert!(!eval("B || C && B"));
        assert!(!eval("!(A && C)") && eval("!(A && B)") && eval("A && !B"));
        assert!(eval("(B || C) && A"));
        assert!(eval("UNKNOWN") && eval("1") && !eval("0") && !eval("false"));
        assert!(eval("A &&"), "malformed: true");
        let mut on = OptionValues::default();
        on.bools.insert("B".into(), true);
        assert!(boolean_expression("A && B", &set, &on));
    }

    #[test]
    fn preprocessed_and_original_reads() {
        let set = options(&["//#define COLORED", "#define QUALITY 2 // [1 2 3]"]);
        let text = "\
screen = QUALITY <empty> [LIGHT] <profile> *
screen.LIGHT = COLORED
screen.LIGHT.columns = 1
sliders = QUALITY
profile.LOW = QUALITY:1 !COLORED !program.composite2
profile.HIGH = profile.LOW QUALITY:3 COLORED
#ifdef MC_OS_MAC
#define QUALITY 5
#endif
#if QUALITY >= 5 && defined MC_OS_MAC
    program.composite1.enabled = false
#else
    program.composite1.enabled = true
#endif
program.world0/shadowcomp.enabled = COLORED
blend.gbuffers_water = SRC_ALPHA ONE_MINUS_SRC_ALPHA ONE ZERO
blend.gbuffers_water.gaux2 = off
blend.gbuffers_water.colortex9 = ONE ONE ONE ONE
size.buffer.colortex4 = 0.50 QUALITY
image.lightimg0 = lighttex0 rgba RGBA16F half_float false false 128 64 128
# a comment, not a directive
iris.features.optional = CUSTOM_IMAGES SSBO
";
        let mut env = Environment {
            defines: Vec::new(),
        };
        env.define("MC_OS_MAC", "");
        let values = OptionValues::default();
        let p = ShaderProperties::new(text, &env, &set, &values).unwrap();
        use MenuElement::*;
        assert_eq!(
            p.screen(None).unwrap(),
            [
                Option("QUALITY".into()),
                Empty,
                Screen("LIGHT".into()),
                Profile,
                Rest
            ]
        );
        assert_eq!(p.screens(), ["LIGHT"]);
        assert_eq!((p.columns(None), p.columns(Some("LIGHT"))), (None, Some(1)));
        assert_eq!(p.sliders(), ["QUALITY"]);
        assert_eq!(
            p.image_formats(),
            [("lightimg0".to_owned(), "rgba16f".to_owned())].into()
        );
        assert_eq!(p.features(), (vec![], vec!["CUSTOM_IMAGES", "SSBO"]));
        // The preprocessed read: #if picked, values expanded, spelling kept.
        assert!(!p.program_enabled("composite1", &set, &values));
        assert!(!p.program_enabled("world0/shadowcomp", &set, &values));
        assert_eq!(
            p.preprocessed.get("size.buffer.colortex4"),
            Some("0.50 5"),
            "QUALITY redefined on macOS"
        );
        assert_eq!(
            p.blend("gbuffers_water"),
            Some(Ok(Blend::Factors(
                ["SRC_ALPHA", "ONE_MINUS_SRC_ALPHA", "ONE", "ZERO"].map(String::from)
            )))
        );
        assert_eq!(
            p.buffer_blends("gbuffers_water"),
            [
                (5, Ok(Blend::Off)),
                (9, Ok(Blend::Factors(["ONE"; 4].map(String::from))))
            ]
        );

        let profiles = p.profiles(&set).unwrap();
        assert_eq!(
            profiles[1].values.get("QUALITY").map(String::as_str),
            Some("3")
        );
        assert_eq!(profiles[1].disabled_programs, ["composite2"]);
        // Defaults (QUALITY 2) match no profile; LOW's values match LOW, which
        // also turns composite2 off.
        assert_eq!(
            ShaderProperties::current_profile(&profiles, &set, &values),
            None
        );
        let low = OptionValues::new(&set, &[("QUALITY".to_owned(), "1".to_owned())].into());
        assert_eq!(
            ShaderProperties::current_profile(&profiles, &set, &low).map(|p| p.name.as_str()),
            Some("LOW")
        );
        assert!(
            !p.program_enabled("composite2", &set, &low)
                && p.program_enabled("composite2", &set, &values)
        );

        // With COLORED on and QUALITY 1, the other branches apply.
        let high = OptionValues::new(
            &set,
            &[
                ("COLORED".to_owned(), "true".to_owned()),
                ("QUALITY".to_owned(), "1".to_owned()),
            ]
            .into(),
        );
        let p = ShaderProperties::new(
            text,
            &Environment {
                defines: Vec::new(),
            },
            &set,
            &high,
        )
        .unwrap();
        assert!(p.program_enabled("composite1", &set, &high));
        assert!(p.program_enabled("world0/shadowcomp", &set, &high));
    }
}
