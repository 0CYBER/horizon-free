//! Resolve bundle dependencies without touching the staged workspace.
use crate::manifest::Side;
mod notices;
pub use notices::{Notice, NoticeKind};
use semver::{Version, VersionReq};
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::path::Path;

/// Parse an installed mod version, padding omitted minor and patch components.
/// This is also the comparison used for saved-world mod versions.
pub fn parse_version(version: &str) -> Result<Version, semver::Error> {
    let padded = match version.matches('.').count() {
        0 => format!("{version}.0.0"),
        1 => format!("{version}.0"),
        _ => version.to_owned(),
    };
    Version::parse(&padded)
}

#[derive(Debug)]
pub enum Error {
    Manifest {
        owner: String,
        target: String,
        reason: String,
    },
    Duplicate(String),
    Missing {
        owner: String,
        target: String,
        note: String,
    },
    NoApi {
        owner: String,
        target: String,
        note: String,
    },
    CargoConflict {
        owner: String,
        target: String,
    },
    Version {
        owner: String,
        target: String,
        version: String,
        requirement: String,
        note: String,
    },
    Cycle {
        mods: Vec<String>,
    },
    Incompatible {
        owner: String,
        target: String,
        version: String,
        /// The owner's reason and link on one logical line, or empty.
        note: String,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Manifest {
                owner,
                target,
                reason,
            } => write!(f, "{owner}: dependency {target}: {reason}"),
            Self::Duplicate(id) => write!(f, "Duplicate mod id {id:?}"),
            Self::Missing {
                owner,
                target,
                note,
            } => write!(f, "{owner} requires missing mod {target}{note}"),
            Self::NoApi {
                owner,
                target,
                note,
            } => {
                write!(
                    f,
                    "{owner} depends on {target}, which has no [api] package{note}"
                )
            }
            Self::CargoConflict { owner, target } => {
                write!(
                    f,
                    "{owner}: dependency {target} conflicts with its Cargo manifest"
                )
            }
            Self::Version {
                owner,
                target,
                version,
                requirement,
                note,
            } => write!(
                f,
                "{owner} requires {target} {requirement}, but installed version is {version}{note}"
            ),
            Self::Cycle { mods } => write!(f, "Mod dependency cycle: {}", mods.join(" -> ")),
            Self::Incompatible {
                owner,
                target,
                version,
                note,
            } => write!(f, "{owner} is incompatible with {target} {version}{note}"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Dependency {
    pub id: String,
    pub requirement: VersionReq,
    pub optional: bool,
    pub side: Side,
    pub reason: Option<String>,
    pub url: Option<String>,
    pub order: mod_vocab::DependencyOrder,
}

pub fn from_manifest(
    values: &BTreeMap<String, crate::manifest::Dependency>,
    running: Side,
) -> Vec<Dependency> {
    values
        .iter()
        .filter_map(|(id, value)| {
            let details = value.details();
            if running != Side::Both && details.side != Side::Both && details.side != running {
                return None;
            }
            Some(Dependency {
                id: id.clone(),
                requirement: details.version,
                optional: details.optional,
                side: details.side,
                reason: details.reason,
                url: details.url,
                order: details.order,
            })
        })
        .collect()
}

#[cfg(test)]
fn parse(owner: &str, value: Option<&toml::Value>) -> Result<Vec<Dependency>, Error> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let manifest = format!(
        "id='test'\nname='Test'\nversion='1'\napi='0.2'\ncapabilities=['registries']\n{}\n[native]\ngame='26.3'\nhorizon='{}'\nmixins='{}'\n",
        toml::to_string(&toml::Table::from_iter([(
            "dependencies".into(),
            value.clone()
        )]))
        .unwrap(),
        "a".repeat(40),
        "b".repeat(40)
    );
    let manifest =
        crate::manifest::Manifest::parse(&manifest).map_err(|reason| Error::Manifest {
            owner: owner.into(),
            target: "[dependencies]".into(),
            reason,
        })?;
    Ok(from_manifest(
        &manifest.dependencies,
        crate::manifest::Side::Both,
    ))
}

#[derive(Clone)]
pub struct Mod<'a> {
    pub id: &'a str,
    pub version: &'a str,
    pub has_api: bool,
    /// Code declarations also participate in physical library linking.
    pub code: bool,
    pub dependencies: &'a [Dependency],
}

#[derive(Debug)]
pub struct Resolution {
    /// Original bundle indices, in dispatch order.
    pub order: Vec<usize>,
    /// What the load rules let through: a discouraged pair, an override applied.
    pub warnings: Vec<Notice>,
}

/// Validate and order a mixed set of code and content declarations.
pub fn resolve(mods: &[Mod<'_>]) -> Result<Resolution, Error> {
    resolve_all(mods, &[], &Overrides::default(), false)
}

/// Physical library order, independent of manifest validation/dispatch overrides.
///
/// Every active code dependency still needs its API package. Version ranges and
/// dispatch order are logical rules; actual Cargo/wasm imports and ABI validation
/// remain the compiler/linker's responsibility and never fall back silently.
pub fn resolve_links(mods: &[Mod<'_>]) -> Result<Resolution, Error> {
    resolve_all(mods, &[], &Overrides::default(), true)
}

/// Validate the applicable installed set with its instance overrides, then return
/// logical dispatch order and structured notices for both affected Mods rows.
pub fn resolve_with_rules(
    mods: &[Mod<'_>],
    rules: &[Vec<Rule>],
    overrides: &Overrides,
) -> Result<Resolution, Error> {
    resolve_all(mods, rules, overrides, false)
}

/// Resolve one applicable installed manifest snapshot with one shared policy for
/// native, wasm and content packages. Indices in the result refer to `manifests`.
pub fn resolve_manifests(
    manifests: &[&crate::Manifest],
    running: Side,
    overrides: &Overrides,
) -> Result<Resolution, Error> {
    let declared: Vec<_> = manifests
        .iter()
        .map(|manifest| from_manifest(&manifest.dependencies, running))
        .collect();
    let inputs: Vec<_> = manifests
        .iter()
        .zip(&declared)
        .map(|(manifest, dependencies)| {
            let code = manifest.native.is_some() || manifest.wasm.is_some();
            Mod {
                id: &manifest.id,
                version: &manifest.version,
                has_api: code,
                code,
                dependencies,
            }
        })
        .collect();
    let rules: Vec<_> = manifests
        .iter()
        .map(|manifest| rules(manifest, running))
        .collect();
    resolve_with_rules(&inputs, &rules, overrides)
}

fn diagnostic_note(reason: Option<&str>, url: Option<&str>) -> String {
    // Diagnostics remain one logical line; only the Mods-page renderer adds line breaks.
    let reason = reason.map(|text| text.trim_end().replace(['\r', '\n'], " "));
    let url = url.map(|text| text.replace(['\r', '\n'], " "));
    match (reason.as_deref(), url.as_deref()) {
        (Some(reason), Some(url)) => format!(" ({reason} {url})"),
        (Some(text), None) | (None, Some(text)) => format!(" ({text})"),
        (None, None) => String::new(),
    }
}

/// What a load rule asks of another mod.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleKind {
    /// `[incompatible]`: refuse to start beside it.
    Incompatible,
    /// `[discouraged]`: start beside it with a warning.
    Discouraged,
    /// `load_before`: run before it when it is loaded.
    LoadBefore,
}

/// One of a mod's load rules about another mod, on the running side.
#[derive(Clone, Debug)]
pub struct Rule {
    pub kind: RuleKind,
    pub id: String,
    /// The other mod's versions the rule matches.
    pub requirement: VersionReq,
    /// Author-supplied text and link, retained separately from diagnostic layout.
    pub reason: Option<String>,
    pub url: Option<String>,
}

/// `manifest`'s load rules that hold on `running`, the client or the dedicated server.
pub fn rules(manifest: &crate::Manifest, running: Side) -> Vec<Rule> {
    fn declared<'a>(
        kind: RuleKind,
        table: &'a BTreeMap<String, crate::manifest::Rule>,
        running: Side,
    ) -> impl Iterator<Item = Rule> + 'a {
        table.iter().filter_map(move |(id, rule)| {
            let rule = rule.details();
            (running == Side::Both || rule.side == Side::Both || rule.side == running).then(|| {
                Rule {
                    kind,
                    id: id.clone(),
                    requirement: rule.version,
                    reason: rule.reason,
                    url: rule.url,
                }
            })
        })
    }
    declared(RuleKind::Incompatible, &manifest.incompatible, running)
        .chain(declared(
            RuleKind::Discouraged,
            &manifest.discouraged,
            running,
        ))
        .chain(manifest.load_before.iter().map(|id| Rule {
            kind: RuleKind::LoadBefore,
            id: id.clone(),
            requirement: VersionReq::STAR,
            reason: None,
            url: None,
        }))
        .collect()
}

/// What an instance's overrides file does to one mod's rule about another (NeoForge's
/// `FMLConfig.DependencyOverride`).
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Override {
    /// Ignore the rule.
    Drop,
    /// Load beside an incompatible mod, with a warning.
    Discourage,
}

/// The overrides file's name, in the instance's `horizon-mods` folder.
pub const OVERRIDES: &str = "dependency-overrides.toml";

/// An instance's overrides of the mods' load rules: `[owner]` tables of `target = "drop"` or
/// `"discourage"`.
#[derive(Debug, Default, Deserialize)]
#[serde(transparent)]
pub struct Overrides(BTreeMap<String, BTreeMap<String, Override>>);

impl Overrides {
    /// The overrides file in `mods_dir`; none when there is no file.
    pub fn read(mods_dir: &Path) -> Result<Self, String> {
        let path = mods_dir.join(OVERRIDES);
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                let overrides: Self =
                    toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
                for (owner, targets) in &overrides.0 {
                    for id in std::iter::once(owner).chain(targets.keys()) {
                        if !crate::manifest::valid_mod_id(id) {
                            return Err(format!(
                                "{}: invalid dependency override mod id {id:?}",
                                path.display()
                            ));
                        }
                    }
                }
                Ok(overrides)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    fn get(&self, owner: &str, target: &str) -> Option<Override> {
        self.0.get(owner)?.get(target).copied()
    }
}

fn resolve_all(
    mods: &[Mod<'_>],
    rules: &[Vec<Rule>],
    overrides: &Overrides,
    links: bool,
) -> Result<Resolution, Error> {
    let mut ids = HashMap::new();
    for (index, item) in mods.iter().enumerate() {
        if ids.insert(item.id, index).is_some() {
            return Err(Error::Duplicate(item.id.into()));
        }
    }
    let mut edges = vec![Vec::new(); mods.len()];
    let mut warnings = Vec::new();
    // A target's version is parsed only when a logical rule uses it, once per resolution.
    let mut versions = HashMap::new();
    for (index, item) in mods.iter().enumerate() {
        for dep in item.dependencies {
            let target = ids.get(dep.id.as_str()).copied();
            let note = diagnostic_note(dep.reason.as_deref(), dep.url.as_deref());
            if !links && let Some(action) = overrides.get(item.id, &dep.id) {
                warnings.push(Notice {
                    owner: item.id.into(),
                    target: dep.id.clone(),
                    version: target.map(|i| mods[i].version.to_owned()),
                    reason: dep.reason.clone(),
                    url: dep.url.clone(),
                    kind: match action {
                        Override::Drop => NoticeKind::OverrideDropped,
                        Override::Discourage => NoticeKind::OverrideDiscouraged,
                    },
                });
                // These remove validation and dispatch edges only. The separate
                // physical resolution still enforces actual library requirements.
                continue;
            }
            let Some(target) = target else {
                if dep.optional {
                    continue;
                }
                return Err(Error::Missing {
                    owner: item.id.into(),
                    target: dep.id.clone(),
                    note,
                });
            };
            let other = &mods[target];
            if item.code && (!item.has_api || !other.has_api) {
                return Err(Error::NoApi {
                    owner: item.id.into(),
                    target: dep.id.clone(),
                    note,
                });
            }
            if !links {
                let version = versions
                    .entry(target)
                    .or_insert_with(|| parse_version(other.version));
                let version = version.as_ref().map_err(|error| Error::Manifest {
                    owner: item.id.into(),
                    target: dep.id.clone(),
                    reason: format!("invalid installed version {:?}: {error}", other.version),
                })?;
                if !dep.requirement.matches(version) {
                    return Err(Error::Version {
                        owner: item.id.into(),
                        target: dep.id.clone(),
                        version: other.version.into(),
                        requirement: dep.requirement.to_string(),
                        note,
                    });
                }
            }
            use mod_vocab::DependencyOrder;
            if links || dep.order == DependencyOrder::After {
                edges[index].push(target);
            } else if dep.order == DependencyOrder::Before {
                edges[target].push(index);
            } else if dep.order != DependencyOrder::None {
                return Err(Error::Manifest {
                    owner: item.id.into(),
                    target: dep.id.clone(),
                    reason: "unsupported dependency order".into(),
                });
            }
        }
    }
    for (index, item) in mods.iter().enumerate() {
        for rule in rules.get(index).into_iter().flatten() {
            let target = ids.get(rule.id.as_str()).copied();
            let mut kind = rule.kind;
            if let Some(action) = overrides.get(item.id, &rule.id) {
                warnings.push(Notice {
                    owner: item.id.into(),
                    target: rule.id.clone(),
                    version: target.map(|i| mods[i].version.to_owned()),
                    reason: rule.reason.clone(),
                    url: rule.url.clone(),
                    kind: match action {
                        Override::Drop => NoticeKind::OverrideDropped,
                        Override::Discourage => NoticeKind::OverrideDiscouraged,
                    },
                });
                if action == Override::Drop || kind == RuleKind::LoadBefore {
                    continue;
                }
                kind = RuleKind::Discouraged;
            }
            let Some(target) = target else {
                continue;
            };
            if kind == RuleKind::LoadBefore {
                edges[target].push(index);
                continue;
            }
            let other = &mods[target];
            let version = versions
                .entry(target)
                .or_insert_with(|| parse_version(other.version));
            let version = version.as_ref().map_err(|error| Error::Manifest {
                owner: item.id.into(),
                target: rule.id.clone(),
                reason: format!("invalid installed version {:?}: {error}", other.version),
            })?;
            if !rule.requirement.matches(version) {
                continue;
            }
            if kind == RuleKind::Incompatible {
                return Err(Error::Incompatible {
                    owner: item.id.into(),
                    target: rule.id.clone(),
                    version: other.version.into(),
                    note: diagnostic_note(rule.reason.as_deref(), rule.url.as_deref()),
                });
            }
            warnings.push(Notice {
                owner: item.id.into(),
                target: rule.id.clone(),
                version: Some(other.version.into()),
                reason: rule.reason.clone(),
                url: rule.url.clone(),
                kind: NoticeKind::Discouraged,
            });
        }
    }
    let mut done = vec![false; mods.len()];
    let mut order = Vec::with_capacity(mods.len());
    while order.len() < mods.len() {
        let next = (0..mods.len()).find(|&i| !done[i] && edges[i].iter().all(|&d| done[d]));
        let Some(next) = next else {
            // Follow remaining edges until a node repeats, reporting the actual cycle only.
            let mut seen = BTreeMap::new();
            let mut path: Vec<usize> = Vec::new();
            let mut current = done.iter().position(|done| !done).expect("unfinished node");
            loop {
                if let Some(&start) = seen.get(&current) {
                    let mut cycle: Vec<String> = path[start..]
                        .iter()
                        .map(|&i| mods[i].id.to_owned())
                        .collect();
                    cycle.push(mods[current].id.to_owned());
                    return Err(Error::Cycle { mods: cycle });
                }
                seen.insert(current, path.len());
                path.push(current);
                current = *edges[current]
                    .iter()
                    .find(|&&i| !done[i])
                    .expect("unfinished dependency");
            }
        };
        done[next] = true;
        order.push(next);
    }
    Ok(Resolution { order, warnings })
}

/// A target's staged Cargo package, or none for an absent optional mod.
pub struct Package<'a> {
    pub dependency: &'a Dependency,
    pub package: Option<&'a str>,
}

/// Add bundle edges to the dependent package. Returns features to enable on the game edge.
pub fn rewrite(
    owner: &str,
    manifest: &mut toml::Table,
    packages: &[Package<'_>],
) -> Result<Vec<String>, Error> {
    let mut updated = manifest.clone();
    let mut enabled = Vec::new();
    for target in packages {
        let id = &target.dependency.id;
        let normalized = id.replace('-', "_");
        let conflict = || Error::CargoConflict {
            owner: owner.into(),
            target: id.clone(),
        };
        let dependencies = updated
            .entry("dependencies")
            .or_insert_with(|| toml::Value::Table(toml::Table::new()))
            .as_table_mut()
            .ok_or_else(conflict)?;
        if dependencies
            .keys()
            .any(|key| key.replace('-', "_") == normalized)
        {
            return Err(conflict());
        }
        if let Some(package) = target.package {
            let mut entry = toml::Table::new();
            entry.insert("path".into(), format!("../{id}").into());
            entry.insert("package".into(), package.into());
            if target.dependency.optional {
                entry.insert("optional".into(), true.into());
            }
            dependencies.insert(id.clone(), toml::Value::Table(entry));
        } else if !target.dependency.optional {
            return Err(Error::Missing {
                owner: owner.into(),
                target: id.clone(),
                note: diagnostic_note(
                    target.dependency.reason.as_deref(),
                    target.dependency.url.as_deref(),
                ),
            });
        }
        if target.dependency.optional {
            let features = updated
                .entry("features")
                .or_insert_with(|| toml::Value::Table(toml::Table::new()))
                .as_table_mut()
                .ok_or_else(conflict)?;
            if features.contains_key(id)
                || features.values().any(|value| {
                    value.as_array().is_some_and(|entries| {
                        entries.iter().any(|entry| {
                            entry.as_str().is_some_and(|s| {
                                s == id
                                    || s == format!("dep:{id}")
                                    || s.starts_with(&format!("{id}/"))
                                    || s.starts_with(&format!("{id}?/"))
                            })
                        })
                    })
                })
            {
                return Err(conflict());
            }
            let entries = if target.package.is_some() {
                enabled.push(id.clone());
                vec![toml::Value::String(format!("dep:{id}"))]
            } else {
                Vec::new()
            };
            features.insert(id.clone(), toml::Value::Array(entries));
        }
    }
    *manifest = updated;
    Ok(enabled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_preserve_author_punctuation_without_forced_line_breaks() {
        assert_eq!(
            diagnostic_note(
                Some("Keep the complete author sentence.  "),
                Some("https://example.test/details")
            ),
            " (Keep the complete author sentence. https://example.test/details)"
        );
        let notice = Notice {
            owner: "owner".into(),
            target: "target".into(),
            version: Some("1.0.0".into()),
            reason: Some("First sentence.\nSecond sentence.".into()),
            url: Some("https://example.test/details".into()),
            kind: NoticeKind::Discouraged,
        };
        assert_eq!(
            notice.to_string(),
            "owner works badly with target 1.0.0 (First sentence. Second sentence. https://example.test/details)"
        );
    }
    #[test]
    fn diagnostic_url_line_breaks_do_not_reach_refusals_or_logs() {
        let url = "https://example.test/\r\ndetails";
        let refusal = Error::Incompatible {
            owner: "owner".into(),
            target: "target".into(),
            version: "1".into(),
            note: diagnostic_note(Some("Reason."), Some(url)),
        };
        assert_eq!(
            refusal.to_string(),
            "owner is incompatible with target 1 (Reason. https://example.test/  details)"
        );
        let notice = Notice {
            owner: "owner".into(),
            target: "target".into(),
            version: Some("1".into()),
            reason: Some("Reason.".into()),
            url: Some(url.into()),
            kind: NoticeKind::Discouraged,
        };
        assert_eq!(
            notice.to_string(),
            "owner works badly with target 1 (Reason. https://example.test/  details)"
        );
    }

    fn deps(text: &str) -> Vec<Dependency> {
        let value: toml::Table = text.parse().unwrap();
        parse("user", value.get("dependencies")).unwrap()
    }
    fn item<'a>(id: &'a str, version: &'a str, dependencies: &'a [Dependency]) -> Mod<'a> {
        Mod {
            id,
            version,
            has_api: true,
            code: true,
            dependencies,
        }
    }
    #[test]
    fn a_client_only_required_dependency_does_not_refuse_a_dedicated_installation() {
        let manifest = crate::Manifest::parse("id='server'\nname='Server'\nversion='1'\napi='0.2'\ncapabilities=['registries']\n[dependencies]\nclient={version='^1',side='client',order='none',reason='Client integration',url='https://example.test/client'}").unwrap();
        let dedicated = from_manifest(&manifest.dependencies, Side::Server);
        assert_eq!(
            resolve(&[item("server", "1", &dedicated)]).unwrap().order,
            [0]
        );
        let client = from_manifest(&manifest.dependencies, Side::Client);
        assert_eq!(client.len(), 1);
        assert_eq!(client[0].reason.as_deref(), Some("Client integration"));
        assert_eq!(
            client[0].url.as_deref(),
            Some("https://example.test/client")
        );
        assert_eq!(client[0].order, mod_vocab::DependencyOrder::None);
        assert!(
            matches!(resolve(&[item("server", "1", &client)]), Err(Error::Missing { owner, target, .. }) if owner == "server" && target == "client")
        );
    }

    #[test]
    fn detailed_dependency_and_rule_defaults_apply_to_dedicated_installations() {
        let source = manifest(
            "owner",
            "[dependencies]\nbase={version='^1'}\n[incompatible]\nbad={version='*'}",
        );
        let declared = from_manifest(&source.dependencies, Side::Server);
        assert_eq!(declared.len(), 1);
        assert_eq!(declared[0].side, Side::Both);
        let active = rules(&source, Side::Server);
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, "bad");
    }

