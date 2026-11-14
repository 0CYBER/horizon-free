//! Host code. Runtime contract (on WasmMod and WasmHandler):
//! `fn call<R>(&self, class: CallClass, frame: Frame, id: u32, handler: Option<Id>,
//! encode: impl FnOnce(&mut Writer), decode: impl FnOnce(&mut Reader<'_>) -> Result<R, WireError>) -> Option<R>`.
//! `None` means refused/trapped; the runtime poisons the group before returning it.
use super::{Method, calls::*, ty::Ty};
use heck::ToSnakeCase;
use std::fmt::Write;

fn snake(owner: &str) -> String {
    owner.rsplit("::").next().unwrap().to_snake_case()
}

fn serves(supers: &[(&str, Vec<String>)], child: &str, parent: &str) -> bool {
    let mut stack = vec![child];
    let mut visited = std::collections::HashSet::new();
    while let Some(name) = stack.pop() {
        if name == parent {
            return true;
        }
        if !visited.insert(name) {
            continue;
        }
        if let Some((_, parents)) = supers
            .iter()
            .find(|(owner, _)| owner.rsplit("::").next() == Some(name))
        {
            stack.extend(parents.iter().map(String::as_str));
        }
    }
    false
}

fn frame(surface: &mut super::Surface<'_>) -> Result<String, String> {
    use crate::index::Found;
    let types = &mut surface.types;
    let api = types.index.module("mod_api", &[])?;
    let mut supers = Vec::new();
    for owner in super::imports() {
        let path = owner.split("::").map(str::to_string).collect::<Vec<_>>();
        let Found::Item(id) = types.index.resolve_path(api, &path)? else {
            return Err(format!("{owner} is not a trait"));
        };
        let syn::Item::Trait(item) = types.index.item(id) else {
            return Err(format!("{owner} is not a trait"));
        };
        let parents = item
            .supertraits
            .iter()
            .filter_map(|b| match b {
                syn::TypeParamBound::Trait(t) => Some(t.path.segments.last()?.ident.to_string()),
                _ => None,
            })
            .collect::<Vec<_>>();
        supers.push((*owner, parents));
    }
    let mut fields = String::new();
    let mut methods = String::new();
    for owner in super::imports() {
        let name = snake(owner);
        let path = trait_path(owner);
        if *owner == "ConfigFiles" {
            let mut roots = String::new();
            for (root, source, accessor) in super::FILE_ROOTS {
                let access = surface
                    .dropped
                    .iter()
                    .chain(surface.methods.iter())
                    .find(|method| method.owner == *source && method.name == *accessor)
                    .and_then(|method| method.access.as_deref())
                    .unwrap_or("WorldOnly");
                let source = snake(source);
                writeln!(roots, "{root} => {{ if !mod_api::__private::access_allowed({accessor:?}, mod_api::__private::LoadAccess::{access}) {{ return Err(ServeError::Phase {{ operation: {accessor:?}, phase: mod_api::__private::current_phase() }}); }} Ok(self.{source}()?.{accessor}()) }},").unwrap();
            }
            writeln!(methods, "pub unsafe fn config_files(&self, root: u8) -> Result<&'static dyn {path}, ServeError> {{ match root {{ {roots} _ => Err(ServeError::Decode(WireError::Invalid(\"file capability root\"))) }} }}").unwrap();
            continue;
        }
        if let Some((_, from, expr)) = super::REACHED.iter().find(|(o, ..)| o == owner) {
            let mutable = from.ends_with("_mut")
                || (from.is_empty()
                    && surface
                        .methods
                        .iter()
                        .any(|m| m.owner == *owner && m.receiver == Some(true)));
            let body = if from.is_empty() {
                let mut body = expr.to_string();
                for accessor in ["level_host", "host", "level_random_host_mut", "host_mut"] {
                    body = body.replace(&format!("{accessor}()"), &format!("self.{accessor}()"));
                }
                body.replace("level_self.host", "level_host")
                    .replace("level_random_self.host_mut", "level_random_host_mut")
            } else {
                format!("let h = self.{from}()?; {expr}")
            };
            let body = body.replace(
                "h.backend().map(crate::BackendRef::open)",
                "crate::backend_of(h)",
            );
            writeln!(methods, "pub unsafe fn {name}{suffix}(&self) -> Result<&'static {m}dyn {path}, ServeError> {{ {body} }}", suffix=if mutable {"_mut"} else {""}, m=if mutable {"mut "} else {""}).unwrap();
            continue;
        }
        writeln!(
            fields,
            "{name}: Option<(*mut (dyn {path} + 'static), bool)>,"
        )
        .unwrap();
        writeln!(methods, "pub fn set_{name}(&mut self, v: &(dyn {path} + '_)) {{ self.{name} = Some((unsafe {{ std::mem::transmute(v as *const (dyn {path} + '_)) }}, false)); }} pub fn set_{name}_mut(&mut self, v: &mut (dyn {path} + '_)) {{ self.{name} = Some((unsafe {{ std::mem::transmute(v as *mut (dyn {path} + '_)) }}, true)); }}").unwrap();
        for mutable in [false, true] {
            let mut choices = Vec::new();
            for sub in super::imports() {
                if super::REACHED.iter().any(|(o, ..)| o == sub) {
                    continue;
                }
                if serves(
                    &supers,
                    sub.rsplit("::").next().unwrap(),
                    owner.rsplit("::").next().unwrap(),
                ) {
                    choices.push(format!(
                        "self.{}{filter}.map(|(p, _)| unsafe {{ &{m}*p as &{m}dyn {path} }})",
                        snake(sub),
                        filter = if mutable { ".filter(|(_, m)| *m)" } else { "" },
                        m = if mutable { "mut " } else { "" }
                    ));
                }
            }
            // A zero-argument accessor lends another frame resource. Resolve directly
            // from a stored source pointer, not recursively through getters: cyclic
            // accessor graphs cannot recurse when no source is present.
            for method in &surface.methods {
                let (target, ret_mut, fallible) = match &method.ret {
                    Ty::Dyn(target, mutable) => (target.as_str(), *mutable, false),
                    _ => match super::fallible_frame_accessor(method) {
                        Some((target, mutable, _)) => (target, mutable, true),
                        None => continue,
                    },
                };
                if !serves(&supers, target, owner.rsplit("::").next().unwrap())
                    || (mutable && !ret_mut)
                    || !method.params.is_empty()
                    || !super::imports().contains(&method.owner)
                    || method.owner == *owner
                    || super::REACHED.iter().any(|(o, ..)| *o == method.owner)
                {
                    continue;
                }
                let source = snake(method.owner);
                let source_mut = method.receiver == Some(true);
                let filter = if source_mut { ".filter(|(_,m)|*m)" } else { "" };
                let s = if source_mut { "mut " } else { "" };
                let m = if mutable { "mut " } else { "" };
                if fallible {
                    choices.push(format!("self.{source}{filter}.and_then(|(p,_)| unsafe {{ (&{s}*p).{}().ok().map(|host| host as &{m}dyn {path}) }})", method.name));
                } else {
                    choices.push(format!("self.{source}{filter}.map(|(p,_)| unsafe {{ (&{s}*p).{}() as &{m}dyn {path} }})", method.name));
                }
            }
            let expr = choices.join(".or_else(|| ");
            let expr = format!("{expr}{}", ")".repeat(choices.len().saturating_sub(1)));
            writeln!(methods, "pub unsafe fn {name}{suffix}(&self) -> Result<&'static {m}dyn {path}, ServeError> {{ {expr}.ok_or(ServeError::Missing(\"{name}\")) }}", suffix=if mutable {"_mut"} else {""}, m=if mutable {"mut "} else {""}).unwrap();
        }
    }
    Ok(format!(
        "#[derive(Default)] pub struct Frame {{ {fields} }}\n// SAFETY: pointers are used only by unsafe serve during the synchronous call that lent them.\nunsafe impl Send for Frame {{}}\nimpl Frame {{ {methods} }}"
    ))
}

