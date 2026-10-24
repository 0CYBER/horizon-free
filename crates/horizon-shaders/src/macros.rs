//! Object-like macros: the ones packs test in `#if` lines and use inside property values
//! (spec 1.5, 1.5.1). The GLSL preprocessor's function-like macros are not needed here.

use std::collections::BTreeMap;

/// `MC_VERSION` of the version Horizon plays: 26.3 → 260300.
pub const MC_VERSION: u32 = mc_version(horizon_core::version::VERSION_ID);

/// The documented rule, `major * 10000 + minor * 100 + patch` (spec 1.5.1, OQ-1): `1.21` → 12100,
/// `1.7.10` → 10710. Parts that are not digits count as 0.
pub const fn mc_version(id: &str) -> u32 {
    let bytes = id.as_bytes();
    let (mut parts, mut part, mut i) = ([0u32; 3], 0, 0);
    while i < bytes.len() && part < 3 {
        match bytes[i] {
            b'.' => part += 1,
            b @ b'0'..=b'9' => parts[part] = parts[part] * 10 + (b - b'0') as u32,
            _ => break,
        }
        i += 1;
    }
    parts[0] * 10000 + parts[1] * 100 + parts[2]
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Macros(BTreeMap<String, String>);

impl Macros {
    /// The standard macros that need no renderer: `MC_VERSION` and this system's `MC_OS_*`.
    /// The graphics ones arrive with the renderer.
    pub fn standard() -> Self {
        let mut m = Self::default();
        m.define("MC_VERSION", MC_VERSION.to_string());
        let os = match std::env::consts::OS {
            "windows" => "MC_OS_WINDOWS",
            "macos" => "MC_OS_MAC",
            "linux" => "MC_OS_LINUX",
            _ => "MC_OS_OTHER",
        };
        m.define(os, "");
        m
    }

    pub fn define(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.0.insert(name.into(), value.into());
    }

    pub fn undef(&mut self, name: &str) {
        self.0.remove(name);
    }

    pub fn is_defined(&self, name: &str) -> bool {
        self.0.contains_key(name)
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.get(name).map(String::as_str)
    }

    /// `text` with every identifier that names a macro replaced by its value, repeatedly; a macro
    /// is not expanded again inside its own expansion, so `A` defined as `A + 1` stops.
    pub fn expand(&self, text: &str) -> String {
        self.expand_hiding(text, &mut Vec::new())
    }

    fn expand_hiding<'a>(&'a self, text: &str, hidden: &mut Vec<&'a str>) -> String {
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(start) = rest.find(is_ident_start) {
            out.push_str(&rest[..start]);
            // A digit run glued to letters (`1e5`, `0x1F`) is a number, not a name.
            let glued = out
                .chars()
                .next_back()
                .is_some_and(|c| c.is_ascii_alphanumeric() || c == '.');
            let len = rest[start..]
                .find(|c: char| !is_ident(c))
                .unwrap_or(rest.len() - start);
            let word = &rest[start..start + len];
            match self.0.get_key_value(word) {
                Some((name, value)) if !glued && !hidden.contains(&name.as_str()) => {
                    hidden.push(name);
                    out.push_str(&self.expand_hiding(value, hidden));
                    hidden.pop();
                }
                _ => out.push_str(word),
            }
            rest = &rest[start + len..];
        }
        out.push_str(rest);
        out
    }
}

pub(crate) fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

pub(crate) fn is_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_follow_the_documented_rule() {
        assert_eq!(mc_version("1.21"), 12100);
        assert_eq!(mc_version("1.21.0"), 12100);
        assert_eq!(mc_version("1.7.10"), 10710);
        assert_eq!(mc_version("26.3"), 260300);
        assert_eq!(MC_VERSION, 260300);
        assert_eq!(Macros::standard().get("MC_VERSION"), Some("260300"));
    }

    #[test]
    fn expansion_replaces_whole_words_and_stops_on_recursion() {
        let mut m = Macros::default();
        m.define("RES", "SIZE");
        m.define("SIZE", "512");
        m.define("LOOP", "LOOP + 1");
        assert_eq!(m.expand("RES RES 0.5"), "512 512 0.5");
        assert_eq!(
            m.expand("RESOLUTION colortex4 LOOP"),
            "RESOLUTION colortex4 LOOP + 1"
        );
        m.define("e5", "x");
        assert_eq!(m.expand("1e5"), "1e5");
    }
}