    #[test]
    fn declaration_boundaries_apply_to_dependencies_and_pair_rules() {
        for table in ["dependencies", "incompatible", "discouraged"] {
            let manifest = |entry: &str| {
                crate::Manifest::parse(&format!(
                    "id='owner'\nname='Owner'\nversion='1'\napi='0.2'\ncapabilities=['registries']\n[{table}]\n{entry}"
                ))
            };
            assert!(
                manifest(&format!(
                    "target={{version='^1',reason='{}',url='https://example.test'}}",
                    "a".repeat(512)
                ))
                .is_ok()
            );
            let error = manifest(&format!(
                "target={{version='^1',reason='{}'}}",
                "a".repeat(513)
            ))
            .unwrap_err();
            assert!(error.contains("target") && error.contains("reason over 512 bytes"));
            assert!(
                manifest("target={version='^1',url='http://example.test'}")
                    .unwrap_err()
                    .contains("url is not https://")
            );
            assert!(
                manifest("'bad id'='^1'")
                    .unwrap_err()
                    .contains("invalid dependency id")
            );
            assert!(manifest("target='not a semver requirement'").is_err());
        }
    }

    #[test]
    fn override_files_validate_ids_and_include_the_path_in_read_and_parse_errors() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join(OVERRIDES);
        assert_eq!(
            Overrides::read(home.path()).unwrap().get("owner", "target"),
            None
        );
        std::fs::write(&path, "[owner]\ntarget='drop'").unwrap();
        assert_eq!(
            Overrides::read(home.path()).unwrap().get("owner", "target"),
            Some(Override::Drop)
        );
        for text in [
            "[",
            "['bad owner']\ntarget='drop'",
            "[owner]\n'bad target'='drop'",
            "[owner]\ntarget='unknown action'",
        ] {
            std::fs::write(&path, text).unwrap();
            let error = Overrides::read(home.path()).unwrap_err();
            assert!(error.contains(path.to_str().unwrap()), "{error}");
            if text.contains("bad ") {
                assert!(error.contains("invalid dependency override mod id"));
            }
        }
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        let error = Overrides::read(home.path()).unwrap_err();
        assert!(error.contains(path.to_str().unwrap()));
    }

