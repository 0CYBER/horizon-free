//! What a mod jar holds, read without decompiling: its metadata
//! (`neoforge.mods.toml`, `mods.toml`, `fabric.mod.json`, `mcmod.info`) and a
//! census of its data, assets and classes. Legacy metadata contributes identity
//! only; license and dependency fields are not interpreted.
use crate::zip_intake::{self, Archive};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct JarInfo {
    pub mod_id: String,
    pub name: String,
    pub version: String,
    pub license: String,
    /// Required dependencies by mod id (loaders and `minecraft` left out).
    pub depends: Vec<String>,
    pub census: Census,
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Census {
    pub blockstates: usize,
    pub item_models: usize,
    pub recipes: usize,
    pub loot_tables: usize,
    pub tags: usize,
    pub worldgen: usize,
    pub structures: usize,
    pub languages: usize,
    pub textures: usize,
    pub sounds: usize,
    pub classes: usize,
    pub mixin_configs: usize,
    /// Files under `data/<namespace>/`, of any kind.
    #[serde(default)]
    pub data_files: usize,
    /// A `pack.mcmeta` at the top: the jar is also a data or resource pack.
    #[serde(default)]
    pub pack_mcmeta: bool,
}

const NOT_MODS: [&str; 5] = ["minecraft", "neoforge", "forge", "fabricloader", "java"];

/// The loader whose metadata identifies a jar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JarLoader {
    NeoForge,
    Fabric,
}

/// Identity read from the jar itself, never inferred from its filename.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JarIdentity {
    pub loader: JarLoader,
    pub id: String,
    pub version: String,
}

/// Read a NeoForge or Fabric jar's own identity, refusing missing or invalid metadata.
/// Each metadata entry is limited to 1 MiB after decompression.
pub fn read_identity(path: &Path) -> Result<JarIdentity, String> {
    let (info, loader) = read_inner(path, true)?;
    let loader = loader.expect("strict metadata always has a loader");
    let bytes = info.mod_id.as_bytes();
    let valid_id = (2..=64).contains(&bytes.len())
        && match loader {
            JarLoader::Fabric => {
                bytes[0].is_ascii_lowercase()
                    && bytes
                        .iter()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_-".contains(b))
            }
            JarLoader::NeoForge => bytes
                .iter()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_'),
        };
    if !valid_id || info.version.is_empty() || info.version.contains("${") {
        return Err(format!(
            "{}: invalid jar identity or version",
            path.display()
        ));
    }
    Ok(JarIdentity {
        loader,
        id: info.mod_id,
        version: info.version,
    })
}

pub fn read(path: &Path) -> Result<JarInfo, String> {
    read_inner(path, false).map(|(info, _)| info)
}

fn read_inner(path: &Path, strict: bool) -> Result<(JarInfo, Option<JarLoader>), String> {
    let file = fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut zip = zip_intake::open(file).map_err(|e| format!("{}: {e}", path.display()))?;
    let text = |zip: &mut Archive<fs::File>, name: &str| -> Result<Option<String>, String> {
        let entry = match zip.by_name(name) {
            Ok(entry) => entry,
            Err(zip::result::ZipError::FileNotFound) => return Ok(None),
            Err(e) if strict => return Err(format!("{}: {name}: {e}", path.display())),
            Err(_) => return Ok(None),
        };
        const MAX_METADATA_BYTES: u64 = 1024 * 1024;
        let declared = entry.size();
        let cap = if strict {
            MAX_METADATA_BYTES
        } else {
            crate::entry::MAX_TEXT
        };
        match crate::entry::read_text(entry, declared, cap) {
            Ok(s) => Ok(Some(s)),
            Err(e) if strict || crate::entry::is_too_big(&e) => {
                Err(format!("{}: {name}: {e}", path.display()))
            }
            Err(_) => Ok(None),
        }
    };
    let neo = text(&mut zip, "META-INF/neoforge.mods.toml")?;
    let (mut info, loader) = if strict {
        if let Some(t) = neo {
            (mods_toml(&t)?, Some(JarLoader::NeoForge))
        } else if let Some(t) = text(&mut zip, "fabric.mod.json")? {
            (fabric(&t)?, Some(JarLoader::Fabric))
        } else {
            return Err(format!(
                "{}: no NeoForge or Fabric metadata",
                path.display()
            ));
        }
    } else {
        let tomls: Vec<String> = neo
            .into_iter()
            .chain(text(&mut zip, "META-INF/mods.toml")?)
            .collect();
        let info = if !tomls.is_empty() {
            metadata(&tomls)?
        } else if let Some(t) = text(&mut zip, "fabric.mod.json")? {
            fabric(&t)?
        } else if let Some(t) = text(&mut zip, "mcmod.info")? {
            mcmod_info(&t)?
        } else {
            JarInfo::default()
        };
        (info, None)
    };
    // `version = "${file.jarVersion}"` is filled in from the manifest at load.
    if (loader == Some(JarLoader::NeoForge) && info.version == "${file.jarVersion}")
        || (!strict && info.version.starts_with("${"))
    {
        info.version = text(&mut zip, "META-INF/MANIFEST.MF")?
            .and_then(|m| manifest_value(&m, "Implementation-Version"))
            .unwrap_or_default();
    }
    if strict {
        return Ok((info, loader));
    }
    let names: Vec<String> = zip
        .file_names()
        .map(|name| name.map(|name| name.into_owned()))
        .collect::<Result<_, _>>()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    info.census = census(&names);
    if info.mod_id.is_empty() {
        info.mod_id = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
    }
    Ok((info, loader))
}

