//! The launcher's generated executable for trait mods.
use super::{internals, Bundle};
use std::fs;
use std::path::Path;

pub(super) fn read(path: &Path) -> Result<toml::Table, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    text.parse().map_err(|e| format!("{}: {e}", path.display()))
}

fn write(path: &Path, value: &toml::Table) -> Result<(), String> {
    let text = toml::to_string(value).map_err(|e| format!("{}: {e}", path.display()))?;
    fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

pub(super) const PACKAGE: &str = "horizon-native-game";
/// The official game's package and executable (docs/layers.md, "How a Horizon crate plugs in"):
/// horizon-client's game with Horizon's own parts, the one the release ships as `horizon-client`.
pub(super) const OFFICIAL: &str = "horizon-official";

/// Whether the staged revision has the official game. One from before it ships horizon-client's
/// executable, as the release did then.
pub(super) fn has_official(stage: &Path) -> bool {
    stage.join("crates/horizon-official/Cargo.toml").is_file()
}

/// The official game, once built into `out`, under the game's name, as the release ships it.
/// Where the generated executable's package is staged, in the workspace.
const GAME: &str = "crates/native-game";

fn path_dep(path: &str) -> toml::Table {
    toml::Table::from_iter([("path".into(), path.into())])
}

const MAIN_TEMPLATE: &str = "crates/horizon-client/src/native_main.rs.template";

pub(super) fn template(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).ok()
        .filter(|template| ["/* MOD_ENTRIES */", "/* MOD_SLOTS */", "/* BUILT_MODS */"].iter()
            .all(|marker| template.matches(marker).count() == 1))
        .ok_or_else(|| crate::i18n::tr("This game source has no native inventory entry template. Update Horizon and rebuild the mods against this game.").to_string())
}

pub(super) fn main_rs(
    template: &str,
    mods: &[&horizon_mod_package::Manifest],
    catalog: &internals::Catalog,
) -> String {
    // Own the bytes even when the staged SDK template references a static in a dylib.
    let template = template.replace(
        "std::hint::black_box(&horizon_mod_package::holder::CAPABILITY);",
        "#[used] static HOLDER_CAPABILITY: [u8; horizon_mod_package::holder::CAPABILITY.len()] = horizon_mod_package::holder::CAPABILITY; std::hint::black_box(&HOLDER_CAPABILITY);",
    );
    let listed = &catalog.points;
    let entries = mods
        .iter()
        .enumerate()
        .map(|(i, m)| format!("        {:?} => mod_{i}::hooks,\n", m.id))
        .collect::<String>();
    let inventory = mods.iter().map(|m| format!(
        "    mod_api::__private::native::BuiltMod {{ id: {:?}, version: {:?}, side: mod_api::Side::{:?} }},\n",
        m.id, m.version, m.side,
    )).collect::<String>();
    let api_points = listed
        .iter()
        .map(super::inject::ListedPoint::emitted)
        .collect::<Vec<_>>();
    let raw = mods
        .iter()
        .enumerate()
        .flat_map(|(i, manifest)| {
            horizon_mod_point_emit::api(manifest, &api_points, &format!("mod_{i}::"))
                .expect("API slots checked")
                .into_iter()
                .map(|expression| format!("        {expression},\n"))
        })
        .collect::<String>();
    let markers = [
        ("/* MOD_SLOTS */", &raw),
        ("/* MOD_ENTRIES */", &entries),
        ("/* BUILT_MODS */", &inventory),
    ];
    template
        .split_inclusive("*/")
        .map(|chunk| {
            markers
                .iter()
                .find_map(|(marker, value)| {
                    chunk
                        .strip_suffix(*marker)
                        .map(|prefix| prefix.trim_end_matches([' ', '\t']).to_owned() + value)
                })
                .unwrap_or_else(|| chunk.to_owned())
        })
        .collect::<String>()
}

/// Parsed once from the pipeline's immutable incoming copy, then reused for staging.
pub(super) struct Project<'a> {
    pub(super) bundle: &'a Bundle,
    pub(super) cargo: toml::Table,
    pub(super) package: String,
    api_deps: Vec<ApiDependency>,
}