    #[test]
    fn content_dependencies_validate_and_order_without_native_api_packages() {
        let manifest = |id: &str, version: &str, dependencies: &str| {
            crate::Manifest::parse(&format!(
                "id='{id}'\nname='{id}'\nversion='{version}'\napi='0.2'\ncapabilities=['registries']\n{dependencies}"
            )).unwrap()
        };
        let addon = manifest(
            "addon",
            "1",
            "[dependencies]\nbase='^1'\nextra={version='^2', optional=true}",
        );
        let base = manifest("base", "1.2", "");
        let dependencies = from_manifest(&addon.dependencies, crate::manifest::Side::Both);
        let mods = [
            Mod {
                id: &addon.id,
                version: &addon.version,
                has_api: false,
                code: false,
                dependencies: &dependencies,
            },
            Mod {
                id: &base.id,
                version: &base.version,
                has_api: false,
                code: false,
                dependencies: &[],
            },
        ];
        assert_eq!(resolve(&mods).unwrap().order, [1, 0]);
        assert!(matches!(resolve(&mods[..1]), Err(Error::Missing { .. })));
        let incompatible = Mod {
            id: "base",
            version: "2",
            has_api: false,
            code: false,
            dependencies: &[],
        };
        assert!(matches!(
            resolve(&[mods[0].clone(), incompatible]),
            Err(Error::Version { .. })
        ));
        let back = from_manifest(
            &manifest("base", "1", "[dependencies]\naddon='^1'").dependencies,
            crate::manifest::Side::Both,
        );
        assert!(matches!(
            resolve(&[
                mods[0].clone(),
                Mod {
                    id: "base",
                    version: "1",
                    has_api: false,
                    code: false,
                    dependencies: &back
                }
            ]),
            Err(Error::Cycle { .. })
        ));
    }

