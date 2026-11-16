//! Resolve named hooks outside the fillable native_hooks directory through logical game modules.
use super::*;

pub(super) fn resolve(root: &Path, target: &str, name: &str) -> Result<syn::ItemFn, String> {
    let native = format!("native_{name}");
    let mut found = Vec::new();
    for file in source::game_files(&root.join("crates/horizon-client/src"), false)? {
        let source = fs::read_to_string(&file.path).map_err(|e| e.to_string())?;
        let parsed = syn::parse_file(&source).map_err(|e| e.to_string())?;
        collect(&parsed.items, &file.module, &file.cfg, &native, &mut found);
    }

    if found.len() != 1 {
        return Err(format!(
            "{name}: expected exactly one {native} Game method under game/, found {}",
            found.len()
        ));
    }
    let (module, cfg, method) = found.pop().unwrap();
    let expected = format!("{module}::Game<'_>::{native}");
    if target != expected {
        return Err(format!(
            "wrong target for {name}: use {expected}, not {target}"
        ));
    }
    struct Unhookable(bool);
    impl<'ast> Visit<'ast> for Unhookable {
        fn visit_macro(&mut self, _: &'ast syn::Macro) {
            self.0 = true;
        }
        fn visit_attribute(&mut self, attr: &'ast syn::Attribute) {
            self.0 |= attr.path().is_ident("cfg") || attr.path().is_ident("cfg_attr");
        }
    }
    let mut invalid = Unhookable(!cfg.is_empty());
    invalid.visit_item_fn(&method);
    if invalid.0 {
        return Err(format!(
            "{name}: remove macro, cfg or cfg_attr from the hook point; it cannot be patched"
        ));
    }
    if !has_call(&method.block, &format!("dispatch_{name}")) {
        return Err(format!(
            "{name}: add dispatch_{name}; hooks outside native_hooks make their own dispatch call and cannot be filled"
        ));
    }
    Ok(method)
}

/// Collect inline modules as well as methods directly in a resolved source file.
fn collect(
    items: &[Item],
    module: &str,
    cfg: &[String],
    native: &str,
    found: &mut Vec<(String, Vec<String>, syn::ItemFn)>,
) {
    for item in items {
        match item {
            Item::Mod(child) if child.content.is_some() => {
                let mut inherited = cfg.to_vec();
                inherited.extend(
                    child
                        .attrs
                        .iter()
                        .filter(|a| a.path().is_ident("cfg") || a.path().is_ident("cfg_attr"))
                        .map(|a| a.to_token_stream().to_string()),
                );
                collect(
                    &child.content.as_ref().unwrap().1,
                    &format!("{module}::{}", child.ident),
                    &inherited,
                    native,
                    found,
                );
            }
            Item::Impl(item) => {
                let Type::Path(ty) = &*item.self_ty else {
                    continue;
                };
                if item.trait_.is_some()
                    || !ty.path.segments.last().is_some_and(|s| s.ident == "Game")
                {
                    continue;
                }
                for mut method in impl_functions(item.clone()).filter(|m| m.sig.ident == native) {
                    method.attrs.extend(item.attrs.clone());
                    found.push((module.to_owned(), cfg.to_vec(), method));
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enclosing_inline_module_cfg_refuses_named_hook() {
        let root =
            std::env::temp_dir().join(format!("named-hooks-inherited-cfg-{}", std::process::id()));
        let src = root.join("crates/horizon-client/src");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("game.rs"), r#"#[cfg(feature = "x")] mod inner { impl Game<'_> { fn native_probe(&self) { dispatch_probe(); } } }"#).unwrap();
        let error = resolve(
            &root,
            "horizon_client::game::inner::Game<'_>::native_probe",
            "probe",
        )
        .err()
        .unwrap();
        assert!(error.contains("macro, cfg or cfg_attr"), "{error}");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn logical_game_hook_discovery_and_refusals() {
        let root = std::env::temp_dir().join(format!("named-hooks-{}", std::process::id()));
        let src = root.join("crates/horizon-client/src");
        fs::create_dir_all(src.join("game/nested")).unwrap();
        fs::write(src.join("game.rs"), "mod nested;").unwrap();
        fs::write(
            src.join("game/nested/mod.rs"),
            "#[path = \"point.rs\"] mod point;",
        )
        .unwrap();
        let path = src.join("game/nested/point.rs");
        let target = "horizon_client::game::nested::point::Game<'_>::native_probe";
        let good = "fn native_probe(&self) { mod_api::dispatch_probe(self, mod_api::mods()); }";
        let write = |body: &str| fs::write(&path, format!("impl Game<'_> {{ {body} }}")).unwrap();
        write(good);
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let catalog = Catalog::load(&repo).unwrap();
        let mut hook = catalog
            .hooks
            .iter()
            .find(|h| h.contract == Contract::Listen && h.scope.is_none())
            .unwrap()
            .clone();
        hook.name = "probe".into();
        hook.target = target.into();
        hook.call.clear();
        hook.context = None;
        hook.owner = None;
        let emitted = dispatch::emit(&root, std::iter::once(&hook), &[], &[]).unwrap();
        assert!(emitted.dispatch.contains("dispatch_probe"));
        assert!(emitted.fills.is_empty());

        assert_eq!(
            resolve(&root, target, "probe").unwrap().sig.ident,
            "native_probe"
        );
        assert!(
            resolve(
                &root,
                "horizon_client::game::wrong::Game<'_>::native_probe",
                "probe"
            )
            .err()
            .unwrap()
            .contains("wrong target for probe")
        );
        for (body, error) in [
            (
                "fn native_probe(&self) { let _ = (); }",
                "add dispatch_probe",
            ),
            (
                "fn native_probe(&self) { log!(); dispatch_probe(); }",
                "macro, cfg or cfg_attr",
            ),
            (
                "#[cfg(feature = \"x\")] fn native_probe(&self) { dispatch_probe(); }",
                "macro, cfg or cfg_attr",
            ),
            (
                "#[cfg_attr(feature = \"x\", inline)] fn native_probe(&self) { dispatch_probe(); }",
                "macro, cfg or cfg_attr",
            ),
            ("", "found 0"),
        ] {
            write(body);
            let actual = resolve(&root, target, "probe").err().unwrap();
            assert!(
                actual.contains("probe") && actual.contains(error),
                "{actual}"
            );
        }
        write(&format!("{good} {good}"));
        assert!(
            resolve(&root, target, "probe")
                .err()
                .unwrap()
                .contains("found 2")
        );
        fs::write(&path, format!("mod inner {{ impl Game<'_> {{ {good} }} }}")).unwrap();
        assert!(
            resolve(
                &root,
                &target.replace("::point::", "::point::inner::"),
                "probe"
            )
            .is_ok()
        );
        fs::remove_dir_all(root).unwrap();
    }
}