/// The mod API's package, the one crate every mod depends on.
pub(super) const POINTS_PACKAGE: &str = "horizon-mod-points";
pub(super) const API_PACKAGE: &str = "horizon-mod-api";

/// The per-mod facade's package, rewritten to its generated crate.
pub(super) const INTERNALS_PACKAGE: &str = "horizon-internals";
/// Each game package a mod may depend on, with its library's name.
pub(super) const PACKAGES: [(&str, &str); 3] = [
    (API_PACKAGE, "mod_api"),
    (POINTS_PACKAGE, "mod_points"),
    (INTERNALS_PACKAGE, "internals"),
];

/// Where a mod's `Cargo.toml` names the SDK package `package`: the copy of its pinned build that
/// `cargo horizon-mod` fetches into the project (`.horizon/sdk/`).
pub(super) fn sdk_path(package: &str) -> String {
    format!(".horizon/sdk/api/crates/{package}")
}

/// Whether `value`, a mod's dependency `key` on the SDK package `package`, names the SDK the one
/// way the launcher takes: by the path to the mod tool's copy of the pinned SDK ([`sdk_path`]).
/// The launcher builds against its own copy of the same build. Every other form is refused under
/// one code, each with its own reason: the git form the path form replaced, a version alone (which
/// Cargo would resolve from crates.io), any other source selector, and a path elsewhere.
pub(super) fn sdk_source(key: &str, package: &str, value: &toml::Value) -> Result<(), String> {
    let path = sdk_path(package);
    let why = if value.get("git").is_some() {
        "the SDK is no longer named by git".to_owned()
    } else if let Some(selector) = [
        "registry",
        "registry-index",
        "branch",
        "tag",
        "rev",
        "workspace",
    ]
    .into_iter()
    .find(|selector| value.get(*selector).is_some())
    {
        format!("{selector} does not name the SDK")
    } else {
        match value.get("path") {
            None => format!(
                "a version names no source, so Cargo would take {package} from crates.io, where anyone can publish that name"
            ),
            Some(written) if written.as_str() != Some(path.as_str()) => {
                format!("the path must be \"{path}\", the pinned SDK the mod tool fetches")
            }
            Some(_) => return Ok(()),
        }
    };
    Err(format!(
        "E_SDK_SOURCE: {key}: {why}; write {key} = {{ path = \"{path}\" }}, then any cargo horizon-mod command fetches the pinned SDK there"
    ))
}

/// A dependency on one of [`PACKAGES`], under `[dependencies]` or a `[target.*]` table.
struct ApiDependency {
    target: Option<String>,
    key: String,
    package: &'static str,
    library: &'static str,
}

impl ApiDependency {
    fn table_mut<'a>(&self, cargo: &'a mut toml::Table) -> &'a mut toml::Table {
        match &self.target {
            Some(target) => cargo["target"][target.as_str()]["dependencies"]
                .as_table_mut()
                .expect("parsed dependencies"),
            None => cargo["dependencies"]
                .as_table_mut()
                .expect("parsed dependencies"),
        }
    }

    fn value_mut<'a>(&self, cargo: &'a mut toml::Table) -> &'a mut toml::Value {
        self.table_mut(cargo)
            .get_mut(&self.key)
            .expect("parsed dependency")
    }
}

#[derive(Debug)]
#[non_exhaustive]
pub(super) enum ProjectError {
    Cargo { mod_id: String, reason: String },
    PackageName { mod_id: String },
    ApiDependency { mod_id: String },
    GameDependency { mod_id: String, key: String },
}

