//! Physical game areas share one logical module scope; compiler and source tools use this list.
use crate::catalog::Catalog;
use std::{fs, path::Path};

pub(crate) const INCLUDE: &str = "include!(concat!(env!(\"OUT_DIR\"), \"/game_modules.rs\"));";

pub(crate) fn declarations(game: &Path) -> Result<String, String> {
    horizon_game_discovery::declarations(game, &["native_hooks", "host", "creatures"])
}

/// Expand the same directory-derived include before source traversal, without needing OUT_DIR.
/// Other includes are left alone: source tools must not evaluate arbitrary build-time code.
pub(crate) fn expand(path: &Path, source: &str) -> Result<String, String> {
    const NATIVE: &str = "include!(concat!(env!(\"OUT_DIR\"), \"/native_hook_modules.rs\"));";
    const LAN: &str = "include!(concat!(env!(\"OUT_DIR\"), \"/lan_action_modules.rs\"));";
    const HOST: &str = "include!(concat!(env!(\"OUT_DIR\"), \"/game_host_modules.rs\"));";
    if !source.contains(INCLUDE)
        && !source.contains(NATIVE)
        && !source.contains(LAN)
        && !source.contains(HOST)
    {
        return Ok(source.to_owned());
    }
    let file = syn::parse_file(source).map_err(|e| e.to_string())?;
    let mut lines: Vec<_> = source.split_inclusive('\n').map(str::to_owned).collect();
    for item in file.items {
        if let syn::Item::Macro(item) = item
            && item.mac.path.is_ident("include")
        {
            let line = item.mac.path.segments[0].ident.span().start().line - 1;
            let declarations = match lines[line].trim_end_matches(['\r', '\n']) {
                INCLUDE => declarations(&path.with_extension(""))?,
                LAN => {
                    let directory = path.parent().ok_or("LAN actions source has no parent")?;
                    horizon_game_discovery::lan_action_modules(
                        directory,
                        Catalog::modules(directory).map_err(|e| e.to_string())?,
                    )?
                }
                HOST => crate::build::game_host_modules(
                    path.canonicalize()
                        .map_err(|e| e.to_string())?
                        .ancestors()
                        .nth(6)
                        .ok_or("game host source outside the game tree")?,
                )?,
                NATIVE => horizon_game_discovery::native_hook_modules(
                    path.canonicalize()
                        .map_err(|e| e.to_string())?
                        .ancestors()
                        .nth(6)
                        .ok_or("native hook source outside the game tree")?,
                )?,
                _ => continue,
            };
            // OUT_DIR lists need absolute paths, but materialized source must survive
            // the launcher's stage.new -> stage copy and removal of the original tree.
            let directory = path
                .parent()
                .ok_or("game source has no parent")?
                .canonicalize()
                .map_err(|e| e.to_string())?;
            let mut declarations = syn::parse_file(&declarations).map_err(|e| e.to_string())?;
            for item in &mut declarations.items {
                let syn::Item::Mod(item) = item else { continue };
                for attribute in &mut item.attrs {
                    if let syn::Meta::NameValue(value) = &mut attribute.meta
                        && value.path.is_ident("path")
                        && let syn::Expr::Lit(value) = &mut value.value
                        && let syn::Lit::Str(literal) = &mut value.lit
                    {
                        let absolute = literal.value();
                        let relative = Path::new(&absolute)
                            .strip_prefix(&directory)
                            .map_err(|e| e.to_string())?
                            .to_string_lossy();
                        *literal = syn::LitStr::new(&relative, literal.span());
                    }
                }
            }
            lines[line] = prettyplease::unparse(&declarations);
        }
    }
    Ok(lines.concat())
}

/// Materialize generated module lists in a disposable game source copy before external tools
/// traverse it. Uses the same source view as point discovery and internals widening, without
/// widening visibility, adding points or requiring an OUT_DIR. Never pass a downloaded cache.
pub fn materialize_game_modules(root: &Path) -> Result<(), String> {
    for file in crate::source::game_files(&root.join("crates/horizon-client/src"), false)? {
        let source =
            fs::read_to_string(&file.path).map_err(|e| format!("{}: {e}", file.path.display()))?;
        let expanded = expand(&file.path, &source)?;
        if expanded != source {
            fs::write(&file.path, expanded).map_err(|e| format!("{}: {e}", file.path.display()))?;
        }
    }
    Ok(())
}

