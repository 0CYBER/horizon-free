//! Function replacement (fn-replace): `[[native.mixin]]` rows built against the prebuilt game, with no
//! game rebuild. Each function the rows target is copied out of the pinned source into one patch
//! crate (`horizon_mod_api_gen::patch`), horizon-mixins applies every mod's rows to those copies
//! in one pass (one combined copy per function, in the tool's order), rustc compiles the crate
//! against the prebuilt game and the mods, and the generated `main` installs each copy through
//! its point's replacement slot. What can't take this route is refused by name; nothing here
//! falls back to compiling the game.
use super::{api, prebuilt::Identity, Bundle};
use crate::i18n::trf;
use crate::rust_toolchain::Toolchain;
use horizon_platform::mod_build::api_gen;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// The patch crates' name (a lower game crate's is `<name>_<crate>`): the copies' ids and the library entry's
/// `--extern` use it. It is not `horizon_patch`, the name of the library built from them: a library and a crate
/// it links with one crate name are indistinguishable to rustc (E0519), and a registry workspace refuses two
/// packages of one name.
pub(super) const CRATE: &str = "horizon_patched";

/// The compiled patch crates, `(crate name, rlib)`, and the installation result expressions the
/// generated `main` installs.
pub(super) struct Patch {
    pub rlibs: Vec<(String, PathBuf)>,
    pub installs: Vec<String>,
}

fn install_expression(target: &str, mods: &[&str], install: &str) -> String {
    format!("({target:?}, &{mods:?}, {install})")
}

/// Preserve every requester at the launcher boundary; the generator only knows targets.
fn target_refusal(
    refused: &api_gen::patch::Refusal,
    requests: &[(String, String, String)],
) -> String {
    let owners = requests
        .iter()
        .filter(|(_, _, target)| target == &refused.target)
        .map(|(id, row, _)| format!("{id}:{row}"))
        .collect::<Vec<_>>()
        .join(", ");
    if owners.is_empty() {
        // Module and bridge ids already include their owning mod.
        return refused.to_string();
    }
    trf(
        "{code}: {owners}: {target}: {why}",
        &[
            ("code", refused.code),
            ("owners", &owners),
            ("target", &refused.target),
            ("why", &refused.why),
        ],
    )
}

/// The support modules and bridges `bundles` add to the patch crate.
fn extras(bundles: &[&Bundle]) -> Result<api_gen::patch::Extras, String> {
    let mut extras = api_gen::patch::Extras::default();
    // The names the helpers and bindings use, which a lower crate's mirror keeps imports for.
    for bundle in bundles {
        for row in &bundle.native().mixin {
            let mut texts =
                vec![fs::read_to_string(bundle.dir.join(&row.helper)).unwrap_or_default()];
            // `bindings` and every other forwarded key: their strings name types and values.
            fn strings(value: &toml::Value, out: &mut Vec<String>) {
                match value {
                    toml::Value::String(text) => out.push(text.replace('$', " ")),
                    toml::Value::Array(items) => items.iter().for_each(|v| strings(v, out)),
                    toml::Value::Table(table) => table.values().for_each(|v| strings(v, out)),
                    _ => {}
                }
            }
            for value in row.options.values() {
                strings(value, &mut texts);
            }
            for text in texts {
                if let Ok(tokens) = text.parse::<proc_macro2::TokenStream>() {
                    api_gen::patch::idents_in(&tokens, &mut extras.idents);
                }
            }
        }
    }
    let normalize = |s: &str| s.replace('-', "_").to_ascii_lowercase();
    for bundle in bundles {
        let id = &bundle.manifest.id;
        let Some(native) = bundle.manifest.native.as_ref() else {
            continue;
        };
        for module in &native.module {
            let path = bundle.dir.join(&module.source);
            extras.modules.push(api_gen::patch::SupportModule {
                id: format!("{id}:{}", module.id),
                file: module.file.clone(),
                name: module.name.clone(),
                text: fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?,
                public: module.visibility.as_deref() == Some("pub"),
            });
        }
        for bridge in &native.bridge {
            let field = |key: &str| bridge.fields.get(key).and_then(toml::Value::as_str);
            extras.bridges.push(api_gen::patch::Bridge {
                id: format!("{id}:{}", bridge.id),
                method: format!(
                    "__horizon_mixins_{}_{}",
                    normalize(id),
                    normalize(&bridge.id)
                ),
                kind: field("kind").unwrap_or_default().to_owned(),
                target: field("target").unwrap_or_default().to_owned(),
                member: field("member").map(str::to_owned),
            });
        }
    }
    Ok(extras)
}