    #[test]
    fn load_rules_refuse_warn_order_and_yield_to_overrides() {
        let header =
            "id='user'\nname='User'\nversion='1'\napi='0.2'\ncapabilities=['registries']\n";
        let user = crate::Manifest::parse(&format!(
            "{header}load_before=['late', 'absent']\n[incompatible]\n\
             broken={{version='<2', reason='Corrupts saves', url='https://example.com/why'}}\n\
             client_only={{side='client'}}\n[discouraged]\nslow='*'\n"
        ))
        .unwrap();
        let server = rules(&user, Side::Server);
        let client = rules(&user, Side::Client);
        let none = Overrides::default();
        let with = |others: &[(&'static str, &'static str)], rules: &[Rule], overrides| {
            let mut mods = vec![item("user", "1", &[])];
            mods.extend(others.iter().map(|&(id, version)| item(id, version, &[])));
            let rules = [rules.to_vec()];
            resolve_with_rules(&mods, &rules, overrides)
        };
        let refused = with(&[("broken", "1.5")], &server, &none).unwrap_err();
        assert_eq!(
            refused.to_string(),
            "user is incompatible with broken 1.5 (Corrupts saves https://example.com/why)"
        );
        assert!(with(&[("broken", "2.0")], &server, &none).is_ok());
        // A client-only rule holds nothing on a dedicated server.
        assert!(with(&[("client_only", "1")], &server, &none).is_ok());
        assert!(with(&[("client_only", "1")], &client, &none).is_err());
        let slow = with(&[("slow", "3")], &server, &none).unwrap();
        assert_eq!(
            slow.warnings
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            ["user works badly with slow 3"]
        );
        // Before a present mod it doesn't depend on; an absent one is ignored.
        let mods = [item("late", "1", &[]), item("user", "1", &[])];
        let rules = [Vec::new(), server.clone()];
        assert_eq!(
            resolve_with_rules(&mods, &rules, &none).unwrap().order,
            [1, 0]
        );
        let discourage: Overrides = toml::from_str("[user]\nbroken = 'discourage'\n").unwrap();
        let warned = with(&[("broken", "1.5")], &server, &discourage).unwrap();
        assert_eq!(
            warned
                .warnings
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            [
                "dependency override: user -> broken only discouraged",
                "user works badly with broken 1.5 (Corrupts saves https://example.com/why)"
            ]
        );
        let drop: Overrides = toml::from_str("[user]\nbroken = 'drop'\n").unwrap();
        let dropped = with(&[("broken", "1.5")], &server, &drop).unwrap();
        assert_eq!(
            dropped
                .warnings
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            ["dependency override: user -> broken dropped"]
        );
        for refused in [
            "[incompatible]\nx={url='http://example.com'}",
            "load_before=['Bad']",
        ] {
            assert!(crate::Manifest::parse(&format!("{header}{refused}")).is_err());
        }
    }

