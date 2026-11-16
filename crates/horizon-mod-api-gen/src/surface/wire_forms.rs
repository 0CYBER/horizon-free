//! How each API type crosses the WebAssembly boundary, read from the code that makes it cross:
//! the `Wire` derive (with `#[wire(via = T)]`), a hand-written `impl Wire` (its
//! `layout_fields!` list, or a `layout` that returns another type's), including the impls the
//! API's own macros write once expanded.
use crate::index::{Found, Index, ItemId};
use std::collections::HashMap;
use syn::{Item, Type};

/// How one type crosses.
#[derive(Clone)]
pub(super) enum WireForm {
    /// `#[derive(Wire)]`: its Rust fields.
    Derive,
    /// As another type, written in `module`.
    Via(Type, usize),
    /// A hand-written impl's declared fields, each type written in `module`.
    Fields(Vec<(String, Type)>, usize),
    /// A hand-written impl that declares no layout of its own.
    Manual,
}

impl WireForm {
    /// Its name in `wire_form`, `via:` followed by the normalised type.
    pub(super) fn name(&self, via: impl FnOnce(&Type, usize) -> String) -> String {
        match self {
            Self::Derive => "derive".into(),
            Self::Via(ty, module) => format!("via:{}", via(ty, *module)),
            Self::Fields(..) => "fields".into(),
            Self::Manual => "manual".into(),
        }
    }
}

/// `ty` without the invisible groups a macro's substitution leaves around it.
pub(super) fn ungroup(ty: &Type) -> &Type {
    match ty {
        Type::Group(group) => ungroup(&group.elem),
        Type::Paren(paren) => ungroup(&paren.elem),
        ty => ty,
    }
}

fn is_wire(path: &syn::Path) -> bool {
    path.segments.last().is_some_and(|s| s.ident == "Wire")
}

/// `#[wire(via = T)]` on a deriving type.
pub(super) fn via_attribute(attrs: &[syn::Attribute]) -> Option<Type> {
    let mut found = None;
    for attr in attrs.iter().filter(|a| a.path().is_ident("wire")) {
        let _ = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("via") {
                found = Some(meta.value()?.parse::<Type>()?);
            } else if meta.input.peek(syn::Token![=]) {
                let _: syn::Expr = meta.value()?.parse()?;
            }
            Ok(())
        });
    }
    found
}

/// The (name, type) pairs of `layout_fields!("a" => T, …)`.
fn layout_fields(tokens: proc_macro2::TokenStream) -> Option<Vec<(String, Type)>> {
    struct Pair(String, Type);
    impl syn::parse::Parse for Pair {
        fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
            let name: syn::LitStr = input.parse()?;
            input.parse::<syn::Token![=>]>()?;
            Ok(Self(name.value(), input.parse()?))
        }
    }
    let pairs = syn::parse::Parser::parse2(
        syn::punctuated::Punctuated::<Pair, syn::Token![,]>::parse_terminated,
        tokens,
    )
    .ok()?;
    Some(pairs.into_iter().map(|Pair(n, t)| (n, t)).collect())
}

/// What a hand-written impl's items declare.
fn impl_form(items: &[syn::ImplItem], module: usize) -> WireForm {
    for item in items {
        if let syn::ImplItem::Macro(mac) = item
            && mac
                .mac
                .path
                .segments
                .last()
                .is_some_and(|s| s.ident == "layout_fields")
        {
            return match layout_fields(mac.mac.tokens.clone()) {
                Some(fields) => WireForm::Fields(fields, module),
                None => WireForm::Manual,
            };
        }
    }
    for item in items {
        let syn::ImplItem::Fn(function) = item else {
            continue;
        };
        if function.sig.ident != "layout" {
            continue;
        }
        // `<T>::layout(visiting)` or `<T as Wire>::layout(visiting)`: it crosses as `T`.
        if let [syn::Stmt::Expr(syn::Expr::Call(call), None)] = &function.block.stmts[..]
            && let syn::Expr::Path(callee) = &*call.func
            && callee
                .path
                .segments
                .last()
                .is_some_and(|s| s.ident == "layout")
        {
            if let Some(qself) = &callee.qself {
                return WireForm::Via((*qself.ty).clone(), module);
            }
            let mut path = callee.path.clone();
            path.segments.pop();
            path.segments.pop_punct();
            if !path.segments.is_empty() {
                return WireForm::Via(Type::Path(syn::TypePath { qself: None, path }), module);
            }
        }
        return WireForm::Manual;
    }
    WireForm::Manual
}

/// Every API type's wire form, from the modules given, keyed by the type's item.
pub(super) fn wire_forms(index: &mut Index<'_>, modules: &[usize]) -> HashMap<ItemId, WireForm> {
    let mut forms = HashMap::new();
    for &module in modules {
        let items = index.modules[module].items.clone();
        for item in items {
            match item {
                Item::Impl(block)
                    if block
                        .trait_
                        .as_ref()
                        .is_some_and(|(_, path, _)| is_wire(path)) =>
                {
                    let Type::Path(self_ty) = ungroup(&block.self_ty) else {
                        continue;
                    };
                    let segments: Vec<String> = self_ty
                        .path
                        .segments
                        .iter()
                        .map(|s| s.ident.to_string())
                        .collect();
                    if let Ok(Found::Item(id)) = index.resolve_path(module, &segments) {
                        forms.insert(id, impl_form(&block.items, module));
                    }
                }
                _ => {}
            }
        }
    }
    forms
}

/// The form of `item` given its attributes and the impls found: a derive, or what an impl says.
pub(super) fn form_of(
    id: ItemId,
    attrs: &[syn::Attribute],
    derives: &[String],
    impls: &HashMap<ItemId, WireForm>,
) -> Option<WireForm> {
    if derives.iter().any(|d| d == "Wire") {
        return Some(match via_attribute(attrs) {
            Some(ty) => WireForm::Via(ty, id.module),
            None => WireForm::Derive,
        });
    }
    impls.get(&id).cloned()
}
