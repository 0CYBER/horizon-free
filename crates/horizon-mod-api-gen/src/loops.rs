//! Published loops (`[[native.loop]]`): the game's `#[horizon_loop]` entry functions, listed by the
//! staging pass into the prebuilt game's `loops.json`, which the launcher checks a mod's rows
//! against and builds each loop's copy from. A new loop is one attribute on its entry function.
use quote::ToTokens;
use serde::{Deserialize, Serialize};
use std::path::Path;
use syn::visit::Visit;

/// The file beside `points.json` in the prebuilt game.
pub const FILE: &str = "loops.json";

/// One published loop.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishedLoop {
    /// What a `[[native.loop]]` row's `loop` names (`block_entities.tick`).
    pub name: String,
    /// The entry function, as a `[[native.mixin]]` row's `target` writes it.
    pub entry: String,
    /// The generic loop method the entry calls with the empty slot (`tick_block_entities_with`).
    pub generic: String,
    /// The slot trait, as the loop method's bound writes it.
    pub slot: String,
    /// `server` or `client`: which game runs it.
    pub side: String,
}

impl PublishedLoop {
    /// The generic loop function, as a `[[native.mixin]]` row's `target` writes it.
    pub fn generic_target(&self) -> String {
        let (owner, _) = self.entry.rsplit_once("::").expect("an entry is a path");
        format!("{owner}::{}", self.generic)
    }
}

/// The game's published loops, from the source at `root`, by name.
pub fn collect(root: &Path) -> Result<Vec<PublishedLoop>, String> {
    let src = root.join("crates/horizon-client/src");
    let mut loops = Vec::new();
    for file in crate::source::game_files(&src, false)? {
        let text = std::fs::read_to_string(&file.path)
            .map_err(|e| format!("{}: {e}", file.path.display()))?;
        if !text.contains("horizon_loop") {
            continue;
        }
        let parsed = syn::parse_file(&text).map_err(|e| format!("{}: {e}", file.path.display()))?;
        for item in &parsed.items {
            let syn::Item::Impl(block) = item else {
                continue;
            };
            for member in &block.items {
                let syn::ImplItem::Fn(function) = member else {
                    continue;
                };
                let Some(marker) = crate::source::marker(&function.attrs, "horizon_loop") else {
                    continue;
                };
                let at = format!("{}::{}", file.module, function.sig.ident);
                loops.push(
                    published(&file.module, block, function, marker)
                        .map_err(|why| format!("#[horizon_loop] on {at}: {why}"))?,
                );
            }
        }
    }
    loops.sort_by(|a, b| a.name.cmp(&b.name));
    if let Some(pair) = loops.windows(2).find(|pair| pair[0].name == pair[1].name) {
        return Err(format!("two functions publish the loop {}", pair[0].name));
    }
    Ok(loops)
}