    #[test]
    fn dispatch_order_and_validation_overrides_never_replace_the_link_graph() {
        for order in ["none", "before"] {
            let declared = deps(&format!(
                "[dependencies]\nbase={{version='^1',order='{order}'}}"
            ));
            let mods = [item("user", "1", &declared), item("base", "1", &[])];
            assert_eq!(resolve(&mods).unwrap().order, [0, 1]);
            assert_eq!(resolve_links(&mods).unwrap().order, [1, 0]);
        }
        let declared = deps(
            "[dependencies]\nbase={version='^2',reason='Integration',url='https://example.test/base'}",
        );
        let mods = [item("user", "1", &declared), item("base", "1", &[])];
        assert!(matches!(resolve(&mods), Err(Error::Version { .. })));
        let dropped: Overrides = toml::from_str("[user]\nbase='drop'").unwrap();
        let logical = resolve_with_rules(&mods, &[], &dropped).unwrap();
        assert_eq!(logical.order, [0, 1]);
        assert_eq!(logical.warnings[0].kind, NoticeKind::OverrideDropped);
        assert_eq!(logical.warnings[0].reason.as_deref(), Some("Integration"));
        assert_eq!(
            logical.warnings[0].url.as_deref(),
            Some("https://example.test/base")
        );
        assert_eq!(resolve_links(&mods).unwrap().order, [1, 0]);
        assert_eq!(
            resolve_with_rules(&mods[..1], &[], &dropped).unwrap().order,
            [0]
        );
        assert!(
            matches!(resolve_links(&mods[..1]), Err(Error::Missing { owner, target, .. }) if owner == "user" && target == "base")
        );
    }

