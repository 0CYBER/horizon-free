//! Differential coverage of prepared dispatch, without stored generated output.
use quote::ToTokens;
use std::collections::{BTreeMap, BTreeSet};

fn hook_entry(hook: &str, name: &str) -> bool {
    name == hook
        || ["prepare", "ask", "release"]
            .iter()
            .any(|prefix| name == format!("{prefix}_{hook}"))
}
fn host_items(file: &syn::File, hook: &str) -> BTreeMap<String, String> {
    let mut items = BTreeMap::new();
    for item in &file.items {
        if let syn::Item::Impl(item) = item {
            let syn::Type::Path(receiver) = &*item.self_ty else {
                continue;
            };
            if !receiver.path.segments.last().is_some_and(|segment| {
                matches!(
                    segment.ident.to_string().as_str(),
                    "WasmMod" | "WasmHandler"
                )
            }) {
                continue;
            }
            let receiver = item.self_ty.to_token_stream().to_string();
            let owner = item
                .trait_
                .as_ref()
                .map(|(_, path, _)| path.to_token_stream().to_string())
                .unwrap_or_default();
            for member in &item.items {
                if let syn::ImplItem::Fn(f) = member
                    && hook_entry(hook, &f.sig.ident.to_string())
                {
                    let key = format!("{receiver}::{owner}::{}", f.sig.ident);
                    assert!(items.insert(key, f.to_token_stream().to_string()).is_none());
                }
            }
        }
    }
    items
}
fn export_ids(file: &syn::File) -> BTreeMap<(String, String), u32> {
    let mut ids = BTreeMap::new();
    let f = file
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Fn(f) if f.sig.ident == "export_id" => Some(f),
            _ => None,
        })
        .expect("host export lookup");
    for stmt in &f.block.stmts {
        if let syn::Stmt::Expr(syn::Expr::Match(m), _) = stmt {
            for arm in &m.arms {
                let syn::Pat::Tuple(pat) = &arm.pat else {
                    continue;
                };
                let names: Vec<_> = pat
                    .elems
                    .iter()
                    .filter_map(|p| {
                        if let syn::Pat::Lit(syn::ExprLit {
                            lit: syn::Lit::Str(s),
                            ..
                        }) = p
                        {
                            Some(s.value())
                        } else {
                            None
                        }
                    })
                    .collect();
                let [owner, name] = names.as_slice() else {
                    panic!("export owner and name");
                };
                let syn::Expr::Call(call) = &*arm.body else {
                    panic!("export id result");
                };
                let syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Int(id),
                    ..
                }) = &call.args[0]
                else {
                    panic!("export id literal");
                };
                assert!(
                    ids.insert((owner.clone(), name.clone()), id.base10_parse().unwrap())
                        .is_none()
                );
            }
        }
    }
    assert!(!ids.is_empty());
    ids
}
fn guest_items(
    file: &syn::File,
    ids: &BTreeMap<(String, String), u32>,
    hook: &str,
) -> BTreeMap<String, String> {
    let mut items = BTreeMap::new();
    for item in &file.items {
        if let syn::Item::Fn(f) = item {
            let name = f.sig.ident.to_string();
            if name
                .strip_prefix("entry_")
                .is_some_and(|name| hook_entry(hook, name))
            {
                assert!(
                    items
                        .insert(name, f.to_token_stream().to_string())
                        .is_none()
                );
            }
            if f.sig.ident != "hook" {
                continue;
            }
            for stmt in &f.block.stmts {
                if let syn::Stmt::Expr(syn::Expr::Match(m), _) = stmt {
                    for ((owner, name), id) in
                        ids.iter().filter(|((_, name), _)| hook_entry(hook, name))
                    {
                        if let Some(arm) = m
                            .arms
                            .iter()
                            .find(|arm| arm.pat.to_token_stream().to_string() == id.to_string())
                        {
                            assert!(
                                items
                                    .insert(
                                        format!("{owner}::{name} dispatch"),
                                        arm.to_token_stream().to_string()
                                    )
                                    .is_none()
                            );
                        }
                    }
                }
            }
        }
    }
    items
}
fn on_stack(run: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(run)
        .unwrap()
        .join()
        .unwrap();
}
#[test]
#[ignore = "daily"]
fn only_can_equip_changes_when_prepared_marker_is_removed() {
    on_stack(|| {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let sources = crate::sources(&root, crate::FillMode::Require).unwrap();
        let providers = crate::capability_providers::generate(&root, &[]).unwrap().0;
        let included = [
            ("mod_api_generated.rs", sources.hooks.as_str()),
            ("mod_api_generated_host.rs", sources.host.as_str()),
            ("mod_api_host_areas.rs", sources.areas.as_str()),
            ("capability_providers_generated.rs", providers.as_str()),
        ];
        let mut catalog = crate::Catalog::load(&root).unwrap();
        // These are the existing production emitters and their parsed-catalog input.
        // Both runs use the exact same native API sources; only the marker changes.
        let generate = |catalog: &crate::Catalog| {
            let markers = catalog
                .hooks
                .iter()
                .filter_map(|hook| {
                    hook.prepared
                        .as_ref()
                        .map(|parameter| (hook.name.clone(), parameter.clone()))
                })
                .collect::<Vec<_>>();
            let isolated = catalog
                .hooks
                .iter()
                .filter(|hook| hook.isolate)
                .map(|hook| hook.name.clone())
                .collect::<std::collections::HashSet<_>>();
            let mut surface = super::surface(&root, &included).unwrap();
            let host = super::host::generate(&mut surface, &markers, &isolated).unwrap();
            let guest = super::calls::dispatch(&surface.types, &surface.methods, &markers).0;
            (
                syn::parse_file(&host).unwrap(),
                syn::parse_file(&guest).unwrap(),
            )
        };
        let (prepared_host, prepared_guest) = generate(&catalog);
        assert_eq!(
            catalog
                .hooks
                .iter_mut()
                .find(|hook| hook.name == "can_equip")
                .unwrap()
                .prepared
                .take()
                .as_deref(),
            Some("stack")
        );
        let (ordinary_host, ordinary_guest) = generate(&catalog);
        let prepared_ids = export_ids(&prepared_host);
        let ordinary_ids = export_ids(&ordinary_host);
        let mut host_changes = BTreeSet::new();
        let mut guest_changes = BTreeSet::new();
        for hook in &catalog.hooks {
            let host_differs =
                host_items(&prepared_host, &hook.name) != host_items(&ordinary_host, &hook.name);
            let guest_differs = guest_items(&prepared_guest, &prepared_ids, &hook.name)
                != guest_items(&ordinary_guest, &ordinary_ids, &hook.name);
            assert_eq!(
                host_differs,
                hook.name == "can_equip",
                "host proxy {}",
                hook.name
            );
            assert_eq!(
                guest_differs,
                hook.name == "can_equip",
                "guest export {}",
                hook.name
            );
            if host_differs {
                host_changes.insert(hook.name.as_str());
            }
            if guest_differs {
                guest_changes.insert(hook.name.as_str());
            }
        }
        assert_eq!(host_changes, BTreeSet::from(["can_equip"]));
        assert_eq!(guest_changes, BTreeSet::from(["can_equip"]));
    });
}
#[test]
#[ignore = "daily"]
fn prepared_fingerprint_covers_marker_entries_and_argument_layouts() {
    on_stack(|| {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let sources = crate::sources(&root, crate::FillMode::Require).unwrap();
        let providers = crate::capability_providers::generate(&root, &[]).unwrap().0;
        let included = [
            ("mod_api_generated.rs", sources.hooks.as_str()),
            ("mod_api_generated_host.rs", sources.host.as_str()),
            ("mod_api_host_areas.rs", sources.areas.as_str()),
            ("capability_providers_generated.rs", providers.as_str()),
        ];
        let surface = super::surface(&root, &included).unwrap();
        let abis =
            super::prepared::describe(&surface.methods, &[("can_equip".into(), "stack".into())])
                .unwrap();
        let baseline = super::prepared::fingerprint(&abis);
        assert_ne!(baseline, super::prepared::fingerprint(&[]));
        let mut changed = abis.clone();
        changed[0].parameter = "another_stack".into();
        assert_ne!(baseline, super::prepared::fingerprint(&changed));
        for i in 0..3 {
            let mut changed = abis.clone();
            changed[0].entries[i].name.push_str("_changed");
            assert_ne!(baseline, super::prepared::fingerprint(&changed));
            let mut changed = abis.clone();
            changed[0].entries[i].id ^= 1;
            assert_ne!(baseline, super::prepared::fingerprint(&changed));
            let mut changed = abis.clone();
            changed[0].entries[i].input[0] = "slot:u64";
            assert_ne!(baseline, super::prepared::fingerprint(&changed));
            assert_ne!(
                super::prepared::layout_code(&abis),
                super::prepared::layout_code(&changed)
            );
            let mut changed = abis.clone();
            changed[0].entries[i].output[0] = "answer:u64";
            assert_ne!(baseline, super::prepared::fingerprint(&changed));
        }
    });
}

