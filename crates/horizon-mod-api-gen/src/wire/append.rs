//! A private collection editable only through public, untransformed enum pushes.
//! Keep its contract separate from public records: replies may append, never replace it.
use super::ty::{Case, Field, List, Ty, Types};
use crate::index::{Found, ItemId};
use std::collections::HashSet;
use syn::{Expr, Fields, Item, Stmt};

#[derive(Debug)]
pub(super) struct Append {
    pub(super) fields: Vec<Field>,
    pub(super) field: usize,
    pub(super) element: Ty,
    pub(super) equal: String,
}

pub(super) fn detect(
    types: &mut Types<'_>,
    id: ItemId,
    item: &Item,
) -> Result<Option<Append>, String> {
    let Item::Struct(record) = item else {
        return Ok(None);
    };
    if record.generics.lifetimes().count() == 0 {
        return Ok(None);
    }
    let Fields::Named(fields) = &record.fields else {
        return Ok(None);
    };
    let private = fields
        .named
        .iter()
        .enumerate()
        .filter(|(_, f)| !matches!(f.vis, syn::Visibility::Public(_)))
        .collect::<Vec<_>>();
    let [(field, native)] = private.as_slice() else {
        return Ok(None);
    };
    // The generated API codec must reach the field, without changing its visibility.
    let syn::Visibility::Restricted(vis) = &native.vis else {
        return Ok(None);
    };
    let module = &types.index.modules[id.module];
    if module.krate != "mod_api"
        || !(vis.path.is_ident("crate") || (vis.path.is_ident("super") && module.path.len() == 1))
    {
        return Ok(None);
    }
    let name = native.ident.as_ref().unwrap().to_string();
    let krate = module.krate;
    let field_ty = types.ty(id.module, &native.ty, &[])?;
    let Ty::Ref(inner, true) = &field_ty else {
        return Ok(None);
    };
    let Ty::List(element, List::Vec) = &**inner else {
        return Ok(None);
    };
    let element = (**element).clone();
    let Ty::Named(e) = &element else {
        return Ok(None);
    };
    // Inspect only the enum queued by this borrowed append-only argument. This
    // describes the checked push contract, not an owned value codec.
    let Some((&element_id, _)) = types.by_item.iter().find(|(_, def)| **def == *e) else {
        return Ok(None);
    };
    let Item::Enum(variants) = types.index.item(element_id).clone() else {
        return Ok(None);
    };
    let mut cases = Vec::new();
    for case in variants.variants {
        let Fields::Unnamed(fields) = case.fields else {
            return Ok(None);
        };
        let payload = fields
            .unnamed
            .iter()
            .map(|f| types.ty(element_id.module, &f.ty, &[]))
            .collect::<Result<Vec<_>, _>>()?;
        cases.push(Case {
            rust: case.ident.to_string(),
            payload,
        });
    }
    let path = &types.defs[*e].rust;
    let Some(equal) = equal_cases(path, &cases) else {
        return Ok(None);
    };
    let type_name = types.defs[*e].rust.rsplit("::").next().unwrap().to_string();
    let implementations = types
        .index
        .modules
        .iter()
        .enumerate()
        .filter(|(_, m)| m.krate == krate)
        .flat_map(|(module, m)| m.items.iter().cloned().map(move |item| (module, item)))
        .collect::<Vec<_>>();
    let mut seen = HashSet::new();
    for (module, item) in implementations {
        let Item::Impl(implementation) = item else {
            continue;
        };
        if !matches!(&*implementation.self_ty,syn::Type::Path(p) if p.path.segments.last().is_some_and(|s|s.ident==record.ident))
        {
            continue;
        }
        let syn::Type::Path(path) = &*implementation.self_ty else {
            continue;
        };
        let path = path
            .path
            .segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect::<Vec<_>>();
        if !matches!(types.index.resolve_path(module, &path), Ok(Found::Item(found)) if found == id)
        {
            continue;
        }
        for method in implementation.items {
            let syn::ImplItem::Fn(method) = method else {
                continue;
            };
            if !matches!(method.vis, syn::Visibility::Public(_)) {
                continue;
            }
            if !method
                .sig
                .receiver()
                .is_some_and(|r| matches!(&*r.ty,syn::Type::Reference(r) if r.mutability.is_some()))
            {
                continue;
            }
            let [Stmt::Expr(Expr::MethodCall(push), _)] = method.block.stmts.as_slice() else {
                return Ok(None);
            };
            if push.method != "push"
                || push.args.len() != 1
                || !matches!(&*push.receiver,Expr::Field(f) if matches!(&*f.base,Expr::Path(p) if p.path.is_ident("self")) && matches!(&f.member,syn::Member::Named(n) if n==name.as_str()))
            {
                return Ok(None);
            }
            let Expr::Call(value) = &push.args[0] else {
                return Ok(None);
            };
            let Expr::Path(path) = &*value.func else {
                return Ok(None);
            };
            let segments = path.path.segments.iter().collect::<Vec<_>>();
            if segments.len() != 2 || segments[0].ident != type_name {
                return Ok(None);
            }
            let Some(case) = cases.iter().find(|c| segments[1].ident == c.rust) else {
                return Ok(None);
            };
            let payload = &case.payload;
            if payload.len() != value.args.len() {
                return Ok(None);
            }
            for (argument, expected) in value.args.iter().zip(payload) {
                // Owning a borrowed string preserves its value exactly; no other
                // transformation is admitted by the append-only proof.
                let (argument, owned_string) = match argument {
                    Expr::Path(path) => (path, false),
                    Expr::MethodCall(call) if call.method == "to_owned" && call.args.is_empty() => {
                        let Expr::Path(path) = &*call.receiver else {
                            return Ok(None);
                        };
                        (path, true)
                    }
                    _ => return Ok(None),
                };
                let Some(parameter)=method.sig.inputs.iter().find_map(|p|match p {
                    syn::FnArg::Typed(p) if matches!(&*p.pat,syn::Pat::Ident(i) if argument.path.is_ident(&i.ident))=>Some(p),
                    _=>None,
                }) else {return Ok(None)};
                let actual = types.ty(module, &parameter.ty, &[])?;
                if if owned_string {
                    actual != Ty::Str(super::ty::Str::Borrowed)
                        || *expected != Ty::Str(super::ty::Str::Owned)
                } else {
                    actual != *expected
                } {
                    return Ok(None);
                }
            }
            seen.insert(case.rust.clone());
        }
    }
    if cases.iter().any(|case| !seen.contains(&case.rust)) {
        return Ok(None);
    }
    let mut described = Vec::new();
    for field in &fields.named {
        let name = field.ident.as_ref().unwrap().to_string();
        described.push(Field {
            rust: name,
            ty: types.ty(id.module, &field.ty, &[])?,
        });
    }
    Ok(Some(Append {
        fields: described,
        field: *field,
        element,
        equal,
    }))
}