fn published(
    module: &str,
    block: &syn::ItemImpl,
    entry: &syn::ImplItemFn,
    marker: &syn::Attribute,
) -> Result<PublishedLoop, String> {
    let pairs = marker
        .parse_args_with(
            syn::punctuated::Punctuated::<syn::MetaNameValue, syn::Token![,]>::parse_terminated,
        )
        .map_err(|e| e.to_string())?;
    let value = |key: &str| {
        pairs.iter().find_map(|pair| match &pair.value {
            syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(text),
                ..
            }) if pair.path.is_ident(key) => Some(text.value()),
            _ => None,
        })
    };
    let name = value("name").ok_or("no name")?;
    let side = value("side").ok_or("no side")?;
    if block.trait_.is_some() {
        return Err("an entry is an inherent method".into());
    }
    // The body is `self.<generic>(&mut ())`, the call a mod build's copy replaces.
    let call = match entry.block.stmts.as_slice() {
        [syn::Stmt::Expr(syn::Expr::MethodCall(call), None)] => call,
        _ => return Err("its body must be one call, `self.<loop>(&mut ())`".into()),
    };
    let empty_slot = matches!(call.args.iter().collect::<Vec<_>>().as_slice(),
        [syn::Expr::Reference(r)] if r.mutability.is_some()
            && matches!(&*r.expr, syn::Expr::Tuple(t) if t.elems.is_empty()));
    if !matches!(&*call.receiver, syn::Expr::Path(p) if p.path.is_ident("self")) || !empty_slot {
        return Err("its body must be one call, `self.<loop>(&mut ())`".into());
    }
    let generic = call.method.to_string();
    let method = block
        .items
        .iter()
        .find_map(|member| match member {
            syn::ImplItem::Fn(f) if f.sig.ident == call.method => Some(f),
            _ => None,
        })
        .ok_or_else(|| format!("no method `{generic}` in its `impl`"))?;
    let slot = match method
        .sig
        .generics
        .type_params()
        .collect::<Vec<_>>()
        .as_slice()
    {
        [param] if param.bounds.len() == 1 => {
            param.bounds.to_token_stream().to_string().replace(' ', "")
        }
        _ => return Err(format!("`{generic}` must be generic over its slot alone")),
    };
    // A loop body that names `self.<entry>` again would run the copy inside itself.
    struct Calls<'a>(&'a syn::Ident, bool);
    impl<'ast> Visit<'ast> for Calls<'_> {
        fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
            self.1 |= call.method == *self.0;
            syn::visit::visit_expr_method_call(self, call);
        }
    }
    let mut calls = Calls(&entry.sig.ident, false);
    calls.visit_block(&method.block);
    if calls.1 {
        return Err(format!("`{generic}` calls the entry again"));
    }
    let owner = block.self_ty.to_token_stream().to_string().replace(' ', "");
    Ok(PublishedLoop {
        name,
        entry: format!("{module}::{owner}::{}", entry.sig.ident),
        generic,
        slot,
        side,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(body: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("crates/horizon-client/src");
        std::fs::create_dir_all(src.join("game")).unwrap();
        std::fs::write(src.join("game.rs"), "mod ticks;\n").unwrap();
        std::fs::write(src.join("game/ticks.rs"), body).unwrap();
        dir
    }

    const LOOP: &str = r#"
impl Game<'_> {
    #[horizon_mod_api_macros::horizon_loop(name = "things.tick", side = "server")]
    pub(super) fn tick_things(&mut self) -> Result<()> {
        self.tick_things_with(&mut ())
    }
    pub fn tick_things_with<S: ThingSlot>(&mut self, slot: &mut S) -> Result<()> {
        Ok(())
    }
}
"#;

    #[test]
    fn a_marked_entry_is_listed() {
        let dir = game(LOOP);
        assert_eq!(
            collect(dir.path()).unwrap(),
            [PublishedLoop {
                name: "things.tick".into(),
                entry: "horizon_client::game::ticks::Game<'_>::tick_things".into(),
                generic: "tick_things_with".into(),
                slot: "ThingSlot".into(),
                side: "server".into(),
            }]
        );
    }

    #[test]
    fn a_marked_entry_of_another_shape_is_refused_by_name() {
        for (from, to, why) in [
            (
                "self.tick_things_with(&mut ())",
                "self.tick_things_with(&mut ());\n        Ok(())",
                "one call",
            ),
            ("(&mut ())", "(&mut 1)", "one call"),
            (
                "<S: ThingSlot>",
                "<S: ThingSlot, T>",
                "generic over its slot alone",
            ),
            (
                "Ok(())\n    }\n}",
                "self.tick_things()\n    }\n}",
                "calls the entry again",
            ),
            (", side = \"server\"", "", "no side"),
        ] {
            let dir = game(&LOOP.replacen(from, to, 1));
            let error = collect(dir.path()).unwrap_err();
            assert!(
                error.contains("#[horizon_loop] on horizon_client::game::ticks::tick_things"),
                "{error}"
            );
            assert!(error.contains(why), "{error}");
        }
    }
}