/// The block of every `fn name` in `items`, nested modules included.
fn blocks(items: &[syn::Item], name: &dyn Fn(&str) -> bool, out: &mut Vec<(String, String)>) {
    for item in items {
        match item {
            syn::Item::Fn(f) if name(&f.sig.ident.to_string()) => out.push((
                f.sig.ident.to_string(),
                f.block.to_token_stream().to_string(),
            )),
            syn::Item::Mod(module) => {
                if let Some((_, content)) = &module.content {
                    blocks(content, name, out);
                }
            }
            _ => {}
        }
    }
}
fn block(text: &str) -> String {
    syn::parse_str::<syn::Block>(text)
        .unwrap()
        .to_token_stream()
        .to_string()
}

/// A guest answers the host with its output's pointer in the high half of a `u64` and its length
/// in the low half, as the host unpacks it (`horizon-mod-wasm`'s runtime, pinned by its WAT
/// guests). The guest side is wasm-only code no native test runs (#18400): `horizon_hook` and each
/// prepared hook's `prepare_`, `ask_` and `release_` exports are pinned here as written.
#[test]
fn guest_exports_pack_the_pointer_high_and_the_length_low() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source =
        std::fs::read_to_string(root.join("crates/horizon-mod-api/src/wire/wire_guest.rs"))
            .unwrap();
    let mut hook = Vec::new();
    blocks(
        &syn::parse_file(&source).unwrap().items,
        &|name| name == "horizon_hook",
        &mut hook,
    );
    assert_eq!(
        hook,
        [(
            "horizon_hook".to_owned(),
            block(
                "{ let (pointer, len) = run(id); (pointer as usize as u64) << 32 | u64::from(len) }"
            )
        )]
    );

    let types = super::ty::Types::new(crate::index::Index::new(&root, &[])).unwrap();
    let can_equip = super::Method {
        owner: "Hooks",
        via: None,
        name: "can_equip".to_owned(),
        access: None,
        receiver: Some(false),
        params: Vec::new(),
        ret: super::ty::Ty::Prim("bool"),
    };
    let guest = super::calls::dispatch(
        &types,
        &[can_equip],
        &[("can_equip".to_owned(), "stack".to_owned())],
    )
    .0;
    let mut exports = Vec::new();
    blocks(
        &syn::parse_file(&guest).unwrap().items,
        &|name| name.starts_with("entry_"),
        &mut exports,
    );
    let packed = |id: u32| {
        block(&format!(
            "{{ let (p, n) = crate::wire::guest::run({id}); (p as usize as u64) << 32 | u64::from(n) }}"
        ))
    };
    assert_eq!(
        exports,
        [
            (
                "entry_prepare_can_equip".to_owned(),
                packed(super::prepared::PREPARE)
            ),
            (
                "entry_ask_can_equip".to_owned(),
                packed(super::names::export_id("Hooks", "can_equip"))
            ),
            (
                "entry_release_can_equip".to_owned(),
                packed(super::prepared::RELEASE)
            ),
        ]
    );
}