/// The patched sources of `projects`' `[[native.mixin]]` rows, supports modules and bridges,
/// ready to compile: one crate folder per name under `dir`.
pub(super) struct Prepared<'a> {
    pub dir: PathBuf,
    pub names: Vec<String>,
    pub installs: Vec<String>,
    /// The mods whose helpers the crates may call.
    pub bundles: Vec<&'a Bundle>,
    count: usize,
    rows: usize,
    started: std::time::Instant,
    copied: std::time::Duration,
    applied: std::time::Duration,
}

/// Builds the patch crates for `projects`' `[[native.mixin]]` rows against `game`, into `out`;
/// `None` when no mod has one. `rlibs` and `crates` are the compiled mods and their crate names,
/// which helpers may call.
#[allow(clippy::too_many_arguments)]
pub(super) fn build(
    rust: &Toolchain,
    game: &Path,
    out: &Path,
    projects: &[api::Project<'_>],
    rlibs: &HashMap<&str, PathBuf>,
    crates: &HashMap<&str, String>,
    sdk: &Identity,
    rustc: impl Fn(&str) -> std::process::Command,
) -> Result<Option<Patch>, String> {
    let Some(prepared) = prepare(rust, game, out, projects, sdk)? else {
        return Ok(None);
    };
    // The patched crates, against the prebuilt game and the mods.
    let mut compiled = Vec::new();
    for name in &prepared.names {
        let rlib = out.join(format!("lib{name}.rlib"));
        let mut hash = sha2::Sha256::new();
        use sha2::Digest;
        hash.update(sdk.commit());
        for bundle in &prepared.bundles {
            hash.update(
                super::source_digest(bundle)
                    .ok_or(crate::i18n::tr("Could not hash patch inputs"))?,
            );
            hash.update(fs::read(&rlibs[bundle.manifest.id.as_str()]).map_err(|e| e.to_string())?);
        }
        let fingerprint = format!("{:x}", hash.finalize());
        let saved = rlib.with_extension("fingerprint");
        if rlib.is_file() && fs::read_to_string(&saved).ok().as_deref() == Some(&fingerprint) {
            compiled.push((name.clone(), rlib));
            continue;
        }
        let mut cmd = rustc("2024");
        cmd.args([
            "--crate-type",
            "rlib",
            "--crate-name",
            name,
            "--cap-lints",
            "allow",
        ])
        .arg("-o")
        .arg(&rlib)
        .arg(prepared.dir.join(name).join("src/lib.rs"));
        for feature in sdk.game_features() {
            cmd.arg("--cfg").arg(format!("feature=\"{feature}\""));
        }
        // An `@` file, one argument per line: an extern per SDK crate passes Windows'
        // 32,767-character command-line limit.
        let mut externs = Vec::new();
        for (krate, library) in sdk_externs(game, sdk)? {
            externs.push(format!("--extern\n{krate}={}", library.display()));
        }
        for bundle in &prepared.bundles {
            let id = bundle.manifest.id.as_str();
            externs.push(format!("--extern\n{}={}", crates[id], rlibs[id].display()));
        }
        let argfile = out.join(format!("{name}.externs"));
        fs::write(&argfile, externs.join("\n"))
            .map_err(|e| format!("Could not write {}: {e}", argfile.display()))?;
        let mut arg = std::ffi::OsString::from("@");
        arg.push(&argfile);
        cmd.arg(arg);
        super::run(cmd, "Compiling the patched functions", |_| {})?;
        fs::write(saved, fingerprint).map_err(|e| e.to_string())?;
        compiled.push((name.clone(), rlib));
    }
    prepared.log()?;
    Ok(Some(Patch {
        rlibs: compiled,
        installs: prepared.installs,
    }))
}

/// The SDK libraries a patch crate compiles against: the game crate and the facade crates.
pub(super) fn sdk_externs(game: &Path, sdk: &Identity) -> Result<Vec<(String, PathBuf)>, String> {
    let deps = game.join("deps");
    let mut externs = vec![(
        "horizon_client".to_owned(),
        super::prebuilt::library(&deps, "horizon_client")?,
    )];
    for krate in sdk.facade_crates() {
        if krate != "horizon_client" {
            externs.push((
                krate.to_owned(),
                super::prebuilt::facade_library(&deps, krate, sdk)?,
            ));
        }
    }
    Ok(externs)
}

impl Prepared<'_> {
    /// The build log's line for this mixin build.
    pub(super) fn log(&self) -> Result<(), String> {
        super::append_log(&trf(
            "Function replacement: {count} functions patched by {rows} mixins, without rebuilding the game (copy {copy} ms, horizon-mixins {tool} ms, compile {compile} ms, total {total} ms)",
            &[
                ("count", &self.count.to_string()),
                ("rows", &self.rows.to_string()),
                ("copy", &self.copied.as_millis().to_string()),
                ("tool", &(self.applied - self.copied).as_millis().to_string()),
                ("compile", &(self.started.elapsed() - self.applied).as_millis().to_string()),
                ("total", &self.started.elapsed().as_millis().to_string()),
            ],
        ))
    }
}