impl std::fmt::Display for ProjectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use crate::i18n::trf;
        let message = match self {
            Self::Cargo { mod_id, reason } => {
                trf("{mod}: {error}", &[("mod", mod_id), ("error", reason)])
            }
            Self::PackageName { mod_id } => {
                trf("{mod}: Cargo.toml has no package name.", &[("mod", mod_id)])
            }
            Self::ApiDependency { mod_id } => trf(
                "{mod}: Cargo.toml has no horizon-mod-api dependency.",
                &[("mod", mod_id)],
            ),
            Self::GameDependency { mod_id, key } => trf(
                "E_GAME_DEPENDENCY: {mod}: Cargo.toml depends on the game ({key}). Declare game internals through horizon-internals instead.",
                &[("mod", mod_id), ("key", key)],
            ),
        };
        f.write_str(&message)
    }
}
impl std::error::Error for ProjectError {}

impl<'a> Project<'a> {
    /// The mod's crate name: its `[lib] name`, or its package name with `_` for `-`.
    pub(super) fn crate_name(&self) -> String {
        self.cargo
            .get("lib")
            .and_then(|l| l.get("name"))
            .and_then(toml::Value::as_str)
            .map_or_else(|| self.package.replace('-', "_"), str::to_owned)
    }

    pub(super) fn read(bundle: &'a Bundle) -> Result<Self, ProjectError> {
        let mod_id = || bundle.manifest.id.clone();
        let cargo = read(&bundle.dir.join("Cargo.toml")).map_err(|reason| ProjectError::Cargo {
            mod_id: mod_id(),
            reason,
        })?;
        for section in ["patch", "replace"] {
            if cargo.contains_key(section) {
                return Err(ProjectError::Cargo {
                    mod_id: mod_id(),
                    reason: format!("[{section}] is not allowed in a native mod"),
                });
            }
        }
        for config in [".cargo/config", ".cargo/config.toml"] {
            if bundle.dir.join(config).exists() {
                return Err(ProjectError::Cargo {
                    mod_id: mod_id(),
                    reason: format!(
                        "{config} is not allowed in a native mod; source replacement is refused"
                    ),
                });
            }
        }
        let package = cargo
            .get("package")
            .and_then(|p| p.get("name"))
            .and_then(toml::Value::as_str)
            .ok_or_else(|| ProjectError::PackageName { mod_id: mod_id() })?
            .to_owned();
        for table in std::iter::once(&cargo).chain(
            cargo
                .get("target")
                .and_then(toml::Value::as_table)
                .into_iter()
                .flat_map(|targets| targets.values())
                .filter_map(toml::Value::as_table),
        ) {
            for kind in [
                "dependencies",
                "dev-dependencies",
                "build-dependencies",
                "dev_dependencies",
                "build_dependencies",
            ] {
                for (key, value) in table
                    .get(kind)
                    .and_then(toml::Value::as_table)
                    .into_iter()
                    .flatten()
                {
                    let package = value
                        .get("package")
                        .and_then(toml::Value::as_str)
                        .unwrap_or(key);
                    if let Some(&(sdk, _)) = PACKAGES.iter().find(|(sdk, _)| *sdk == package) {
                        sdk_source(key, sdk, value).map_err(|reason| ProjectError::Cargo {
                            mod_id: mod_id(),
                            reason,
                        })?;
                    } else {
                        for selector in [
                            "git",
                            "registry",
                            "registry-index",
                            "branch",
                            "rev",
                            "tag",
                            "workspace",
                        ] {
                            if value.get(selector).is_some() {
                                return Err(ProjectError::Cargo { mod_id: mod_id(), reason: format!("{key}: {selector} source is refused; use crates.io with Cargo.lock") });
                            }
                        }
                    }
                    if value
                        .get("package")
                        .and_then(toml::Value::as_str)
                        .unwrap_or(key)
                        == "horizon-client"
                    {
                        return Err(ProjectError::GameDependency {
                            mod_id: mod_id(),
                            key: key.clone(),
                        });
                    }
                }
            }
        }
        let mut api_deps = Vec::new();
        for (target, dependencies) in dependency_tables(&cargo) {
            for (key, value) in dependencies {
                let package = value
                    .get("package")
                    .and_then(toml::Value::as_str)
                    .unwrap_or(key);
                if let Some(&(package, library)) =
                    PACKAGES.iter().find(|(name, _)| *name == package)
                {
                    api_deps.push(ApiDependency {
                        target: target.map(str::to_owned),
                        key: key.clone(),
                        package,
                        library,
                    });
                    continue;
                }
            }
        }
        if !api_deps
            .iter()
            .any(|api| api.target.is_none() && api.package == API_PACKAGE)
        {
            return Err(ProjectError::ApiDependency { mod_id: mod_id() });
        }
        Ok(Self {
            bundle,
            cargo,
            package,
            api_deps,
        })
    }

