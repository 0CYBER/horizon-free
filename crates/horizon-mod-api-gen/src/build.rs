//! Cargo-facing generation: the mod API's and the game's generated files.
use super::{Catalog, Target, free_function_source, host, sources};
use quote::ToTokens;
use std::{fs, path::Path};
mod vocab_boundary;

fn write_vocab_output(
    out_dir: &Path,
    name: &str,
    code: &str,
    inventory: &vocab_boundary::Inventory,
    sources: &std::collections::BTreeSet<std::path::PathBuf>,
) -> Result<(), String> {
    vocab_boundary::validate_with_modules(name, code, inventory, sources)
        .map_err(|e| format!("{name}: {e}"))?;
    write_if_changed(out_dir.join(name), code).map_err(|e| e.to_string())
}

/// Preserve output timestamps when generation produces identical bytes.
pub fn write_if_changed(path: impl AsRef<Path>, bytes: impl AsRef<[u8]>) -> std::io::Result<()> {
    let path = path.as_ref();
    let bytes = bytes.as_ref();
    match fs::read(path) {
        Ok(previous) if previous == bytes => return Ok(()),
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    fs::write(path, bytes)
}

/// Generate only this crate's files, tracking every source read by the generator.
/// `root` is supplied at run time by the consuming crate's build script.
pub fn generate(root: &Path, out_dir: &Path, target: Target) -> Result<(), String> {
    track(root, !matches!(&target, Target::Vocab))?;
    let inventory = if matches!(target, Target::Vocab) {
        let path = root.join("tools/layering/mod_vocab_state.json");
        crate::rerun_if_changed(&path.display());
        vocab_boundary::inventory(root)?
    } else {
        vocab_boundary::Inventory::new()
    };
    let vocab_sources = if matches!(target, Target::Vocab) {
        vocab_boundary::modules::sources(root)?
    } else {
        std::collections::BTreeSet::new()
    };
    if matches!(&target, Target::Client) {
        crate::rerun_if_staged(root);
    }
    let generated = sources(root, super::FillMode::Require)?;
    let outputs = match target {
        Target::Client => vec![
            (
                "native_hook_modules.rs",
                horizon_game_discovery::native_hook_modules(root)?,
            ),
            ("lan_action_modules.rs", {
                let directory = root.join("crates/horizon-client/src/game/network/lan_actions");
                horizon_game_discovery::lan_action_modules(
                    &directory,
                    Catalog::modules(&directory).map_err(|e| e.to_string())?,
                )?
            }),
            ("game_host_modules.rs", game_host_modules(root)?),
            ("mod_api_fire_generated.rs", generated.fire),
            // The mob tick's goal read view (arismoko/horizon#1077), included beside the
            // engine's `GoalCx` in `mobs/part_goals.rs`.
            ("mod_api_goal_view_generated.rs", generated.goal_view),
        ],
        Target::Api | Target::Vocab => {
            if matches!(target, Target::Api) {
                write_if_changed(
                    out_dir.join("parts_registration_modules.rs"),
                    super::parts::registration_source(root)?,
                )
                .map_err(|e| e.to_string())?;
            }
            if matches!(target, Target::Vocab) {
                write_vocab_output(
                    out_dir,
                    super::parts::OUTPUT,
                    &super::parts::source(root)?,
                    &inventory,
                    &vocab_sources,
                )?;
                for (name, code) in super::exports::sources(root)? {
                    write_vocab_output(out_dir, &name, &code, &inventory, &vocab_sources)?;
                }
            }
            let hooks = generated.hooks;
            let included = [
                (host::AREAS_OUTPUT, generated.areas.as_str()),
                ("mod_api_generated.rs", hooks.as_str()),
                ("mod_api_generated_host.rs", generated.host.as_str()),
            ];
            let (providers, sources) = super::capability_providers::generate(root, &included)?;
            // The resolver recurses through the API's types: give it a deep stack.
            let generated_wire = std::thread::scope(|scope| {
                std::thread::Builder::new()
                    .stack_size(64 << 20)
                    .spawn_scoped(scope, || super::wire::generate(root, &included))
                    .map_err(|e| e.to_string())?
                    .join()
                    .map_err(|_| "generating the wire impls panicked".to_string())?
            })?;
            for path in sources.iter().chain(&generated_wire.sources) {
                crate::rerun_if_changed(&path.display());
            }
            let outputs = vec![
                (host::AREAS_OUTPUT, generated.areas),
                ("mod_api_generated.rs", hooks),
                ("mod_api_generated_host.rs", generated.host),
                ("capability_providers_generated.rs", providers),
                ("wire_generated.rs", generated_wire.wire),
                ("wire_guest_generated.rs", generated_wire.guest),
            ];
            outputs
                .into_iter()
                .filter(|(name, _)| match target {
                    Target::Api => *name == "wire_guest_generated.rs",
                    Target::Vocab => *name != "wire_guest_generated.rs",
                    Target::Client => unreachable!(),
                })
                .collect()
        }
    };
    for (name, code) in outputs {
        let code = generated_source(root, &target, name, code)?;
        if matches!(target, Target::Vocab) {
            write_vocab_output(out_dir, name, &code, &inventory, &vocab_sources)?;
        } else {
            write_if_changed(out_dir.join(name), &code).map_err(|e| e.to_string())?;
        }
        syn::parse_file(&code).map_err(|e| format!("generated Rust: {name}: {e}"))?;
    }
    Ok(())
}

fn generated_source(
    root: &Path,
    target: &Target,
    name: &str,
    code: String,
) -> Result<String, String> {
    if matches!(target, Target::Client)
        && matches!(
            name,
            "mod_api_fire_generated.rs" | "mod_api_goal_view_generated.rs"
        )
        && root.join(".horizon-source-staged").is_file()
    {
        super::internals::widen_generated(Path::new(name), code)
    } else {
        Ok(code)
    }
}

#[cfg(test)]
mod generated_visibility_tests {
    use super::*;

    #[test]
    fn staged_game_helpers_are_widened_but_source_helpers_are_unchanged() {
        let root =
            std::env::temp_dir().join(format!("fnr-generated-visibility-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let input = "impl Game<'_> { pub(super) fn fire(&mut self) {} }";
        for name in [
            "mod_api_fire_generated.rs",
            "mod_api_goal_view_generated.rs",
        ] {
            assert_eq!(
                generated_source(&root, &Target::Client, name, input.into()).unwrap(),
                input
            );
        }
        fs::write(root.join(".horizon-source-staged"), "").unwrap();
        for name in [
            "mod_api_fire_generated.rs",
            "mod_api_goal_view_generated.rs",
        ] {
            assert_eq!(
                generated_source(&root, &Target::Client, name, input.into()).unwrap(),
                "impl Game<'_> { pub fn fire(&mut self) {} }"
            );
            assert_eq!(
                generated_source(&root, &Target::Vocab, name, input.into()).unwrap(),
                input
            );
        }
        fs::remove_dir_all(root).unwrap();
    }
}

/// The game's side of the host traits: one module for each area file of `game/host/`.
pub(crate) fn game_host_modules(root: &Path) -> Result<String, String> {
    let mut modules = horizon_game_discovery::host_modules(root)?;
    modules.push_str(&format!(
        "mod generated {{\n{}\n}}\n",
        host::game_forwarders(root)?
    ));
    Ok(modules)
}

/// Generate the native-only raw render records, without trait dispatch or subscriptions.
pub fn generate_render(root: &Path, out_dir: &Path) -> Result<(), String> {
    let declarations = root.join(super::catalog::DECLARATIONS);
    crate::rerun_if_changed(&declarations.display());
    for module in Catalog::modules(&declarations)? {
        crate::rerun_if_changed(&declarations.join(format!("{module}.rs")).display());
    }
    crate::rerun_if_changed(
        &root
            .join("crates/horizon-mod-render/src/render.rs")
            .display(),
    );
    let source = super::dispatch::render(root, &Catalog::load_declarations(root)?)?.render_dispatch;
    use syn::spanned::Spanned;
    let file = syn::parse_file(&source).map_err(|error| error.to_string())?;
    let lines: Vec<_> = std::iter::once(0)
        .chain(source.match_indices('\n').map(|(index, _)| index + 1))
        .collect();
    let offset = |point: proc_macro2::LineColumn| lines[point.line - 1] + point.column;
    let mut records = String::from("use super::*;\n");
    for item in file.items {
        let syn::Item::Struct(record) = item else {
            continue;
        };
        let start = record
            .attrs
            .iter()
            .find(|attr| !attr.path().is_ident("doc"))
            .map_or_else(|| record.vis.span().start(), |attr| attr.span().start());
        records.push_str(&source[offset(start)..offset(record.span().end())]);
        records.push('\n');
    }
    syn::parse_file(&records).map_err(|error| error.to_string())?;
    write_if_changed(out_dir.join("native_render_generated.rs"), records)
        .map_err(|error| error.to_string())
}

#[test]
fn render_records_need_no_game_sources() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = workspace.join(format!("target/render-records-only-{}", std::process::id()));
    let declarations = root.join(super::catalog::DECLARATIONS);
    fs::create_dir_all(&declarations).unwrap();
    for entry in fs::read_dir(workspace.join(super::catalog::DECLARATIONS)).unwrap() {
        let path = entry.unwrap().path();
        if path.is_file() {
            fs::copy(&path, declarations.join(path.file_name().unwrap())).unwrap();
        }
    }
    let raw = "crates/horizon-mod-render/src/render.rs";
    fs::create_dir_all(root.join(raw).parent().unwrap()).unwrap();
    fs::copy(workspace.join(raw), root.join(raw)).unwrap();
    assert!(!root.join("crates/horizon-client").exists());
    generate_render(&root, &root).unwrap();
    let generated = fs::read_to_string(root.join("native_render_generated.rs")).unwrap();
    let file = syn::parse_file(&generated).unwrap();
    assert_eq!(
        file.items
            .iter()
            .filter(|item| matches!(item, syn::Item::Struct(_)))
            .count(),
        5
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "daily"]
fn listed_render_slots_have_eligible_points() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    super::points::game_points(&root).unwrap();
}

/// The `mod_api` types with a `Wire` layout ([`super::wire`]), for point records.
/// A source tree without the mod API (a generator fixture's) has none.
pub(crate) fn wire_types(root: &Path) -> Result<std::collections::BTreeSet<String>, String> {
    if !root.join("crates/horizon-mod-vocab/src/hooks").is_dir() {
        return Ok(Default::default());
    }
    let generated = sources(root, super::FillMode::Require)?;
    wire_types_from_sources(root, &generated)
}

/// Resolve Wire layouts from already validated, immutable API sources.
/// The caller has established that the mod API directory exists.
pub(super) fn wire_types_from_sources(
    root: &Path,
    generated: &super::Sources,
) -> Result<std::collections::BTreeSet<String>, String> {
    let included = [
        (host::AREAS_OUTPUT, generated.areas.as_str()),
        ("mod_api_generated.rs", generated.hooks.as_str()),
        ("mod_api_generated_host.rs", generated.host.as_str()),
    ];
    // The resolver recurses through the API's types: give it a deep stack.
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(64 << 20)
            .spawn_scoped(scope, || super::wire::owned_types(root, &included))
            .map_err(|e| e.to_string())?
            .join()
            .map_err(|_| "reading the wire types panicked".to_string())?
    })
}