fn resource(ty: &Ty) -> bool {
    match ty {
        Ty::Handler(..) => true,
        Ty::Option(t) => resource(t),
        _ => false,
    }
}
fn supported_resource(ty: &Ty) -> bool {
    match ty {
        Ty::Handler(name, _) => name == super::ERASED || super::exports().contains(&name.as_str()),
        Ty::Option(t) => supported_resource(t),
        _ => false,
    }
}
pub(super) fn put(ty: &Ty, expr: &str) -> String {
    match ty {
        Ty::Ref(..) => format!("w.put(&*{expr});"),
        Ty::Str(super::ty::Str::Borrowed | super::ty::Str::Static)
        | Ty::List(_, super::ty::List::Slice) => format!("w.put({expr});"),
        Ty::Option(inner)
            if matches!(
                **inner,
                Ty::Ref(..)
                    | Ty::Str(super::ty::Str::Borrowed)
                    | Ty::List(_, super::ty::List::Slice)
            ) =>
        {
            // Already a borrow: `Option<&T>` is passed as it is.
            format!("w.put_opt({expr});")
        }
        _ => format!("w.put(&{expr});"),
    }
}

fn event_input(
    types: &super::ty::Types<'_>,
    def: usize,
    expr: &str,
    setup: &mut String,
    put: &mut String,
    prepared: Option<&str>,
) {
    if types.defs[def].append.is_some() || types.defs[def].projection.is_some() {
        write!(
            put,
            "{{ use mod_api::wire::Put as _; ({expr}).put_to(w); }}"
        )
        .unwrap();
        return;
    }
    if let Some(cases) = frame_cases(types, def) {
        let tag = format!("tag_{}", expr.replace('.', "_"));
        write!(setup, "let {tag}=match &mut {expr} {{").unwrap();
        for (i, case) in cases.iter().enumerate() {
            let v = &case.payload;
            let Ty::Dyn(name, m) = &v[0] else {
                unreachable!()
            };
            write!(
                setup,
                "{}::{}(h) => {{ frame.set_{}{}(&{}**h); {i}u32 }},",
                types.defs[def].rust,
                case.rust,
                snake(name),
                if *m { "_mut" } else { "" },
                if *m { "mut " } else { "" }
            )
            .unwrap();
        }
        setup.push_str("};");
        write!(put, "w.put(&{tag});").unwrap();
        return;
    }
    let Some(fields) = types.defs[def].borrowed.fields() else {
        return;
    };
    for f in fields.iter() {
        let path = format!("{expr}.{}", f.rust);
        if prepared == Some(f.rust.as_str()) {
            put.push_str("w.put(&prepared.slot);w.put(&prepared.generation);");
            continue;
        }
        if prepared.is_some() && f.rust == "slot" {
            put.push_str("w.put(&slot_id);");
            continue;
        }
        if prepared.is_some() && f.rust == "kind" {
            put.push_str("w.put(&kind_id);");
            continue;
        }
        match &f.ty {
            Ty::Dyn(name, m) => {
                write!(
                    setup,
                    "frame.set_{}{}(&{}*{path});",
                    snake(name),
                    if *m { "_mut" } else { "" },
                    if *m { "mut " } else { "" }
                )
                .unwrap();
            }
            ty => {
                let inner = match ty {
                    Ty::Ref(t, _) => &**t,
                    t => t,
                };
                if let Ty::Named(d) = inner
                    && types.defs[*d].lifetimes != 0
                    && types.defs[*d].view.is_none()
                {
                    event_input(types, *d, &path, setup, put, None);
                } else if lifetime_def(types, ty)
                    && !matches!(inner, Ty::Named(d) if types.defs[*d].view.is_some())
                {
                    put.push_str(&super::calls::snapshot(types, ty, &path).unwrap());
                } else {
                    put.push_str(&self::put(ty, &path));
                }
            }
        }
    }
}
/// What a proxy answers when its guest call answers nothing. `reason`, where there was a call,
/// is the expression naming why (`crate::wasm_call_refusal()`, the including crate's). An
/// isolated hook (#2414 ruling) unwinds instead: the isolated dispatch catches the unwind the
/// same way it catches a native listener's panic, so a trapped or refused wasm call cannot fall
/// back to `Continue` and leave an earlier listener's replacement standing (review round 1).
fn fallback(
    types: &super::ty::Types<'_>,
    m: &Method,
    reason: Option<&str>,
    isolate: bool,
) -> String {
    match &m.ret {
        // String-error operations can report a refused VM call directly. In particular,
        // callbacks must not turn a trap after staged effects into a successful commit.
        Ty::Result(_, error) if matches!(**error, Ty::Str(super::ty::Str::Owned)) => match reason {
            Some(reason) => {
                format!("Err(format!(\"wasm call refused or trapped: {{}}\", {reason}))")
            }
            None => "Err(String::from(\"wasm call refused or trapped\"))".into(),
        },
        Ty::Result(ok, _) if **ok == Ty::Unit => "Ok(())".into(),
        Ty::Result(..) => "Err(call_refused())".into(),
        Ty::Named(d) if types.defs[*d].rust.ends_with("::Flow") => {
            if isolate {
                match reason {
                    Some(reason) => format!(
                        "panic!(\"{}: wasm call refused or trapped: {{}}\", {reason})",
                        m.name
                    ),
                    None => format!("panic!(\"{}: wasm call refused or trapped\")", m.name),
                }
            } else {
                "mod_api::Flow::Continue".into()
            }
        }
        _ => "NoAnswer::no_answer()".into(),
    }
}

