//! Terrain render-layer declarations in `block.properties` (spec 4.2).
//! Kept separate from material IDs: changing a layer must not change a block's ID.

use crate::Properties;

/// The terrain pass and alpha handling requested by a pack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderLayer {
    Solid,
    /// Both `cutout` and `cutout_mipped` use the terrain atlas's mipmaps.
    Cutout,
    Translucent,
}

#[derive(Clone, Debug)]
struct Selection {
    name: String,
    filters: Vec<(String, Vec<String>)>,
    layer: RenderLayer,
}

/// Named block-state selections; unknown registry names simply never match.
#[derive(Clone, Debug, Default)]
pub struct BlockLayers {
    selections: Vec<Selection>,
    pub notes: Vec<String>,
}

impl BlockLayers {
    pub fn parse(properties: &Properties) -> Self {
        let mut out = Self::default();
        for entry in &properties.entries {
            let Some(key) = entry.key.strip_prefix("layer.") else {
                continue;
            };
            let layer = match key {
                "solid" => RenderLayer::Solid,
                "cutout" | "cutout_mipped" => RenderLayer::Cutout,
                "translucent" => RenderLayer::Translucent,
                _ => {
                    out.notes
                        .push(format!("line {}: unknown render layer {key}", entry.line));
                    continue;
                }
            };
            for token in entry.value.split_whitespace() {
                if let Some((name, filters)) = selection(token) {
                    out.selections.push(Selection {
                        name,
                        filters,
                        layer,
                    });
                } else {
                    out.notes.push(format!(
                        "line {}: invalid layer block selection {token}",
                        entry.line
                    ));
                }
            }
        }
        out
    }

    /// All filters must match; values within one filter are alternatives. More specific
    /// selections win, with file order breaking ties, as for the accepted named ID rules.
    pub fn layer<'a>(
        &self,
        name: &str,
        property: impl Fn(&str) -> Option<&'a str>,
    ) -> Option<RenderLayer> {
        let mut best = None;
        for selection in &self.selections {
            if selection.name != name
                || !selection.filters.iter().all(|(key, values)| {
                    property(key).is_some_and(|value| values.iter().any(|v| v == value))
                })
            {
                continue;
            }
            let specificity = selection.filters.len();
            if best.is_none_or(|(n, _)| specificity > n) {
                best = Some((specificity, selection.layer));
            }
        }
        best.map(|(_, layer)| layer)
    }
}

fn selection(token: &str) -> Option<(String, Vec<(String, Vec<String>)>)> {
    let mut parts = token.split(':').peekable();
    let first = parts.next()?;
    let (namespace, name) = if parts.peek().is_some_and(|part| !part.contains('=')) {
        (first, parts.next()?)
    } else {
        ("minecraft", first)
    };
    let name_part = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_./-".contains(&b))
    };
    if !name_part(namespace) || !name_part(name) {
        return None;
    }
    let mut filters = Vec::new();
    for part in parts {
        let (key, values) = part.split_once('=')?;
        if key.is_empty() || values.split(',').any(|v| v.is_empty() || v.contains('=')) {
            return None;
        }
        filters.push((
            key.to_owned(),
            values.split(',').map(str::to_owned).collect(),
        ));
    }
    Some((format!("{namespace}:{name}"), filters))
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::Macros;

    #[test]
    fn layers_match_names_and_all_state_filters() {
        let p = Properties::parse(
            "block.7=example:panel\n\
             layer.translucent=example:panel:half=upper,lower:wet=false glass\n\
             layer.solid=example:panel:half=upper:wet=true\n\
             layer.cutout=example:leaf\n\
             layer.cutout_mipped=example:grate\n\
             layer.unknown=example:panel\n",
            &Macros::standard(),
        );
        let layers = BlockLayers::parse(&p);
        let pick = |name, half, wet| {
            layers.layer(name, |key| match key {
                "half" => Some(half),
                "wet" => Some(wet),
                _ => None,
            })
        };
        assert_eq!(
            pick("example:panel", "upper", "false"),
            Some(RenderLayer::Translucent)
        );
        assert_eq!(
            pick("example:panel", "lower", "false"),
            Some(RenderLayer::Translucent)
        );
        assert_eq!(
            pick("example:panel", "upper", "true"),
            Some(RenderLayer::Solid)
        );
        assert_eq!(pick("example:panel", "lower", "true"), None);
        assert_eq!(layers.layer("example:panel", |_| None), None);
        assert_eq!(
            layers.layer("minecraft:glass", |_| None),
            Some(RenderLayer::Translucent)
        );
        assert_eq!(
            layers.layer("example:leaf", |_| None),
            Some(RenderLayer::Cutout)
        );
        assert_eq!(
            layers.layer("example:grate", |_| None),
            Some(RenderLayer::Cutout)
        );
        assert_eq!(layers.layer("example:missing", |_| None), None);
        assert_eq!(layers.notes.len(), 1);
        assert_eq!(
            BlockLayers::default().layer("example:panel", |_| None),
            None
        );
    }
}
