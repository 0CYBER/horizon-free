//! The guest's side of the calls: `Guest`, the host as a mod reaches it, each imported method
//! encoding its arguments, crossing once and decoding the reply. Methods with a `&dyn` frame
//! object, a handler or a borrowed return are listed apart: they need the call frame.
use super::ty::{List, Str, Ty, Types};
use super::{Method, exports};
use std::fmt::Write;

/// `ty` as Rust names it in a trait method's signature.
pub(super) fn rust_ty(types: &Types<'_>, ty: &Ty, ret: bool) -> String {
    match ty {
        Ty::Unit => "()".into(),
        Ty::Prim(rust) | Ty::Wide(rust) => (*rust).into(),
        Ty::Duration => "std::time::Duration".into(),
        Ty::Handler(name, false) => format!("std::sync::Arc<dyn {}>", trait_path(name)),
        Ty::Handler(name, true) => format!("Box<dyn {}>", trait_path(name)),
        Ty::Str(Str::Owned) => "String".into(),
        Ty::Str(Str::Borrowed) => "&str".into(),
        Ty::Str(Str::Static) => "&'static str".into(),
        Ty::Str(Str::Boxed) => "Box<str>".into(),
        Ty::Str(Str::Shared) => "std::sync::Arc<str>".into(),
        Ty::Str(Str::Cow) => "std::borrow::Cow<'static, str>".into(),
        Ty::List(item, List::Set(set)) => format!("{set}<{}>", rust_ty(types, item, ret)),
        Ty::List(item, List::Vec) => format!("Vec<{}>", rust_ty(types, item, ret)),
        Ty::List(item, List::Slice) => format!("&[{}]", rust_ty(types, item, ret)),
        Ty::List(item, List::Boxed) => format!("Box<[{}]>", rust_ty(types, item, ret)),
        Ty::List(item, List::Shared) => format!("std::sync::Arc<[{}]>", rust_ty(types, item, ret)),
        Ty::List(item, List::Map(map)) => match &**item {
            Ty::Tuple(pair) => format!(
                "{map}<{}, {}>",
                rust_ty(types, &pair[0], ret),
                rust_ty(types, &pair[1], ret)
            ),
            _ => unreachable!("a map's items are pairs"),
        },
        Ty::Array(item, n) => format!("[{}; {n}]", rust_ty(types, item, ret)),
        Ty::Option(item) => format!("Option<{}>", rust_ty(types, item, ret)),
        Ty::Result(a, b) => format!(
            "Result<{}, {}>",
            rust_ty(types, a, ret),
            rust_ty(types, b, ret)
        ),
        Ty::Tuple(items) => format!(
            "({},)",
            items
                .iter()
                .map(|i| rust_ty(types, i, ret))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Ty::Ref(inner, true) => format!("&mut {}", rust_ty(types, inner, ret)),
        Ty::Ref(inner, false) => format!("&{}", rust_ty(types, inner, ret)),
        Ty::Boxed(inner) => format!("Box<{}>", rust_ty(types, inner, ret)),
        Ty::Shared(inner) => format!("std::sync::Arc<{}>", rust_ty(types, inner, ret)),
        Ty::Named(def) => types.defs[*def].rust.clone(),
        Ty::Dyn(name, mutable) => {
            // A returned shared one is the guest's unit, for any borrow; a mutable one, being
            // invariant, keeps the borrow's own lifetime.
            let lifetime = if ret && !mutable { "'static " } else { "" };
            let m = if *mutable { "mut " } else { "" };
            format!("&{lifetime}{m}dyn {}", trait_path(name))
        }
    }
}

pub(super) fn trait_path(owner: &str) -> String {
    if owner == super::ERASED {
        return "std::any::Any + Send + Sync".into();
    }
    // Every name in it is `mod_api`'s, but the prelude's (`ResourceHandler<ItemResource>`,
    // `EntityCapabilityProvider<dyn EnergyHandler, Option<Direction>>`).
    struct Qualify;
    impl syn::visit_mut::VisitMut for Qualify {
        fn visit_path_mut(&mut self, path: &mut syn::Path) {
            syn::visit_mut::visit_path_mut(self, path);
            let outside = ["Option", "Vec", "Box", "String", "Result", "crate", "std"];
            if path.leading_colon.is_none()
                && !outside.contains(&path.segments[0].ident.to_string().as_str())
            {
                path.segments.insert(0, syn::parse_quote!(crate));
            }
            if path.segments.len() == 4
                && path.segments[0].ident == "crate"
                && path.segments[1].ident == "host"
            {
                let name = path.segments.last().unwrap().clone();
                *path = syn::parse_quote!(crate::prelude::#name);
            } else if path.segments.len() == 3
                && path.segments[0].ident == "crate"
                && path.segments[1].ident == "parts"
            {
                let name = path.segments.last().unwrap().clone();
                *path = syn::parse_quote!(crate::#name);
            }
        }
    }
    let owner = super::imports()
        .iter()
        .copied()
        .find(|candidate| candidate.rsplit("::").next() == Some(owner))
        .unwrap_or(owner);
    let mut path: syn::Path = syn::parse_str(owner).expect("an owner is a path");
    syn::visit_mut::VisitMut::visit_path_mut(&mut Qualify, &mut path);
    super::ty::written(&path)
}

/// A method's Rust signature, its arguments named `p0…`.
pub(super) fn rust_sig(types: &Types<'_>, method: &Method) -> String {
    let mut params = Vec::new();
    match method.receiver {
        Some(true) => params.push("&mut self".to_string()),
        Some(false) => params.push("&self".to_string()),
        None => {}
    }
    for (i, (_, ty)) in method.params.iter().enumerate() {
        params.push(format!("p{i}: {}", rust_ty(types, ty, false)));
    }
    let ret = match &method.ret {
        Ty::Unit => String::new(),
        ty => format!(" -> {}", rust_ty(types, ty, true)),
    };
    format!("fn {}({}){ret}", method.name, params.join(", "))
}

/// The ids of every imported method, in the order they are generated: the table both sides
/// share, so an id is never sent as a name.
pub(super) fn imports(methods: &[Method]) -> Vec<&Method> {
    methods
        .iter()
        .filter(|m| !exports().contains(&m.owner))
        .collect()
}

pub(super) fn held_methods(methods: &[Method]) -> Vec<&Method> {
    methods.iter().filter(|m| super::held(m.owner)).collect()
}

pub(super) fn has_frame(ty: &Ty) -> bool {
    match ty {
        Ty::Dyn(..) => true,
        Ty::List(t, _)
        | Ty::Option(t)
        | Ty::Array(t, _)
        | Ty::Ref(t, _)
        | Ty::Boxed(t)
        | Ty::Shared(t) => has_frame(t),
        Ty::Result(a, b) => has_frame(a) || has_frame(b),
        Ty::Tuple(v) => v.iter().any(has_frame),
        _ => false,
    }
}

fn borrowed_ret(ty: &Ty) -> bool {
    match ty {
        Ty::Ref(..) | Ty::Str(Str::Borrowed) | Ty::List(_, List::Slice) => true,
        Ty::Option(t) | Ty::Boxed(t) | Ty::Shared(t) | Ty::Array(t, _) | Ty::List(t, _) => {
            borrowed_ret(t)
        }
        Ty::Result(a, b) => borrowed_ret(a) || borrowed_ret(b),
        Ty::Tuple(v) => v.iter().any(borrowed_ret),
        _ => false,
    }
}

/// Whether `ty` holds a `&'static str`, which the guest takes from the game's interned names.
fn has_static(ty: &Ty) -> bool {
    match ty {
        Ty::Str(Str::Static) => true,
        Ty::Option(t) | Ty::Boxed(t) | Ty::Shared(t) | Ty::Array(t, _) | Ty::List(t, _) => {
            has_static(t)
        }
        Ty::Result(a, b) => has_static(a) || has_static(b),
        Ty::Tuple(v) => v.iter().any(has_static),
        _ => false,
    }
}

/// `e`, the owned value an import answered with, as the borrowed form its method returns: kept
/// until the call in progress returns, as a borrow of the game's data is for the same time.
fn lend_ret(ty: &Ty, e: &str) -> Option<String> {
    let kept = "unsafe { crate::wire::guest::kept";
    let needs = |t: &Ty| borrowed_ret(t) || has_static(t);
    Some(match ty {
        Ty::Str(Str::Static) => format!(
            "crate::wire::intern(&{e}).expect(\"the host named a string longer than the boundary allows\")"
        ),
        Ty::Str(Str::Borrowed) => format!("{kept}({e}) }}.as_str()"),
        Ty::List(_, List::Slice) => format!("{kept}({e}) }}.as_slice()"),
        Ty::Ref(..) => format!("{kept}({e}) }}"),
        Ty::Option(t) if needs(t) => format!("{e}.map(|x| {})", lend_ret(t, "x")?),
        Ty::Tuple(items) if needs(ty) => format!(
            "({},)",
            items
                .iter()
                .enumerate()
                .map(|(i, t)| lend_ret(t, &format!("({e}).{i}")))
                .collect::<Option<Vec<_>>>()?
                .join(",")
        ),
        Ty::Array(t, _) if needs(t) => format!("{e}.map(|x| {})", lend_ret(t, "x")?),
        Ty::Result(a, b) if needs(a) || needs(b) => {
            format!(
                "{e}.map(|x| {}).map_err(|x| {})",
                lend_ret(a, "x")?,
                lend_ret(b, "x")?
            )
        }
        ty if !needs(ty) => e.to_string(),
        _ => return None,
    })
}

/// `Guest` and its impls for every plain import, and the table of ids.
pub(super) fn guest(
    types: &Types<'_>,
    methods: &[Method],
    dropped: &[Method],
    excused: &[(&'static str, String)],
    parts: &[(&'static str, String)],
) -> String {
    let mut out = String::from(
        "// @generated by horizon-mod-api-gen; do not edit.\n// no wire: guest transport service, not a boundary value.\npub struct Guest;\n// no wire: bounded selector for a borrowed file capability.\nstruct FileGuest(u8);\n",
    );
    let all = imports(methods);
    // Every imported trait, a method or not (one with none still holds up its subtraits), and
    // each area trait that is a part of one.
    let mut owners: Vec<&str> = super::imports().to_vec();
    owners.extend(parts.iter().map(|(_, via)| via.as_str()));
    for owner in owners {
        let proxy = if owner == "ConfigFiles" {
            "FileGuest"
        } else {
            "Guest"
        };
        writeln!(out, "impl {} for {proxy} {{", trait_path(owner)).unwrap();
        out.push_str(match owner {
            "Host" => "fn backend(&self) -> Option<crate::BackendRef<'_>> { Some(crate::BackendRef::of(self)) }\nfn capability_providers(&self) -> &'static dyn crate::CapabilityProviders { &Guest }\n",
            "ServerHost" => "fn backend_mut(&mut self) -> Option<crate::BackendMutRef<'_>> { Some(crate::BackendMutRef::of(self)) }\n",
            "ClientHost" => "fn client_backend_mut(&mut self) -> Option<crate::ClientBackendMutRef<'_>> { Some(crate::ClientBackendMutRef::of(self)) }\n",
            _ => "",
        });
        for (id, m) in all
            .iter()
            .enumerate()
            .filter(|(_, m)| m.via.as_deref().unwrap_or(m.owner) == owner)
        {
            let prefix = if owner == "ConfigFiles" {
                "w.put(&self.0);"
            } else {
                ""
            };
            writeln!(out, "{}", import_body(types, m, id, prefix)).unwrap();
        }
        for m in dropped.iter().filter(|m| {
            !exports().contains(&m.owner) && m.via.as_deref().unwrap_or(m.owner) == owner
        }) {
            // Frame accessors are local proxies, not absent capabilities. Their service
            // methods cross the wire and the engine owns the phase check there.
            if super::frame_accessor(m) {
                writeln!(out, "{}", import_body(types, m, 0, "")).unwrap();
                continue;
            }
            // What the API keeps off wasm exists only on a native build of the guest.
            if excused.iter().any(|(o, n)| *o == m.owner && *n == m.name) {
                out.push_str("#[cfg(not(target_arch = \"wasm32\"))]\n");
            }
            writeln!(
                out,
                "{} {{ crate::wire::guest::unsupported(\"{}::{}\") }}",
                rust_sig(types, m),
                m.owner,
                m.name
            )
            .unwrap();
        }
        out.push_str("}\n");
    }
    for owner in super::exports().iter().filter(|owner| super::held(owner)) {
        writeln!(
            out,
            "impl {} for crate::wire::guest::Held {{",
            trait_path(owner)
        )
        .unwrap();
        for (id, m) in held_methods(methods)
            .into_iter()
            .enumerate()
            .filter(|(_, m)| m.owner == *owner)
        {
            out.push_str(&import_body(types, m, all.len() + id, "w.put(&self.0);"));
        }
        out.push_str("}\n");
    }
    out.push_str(&resource_codec(types));
    out
}

fn resource_codec(types: &Types<'_>) -> String {
    let mut store = String::from(
        r#"if value.is::<std::sync::Arc<dyn std::any::Any + Send + Sync>>() {
        let value=*value.downcast::<std::sync::Arc<dyn std::any::Any + Send + Sync>>().map_err(|_|crate::wire::WireError::Invalid("wrong erased handler"))?;
        if value.is::<crate::wire::Unsupported>() {return Ok(crate::wire::Unsupported::ID)}
        return Ok(crate::wire::guest::add_handler(vec![crate::wire::guest::view::<dyn std::any::Any + Send + Sync>(value)]));
    }"#,
    );
    let mut load = String::from(
        r#"if view==std::any::TypeId::of::<std::sync::Arc<dyn std::any::Any + Send + Sync>>() {
        let value=if id==crate::wire::Unsupported::ID {crate::wire::Unsupported::shared()} else {crate::wire::guest::handler::<dyn std::any::Any + Send + Sync>(id)?};
        return Ok(Box::new(value));
    }"#,
    );
    for (owner, views) in &types.views {
        let path = trait_path(owner);
        let views = views
            .iter()
            .map(|v| {
                format!(
                    "crate::wire::guest::view::<dyn {}>(value.clone())",
                    trait_path(v)
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        writeln!(store,"if value.is::<std::sync::Arc<dyn {path}>>() {{ let value=*value.downcast::<std::sync::Arc<dyn {path}>>().map_err(|_|crate::wire::WireError::Invalid(\"wrong handler\"))?; return Ok(crate::wire::guest::add_handler(vec![{views}])); }}").unwrap();
        if super::held(owner) {
            for container in ["Box", "std::sync::Arc"] {
                writeln!(load,"if view == std::any::TypeId::of::<{container}<dyn {path}>>() {{ return Ok(Box::new({container}::new(crate::wire::guest::Held(id)) as {container}<dyn {path}>)); }}").unwrap();
            }
        }
    }
    format!(
        "struct GuestCodec; impl crate::wire::ResourceCodec for GuestCodec {{ fn store(&self,value:Box<dyn std::any::Any>)->Result<crate::wire::Id,crate::wire::WireError> {{ {store} Err(crate::wire::WireError::Invalid(\"unknown handler type\")) }} fn load(&self,id:crate::wire::Id,view:std::any::TypeId)->Result<Box<dyn std::any::Any>,crate::wire::WireError> {{ {load} Err(crate::wire::WireError::Invalid(\"unknown held view\")) }} }} thread_local! {{ static CODEC:std::rc::Rc<dyn crate::wire::ResourceCodec> = std::rc::Rc::new(GuestCodec); }} pub fn resource_scope()->crate::wire::ResourceScope {{ CODEC.with(|codec| crate::wire::ResourceScope::new(codec.clone())) }}\n"
    )
}

/// Whether `ty` names a type that borrows (`CubeFace<'_>`), which has no owned wire form.
pub(super) fn lifetime_def(types: &Types<'_>, ty: &Ty) -> bool {
    match ty {
        Ty::Named(d) => types.defs[*d].lifetimes != 0,
        Ty::Ref(t, _)
        | Ty::Option(t)
        | Ty::Array(t, _)
        | Ty::Boxed(t)
        | Ty::Shared(t)
        | Ty::List(t, _) => lifetime_def(types, t),
        Ty::Result(a, b) => lifetime_def(types, a) || lifetime_def(types, b),
        Ty::Tuple(v) => v.iter().any(|t| lifetime_def(types, t)),
        _ => false,
    }
}

/// A borrow inside a list, tuple or `Option` the generated code can't write as one value
/// (`&[(&str, &str)]`).
pub(super) fn has_ref_in_data(ty: &Ty) -> bool {
    fn inner(ty: &Ty) -> bool {
        match ty {
            Ty::Ref(..) | Ty::Str(Str::Borrowed) | Ty::List(_, List::Slice) => true,
            Ty::List(t, _) | Ty::Option(t) | Ty::Array(t, _) | Ty::Boxed(t) | Ty::Shared(t) => {
                inner(t)
            }
            Ty::Tuple(v) => v.iter().any(inner),
            Ty::Result(a, b) => inner(a) || inner(b),
            _ => false,
        }
    }
    match ty {
        Ty::Ref(t, _) => inner(t),
        Ty::List(t, List::Slice) => !matches!(**t, Ty::Str(Str::Borrowed)) && inner(t),
        Ty::Option(t)
            if matches!(
                **t,
                Ty::Ref(..) | Ty::Str(Str::Borrowed) | Ty::List(_, List::Slice)
            ) =>
        {
            has_ref_in_data(t)
        }
        Ty::Str(Str::Borrowed) => false,
        ty => inner(ty),
    }
}

/// Mutable scalar inputs cross by value and return in the checked reply writebacks.
pub(super) fn mutable_scalar(ty: &Ty) -> bool {
    matches!(ty,Ty::Ref(t,true) if matches!(**t,Ty::Prim(..)|Ty::Wide(..)|Ty::Unit|Ty::Duration))
}

/// The mod's body of import `id`: its arguments encoded, one crossing, the answer decoded. A
/// frame object it returns is a local proxy; file capabilities retain their root selector.
/// A handler it passes is kept under every trait it answers as and crosses as its id.
fn import_body(types: &Types<'_>, m: &Method, id: usize, prefix: &str) -> String {
    let normal = uncached_import_body(types, m, id, prefix);
    let cache = match m.name.as_str() {
        "entity_kind" => Some("cached_equip_kind(p0)"),
        "entity_position" => Some("cached_equip_position(p0)"),
        "living_health" => Some("cached_equip_health(p0)"),
        "living_max_health" => Some("cached_equip_max_health(p0)"),
        "living_alive" => Some("cached_equip_alive(p0)"),
        "living" => Some("cached_equip_living(p0)"),
        "living_equipment" => Some("cached_equip_equipment(p0,p1)"),
        "living_hand_item" => Some("cached_equip_hand_item(p0,p1)"),
        _ => None,
    };
    if let Some(cache) = cache {
        normal.replacen(
            " {",
            &format!(" {{ if let Some(value)=crate::__private::{cache} {{ crate::__private::check_access({:?}, crate::__private::LoadAccess::WorldOnly); return value; }}", m.name),
            1,
        )
    } else {
        normal
    }
}
fn uncached_import_body(types: &Types<'_>, m: &Method, id: usize, prefix: &str) -> String {
    let sig = rust_sig(types, m);
    let guest = "crate::wire::guest";
    if let Some((root, _, _)) = super::FILE_ROOTS
        .iter()
        .find(|(_, owner, name)| *owner == m.owner && *name == m.name)
    {
        return format!("{sig} {{ &FileGuest({root}) }}");
    }
    if let Ty::Dyn(_, mutable) = &m.ret {
        let body = if *mutable {
            "::std::boxed::Box::leak(::std::boxed::Box::new(Guest))"
        } else {
            "&Guest"
        };
        return format!("{sig} {{ {body} }}");
    }
    if let Some((target, mutable, error)) = super::fallible_frame_accessor(m) {
        let error = rust_ty(types, error, false);
        let value = if mutable {
            format!(
                "::std::boxed::Box::leak(::std::boxed::Box::new(Guest)) as &mut dyn {}",
                trait_path(target)
            )
        } else {
            format!("&Guest as &dyn {}", trait_path(target))
        };
        return format!(
            "{sig} {{ let w = {guest}::begin(); let r = {guest}::host({id}u32, w); let answer: Result<(), {error}> = r.take().expect(\"the host answered with an invalid frame-access result\"); answer.map(|()| {value}) }}"
        );
    }
    let mut put = String::new();
    let mut back = Vec::new();
    for (i, (_, ty)) in m.params.iter().enumerate() {
        if mutable_scalar(ty) {
            back.push((i, held(types, ty).unwrap()));
        }
        match ty {
            Ty::Dyn(..) => {}
            Ty::Handler(..) => put.push_str(&format!("w.put_owned(p{i});")),
            ty if has_frame(ty) || (matches!(ty, Ty::Ref(_, true)) && !mutable_scalar(ty)) => {
                return format!(
                    "{sig} {{ {guest}::unsupported(\"{}::{}\") }}",
                    m.owner, m.name
                );
            }
            ty if (has_ref_in_data(ty) && lend(types, ty, "p").is_none())
                || (lifetime_def(types, ty) && snapshot(types, ty, "p").is_none()) =>
            {
                return format!(
                    "{sig} {{ {guest}::unsupported(\"{}::{}\") }}",
                    m.owner, m.name
                );
            }
            ty if lifetime_def(types, ty) => {
                put.push_str(&snapshot(types, ty, &format!("p{i}")).unwrap());
            }
            Ty::Ref(..) | Ty::Str(Str::Borrowed) | Ty::List(_, List::Slice) => {
                put.push_str(&format!("w.put(p{i});"))
            }
            Ty::Option(inner)
                if matches!(
                    **inner,
                    Ty::Ref(..) | Ty::Str(Str::Borrowed) | Ty::List(_, List::Slice)
                ) =>
            {
                put.push_str(&format!("w.put_opt(p{i});"))
            }
            _ => put.push_str(&format!("w.put_owned(p{i});")),
        }
    }
    let lent =
        held_in(types, &m.ret, true).and_then(|owned| Some((owned, lend_ret(&m.ret, "answer")?)));
    let (Some((owned, lent)), false) = (lent, has_frame(&m.ret)) else {
        return format!(
            "{sig} {{ {guest}::unsupported(\"{}::{}\") }}",
            m.owner, m.name
        );
    };
    let static_string = m.ret == Ty::Str(Str::Static);
    let (owned, lent, take_method) = if static_string {
        (
            "&'static str".to_string(),
            "answer".to_string(),
            "take_static",
        )
    } else {
        (owned, lent, "take")
    };
    let prefix = if static_string {
        format!("{prefix} w.put(&{guest}::static_ack());")
    } else {
        prefix.to_string()
    };
    let take = if !back.is_empty() {
        let shape = back
            .iter()
            .map(|(_, t)| t.as_str())
            .collect::<Vec<_>>()
            .join(",");
        let apply = back
            .iter()
            .enumerate()
            .map(|(n, (i, _))| format!("*p{i}=back.{n};"))
            .collect::<String>();
        format!(
            "let (answer,back):({owned},({shape},))=r.{take_method}().expect(\"the host answered with a value of the wrong type\"); let answer={lent}; {apply} answer"
        )
    } else if static_string {
        "let (answer,()):(&'static str,())=r.take_static().expect(\"the host answered with an invalid static string\"); answer".to_string()
    } else if m.ret == Ty::Unit {
        String::new()
    } else {
        format!(
            "let answer: {owned} = r.take().expect(\"the host answered with a value of the wrong type\"); {lent}"
        )
    };
    format!(
        "{sig} {{ let mut w = {guest}::begin(); {prefix} {put} let r = {guest}::host({id}u32, w); {take} }}"
    )
}

/// The type a decoded argument is held as: the owned form of a borrowed one.
fn held_in(types: &Types<'_>, ty: &Ty, statics: bool) -> Option<String> {
    Some(match ty {
        Ty::Str(Str::Borrowed) => "String".into(),
        Ty::List(item, List::Slice) => format!("Vec<{}>", held_in(types, item, statics)?),
        Ty::Ref(inner, _) => held_in(types, inner, statics)?,
        Ty::Str(Str::Static) if statics => "String".into(),
        Ty::Named(def) if types.defs[*def].lifetimes != 0 => {
            let fields = types.defs[*def].borrowed.fields()?;
            format!(
                "({},)",
                fields
                    .iter()
                    .map(|f| held_in(types, &f.ty, statics))
                    .collect::<Option<Vec<_>>>()?
                    .join(",")
            )
        }
        Ty::Dyn(..) => return None,
        Ty::Handler(..) => rust_ty(types, ty, false),
        Ty::Tuple(items) => format!(
            "({},)",
            items
                .iter()
                .map(|t| held_in(types, t, statics))
                .collect::<Option<Vec<_>>>()?
                .join(", ")
        ),
        Ty::Array(item, n) => format!("[{}; {n}]", held_in(types, item, statics)?),
        Ty::Option(inner) => format!("Option<{}>", held_in(types, inner, statics)?),
        Ty::Result(ok, err) => format!(
            "Result<{}, {}>",
            held_in(types, ok, statics)?,
            held_in(types, err, statics)?
        ),
        Ty::List(item, List::Vec) => format!("Vec<{}>", held_in(types, item, statics)?),
        other => rust_ty(types, other, false),
    })
}

pub(super) fn frame_cases<'t>(types: &'t Types<'_>, def: usize) -> Option<&'t [super::ty::Case]> {
    let super::ty::BorrowedShape::Variant(cases) = &types.defs[def].borrowed else {
        return None;
    };
    Some(cases.as_slice())
}
pub(super) fn held(types: &Types<'_>, ty: &Ty) -> Option<String> {
    held_in(types, ty, false)
}

/// How a field of a borrowed record is read into one: the expression building it from `local`,
/// the local's owned type, and whether the mod may change it (so it is sent back).
pub(super) struct Lent {
    pub(super) owned: String,
    pub(super) expr: String,
    pub(super) mutable: bool,
}

fn reborrow(types: &Types<'_>, ty: &Ty, place: &str) -> Option<String> {
    Some(match ty {
        Ty::Prim(..) | Ty::Wide(..) | Ty::Unit | Ty::Str(Str::Static) => place.to_owned(),
        Ty::Str(Str::Borrowed) => format!("({place}).as_str()"),
        Ty::Str(Str::Owned) => format!("({place}).clone()"),
        Ty::Named(d) if types.defs[*d].lifetimes != 0 => {
            let fields = types.defs[*d].borrowed.fields()?;
            format!(
                "mod_api::__wire_record_{d}({})",
                fields
                    .iter()
                    .enumerate()
                    .map(|(i, f)| reborrow(types, &f.ty, &format!("({place}).{i}")))
                    .collect::<Option<Vec<_>>>()?
                    .join(",")
            )
        }
        Ty::Ref(_, false) if !lifetime_def(types, ty) => format!("&({place})"),
        Ty::Array(t, _) => format!(
            "std::array::from_fn(|i| {})",
            reborrow(types, t, &format!("({place})[i]"))?
        ),
        Ty::List(_, List::Slice) => format!("({place}).as_slice()"),
        Ty::Option(t) => format!("({place}).as_ref().map(|x|{})", reborrow(types, t, "*x")?),
        Ty::Tuple(v) => format!(
            "({},)",
            v.iter()
                .enumerate()
                .map(|(i, t)| reborrow(types, t, &format!("({place}).{i}")))
                .collect::<Option<Vec<_>>>()?
                .join(",")
        ),
        _ if !lifetime_def(types, ty) => format!("({place}).clone()"),
        _ => return None,
    })
}
/// Writes a borrowed record tree directly in ordinary Wire order, without cloning its data.
pub(super) fn snapshot(types: &Types<'_>, ty: &Ty, place: &str) -> Option<String> {
    Some(match ty {
        Ty::Named(d) if types.defs[*d].lifetimes != 0 => {
            let fields = types.defs[*d].borrowed.fields()?;
            fields
                .iter()
                .map(|f| snapshot(types, &f.ty, &format!("({place}).{}", f.rust)))
                .collect::<Option<Vec<_>>>()?
                .join("")
                .to_string()
        }
        Ty::Ref(t, _) => snapshot(types, t, &format!("*({place})"))?,
        Ty::Option(t) => format!(
            "match &({place}) {{ Some(x) => {{ w.put(&1u8); {} }}, None => w.put(&0u8) }}",
            snapshot(types, t, "*x")?
        ),
        Ty::Array(t, _) => format!("for x in &({place}) {{ {} }}", snapshot(types, t, "*x")?),
        Ty::Str(Str::Borrowed) => format!("w.put({place});"),
        _ if !lifetime_def(types, ty) => format!("w.put(&({place}));"),
        _ => return None,
    })
}
pub(super) fn lend(types: &Types<'_>, ty: &Ty, local: &str) -> Option<Lent> {
    let (owned, expr, mutable) = match ty {
        ty if lifetime_def(types, ty) => (held(types, ty)?, reborrow(types, ty, local)?, false),
        Ty::Ref(inner, m) if !matches!(**inner, Ty::Dyn(..)) => (
            held(types, inner)?,
            format!("&{}{local}", if *m { "mut " } else { "" }),
            *m,
        ),
        Ty::Str(Str::Borrowed) => ("String".into(), format!("&{local}"), false),
        Ty::List(item, List::Slice) if has_ref_in_data(ty) => (
            held(types, ty)?,
            format!(
                "&{local}.iter().map(|x|{}).collect::<Vec<_>>()",
                reborrow(types, item, "*x")?
            ),
            false,
        ),
        Ty::List(_, List::Slice) => (held(types, ty)?, format!("&{local}"), false),
        Ty::Option(inner) => match &**inner {
            Ty::Ref(t, m) => (
                format!("Option<{}>", held(types, t)?),
                format!("{local}.as_{}ref()", if *m { "m" } else { "" })
                    .replace("as_mref", "as_mut"),
                *m,
            ),
            Ty::Str(Str::Borrowed) | Ty::List(_, List::Slice) => {
                (held(types, ty)?, format!("{local}.as_deref()"), false)
            }
            _ if has_ref_in_data(ty) => (held(types, ty)?, reborrow(types, ty, local)?, false),
            _ => (held(types, ty)?, local.to_string(), true),
        },
        _ if has_ref_in_data(ty) => (held(types, ty)?, reborrow(types, ty, local)?, false),
        _ => (held(types, ty)?, local.to_string(), true),
    };
    Some(Lent {
        owned,
        expr,
        mutable,
    })
}

pub(super) struct Writeback {
    pub(super) local: String,
    pub(super) path: String,
    pub(super) owned: String,
    pub(super) reference: bool,
    pub(super) optional: bool,
    pub(super) view: bool,
    pub(super) append: bool,
}
/// Writable leaves in wire order, shared by the guest's reply and host's atomic apply.
pub(super) fn writeback_fields(
    types: &Types<'_>,
    def: usize,
    name: &str,
    mutable: bool,
) -> Option<Vec<Writeback>> {
    if frame_cases(types, def).is_some() || types.defs[def].projection.is_some() {
        return Some(Vec::new());
    }
    let fields = types.defs[def].borrow_fields()?;
    let mut own = Vec::new();
    let mut refs = Vec::new();
    for (i, f) in fields.iter().enumerate() {
        let local = format!("{name}_{i}");
        if matches!(f.ty, Ty::Dyn(..)) {
            continue;
        }
        let (inner, m) = match &f.ty {
            Ty::Ref(t, m) => (&**t, *m),
            ty => (ty, true),
        };
        if let Ty::Named(d) = inner
            && frame_cases(types, *d).is_some()
        {
            continue;
        }
        if let Ty::Named(d) = inner
            && let Some(view) = &types.defs[*d].view
        {
            if mutable && view.mutable {
                own.push(Writeback {
                    local,
                    path: format!("{name}.{}", f.rust),
                    owned: rust_ty(types, &view.snapshot, false),
                    reference: false,
                    optional: false,
                    view: true,
                    append: false,
                });
            }
            continue;
        }
        if let Ty::Named(d) = inner
            && types.defs[*d].lifetimes != 0
        {
            for mut leaf in writeback_fields(types, *d, &local, mutable && m)? {
                leaf.path = leaf.path.replacen(&local, &format!("{name}.{}", f.rust), 1);
                own.push(leaf);
            }
            continue;
        }
        if matches!(&f.ty, Ty::Option(t) if matches!(&**t, Ty::Ref(inner, false) if lifetime_def(types, inner)))
        {
            continue;
        }
        let lent = lend(types, &f.ty, &local)?;
        if mutable && lent.mutable {
            let by_ref = matches!(f.ty, Ty::Ref(..) | Ty::Option(..)) && lent.expr != local;
            let leaf = Writeback {
                local,
                path: format!("{name}.{}", f.rust),
                owned: lent.owned,
                reference: matches!(f.ty, Ty::Ref(_, true)),
                optional: matches!(&f.ty,Ty::Option(t) if matches!(**t,Ty::Ref(_,true))),
                view: false,
                append: types.defs[def]
                    .append
                    .as_ref()
                    .is_some_and(|a| a.field == i),
            };
            if by_ref {
                refs.push(leaf);
            } else {
                own.push(leaf);
            }
        }
    }
    own.extend(refs);
    if types.defs[def].rust.ends_with("BlockEntityTickEvent") {
        own.sort_by_key(|leaf| !leaf.path.ends_with(".reply"));
    }
    Some(own)
}

fn borrowed(
    types: &Types<'_>,
    def: usize,
    name: &str,
    mutable: bool,
) -> Option<(String, String, String)> {
    borrowed_prepared(types, def, name, mutable, None)
}
fn borrowed_prepared(
    types: &Types<'_>,
    def: usize,
    name: &str,
    mutable: bool,
    prepared: Option<(&str, &str)>,
) -> Option<(String, String, String)> {
    let d = &types.defs[def];
    if let Some(cases) = frame_cases(types, def) {
        let mut arms = String::new();
        for (i, case) in cases.iter().enumerate() {
            let v = &case.payload;
            let Ty::Dyn(_, m) = &v[0] else { unreachable!() };
            write!(
                arms,
                "{i} => {}::{}(&{}{name}_frame),",
                d.rust,
                case.rust,
                if *m { "mut " } else { "" }
            )
            .unwrap();
        }
        let read = format!(
            "let mut {name}_frame=Guest; let {name}=match input.take::<u32>()? {{ {arms} tag=>return Err(crate::wire::WireError::Tag(tag)) }};"
        );
        return Some((read, name.into(), String::new()));
    }
    if let Some(view) = &d.view {
        let pointee = rust_ty(types, &view.pointee, false);
        let read = format!(
            "let mut {name}_held: {pointee} = input.take()?; let mut {name} = {}::new(&{}{name}_held);",
            d.rust,
            if view.mutable { "mut " } else { "" }
        );
        return Some((
            read,
            name.into(),
            format!("{name}.__wire_put_snapshot(out);"),
        ));
    }
    let fields = d.borrow_fields()?;
    let (mut read, mut build, mut back) = (String::new(), Vec::new(), String::new());
    for (i, f) in fields.iter().enumerate() {
        let local = format!("{name}_{i}");
        if let Some((hook, field)) = prepared
            && f.rust == field
        {
            writeln!(read, "let {local}_slot:u32=input.take()?; let {local}_generation:u32=input.take()?; let {local}=PREPARED_{}.with(|slots| slots.borrow().borrow({local}_slot,{local}_generation))?;", hook.to_uppercase()).unwrap();
            build.push(format!("{}: &{local}.0", f.rust));
            continue;
        }
        if prepared.is_some() && f.rust == "slot" {
            let slots = super::prepared::SLOTS;
            writeln!(read,"let {local}_id:u32=input.take()?; let {local}= {slots:?}.get({local}_id as usize).copied().ok_or(crate::wire::WireError::Invalid(\"unknown equipment slot\"))?;").unwrap();
            build.push(format!("{}: {local}", f.rust));
            continue;
        }
        if let Some((_, field)) = prepared
            && f.rust == "kind"
        {
            let at = fields.iter().position(|f| f.rust == field)?;
            writeln!(read,"let {local}_id:u32=input.take()?; let {local}={name}_{at}.1.get({local}_id as usize).ok_or(crate::wire::WireError::Invalid(\"unknown prepared entity kind\"))?;").unwrap();
            build.push(format!("{}: {local}.as_str()", f.rust));
            continue;
        }
        // A frame object the event carries is the guest's own, never sent.
        if let Ty::Dyn(_, m) = &f.ty {
            writeln!(read, "let mut {local} = Guest;").unwrap();
            build.push(format!(
                "{}: &{}{local}",
                f.rust,
                if *m { "mut " } else { "" }
            ));
            continue;
        }
        // An event inside the event is read the same way, down to its own fields.
        let nested = match &f.ty {
            Ty::Ref(t, m) => Some((&**t, *m, true)),
            ty => Some((ty, mutable, false)),
        };
        if let Some((Ty::Named(d), m, by_ref)) = nested
            && types.defs[*d].lifetimes != 0
        {
            let (inner, expr, send) = borrowed(types, *d, &local, m)?;
            read.push_str(&inner);
            let m = if m { "mut " } else { "" };
            build.push(format!(
                "{}: {}{expr}",
                f.rust,
                if by_ref {
                    format!("&{m}")
                } else {
                    String::new()
                }
            ));
            let _ = send;
            continue;
        }
        // Optional references to borrowed records need a local record before borrowing it.
        if let Ty::Option(t) = &f.ty
            && let Ty::Ref(inner, false) = &**t
            && lifetime_def(types, inner)
        {
            let owned = held(types, &f.ty)?;
            let rebuilt = reborrow(types, inner, "*x")?;
            write!(read, "let {local}_held: {owned} = input.take()?; let {local} = {local}_held.as_ref().map(|x| {rebuilt});").unwrap();
            build.push(format!("{}: {local}.as_ref()", f.rust));
            continue;
        }
        let lent = lend(types, &f.ty, &local)?;
        let m = if matches!(f.ty, Ty::Ref(_, true))
            || matches!(&f.ty, Ty::Option(t) if matches!(**t, Ty::Ref(_, true)))
        {
            "mut "
        } else {
            ""
        };
        writeln!(read, "let {m}{local}: {} = input.take()?;", lent.owned).unwrap();
        build.push(format!("{}: {}", f.rust, lent.expr));
    }
    for leaf in writeback_fields(types, def, name, mutable)? {
        if leaf.append {
            write!(back, "out.put({name}.__wire_changes());").unwrap();
        } else if leaf.optional {
            write!(back, "out.put_opt({}.as_deref());", leaf.path).unwrap();
        } else if leaf.view {
            if d.rust.ends_with("BlockEntityTickEvent") {
                write!(back, "if *{name}.reply == crate::BlockEntityTickReply::Changed {{ {}.__wire_put_snapshot(out); }}", leaf.path).unwrap();
            } else {
                write!(back, "{}.__wire_put_snapshot(out);", leaf.path).unwrap();
            }
        } else {
            write!(
                back,
                "out.put(&{});",
                if leaf.reference {
                    format!("*{}", leaf.path)
                } else {
                    leaf.path
                }
            )
            .unwrap();
        }
    }
    let m = if mutable { "mut " } else { "" };
    if d.projection.is_some() {
        let args = build
            .iter()
            .map(|f| f.split_once(": ").unwrap().1)
            .collect::<Vec<_>>()
            .join(",");
        read.push_str(&format!(
            "let {m}{name} = {}::__wire_from_fields({args});",
            d.rust
        ));
    } else if matches!(d.borrowed, super::ty::BorrowedShape::TupleRecord(_)) {
        read.push_str(&format!(
            "let {m}{name} = {} {{ {} }};",
            d.rust,
            build.join(", ")
        ));
    } else {
        let args = build
            .iter()
            .map(|field| field.split_once(": ").unwrap().1)
            .collect::<Vec<_>>()
            .join(", ");
        read.push_str(&format!(
            "let {m}{name} = crate::__wire_record_{def}({args});"
        ));
    }
    Some((read, name.to_string(), back))
}

/// The export a mod answers with the hooks it listens to: they cross as names, which stay
/// the same across builds, and the game maps them to its own ids.
fn is_subscription(m: &Method) -> bool {
    super::SUBSCRIBED
        .iter()
        .any(|(owner, name)| *owner == m.owner && *name == m.name)
}

/// `hook_name`: a hook's name by its id, as a mod sends its subscriptions.
pub(super) fn hook_names(hooks: &[(String, String)]) -> String {
    let arms: String = hooks
        .iter()
        .map(|(variant, name)| format!("crate::HookId::{variant} => \"{name}\","))
        .collect();
    format!("pub fn hook_name(id: crate::HookId) -> &'static str {{ match id {{ {arms} }} }}\n")
}

/// Whether an export has what its dispatch arm needs: every argument decodes to an owned value
/// or is the frame's host, and the answer is owned.
pub(super) fn dispatchable(types: &Types<'_>, m: &Method) -> bool {
    is_subscription(m)
        || !has_frame(&m.ret)
            && !borrowed_ret(&m.ret)
            && m.params.iter().all(|(_, ty)| match ty {
                Ty::Dyn(..) => true,
                Ty::Ref(inner, m) => match &**inner {
                    Ty::Named(d) if types.defs[*d].lifetimes != 0 => {
                        borrowed(types, *d, "e", *m).is_some()
                    }
                    _ => held(types, ty).is_some(),
                },
                Ty::Named(d) if types.defs[*d].lifetimes != 0 => {
                    borrowed(types, *d, "e", false).is_some()
                }
                _ => held(types, ty).is_some(),
            })
}

/// The guest's `hook`: one arm for every export a mod can be called through, reading the
/// arguments from the buffer, calling the mod, and writing the answer then each `&mut`
/// argument back whole (the host takes from it the fields a mod may change). The arms'
/// ids are the exports' positions; the exports that need more than that are returned by name.
pub(super) fn dispatch(
    types: &Types<'_>,
    methods: &[Method],
    prepared: &[(String, String)],
) -> (String, Vec<String>) {
    let all: Vec<&Method> = methods
        .iter()
        .filter(|m| exports().contains(&m.owner))
        .collect();
    let mut arms = String::new();
    let mut preparation = String::new();
    let mut apart = Vec::new();
    for (id, m) in all.iter().enumerate() {
        if !dispatchable(types, m) {
            apart.push(format!("{}::{}", m.owner, m.name));
            continue;
        }
        let receiver = match super::mod_receiver(m.owner) {
            Some(receiver) => receiver.to_string(),
            None => format!(
                "crate::wire::guest::handler::<dyn {}>(input.take()?)?",
                trait_path(m.owner)
            ),
        };
        let prepared_parameter = prepared
            .iter()
            .find(|(hook, _)| hook == &m.name)
            .map(|(hook, field)| (hook.as_str(), field.as_str()));
        if let Some((hook, _)) = prepared_parameter {
            let upper = hook.to_uppercase();
            let prepare = super::prepared::PREPARE;
            let release = super::prepared::RELEASE;
            writeln!(preparation, "thread_local! {{ static PREPARED_{upper}: std::cell::RefCell<crate::wire::guest::PreparedSlots<(crate::ItemStack,Vec<String>,std::rc::Rc<crate::__private::EquipReadSet>)>> = std::cell::RefCell::default(); }}").unwrap();
            writeln!(preparation, "#[cfg(target_arch=\"wasm32\")] #[unsafe(export_name=\"prepare_{hook}\")] pub extern \"C\" fn entry_prepare_{hook}()->u64 {{ let (p,n)=crate::wire::guest::run({prepare}); (p as usize as u64)<<32 | u64::from(n) }}").unwrap();
            writeln!(preparation, "#[cfg(target_arch=\"wasm32\")] #[unsafe(export_name=\"ask_{hook}\")] pub extern \"C\" fn entry_ask_{hook}()->u64 {{ let (p,n)=crate::wire::guest::run({id}); (p as usize as u64)<<32 | u64::from(n) }}").unwrap();
            writeln!(preparation, "#[cfg(target_arch=\"wasm32\")] #[unsafe(export_name=\"release_{hook}\")] pub extern \"C\" fn entry_release_{hook}()->u64 {{ let (p,n)=crate::wire::guest::run({release}); (p as usize as u64)<<32 | u64::from(n) }}").unwrap();
            writeln!(arms, "{prepare} => {{ let slot:u32=input.take()?; let generation:u32=input.take()?; let stack:crate::ItemStack=input.take()?; let kinds:Vec<String>=input.take()?; let reads:Vec<crate::EquipReadRow>=input.take()?; if !input.rest().is_empty() {{ return Err(crate::wire::WireError::Trailing); }} PREPARED_{upper}.with(|slots| slots.borrow_mut().prepare(slot,generation,(stack,kinds,std::rc::Rc::new(crate::__private::EquipReadSet::new(reads)))))?; out.put(&(slot,generation)); }}, {release} => {{ let slot:u32=input.take()?; let generation:u32=input.take()?; if !input.rest().is_empty() {{ return Err(crate::wire::WireError::Trailing); }} PREPARED_{upper}.with(|slots| slots.borrow_mut().release(slot,generation))?; }}").unwrap();
        }
        let mut body = String::new();
        let mut args = Vec::new();
        let mut back = String::new();
        for (i, (_, ty)) in m.params.iter().enumerate() {
            match ty {
                Ty::Dyn(_, true) => args.push("&mut host".to_string()),
                Ty::Dyn(..) => args.push("&host".to_string()),
                Ty::Named(d) if types.defs[*d].lifetimes != 0 => {
                    let (read, name, _) = borrowed(types, *d, &format!("p{i}"), false).unwrap();
                    body.push_str(&read);
                    args.push(name);
                }
                Ty::Ref(inner, m) if matches!(&**inner, Ty::Named(d) if types.defs[*d].lifetimes != 0) =>
                {
                    let Ty::Named(d) = &**inner else {
                        unreachable!()
                    };
                    let (read, name, send) =
                        borrowed_prepared(types, *d, &format!("p{i}"), *m, prepared_parameter)
                            .unwrap();
                    writeln!(body, "{read}").unwrap();
                    args.push(format!("&{}{name}", if *m { "mut " } else { "" }));
                    back.push_str(&send);
                }
                Ty::Ref(_, true) => {
                    writeln!(
                        body,
                        "let mut p{i}: {} = input.take()?;",
                        held(types, ty).unwrap()
                    )
                    .unwrap();
                    args.push(format!("&mut p{i}"));
                    write!(back, "out.put(&p{i});").unwrap();
                }
                Ty::Ref(..) | Ty::Str(Str::Borrowed) | Ty::List(_, List::Slice) => {
                    writeln!(
                        body,
                        "let p{i}: {} = input.take()?;",
                        held(types, ty).unwrap()
                    )
                    .unwrap();
                    args.push(format!("&p{i}"));
                }
                // An optional borrow (`Option<&PosedParts>`) arrives owned and is lent again.
                Ty::Option(inner)
                    if matches!(
                        **inner,
                        Ty::Ref(_, false) | Ty::Str(Str::Borrowed) | Ty::List(_, List::Slice)
                    ) =>
                {
                    writeln!(
                        body,
                        "let p{i}: {} = input.take()?;",
                        held(types, ty).unwrap()
                    )
                    .unwrap();
                    let lend = if matches!(**inner, Ty::Ref(..)) {
                        "as_ref"
                    } else {
                        "as_deref"
                    };
                    args.push(format!("p{i}.{lend}()"));
                }
                _ => {
                    writeln!(
                        body,
                        "let p{i}: {} = input.take()?;",
                        held(types, ty).unwrap()
                    )
                    .unwrap();
                    args.push(format!("p{i}"));
                }
            }
        }
        if prepared_parameter.is_some() {
            body.push_str("let _reads=crate::__private::enter_equip_reads(std::rc::Rc::clone(&p1_0.2),p1.entity.entity());");
        }
        let call = format!("receiver.{}({})", m.name, args.join(", "));
        let answer = if m.owner == "Hooks" && m.name == "subscribed" {
            format!(
                "{call}.iter().flatten().map(|id| {{ let listener = receiver.listeners().iter().find(|l| l.hook.is_some_and(|h| h as usize == *id as usize)); (hook_name(*id).to_string(), listener.map_or(0, |l| l.priority), listener.is_some_and(|l| l.receive_cancelled)) }}).collect::<Vec<_>>()"
            )
        } else if is_subscription(m) {
            format!(
                "{call}.iter().flatten().map(|id| hook_name(*id).to_string()).collect::<Vec<String>>()"
            )
        } else {
            call
        };
        if m.owner == "Hooks" && m.name == "register" {
            back.push_str("for warning in crate::__private::take_registration_warnings(&p1) { crate::RegistrationRefusalRegistrar::registration_warning(&mut host, warning); } if let Some(reason) = crate::__private::take_registration_refusal(&p1) { crate::RegistrationRefusalRegistrar::registration_refusal(&mut host, reason); }");
        }
        writeln!(
            arms,
            "{id} => {{ let receiver = {receiver}; {body} let answer = {answer}; out.put_owned(answer); {back} }}"
        )
        .unwrap();
    }
    let code = format!(
        "// @generated by horizon-mod-api-gen; do not edit.\n{preparation}\npub fn hook(id: u32, input: &mut crate::wire::Reader<'_>, out: &mut crate::wire::Writer) -> Result<(), crate::wire::WireError> {{\nlet mut host = Guest;\nmatch id {{\n{arms}_ => return Err(crate::wire::WireError::Tag(id)) }}\nOk(())\n}}\n"
    );
    (code, apart)
}

#[test]
#[ignore = "daily"]
fn a_reborrowed_array_has_no_redundant_parentheses() {
    // `-D warnings` lint jobs refuse `unused_parens` in the generated host.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let generated = crate::sources(&root, crate::FillMode::Require).unwrap();
    let included = [
        ("mod_api_generated.rs", generated.hooks.as_str()),
        ("mod_api_generated_host.rs", generated.host.as_str()),
        ("mod_api_host_areas.rs", generated.areas.as_str()),
    ];
    let types = Types::new(crate::index::Index::new(&root, &included)).unwrap();
    let cube = Ty::Option(Box::new(Ty::Array(Box::new(Ty::Prim("f32")), 4)));
    assert_eq!(
        reborrow(&types, &cube, "p1").unwrap(),
        "(p1).as_ref().map(|x|std::array::from_fn(|i| (*x)[i]))"
    );
    assert_eq!(
        reborrow(&types, &Ty::Prim("u32"), "(p1).3").unwrap(),
        "(p1).3"
    );
}