fn resources() -> String {
    let mut store = String::from(
        r#"if value.is::<std::sync::Arc<dyn std::any::Any + Send + Sync>>() {
        let value=*value.downcast::<std::sync::Arc<dyn std::any::Any + Send + Sync>>().map_err(|_|WireError::Invalid("wrong erased handler"))?;
        if let Some(proxy)=value.downcast_ref::<crate::WasmHandler>() {
            let module=unsafe {(&*cx).module.as_ref()}.ok_or(WireError::Invalid("no calling module"))?;
            if proxy.belongs_to(module) {return Ok(proxy.index)}
        }
        return Ok(mod_api::wire::Unsupported::ID);
    }"#,
    );
    let mut load = String::from(
        r#"if view==std::any::TypeId::of::<std::sync::Arc<dyn std::any::Any + Send + Sync>>() {
        let value:std::sync::Arc<dyn std::any::Any + Send + Sync>=if id==mod_api::wire::Unsupported::ID {mod_api::wire::Unsupported::shared()} else {std::sync::Arc::new(crate::WasmHandler::new(&module,id))};
        return Ok(Box::new(value));
    }"#,
    );
    for owner in super::exports()
        .iter()
        .filter(|owner| super::mod_receiver(owner).is_none())
    {
        let path = trait_path(owner);
        writeln!(store,"if value.is::<std::sync::Arc<dyn {path}>>() {{ let value = *value.downcast::<std::sync::Arc<dyn {path}>>().map_err(|_| WireError::Invalid(\"wrong handler\"))?; return Ok(unsafe {{ (&mut *cx).held.insert_long(Box::new(OwnerThread::new(value))) }}); }}").unwrap();
        for container in ["Box", "std::sync::Arc"] {
            writeln!(load,"if view == std::any::TypeId::of::<{container}<dyn {path}>>() {{ return Ok(Box::new({container}::new(crate::WasmHandler::new(&module,id)) as {container}<dyn {path}>)); }}").unwrap();
        }
    }
    format!(
        r#"struct HostCodec;
thread_local! {{
    static CURRENT_CX:std::cell::Cell<*mut Cx> = const {{std::cell::Cell::new(std::ptr::null_mut())}};
    static HOST_CODEC:std::rc::Rc<dyn mod_api::wire::ResourceCodec> = std::rc::Rc::new(HostCodec);
}}
fn current_cx()->Result<*mut Cx,WireError> {{
    let cx=CURRENT_CX.with(|current|current.get());
    if cx.is_null() {{Err(WireError::Invalid("no resource context"))}} else {{Ok(cx)}}
}}
impl mod_api::wire::ResourceCodec for HostCodec {{
    fn store(&self,value:Box<dyn std::any::Any>)->Result<mod_api::wire::Id,WireError> {{
        let cx=current_cx()?;
        {store}
        Err(WireError::Invalid("unknown handler type"))
    }}
    fn load(&self,id:mod_api::wire::Id,view:std::any::TypeId)->Result<Box<dyn std::any::Any>,WireError> {{
        let cx=current_cx()?;
        let module=unsafe {{(&*cx).module.as_ref()}}.ok_or(WireError::Invalid("no calling module"))?;
        {load}
        Err(WireError::Invalid("unknown handler view"))
    }}
    fn refused(&self,handler:&str) {{
        let Ok(cx)=current_cx() else {{return}};
        let Some(module)=(unsafe {{(&*cx).module.as_ref()}}) else {{return}};
        let seen=unsafe {{&mut (*cx).refused}};
        match seen.get_mut(module.mod_id()) {{
            Some(names) if names.contains(handler)=>return,
            Some(names)=>{{names.insert(handler.to_owned());}},
            None=>{{seen.insert(module.mod_id().to_owned(),std::collections::HashSet::from([handler.to_owned()]));}},
        }}
        log::debug!("wasm mod {{}}: {{handler:?}} unsupported: cross-group capability",module.mod_id());
    }}
}}
struct HostContext(*mut Cx);
impl Drop for HostContext {{fn drop(&mut self) {{CURRENT_CX.with(|current|current.set(self.0));}}}}
// Field order removes the codec scope before restoring the outer context.
pub struct HostResources {{_scope:mod_api::wire::ResourceScope,_context:HostContext}}
/// # Safety
/// cx and its frame pointees stay live until this guard drops. These private runtime
/// guards must drop in reverse construction order, before the frame or store is released.
pub unsafe fn resource_scope(cx:*mut Cx,native_root:bool)->HostResources {{
    let context=HostContext(CURRENT_CX.with(|current|current.replace(cx)));
    let codec=HOST_CODEC.with(std::rc::Rc::clone);
    let scope=if native_root {{unsafe {{mod_api::wire::ResourceScope::native_root(codec)}}}} else {{mod_api::wire::ResourceScope::new(codec)}};
    HostResources {{_scope:scope,_context:context}}
}}
"#
    )
}
pub(super) fn generate(
    surface: &mut super::Surface<'_>,
    prepared_hooks: &[(String, String)],
    isolated_hooks: &std::collections::HashSet<String>,
) -> Result<String, String> {
    let mut code = String::from(
        "// @generated by horizon-mod-api-gen; do not edit.\nuse mod_api::wire::{Reader, Writer, WireError, Slots};\n#[derive(Debug)] pub enum ServeError { Decode(WireError), Missing(&'static str), Stale, WrongThread, Phase { operation: &'static str, phase: mod_api::__private::ModPhase }, Unsupported(u32) } impl From<WireError> for ServeError { fn from(e: WireError) -> Self { Self::Decode(e) } } impl std::fmt::Display for ServeError { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, \"{self:?}\") } } impl std::error::Error for ServeError {} fn missing(s: &'static str) -> ServeError { ServeError::Missing(s) }\n#[derive(Default)] pub struct Cx { pub frames: Vec<Frame>, pub held: Slots<Box<dyn std::any::Any + Send>>, pub module: *const crate::WasmMod, pub refused: std::collections::HashMap<String,std::collections::HashSet<String>>, static_strings: std::collections::HashMap<&'static str,u32> } impl Cx { pub fn frame(&self) -> Result<&Frame, ServeError> { self.frames.last().ok_or(ServeError::Missing(\"frame\")) } }\n",
    );
    code.push_str("use crate::{NoAnswer, call_refused};\n");
    code.push_str(&frame(surface)?);
    code.push_str(include_str!("owner_thread.rs"));
    code.push_str(&resources());
    let types = &surface.types;
    code.push_str(
        "pub fn export_id(owner: &str, name: &str) -> Option<u32> { match (owner,name) {\n",
    );
    for (id, m) in surface
        .methods
        .iter()
        .filter(|m| super::exports().contains(&m.owner))
        .enumerate()
    {
        writeln!(code, "({:?},{:?}) => Some({id}),", m.owner, m.name).unwrap();
    }
    for abi in super::prepared::describe(&surface.methods, prepared_hooks)? {
        for entry in abi.entries {
            writeln!(
                code,
                "(\"ItemPart\",{:?}) => Some({}),",
                entry.name, entry.id
            )
            .unwrap();
        }
    }
    code.push_str("_ => None } }\n");
    code.push_str(
        "pub fn import_id(owner: &str, name: &str) -> Option<u32> { match (owner,name) {\n",
    );
    for (id, m) in imports(&surface.methods)
        .iter()
        .copied()
        .chain(held_methods(&surface.methods))
        .enumerate()
    {
        writeln!(code, "({:?},{:?}) => Some({id}),", m.owner, m.name).unwrap();
    }
    code.push_str("_ => None } }\n");
    code.push_str(
        r#"
impl Cx {
    fn check_static_ack(&self, ack: u32) -> Result<(), ServeError> {
        if ack != u32::MAX && ack as usize >= self.static_strings.len() {
            return Err(WireError::Invalid("static string acknowledgement outside table").into());
        }
        Ok(())
    }
    fn put_static(&mut self, out: &mut Writer, text: &'static str, ack: u32) -> Result<(), ServeError> {
        if text.len() > mod_api::wire::MAX_STATIC_STRING_BYTES {
            return Err(WireError::Invalid("static string too long").into());
        }
        let id = if let Some(&id) = self.static_strings.get(text) { id } else {
            if self.static_strings.len() >= mod_api::wire::MAX_STATIC_STRINGS {
                // Admission is an optimization: a full group table cannot impose a
                // stricter limit than the existing per-member static string storage.
                out.put(&2u8);
                out.put(&u32::MAX);
                out.put(text);
                return Ok(());
            }
            let id = self.static_strings.len() as u32;
            self.static_strings.insert(text, id);
            id
        };
        let definition = id != ack;
        out.put(&u8::from(definition));
        out.put(&id);
        if definition { out.put(text); }
        Ok(())
    }
}
"#,
    );
    code.push_str("pub unsafe fn serve(id: u32, cx: *mut Cx, bytes: &[u8], out: &mut Writer) -> Result<(), ServeError> { let mut input = Reader::new(bytes); match id {\n");
    let imports = imports(&surface.methods);
    let mut import_gaps = Vec::new();
    for (id, m) in imports
        .iter()
        .copied()
        .chain(held_methods(&surface.methods))
        .enumerate()
    {
        let is_held = id >= imports.len();
        if (super::fallible_frame_accessor(m).is_none()
            && ((resource(&m.ret) && !supported_resource(&m.ret))
                || (has_frame(&m.ret) && !resource(&m.ret))))
            || m.params
                .iter()
                .any(|(_, t)| !matches!(t, Ty::Dyn(..)) && has_frame(t) && !supported_resource(t))
            || m.params
                .iter()
                .any(|(_, t)| matches!(t, Ty::Ref(_, true)) && !mutable_scalar(t))
            || m.params.iter().any(|(_, t)| {
                (lifetime_def(types, t)
                    && (super::calls::snapshot(types, t, "p").is_none()
                        || lend(types, t, "p").is_none()))
                    || (has_ref_in_data(t) && lend(types, t, "p").is_none())
                    || (!matches!(t, Ty::Dyn(..)) && !resource(t) && held(types, t).is_none())
            })
        {
            import_gaps.push(format!(
                "{}{}::{}",
                if is_held { "held-" } else { "" },
                m.owner,
                m.name
            ));
            continue;
        }
        let mut body = if is_held {
            "let held_id: mod_api::wire::Id = input.take()?;".to_string()
        } else {
            String::new()
        };
        if m.owner == "ConfigFiles" {
            body.push_str("let file_root: u8 = input.take()?; if file_root > 2 { return Err(ServeError::Decode(WireError::Invalid(\"file capability root\"))); }");
        }
        if m.ret == Ty::Str(super::ty::Str::Static) {
            body.push_str("let static_ack: u32 = input.take()?; unsafe { (&*cx).check_static_ack(static_ack)?; }");
        }
        let mut args = Vec::new();
        for (i, (_, ty)) in m.params.iter().enumerate() {
            if let Ty::Dyn(name, mutable) = ty {
                args.push(format!(
                    "unsafe {{ (&*cx).frame()?.{}{}()? }}",
                    snake(name),
                    if *mutable { "_mut" } else { "" }
                ));
                continue;
            }
            writeln!(
                body,
                "let {mutable}p{i}: {} = input.take()?;",
                if resource(ty) {
                    rust_ty(types, ty, false)
                } else {
                    held(types, ty).unwrap()
                },
                mutable = if mutable_scalar(ty) { "mut " } else { "" }
            )
            .unwrap();
            let arg = match ty {
                Ty::List(inner, super::ty::List::Slice)
                    if matches!(**inner, Ty::Str(super::ty::Str::Borrowed)) =>
                {
                    format!("&p{i}.iter().map(String::as_str).collect::<Vec<_>>()")
                }
                _ if resource(ty) => format!("p{i}"),
                _ => lend(types, ty, &format!("p{i}")).unwrap().expr,
            };
            args.push(arg);
        }
        let mut answer: String = if super::fallible_frame_accessor(m).is_some() {
            "out.put_owned(answer.map(|_| ()));".into()
        } else if m.ret == Ty::Str(super::ty::Str::Static) {
            "unsafe { (&mut *cx).put_static(out,answer,static_ack)?; }".into()
        } else if resource(&m.ret) {
            "out.put_owned(answer);".into()
        } else if matches!(&m.ret,Ty::Option(t) if matches!(**t,Ty::Str(super::ty::Str::Borrowed | super::ty::Str::Static) | Ty::Ref(..) | Ty::List(_,super::ty::List::Slice)))
        {
            "out.put_opt(answer);".into()
        } else {
            "out.put_owned(answer);".into()
        };
        for (i, (_, ty)) in m.params.iter().enumerate() {
            if mutable_scalar(ty) {
                write!(answer, "out.put(&p{i});").unwrap();
            }
        }
        let receiver = if is_held {
            format!(
                "unsafe {{ (&*cx).held.get(held_id).map_err(|_| ServeError::Stale)?.downcast_ref::<OwnerThread<std::sync::Arc<dyn {}>>>().ok_or(ServeError::Stale)?.get()?.clone() }}",
                trait_path(m.owner)
            )
        } else if m.owner == "ConfigFiles" {
            "unsafe { (&*cx).frame()?.config_files(file_root)? }".to_string()
        } else {
            format!(
                "unsafe {{ (&*cx).frame()?.{}{}()? }}",
                snake(m.owner),
                if m.receiver == Some(true) { "_mut" } else { "" }
            )
        };
        let accessor = super::REACHED
            .iter()
            .find(|(owner, _, _)| *owner == m.owner)
            .filter(|_| m.owner != "ConfigFiles")
            .and_then(|_| {
                surface.dropped.iter().find(|candidate| {
                    super::frame_accessor(candidate)
                        && matches!(&candidate.ret, Ty::Dyn(name, _) if name == m.owner)
                })
            });
        let mut access = String::new();
        for checked in accessor.into_iter().chain(std::iter::once(m)) {
            if let Some(level) = &checked.access {
                let operation = &checked.name;
                let refusal = match &checked.ret {
                    Ty::Result(_, error) if matches!(&**error, Ty::Named(d) if matches!(types.defs[*d].rust.rsplit("::").next().map(str::trim), Some("LevelReadError" | "LevelWriteError"))) =>
                    {
                        let result = rust_ty(types, &checked.ret, true);
                        let error = rust_ty(types, error, true);
                        format!(
                            "out.put_owned(<{result}>::Err({error}::Unavailable)); return Ok(());"
                        )
                    }
                    _ => format!(
                        "return Err(ServeError::Phase {{ operation: {operation:?}, phase: mod_api::__private::current_phase() }});"
                    ),
                };
                write!(access, "if !mod_api::__private::access_allowed({operation:?}, mod_api::__private::LoadAccess::{level}) {{ {refusal} }}").unwrap();
            }
        }
        // Keep decoded values on the selected method's stack, not in the dispatcher's
        // shared frame: a native import may synchronously re-enter wasm.
        let prepared_check = if matches!(
            m.name.as_str(),
            "attach_item_part" | "attach_item_part_tag"
        ) {
            "let module=unsafe { (&*cx).module.as_ref() }.ok_or(ServeError::Missing(\"calling module\"))?; if let Err(reason)=module.check_prepared_hooks(p2.item_subscribed()) { unsafe { (&*cx).frame()?.registrar_mut()? }.registration_refusal(reason); return Ok(()); }"
        } else {
            ""
        };
        writeln!(code, "{id} => {{ #[inline(never)] unsafe fn invoke(cx: *mut Cx, input: &mut Reader<'_>, out: &mut Writer) -> Result<(), ServeError> {{ {body} if !input.rest().is_empty() {{ return Err(ServeError::Decode(WireError::Tag(u32::MAX))); }} {access} {prepared_check} let receiver = {receiver}; let answer = receiver.{}({}); {answer} Ok(()) }} unsafe {{ invoke(cx, &mut input, out)?; }} }},",m.name,args.join(",")).unwrap();
    }
    code.push_str("_ => return Err(ServeError::Unsupported(id)) } Ok(()) }\n");
    writeln!(
        code,
        "pub const UNSUPPORTED_IMPORTS: &[&str] = &{import_gaps:?};"
    )
    .unwrap();
    let mut prepared_impls = String::new();
    if !prepared_hooks.is_empty() {
        code.push_str("pub struct Prepared<'a> { slot:u32, generation:u32, kinds:Vec<String>, release:Option<Box<dyn FnOnce()+'a>> } impl Drop for Prepared<'_> { fn drop(&mut self) { if let Some(release)=self.release.take() { release(); } } }\n");
    }
    let mut gaps = Vec::new();
    for owner in super::exports() {
        let receiver = if super::mod_receiver(owner).is_some() {
            "WasmMod"
        } else {
            "WasmHandler"
        };
        let mut groups = vec![owner.to_string()];
        for m in surface
            .methods
            .iter()
            .chain(&surface.dropped)
            .filter(|m| m.owner == *owner)
        {
            if let Some(via) = &m.via
                && !groups.contains(via)
            {
                groups.push(via.clone());
            }
        }
        for group in groups {
            writeln!(code, "impl {} for crate::{receiver} {{", trait_path(&group)).unwrap();
            if receiver == "WasmHandler" && group == "ItemPart" && !prepared_hooks.is_empty() {
                code.push_str("fn __prepare_equip(&self,stack:&mod_api::ItemStack,kinds:&[&str],reads:&[mod_api::EquipReadRow])->Result<Option<Box<dyn std::any::Any>>,&'static str> { let instance=self.module.equip_instance(); if crate::prepared::lookup(instance,stack).is_some() { return Ok(None); } let prepared=self.prepare_can_equip_with_reads(stack,kinds,reads).ok_or(\"equipment preparation refused\")?; Ok(Some(Box::new(crate::prepared::activate(instance,stack,prepared)))) }\n");
            }
            if receiver == "WasmMod" && group == "Hooks" {
                code.push_str(
                    "fn listeners(&self) -> &'static [mod_api::Listener] { self.listeners }\n",
                );
            }
            for (id, m) in surface
                .methods
                .iter()
                .filter(|m| super::exports().contains(&m.owner))
                .chain(
                    surface
                        .dropped
                        .iter()
                        .filter(|m| super::exports().contains(&m.owner)),
                )
                .enumerate()
                .filter(|(_, m)| m.owner == *owner && m.via.as_deref().unwrap_or(m.owner) == group)
            {
                let sig = rust_sig(types, m);
                if super::SUBSCRIBED
                    .iter()
                    .any(|(o, n)| *o == m.owner && *n == m.name)
                {
                    let body = if receiver == "WasmMod" {
                        "self.subscribed".to_string()
                    } else {
                        format!("self.subscriptions({owner:?}, {id})")
                    };
                    writeln!(
                        code,
                        "fn {}(&self) -> &'static [Option<mod_api::HookId>] {{ {body} }}",
                        m.name
                    )
                    .unwrap();
                    continue;
                }
                if surface.dropped.iter().any(|d| std::ptr::eq(d, m)) || !dispatchable(types, m) {
                    gaps.push(format!("{}::{}", m.owner, m.name));
                    writeln!(code, "{sig} {{ {} }}", fallback(types, m, None, false)).unwrap();
                    continue;
                }
                let prepared_field = prepared_hooks
                    .iter()
                    .find(|(name, _)| name == &m.name)
                    .map(|(_, field)| field.as_str());
                let prepared_event = if let Some(field) = prepared_field {
                    if m.owner != "ItemPart" || m.name != "can_equip" || field != "stack" {
                        return Err(format!(
                            "unsupported prepared parameter {}::{}::{field}",
                            m.owner, m.name
                        ));
                    }
                    Some(
                        m.params
                            .iter()
                            .enumerate()
                            .find_map(|(i, (_, ty))| {
                                let Ty::Ref(inner, true) = ty else {
                                    return None;
                                };
                                let Ty::Named(d) = &**inner else {
                                    return None;
                                };
                                types.defs[*d]
                                    .borrow_fields()?
                                    .iter()
                                    .any(|f| f.rust == field)
                                    .then_some(i)
                            })
                            .ok_or_else(|| {
                                format!("prepared parameter {field} missing in {}", m.name)
                            })?,
                    )
                } else {
                    None
                };
                let (mut setup, mut put, mut take, mut apply) =
                    (String::new(), String::new(), String::new(), String::new());
                let mut backs = vec!["answer".to_string()];
                for (i, (_, ty)) in m.params.iter().enumerate() {
                    match ty {
                        Ty::Dyn(name, mutable) => {
                            write!(
                                setup,
                                "frame.set_{}{}(p{i});",
                                snake(name),
                                if *mutable { "_mut" } else { "" }
                            )
                            .unwrap();
                        }
                        Ty::Named(d) if types.defs[*d].lifetimes != 0 => {
                            write!(setup, "let mut p{i}=p{i};").unwrap();
                            event_input(
                                types,
                                *d,
                                &format!("p{i}"),
                                &mut setup,
                                &mut put,
                                prepared_field,
                            );
                        }
                        Ty::Ref(inner, mutable) if matches!(&**inner, Ty::Named(d) if types.defs[*d].lifetimes != 0) =>
                        {
                            let Ty::Named(d) = &**inner else {
                                unreachable!()
                            };
                            event_input(
                                types,
                                *d,
                                &format!("p{i}"),
                                &mut setup,
                                &mut put,
                                prepared_field,
                            );
                            let leaves =
                                writeback_fields(types, *d, &format!("p{i}"), *mutable).unwrap();
                            let tick_reply = types.defs[*d]
                                .rust
                                .ends_with("BlockEntityTickEvent")
                                .then(|| {
                                    leaves
                                        .iter()
                                        .find(|leaf| leaf.path.ends_with(".reply"))
                                        .expect("tick reply")
                                        .local
                                        .clone()
                                });
                            for leaf in leaves {
                                let local = format!("b_{}", leaf.local);
                                backs.push(local.clone());
                                if leaf.view
                                    && let Some(reply) = &tick_reply
                                {
                                    write!(take, "let {local}: Option<{}> = if b_{reply} == mod_api::BlockEntityTickReply::Changed {{ Some(r.take()?) }} else {{ None }};", leaf.owned).unwrap();
                                } else {
                                    write!(take, "let {local}: {} = r.take()?;", leaf.owned)
                                        .unwrap();
                                }
                                let path = leaf.path;
                                if leaf.append {
                                    let owner = path.rsplit_once('.').unwrap().0;
                                    write!(take, "let {local} = {owner}.__wire_checked({local})?;")
                                        .unwrap();
                                    write!(apply, "{owner}.__wire_apply({local});").unwrap();
                                } else if leaf.view {
                                    if tick_reply.is_some() {
                                        write!(apply, "if let Some(snapshot) = {local} {{ {path}.__wire_apply(snapshot); }}").unwrap();
                                    } else {
                                        write!(apply, "{path}.__wire_apply({local});").unwrap();
                                    }
                                } else if leaf.optional {
                                    write!(take,"if {path}.is_some() != {local}.is_some() {{ return Err(WireError::Tag(u32::MAX)); }}").unwrap();
                                    write!(apply,"if let (Some(target),Some(value)) = ({path}.as_deref_mut(),{local}) {{ *target=value; }}").unwrap();
                                } else {
                                    write!(
                                        apply,
                                        "{}{path} = {local};",
                                        if leaf.reference { "*" } else { "" }
                                    )
                                    .unwrap();
                                }
                            }
                        }
                        _ => {
                            put.push_str(&if matches!(
                                ty,
                                Ty::Ref(..)
                                    | Ty::Str(super::ty::Str::Borrowed)
                                    | Ty::List(_, super::ty::List::Slice)
                            ) || matches!(ty, Ty::Option(inner) if matches!(
                                **inner,
                                Ty::Ref(_, false)
                                    | Ty::Str(super::ty::Str::Borrowed)
                                    | Ty::List(_, super::ty::List::Slice)
                            )) {
                                self::put(ty, &format!("p{i}"))
                            } else {
                                format!("w.put_owned(p{i});")
                            });
                            if matches!(ty, Ty::Ref(_, true)) {
                                backs.push(format!("b{i}"));
                                write!(take, "let b{i}: {} = r.take()?;", held(types, ty).unwrap())
                                    .unwrap();
                                write!(apply, "*p{i} = b{i};").unwrap();
                            }
                        }
                    }
                }
                let class =
                    super::call_class(types, m).replace("super::Call::", "crate::CallClass::");
                let tuple = format!("({},)", backs.join(","));
                let refusal = if let Some(event) = prepared_event {
                    format!("{{ p{event}.can_equip=false; }}")
                } else {
                    // A hook the catalog isolates (#2414) must fail loudly: the
                    // isolated dispatch unwinds this proxy the same way it
                    // unwinds a panicking native listener, instead of a silent
                    // `Continue` that leaves an earlier replacement standing.
                    let isolate = m.owner == "Hooks" && isolated_hooks.contains(&m.name);
                    fallback(types, m, Some("crate::wasm_call_refusal()"), isolate)
                };
                let mut method = String::new();
                writeln!(method,"{sig} {{ let mut frame = Frame::default(); {setup} match self{module}.call({class}, frame, {id}, {}, |w| {{ {put} }}, |r| {{ let answer: {} = r.take()?; {take} if !r.rest().is_empty() {{ return Err(WireError::Tag(u32::MAX)); }} Ok({tuple}) }}) {{ Some({tuple}) => {{ {apply} answer }}, None => {} }} }}",if receiver=="WasmMod" {"None"} else {"Some(self.index)"},rust_ty(types,&m.ret,true),refusal,module=if receiver=="WasmMod" {""} else {".module"}).unwrap();
                if let Some(event) = prepared_event {
                    let name = &m.name;
                    let arguments = (0..m.params.len())
                        .map(|i| format!("p{i}"))
                        .collect::<Vec<_>>()
                        .join(",");
                    writeln!(code, "{sig} {{ if let Some(prepared)=crate::prepared::lookup(self.module.equip_instance(),p{event}.stack) {{ self.ask_{name}(&prepared,{arguments}); return; }} let Some(prepared)=self.prepare_{name}(p{event}.stack,&[p{event}.kind]) else {{ p{event}.can_equip=false; return; }}; self.ask_{name}(&prepared,{arguments}); }}").unwrap();
                    let ask_sig = sig.replacen(
                        &format!("fn {name}(&self,"),
                        &format!("pub fn ask_{name}(&self,prepared:&Prepared<'_>,"),
                        1,
                    );
                    let slots = super::prepared::SLOTS;
                    let ask = method.replacen(&sig, &ask_sig, 1).replacen("let mut frame = Frame::default();", &format!("let Some(slot_id)={slots:?}.iter().position(|slot| *slot==p{event}.slot).map(|i|i as u32) else {{ p{event}.can_equip=false;return; }}; let Some(kind_id)=prepared.kinds.iter().position(|kind| kind==p{event}.kind).map(|i|i as u32) else {{ p{event}.can_equip=false;return; }}; if !self.module.check(prepared.slot,prepared.generation) {{ p{event}.can_equip=false; return; }} let mut frame = Frame::default();"), 1);
                    writeln!(prepared_impls, "impl crate::WasmHandler {{ pub fn prepare_{name}(&self,stack:&mod_api::ItemStack,kinds:&[&str])->Option<Prepared<'static>> {{ self.prepare_{name}_with_reads(stack,kinds,&[]) }} pub fn prepare_{name}_with_reads(&self,stack:&mod_api::ItemStack,kinds:&[&str],reads:&[mod_api::EquipReadRow])->Option<Prepared<'static>> {{ let mut bytes=Writer::new(); bytes.put(stack); let kinds:Vec<String>=kinds.iter().map(|s|s.to_string()).collect(); bytes.put(&kinds); bytes.put(reads); if bytes.error().is_some() {{ return None; }} let (slot,generation)=self.module.prepare(bytes.bytes())?; let module=self.module.clone(); Some(Prepared {{ slot,generation,kinds,release:Some(Box::new(move || module.release(slot,generation))) }}) }} {ask} }}").unwrap();
                } else {
                    code.push_str(&method);
                }
            }
            code.push_str("}\n");
        }
    }
    code.push_str(&prepared_impls);
    writeln!(code, "pub const UNSUPPORTED_EXPORTS: &[&str] = &{gaps:?};").unwrap();
    // The API is qualified as crate in the shared signature writer, but lives in mod_api here.
    Ok(code
        .replace("crate ::", "mod_api ::")
        .replace("crate::", "mod_api::")
        .replace("mod_api::Wasm", "crate::Wasm")
        .replace("mod_api::CallClass", "crate::CallClass")
        .replace("mod_api::refused", "crate::refused")
        .replace("mod_api::NoAnswer", "crate::NoAnswer")
        .replace("mod_api::wasm_call_refusal", "crate::wasm_call_refusal")
        .replace("mod_api::prepared::", "crate::prepared::")
        .replace(
            "use mod_api::{NoAnswer, call_refused}",
            "use crate::{NoAnswer, call_refused}",
        ))
}

