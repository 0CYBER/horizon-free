//! Modrinth metadata for the pack's files: the version of each file by
//! SHA-1, and its project (categories, licence, sides). Only metadata (the
//! jars are `fetch`'s). Answers are cached in `<work>/modrinth.json`, so a
//! rerun works offline.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, path::Path};

const API: &str = "https://api.modrinth.com/v2";
pub(crate) const AGENT: &str = "horizon-rust/horizon horizon-port-pack/0.1 (docs/modpacks.md)";

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Cache {
    /// Version objects by file SHA-1.
    pub versions: BTreeMap<String, Value>,
    /// Project objects by project id.
    pub projects: BTreeMap<String, Value>,
}

impl Cache {
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    /// Ask Modrinth about the hashes and projects not cached yet.
    pub fn fetch(&mut self, hashes: &[String]) -> Result<(), String> {
        let missing: Vec<&String> = hashes
            .iter()
            .filter(|h| !self.versions.contains_key(*h))
            .collect();
        for chunk in missing.chunks(500) {
            let found = version_files(chunk, "sha1")?;
            for (hash, version) in found.as_object().into_iter().flatten() {
                self.versions.insert(hash.clone(), version.clone());
            }
        }
        let mut ids: Vec<String> = self
            .versions
            .values()
            .filter_map(|v| v["project_id"].as_str().map(str::to_owned))
            .filter(|id| !self.projects.contains_key(id))
            .collect();
        ids.sort();
        ids.dedup();
        for chunk in ids.chunks(100) {
            let list = serde_json::to_string(chunk).unwrap_or_default();
            let found = get(&format!("{API}/projects?ids={}", encode(&list)))?;
            for project in found.as_array().into_iter().flatten() {
                if let Some(id) = project["id"].as_str() {
                    self.projects.insert(id.to_owned(), project.clone());
                }
            }
        }
        Ok(())
    }

    pub fn project_of(&self, sha1: &str) -> Option<&Value> {
        let id = self.versions.get(sha1)?["project_id"].as_str()?;
        self.projects.get(id)
    }

    /// Required dependencies of a file's version, as project ids.
    pub fn required(&self, sha1: &str) -> Vec<String> {
        let Some(v) = self.versions.get(sha1) else {
            return Vec::new();
        };
        v["dependencies"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|d| d["dependency_type"] == "required")
            .filter_map(|d| d["project_id"].as_str().map(str::to_owned))
            .collect()
    }
}

/// Resolve the missing manifest jars in one exact-SHA-512 request, after download consent.
pub fn jar_urls(
    jars: &[&horizon_mod_package::manifest::JarRef],
) -> Result<BTreeMap<String, String>, String> {
    let hashes: Vec<_> = jars.iter().map(|jar| jar.sha512()).collect();
    jar_file_urls(version_files(hashes, "sha512")?)
}

fn version_files(hashes: impl Serialize, algorithm: &str) -> Result<Value, String> {
    post(
        &format!("{API}/version_files"),
        &serde_json::json!({"hashes": hashes, "algorithm": algorithm}),
    )
}

fn jar_file_urls(found: Value) -> Result<BTreeMap<String, String>, String> {
    #[derive(Deserialize)]
    struct Version {
        files: Vec<File>,
    }
    #[derive(Deserialize)]
    struct File {
        hashes: BTreeMap<String, String>,
        url: String,
    }
    let versions: BTreeMap<String, Version> =
        serde_json::from_value(found).map_err(|e| format!("Modrinth version files: {e}"))?;
    Ok(versions
        .into_iter()
        .filter_map(|(hash, version)| {
            version
                .files
                .into_iter()
                .find(|file| {
                    file.hashes
                        .get("sha512")
                        .is_some_and(|actual| actual.eq_ignore_ascii_case(&hash))
                })
                .map(|file| (hash, file.url))
        })
        .collect())
}

fn post(url: &str, body: &Value) -> Result<Value, String> {
    let response = crate::net::agent(url)
        .post(url)
        .set("User-Agent", AGENT)
        .set("Content-Type", "application/json")
        .send_string(&body.to_string())
        .map_err(|e| format!("{url}: {e}"))?;
    read(url, response)
}

fn get(url: &str) -> Result<Value, String> {
    let response = crate::net::agent(url)
        .get(url)
        .set("User-Agent", AGENT)
        .call()
        .map_err(|e| format!("{url}: {e}"))?;
    read(url, response)
}

fn read(url: &str, response: ureq::Response) -> Result<Value, String> {
    let text = response.into_string().map_err(|e| format!("{url}: {e}"))?;
    serde_json::from_str(&text).map_err(|e| format!("{url}: {e}"))
}

/// Percent-encoding for a query value.
fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_hash_selects_the_file_not_the_versions_primary_jar() {
        let hash = "a".repeat(128);
        let other = "b".repeat(128);
        let found = serde_json::json!({
            hash.clone(): {"files": [
                {"primary": true, "url": "wrong.jar", "hashes": {"sha512": other}},
                {"url": "exact.jar", "hashes": {"sha512": hash}}
            ]},
            other.clone(): {"files": [
                {"url": "other.jar", "hashes": {"sha512": other.to_uppercase()}}
            ]}
        });
        assert_eq!(
            jar_file_urls(found).unwrap(),
            BTreeMap::from([
                (hash.clone(), "exact.jar".into()),
                (other, "other.jar".into()),
            ])
        );
        assert!(jar_file_urls(serde_json::json!({})).unwrap().is_empty());
        assert!(
            jar_file_urls(serde_json::json!({hash.clone(): {"files": []}}))
                .unwrap()
                .is_empty()
        );
        assert!(jar_file_urls(serde_json::json!({hash.clone(): {"files": "broken"}})).is_err());
        assert!(jar_file_urls(serde_json::json!([])).is_err());
    }
}