/// `horizon-mod-points`' generated file: each point's record, `PointId` and typed function constructors,
/// from the game's source, tracking the game module's files.
pub fn generate_points(root: &Path, out_dir: &Path) -> Result<(), String> {
    crate::interface_cache::with_cache(out_dir, || {
        crate::source::with_walk_cache(|| generate_points_inner(root, out_dir))
    })
}

/// Generate the engine-owned point vocabulary and typed dispatch.
pub fn generate_engine_points(root: &Path, out_dir: &Path) -> Result<(), String> {
    crate::interface_cache::with_cache(out_dir, || {
        crate::source::with_walk_cache(|| {
            for path in super::points::tracked(root)? {
                crate::rerun_if_changed(&path.display());
            }
            crate::rerun_if_staged(root);
            let points = super::points::game_points(root)?;
            println!("cargo:rerun-if-env-changed=HORIZON_POINT_NAMES");
            let selected = std::env::var("HORIZON_POINT_NAMES")
                .ok()
                .map(|json| {
                    serde_json::from_str::<std::collections::BTreeSet<String>>(&json)
                        .map_err(|error| format!("HORIZON_POINT_NAMES: {error}"))
                })
                .transpose()?;
            let code = super::points::engine_records(&points, selected.as_ref());
            let generated = syn::parse_file(&code).map_err(|e| format!("engine points: {e}"))?;
            let mut code = String::new();
            for item in generated.items {
                code.push_str(&item.to_token_stream().to_string());
                code.push('\n');
            }
            write_if_changed(out_dir.join("engine_points.rs"), code).map_err(|e| e.to_string())
        })
    })
}

