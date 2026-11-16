//! `cargo horizon-mod check <file>`: what the registry checks of a submission,
//! run anywhere: its kind's own checks (docs/modding/src/reference/registry-index.md), its signature
//! when given a key, and for a `.hmod` that it is content only.
use horizon_mod_package::index::{self, Facts, Kind};
use serde_json::{Value, json};
use std::path::Path;

pub struct Report {
    pub kind: Kind,
    pub facts: Facts,
    pub sha256: String,
    pub size: u64,
    pub signed: bool,
}

impl Report {
    pub fn json(&self) -> Value {
        let requires: Vec<Value> = self
            .facts
            .requires
            .iter()
            .map(|r| json!({"id": r.id, "version": r.version, "sha256": r.sha256}))
            .collect();
        json!({
            "kind": self.kind.name(),
            "id": self.facts.id,
            "version": self.facts.version,
            "api": self.facts.api,
            "side": self.facts.side,
            "capabilities": self.facts.capabilities,
            "game": self.facts.game,
            "requires": requires,
            "sha256": self.sha256,
            "size": self.size,
            "signed": self.signed,
        })
    }

    pub fn text(&self) -> String {
        let f = &self.facts;
        let mut out = format!(
            "{} {} ({}",
            f.id.as_deref().unwrap_or("resource pack"),
            f.version.as_deref().unwrap_or(""),
            self.kind.name()
        );
        if let Some(side) = f.side {
            let side = serde_json::to_value(side).unwrap_or_default();
            out += &format!(", API {}, {}", f.api, side.as_str().unwrap_or("?"));
        } else if self.kind == Kind::Shader {
            out += &format!(", shader API {}", f.api);
        } else if !f.api.is_empty() {
            out += &format!(", format {}", f.api);
        }
        if let Some(game) = &f.game {
            out += &format!(", for {game}");
        }
        let signed = if self.signed {
            "signature verified"
        } else {
            "unsigned"
        };
        out += &format!(
            ")\n  {} bytes, SHA-256 {}, {signed}\n",
            self.size, self.sha256
        );
        for c in &f.capabilities {
            out += &format!("  may {}\n", c.describe());
        }
        for r in &f.requires {
            out += &format!("  needs {} {} ({})\n", r.id, r.version, &r.sha256[..12]);
        }
        if self.kind == Kind::NativeMod {
            out += "  native source: unsandboxed; not executed by the registry check. Compile and test against the exact pinned Horizon and Mixins revisions before review.\n";
        }
        if self.kind == Kind::Mod {
            out += "  no code: content only, nothing to run\n";
        }
        out
    }
}

/// The kind of a package, inspecting mod archives for their native manifest.
pub fn kind_of(path: &Path) -> Option<Kind> {
    match path.extension()?.to_str()? {
        "hmod" if index::native::holds(path) => Some(Kind::NativeMod),
        "hmod" => Some(Kind::Mod),
        "hpack" => Some(Kind::Modpack),
        "zip" => Some(horizon_mod_package::index::zip_kind(path)),
        "hshader" => Some(Kind::Shader),
        _ => None,
    }
}

/// Checks the file at `path` as `kind`; `key` and `signature` (the
/// `.minisig` text) check its signature.
pub fn check(
    path: &Path,
    kind: Kind,
    key: Option<&str>,
    signature: Option<&str>,
) -> Result<Report, String> {
    let size = std::fs::metadata(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .len();
    if size > index::MAX_FILE {
        return Err(format!("over {} MiB", index::MAX_FILE >> 20));
    }
    index::open(&kind)?;
    let facts = index::inspect(&kind, path)?;
    let signed = match (key, signature) {
        (Some(key), Some(sig)) => {
            index::verify_signature(key, sig, path)?;
            true
        }
        (None, None) => false,
        _ => return Err("--key and --signature go together".into()),
    };
    Ok(Report {
        kind,
        facts,
        sha256: index::sha256_file(path)?,
        size,
        signed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../horizon-mod-package/tests/index")
            .join(name)
    }

    #[test]
    fn checks_each_kind_its_own_way() {
        let pack = fixture("cozy-1.0.0.hpack");
        let report = check(&pack, kind_of(&pack).unwrap(), None, None).unwrap();
        assert_eq!(report.facts.requires[0].id, "coords_hud");
        assert!(
            report
                .text()
                .contains("cozy 1.0.0 (modpack, format 1, for 26.3)")
        );
        // A signed content mod passes; a bad signature is still refused.
        let hmod = fixture("coords_hud-0.1.0.hmod");
        let key = std::fs::read_to_string(fixture("test.pub")).unwrap();
        let sig = std::fs::read_to_string(fixture("coords_hud-0.1.0.hmod.minisig")).unwrap();
        let key = key.lines().last().unwrap();
        let report = check(&hmod, Kind::Mod, Some(key), Some(&sig)).unwrap();
        assert!(report.signed);
        assert!(report.text().contains("no code: content only"));
        let e = check(
            &hmod,
            Kind::Mod,
            Some(key),
            Some("untrusted comment: x\nRUQ=\n"),
        )
        .err()
        .unwrap();
        assert!(e.contains("signature"), "{e}");
        let e = check(&pack, Kind::Shader, None, None).err().unwrap();
        assert!(e.contains("horizon-shaders.toml"), "{e}");
        assert!(check(&pack, Kind::Mod, None, None).is_err());
    }
}