    fn manifest(id: &str, extra: &str) -> crate::Manifest {
        crate::Manifest::parse(&format!(
            "id='{id}'\nname='{id}'\nversion='1'\napi='0.2'\ncapabilities=['registries']\n{extra}"
        ))
        .unwrap()
    }

    #[test]
    fn content_and_mixed_rules_share_refusal_without_turning_content_into_a_library() {
        let owner = manifest(
            "owner",
            "[incompatible]\nother={version='*',reason='Bad interaction',url='https://example.test/why'}",
        );
        let plain = manifest("other", "");
        let code = manifest(
            "other",
            &format!(
                "[native]\ngame='26.3'\nhorizon='{}'\nmixins='{}'",
                "a".repeat(40),
                "b".repeat(40)
            ),
        );
        for other in [&plain, &code] {
            let error = resolve_manifests(&[&owner, other], Side::Client, &Overrides::default())
                .unwrap_err();
            assert_eq!(
                error.to_string(),
                "owner is incompatible with other 1 (Bad interaction https://example.test/why)"
            );
        }
        let mut dependent = manifest("dependent", "[dependencies]\nother='^1'");
        assert_eq!(
            resolve_manifests(&[&dependent, &code], Side::Client, &Overrides::default())
                .unwrap()
                .order,
            [1, 0]
        );
        dependent.native = code.native.clone();
        assert!(
            matches!(resolve_manifests(&[&dependent, &plain], Side::Client, &Overrides::default()), Err(Error::NoApi { owner, target, .. }) if owner == "dependent" && target == "other")
        );
    }