pub(super) fn generate_points_inner(root: &Path, out_dir: &Path) -> Result<(), String> {
    // Phase durations for CI's build report (points-timing.txt beside the output); never read back.
    let started = std::time::Instant::now();
    let mut timing = String::new();
    let mut phase = |name: &str| {
        timing.push_str(&format!("{name} {:.2}s\n", started.elapsed().as_secs_f64()));
    };
    let src = root.join("crates/horizon-client/src");
    crate::rerun_if_changed(&src.join("game.rs").display());
    crate::rerun_if_changed(&src.join("game").display());
    for path in super::points::tracked(root)? {
        crate::rerun_if_changed(&path.display());
    }
    phase("tracked");
    println!("cargo:rerun-if-env-changed=HORIZON_POINT_NAMES");
    let selected = std::env::var("HORIZON_POINT_NAMES")
        .ok()
        .map(|json| {
            serde_json::from_str::<std::collections::BTreeSet<String>>(&json)
                .map_err(|error| format!("HORIZON_POINT_NAMES: {error}"))
        })
        .transpose()?;
    // Only a staged copy compiles the points' sites into the game; the wasm host's listener
    // fixture brings its own.
    crate::rerun_if_staged(root);
    let staged =
        root.join(".horizon-source-staged").is_file() || cfg!(feature = "listener-fixture");
    let points = super::points::game_points(root)?;
    phase("game_points");
    let code = super::points::records_selected(&points, selected.as_ref())
        + &format!(
            "/// Whether this build compiled the points' sites into the game: only the prebuilt game, built from a staged copy of the source, has them. A build without them refuses every listener at load.\npub const STAGED: bool = {staged};\n"
        );
    phase("records");
    let generated = syn::parse_file(&code).map_err(|e| format!("generated Rust: points: {e}"))?;
    // A single multi-megabyte line makes Clippy's source lookups prohibitively slow.
    let mut code = String::new();
    for item in generated.items {
        code.push_str(&item.to_token_stream().to_string());
        code.push('\n');
    }
    phase("format");
    write_if_changed(out_dir.join("points_generated.rs"), code).map_err(|e| e.to_string())?;
    phase("written");
    fs::write(out_dir.join("points-timing.txt"), timing).map_err(|e| e.to_string())
}