pub(crate) fn parse(path: &Path, source: &str) -> Result<syn::File, String> {
    syn::parse_file(&expand(path, source)?).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn materialization_writes_a_plain_module_view_without_instrumentation() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target")
            .join(format!("materialized-areas-{}", std::process::id()));
        let src = root.join("crates/horizon-client/src");
        let area = src.join("game/runtime");
        fs::create_dir_all(&area).unwrap();
        fs::write(area.join(".game-area"), "").unwrap();
        fs::write(src.join("game.rs"), format!("{INCLUDE}\n")).unwrap();
        let body = "fn untouched() -> u32 { 7 }";
        fs::write(area.join("feature.rs"), body).unwrap();
        materialize_game_modules(&root).unwrap();
        let source = fs::read_to_string(src.join("game.rs")).unwrap();
        let file = syn::parse_file(&source).unwrap();
        let [syn::Item::Mod(module)] = file.items.as_slice() else {
            panic!("{source}")
        };
        assert_eq!(module.ident, "feature");
        let syn::Meta::NameValue(path) = &module.attrs[0].meta else {
            panic!("{source}")
        };
        let syn::Expr::Lit(value) = &path.value else {
            panic!("{source}")
        };
        let syn::Lit::Str(relative) = &value.lit else {
            panic!("{source}")
        };
        assert_eq!(
            fs::read_to_string(src.join(relative.value())).unwrap(),
            body
        );
        assert_eq!(fs::read_to_string(area.join("feature.rs")).unwrap(), body);
        materialize_game_modules(&root).unwrap();
        assert_eq!(fs::read_to_string(src.join("game.rs")).unwrap(), source);
        fs::remove_dir_all(root).unwrap();
    }

    /// Git for Windows checks sources out with CRLF by default.
    #[test]
    fn an_include_is_expanded_whatever_ends_its_line() {
        let directory = std::env::temp_dir().join(format!("lan-line-ends-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let root = directory.join("mod.rs");
        fs::write(directory.join("turns.rs"), "").unwrap();
        for end in ["\n", "\r\n"] {
            let source =
                format!("include!(concat!(env!(\"OUT_DIR\"), \"/lan_action_modules.rs\"));{end}");
            fs::write(&root, &source).unwrap();
            let code = expand(&root, &source).unwrap();
            assert!(code.contains("mod turns;"), "{end:?}");
        }
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn lan_actions_are_discovered_without_lists_and_reject_shared_methods() {
        let directory = std::env::temp_dir().join(format!("lan-actions-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let include = "include!(concat!(env!(\"OUT_DIR\"), \"/lan_action_modules.rs\"));";
        let root = directory.join("mod.rs");
        fs::write(&root, include).unwrap();
        fs::write(directory.join("shared.rs"), "struct State;").unwrap();
        fs::write(
            directory.join("turns.rs"),
            "impl Game { fn turn(&self) {} }",
        )
        .unwrap();
        let code = expand(&root, include).unwrap();
        assert!(code.contains("mod turns;"));
        assert!(!code.contains("mod shared;") && !code.contains("mod mod;"));
        fs::write(directory.join("new_area.rs"), "").unwrap();
        assert!(expand(&root, include).unwrap().contains("mod new_area;"));
        fs::write(
            directory.join("shared.rs"),
            "impl Game<'_> { fn vehicle_has_other_viewers() {} } impl Game<'_> { fn __fnr_vehicle_has_other_viewers(f: fn()) -> bool { true } fn __fnr_get_vehicle_has_other_viewers() -> Option<fn()> { None } }", 
        )
        .unwrap();
        assert!(
            horizon_game_discovery::lan_action_modules(
                &directory,
                Catalog::modules(&directory).unwrap()
            )
            .is_ok()
        );
        for file in ["shared.rs", "mod.rs"] {
            fs::write(
                directory.join(file),
                "impl Game<'_> { fn misplaced(&self) {} }",
            )
            .unwrap();
            assert!(
                horizon_game_discovery::lan_action_modules(
                    &directory,
                    Catalog::modules(&directory).unwrap()
                )
                .unwrap_err()
                .contains("action area")
            );
            fs::write(directory.join(file), "").unwrap();
        }
        fs::remove_dir_all(directory).unwrap();
        let game = Path::new(env!("CARGO_MANIFEST_DIR")).join("../horizon-client/src/game");
        let directory = game.join("network/lan_actions");
        let actual = horizon_game_discovery::lan_action_modules(
            &directory,
            Catalog::modules(&directory).unwrap(),
        )
        .unwrap();
        assert!(!actual.contains("mod shared;"));
        assert!(actual.contains("mod packets;"));
    }

    #[test]
    #[ignore = "daily"]
    fn existing_game_exports_keep_their_visibility() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../horizon-client/src");
        let code = declarations(&src.join("game")).unwrap();
        for (name, visibility) in [
            ("cushions", "pub(crate)"),
            ("mods", "pub(crate)"),
            ("native_block_entity_draw", "pub"),
            ("native_client_services", "pub"),
        ] {
            assert!(code.contains(&format!("{visibility} mod {name};")));
        }
        let files = crate::source::game_files(&src, true).unwrap();
        let game_tests = "horizon_client::game::game_tests";
        assert!(files.iter().any(|file| file.module == game_tests));
    }

    #[test]
    fn discovery_preserves_scope_cfg_and_visibility_and_rejects_drift() {
        let root = std::env::temp_dir().join(format!("game-areas-{}", std::process::id()));
        let game = root.join("game");
        let area = game.join("network");
        fs::create_dir_all(&area).unwrap();
        fs::write(area.join(".game-area"), "").unwrap();
        let source = format!("const DOC: &str = r#\"\n{INCLUDE}\n\"#;\n{INCLUDE}");
        fs::write(root.join("game.rs"), &source).unwrap();
        assert!(
            expand(&root.join("game.rs"), &source)
                .unwrap()
                .contains(INCLUDE)
        );
        fs::create_dir_all(area.join("remote")).unwrap();
        fs::write(area.join("remote/mod.rs"), "mod child;").unwrap();
        fs::write(area.join("remote/child.rs"), "").unwrap();
        fs::write(area.join("probe_tests.rs"), "").unwrap();
        fs::write(area.join("game_tests.rs"), "// game-module-cfg: always\n").unwrap();
        fs::write(
            area.join("service.rs"),
            "// game-module-visibility: pub(crate)\n",
        )
        .unwrap();
        let files = crate::source::game_files(&root, false).unwrap();
        let names: Vec<_> = files.iter().map(|file| file.module.as_str()).collect();
        assert!(names.contains(&"horizon_client::game::remote::child"));
        assert!(names.contains(&"horizon_client::game::game_tests"));
        assert!(
            !names
                .iter()
                .any(|name| name.contains("probe_tests") || name.contains("network"))
        );
        let code = declarations(&game).unwrap();
        assert!(code.contains("pub(crate) mod service;"));
        fs::write(area.join("new_feature.rs"), "").unwrap();
        assert!(declarations(&game).unwrap().contains("mod new_feature;"));
        fs::write(game.join("loose.rs"), "").unwrap();
        assert!(declarations(&game).unwrap_err().contains("area folder"));
        fs::remove_file(game.join("loose.rs")).unwrap();
        let other = game.join("world");
        fs::create_dir(&other).unwrap();
        fs::write(other.join(".game-area"), "").unwrap();
        fs::write(other.join("remote.rs"), "").unwrap();
        assert!(
            declarations(&game)
                .unwrap_err()
                .contains("duplicate game module remote")
        );
        fs::remove_dir_all(root).unwrap();
    }
}