/// A main-section attribute of `META-INF/MANIFEST.MF`.
fn manifest_value(manifest: &str, key: &str) -> Option<String> {
    let manifest = manifest.replace("\r\n", "\n").replace('\r', "\n");
    let mut value: Option<String> = None;
    for line in manifest.lines().take_while(|line| !line.is_empty()) {
        if let Some(continuation) = line.strip_prefix(' ') {
            if let Some(value) = &mut value {
                value.push_str(continuation);
            }
        } else if value.is_some() {
            break;
        } else if let Some((name, contents)) = line.split_once(':')
            && name.eq_ignore_ascii_case(key)
        {
            value = Some(contents.strip_prefix(' ').unwrap_or(contents).to_owned());
        }
    }
    value.filter(|value| !value.is_empty())
}

/// The jar's mod from its metadata files (`neoforge.mods.toml` first, then
/// `mods.toml`). A jar built for one loader may carry the other loader's file
/// as an unfilled template (`modId = "${id}"`, `[[dependencies.${id}]]`), which
/// its own loader never reads: the first file that parses and names a real mod
/// id is the one. When none parses, the first file is read line by line
/// ([`mods_toml_lines`]), as a loader that reads past what it does not need.
pub fn metadata(tomls: &[String]) -> Result<JarInfo, String> {
    let real = |i: &JarInfo| !i.mod_id.is_empty() && !i.mod_id.contains("${");
    let mut first_error = None;
    for t in tomls {
        match mods_toml(t) {
            Ok(i) if real(&i) => return Ok(i),
            Ok(_) => {}
            Err(e) => {
                first_error.get_or_insert(e);
            }
        }
    }
    let lines: Vec<JarInfo> = tomls.iter().map(|t| mods_toml_lines(t)).collect();
    // A lone template still names its (placeholder) mod id: last resort.
    lines
        .iter()
        .position(real)
        .or_else(|| lines.iter().position(|i| !i.mod_id.is_empty()))
        .map(|i| lines.into_iter().nth(i).unwrap_or_default())
        .ok_or_else(|| first_error.unwrap_or_else(|| "mods.toml: no [[mods]] modId".into()))
}

/// The first `[[mods]]` table's `modId`, `displayName` and `version` and the
/// top-level `license`, read line by line (`key = "value"`, `#` comments), for
/// a metadata file a TOML parser refuses. Dependencies are not read.
pub fn mods_toml_lines(text: &str) -> JarInfo {
    let mut info = JarInfo::default();
    // 0: top level, 1: the first [[mods]], 2: past it.
    let mut section = 0;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            let name = line.trim_matches(['[', ']']).trim();
            section = match (section, name) {
                (0, "mods") => 1,
                _ => 2,
            };
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        let Some(value) = value
            .strip_prefix('"')
            .and_then(|v| v.split_once('"'))
            .or_else(|| value.strip_prefix('\'').and_then(|v| v.split_once('\'')))
            .map(|(v, _)| v.to_owned())
        else {
            continue;
        };
        match (section, key.trim()) {
            (0, "license") => info.license = value,
            (1, "modId") if info.mod_id.is_empty() => info.mod_id = value,
            (1, "displayName") => info.name = value,
            (1, "version") => info.version = value,
            _ => {}
        }
    }
    info
}