pub(super) fn track(root: &Path, all_game: bool) -> Result<(), String> {
    fn directory(path: &Path, extension: &str, recursive: bool) -> Result<(), String> {
        // Track the directory as well: adding a helper source or fragment changes the inputs.
        crate::rerun_if_changed(&path.display());
        for entry in fs::read_dir(path).map_err(|e| e.to_string())? {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.is_dir() && recursive {
                directory(&path, extension, recursive)?;
            } else if path.extension().is_some_and(|ext| ext == extension) {
                crate::rerun_if_changed(&path.display());
            }
        }
        Ok(())
    }
    crate::rerun_if_changed(&root.join("crates/horizon-mod-vocab/src").display());
    for retired in super::catalog::RETIRED {
        // Only while present: Cargo reruns a build script on every build for a path that doesn't
        // exist. One that appears fails the next generation, and every fresh build, such as CI's.
        let path = root.join(retired);
        if path.exists() {
            crate::rerun_if_changed(&path.display());
        }
    }
    directory(&root.join(super::catalog::DECLARATIONS), "rs", false)?;
    directory(&root.join(super::exports::DIRECTORY), "rs", false)?;
    directory(
        &root.join("crates/horizon-mod-vocab/src/parts"),
        "rs",
        false,
    )?;
    if all_game {
        directory(&root.join("crates/horizon-client/src/game"), "rs", true)?;
        crate::rerun_if_changed(&root.join("crates/horizon-client/src/game.rs").display());
    } else {
        directory(
            &root.join("crates/horizon-client/src/game/native_hooks"),
            "rs",
            false,
        )?;
    }
    for hook in Catalog::load(root)?.hooks {
        if let Some((krate, path)) = free_function_source(&hook.target) {
            let (module, _) = path.rsplit_once("::").ok_or("target missing module")?;
            let path = root
                .join("crates")
                .join(krate)
                .join("src")
                .join(format!("{}.rs", module.replace("::", "/")));
            crate::rerun_if_changed(&path.display());
        }
    }
    Ok(())
}