// Compare transport identity, including NaN payloads, without serializing or allocating.
fn equal(ty: &Ty, a: &str, b: &str) -> Option<String> {
    Some(match ty {
        Ty::Prim("f32" | "f64") => format!("({a}).to_bits()==({b}).to_bits()"),
        Ty::Unit | Ty::Prim(..) | Ty::Wide(..) | Ty::Str(..) | Ty::Duration => {
            format!("({a})==({b})")
        }
        Ty::Ref(t, _) | Ty::Boxed(t) | Ty::Shared(t) => equal(t, a, b)?,
        Ty::Option(t) => format!(
            "match ({a},{b}) {{ (Some(a),Some(b))=>{},(None,None)=>true,_=>false }}",
            equal(t, "a", "b")?
        ),
        Ty::Result(ok, err) => format!(
            "match ({a},{b}) {{ (Ok(a),Ok(b))=>{},(Err(a),Err(b))=>{},_=>false }}",
            equal(ok, "a", "b")?,
            equal(err, "a", "b")?
        ),
        Ty::List(t, List::Vec | List::Slice) | Ty::Array(t, _) => format!(
            "({a}).len()==({b}).len() && ({a}).iter().zip(({b}).iter()).all(|(a,b)|{})",
            equal(t, "a", "b")?
        ),
        Ty::Tuple(v) => v
            .iter()
            .enumerate()
            .map(|(i, t)| equal(t, &format!("&({a}).{i}"), &format!("&({b}).{i}")))
            .collect::<Option<Vec<_>>>()?
            .join(" && "),
        _ => return None,
    })
}