/// Generate the host boundary from the same source surface as the guest.
pub fn generate_host(root: &std::path::Path) -> Result<String, String> {
    // The build script that runs this reads the vocabulary's hook declarations, exports and
    // parts and the native hook sources: ask Cargo to rerun it when any of them changes, from the
    // generator's own list of what it reads (a list in the build script would drift).
    crate::build::track(root, false)?;
    host_and_coverage(root).map(|(code, _)| code)
}

/// The book's wasm boundary coverage page, generated from the same surface and host gap tables
/// as the host boundary: the book's preprocessor renders it at build time.
pub fn generate_coverage(root: &std::path::Path) -> Result<String, String> {
    host_and_coverage(root).map(|(_, report)| report)
}

fn host_and_coverage(root: &std::path::Path) -> Result<(String, String), String> {
    let generated = crate::sources(root, crate::FillMode::Require)?;
    let (providers, provider_sources) = crate::capability_providers::generate(root, &[])?;
    for path in provider_sources {
        crate::rerun_if_changed(&path.display());
    }
    let included = [
        ("mod_api_generated.rs", generated.hooks.as_str()),
        ("mod_api_generated_host.rs", generated.host.as_str()),
        ("mod_api_host_areas.rs", generated.areas.as_str()),
        ("capability_providers_generated.rs", providers.as_str()),
    ];
    let mut surface = super::surface(root, &included)?;
    let count = surface
        .methods
        .iter()
        .filter(|m| super::exports().contains(&m.owner))
        .position(|m| m.owner == "Hooks" && m.name == "subscribed")
        .ok_or("Hooks::subscribed absent")?;
    let catalog = crate::Catalog::load(root)?;
    let prepared_hooks = catalog
        .hooks
        .iter()
        .filter_map(|hook| {
            hook.prepared
                .as_deref()
                .map(|parameter| (hook.name.clone(), parameter.to_owned()))
        })
        .collect::<Vec<_>>();
    let isolated_hooks = catalog
        .hooks
        .iter()
        .filter(|hook| hook.isolate)
        .map(|hook| hook.name.clone())
        .collect::<std::collections::HashSet<_>>();
    let mut code = generate(&mut surface, &prepared_hooks, &isolated_hooks)?;
    let entries = super::prepared::describe(&surface.methods, &prepared_hooks)?
        .into_iter()
        .map(|abi| {
            let entries: [(String, u32); 3] = abi
                .entries
                .into_iter()
                .map(|entry| (entry.name, entry.id))
                .collect::<Vec<_>>()
                .try_into()
                .expect("prepare, ask and release entries");
            (abi.hook, entries)
        })
        .collect::<Vec<_>>();
    writeln!(
        code,
        "pub const PREPARED_EXPORTS: &[(&str, [(&str,u32);3])] = &{entries:?};"
    )
    .unwrap();

    let imports = code
        .lines()
        .find(|l| l.starts_with("pub const UNSUPPORTED_IMPORTS:"))
        .unwrap_or("");
    let exports = code
        .lines()
        .find(|l| l.starts_with("pub const UNSUPPORTED_EXPORTS:"))
        .unwrap_or("");
    let mut report = String::from(
        "# Wire boundary coverage\n\nSurface methods excluding unbound and Not on wasm. Generated from both sides' shared surface and current host gap tables.\n\n| Method | Status | Reason |\n|---|---|---|\n",
    );
    for m in surface.methods.iter().chain(&surface.dropped) {
        if surface
            .excused
            .iter()
            .any(|(o, n)| *o == m.owner && *n == m.name)
        {
            continue;
        }
        let name = format!("{}::{}", m.owner, m.name);
        let dropped = surface.dropped.iter().any(|d| std::ptr::eq(d, m));
        let gap = dropped
            || if super::exports().contains(&m.owner) {
                exports.contains(&format!("{name:?}"))
            } else {
                imports.contains(&format!("{name:?}")) && !super::frame_accessor(m)
            };
        writeln!(
            report,
            "| `{name}` | {} | {} |",
            if gap { "gap" } else { "crosses" },
            if dropped {
                "mentions skipped type"
            } else if gap {
                "borrow/resource shape not yet supported"
            } else if super::frame_accessor(m) {
                "frame accessor; no bytes"
            } else if prepared_hooks.iter().any(|(name, _)| *name == m.name) {
                "prepared owned stack; borrowed scalar candidate calls"
            } else {
                "Wire call"
            }
        )
        .unwrap();
    }
    report.push_str("\n| Library mod plain `pub fn` / `pub static` called by a dependent | crosses | shared dependency rlib, stable crate identity, and retained public exports; production-loader library fixture covers separate library/observer/dependent builds |\n");
    let gaps = report
        .lines()
        .filter(|line| line.contains("| gap |") || line.contains("| OPEN |"))
        .count();
    writeln!(report, "\nRequired gap total: {gaps}.").unwrap();
    report.push_str(include_str!("../../coverage-notes.md"));
    writeln!(code, "pub const SUBSCRIBED: u32 = {count};").unwrap();
    code.push_str("pub fn hook_id(name: &str) -> Option<mod_api::HookId> { match name {\n");
    for (variant, name) in &surface.hooks {
        writeln!(code, "{:?} => Some(mod_api::HookId::{}),", name, variant).unwrap();
    }
    code.push_str("_ => None } } pub fn subscriptions_of(owner: &str, names: Vec<String>) -> &'static [Option<mod_api::HookId>] { names.into_iter().filter_map(|name| { let id = hook_id(&name); if id.is_none() { log::warn!(\"wasm {owner}: unknown hook {name}\"); } id.map(Some) }).collect::<Vec<_>>().leak() }\n");
    for path in surface.types.index.read.take() {
        crate::rerun_if_changed(&path.display());
    }
    Ok((code, report))
}

