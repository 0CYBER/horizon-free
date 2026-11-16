//! The staged game's internals catalog, shared by the launcher and mod tools.
use crate::manifest::Internal;
use std::{fs, path::Path};

/// One signature in `internals.json`, including guarded alternatives and lower-crate shapes.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct ListedInternal {
    pub path: String,
    pub kind: String,
    #[serde(default)]
    pub cfg: String,
    pub signature: String,
    pub hash: String,
    #[serde(default)]
    pub shape_only: bool,
}

/// Why a declared internal cannot be used with this game.
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    Missing {
        mod_id: String,
        path: String,
    },
    Changed {
        mod_id: String,
        path: String,
        signature: String,
    },
    Gated {
        mod_id: String,
        path: String,
    },
    Ambiguous {
        mod_id: String,
        path: String,
        kinds: Vec<String>,
    },
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing { mod_id, path } => write!(
                f,
                "E_INTERNAL_MISSING: {mod_id} uses {path}, which this game doesn't have; run cargo horizon-mod internals sync to discover this pin's used internals."
            ),
            Self::Changed {
                mod_id,
                path,
                signature,
            } => write!(
                f,
                "E_INTERNAL_CHANGED: {mod_id} uses {path}, whose signature is now {signature}; run cargo horizon-mod update to see what changed."
            ),
            Self::Ambiguous {
                mod_id,
                path,
                kinds,
            } => write!(
                f,
                "E_INTERNAL_AMBIGUOUS: {mod_id} uses {path}, which has multiple kinds; choose a kind from: {}.",
                kinds.join(", ")
            ),
            Self::Gated { mod_id, path } => write!(
                f,
                "E_INTERNAL_GATED: {mod_id} uses {path}, which exists only under a cfg; target-specific internals aren't supported."
            ),
        }
    }
}
impl std::error::Error for Error {}

/// Read a catalog without compiling or executing game code.
pub fn read(path: &Path) -> Result<Vec<ListedInternal>, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// The entries of an `internals.json` text, after its build-data header is checked.
pub fn parse(text: &str) -> Result<Vec<ListedInternal>, String> {
    #[derive(serde::Deserialize)]
    struct Internals {
        internals: Vec<ListedInternal>,
    }
    let (_, body) = crate::build_data::read::<Internals>(crate::build_data::INTERNALS, text)?;
    Ok(body.internals)
}

/// Resolve an exposed, unconditional entry. Shape-only entries never grant access.
pub fn resolve<'a>(
    entries: &'a [ListedInternal],
    mod_id: &str,
    path: &str,
    kind: Option<&str>,
) -> Result<&'a ListedInternal, Error> {
    let candidates = entries.iter().filter(|e| e.path == path && !e.shape_only);
    if kind.is_none() {
        let kinds: std::collections::BTreeSet<_> =
            candidates.clone().map(|e| e.kind.clone()).collect();
        if kinds.len() > 1 {
            return Err(Error::Ambiguous {
                mod_id: mod_id.to_owned(),
                path: path.to_owned(),
                kinds: kinds.into_iter().collect(),
            });
        }
    }
    let mut gated = false;
    for entry in candidates.filter(|e| kind.is_none_or(|kind| e.kind == kind)) {
        if entry.cfg.is_empty() {
            return Ok(entry);
        }
        gated = true;
    }
    let (mod_id, path) = (mod_id.to_owned(), path.to_owned());
    Err(if gated {
        Error::Gated { mod_id, path }
    } else {
        Error::Missing { mod_id, path }
    })
}

/// Check a declaration's fingerprint before a mod is compiled. An absent fingerprint follows
/// `entries`, the pinned game's catalog; the declaration must still resolve.
pub fn check(entries: &[ListedInternal], mod_id: &str, declared: &Internal) -> Result<(), Error> {
    let entry = resolve(entries, mod_id, &declared.path, declared.kind.as_deref())?;
    if declared
        .hash
        .as_ref()
        .is_some_and(|hash| *hash != entry.hash)
    {
        return Err(Error::Changed {
            mod_id: mod_id.to_owned(),
            path: declared.path.clone(),
            signature: entry.signature.clone(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_named_members_require_a_kind_and_check_its_hash() {
        let path = "horizon_client::game::Round::listening";
        // Test both the free-function and inherent-method kinds used by the catalog.
        for kind in ["fn", "method"] {
            let entries: Vec<ListedInternal> = serde_json::from_value(serde_json::json!([
                { "path": path, "kind": "field", "signature": "bool", "hash": "field" },
                { "path": path, "kind": kind, "signature": "fn listening(&self) -> bool", "hash": "function" }
            ])).unwrap();
            let mut declared = Internal {
                path: path.into(),
                kind: Some(kind.into()),
                hash: Some("function".into()),
            };
            check(&entries, "example", &declared).unwrap();
            declared.hash = Some("field".into());
            assert!(
                matches!(check(&entries, "example", &declared), Err(Error::Changed { signature, .. }) if signature == "fn listening(&self) -> bool")
            );
            declared.kind = None;
            let error = check(&entries, "example", &declared).unwrap_err();
            assert_eq!(
                error,
                Error::Ambiguous {
                    mod_id: "example".into(),
                    path: path.into(),
                    kinds: vec!["field".into(), kind.into()],
                }
            );
            assert!(error.to_string().contains("E_INTERNAL_AMBIGUOUS"));
            assert!(error.to_string().contains(path));
            declared.kind = Some("const".into());
            assert!(matches!(
                check(&entries, "example", &declared),
                Err(Error::Missing { .. })
            ));
        }
    }

    #[test]
    fn an_absent_hash_follows_the_catalog_and_must_still_resolve() {
        let entries: Vec<ListedInternal> = serde_json::from_value(serde_json::json!([
            { "path": "Game::world", "kind": "field", "signature": "World", "hash": "world" }
        ]))
        .unwrap();
        let mut declared = Internal {
            path: "Game::world".into(),
            kind: None,
            hash: None,
        };
        check(&entries, "example", &declared).unwrap();
        declared.hash = Some("stale".into());
        assert!(matches!(
            check(&entries, "example", &declared),
            Err(Error::Changed { .. })
        ));
        declared.hash = None;
        declared.path = "Game::gone".into();
        assert!(matches!(
            check(&entries, "example", &declared),
            Err(Error::Missing { .. })
        ));
    }

    #[test]
    fn kind_resolution_keeps_gated_and_shape_only_refusals() {
        let entries: Vec<ListedInternal> = serde_json::from_value(serde_json::json!([
            { "path": "Round::listening", "kind": "field", "signature": "bool", "hash": "field" },
            { "path": "Round::listening", "kind": "method", "cfg": "feature = tests", "signature": "fn listening(&self)", "hash": "gated" },
            { "path": "Round::listening", "kind": "method", "signature": "fn listening(&self)", "hash": "shape", "shape_only": true }
        ])).unwrap();
        assert!(matches!(
            resolve(&entries, "example", "Round::listening", Some("method")),
            Err(Error::Gated { .. })
        ));
        assert_eq!(
            resolve(&entries, "example", "Round::listening", Some("field"))
                .unwrap()
                .hash,
            "field"
        );
    }
}