    #[test]
    fn a_content_cycle_reports_its_literal_path_without_unrelated_nodes() {
        let a = manifest("a", "[dependencies]\nb='^1'");
        let b = manifest("b", "[dependencies]\na='^1'");
        let free = manifest("free", "");
        assert_eq!(
            resolve_manifests(&[&a, &b, &free], Side::Client, &Overrides::default())
                .unwrap_err()
                .to_string(),
            "Mod dependency cycle: a -> b -> a"
        );
        let dropped: Overrides = toml::from_str("[a]\nb='drop'").unwrap();
        assert_eq!(
            resolve_manifests(&[&a, &b, &free], Side::Client, &dropped)
                .unwrap()
                .order,
            [0, 1, 2]
        );
    }

    #[test]
    fn order_and_ties_include_present_optional() {
        let user = deps("[dependencies]\nbase = '^1'\nextra = { version = '^2', optional = true }");
        let mods = [
            item("user", "1.0.0", &user),
            item("base", "1.2", &[]),
            item("extra", "2", &[]),
            item("other", "1.0.0", &[]),
        ];
        let result = resolve(&mods).unwrap();
        assert_eq!(result.order, [1, 2, 0, 3]);
        assert_eq!(
            resolve(&[item("user", "1.0.0", &user), item("base", "1.2.0", &[])])
                .unwrap()
                .order,
            [1, 0]
        );
    }
    #[test]
    fn failures_name_mods() {
        let required = deps("[dependencies]\nbase = '^1'");
        let optional = deps("[dependencies]\nbase = { version = '^1', optional = true }");
        for dependencies in [&required, &optional] {
            let err = resolve(&[
                item("user", "1.0.0", dependencies),
                item("base", "2.0.0", &[]),
            ])
            .err()
            .unwrap()
            .to_string();
            assert!(err.contains("user") && err.contains("base"));
        }
        let err = resolve(&[item("user", "1.0.0", &required)])
            .err()
            .unwrap()
            .to_string();
        assert!(err.contains("user") && err.contains("base"));
        let back = deps("[dependencies]\nuser = '^1'");
        let err = resolve(&[
            item("user", "1.0.0", &required),
            item("base", "1.0.0", &back),
        ])
        .err()
        .unwrap()
        .to_string();
        assert!(err.contains("user") && err.contains("base"));
    }
    #[test]
    fn rewrites_installed_and_absent_optional_dependencies() {
        let deps = deps("[dependencies]\nbase = '^1'\nextra = { version = '^2', optional = true }");
        let mut manifest: toml::Table = "[dependencies]\nhorizon-mod-api = '1'".parse().unwrap();
        let packages = [
            Package {
                dependency: &deps[0],
                package: Some("base-package"),
            },
            Package {
                dependency: &deps[1],
                package: Some("extra-package"),
            },
        ];
        assert_eq!(
            rewrite("user", &mut manifest, &packages).unwrap(),
            ["extra"]
        );
        assert_eq!(
            manifest["dependencies"]["base"]["path"].as_str(),
            Some("../base")
        );
        assert_eq!(
            manifest["dependencies"]["base"]["package"].as_str(),
            Some("base-package")
        );
        assert_eq!(
            manifest["dependencies"]["extra"]["optional"].as_bool(),
            Some(true)
        );
        assert_eq!(manifest["features"]["extra"][0].as_str(), Some("dep:extra"));
        let mut absent = toml::Table::new();
        assert!(
            rewrite(
                "user",
                &mut absent,
                &[Package {
                    dependency: &deps[1],
                    package: None
                }]
            )
            .unwrap()
            .is_empty()
        );
        assert!(absent["features"]["extra"].as_array().unwrap().is_empty());
        assert!(absent["dependencies"].get("extra").is_none());
    }

    #[test]
    fn rejects_conflicting_features_and_dependency_names_atomically() {
        let deps = deps("[dependencies]\nextra = { version = '^2', optional = true }");
        for text in [
            "[features]\ndefault = ['extra']",
            "[dependencies]\nextra = '1'",
        ] {
            let mut manifest: toml::Table = text.parse().unwrap();
            let original = manifest.clone();
            assert!(
                rewrite(
                    "user",
                    &mut manifest,
                    &[Package {
                        dependency: &deps[0],
                        package: Some("extra-package")
                    }]
                )
                .is_err()
            );
            assert_eq!(manifest, original);
        }
    }

    #[test]
    fn malformed_and_no_api() {
        let value: toml::Table = "[dependencies]\nbase = { version = '^1', optional = 'yes' }"
            .parse()
            .unwrap();
        assert!(parse("user", value.get("dependencies")).is_err());
        let required = deps("[dependencies]\nbase = '^1'");
        let mut base = item("base", "1.0.0", &[]);
        base.has_api = false;
        assert!(matches!(
            resolve(&[item("user", "1.0.0", &required), base]),
            Err(Error::NoApi { .. })
        ));
    }
}