    /// The manifest's dependency tables: `[dependencies]` (no target), then each
    /// `[target.<cfg>.dependencies]`.
    pub(super) fn dependency_tables(
        &self,
    ) -> impl Iterator<Item = (Option<&str>, &toml::Table)> + '_ {
        dependency_tables(&self.cargo)
    }

    /// The names the mod's code calls the game's packages by (`mod_api` or a
    /// renamed dependency's key), each with its package's library name.
    pub(super) fn api_dependencies(&self) -> std::collections::BTreeSet<(String, &'static str)> {
        self.api_deps
            .iter()
            .map(|api| {
                let name = if api.key == api.package {
                    api.library.to_owned()
                } else {
                    api.key.replace('-', "_")
                };
                (name, api.library)
            })
            .collect()
    }
}

fn dependency_tables(cargo: &toml::Table) -> impl Iterator<Item = (Option<&str>, &toml::Table)> {
    let targets = cargo
        .get("target")
        .and_then(toml::Value::as_table)
        .into_iter()
        .flatten()
        .map(|(target, config)| (Some(target.as_str()), config.get("dependencies")));
    std::iter::once((None, cargo.get("dependencies")))
        .chain(targets)
        .filter_map(|(target, table)| Some((target, table?.as_table()?)))
}

// Add API packages and the generated executable to the fresh staged workspace.
pub(super) fn stage(
    stage: &Path,
    projects: &[Project<'_>],
    declared: &[Vec<super::dependencies::Dependency>],
    resolution: &super::dependencies::Resolution,
    catalog: &internals::Catalog,
) -> Result<bool, String> {
    let mods: Vec<_> = resolution
        .order
        .iter()
        .map(|&i| (i, &projects[i]))
        .collect();
    if mods.is_empty() {
        return Ok(false);
    }
    let facades = catalog
        .prepare(mods.iter().map(|(_, project)| &project.bundle.manifest))
        .map_err(|e| e.to_string())?;
    let template = template(&stage.join(MAIN_TEMPLATE))?;
    let packages: std::collections::HashMap<_, _> = mods
        .iter()
        .map(|(_, project)| (project.bundle.manifest.id.as_str(), project.package.clone()))
        .collect();
    let mut deps = toml::Table::new();
    // The generated game's features own sound/controllers, including the server's
    // `--no-default-features`; its client dependency never enables them independently.
    // `horizon-threads` stays for games pinned before the allocator moved into
    // `horizon-client`'s library: their template still names `horizon_threads::ALLOCATOR`.
    for name in [
        "horizon-client",
        "horizon-mod-api",
        "horizon-threads",
        "horizon-mod-package",
    ] {
        let mut dependency = path_dep(&format!("../{name}"));
        if name == "horizon-client" {
            dependency.insert("default-features".into(), false.into());
        }
        deps.insert(name.into(), toml::Value::Table(dependency));
    }
    let mut members = Vec::new();
    for (i, (index, project)) in mods.iter().enumerate() {
        let bundle = project.bundle;
        let relative = format!("native-api/{}", bundle.manifest.id);
        let dest = stage.join(&relative);
        super::copy_bundle(bundle, &dest)?;

        let path = dest.join("Cargo.toml");
        let mut manifest = project.cargo.clone();
        let package = project.package.clone();
        for api in &project.api_deps {
            let dependency = api.value_mut(&mut manifest);
            let mut rewritten = path_dep(&if api.package == INTERNALS_PACKAGE {
                format!("../../{}", internals::relative(&bundle.manifest.id))
            } else {
                format!("../../crates/{}", api.package)
            });
            if let Some(old) = dependency.as_table() {
                // A renamed dependency also needs its package name.
                for key in ["package", "features", "default-features"] {
                    if let Some(value) = old.get(key) {
                        rewritten.insert(key.into(), value.clone());
                    }
                }
            }
            if api.package == INTERNALS_PACKAGE {
                rewritten.insert(
                    "package".into(),
                    internals::package(&bundle.manifest.id).into(),
                );
            }
            *dependency = toml::Value::Table(rewritten);
            // Cargo treats a key differing from `package` as the crate's alias. Preserve the
            // facade's library name when giving its per-mod package a unique name.
            if api.package == INTERNALS_PACKAGE && api.key == INTERNALS_PACKAGE {
                let table = api.table_mut(&mut manifest);
                let value = table.remove(&api.key).expect("rewritten dependency");
                table.insert("internals".into(), value);
            }
        }
        let targets: Vec<_> = declared[*index]
            .iter()
            .map(|dependency| super::dependencies::Package {
                dependency,
                package: packages.get(dependency.id.as_str()).map(String::as_str),
            })
            .collect();
        let enabled = super::dependencies::rewrite(&bundle.manifest.id, &mut manifest, &targets)
            .map_err(|e| e.to_string())?;
        write(&path, &manifest)?;
        let mut dep = toml::Table::new();
        dep.insert("path".into(), format!("../../{relative}").into());
        dep.insert("package".into(), package.into());
        if !enabled.is_empty() {
            dep.insert(
                "features".into(),
                toml::Value::Array(enabled.into_iter().map(toml::Value::String).collect()),
            );
        }
        deps.insert(format!("mod_{i}"), toml::Value::Table(dep));
        if internals::needed(&bundle.manifest)
            || project
                .api_deps
                .iter()
                .any(|dep| dep.package == INTERNALS_PACKAGE)
        {
            internals::stage(stage, project, &facades[bundle.manifest.id.as_str()])?;
            let facade = internals::relative(&bundle.manifest.id);
            let mut dependency = path_dep(&format!("../../{facade}"));
            dependency.insert(
                "package".into(),
                internals::package(&bundle.manifest.id).into(),
            );
            deps.insert(format!("mod_{i}_internals"), toml::Value::Table(dependency));
            members.push(facade);
        }
        members.push(relative);
    }
    // Beside horizon-client, so its build script (the Windows stack size and icon, for the
    // `horizon-client` binary this package owns now) finds `packaging/` where it looks.
    let game = stage.join(GAME);
    fs::create_dir_all(game.join("src")).map_err(|e| e.to_string())?;
    let mut manifest = toml::Table::new();
    manifest.insert(
        "package".into(),
        toml::Value::Table(toml::Table::from_iter([
            ("name".into(), PACKAGE.into()),
            ("version".into(), "0.1.0".into()),
            ("edition".into(), "2024".into()),
            ("publish".into(), false.into()),
            ("build".into(), "../horizon-client/build.rs".into()),
        ])),
    );
    // This sibling package uses the client's build.rs, so its build dependencies apply too;
    // relative paths still point to the same crates from crates/native-game.
    let client = read(&stage.join("crates/horizon-client/Cargo.toml"))?;
    if let Some(dependencies) = client.get("build-dependencies") {
        manifest.insert("build-dependencies".into(), dependencies.clone());
    }
    let windows = toml::Table::from_iter([(
        "build-dependencies".into(),
        toml::Value::Table(toml::Table::from_iter([(
            "winresource".into(),
            "0.1".into(),
        )])),
    )]);
    manifest.insert(
        "target".into(),
        toml::Value::Table(toml::Table::from_iter([(
            "cfg(windows)".into(),
            toml::Value::Table(windows),
        )])),
    );
    manifest.insert(
        "bin".into(),
        toml::Value::Array(vec![toml::Value::Table(toml::Table::from_iter([
            ("name".into(), "horizon-client".into()),
            ("path".into(), "src/main.rs".into()),
        ]))]),
    );
    let official = has_official(stage);
    if official {
        let mut dep = toml::Table::new();
        dep.insert("path".into(), "../horizon-official".into());
        dep.insert("optional".into(), true.into());
        deps.insert(OFFICIAL.into(), toml::Value::Table(dep));
    }
    if stage.join("crates/horizon-mod-host/Cargo.toml").is_file() {
        deps.insert(
            "horizon-mod-host".into(),
            toml::Value::Table(toml::Table::from_iter([
                ("path".into(), "../horizon-mod-host".into()),
                ("default-features".into(), false.into()),
            ])),
        );
    }
    manifest.insert("dependencies".into(), toml::Value::Table(deps));
    manifest.insert(
        "features".into(),
        toml::Value::Table(toml::Table::from_iter([(
            "default".into(),
            toml::Value::Array(vec!["horizon-client/default".into()]),
        )])),
    );
    if official {
        let features = manifest
            .get_mut("features")
            .unwrap()
            .as_table_mut()
            .unwrap();
        features
            .get_mut("default")
            .unwrap()
            .as_array_mut()
            .unwrap()
            .push("official".into());
        features.insert(
            "official".into(),
            toml::Value::Array(vec!["dep:horizon-official".into()]),
        );
    }
    write(&game.join("Cargo.toml"), &manifest)?;
    let manifests: Vec<_> = mods
        .iter()
        .map(|(_, project)| &project.bundle.manifest)
        .collect();
    let mut main = main_rs(&template, &manifests, catalog);
    if official {
        main = main.replace(
            "fn main() {",
            "fn main() {\n    #[cfg(feature = \"official\")]\n    horizon_official::register();",
        );
    }
    fs::write(game.join("src/main.rs"), main).map_err(|e| e.to_string())?;
    let root = stage.join("Cargo.toml");
    let mut workspace = read(&root)?;
    let list = workspace
        .get_mut("workspace")
        .and_then(|w| w.get_mut("members"))
        .and_then(toml::Value::as_array_mut)
        .ok_or("workspace has no members")?;
    members.push(GAME.into());
    list.extend(members.into_iter().map(toml::Value::String));
    write(&root, &workspace)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> std::path::PathBuf {
        let name = std::thread::current().name().unwrap().replace(':', "-");
        let dir = super::super::tests::dir(&name);
        super::super::tests::write_bundle(&dir, "example", "");
        fs::write(dir.join("Cargo.toml"), "[package]\nname='example-mod'\nversion='9.0.0'\nedition='2024'\n[dependencies]\nhorizon-mod-api={path='.horizon/sdk/api/crates/horizon-mod-api'}\n").unwrap();
        dir
    }

    #[test]
    #[ignore = "daily: starts rustc"]
    fn wrapper_owns_holder_marker() {
        let dir = fixture();
        let source = dir.join("main.rs");
        let output = dir.join(std::env::consts::EXE_SUFFIX.to_owned() + "wrapper");
        let library_source = dir.join("library.rs");
        fs::write(&library_source, "pub mod holder { pub static CAPABILITY: [u8; 26] = *b\"horizon-private-holder-v2;\"; }").unwrap();
        let library = dir.join(format!(
            "{}horizon_mod_package{}",
            std::env::consts::DLL_PREFIX,
            std::env::consts::DLL_SUFFIX
        ));
        assert!(std::process::Command::new("rustc")
            .args([
                "--crate-type=dylib",
                "--crate-name=horizon_mod_package",
                "-Cprefer-dynamic"
            ])
            .arg(&library_source)
            .arg("-o")
            .arg(&library)
            .status()
            .unwrap()
            .success());
        let template =
            "fn main() { std::hint::black_box(&horizon_mod_package::holder::CAPABILITY); }";
        fs::write(
            &source,
            main_rs(template, &[], &internals::Catalog::default()),
        )
        .unwrap();
        assert!(std::process::Command::new("rustc")
            .args(["--edition=2021", "-O", "-Cprefer-dynamic", "--extern"])
            .arg(format!("horizon_mod_package={}", library.display()))
            .arg(&source)
            .arg("-o")
            .arg(&output)
            .status()
            .unwrap()
            .success());
        assert!(fs::read(&output)
            .unwrap()
            .windows(26)
            .any(|w| w == b"horizon-private-holder-v2;"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn projects_refuse_a_game_dependency_and_need_the_mod_api() {
        let dir = fixture();
        for (dependencies, game) in [
            (
                "[dependencies]\nhorizon-mod-api={path='.horizon/sdk/api/crates/horizon-mod-api'}\nhorizon-client='0.1'",
                Some("horizon-client"),
            ),
            (
                "[dependencies.api]\npackage='horizon-mod-api'\npath='.horizon/sdk/api/crates/horizon-mod-api'\n[target.'cfg(unix)'.dependencies.game]\npackage='horizon-client'\nversion='0.1'",
                Some("game"),
            ),
            ("[dependencies]\nserde='1'", None),
            (
                "[target.'cfg(unix)'.dependencies]\nhorizon-mod-api={path='.horizon/sdk/api/crates/horizon-mod-api'}",
                None,
            ),
        ] {
            let text = format!("[package]\nname='example-mod'\nversion='1.0.0'\n{dependencies}\n");
            fs::write(dir.join("Cargo.toml"), text).unwrap();
            let bundle = super::super::bundle_in(&dir).unwrap().unwrap();
            let error = Project::read(&bundle).err().unwrap();
            match game {
                Some(game) => assert!(
                    matches!(&error, ProjectError::GameDependency { mod_id, key } if mod_id == "example" && key == game),
                    "{error}"
                ),
                None => assert!(
                    matches!(&error, ProjectError::ApiDependency { .. }),
                    "{error}"
                ),
            }
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_sdk_is_taken_only_by_its_path_form_and_every_other_form_is_refused_by_name() {
        let dir = fixture();
        for (form, refusal) in [
            ("{ path = '.horizon/sdk/api/crates/horizon-mod-api' }", None),
            (
                "{ path = '.horizon/sdk/api/crates/horizon-mod-api', features = ['testing'] }",
                None,
            ),
            (
                "{ git = 'https://github.com/arismoko/horizon', rev = 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa' }",
                Some("example: E_SDK_SOURCE: horizon-mod-api: the SDK is no longer named by git; write horizon-mod-api = { path = \".horizon/sdk/api/crates/horizon-mod-api\" }"),
            ),
            (
                "'0.1'",
                Some("example: E_SDK_SOURCE: horizon-mod-api: a version names no source, so Cargo would take horizon-mod-api from crates.io"),
            ),
            (
                "{ version = '0.1', features = ['testing'] }",
                Some("example: E_SDK_SOURCE: horizon-mod-api: a version names no source"),
            ),
            (
                "{ workspace = true }",
                Some("example: E_SDK_SOURCE: horizon-mod-api: workspace does not name the SDK"),
            ),
            (
                "{ path = '../horizon/crates/horizon-mod-api' }",
                Some("example: E_SDK_SOURCE: horizon-mod-api: the path must be \".horizon/sdk/api/crates/horizon-mod-api\""),
            ),
        ] {
            let text = format!(
                "[package]\nname='example-mod'\nversion='1.0.0'\n[dependencies]\nhorizon-mod-api = {form}\n"
            );
            fs::write(dir.join("Cargo.toml"), text).unwrap();
            let bundle = super::super::bundle_in(&dir).unwrap().unwrap();
            match (Project::read(&bundle), refusal) {
                (Ok(_), None) => {}
                (Err(error), Some(refusal)) => {
                    assert!(error.to_string().starts_with(refusal), "{error}")
                }
                (result, _) => panic!("{form}: {:?}", result.err()),
            }
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn generates_only_tool_contract_tables_without_mutating_package() {
        let dir = fixture();
        let path = dir.join(horizon_mod_package::manifest::MANIFEST);
        let text = fs::read_to_string(&path).unwrap()
            + "[[native.module]]\nid='test'\nfile='crates/horizon-client/src/game.rs'\nname='support'\nsource='src/lib.rs'\n";
        fs::write(&path, &text).unwrap();
        let bundle = super::super::bundle_in(&dir).unwrap().unwrap();
        assert!(bundle.needs_mixins());
        super::super::package::tool_manifest(&bundle).unwrap();
        let value = read(&dir.join(super::super::MANIFEST)).unwrap();
        assert!(!value.contains_key("api"));
        assert!(!value.contains_key("native"));
        assert!(value.contains_key("module"));
        assert!(!value.contains_key("injection"));
        assert_eq!(value["mod"]["id"].as_str(), Some("example"));
        assert_eq!(fs::read_to_string(path).unwrap(), text);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn generated_main_keeps_mod_order() {
        let template =
            include_str!("../../../crates/horizon-client/src/native_mod_entry.rs.template");
        let dir = fixture();
        let mut base = horizon_mod_package::Package::read(&dir).unwrap().manifest;
        base.id = "base".into();
        base.version = "1.2.3".into();
        base.side = mod_vocab::Side::Both;
        let mut extra = base.clone();
        extra.id = "extra".into();
        extra.version = "2.0.0".into();
        extra.side = mod_vocab::Side::Client;
        let mods = [&base, &extra];
        for count in 0..=2 {
            let text = main_rs(template, &mods[..count], &internals::Catalog::default());
            let padded = ["/* MOD_ENTRIES */", "/* MOD_SLOTS */", "/* BUILT_MODS */"]
                .into_iter()
                .fold(template.to_owned(), |text, marker| {
                    text.replace(marker, &format!("    {marker}"))
                });
            assert_eq!(
                main_rs(&padded, &mods[..count], &internals::Catalog::default()),
                text
            );
            let facts: Vec<_> = text
                .lines()
                .filter(|line| line.contains(" => mod_"))
                .collect();
            assert_eq!(
                facts,
                &[
                    "        \"base\" => mod_0::hooks,",
                    "        \"extra\" => mod_1::hooks,",
                ][..count]
            );
            let inventory: Vec<_> = text
                .lines()
                .filter(|line| line.contains("BuiltMod {"))
                .collect();
            assert_eq!(
                inventory,
                &[
                    "    mod_api::__private::native::BuiltMod { id: \"base\", version: \"1.2.3\", side: mod_api::Side::Both },",
                    "    mod_api::__private::native::BuiltMod { id: \"extra\", version: \"2.0.0\", side: mod_api::Side::Client },",
                ][..count]
            );
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn named_source_without_an_entry_template_is_refused_before_staging() {
        let dir = fixture();
        let stage_dir = dir.with_extension("workspace");
        let _ = fs::remove_dir_all(&stage_dir);
        let source = stage_dir.join("crates/horizon-client/src");
        fs::create_dir_all(&source).unwrap();
        let resolution = super::super::dependencies::Resolution {
            order: vec![0],
            warnings: Vec::new(),
        };
        for text in ["", "pub struct ModEntry { id: &'static str }"] {
            fs::write(source.join("mod_api.rs"), text).unwrap();
            let bundle = super::super::bundle_in(&dir).unwrap().unwrap();
            let project = Project::read(&bundle).unwrap();
            let error = stage(
                &stage_dir,
                &[project],
                &[vec![]],
                &resolution,
                &internals::Catalog::default(),
            )
            .unwrap_err();
            assert!(error.contains("Update Horizon and rebuild"), "{error}");
            assert!(!stage_dir.join(GAME).exists());
            assert!(!stage_dir.join("native-api").exists());
        }
        assert!(!stage(
            &stage_dir,
            &[],
            &[],
            &super::super::dependencies::Resolution {
                order: vec![],
                warnings: Vec::new(),
            },
            &internals::Catalog::default(),
        )
        .unwrap());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn api_only_skips_mixins() {
        let dir = fixture();
        let bundle = super::super::bundle_in(&dir).unwrap().unwrap();
        assert!(!bundle.needs_mixins());
        fs::remove_dir_all(dir).unwrap();
    }
}
