//! The namespaces content mods add to a resource root, which loaders read
//! besides `minecraft`.
use std::{fs, path::Path};

/// In a root built here: the namespaces the content mods own (their ids and
/// listed namespaces), which loaders read besides `minecraft`.
pub const MARKER: &str = "horizon-mod-content.json";

/// The content mods' namespaces in a root built by `mod_content::build`; none in any
/// other root, so vanilla's loaders read exactly what they did.
pub fn namespaces(root: &Path) -> Vec<String> {
    fs::read(root.join(MARKER))
        .ok()
        .map(|bytes| decode_namespaces(&bytes))
        .unwrap_or_default()
}

/// Namespaces from the generated content marker, shared with resource-source loaders.
/// Never `minecraft`, which a root built before that was left out could list.
pub fn decode_namespaces(bytes: &[u8]) -> Vec<String> {
    let mut namespaces: Vec<String> = serde_json::from_slice(bytes).unwrap_or_default();
    namespaces.retain(|namespace| namespace != "minecraft");
    namespaces
}

/// The namespaces whose tags loaders read in `root`: `minecraft`, and in a
/// root built by `mod_content::build` the convention tags (`c`, shared between mods as
/// on NeoForge and Fabric) and the content mods' own.
pub fn tag_namespaces(root: &Path) -> Vec<String> {
    let mods = namespaces(root);
    let mut out = vec!["minecraft".to_owned()];
    if !mods.is_empty() {
        out.push("c".to_owned());
        out.extend(mods);
    }
    out
}

#[cfg(test)]
mod tests {
    /// A root built while a mod's `data/minecraft/...` files still listed
    /// vanilla's namespace reads as if they hadn't.
    #[test]
    fn vanilla_is_never_a_content_namespace() {
        assert_eq!(
            super::decode_namespaces(br#"["minecraft", "salvage"]"#),
            ["salvage"]
        );
    }
}