/// Prefix comparisons preserve float bit patterns, without serializing or allocating.
fn equal_cases(path: &str, cases: &[Case]) -> Option<String> {
    let mut arms = Vec::new();
    for case in cases {
        let fields = &case.payload;
        let left = (0..fields.len())
            .map(|i| format!("a{i}"))
            .collect::<Vec<_>>()
            .join(",");
        let right = (0..fields.len())
            .map(|i| format!("b{i}"))
            .collect::<Vec<_>>()
            .join(",");
        let equals = fields
            .iter()
            .enumerate()
            .map(|(i, ty)| equal(ty, &format!("a{i}"), &format!("b{i}")))
            .collect::<Option<Vec<_>>>()?
            .join(" && ");
        arms.push(format!(
            "({path}::{}({left}),{path}::{}({right}))=>{equals}",
            case.rust, case.rust
        ));
    }
    Some(format!("match (a,b) {{ {},_=>false }}", arms.join(",")))
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "daily"]
    fn append_shape_requires_public_untransformed_typed_pushes() {
        let run = || {
            let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
            let generated = crate::sources(&root, crate::FillMode::Require).unwrap();
            let providers = crate::capability_providers::generate(&root, &[]).unwrap().0;
            let included = [
                ("mod_api_generated.rs", generated.hooks.as_str()),
                ("mod_api_generated_host.rs", generated.host.as_str()),
                ("mod_api_host_areas.rs", generated.areas.as_str()),
                ("capability_providers_generated.rs", providers.as_str()),
            ];
            let mut types = super::Types::new(crate::index::Index::new(&root, &included)).unwrap();
            let api = types.index.module("mod_api", &[]).unwrap();
            let super::Found::Item(id) = types.index.resolve(api, "ApplyEffectTickEvent").unwrap()
            else {
                panic!("missing event")
            };
            let item = types.index.item(id).clone();
            assert!(super::detect(&mut types, id, &item).unwrap().is_some());
            let (module, at) = types.index.modules.iter().enumerate().find_map(|(module, m)| {
                m.items.iter().position(|item| {
                    let syn::Item::Impl(i) = item else { return false; };
                    matches!(&*i.self_ty, syn::Type::Path(p)
                        if p.path.segments.last().is_some_and(|s| s.ident == "ApplyEffectTickEvent"))
                }).map(|at| (module, at))
            }).unwrap();
            let original = types.index.modules[module].items[at].clone();
            for mutation in 0..5 {
                let mut changed = original.clone();
                let syn::Item::Impl(i) = &mut changed else {
                    unreachable!()
                };
                let method = i
                    .items
                    .iter_mut()
                    .find_map(|item| match item {
                        syn::ImplItem::Fn(f)
                            if f.sig.ident == if mutation < 3 { "heal" } else { "hurt" } =>
                        {
                            Some(f)
                        }
                        _ => None,
                    })
                    .unwrap();
                match mutation {
                    0 => method.vis = syn::Visibility::Inherited,
                    1 => {
                        method.block = syn::parse_quote!({
                            self.changes.push(EffectTickChange::Heal(amount.max(0.0)));
                        })
                    }
                    2 => {
                        method.block = syn::parse_quote!({
                            self.changes.clear();
                        })
                    }
                    3 => {
                        method.block = syn::parse_quote!({
                            self.changes
                                .push(EffectTickChange::Hurt(damage_type.to_uppercase(), amount));
                        })
                    }
                    4 => {
                        method.block = syn::parse_quote!({
                            self.changes.reverse();
                            self.changes
                                .push(EffectTickChange::Hurt(damage_type.to_owned(), amount));
                        })
                    }
                    _ => unreachable!(),
                }
                types.index.modules[module].items[at] = changed;
                assert!(
                    super::detect(&mut types, id, &item).unwrap().is_none(),
                    "unproven append shape {mutation}"
                );
            }
        };
        std::thread::Builder::new()
            .stack_size(64 << 20)
            .spawn(run)
            .unwrap()
            .join()
            .unwrap();
    }
}