/// The published loops `bundles`' `[[native.loop]]` rows fill, with their slots in the order they
/// are asked, from the game's `loops.json`. A row naming no published loop, and a mixin that
/// replaces or redirects a filled loop's entry (anything but `HEAD` and `RETURN`, which compose
/// around the loop), are refused by name.
fn loop_slots(game: &Path, bundles: &[&Bundle]) -> Result<Vec<api_gen::patch::LoopSlots>, String> {
    use horizon_platform::mod_package::manifest::mixin::Kind;
    let rows: Vec<(
        &Bundle,
        &horizon_platform::mod_package::manifest::loops::Loop,
    )> = bundles
        .iter()
        .flat_map(|bundle| bundle.native().r#loop.iter().map(move |row| (*bundle, row)))
        .collect();
    let path = game.join(api_gen::loops::FILE);
    // A game built before published loops has no list, and so no loop a row can name.
    #[derive(serde::Deserialize)]
    struct Loops {
        loops: Vec<api_gen::loops::PublishedLoop>,
    }
    let published = match fs::read_to_string(&path) {
        Ok(text) => {
            horizon_platform::mod_package::build_data::read::<Loops>(api_gen::loops::FILE, &text)
                .map_err(|e| format!("{}: {e}", path.display()))?
                .1
                .loops
        }
        Err(_) => Vec::new(),
    };
    let mut refusals = Vec::new();
    // A loop with a mixin on its function is copied with it, slots or none.
    let mut filled: Vec<api_gen::patch::LoopSlots> = published
        .iter()
        .filter(|l| {
            let generic = l.generic_target();
            bundles
                .iter()
                .any(|b| b.native().mixin.iter().any(|m| m.target == generic))
        })
        .map(|l| api_gen::patch::LoopSlots {
            published: l.clone(),
            slots: Vec::new(),
        })
        .collect();
    let mut ordered = rows;
    ordered.sort_by(|(a, ra), (b, rb)| {
        (ra.priority, &a.manifest.id, &ra.id).cmp(&(rb.priority, &b.manifest.id, &rb.id))
    });
    for (bundle, row) in ordered {
        let id = &bundle.manifest.id;
        let Some(found) = published.iter().find(|l| l.name == row.name) else {
            let known: Vec<&str> = published.iter().map(|l| l.name.as_str()).collect();
            refusals.push(format!(
                "E_LOOP_UNKNOWN: {id}:{}: the game publishes no loop `{}`; its loops: {}",
                row.id,
                row.name,
                if known.is_empty() {
                    "none".to_owned()
                } else {
                    known.join(", ")
                }
            ));
            continue;
        };
        let path = bundle.dir.join(&row.helper);
        let helper = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let slot = api_gen::patch::LoopSlot {
            mod_id: id.clone(),
            id: row.id.clone(),
            helper,
            with: row.with.clone(),
        };
        match filled.iter_mut().find(|l| l.published.name == found.name) {
            Some(slots) => slots.slots.push(slot),
            None => filled.push(api_gen::patch::LoopSlots {
                published: found.clone(),
                slots: vec![slot],
            }),
        }
    }
    for bundle in bundles {
        for mixin in &bundle.native().mixin {
            let Some(slots) = filled.iter().find(|l| {
                !l.slots.is_empty()
                    && (l.published.entry == mixin.target
                        || (l.published.generic_target() == mixin.target
                            && mixin.kind == Kind::Overwrite))
            }) else {
                continue;
            };
            if mixin.kind == Kind::Overwrite
                || !matches!(mixin.kind, Kind::Head | Kind::Return)
                    && slots.published.entry == mixin.target
            {
                let fillers: Vec<&str> = slots.slots.iter().map(|s| s.mod_id.as_str()).collect();
                refusals.push(format!(
                    "E_LOOP_ENTRY_REPLACED: {}:{}: the {} mixin on {} would replace or bypass the published loop `{}`, filled by {}; only HEAD and RETURN mixins compose with it",
                    bundle.manifest.id,
                    mixin.id,
                    mixin.kind.name(),
                    mixin.target,
                    slots.published.name,
                    fillers.join(", ")
                ));
            }
        }
    }
    if refusals.is_empty() {
        Ok(filled)
    } else {
        Err(refusals.join("\n"))
    }
}

/// The game build's function index (`functions.json`), from its SDK at `game`: where each function
/// a row may target is in the source, and whether it can be copied. With no index for this build
/// nothing is copied: the source is never walked instead.
fn function_index(game: &Path, commit: &str) -> Result<api_gen::functions::FunctionIndex, String> {
    use api_gen::functions::FILE;
    let path = game.join(FILE);
    let text = fs::read_to_string(&path).map_err(|e| {
        format!(
            "E_MIXIN_INDEX: {}: {e}: the game build's SDK has no function index, so no function can be copied; fetch the SDK again",
            path.display()
        )
    })?;
    let (build, index): (_, api_gen::functions::FunctionIndex) =
        horizon_platform::mod_package::build_data::read(FILE, &text)
            .map_err(|e| format!("E_MIXIN_INDEX: {e}"))?;
    if build.commit != commit {
        return Err(format!(
            "E_MIXIN_INDEX: {FILE} describes game build {}, not {commit}; fetch the SDK again",
            build.commit
        ));
    }
    index.checked().map_err(|e| format!("E_MIXIN_INDEX: {e}"))
}

/// Copies, generates and transforms the patch crates for `projects` into `out`, without
/// compiling them; `None` when no mod has a mixin, module, bridge or loop.
pub(super) fn prepare<'a>(
    rust: &Toolchain,
    game: &Path,
    out: &Path,
    projects: &[api::Project<'a>],
    sdk: &Identity,
) -> Result<Option<Prepared<'a>>, String> {
    let bundles: Vec<&Bundle> = projects
        .iter()
        .map(|project| project.bundle)
        .filter(|bundle| {
            bundle.needs_mixins()
                || bundle.manifest.native.as_ref().is_some_and(|n| {
                    !n.state.is_empty()
                        || !n.bridge.is_empty()
                        || !n.module.is_empty()
                        || !n.r#loop.is_empty()
                })
        })
        .collect();
    if bundles.is_empty() {
        return Ok(None);
    }
    let loops = loop_slots(game, &bundles)?;
    let requests: Vec<(String, String, String)> = bundles
        .iter()
        .flat_map(|bundle| {
            bundle.native().mixin.iter().map(|row| {
                (
                    bundle.manifest.id.clone(),
                    row.id.clone(),
                    row.target.clone(),
                )
            })
        })
        .collect();
    let rows: Vec<String> = bundles
        .iter()
        .flat_map(|bundle| bundle.native().mixin.iter().map(|row| row.target.clone()))
        .collect();
    // Built once per pin and patch, before the clock: the timing is the mixin build's own.
    let tool = if rows.is_empty() {
        None
    } else {
        Some(super::mixins_patch::tool(rust, &|_| {})?)
    };
    let source = super::source(sdk.commit())?;
    let started = std::time::Instant::now();
    let index = function_index(game, sdk.commit())?;
    // A filled loop's entry, or one whose loop function has rows, is copied like a mixin's target,
    // with the loop's body.
    let targets: Vec<String> = rows
        .iter()
        .cloned()
        .chain(loops.iter().map(|l| l.published.entry.clone()))
        .collect();
    let loop_count = loops.iter().filter(|l| !l.slots.is_empty()).count();
    let slot_count = loops.iter().map(|l| l.slots.len()).sum::<usize>();
    // The generator refuses targets the staged game cannot replace.
    let mut extras = extras(&bundles)?;
    extras.loops = loops;
    let generated = api_gen::patch::generate(&source, CRATE, &targets, &extras, &index);
    let refusals: Vec<String> = match &generated {
        Ok(_) => Vec::new(),
        Err(refused) => refused
            .iter()
            .map(|r| target_refusal(r, &requests))
            .collect(),
    };
    if !refusals.is_empty() {
        return Err(refusals.join("\n"));
    }
    let generated = generated.expect("no refusals");
    let ids: HashMap<&str, &str> = generated
        .patched
        .iter()
        .map(|p| (p.target.as_str(), p.id.as_str()))
        .collect();
    // The workspace horizon-mixins plans against: one patch crate per game crate with copies.
    let workspace = out.join("patch-workspace");
    let _ = fs::remove_dir_all(&workspace);
    let write = |path: PathBuf, text: &str| {
        fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
    };
    let names: Vec<&str> = generated
        .libs
        .iter()
        .map(|(name, _)| name.as_str())
        .collect();
    fs::create_dir_all(&workspace).map_err(|e| e.to_string())?;
    write(
        workspace.join("Cargo.toml"),
        &format!(
            "[workspace]\nmembers = [{}]\nresolver = \"2\"\n",
            names
                .iter()
                .map(|n| format!("\"{n}\""))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    )?;
    write(
        workspace.join("Cargo.lock"),
        &format!(
            "# Generated by the launcher.\nversion = 4\n{}",
            names
                .iter()
                .map(|n| format!("\n[[package]]\nname = \"{n}\"\nversion = \"0.0.0\"\n"))
                .collect::<String>()
        ),
    )?;
    write(workspace.join(".horizon-revision"), sdk.commit())?;
    for (name, lib) in &generated.libs {
        let package = workspace.join(name);
        fs::create_dir_all(package.join("src")).map_err(|e| e.to_string())?;
        write(
            package.join("Cargo.toml"),
            &format!("[package]\nname = \"{name}\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n[lib]\npath = \"src/lib.rs\"\n"),
        )?;
        write(package.join("src/lib.rs"), lib)?;
    }
    // Each mod's rows, retargeted at the copies, as the tool's bundles.
    let mods = out.join("patch-mods");
    let _ = fs::remove_dir_all(&mods);
    for bundle in bundles
        .iter()
        .filter(|bundle| !bundle.native().mixin.is_empty())
    {
        let dir = mods.join(&bundle.manifest.id);
        super::package::copy_tool_input(bundle, &dir)?;
        let mut rows = bundle.native().mixin.clone();
        for row in &mut rows {
            row.target = ids[row.target.as_str()].to_owned();
        }
        let mut manifest = toml::Table::new();
        manifest.insert(
            "mod".into(),
            toml::Value::Table(toml::Table::from_iter([
                ("id".into(), bundle.manifest.id.clone().into()),
                ("version".into(), bundle.manifest.version.clone().into()),
            ])),
        );
        manifest.insert(
            "compatibility".into(),
            toml::Value::Table(toml::Table::from_iter([(
                "horizon_revision".into(),
                sdk.commit().to_owned().into(),
            )])),
        );
        manifest.insert(
            "injection".into(),
            toml::Value::try_from(&rows).map_err(|e| e.to_string())?,
        );
        write(
            dir.join("horizon-mixin.toml"),
            &toml::to_string(&manifest).map_err(|e| e.to_string())?,
        )?;
    }
    let copied = started.elapsed();
    // With no rows (only modules or bridges), the crate compiles as generated.
    let patched = if let Some(tool) = &tool {
        let patched = out.join("patch-applied");
        super::run(
            super::apply(rust, tool, &workspace, &mods, &patched),
            "Applying the mixins to their functions",
            |_| {},
        )?;
        patched
    } else {
        workspace.clone()
    };
    let applied = started.elapsed();
    if loop_count > 0 {
        super::append_log(&trf(
            "Published loops: {loops} filled by {slots} mod slots",
            &[
                ("loops", &loop_count.to_string()),
                ("slots", &slot_count.to_string()),
            ],
        ))?;
    }
    Ok(Some(Prepared {
        dir: patched,
        names: names.iter().map(|name| name.to_string()).collect(),
        installs: generated
            .patched
            .iter()
            .filter_map(|p| {
                let mut mods = requests
                    .iter()
                    .filter(|(_, _, target)| target == &p.target)
                    .map(|(id, _, _)| id.as_str())
                    .collect::<Vec<_>>();
                // Published loop copies carry their slot owners too.
                for l in &extras.loops {
                    if l.published.entry == p.target {
                        mods.extend(l.slots.iter().map(|slot| slot.mod_id.as_str()));
                    }
                }
                mods.sort_unstable();
                mods.dedup();
                Some(install_expression(&p.target, &mods, p.install.as_ref()?))
            })
            .collect(),
        bundles,
        count: generated.patched.len(),
        rows: rows.len(),
        started,
        copied,
        applied,
    }))
}

#[cfg(test)]
mod refusal_tests {
    #[test]
    fn a_missing_or_foreign_function_index_is_refused_by_name() {
        let game = tempfile::tempdir().unwrap();
        let commit = "c".repeat(40);
        let error = super::function_index(game.path(), &commit).unwrap_err();
        assert!(
            error.starts_with("E_MIXIN_INDEX: ") && error.contains("no function index"),
            "{error}"
        );
        let write = |commit: &str| {
            std::fs::write(
                game.path().join("functions.json"),
                format!(
                    r#"{{"schema":1,"build":{{"commit":"{commit}"}},"crates":{{}},"functions":{{}},"refused":{{}}}}"#
                ),
            )
            .unwrap();
        };
        write(&"d".repeat(40));
        let error = super::function_index(game.path(), &commit).unwrap_err();
        assert!(
            error.starts_with("E_MIXIN_INDEX: functions.json describes game build dddd"),
            "{error}"
        );
        write(&commit);
        assert_eq!(
            super::function_index(game.path(), &commit).unwrap(),
            Default::default()
        );
        // A tampered index whose module the copy would write as Rust is refused, not a panic.
        std::fs::write(
            game.path().join("functions.json"),
            format!(
                r#"{{"schema":1,"build":{{"commit":"{commit}"}},"crates":{{"horizon_client":{{"library":"horizon_client","modules":{{"horizon_client::game":[]}}}}}},"functions":{{"horizon_client::game::f":{{"module":"horizon_client::not a path!","file":"crates/horizon-client/src/game.rs","span":[0,1],"signature_hash":"","sha256":""}}}},"refused":{{}}}}"#
            ),
        )
        .unwrap();
        let error = super::function_index(game.path(), &commit).unwrap_err();
        assert!(
            error.starts_with("E_MIXIN_INDEX: functions.json: function `horizon_client::game::f` is in `horizon_client::not a path!`"),
            "{error}"
        );
    }

    #[test]
    fn generated_install_passes_owners_and_result_to_startup_gate() {
        assert_eq!(
            super::install_expression("game::tick", &["first", "second"], "patch::install()"),
            r#"("game::tick", &["first", "second"], patch::install())"#
        );
        // The patch library's entry module hands its installs to the loader, which passes them
        // to the startup gate before any mod's entries are registered.
        let template =
            include_str!("../../../crates/horizon-client/src/native_mod_entry.rs.template");
        let installs = template.find("fn installs()").unwrap();
        assert!(template[installs..].find("/* MOD_REDIRECTS */").is_some());
    }

    #[test]
    fn target_refusal_names_every_requesting_row() {
        let refused = super::api_gen::patch::Refusal {
            code: "E_MIXIN_NOT_REPLACEABLE",
            target: "game::tick".into(),
            why: "no slot".into(),
        };
        let requests = vec![
            ("first".into(), "before".into(), "game::tick".into()),
            ("second".into(), "after".into(), "game::tick".into()),
            ("other".into(), "row".into(), "game::other".into()),
        ];
        assert_eq!(
            super::target_refusal(&refused, &requests),
            "E_MIXIN_NOT_REPLACEABLE: first:before, second:after: game::tick: no slot"
        );
    }
}