/// `[[mods]]` (the first one is the jar's mod), the top-level `license`, and
/// `[[dependencies.<id>]]` with `type = "required"` (or the older
/// `mandatory = true`).
pub fn mods_toml(text: &str) -> Result<JarInfo, String> {
    let doc: toml::Table = toml::from_str(text).map_err(|e| format!("mods.toml: {e}"))?;
    let first = doc
        .get("mods")
        .and_then(|m| m.as_array())
        .and_then(|m| m.first())
        .and_then(|m| m.as_table());
    let get = |t: Option<&toml::Table>, k: &str| {
        t.and_then(|t| t.get(k))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned()
    };
    let mod_id = get(first, "modId");
    let mut depends = Vec::new();
    if let Some(deps) = doc
        .get("dependencies")
        .and_then(|d| d.get(&mod_id))
        .and_then(|d| d.as_array())
    {
        for d in deps.iter().filter_map(|d| d.as_table()) {
            let required = d.get("type").and_then(|t| t.as_str()).map_or_else(
                || d.get("mandatory").and_then(|m| m.as_bool()) == Some(true),
                |t| t.eq_ignore_ascii_case("required"),
            );
            let id = get(Some(d), "modId");
            if required && !NOT_MODS.contains(&id.as_str()) && !id.is_empty() {
                depends.push(id);
            }
        }
    }
    Ok(JarInfo {
        name: get(first, "displayName"),
        version: get(first, "version"),
        license: get(Some(&doc), "license"),
        mod_id,
        depends,
        census: Census::default(),
    })
}

pub fn fabric(text: &str) -> Result<JarInfo, String> {
    let v: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("fabric.mod.json: {e}"))?;
    let s = |k: &str| v[k].as_str().unwrap_or("").to_owned();
    let license = match &v["license"] {
        serde_json::Value::Array(l) => l
            .iter()
            .filter_map(|x| x.as_str())
            .collect::<Vec<_>>()
            .join(" OR "),
        l => l.as_str().unwrap_or("").to_owned(),
    };
    let depends = v["depends"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(id, _)| id.clone())
        .filter(|id| !NOT_MODS.contains(&id.as_str()) && !id.starts_with("fabric"))
        .collect();
    Ok(JarInfo {
        mod_id: s("id"),
        name: s("name"),
        version: s("version"),
        license,
        depends,
        census: Census::default(),
    })
}

/// Read the first mod's identity from legacy Forge metadata.
/// License and dependency fields remain unknown.
fn mcmod_info(text: &str) -> Result<JarInfo, String> {
    let value: serde_json::Value = serde_json::from_str(text)
        .or_else(|error| {
            let Some(normalized) = escape_legacy_string_newlines(text) else {
                return Err(error);
            };
            serde_json::from_str(&normalized).map_err(|_| error)
        })
        .map_err(|e| format!("mcmod.info: {e}"))?;
    let first = match &value {
        serde_json::Value::Array(mods) => mods.first(),
        serde_json::Value::Object(root) => root
            .get("modList")
            .and_then(serde_json::Value::as_array)
            .and_then(|mods| mods.first()),
        _ => None,
    }
    .and_then(serde_json::Value::as_object);
    let get = |key: &str| {
        first
            .and_then(|mod_info| mod_info.get(key))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_owned()
    };
    Ok(JarInfo {
        mod_id: get("modid"),
        name: get("name"),
        version: get("version"),
        ..JarInfo::default()
    })
}

// Forge 1.7.10 packs can contain literal line feeds in legacy metadata strings.
// Escape just those line feeds and let the JSON parser validate the result.
fn escape_legacy_string_newlines(text: &str) -> Option<String> {
    let mut normalized = String::new();
    let mut copied_until = 0;
    let mut quoted = false;
    let mut characters = text.char_indices();
    while let Some((offset, character)) = characters.next() {
        match character {
            '"' => quoted = !quoted,
            '\\' if quoted => {
                characters.next();
            }
            '\n' if quoted => {
                normalized.push_str(&text[copied_until..offset]);
                normalized.push_str("\\n");
                copied_until = offset + 1;
            }
            _ => {}
        }
    }
    if copied_until == 0 {
        return None;
    }
    normalized.push_str(&text[copied_until..]);
    Some(normalized)
}

