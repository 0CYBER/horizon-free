//! The internals catalog as the generator reads it.
use crate::internals::Internal;
use std::collections::BTreeMap;

/// An owned catalog boundary. Static tags in `Internal` are selected here, never leaked strings.
pub fn catalog(text: &str) -> Result<Vec<Internal>, String> {
    #[derive(serde::Deserialize)]
    struct Entry {
        path: String,
        kind: String,
        signature: String,
        hash: String,
        #[serde(default)]
        cfg: String,
        #[serde(default)]
        shape_only: bool,
        #[serde(default)]
        opaque_reason: Option<String>,
        #[serde(default)]
        refs: BTreeMap<String, String>,
    }
    let entries: Vec<Entry> = serde_json::from_str(text).map_err(|e| e.to_string())?;
    entries
        .into_iter()
        .map(|e| {
            Ok(Internal {
                path: e.path,
                kind: match e.kind.as_str() {
                    "struct" => "struct",
                    "enum" => "enum",
                    "field" => "field",
                    "method" => "method",
                    "fn" => "fn",
                    "const" => "const",
                    "static" => "static",
                    "type" => "type",
                    "trait" => "trait",
                    "mod" => "mod",
                    _ => return Err(format!("unknown internal kind {}", e.kind)),
                },
                signature: e.signature,
                hash: e.hash,
                cfg: e.cfg,
                shape_only: e.shape_only,
                opaque_reason: e.opaque_reason.map(|_| "opaque layout"),
                refs: e.refs,
            })
        })
        .collect()
}