#[cfg(test)]
mod tests {
    /// The wasm host's build script runs `generate_host`, so every file the generator reads must be
    /// one Cargo is told to watch: a hook signature moved in the vocabulary left the generated code
    /// stale until the build script was touched (#3123).
    #[test]
    #[ignore = "daily"]
    fn host_generation_asks_cargo_to_watch_the_vocabulary_hook_sources() {
        let captured = std::thread::Builder::new()
            .stack_size(64 << 20)
            .spawn(|| {
                let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
                crate::CAPTURED_RERUNS.set(Some(Vec::new()));
                let result = super::generate_host(&root);
                let captured = crate::CAPTURED_RERUNS.take().unwrap();
                result.unwrap();
                captured
            })
            .unwrap()
            .join()
            .unwrap();
        let watched: Vec<String> = captured
            .iter()
            .map(|path| path.replace('\\', "/"))
            .collect();
        for needed in [
            crate::catalog::DECLARATIONS,
            "crates/horizon-mod-vocab/src/exports",
            "crates/horizon-mod-vocab/src/parts",
            "crates/horizon-mod-vocab/src/lib.rs",
            "crates/horizon-mod-vocab/src/host/draw.rs",
            "crates/horizon-mod-vocab/src/exports/generated_host.rs",
            "crates/horizon-client/src/game/native_hooks",
        ] {
            assert!(
                watched.iter().any(|path| path.ends_with(needed)),
                "the host generator reads {needed} but does not ask Cargo to watch it: {watched:#?}"
            );
        }
    }
}