pub fn census(names: &[String]) -> Census {
    let mut c = Census::default();
    for n in names {
        let parts: Vec<&str> = n.split('/').collect();
        if n.ends_with(".class") {
            c.classes += 1;
        } else if n.ends_with(".mixins.json") || (parts.len() == 1 && n.contains("mixins")) {
            c.mixin_configs += 1;
        }
        c.pack_mcmeta |= n == "pack.mcmeta";
        if parts.len() > 2 && parts[0] == "data" && !n.ends_with('/') {
            c.data_files += 1;
        }
        // assets/<ns>/<kind>/... and data/<ns>/<kind>/...
        let (Some(root), Some(kind)) = (parts.first(), parts.get(2)) else {
            continue;
        };
        let json = n.ends_with(".json");
        match (*root, *kind) {
            ("assets", "blockstates") if json => c.blockstates += 1,
            ("assets", "models") if json && parts.get(3) == Some(&"item") => c.item_models += 1,
            ("assets", "lang") => c.languages += 1,
            ("assets", "textures") if n.ends_with(".png") => c.textures += 1,
            ("assets", "sounds.json") => c.sounds += 1,
            ("data", "recipe" | "recipes") if json => c.recipes += 1,
            ("data", "loot_table" | "loot_tables") if json => c.loot_tables += 1,
            ("data", "tags") if json => c.tags += 1,
            ("data", "worldgen") if json => c.worldgen += 1,
            ("data", "structure" | "structures") if n.ends_with(".nbt") => c.structures += 1,
            _ => {}
        }
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::Write,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    static NEXT_TEST_JAR: AtomicU64 = AtomicU64::new(0);

    struct SyntheticJar(PathBuf, PathBuf);

    impl Drop for SyntheticJar {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
            let _ = fs::remove_dir(&self.1);
        }
    }

    fn synthetic_jar<B: AsRef<[u8]>>(stem: &str, entries: &[(&str, B)]) -> SyntheticJar {
        let id = NEXT_TEST_JAR.fetch_add(1, Ordering::Relaxed);
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            ".jar-info-test-{}-{}-{id}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&dir).unwrap();
        let path = dir.join(format!("{stem}.jar"));
        let mut archive = zip::ZipWriter::new(fs::File::create(&path).unwrap());
        for (name, contents) in entries {
            archive
                .start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            archive.write_all(contents.as_ref()).unwrap();
        }
        archive.finish().unwrap();
        SyntheticJar(path, dir)
    }

    #[test]
    fn manifest_version_continuations_are_read_in_the_main_section_only() {
        let metadata = "[[mods]]\nmodId = 'ruby'\nversion = '${file.jarVersion}'\n";
        let folded = synthetic_jar(
            "folded-version",
            &[
                ("META-INF/neoforge.mods.toml", metadata),
                (
                    "META-INF/MANIFEST.MF",
                    "Manifest-Version: 1.0\r\nimplementation-version: 1.2-\r\n snapshot\r\nNext-Attribute: ignored\r\n\r\nName: file\r\nImplementation-Version: wrong\r\n",
                ),
            ],
        );
        assert_eq!(read(&folded.0).unwrap().version, "1.2-snapshot");
        assert_eq!(read_identity(&folded.0).unwrap().version, "1.2-snapshot");
        let lone_cr = synthetic_jar(
            "cr-version",
            &[
                ("META-INF/neoforge.mods.toml", metadata),
                (
                    "META-INF/MANIFEST.MF",
                    "Manifest-Version: 1.0\rImplementation-Version:  2.3-\r build\r\rName: file\rImplementation-Version: wrong\r",
                ),
            ],
        );
        assert_eq!(read(&lone_cr.0).unwrap().version, " 2.3-build");
        assert_eq!(read_identity(&lone_cr.0).unwrap().version, " 2.3-build");
        let named = synthetic_jar(
            "named-section-only",
            &[
                ("META-INF/neoforge.mods.toml", metadata),
                (
                    "META-INF/MANIFEST.MF",
                    "Manifest-Version: 1.0\r\n\r\nName: file\r\nImplementation-Version: wrong\r\n",
                ),
            ],
        );
        assert_eq!(read(&named.0).unwrap().version, "");
        assert!(read_identity(&named.0).is_err());
    }

    #[test]
    fn strict_identity_uses_loader_metadata_and_never_filename() {
        let fabric = synthetic_jar(
            "different-filename",
            &[("fabric.mod.json", r#"{"id":"fabric-api","version":"1.2"}"#)],
        );
        assert_eq!(
            read_identity(&fabric.0).unwrap(),
            JarIdentity {
                loader: JarLoader::Fabric,
                id: "fabric-api".into(),
                version: "1.2".into()
            }
        );
        let neo = synthetic_jar(
            "neo",
            &[
                (
                    "META-INF/neoforge.mods.toml",
                    "[[mods]]\nmodId = 'example_mod'\nversion = '${file.jarVersion}'\n",
                ),
                (
                    "META-INF/MANIFEST.MF",
                    "Manifest-Version: 1.0\nImplementation-Version: 3.2\n\n",
                ),
            ],
        );
        let identity = read_identity(&neo.0).unwrap();
        assert_eq!(
            (
                identity.loader,
                identity.id.as_str(),
                identity.version.as_str()
            ),
            (JarLoader::NeoForge, "example_mod", "3.2")
        );
        for (stem, entries) in [
            ("missing", vec![]),
            ("no-id", vec![("fabric.mod.json", r#"{"version":"1"}"#)]),
            (
                "bad-id",
                vec![("fabric.mod.json", r#"{"id":"-bad","version":"1"}"#)],
            ),
            (
                "uppercase",
                vec![("fabric.mod.json", r#"{"id":"Fabric","version":"1"}"#)],
            ),
            ("no-version", vec![("fabric.mod.json", r#"{"id":"good"}"#)]),
            (
                "neo-hyphen",
                vec![(
                    "META-INF/neoforge.mods.toml",
                    "[[mods]]\nmodId = 'bad-id'\nversion = '1'\n",
                )],
            ),
            (
                "neo-template",
                vec![(
                    "META-INF/neoforge.mods.toml",
                    "[[mods]]\nmodId = 'good'\nversion = '${version}'\n",
                )],
            ),
            (
                "legacy",
                vec![("mcmod.info", r#"[{"modid":"legacy","version":"1"}]"#)],
            ),
        ] {
            let jar = synthetic_jar(stem, &entries);
            assert!(read_identity(&jar.0).is_err(), "{stem}");
        }
    }

    #[test]
    fn unreadable_neoforge_metadata_is_not_absent_on_the_strict_route() {
        let jar = synthetic_jar(
            "bad-utf8",
            &[
                ("META-INF/neoforge.mods.toml", b"\xff".as_slice()),
                (
                    "fabric.mod.json",
                    br#"{"id":"fabric-api","version":"1"}"#.as_slice(),
                ),
            ],
        );
        assert_eq!(read(&jar.0).unwrap().mod_id, "fabric-api");
        let error = read_identity(&jar.0).unwrap_err();
        assert!(error.contains("META-INF/neoforge.mods.toml"), "{error}");
    }

    #[test]
    fn strict_metadata_does_not_apply_inventory_tolerance() {
        let malformed = synthetic_jar(
            "duplicate-key",
            &[(
                "META-INF/neoforge.mods.toml",
                "[[mods]]\nmodId = 'ruby'\nmodId = 'ruby'\nversion = '1'\n",
            )],
        );
        assert_eq!(read(&malformed.0).unwrap().mod_id, "ruby");
        assert!(read_identity(&malformed.0).is_err());
        let unresolved = synthetic_jar(
            "unresolved-version",
            &[
                (
                    "META-INF/neoforge.mods.toml",
                    "[[mods]]\nmodId = 'ruby'\nversion = '${unknown}'\n",
                ),
                ("META-INF/MANIFEST.MF", "Implementation-Version: 1.0\n\n"),
            ],
        );
        assert_eq!(read(&unresolved.0).unwrap().version, "1.0");
        assert!(read_identity(&unresolved.0).is_err());
    }

    #[test]
    fn fabric_versions_never_borrow_neoforges_manifest_placeholder() {
        let jar = synthetic_jar(
            "fabric-placeholder",
            &[
                (
                    "fabric.mod.json",
                    r#"{"id":"fabric-api","version":"${file.jarVersion}"}"#,
                ),
                ("META-INF/MANIFEST.MF", "Implementation-Version: 1.0\n\n"),
            ],
        );
        assert_eq!(read(&jar.0).unwrap().version, "1.0");
        assert!(read_identity(&jar.0).is_err());
    }

    #[test]
    fn reads_neoforge_metadata() {
        let info = mods_toml(
            r#"modLoader = "javafml"
license = "MIT"
[[mods]]
modId = "ruby"
version = "1.2.0"
displayName = "Ruby"
[[dependencies.ruby]]
modId = "neoforge"
type = "required"
[[dependencies.ruby]]
modId = "geckolib"
type = "required"
[[dependencies.ruby]]
modId = "jei"
type = "optional"
[[dependencies.ruby]]
modId = "curios"
mandatory = true
"#,
        )
        .unwrap();
        assert_eq!(
            (
                info.mod_id.as_str(),
                info.version.as_str(),
                info.license.as_str()
            ),
            ("ruby", "1.2.0", "MIT")
        );
        assert_eq!(info.depends, ["geckolib", "curios"]);
    }

    /// A jar whose only metadata file is an unfilled template (placeholder mod id and
    /// `[[dependencies.${id}]]`) is read line by line, not refused with the TOML error.
    #[test]
    fn lone_placeholder_template_is_not_a_toml_error() {
        let t = "[[mods]]
modId = \"${mod_id}\"

[[dependencies.${mod_id}]]
modId = \"neoforge\"
";
        assert!(mods_toml(t).is_err());
        let info = metadata(&[t.to_owned()]).unwrap();
        assert_eq!(info.mod_id, "${mod_id}");
        assert!(metadata(&[String::new()]).is_err());
    }

    /// Drip Sounds' Forge jar carries an unfilled NeoForge template beside its
    /// `mods.toml`; a file no parser reads still gives its id line by line.
    #[test]
    fn templates_and_broken_files_are_read_tolerantly() {
        let template = "license = \"LGPL\"\n[[mods]]\nmodId = \"${id}\"\nversion = \"${version}\"\n\n[[dependencies.${id}]]\nmodId = \"minecraft\"\n";
        let forge = "license = \"LGPL-3.0-only\"\n[[mods]]\nmodId = \"waterdripsound\"\ndisplayName = \"Drip Sounds\"\nversion = \"0.6.3\"\n[[dependencies.waterdripsound]]\nmodId = \"cloth_config\"\nmandatory = true\n";
        assert!(mods_toml(template).is_err());
        let info = metadata(&[template.to_owned(), forge.to_owned()]).unwrap();
        assert_eq!(
            (info.mod_id.as_str(), info.version.as_str()),
            ("waterdripsound", "0.6.3")
        );
        assert_eq!(info.depends, ["cloth_config"]);
        // Nothing parses: the lines still name the mod (a duplicate key is a TOML error).
        let broken = "license = 'MIT' # x\n[[mods]]\nmodId = \"ruby\"\nmodId = \"ruby\"\nversion = \"2\"\ndisplayName = \"Ruby\"\n[[mods]]\nmodId = \"other\"\n";
        assert!(mods_toml(broken).is_err());
        let info = metadata(&[template.to_owned(), broken.to_owned()]).unwrap();
        assert_eq!(
            (
                info.mod_id.as_str(),
                info.name.as_str(),
                info.license.as_str()
            ),
            ("ruby", "Ruby", "MIT")
        );
        // A lone template still names its placeholder id (see
        // `lone_placeholder_template_is_not_a_toml_error`).
        assert_eq!(metadata(&[template.to_owned()]).unwrap().mod_id, "${id}");
    }

    #[test]
    fn reads_fabric_metadata() {
        let info = fabric(
            r#"{"id": "gem", "version": "3", "name": "Gem", "license": ["MIT", "Apache-2.0"],
                "depends": {"fabricloader": "*", "fabric-api": "*", "minecraft": "1.21.1", "cloth-config": "*"}}"#,
        )
        .unwrap();
        assert_eq!(info.license, "MIT OR Apache-2.0");
        assert_eq!(info.depends, ["cloth-config"]);
    }

    #[test]
    fn reads_first_legacy_mod_from_root_array_without_inventing_license_or_dependencies() {
        let jar = synthetic_jar(
            "legacy-array",
            &[(
                "mcmod.info",
                r#"[
                    {"modid":"legacy_array","name":"Legacy Array","version":"1.2.3","requiredMods":["other_mod"]},
                    {"modid":"ignored_second","name":"Second Mod","version":"9.9"}
                ]"#,
            )],
        );

        let info = read(&jar.0).unwrap();

        assert_eq!(
            (
                info.mod_id.as_str(),
                info.name.as_str(),
                info.version.as_str()
            ),
            ("legacy_array", "Legacy Array", "1.2.3")
        );
        assert!(info.license.is_empty());
        assert!(
            info.depends.is_empty(),
            "legacy dependency fields are intentionally not interpreted"
        );
    }

    #[test]
    fn reads_first_legacy_mod_from_object_mod_list_and_keeps_unverified_license_empty() {
        let jar = synthetic_jar(
            "legacy-object",
            &[(
                "mcmod.info",
                r#"{
                    "modListVersion": 2,
                    "modList": [
                        {"modid":"legacy_object","name":"Legacy Object","version":"2.4","license":"unverified-custom"},
                        {"modid":"ignored_second","name":"Second Mod","version":"9.9"}
                    ]
                }"#,
            )],
        );

        let info = read(&jar.0).unwrap();

        assert_eq!(
            (
                info.mod_id.as_str(),
                info.name.as_str(),
                info.version.as_str()
            ),
            ("legacy_object", "Legacy Object", "2.4")
        );
        assert!(info.license.is_empty());
        assert!(info.depends.is_empty());
    }

    #[test]
    fn preserves_neoforge_forge_and_fabric_precedence_over_legacy_metadata() {
        let neoforge = synthetic_jar(
            "neoforge-precedence",
            &[
                (
                    "META-INF/neoforge.mods.toml",
                    "[[mods]]\nmodId = \"neo\"\nversion = \"neo-version\"\ndisplayName = \"NeoForge\"\n",
                ),
                (
                    "META-INF/mods.toml",
                    "[[mods]]\nmodId = \"forge\"\nversion = \"forge-version\"\ndisplayName = \"Forge\"\n",
                ),
                (
                    "fabric.mod.json",
                    r#"{"id":"fabric","name":"Fabric","version":"fabric-version"}"#,
                ),
                (
                    "mcmod.info",
                    r#"[{"modid":"legacy","name":"Legacy","version":"legacy-version"}]"#,
                ),
            ],
        );
        let forge = synthetic_jar(
            "forge-precedence",
            &[
                (
                    "META-INF/mods.toml",
                    "[[mods]]\nmodId = \"forge\"\nversion = \"forge-version\"\ndisplayName = \"Forge\"\n",
                ),
                (
                    "fabric.mod.json",
                    r#"{"id":"fabric","name":"Fabric","version":"fabric-version"}"#,
                ),
                (
                    "mcmod.info",
                    r#"[{"modid":"legacy","name":"Legacy","version":"legacy-version"}]"#,
                ),
            ],
        );
        let fabric = synthetic_jar(
            "fabric-precedence",
            &[
                (
                    "fabric.mod.json",
                    r#"{"id":"fabric","name":"Fabric","version":"fabric-version"}"#,
                ),
                (
                    "mcmod.info",
                    r#"[{"modid":"legacy","name":"Legacy","version":"legacy-version"}]"#,
                ),
            ],
        );

        let neo_info = read(&neoforge.0).unwrap();
        let forge_info = read(&forge.0).unwrap();
        let fabric_info = read(&fabric.0).unwrap();

        assert_eq!(
            (neo_info.mod_id.as_str(), neo_info.version.as_str()),
            ("neo", "neo-version")
        );
        assert_eq!(
            (forge_info.mod_id.as_str(), forge_info.version.as_str()),
            ("forge", "forge-version")
        );
        assert_eq!(
            (fabric_info.mod_id.as_str(), fabric_info.version.as_str()),
            ("fabric", "fabric-version")
        );
    }

    #[test]
    fn strict_identity_bounds_decompressed_metadata_without_changing_inventory() {
        for size in [1_048_576, 1_048_577] {
            let mut metadata = br#"{"id":"bounded","version":"1","description":""#.to_vec();
            metadata.resize(size - 2, b'x');
            metadata.extend_from_slice(b"\"}");
            let jar = synthetic_jar("metadata-boundary", &[("fabric.mod.json", metadata)]);
            let identity = read_identity(&jar.0);
            if size == 1_048_576 {
                let identity = identity.unwrap();
                assert_eq!(identity.id, "bounded");
                assert_eq!(identity.version, "1");
            } else {
                let error = identity.unwrap_err();
                assert!(
                    error.contains("fabric.mod.json: an entry unpacks to more than 1 MiB"),
                    "{error}"
                );
            }
            assert_eq!(read(&jar.0).unwrap().mod_id, "bounded");
        }
    }

    #[test]
    fn reads_legacy_metadata_with_literal_line_feeds_and_counts_the_contents() {
        let jar = synthetic_jar(
            "legacy-multiline",
            &[
                (
                    "mcmod.info",
                    r#"[{"modid":"legacy","version":"1.2.3","name":"Legacy \"Demo\"
résumé","description":"One line
another line"}]"#,
                ),
                ("legacy/Example.class", ""),
                ("assets/legacy/textures/item/example.png", ""),
            ],
        );

        let info = read(&jar.0).unwrap();

        assert_eq!(info.mod_id, "legacy");
        assert_eq!(info.version, "1.2.3");
        assert_eq!(info.name, "Legacy \"Demo\"\nrésumé");
        assert_eq!(info.census.classes, 1);
        assert_eq!(info.census.textures, 1);
    }

    #[test]
    fn malformed_legacy_json_is_an_error_instead_of_filename_fallback() {
        for metadata in [
            "[{not-json]",
            "[{\"name\":\"first\nsecond\tthird\"}]",
            "[{\"name\":\"first\\\nsecond\"}]",
        ] {
            let jar = synthetic_jar("malformed-legacy", &[("mcmod.info", metadata)]);

            let error = read(&jar.0).unwrap_err();

            assert!(error.starts_with("mcmod.info:"), "{error}");
        }
    }

    #[test]
    fn modern_fabric_metadata_still_rejects_literal_line_feeds() {
        let jar = synthetic_jar(
            "fabric-multiline",
            &[(
                "fabric.mod.json",
                "{\"id\":\"example\",\"name\":\"First\nSecond\",\"version\":\"1\"}",
            )],
        );

        let error = read_identity(&jar.0).unwrap_err();

        assert!(error.starts_with("fabric.mod.json:"), "{error}");
    }

    #[test]
    fn missing_or_empty_legacy_mod_id_uses_the_jar_filename() {
        let missing = synthetic_jar(
            "missing-id",
            &[(
                "mcmod.info",
                r#"[{"name":"No Identifier","version":"1.0"}]"#,
            )],
        );
        let empty = synthetic_jar(
            "empty-id",
            &[(
                "mcmod.info",
                r#"[{"modid":"","name":"Empty Identifier","version":"1.1"}]"#,
            )],
        );

        let missing_info = read(&missing.0).unwrap();
        let empty_info = read(&empty.0).unwrap();

        assert_eq!(missing_info.mod_id, "missing-id");
        assert_eq!(empty_info.mod_id, "empty-id");
        assert_eq!(missing_info.name, "No Identifier");
        assert_eq!(missing_info.version, "1.0");
        assert_eq!(empty_info.name, "Empty Identifier");
        assert_eq!(empty_info.version, "1.1");
    }

    #[test]
    fn resolves_legacy_version_placeholder_from_manifest() {
        let jar = synthetic_jar(
            "legacy-manifest-version",
            &[
                (
                    "mcmod.info",
                    r#"[{"modid":"legacy","name":"Legacy","version":"${file.jarVersion}"}]"#,
                ),
                (
                    "META-INF/MANIFEST.MF",
                    "Manifest-Version: 1.0\r\nImplementation-Version: 6.2.1\r\n\r\n",
                ),
            ],
        );

        let info = read(&jar.0).unwrap();

        assert_eq!(info.version, "6.2.1");
    }

    #[test]
    fn counts_files_by_kind() {
        let names = [
            "assets/ruby/blockstates/ruby_block.json",
            "assets/ruby/models/item/ruby.json",
            "assets/ruby/models/block/ruby_block.json",
            "assets/ruby/textures/item/ruby.png",
            "assets/ruby/lang/en_us.json",
            "data/ruby/recipe/ruby.json",
            "data/ruby/loot_table/blocks/ruby_block.json",
            "data/minecraft/tags/block/mineable/pickaxe.json",
            "data/ruby/structure/hut.nbt",
            "com/example/Ruby.class",
            "ruby.mixins.json",
        ]
        .map(String::from);
        let c = census(&names);
        assert_eq!(
            (c.blockstates, c.item_models, c.textures, c.languages),
            (1, 1, 1, 1)
        );
        assert_eq!(
            (c.recipes, c.loot_tables, c.tags, c.structures),
            (1, 1, 1, 1)
        );
        assert_eq!((c.classes, c.mixin_configs), (1, 1));
    }
}
