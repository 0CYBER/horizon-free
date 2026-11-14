use super::*;

pub(super) struct HookSources {
    pub traits: String,
    pub dispatch: String,
    pub render_dispatch: String,
    pub part_traits: BTreeMap<&'static str, String>,
    pub fills: Vec<Fill>,
    pub fire: String,
    pub hook_ids: String,
    pub hook_constants: String,
    /// A `()` constant named after each hook a mod may cancel, which `#[listener(receive_cancelled)]`
    /// names so that the option on any other hook fails to compile.
    pub cancel_hooks: String,
    pub hook_count: usize,
}

/// The most parameters a `native_*` target takes, receiver included (clippy's limit): a hook
/// with more declares its parameters in `hooks/*.rs` and its target takes one `<Name>Call`.
const MAX_TARGET_PARAMETERS: usize = 7;

/// Refuses a flat target that takes more than [`MAX_TARGET_PARAMETERS`].
fn target_parameters(name: &str, native: &str, count: usize, carrier: &str) -> Result<(), String> {
    if count > MAX_TARGET_PARAMETERS {
        return Err(format!(
            "{name}: {native} takes {count} parameters, more than {MAX_TARGET_PARAMETERS}: declare them on the hook in hooks/*.rs so it takes one {carrier}"
        ));
    }
    Ok(())
}

/// Whether an isolated dispatch can roll a listener's writes through a mutable-reference field
/// back to the requested value: the pointee must be a `Copy` primitive or a fixed array of them.
fn rollback_pointee_is_copy(field_ty: &Type) -> bool {
    match field_ty {
        Type::Reference(reference) => rollback_pointee_is_copy(&reference.elem),
        Type::Array(array) => is_primitive_scalar(&array.elem),
        other => is_primitive_scalar(other),
    }
}

pub(super) fn is_render(hook: &Hook) -> bool {
    hook.target.starts_with("mod_render::render::")
}

/// Render emission reads only the catalog and its native raw target signatures.
pub(super) fn render(root: &Path, catalog: &Catalog) -> Result<HookSources, String> {
    emit(
        root,
        catalog.hooks.iter().filter(|hook| is_render(hook)),
        &[],
        &catalog.draw_contexts,
    )
}

pub(super) fn emit<'a>(
    root: &Path,
    hooks: impl Iterator<Item = &'a Hook>,
    methods: &[syn::ItemFn],
    contexts: &[DrawContext],
) -> Result<HookSources, String> {
    let mut hook_ids = String::new();
    let mut hook_constants = String::new();
    let mut cancel_hooks = String::new();
    let mut hook_count = 0;
    let mut traits = String::new();
    let mut dispatch = String::new();
    let mut render_dispatch = String::new();
    let mut part_traits: BTreeMap<&str, String> = BTreeMap::new();
    let mut fills = Vec::new();
    let mut fire = String::new();
    let mut calls = String::new();
    for hook in hooks {
        let name = hook.name.as_str();
        let native = format!("native_{name}");
        let target = hook.target.as_str();
        let is_render = is_render(hook);
        let external = free_function_source(target);
        let in_game = external.is_none();
        let (method, call) = if let Some((krate, path)) = external {
            let Some((module, function)) = path.rsplit_once("::") else {
                return Err(format!("wrong target for {name}"));
            };
            if function != if is_render { name } else { &native } {
                return Err(format!("wrong target for {name}"));
            }
            let path = root
                .join("crates")
                .join(krate)
                .join("src")
                .join(format!("{}.rs", module.replace("::", "/")));
            let source =
                fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let file = syn::parse_file(&source).map_err(|e| e.to_string())?;
            let functions: Vec<_> = file
                .items
                .into_iter()
                .filter_map(|item| match item {
                    Item::Fn(function) => Some(function),
                    _ => None,
                })
                .collect();
            (
                one_function(functions.iter(), function)?.clone(),
                // A hook in the mod API crate, or in a crate above it, calls its dispatch
                // directly; one below it calls the callback the game installs.
                if matches!(krate, "horizon-mod-api" | "horizon-client-render")
                    || super::has_call(
                        &one_function(functions.iter(), function)?.block,
                        &format!("dispatch_{name}"),
                    )
                {
                    format!("dispatch_{name}")
                } else {
                    name.to_owned()
                },
            )
        } else {
            if !target.starts_with("horizon_client::game::")
                || !target.ends_with(&format!("::{native}"))
            {
                return Err(format!("wrong target for {name}"));
            }
            (
                if target.starts_with("horizon_client::game::native_hooks::") {
                    one_function(methods.iter(), &native)?.clone()
                } else {
                    super::named_hooks::resolve(root, target, name)?
                },
                format!("dispatch_{name}"),
            )
        };
        let fill = !is_render && needs_fill(&method, &call, in_game)?;
        let sig = &method.sig;
        let mut native_args = Vec::new();
        let mut receiver_mut = None;
        for arg in &sig.inputs {
            match arg {
                FnArg::Receiver(r) => {
                    receiver_mut = Some(r.mutability.is_some());
                }
                FnArg::Typed(arg) => {
                    let Pat::Ident(pat) = &*arg.pat else {
                        return Err(format!("unsupported argument in {name}"));
                    };
                    native_args.push((pat.ident.to_string(), &*arg.ty));
                }
            }
        }
        // A hook that declares its parameters is called with them as one `<Name>Call`.
        let carrier = format!("{}Call", hook_id(name));
        let declared = hook
            .call
            .iter()
            .map(|(arg, ty)| Ok((arg.clone(), syn::parse_str::<Type>(ty)?)))
            .collect::<Result<Vec<_>, syn::Error>>()
            .map_err(|e| format!("{name}: {e}"))?;
        let mut args: Vec<(String, &Type)> = if declared.is_empty() {
            target_parameters(name, &native, sig.inputs.len(), &carrier)?;
            native_args
        } else {
            let taken_ticks = native_args.iter().position(|(arg, _)| arg == "ticks");
            let others: Vec<_> = native_args
                .iter()
                .filter(|(arg, _)| arg != "ticks")
                .collect();
            let [(call_arg, call_ty)] = others[..] else {
                return Err(format!(
                    "{name}: {native} takes its declared parameters as one `call: {carrier}`"
                ));
            };
            if call_arg != "call" || !call_ty.to_token_stream().to_string().contains(&carrier) {
                return Err(format!(
                    "{name}: {native} takes its declared parameters as one `call: {carrier}`"
                ));
            }
            taken_ticks
                .map(|index| native_args[index].clone())
                .into_iter()
                .chain(declared.iter().map(|(arg, ty)| (arg.clone(), ty)))
                .collect()
        };
        let is_level = host_param::is_level(hook);
        let ticks = is_level
            && args.iter().any(|(name, ty)| {
                name == "ticks" && ty.to_token_stream().to_string() == "& mut Ticks"
            });
        host_param::validate(hook, name, is_level, receiver_mut, ticks)?;
        let context = host_param::context_param(hook, &mut args)?;
        if ticks {
            args.retain(|(name, _)| name != "ticks");
        }
        // Engine-inferred state types become owned API values at the checked crossing.
        // Explicit catalog declarations remain authoritative.
        let inferred_types: Vec<Type> = args
            .iter()
            .map(|(_, ty)| {
                let mut ty = (*ty).clone();
                if declared.is_empty() {
                    use syn::visit_mut::VisitMut;
                    struct ApiState;
                    impl VisitMut for ApiState {
                        fn visit_type_path_mut(&mut self, ty: &mut syn::TypePath) {
                            if ty.path.segments.len() == 1 && ty.path.is_ident("BlockStateId") {
                                ty.path = syn::parse_quote!(BlockState);
                            }
                            syn::visit_mut::visit_type_path_mut(self, ty);
                        }
                    }
                    ApiState.visit_type_mut(&mut ty);
                }
                ty
            })
            .collect();
        args = args
            .into_iter()
            .zip(&inferred_types)
            .map(|((name, _), ty)| (name, ty))
            .collect();
        let carried = args.clone();
        let mut used: HashSet<_> = args.iter().map(|(name, _)| name.clone()).collect();
        let mod_item = fresh_name("mod_hook", &mut used);
        let part_list = fresh_name("parts", &mut used);
        let part_item = fresh_name("part", &mut used);
        let part_table = fresh_name("part_table", &mut used);
        let replacement = fresh_name("replacement", &mut used);
        let receives = fresh_name("receives_cancelled", &mut used);
        let mod_entry = fresh_name("mod_entry", &mut used);
        let entered = fresh_name("_entered", &mut used);
        let batch = fresh_name("batch", &mut used);
        let contract = hook.contract;
        let out = hook.out.as_deref();
        let answered = hook.answered_by_mod;
        let output = if let Some(out) = out {
            let index = args
                .iter()
                .position(|(n, _)| n == out)
                .ok_or_else(|| format!("missing output {out} in {name}"))?;
            Some(args.remove(index))
        } else {
            None
        };
        if hook.stack_count.is_some() {
            crate::catalog::validate_stack_count_output(name, output.as_ref().map(|(_, ty)| *ty))
                .map_err(|error| error.to_string())?;
        }
        let collect_cancel = contract == Contract::Collect
            && hook.cancellable == Some(true)
            && args.iter().any(|(name, ty)| {
                name == "cancel" && ty.to_token_stream().to_string() == "& mut bool"
            });
        let cancel_out = if collect_cancel { Some("cancel") } else { out };
        let result = if collect_cancel {
            " -> Flow".to_owned()
        } else if let Some((_, ty)) = &output {
            let Type::Reference(reference) = ty else {
                return Err(format!("output must be a mutable reference in {name}"));
            };
            if reference.mutability.is_none() {
                return Err(format!("output must be a mutable reference in {name}"));
            }
            match contract {
                Contract::Cancel if reference.elem.to_token_stream().to_string() == "bool" => {
                    " -> Flow".to_owned()
                }
                Contract::Replace if answered => {
                    let answer = answered_type(&reference.elem)
                        .ok_or_else(|| format!("{name}: output must be Option<Answered<T>>"))?;
                    format!(" -> Option<{}>", answer.to_token_stream())
                }
                Contract::Replace => format!(" -> {}", reference.elem.to_token_stream()),
                Contract::Collect if answered && answered_batch(&reference.elem).is_none() => {
                    return Err(format!("{name}: output must be Vec<Answered<T>>"));
                }
                Contract::Collect => String::new(),
                _ => return Err(format!("unexpected output type for {name}")),
            }
        } else {
            String::new()
        };
        let host = context
            .or_else(|| host_param::host_param(hook, receiver_mut, ticks))
            .map(|param| {
                (
                    fresh_name(
                        if hook.context.is_some() {
                            "context"
                        } else if is_level {
                            "level"
                        } else {
                            "host"
                        },
                        &mut used,
                    ),
                    param,
                )
            });
        // A scoped hook's parts: the trait its method goes on, and the lookup
        // of the parts attached to the thing its `key` argument names.
        let scoped = match &hook.scope {
            None => None,
            Some(scope) => {
                let key = scope.key.as_str();
                let (part_trait, table) = match scope.kind {
                    PartKind::Block => ("BlockPart", "block"),
                    PartKind::BlockEntity => ("BlockEntityPart", "block_entity"),
                    PartKind::Item => ("ItemPart", "item"),
                    PartKind::Menu => ("MenuPart", "menu"),
                    PartKind::Fluid => ("FluidPart", "fluid"),
                    PartKind::Entity => ("EntityPart", "entity"),
                };
                if is_render {
                    return Err(format!("render hook {name} cannot be scoped"));
                }
                let (_, ty) = args
                    .iter()
                    .find(|(arg, _)| arg == key)
                    .ok_or_else(|| format!("key {key} is not an argument of {name}"))?;
                let key_type = ty.to_token_stream().to_string().replace("mod_api :: ", "");
                let lookup = match key_type.as_str() {
                    "BlockState" if table == "block" => format!("block_value({key})"),
                    "BlockState" => {
                        format!(
                            "{table}(|| super::content::registry_keys::block_id({key}.block()))"
                        )
                    }
                    "& ItemStack" => {
                        format!("{table}(|| super::content::registry_keys::item_id({key}.item()))")
                    }
                    "BlockEntityTypeKey" if table == "block_entity" => format!(
                        "block_entity_kind(|| super::content::registry_keys::block_entity_type_name({key}))"
                    ),
                    "& str" => format!("{table}(|| Some({key}))"),
                    "BlockEntityRef" if table == "block_entity" => {
                        let host = &host.as_ref().ok_or("block entity handle needs a Host")?.0;
                        format!(
                            "block_entity_kind(|| {key}.kind({host}).and_then(super::content::registry_keys::block_entity_type_name))"
                        )
                    }
                    // An entity's type id need not be its block's: its own table.
                    "& ModBlockEntity" | "& mut ModBlockEntity" => {
                        format!("block_entity_kind(|| Some({key}.id()))")
                    }
                    other => return Err(format!("key {key} of {name} has type {other}")),
                };
                Some((part_trait, lookup))
            }
        };
        let event = fresh_name("event", &mut used);
        let event_type = format!("{}Event", hook_id(name));
        let hook_id = event_type.strip_suffix("Event").unwrap();
        if !is_render {
            hook_ids.push_str(&format!("{hook_id},\n"));
            hook_constants.push_str(&format!(
                "pub const {name}: Option<super::HookId> = Some(super::HookId::{hook_id});\n"
            ));
            hook_count += 1;
        }
        // A mod listener may still be told of a cancelled event, and see and clear its cancel; a
        // part never is.
        let mod_cancel = (contract == Contract::Cancel || collect_cancel) && scoped.is_none();
        if mod_cancel && !is_render {
            cancel_hooks.push_str(&format!("pub const {name}: () = ();\n"));
        }
        let mut trait_params = vec!["&self".to_owned()];
        let mut dispatch_params = Vec::new();
        let mut call_args = Vec::new();
        if let Some((name, param)) = &host {
            trait_params.push(format!("{name}: {}", param.ty));
            let by_side = param.ty == "HostSide" || param.ty == "LevelSide";
            dispatch_params.push(format!(
                "{}{name}: {}",
                if by_side { "mut " } else { "" },
                param.ty
            ));
            call_args.push(if by_side {
                format!("{name}.reborrow()")
            } else if param.ty.starts_with("&mut ") {
                format!("&mut *{name}")
            } else {
                name.clone()
            });
        }

        let mut lifetimes = EventLifetimes::default();
        let mut fields = String::new();
        let mut inits = Vec::new();
        let mut write_back = String::new();
        // The fields an isolated dispatch rolls back to the requested values on a listener's
        // unwind (#2414 ruling), each with whether the event holds it behind a `&mut`.
        let mut rollback: Vec<(String, bool)> = Vec::new();
        let stack_outputs: Vec<_> = args
            .iter()
            .chain(output.as_ref().filter(|_| contract == Contract::Collect))
            .filter_map(|(name, ty)| match ty {
                Type::Reference(reference)
                    if reference.mutability.is_some() && stack_list_output(ty) =>
                {
                    Some((name, &reference.elem))
                }
                _ => None,
            })
            .collect();
        // Keep the last accepted API value, not a new clone for every untouched listener.
        let stack_snapshots = stack_outputs
            .iter()
            .map(|(name, _)| format!("let mut __previous_{name} = {name}.clone();"))
            .collect::<String>();
        let stack_after = stack_outputs.iter().map(|(output, _)| format!(
            "if *{event}.{output} != __previous_{output} {{ match __validate_{output}({event}.{output}) {{ Ok(()) => __previous_{output} = {event}.{output}.clone(), Err(error) => {{ *{event}.{output} = __previous_{output}.clone(); log::warn!(\"mod {{}} hook {name} output {output} refused: {{error:?}}\", super::content::crash::current_native_mod().unwrap_or(\"unknown\")); }} }} }}"
        )).collect::<String>();
        let collected = output.as_ref().filter(|_| contract == Contract::Collect);
        // A draw surface reaches mods as its context's trait object, which the native call
        // builds from `Game` and the surface.
        let mut surface_calls = std::collections::HashMap::new();
        for (name, ty) in args.iter().chain(collected) {
            let drawn = DrawContext::of(contexts, ty);
            if let Some((context, mutable)) = drawn {
                if !mutable || receiver_mut != Some(false) {
                    return Err(format!(
                        "{}: a drawing hook takes &self and its surface as &mut",
                        hook.name
                    ));
                }
                surface_calls.insert(
                    name.as_str(),
                    format!(
                        "&mut crate::game::host::draw::{} {{ game: self, surface: {name} }}",
                        context.game_struct()
                    ),
                );
            }
            let drawn: Option<Type> = drawn.map(|(context, _)| {
                syn::parse_str(&format!("&mut dyn {}", context.trait_name)).expect("a trait name")
            });
            let ty = drawn.as_ref().unwrap_or(ty);
            let mut owned_ty = ty.clone();
            struct OwnedLootContext;
            impl syn::visit_mut::VisitMut for OwnedLootContext {
                fn visit_type_path_mut(&mut self, path: &mut syn::TypePath) {
                    if path.path.segments.len() == 1 && path.path.segments[0].ident == "LootContext"
                    {
                        path.path.segments[0].arguments = syn::PathArguments::None;
                    } else {
                        syn::visit_mut::visit_type_path_mut(self, path);
                    }
                }
            }
            syn::visit_mut::VisitMut::visit_type_mut(&mut OwnedLootContext, &mut owned_ty);
            let ty = &owned_ty;
            dispatch_params.push(format!("{name}: {}", ty.to_token_stream()));
            if collect_cancel && name == "cancel" {
                continue;
            }
            let mutable_entity = ty.to_token_stream().to_string() == "& mut ModBlockEntity";
            // An answered collect hook's mods each push plain entries into their own batch;
            // dispatch, not the mod, names the mod on each.
            let batch_entry =
                (answered && contract == Contract::Collect && Some(name.as_str()) == out)
                    .then(|| match ty {
                        Type::Reference(reference) => answered_batch(&reference.elem),
                        _ => None,
                    })
                    .flatten();
            let field_ty = if mutable_entity {
                syn::parse_quote!(ModBlockEntityView<'_>)
            } else if let Some(entry) = batch_entry {
                syn::parse_quote!(&mut Vec<#entry>)
            } else {
                ty.clone()
            };
            let (field, scalar) = lifetimes.field(&field_ty);
            if scalar {
                rollback.push((name.clone(), false));
            } else if matches!(&field_ty, Type::Reference(reference) if reference.mutability.is_some())
            {
                if hook.isolate && !rollback_pointee_is_copy(&field_ty) {
                    return Err(format!(
                        "{}: isolate rolls a listener's writes back to the requested values; \
                         its mutable argument {name} must be a Copy primitive or a fixed array \
                         of them",
                        hook.name
                    ));
                }
                rollback.push((name.clone(), true));
            }
            let visibility = if event_type == "ApplyEffectTickEvent" && name == "changes" {
                "pub(super)"
            } else {
                "pub"
            };
            fields.push_str(&format!(
                "{visibility} {name}: {},\n",
                field.to_token_stream()
            ));
            if scalar {
                inits.push(format!("{name}: *{name}"));
                write_back.push_str(&format!("*{name} = {event}.{name};"));
            } else if mutable_entity {
                inits.push(format!("{name}: ModBlockEntityView::new({name})"));
            } else if batch_entry.is_some() {
                inits.push(format!("{name}: &mut {batch}"));
            } else {
                inits.push(name.clone());
            }
        }
        if mod_cancel {
            if args.iter().any(|(arg, _)| arg == "cancelled") {
                return Err(format!(
                    "{name}: a cancel hook's event has its own `cancelled` field"
                ));
            }
            fields.push_str("/// Whether the event is cancelled: by the game or an earlier listener, or by this one\n/// setting it. Only a listener that receives cancelled events (`#[listener(receive_cancelled)]`)\n/// sees `true`, and setting it back to `false` lets the game's behaviour run.\npub cancelled: bool,\n");
            inits.push(format!("cancelled: *{}", cancel_out.unwrap()));
        }
        if let Some((name, ty)) = output.as_ref().filter(|_| contract != Contract::Collect) {
            dispatch_params.push(format!("{name}: {}", ty.to_token_stream()));
        }
        if !declared.is_empty() {
            dispatch_params.truncate(usize::from(host.is_some()));
            let (fields, generics) = carrier_fields(&carried);
            dispatch_params.push(format!(
                "call: calls::{carrier}{}",
                if generics.is_empty() { "" } else { "<'_>" }
            ));
            calls.push_str(&format!(
                "/// What the game passes `dispatch_{name}`: the hook's declared parameters.\n#[doc(hidden)]\n#[allow(missing_docs)]\n// no wire: internal call carrier, not an owned boundary value.\npub struct {carrier}{generics} {{\n{fields}}}\n"
            ));
        }
        for (name, ty) in &stack_outputs {
            dispatch_params.push(format!(
                "__validate_{name}: &mut dyn FnMut(&{ty}) -> Result<(), StackWriteError>",
                ty = ty.to_token_stream()
            ));
        }
        let (generics, elided) = match (lifetimes.a, lifetimes.b) {
            (_, true) => ("<'a, 'b>", "<'_, '_>"),
            (true, false) => ("<'a>", "<'_>"),
            (false, false) => ("", ""),
        };
        let wire_form = if generics.is_empty() {
            "#[derive(horizon_mod_api_macros::Wire)]"
        } else {
            "// no wire: borrowed event crosses as call arguments."
        };
        if is_render {
            render_dispatch.push_str(&format!(
                "#[allow(missing_docs)]\n{wire_form}\npub struct {event_type}{generics} {{\n{fields}}}\n"
            ));
            continue;
        }
        trait_params.push(format!("{event}: &mut {event_type}{elided}"));
        call_args.push(format!("&mut {event}"));
        let unused = trait_params[1..]
            .iter()
            .map(|param| format!("let _ = {};", param.split(':').next().unwrap()))
            .collect::<String>();
        let default = match contract {
            _ if collect_cancel => "Flow::Continue",
            Contract::Cancel => "Flow::Continue",
            Contract::Replace => "None",
            _ => "",
        };
        // Reuse the exact host and drawing conversions for generated game-side calls.
        if in_game {
            // A scoped read hook can run against either the game or a borrowed
            // tick view. Its native receiver carries the selected host.
            let scoped_read_host = target.contains("::NativeHookHost<");
            if scoped_read_host
                && (hook.host != Some(HostAccess::Read) || receiver_mut != Some(false))
            {
                return Err(format!(
                    "{name}: NativeHookHost requires a read-only host and receiver"
                ));
            }
            let passed = host
                .as_ref()
                .map(|(_, param)| {
                    if scoped_read_host {
                        "self.host".to_owned()
                    } else if param.call == "&*self" && receiver_mut == Some(false) {
                        "self".to_owned()
                    } else {
                        param.call.to_owned()
                    }
                })
                .into_iter()
                .chain(if declared.is_empty() {
                    args.iter()
                        .chain(output.as_ref())
                        .map(|(n, _)| {
                            surface_calls
                                .get(n.as_str())
                                .cloned()
                                .unwrap_or_else(|| n.clone())
                        })
                        .collect::<Vec<_>>()
                } else {
                    vec!["call".to_owned()]
                })
                .collect::<Vec<_>>()
                .join(", ");
            let mut signature = sig.clone();
            signature.ident = syn::Ident::new(&format!("fire_{name}"), signature.ident.span());
            let target_path: syn::Path = syn::parse_str(target)
                .map_err(|error| format!("invalid target for {name}: {error}"))?;
            let receiver = &target_path.segments[target_path.segments.len() - 2].ident;
            let guarded_carrier = !declared.is_empty()
                && (hook.stack_count.is_some()
                    || carried.iter().any(|(_, ty)| {
                        ty.to_token_stream()
                            .to_string()
                            .split(|c: char| !c.is_alphanumeric())
                            .any(|word| {
                                matches!(word, "BlockState" | "LandingWrite" | "InsideWrite")
                            })
                    }));
            if guarded_carrier {
                let engine: Vec<_> = carried
                    .iter()
                    .map(|(name, ty)| {
                        let mut ty = (*ty).clone();
                        struct EngineTypes;
                        impl syn::visit_mut::VisitMut for EngineTypes {
                            fn visit_type_mut(&mut self, ty: &mut Type) {
                                match ty.to_token_stream().to_string().as_str() {
                                    "BlockState" => {
                                        *ty =
                                            syn::parse_quote!(horizon_world::registry::BlockStateId)
                                    }
                                    "LandingWrite" => {
                                        *ty = syn::parse_quote!(horizon_world::level::LandingWrite)
                                    }
                                    "InsideWrite" => {
                                        *ty = syn::parse_quote!(horizon_world::level::InsideWrite)
                                    }
                                    _ => syn::visit_mut::visit_type_mut(self, ty),
                                }
                            }
                        }
                        syn::visit_mut::VisitMut::visit_type_mut(&mut EngineTypes, &mut ty);
                        (name.clone(), ty)
                    })
                    .collect();
                let refs: Vec<_> = engine.iter().map(|(name, ty)| (name.clone(), ty)).collect();
                let (fields, generics) = carrier_fields(&refs);
                let visibility = method.vis.to_token_stream();
                fire.push_str(&format!(
                    "{visibility} struct {carrier}{generics} {{ {fields} }}\n"
                ));
                for arg in &mut signature.inputs {
                    if let FnArg::Typed(arg) = arg
                        && arg.pat.to_token_stream().to_string() == "call"
                    {
                        *arg.ty = syn::parse_str(&format!(
                            "{carrier}{}",
                            if generics.is_empty() { "" } else { "<'_>" }
                        ))
                        .unwrap();
                    }
                }
            }
            let crossing = if declared.is_empty() || guarded_carrier {
                stack_crossing_routed(
                    &args,
                    output.as_ref(),
                    contract == Contract::Collect,
                    hook_id,
                    scoped.is_some(),
                    hook.scope.as_ref().and_then(|scope| match scope.kind {
                        PartKind::Block => Some(scope.key.as_str()),
                        _ => None,
                    }),
                    &surface_calls,
                    match hook.stack_count {
                        Some(crate::catalog::StackCount::Trim) => hook.out.as_deref(),
                        None => None,
                    },
                )
            } else {
                None
            };
            let body = if let Some(crossing) = crossing {
                let passed = host
                    .as_ref()
                    .map(|(_, param)| {
                        if scoped_read_host {
                            "self.host".to_owned()
                        } else if param.call == "&*self" && receiver_mut == Some(false) {
                            "self".to_owned()
                        } else {
                            param.call.to_owned()
                        }
                    })
                    .into_iter()
                    .chain(crossing.arguments.iter().cloned().map(|argument| {
                        surface_calls
                            .get(argument.as_str())
                            .cloned()
                            .unwrap_or(argument)
                    }))
                    .chain(crossing.validators.iter().cloned())
                    .collect::<Vec<_>>()
                    .join(", ");
                if guarded_carrier {
                    let names = carried
                        .iter()
                        .map(|(name, _)| name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    let fields = args
                        .iter()
                        .chain(output.as_ref())
                        .zip(&crossing.arguments)
                        .map(|((name, _), value)| format!("{name}: {value}"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    let host = host
                        .as_ref()
                        .map(|(_, param)| format!("{}, ", param.call))
                        .unwrap_or_default();
                    format!(
                        "let {carrier} {{ {names} }} = call; {} mod_api::{call}({host}mod_api::__private::{carrier} {{ {fields} }}{}); {}",
                        crossing.before,
                        crossing
                            .validators
                            .iter()
                            .map(|value| format!(", {value}"))
                            .collect::<String>(),
                        crossing.after
                    )
                } else {
                    format!(
                        "{} mod_api::{call}({passed}); {}",
                        crossing.before, crossing.after
                    )
                }
            } else {
                format!("mod_api::{call}({passed});")
            };
            fire.push_str(&format!(
                "impl {receiver}<'_> {{ #[inline] pub(super) {signature} {{ {body} }} }}\n",
                signature = signature.to_token_stream()
            ));
            if fill {
                let passed = sig
                    .inputs
                    .iter()
                    .filter_map(|arg| match arg {
                        FnArg::Typed(arg) => Some(arg.pat.to_token_stream().to_string()),
                        FnArg::Receiver(_) => None,
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                fills.push(Fill {
                    native: native.clone(),
                    call: format!("self.fire_{name}({passed});"),
                });
            }
        }
        let invoke = call_args.join(", ");
        let trait_params = trait_params.join(", ");
        let dispatch_params = dispatch_params.join(", ");
        let trait_name = "Hooks";
        let mut method_source = String::new();
        for line in hook.doc.trim().lines() {
            method_source.push_str(&format!(
                "///{}{line}\n",
                if line.is_empty() { "" } else { " " }
            ));
        }
        let side = match hook.side {
            Side::Server => "server",
            Side::Client => "client",
            Side::Both => "both",
        };
        method_source.push_str(&format!("/// Side: {side}.\n"));
        // A scoped hook is its part's method alone: a global moment beside it is its own hook.
        let callee = if scoped.is_some() { "part" } else { "mod" };
        match (contract, out) {
            (Contract::Cancel, Some(out)) if mod_cancel => method_source.push_str(&format!(
                "///\n/// Return `Flow::Cancel` to take it: this sets `{out}`, and later mods are skipped unless they\n/// listen with `#[listener(receive_cancelled)]`; those see `{event}.cancelled` and may set it back to\n/// `false`.\n"
            )),
            (Contract::Cancel, Some(out)) => method_source.push_str(&format!(
                "///\n/// Return `Flow::Cancel` to take it: this sets `{out}`, and later {callee}s are skipped.\n"
            )),
            (Contract::Replace, Some(out)) => method_source.push_str(&format!(
                "///\n/// Return `Some` to set `{out}`; the first {callee} returning `Some` wins.\n"
            )),
            (Contract::Collect, Some(out)) if matches!(name, "blocks" | "items") => method_source.push_str(&format!(
                "///\n/// Each mod receives its own empty `{event}.{out}` batch. A bare name, in an identifier or\n/// in `block_item`, `pick_item`, `wall_block`, `base_block`, `crafting_remainder` or `Item::block`,\n/// is that mod's own; declared identifiers must belong to\n/// that mod's manifest-owned namespaces, and unowned identifiers refuse registration.\n"
            )),
            (Contract::Collect, Some(out)) if answered => method_source.push_str(&format!(
                "///\n/// Each mod receives its own empty `{event}.{out}` batch; every entry it adds is recorded\n/// as that mod's.\n"
            )),
            (Contract::Collect, Some(out)) => method_source.push_str(&format!(
                "///\n/// Every {callee} gets `{event}.{out}` in order and may add, change or remove {out}.\n"
            )),
            _ => {}
        }
        method_source.push_str(&format!(
            "fn {name}({trait_params}){result} {{ {unused} {default} }}\n"
        ));
        match &scoped {
            Some((part_trait, _)) => part_traits
                .entry(*part_trait)
                .or_default()
                .push_str(&method_source),
            None => traits.push_str(&method_source),
        }
        let trait_name = scoped
            .as_ref()
            .map_or(trait_name, |(part_trait, _)| part_trait);
        dispatch.push_str(&format!(
            "/// What [`{trait_name}::{name}`] is told. Each field is that hook's argument of the same name, described in the hook's doc.\n#[allow(missing_docs)]\n{wire_form}\npub struct {event_type}{generics} {{\n{fields}}}\n"
        ));
        let build = format!(
            "{stack_snapshots}{}let {}{event} = {event_type} {{ {} }};",
            if answered && contract == Contract::Collect {
                format!("let mut {batch} = Vec::new(); ")
            } else {
                String::new()
            },
            if matches!(name, "blocks" | "items") {
                ""
            } else {
                "mut "
            },
            inits.join(", "),
        );
        let build = if name == "block_entity_tick" {
            format!("{build} let mut tick_changed = false;")
        } else {
            build
        };
        // A trait mod's call is marked with the mod for a panic to name.
        let each = match &hook.owner {
            Some(owner) => format!("super::entered_owned(HookId::{hook_id}, \"{name}\", {owner})"),
            None if answered || hook.collect_validator.is_some() => {
                format!("super::entered_with_id(HookId::{hook_id}, \"{name}\")")
            }
            None => format!("super::entered(HookId::{hook_id}, \"{name}\")"),
        };
        // One callee's call: a mod's, or a scoped hook's part's.
        let call = |callee: &str, exit: &str| {
            // Early exits must validate while the listener still owns the scope.
            let exit = format!("{stack_after} {exit}");
            let invocation = match contract {
                _ if name == "block_entity_tick" => format!(
                    "*{event}.reply = BlockEntityTickReply::Changed; {event}.entity.__begin_tick_listener(); {callee}.{name}({invoke}); let listener_reply = *{event}.reply; {event}.entity.__finish_tick_listener(listener_reply); tick_changed |= listener_reply == BlockEntityTickReply::Changed;"
                ),
                Contract::Cancel => format!(
                    "if matches!({callee}.{name}({invoke}), Flow::Cancel) {{ *{o} = true; {exit}; }}",
                    o = out.unwrap()
                ),
                Contract::Replace if answered => format!(
                    "if let Some({replacement}) = {callee}.{name}({invoke}) {{ *{o} = Some(Answered {{ answer: {replacement}, mod_id }}); {exit}; }}",
                    o = out.unwrap()
                ),
                Contract::Replace => format!(
                    "if let Some({replacement}) = {callee}.{name}({invoke}) {{ *{o} = Some({replacement}); {exit}; }}",
                    o = out.unwrap()
                ),
                Contract::Collect if answered => format!(
                    "{callee}.{name}({invoke}); {o}.extend({event}.{o}.drain(..).map(|answer| Answered {{ answer, mod_id }}));",
                    o = out.unwrap()
                ),
                Contract::Collect if hook.collect_validator.is_some() => {
                    let validator = hook.collect_validator.as_ref().unwrap();
                    let output = out.unwrap();
                    format!(
                        "let previous = {event}.{output}.clone(); {callee}.{name}({invoke}); {validator}(mod_id, &previous, {event}.{output});"
                    )
                }
                _ => format!("{callee}.{name}({invoke});"),
            };
            format!("{invocation} {stack_after}")
        };
        let binding = if answered || hook.collect_validator.is_some() {
            format!("(mod_id, {mod_item})")
        } else {
            mod_item.clone()
        };
        // The #2414 ruling an isolated cancel hook carries: a listener that unwinds keeps the
        // **requested** outputs — not one an earlier listener had proposed — is skipped by this
        // hook for the session, and no later listener runs on this invocation.
        let (isolate_snapshot, isolate_restore) = if hook.isolate {
            let snapshot = rollback
                .iter()
                .map(|(name, by_reference)| {
                    format!(
                        "let __requested_{name} = {}{event}.{name}; ",
                        if *by_reference { "*" } else { "" }
                    )
                })
                .chain(std::iter::once(format!(
                    "let __requested_cancelled = {event}.cancelled; "
                )))
                .collect::<String>();
            let restore = rollback
                .iter()
                .map(|(name, by_reference)| {
                    format!(
                        "{}{event}.{name} = __requested_{name}; ",
                        if *by_reference { "*" } else { "" }
                    )
                })
                .chain(std::iter::once(format!(
                    "{event}.cancelled = __requested_cancelled; "
                )))
                .collect::<String>();
            (snapshot, restore)
        } else {
            (String::new(), String::new())
        };
        // A listener's stack-list commit follows the cancel statement, separated by `;` so the
        // generated `if` is never read as the start of an `else if`.
        let cancel_step = {
            let cancel = format!(
                "if matches!({mod_item}.{name}({invoke}), Flow::Cancel) {{ {event}.cancelled = true; }}"
            );
            if stack_after.is_empty() {
                cancel
            } else {
                format!("{cancel}; {stack_after}")
            }
        };
        let (lookup, calls) = match &scoped {
            None if matches!(name, "blocks" | "items") => (
                String::new(),
                format!(
                    "for (mod_id, {mod_item}) in super::entered_with_id(HookId::{hook_id}, \"{name}\") {{ let mut local = Vec::new(); {{ let mut {event} = {event_type} {{ {name}: &mut local }}; {mod_item}.{name}(context, &mut {event}); }} for declaration in &mut local {{ if let Err(reason) = super::parts::own_declaration(mod_id, declaration) {{ context.refuse_declaration(reason); }} }} {event}.{name}.extend(local); }}"
                ),
            ),
            None if name == "setting_changed" => (
                String::new(),
                format!(
                    "{{ let mut {entered} = super::runtime::ListenerScope::new(); for (_, owner) in super::listening(HookId::{hook_id}) {{ if owner.id != {event}.mod_id {{ continue; }} let {mod_item} = {entered}.enter(owner, \"{name}\"); {} }} }}",
                    call(&mod_item, "break"),
                ),
            ),
            None if mod_cancel && hook.isolate => (
                String::new(),
                format!(
                    "{{ {isolate_snapshot} let mut {entered} = super::runtime::ListenerScope::new(); for ({receives}, {mod_entry}) in super::listening(HookId::{hook_id}) {{ if super::runtime::poisoned(HookId::{hook_id}, {mod_entry}.id) || ({event}.cancelled && !{receives}) {{ continue; }} let {mod_item} = {entered}.enter({mod_entry}, \"{name}\"); match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {mod_item}.{name}({invoke}))) {{ Ok(Flow::Cancel) => {{ {event}.cancelled = true; }} Ok(_) => {{}} Err(_unwound) => {{ {isolate_restore} super::runtime::poison(HookId::{hook_id}, {mod_entry}.id); *{o} = {event}.cancelled; return; }} }} }} }} *{o} = {event}.cancelled;",
                    o = out.unwrap()
                ),
            ),
            None if mod_cancel => (
                String::new(),
                format!(
                    "{{ let mut {entered} = super::runtime::ListenerScope::new(); for ({receives}, {mod_entry}) in super::listening(HookId::{hook_id}) {{ if {event}.cancelled && !{receives} {{ continue; }} let {mod_item} = {entered}.enter({mod_entry}, \"{name}\"); {cancel_step} }} }} *{o} = {event}.cancelled;",
                    o = cancel_out.unwrap()
                ),
            ),
            None => (
                String::new(),
                format!("for {binding} in {each} {{ {} }}", call(&mod_item, "break")),
            ),
            Some((_, lookup)) => (
                // The parts are looked up first: the key may be moved into the event. A worldgen
                // Part method runs on the worker threads too: it reads the server's published table.
                format!(
                    "\nif !super::has_part_hook(HookId::{hook_id}) {{ return; }} let {part_table} = {}; let {part_list} = {part_table}.for_hook(HookId::{hook_id}).map_or(&[][..], |table| table.{lookup}); if {part_list}.is_empty() {{ return; }}",
                    if hook.context == Some(PhaseContext::Worldgen) {
                        "super::worldgen_parts()".to_owned()
                    } else {
                        format!("super::parts(HookId::{hook_id}, \"{name}\")")
                    }
                ),
                format!(
                    "for {part_item} in super::entered_parts({part_list}, \"{name}\") {{ {} }}",
                    call(&part_item, "break"),
                ),
            ),
        };
        let write_back = if name == "block_entity_tick" {
            format!(
                "*{event}.reply = if tick_changed {{ BlockEntityTickReply::Changed }} else {{ BlockEntityTickReply::Unchanged }}; {write_back}"
            )
        } else {
            write_back
        };
        let body = match contract {
            _ if mod_cancel => format!(
                "if *{o} && !super::receives_cancelled(HookId::{hook_id}) {{ return; }} {build} {calls} {write_back}",
                o = cancel_out.unwrap()
            ),
            Contract::Cancel => format!(
                "if *{o} {{ return; }} {lookup} {build} {calls} {write_back}",
                o = out.unwrap()
            ),
            Contract::Replace => format!(
                "if {o}.is_some() {{ return; }} {lookup} {build} {calls} {write_back}",
                o = out.unwrap()
            ),
            _ => format!("{lookup} {build} {calls} {write_back}"),
        };
        let early_return = if scoped.is_some() {
            String::new()
        } else {
            format!("if !super::listened(HookId::{hook_id}) {{ return; }}")
        };
        let unpack = if declared.is_empty() {
            String::new()
        } else {
            let names: Vec<_> = carried.iter().map(|(arg, _)| arg.as_str()).collect();
            format!("let calls::{carrier} {{ {} }} = call;\n", names.join(", "))
        };
        dispatch.push_str(&format!(
            "#[doc(hidden)] pub fn dispatch_{name}({dispatch_params}) {{ {unpack}{early_return}\n{body} }}\n"
        ));
    }
    if !calls.is_empty() {
        dispatch.push_str(&format!(
            "#[doc(hidden)]\n#[allow(missing_docs)]\npub mod calls {{\nuse super::*;\n{calls}}}\n"
        ));
    }
    Ok(HookSources {
        traits,
        dispatch,
        render_dispatch,
        part_traits,
        fills,
        fire,
        hook_ids,
        hook_constants,
        cancel_hooks,
        hook_count,
    })
}

/// A carrier's fields, each type with its elided lifetimes named `'a`, and its generics.
struct StackCrossing {
    before: String,
    arguments: Vec<String>,
    validators: Vec<String>,
    after: String,
}

/// Declared owned-value arguments select this guarded conversion seam. Event records
/// are built explicitly at their game call sites, not inferred from game bodies.
#[cfg(test)]
fn stack_crossing(
    args: &[(String, &Type)],
    output: Option<&(String, &Type)>,
    hook_id: &str,
    scoped: bool,
    surfaces: &std::collections::HashMap<&str, String>,
) -> Option<StackCrossing> {
    stack_crossing_routed(args, output, false, hook_id, scoped, None, surfaces, None)
}

fn stack_crossing_routed(
    args: &[(String, &Type)],
    output: Option<&(String, &Type)>,
    listener_output: bool,
    hook_id: &str,
    scoped: bool,
    block_scope: Option<&str>,
    surfaces: &std::collections::HashMap<&str, String>,
    trim_count_output: Option<&str>,
) -> Option<StackCrossing> {
    const STACK_VALUES: &[&str] = &[
        "ItemStack",
        "ItemCost",
        "MerchantOffer",
        "Suggestion",
        "GameRuleValue",
        "PlacementContext",
        "LootStack",
        "LootContext",
        "ParticleOptions",
    ];
    const BLOCK_VALUES: &[&str] = &["BlockState", "LandingWrite", "InsideWrite"];
    const INFALLIBLE_VALUES: &[&str] = &["GuiDraw", "Text"];
    let parameters = args.iter().chain(output).collect::<Vec<_>>();
    if !parameters.iter().any(|(_, ty)| {
        contains_boundary(ty, STACK_VALUES)
            || contains_boundary(ty, INFALLIBLE_VALUES)
            || contains_boundary(ty, BLOCK_VALUES)
    }) {
        return None;
    }
    let guard = if scoped { "has_part_hook" } else { "listened" };
    let mut before =
        format!("if !mod_api::__private::{guard}(mod_api::HookId::{hook_id}) {{ return; }} ");
    if let Some(key) = block_scope {
        before.push_str(&format!("if !crate::game::api_values::block_state::block_listens(mod_api::HookId::{hook_id}, {key}) {{ return; }} "));
    }
    before.push_str("use crate::game::api_values::TryIntoApi; ");
    if parameters.iter().any(|(name, ty)| {
        trim_count_output != Some(name.as_str())
            && (contains_boundary(ty, STACK_VALUES)
                || contains_boundary(ty, INFALLIBLE_VALUES)
                || contains_boundary(ty, BLOCK_VALUES))
            && matches!(ty, Type::Reference(r) if r.mutability.is_some())
    }) {
        before.push_str("use crate::game::api_values::TryIntoEngine; ");
    }
    if trim_count_output.is_some() {
        before.push_str("use crate::game::api_values::TryIntoEnginePreservingCount; ");
    }
    if parameters.iter().any(|(name, ty)| {
        contains_boundary(ty, STACK_VALUES)
            && !contains_boundary(ty, &["LootStack", "ParticleOptions"])
            && (!stack_list_output(ty)
                || (!listener_output && output.is_some_and(|(output, _)| output == name)))
            && matches!(ty, Type::Reference(r) if r.mutability.is_some())
    }) {
        before.push_str("use crate::game::api_values::ItemBoundary; ");
    }
    if args
        .iter()
        .chain(output.filter(|_| listener_output))
        .any(|(_, ty)| {
            stack_list_output(ty)
                && matches!(ty, Type::Reference(reference) if reference.mutability.is_some())
        })
    {
        before.push_str("let __boundary_items = self.stack_boundary_items(); ");
    }
    let mut arguments = Vec::new();
    let mut stack_validators = Vec::new();
    let mut validations = String::new();
    let mut writes = String::new();
    for (name, ty) in parameters {
        let block = contains_boundary(ty, BLOCK_VALUES);
        let stack = contains_boundary(ty, STACK_VALUES);
        let infallible = contains_boundary(ty, INFALLIBLE_VALUES);
        let mutable = matches!(ty, Type::Reference(r) if r.mutability.is_some());
        if surfaces.contains_key(name.as_str())
            || matches!(ty, Type::Reference(r) if matches!(&*r.elem, Type::TraitObject(_)))
            || (!stack && !block && !infallible && !mutable)
        {
            arguments.push(name.clone());
            continue;
        }
        let local = format!("__api_{name}");
        let value = if infallible || stack || block {
            format!("match {name}.try_into_api() {{ Ok(value) => value, Err(_) => return }}")
        } else {
            format!("(*{name}).to_owned()")
        };
        before.push_str(&format!(
            "let {mutability}{local} = {value}; ",
            mutability = if mutable { "mut " } else { "" }
        ));
        arguments.push(match ty {
            Type::Reference(_) => format!("&{}{local}", if mutable { "mut " } else { "" }),
            Type::Path(_)
                if generic_of(ty, "Option").is_some_and(|ty| matches!(ty, Type::Reference(_))) =>
            {
                format!("{local}.as_ref()")
            }
            _ => local.clone(),
        });
        if mutable {
            // Owned stack records have value equality. Other boundary records retain
            // their existing validation path until they provide comparable snapshots.
            let comparable = contains_boundary(ty, &["ItemStack"]);
            let per_listener = stack_list_output(ty)
                && (listener_output || !output.is_some_and(|(output, _)| output == name));
            if comparable {
                before.push_str(&format!("let __original_{name} = {local}.clone(); "));
            }
            if per_listener {
                // Conversion belongs to the listener, not to the final event.
                // Keep the converted candidate so committing does not convert twice.
                let Type::Reference(reference) = ty else {
                    unreachable!()
                };
                let api_type = reference
                    .elem
                    .to_token_stream()
                    .to_string()
                    .replace("ItemStack", "mod_api::ItemStack");
                let conversion = if trim_count_output == Some(name.as_str()) {
                    "value.try_into_engine_preserving_count(&**items)?"
                } else {
                    "value.try_into_engine(&**items)?"
                };
                before.push_str(&format!(
                    "let mut __accepted_{name} = None; let mut __validate_{name} = |value: &{api_type}| -> Result<(), mod_api::StackWriteError> {{ let items = __boundary_items.as_ref().ok_or(mod_api::StackWriteError::Unavailable)?; let candidate = {conversion}; __accepted_{name} = Some(candidate); Ok(()) }}; "
                ));
                stack_validators.push(format!("&mut __validate_{name}"));
                writes.push_str(&format!(
                    "if {local} != __original_{name} {{ if let Some(candidate) = __accepted_{name} {{ *{name} = candidate; }} }} "
                ));
                continue;
            }
            if comparable {
                validations.push_str(&format!(
                    "let __candidate_{name} = if {local} != __original_{name} {{ "
                ));
            }
            let converted = format!("__engine_{name}");
            let value = if infallible {
                format!(
                    "match {local}.try_into_engine(&()) {{ Ok(value) => value, Err(_) => return }}"
                )
            } else if block {
                format!(
                    "match {local}.try_into_engine(&*self) {{ Ok(value) => value, Err(error) => {{ log::warn!(\"mod block state write refused: {{error:?}}\"); return; }} }}"
                )
            } else if stack {
                let result = format!("__checked_{name}");
                if contains_boundary(ty, &["LootStack", "ParticleOptions"]) {
                    validations.push_str(&format!("let {result} = {local}.try_into_engine(&()); "));
                } else {
                    validations.push_str(&format!("let {result}: Result<_, mod_api::StackWriteError> = self.boundary_items().ok_or(mod_api::StackWriteError::Unavailable).and_then(|items| {local}.try_into_engine(items)); "));
                }
                format!(
                    "match {result} {{ Ok(value) => value, Err(error) => {{ log::warn!(\"mod {} write refused: {{error:?}}\"); return; }} }}",
                    hook_id.to_snake_case()
                )
            } else {
                local
            };
            if comparable {
                validations.push_str(&format!("Some({value}) }} else {{ None }}; "));
                writes.push_str(&format!(
                    "if let Some({converted}) = __candidate_{name} {{ *{name} = {converted}; }} "
                ));
            } else {
                validations.push_str(&format!("let {converted} = {value}; "));
                writes.push_str(&format!("*{name} = {converted}; "));
            }
        }
    }
    Some(StackCrossing {
        before,
        arguments,
        validators: stack_validators,
        after: format!("{validations}{writes}"),
    })
}

#[cfg(test)]
#[path = "writeback_tests.rs"]
mod writeback_tests;

#[cfg(test)]
mod stack_crossing_tests {
    use super::*;

    #[test]
    #[ignore = "daily"]
    fn level_block_read_refusal_runtime_and_mutants() {
        let mut implementations = String::new();
        for source in [
            include_str!("../../horizon-client/src/game/modding/mod_api_host.rs"),
            include_str!("../../horizon-client/src/game/modding/native_level_signals.rs"),
        ] {
            for item in syn::parse_file(source).unwrap().items {
                let syn::Item::Impl(item) = item else {
                    continue;
                };
                if !item
                    .trait_
                    .as_ref()
                    .is_some_and(|(_, path, _)| path.segments.last().unwrap().ident == "LevelHost")
                {
                    continue;
                }
                let target = item.self_ty.to_token_stream().to_string();
                let target = if target.contains("GameLevel") && !target.contains("LevelTicks") {
                    "GameLevel"
                } else if target.contains("HitLevel") {
                    "HitLevel"
                } else if target.contains("ReadLevel") {
                    "ReadLevel"
                } else if target.contains("LevelTicks") {
                    "LevelTicks<'_>"
                } else {
                    continue;
                };
                let method = item
                    .items
                    .iter()
                    .find_map(|item| match item {
                        syn::ImplItem::Fn(method) if method.sig.ident == "block_state" => {
                            Some(method.to_token_stream().to_string())
                        }
                        _ => None,
                    })
                    .unwrap()
                    .replace("use super :: api_values :: TryIntoApi ;", "");
                implementations.push_str(&format!(
                    "impl crate::LevelHost for {target} {{ {method} }}\n"
                ));
            }
        }
        assert_eq!(implementations.matches("impl crate::LevelHost").count(), 4);
        let root = r#"
struct BlockPos([i32;3]); impl BlockPos { fn new(x:i32,y:i32,z:i32)->Self {Self([x,y,z])} }
trait LevelHost { fn block_state(&self,pos:[i32;3])->Option<BlockState>; }
mod level { pub trait LevelAccess { fn block_state(&self,pos:crate::BlockPos)->crate::registry::BlockStateId; } }
"#;
        let directory =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/level-read-proof");
        std::fs::create_dir_all(&directory).unwrap();
        let index = directory.join("index.rs");
        std::fs::write(
            &index,
            include_str!("../../horizon-client/src/game/modding/api_values/block_state.rs"),
        )
        .unwrap();
        for (name, methods, success) in [
            ("baseline", implementations.clone(), true),
            (
                "fabricated_air",
                implementations.replace(
                    ". ok ()",
                    ". ok () . or (Some(crate::BlockState(crate::BlockKey(0),0)))",
                ),
                false,
            ),
            (
                "panic_on_refusal",
                implementations
                    .replace(". ok ()", ". ok () . or_else(|| panic!(\"missing index\"))"),
                false,
            ),
        ] {
            if !success {
                assert_ne!(methods, implementations);
            }
            let proof = include_str!("../../../tools/refactors/api-pr3b/level_read.rs")
                .replace("/* READ_IMPLS */", &methods);
            let fixture = include_str!("../../../tools/refactors/api-pr3b/hook_index.rs")
                .replace("INDEX_PATH", index.to_str().unwrap())
                .replace(
                    "pub fn run() {",
                    &format!("{proof}\npub fn original_index_proof() {{"),
                )
                + root;
            let source = directory.join(format!("{name}.rs"));
            let binary = directory.join(name);
            std::fs::write(&source, fixture).unwrap();
            let compiler = std::process::Command::new("rustc")
                .args(["--edition=2024", "-O"])
                .arg(source)
                .arg("-o")
                .arg(&binary)
                .output()
                .unwrap();
            assert!(
                compiler.status.success(),
                "{name}: {}",
                String::from_utf8_lossy(&compiler.stderr)
            );
            let result = std::process::Command::new(binary).output().unwrap();
            std::fs::write(
                directory.join(format!("{name}.log")),
                format!(
                    "exit: {}\nstdout:\n{}\nstderr:\n{}",
                    result.status,
                    String::from_utf8_lossy(&result.stdout),
                    String::from_utf8_lossy(&result.stderr)
                ),
            )
            .unwrap();
            assert_eq!(
                result.status.success(),
                success,
                "{name}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
            println!(
                "{name}: {}{}",
                String::from_utf8_lossy(&result.stdout),
                if success { "passed" } else { "mutant killed" }
            );
        }
    }

    #[test]
    #[ignore = "daily"]
    fn block_numeric_index_runtime_and_mutants() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let directory = root.join("target/hook-index-proof");
        std::fs::create_dir_all(&directory).unwrap();
        let production = std::fs::read_to_string(
            root.join("crates/horizon-client/src/game/modding/api_values/block_state.rs"),
        )
        .unwrap();
        let fixture = include_str!("../../../tools/refactors/api-pr3b/hook_index.rs");
        for (name, code, success) in [
            ("baseline", production.clone(), true),
            (
                "stale_mapping",
                production.replace("    clear();", ""),
                false,
            ),
            (
                "global_ordinal",
                production.replace(".get(state.0 as usize)", ".get(0)"),
                false,
            ),
        ] {
            let index = directory.join(format!("{name}_index.rs"));
            std::fs::write(&index, code).unwrap();
            let source = directory.join(format!("{name}.rs"));
            std::fs::write(
                &source,
                fixture.replace("INDEX_PATH", index.to_str().unwrap()),
            )
            .unwrap();
            let binary = directory.join(name);
            let compiler = std::process::Command::new("rustc")
                .args(["--edition=2024", "-O"])
                .arg(&source)
                .arg("-o")
                .arg(&binary)
                .output()
                .unwrap();
            assert!(
                compiler.status.success(),
                "{}",
                String::from_utf8_lossy(&compiler.stderr)
            );
            let result = std::process::Command::new(binary).output().unwrap();
            assert_eq!(
                result.status.success(),
                success,
                "{name}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
            println!(
                "{name}: {}{}",
                String::from_utf8_lossy(&result.stdout),
                if success { "passed" } else { "mutant killed" }
            );
        }
    }
    #[test]
    #[ignore = "daily"]
    fn per_block_routing_runtime_and_mutants() {
        let state: Type = syn::parse_quote!(BlockState);
        let writes: Type = syn::parse_quote!(&mut Vec<LandingWrite>);
        let crossing = stack_crossing_routed(
            &[
                ("state".into(), &state),
                ("secondary".into(), &state),
                ("writes".into(), &writes),
            ],
            None,
            false,
            "Proof",
            true,
            Some("state"),
            &Default::default(),
            None,
        )
        .unwrap();
        let global = stack_crossing_routed(
            &[
                ("state".into(), &state),
                ("secondary".into(), &state),
                ("writes".into(), &writes),
            ],
            None,
            false,
            "Proof",
            false,
            None,
            &Default::default(),
            None,
        )
        .unwrap();
        let global = format!(
            "{} callback(__api_state, __api_secondary, &mut __api_writes); {}",
            global.before, global.after
        );
        let guard = "if !crate::game::api_values::block_state::block_listens(mod_api::HookId::Proof, state) { return; }";
        let body = format!(
            "{} callback(__api_state, __api_secondary, &mut __api_writes); {}",
            crossing.before, crossing.after
        );
        let unguarded = body.replace(guard, "");
        let late = unguarded.replace("callback(", &format!("{guard} callback("));
        let after_scalar = unguarded.replace(
            "let __api_secondary",
            &format!("{guard} let __api_secondary"),
        );
        assert_ne!(body, unguarded);
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/block-routing-proof");
        std::fs::create_dir_all(&directory).unwrap();
        let index = directory.join("index.rs");
        std::fs::write(
            &index,
            // Count actual scalar crossings as well as collection conversions. The
            // routing helper itself must not trigger this conversion observer.
            include_str!("../../horizon-client/src/game/modding/api_values/block_state.rs").replace(
                "pub(crate) fn to_api(&self, state: BlockStateId) -> Result<BlockState, BlockStateError> {",
                "pub(crate) fn to_api(&self, state: BlockStateId) -> Result<BlockState, BlockStateError> { crate::CONVERSIONS.fetch_add(1, crate::Ordering::Relaxed);",
            ),
        )
        .unwrap();
        for (name, body, success) in [
            ("baseline", body, true),
            ("no_guard", unguarded, false),
            ("late_guard", late, false),
            ("after_scalar_guard", after_scalar, false),
            ("global_listener", global, true),
        ] {
            let source = directory.join(format!("{name}.rs"));
            let binary = directory.join(name);
            let mut fixture = include_str!("../../../tools/refactors/api-pr3b/block_routing.rs")
                .replace("INDEX_PATH", index.to_str().unwrap())
                .replace("FIRE_BODY", &body);
            if name == "global_listener" {
                fixture = fixture.replace("fn main()", "fn routing_main()");
                fixture.push_str(
                    r#"
fn main() {
    let b = BlockState(BlockKey(8), 0);
    game::api_values::bind(&registry::BlockRegistry(vec![Ok(b)]));
    let mut writes = vec![42];
    Game.fire(registry::BlockStateId(0), registry::BlockStateId(0), &mut writes);
    assert_eq!(CALLS.load(Ordering::Relaxed), 1);
    assert_eq!(writes, vec![42,99]);
    println!("ordinary global listener sees unrelated block B");
}
"#,
                );
            }
            std::fs::write(&source, fixture).unwrap();
            let compiled = std::process::Command::new("rustc")
                .args(["--edition=2024", "-O"])
                .arg(source)
                .arg("-o")
                .arg(&binary)
                .output()
                .unwrap();
            assert!(
                compiled.status.success(),
                "{}",
                String::from_utf8_lossy(&compiled.stderr)
            );
            let result = std::process::Command::new(binary).output().unwrap();
            assert_eq!(
                result.status.success(),
                success,
                "{name}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
            println!(
                "{name}: {} {}",
                String::from_utf8_lossy(&result.stdout),
                if success { "passed" } else { "mutant killed" }
            );
        }
    }

    #[test]
    #[ignore = "daily"]
    fn block_crossing_runtime_and_mutants() {
        let state: Type = syn::parse_quote!(&mut BlockState);
        let sibling: Type = syn::parse_quote!(&mut u32);
        // Sibling first catches early commits even when the checked field follows it.
        let crossing = stack_crossing(
            &[("sibling".into(), &sibling), ("state".into(), &state)],
            None,
            "BlockProof",
            true,
            &Default::default(),
        )
        .unwrap();
        let body = format!(
            "{} callback(&mut __api_state, &mut __api_sibling); {}",
            crossing.before, crossing.after
        );
        let early = body.replace(
            "let __engine_sibling = __api_sibling;",
            "let __engine_sibling = __api_sibling; *sibling = __engine_sibling;",
        );
        let guard =
            "if !mod_api::__private::has_part_hook(mod_api::HookId::BlockProof) { return; }";
        let unguarded = body.replace(guard, "");
        assert_ne!(early, body);
        assert_ne!(unguarded, body);
        let silent = body.replace(
            "log::warn!(\"mod block state write refused: {error:?}\");",
            "",
        );
        assert_ne!(silent, body);
        let fixture = include_str!("../../../tools/refactors/api-pr3b/hook_crossing.rs");
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/hook-crossing-proof");
        std::fs::create_dir_all(&directory).unwrap();
        for (name, body, success) in [
            ("baseline", body, true),
            ("early_commit", early, false),
            ("no_guard", unguarded, false),
            ("silent_refusal", silent, false),
        ] {
            let source = directory.join(format!("{name}.rs"));
            let binary = directory.join(name);
            std::fs::write(&source, fixture.replace("FIRE_BODY", &body)).unwrap();
            let compiler = std::process::Command::new("rustc")
                .args(["--edition=2024", "-O"])
                .arg(&source)
                .arg("-o")
                .arg(&binary)
                .output()
                .unwrap();
            assert!(
                compiler.status.success(),
                "{}",
                String::from_utf8_lossy(&compiler.stderr)
            );
            let result = std::process::Command::new(binary).output().unwrap();
            assert_eq!(
                result.status.success(),
                success,
                "{name}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
            println!(
                "{name}: {}{}",
                String::from_utf8_lossy(&result.stdout),
                if success { "passed" } else { "mutant killed" }
            );
        }
    }

    #[test]
    fn boundary_draw_context_is_borrowed_not_snapshotted() {
        let state: Type = syn::parse_quote!(BlockState);
        let draw: Type = syn::parse_quote!(&mut dyn CameraDrawContext);
        let replace: Type = syn::parse_quote!(&mut bool);
        let crossing = stack_crossing(
            &[
                ("state".into(), &state),
                ("draw".into(), &draw),
                ("replace".into(), &replace),
            ],
            None,
            "BlockScreenEffectDraw",
            false,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(
            crossing.arguments,
            ["__api_state", "draw", "&mut __api_replace"]
        );
        assert!(!crossing.before.contains("__api_draw"));
        assert!(!crossing.after.contains("*draw"));
    }

    #[test]
    fn landing_writes_are_checked_and_staged_with_siblings() {
        let writes: Type = syn::parse_quote!(&mut Vec<LandingWrite>);
        let sibling: Type = syn::parse_quote!(&mut bool);
        let crossing = stack_crossing(
            &[("sibling".into(), &sibling), ("writes".into(), &writes)],
            None,
            "BlockLanding",
            false,
            &Default::default(),
        )
        .unwrap();
        assert!(crossing.before.contains("writes.try_into_api()"));
        assert!(!crossing.before.contains("ItemBoundary"));
        assert!(
            crossing.after.find("try_into_engine(&*self)").unwrap()
                < crossing.after.find("*sibling =").unwrap()
        );
    }

    #[test]
    fn gui_snapshot_and_writeback_are_guarded_and_context_free() {
        let draws: Type = syn::parse_quote!(&mut Option<Vec<GuiDraw>>);
        let crossing = stack_crossing(
            &[("draws".into(), &draws)],
            None,
            "HealthHud",
            false,
            &Default::default(),
        )
        .unwrap();
        assert!(
            crossing
                .before
                .starts_with("if !mod_api::__private::listened")
        );
        assert!(
            crossing
                .before
                .contains("match draws.try_into_api() { Ok(value) => value, Err(_) => return }")
        );
        assert!(crossing.after.contains(
            "match __api_draws.try_into_engine(&()) { Ok(value) => value, Err(_) => return }"
        ));
        assert!(crossing.after.ends_with("*draws = __engine_draws; "));
        assert!(!crossing.before.contains("ItemBoundary"));
        assert!(!crossing.after.contains("StackWriteError"));
    }
    #[test]
    fn completion_hook_converts_only_after_subscription_check() {
        let ty: Type = syn::parse_quote!(&mut Vec<Suggestion>);
        let crossing = stack_crossing(
            &[("suggestions".to_owned(), &ty)],
            None,
            "TabComplete",
            false,
            &Default::default(),
        )
        .unwrap();
        assert!(
            crossing
                .before
                .starts_with("if !mod_api::__private::listened")
        );
        assert!(crossing.before.contains("suggestions.try_into_api()"));
        assert_eq!(crossing.arguments, ["&mut __api_suggestions"]);
        let body = format!(
            "{{ {} dispatch(&mut __api_suggestions); {} }}",
            crossing.before, crossing.after
        );
        syn::parse_str::<syn::Block>(&body).unwrap();
    }
    #[test]
    #[ignore = "daily"]
    fn unsubscribed_owned_values_never_convert_or_allocate() {
        for record in ["ItemStack", "ParticleOptions"] {
            let stack: Type = syn::parse_str(&format!("&{record}")).unwrap();
            let claimed: Type = syn::parse_quote!(&mut bool);
            let args = [
                ("stack".to_owned(), &stack),
                ("claimed".to_owned(), &claimed),
            ];
            let crossing = stack_crossing(
                &args,
                None,
                "UseItem",
                record == "ItemStack",
                &Default::default(),
            )
            .unwrap();
            let body = format!(
                "{} mod_api::dispatch_use_item(self, {}); {}",
                crossing.before,
                crossing.arguments.join(", "),
                crossing.after
            );
            let source = r#"
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
struct Counter;
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
unsafe impl std::alloc::GlobalAlloc for Counter {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { std::alloc::System.alloc(layout) }
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: std::alloc::Layout) {
        unsafe { std::alloc::System.dealloc(pointer, layout); }
    }
}
#[global_allocator] static ALLOCATOR: Counter = Counter;
static SUBSCRIBED: AtomicBool = AtomicBool::new(false);
static CONVERSIONS: AtomicUsize = AtomicUsize::new(0);
static DISPATCHES: AtomicUsize = AtomicUsize::new(0);
pub struct ItemStack;
pub struct ApiStack;
pub struct Game;
mod game { pub mod api_values {
    pub trait TryIntoApi { fn try_into_api(&self) -> Result<crate::ApiStack, ()>; }
    impl TryIntoApi for crate::ItemStack {
        fn try_into_api(&self) -> Result<crate::ApiStack, ()> {
            crate::CONVERSIONS.fetch_add(1, crate::Ordering::Relaxed);
            Ok(crate::ApiStack)
        }
    }
} }
mod mod_api {
    pub enum HookId { UseItem }
    pub mod __private {
        pub fn has_part_hook(_: super::HookId) -> bool {
            crate::SUBSCRIBED.load(crate::Ordering::Relaxed)
        }
    }
    pub fn dispatch_use_item(_: &mut crate::Game, _: &crate::ApiStack, claimed: &mut bool) {
        crate::DISPATCHES.fetch_add(1, crate::Ordering::Relaxed);
        *claimed = true;
    }
}
impl Game {
    fn fire_use_item(&mut self, stack: &ItemStack, claimed: &mut bool) { FIRE_BODY }
}
fn main() {
    let mut game = Game;
    let mut claimed = false;
    let allocations = ALLOCATIONS.load(Ordering::Relaxed);
    for _ in 0..1_000 { game.fire_use_item(&ItemStack, &mut claimed); }
    assert_eq!(ALLOCATIONS.load(Ordering::Relaxed), allocations);
    assert_eq!(CONVERSIONS.load(Ordering::Relaxed), 0);
    assert_eq!(DISPATCHES.load(Ordering::Relaxed), 0);
    assert!(!claimed);
    SUBSCRIBED.store(true, Ordering::Relaxed);
    game.fire_use_item(&ItemStack, &mut claimed);
    assert_eq!(CONVERSIONS.load(Ordering::Relaxed), 1);
    assert_eq!(DISPATCHES.load(Ordering::Relaxed), 1);
    assert!(claimed);
}
"#
            .replace("FIRE_BODY", &body)
            .replace("ItemStack", record)
            .replace(
                "has_part_hook",
                if record == "ItemStack" {
                    "has_part_hook"
                } else {
                    "listened"
                },
            );
            let target = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let directory = target.join(format!("item-fire-{}-{unique}", std::process::id()));
            std::fs::create_dir_all(&directory).unwrap();
            let input = directory.join("fire.rs");
            let output = directory.join(format!("fire{}", std::env::consts::EXE_SUFFIX));
            std::fs::write(&input, source).unwrap();
            let compiler = std::process::Command::new("rustc")
                .args(["--edition=2024", "--crate-name=item_fire_counter"])
                .arg(&input)
                .arg("-o")
                .arg(&output)
                .output()
                .unwrap();
            assert!(
                compiler.status.success(),
                "{}",
                String::from_utf8_lossy(&compiler.stderr)
            );
            assert!(
                std::process::Command::new(output)
                    .status()
                    .unwrap()
                    .success()
            );
            std::fs::remove_dir_all(directory).unwrap();
        }
    }

    #[test]
    fn qualified_names_do_not_select_a_root_record_crossing() {
        for ty in [
            syn::parse_quote!(&foreign::ItemStack),
            syn::parse_quote!(&mod_api::ItemStack),
            syn::parse_quote!(&mut Vec<foreign::MerchantOffer>),
        ] {
            assert!(
                stack_crossing(
                    &[("value".to_owned(), &ty)],
                    None,
                    "AnyName",
                    false,
                    &Default::default()
                )
                .is_none()
            );
        }
        let ty = syn::parse_quote!(&mut Option<ItemStack>);
        assert!(
            stack_crossing(
                &[("value".to_owned(), &ty)],
                None,
                "AnyName",
                false,
                &Default::default()
            )
            .is_some()
        );
    }
    #[test]
    fn rule_hook_gates_before_copying_value() {
        let ty: Type = syn::parse_quote!(GameRuleValue);
        let crossing = stack_crossing(
            &[("value".to_owned(), &ty)],
            None,
            "GameRuleChanged",
            false,
            &Default::default(),
        )
        .unwrap();
        assert!(
            crossing
                .before
                .starts_with("if !mod_api::__private::listened")
        );
        assert!(crossing.before.contains("value.try_into_api()"));
        assert_eq!(crossing.arguments, ["__api_value"]);
        assert!(crossing.after.is_empty());
    }

    #[test]
    fn placement_hook_gates_before_snapshot_and_stages_the_answer() {
        let context: Type = syn::parse_quote!(&PlacementContext);
        let answer: Type = syn::parse_quote!(&mut Option<Option<BlockStateId>>);
        let crossing = stack_crossing(
            &[
                ("context".to_owned(), &context),
                ("answer".to_owned(), &answer),
            ],
            None,
            "StateForPlacement",
            true,
            &Default::default(),
        )
        .unwrap();
        assert!(
            crossing
                .before
                .starts_with("if !mod_api::__private::has_part_hook")
        );
        assert!(crossing.before.contains("context.try_into_api()"));
        assert_eq!(crossing.arguments, ["&__api_context", "&mut __api_answer"]);
        assert!(crossing.after.contains("*answer = __engine_answer"));
    }

    #[test]
    fn plain_hook_has_no_stack_seam() {
        let scalar: Type = syn::parse_quote!(&mut u32);
        assert!(
            stack_crossing(
                &[("power".to_owned(), &scalar)],
                None,
                "Plain",
                false,
                &Default::default()
            )
            .is_none()
        );
    }
}

fn carrier_fields(args: &[(String, &Type)]) -> (String, &'static str) {
    let mut lifetimes = CarrierLifetimes::default();
    let fields = args
        .iter()
        .map(|(arg, ty)| {
            let mut ty = (*ty).clone();
            lifetimes.visit_type_mut(&mut ty);
            format!("pub {arg}: {},\n", ty.to_token_stream())
        })
        .collect();
    (fields, if lifetimes.used { "<'a>" } else { "" })
}

/// Names every reference and elided lifetime of a carrier's field types `'a`.
#[derive(Default)]
struct CarrierLifetimes {
    used: bool,
}

impl VisitMut for CarrierLifetimes {
    fn visit_type_reference_mut(&mut self, reference: &mut syn::TypeReference) {
        self.used = true;
        reference.lifetime = Some(syn::parse_quote!('a));
        self.visit_type_mut(&mut reference.elem);
    }
    fn visit_lifetime_mut(&mut self, lifetime: &mut syn::Lifetime) {
        if lifetime.ident == "_" {
            self.used = true;
            *lifetime = syn::parse_quote!('a);
        }
    }
}

/// `T` of an `Option<Answered<T>>` output, if that is its type.
fn answered_type(ty: &Type) -> Option<&Type> {
    generic_of(generic_of(ty, "Option")?, "Answered")
}

/// `T` of a `Vec<Answered<T>>` output, if that is its type.
fn answered_batch(ty: &Type) -> Option<&Type> {
    generic_of(generic_of(ty, "Vec")?, "Answered")
}

/// `T` of a `name<T>` type.
fn generic_of<'a>(ty: &'a Type, name: &str) -> Option<&'a Type> {
    let Type::Path(path) = ty else { return None };
    let segment = path.path.segments.last().filter(|s| s.ident == name)?;
    let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
        return None;
    };
    match args.args.first()? {
        syn::GenericArgument::Type(ty) => Some(ty),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "daily"]
    fn setting_changes_notify_only_the_owner() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let sources = super::super::sources(&root, super::super::FillMode::Require).unwrap();
        let file = syn::parse_file(&sources.hooks).unwrap();
        let function = file
            .items
            .into_iter()
            .find_map(|item| match item {
                syn::Item::Fn(f) if f.sig.ident == "dispatch_setting_changed" => Some(f),
                _ => None,
            })
            .unwrap();
        let function = function.to_token_stream().to_string();
        let dir = root.join("target/setting-owner-test");
        std::fs::create_dir_all(&dir).unwrap();
        for (name, function, succeeds) in [
            ("owner", function.clone(), true),
            (
                "broadcast",
                function.replace("if owner . id != event . mod_id { continue ; }", ""),
                false,
            ),
        ] {
            let program = format!(
                r#"
#![allow(dead_code, unused_variables)]
use std::sync::atomic::{{AtomicUsize, Ordering}};
static A: AtomicUsize = AtomicUsize::new(0);
static B: AtomicUsize = AtomicUsize::new(0);
struct Owner {{ id: &'static str }}
static OWNERS: [Owner; 2] = [Owner {{ id: "a" }}, Owner {{ id: "b" }}];
fn listened(_: generated::HookId) -> bool {{ true }}
fn listening(_: generated::HookId) -> impl Iterator<Item=(bool, &'static Owner)> {{ OWNERS.iter().map(|m| (false, m)) }}
mod runtime {{
    pub struct ListenerScope;
    impl ListenerScope {{
        pub fn new() -> Self {{ Self }}
        pub fn enter<'a>(&mut self, owner: &'a super::Owner, _: &str) -> &'a super::Owner {{ owner }}
    }}
}}
impl Owner {{ fn setting_changed(&self, _: &mut dyn generated::ClientHost, _: &mut generated::SettingChangedEvent<'_>) {{ if self.id == "a" {{ A.fetch_add(1, Ordering::Relaxed); }} else {{ B.fetch_add(1, Ordering::Relaxed); }} }} }}
mod generated {{
pub trait ClientHost {{}}
impl ClientHost for () {{}}
pub enum HookId {{ SettingChanged }}
pub struct SettingChangedEvent<'a> {{ pub mod_id: &'a str, pub key: &'a str }}
{function}
}}
fn main() {{ generated::dispatch_setting_changed(&mut (), "a", "toggle"); assert_eq!(A.load(Ordering::Relaxed), 1); assert_eq!(B.load(Ordering::Relaxed), 0); }}
"#
            );
            let source = dir.join(format!("{name}.rs"));
            let binary = dir.join(name);
            std::fs::write(&source, program).unwrap();
            let output = std::process::Command::new("rustc")
                .arg("--edition=2024")
                .arg(&source)
                .arg("-o")
                .arg(&binary)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                std::process::Command::new(binary)
                    .status()
                    .unwrap()
                    .success(),
                succeeds,
                "{name}"
            );
        }
    }

    #[test]
    #[ignore = "daily"]
    fn settings_boundary_owns_and_caches_values() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let source = std::fs::read_to_string(
            root.join("crates/horizon-client/src/game/native_hooks/base.rs"),
        )
        .unwrap();
        let source = source
            + &std::fs::read_to_string(
                root.join("crates/horizon-client/src/game/native_hooks/shared.rs"),
            )
            .unwrap();
        let file = syn::parse_file(&source).unwrap();
        let methods: String = file
            .items
            .into_iter()
            .filter_map(|item| match item {
                syn::Item::Impl(i) => Some(
                    i.items
                        .into_iter()
                        .filter_map(|item| match item {
                            syn::ImplItem::Fn(f)
                                if [
                                    "setting",
                                    "stored_setting",
                                    "set_setting",
                                    "store_setting",
                                    "another_mods",
                                    "native_setting_event",
                                ]
                                .contains(&f.sig.ident.to_string().as_str()) =>
                            {
                                Some(f.to_token_stream().to_string())
                            }
                            _ => None,
                        })
                        .collect::<String>(),
                ),
                _ => None,
            })
            .collect();
        let settings_source = std::fs::read_to_string(
            root.join("crates/horizon-client/src/game/native_hooks/mod_settings.rs"),
        )
        .unwrap();
        let settings_file = syn::parse_file(&settings_source).unwrap();
        let settings: String = settings_file
            .items
            .into_iter()
            .filter_map(|item| match item {
                syn::Item::Fn(f) => Some(f.to_token_stream().to_string()),
                syn::Item::Static(s) => Some(s.to_token_stream().to_string()),
                _ => None,
            })
            .collect();
        let dir = root.join("target/settings-boundary-test");
        std::fs::create_dir_all(&dir).unwrap();
        let program = format!(
            r#"
#![allow(dead_code)]
mod mod_api {{
    #[derive(Clone, Copy, Debug, PartialEq)] pub enum SettingValue {{ Toggle(bool), Choice(usize), Range(f64), Color(u32) }}
    pub struct Spec;
    impl Spec {{ pub fn store(&self, value: SettingValue) -> Option<String> {{ match value {{ SettingValue::Toggle(v) => Some(v.to_string()), _ => None }} }} pub fn read(&self, raw: Option<&str>) -> SettingValue {{ SettingValue::Toggle(raw == Some("true")) }} }}
    pub mod content {{ pub mod crash {{ pub fn current_native_mod() -> Option<&'static str> {{ Some("a") }} }} }}
    pub mod __private {{ pub fn setting(owner: &str, key: &str) -> Option<super::Spec> {{ (owner == "a" && key == "toggle").then_some(super::Spec) }} }}
}}
mod horizon_world {{ pub mod native_load_context {{ pub fn read_setting(dir: &std::path::Path, key: &str) -> Option<String> {{ std::fs::read_to_string(dir.join(key)).ok() }} }} }}
mod game {{
use crate::mod_api;
mod mod_settings {{
use crate::{{mod_api, horizon_world}};
use std::{{fs, path::{{Path, PathBuf}}, collections::HashMap, sync::{{LazyLock, Mutex}}}};
{settings}
pub(super) fn typed(dir: &Path, key: &str) -> Option<mod_api::SettingValue> {{ read_typed(dir, "a", key) }}
}}
use std::{{fs, path::PathBuf, cell::RefCell, collections::BTreeMap}};
struct Context;
impl Context {{ fn setting(&self, _: &str, _: &str) -> Option<String> {{ Some("loaded".into()) }} }}
enum NativeSettingValue {{ Toggle(bool), Choice {{ index: usize }}, Range {{ value: f64 }}, Color(u32) }}
struct Setting {{ key: String, value: NativeSettingValue }}
struct Page {{ settings: Vec<Setting> }}
struct Row {{ id: String, native: Option<Page> }}
struct Mods {{ installed: Vec<Row> }}
struct Settings {{ mods: Mods }}
struct Game {{ settings: Settings, notifications: usize, loop_context: Context, setting_overrides: RefCell<BTreeMap<(String, String), String>>, root: PathBuf }}
impl Game {{
    fn native_setting_changed(&mut self, _: &str, _: &str) {{ self.notifications += 1; }}
    fn native_data_dir(&self, id: &str) -> PathBuf {{ self.root.join(id) }}
    {methods}
}}
pub fn run(root: PathBuf) {{
    let mut game = Game {{ settings: Settings {{ mods: Mods {{ installed: vec![Row {{ id: "a".into(), native: Some(Page {{ settings: vec![Setting {{ key: "toggle".into(), value: NativeSettingValue::Toggle(true) }}] }}) }}] }} }}, notifications: 0, loop_context: Context, setting_overrides: RefCell::new(BTreeMap::new()), root }};
    assert_eq!(game.setting("a", "toggle"), None);
    assert_eq!(game.setting("b", "toggle"), None);
    game.set_setting("b", "toggle", "stolen");
    assert!(!game.native_data_dir("b").exists());
    game.set_setting("a", "toggle", "edited");
    assert_eq!(game.setting("a", "toggle").as_deref(), Some("edited"));
    game.native_setting_event(0, 0);
    assert_eq!(game.notifications, 1);
    game.set_setting("a", "toggle", "true");
    assert_eq!(mod_settings::typed(&game.native_data_dir("a"), "toggle"), Some(mod_api::SettingValue::Toggle(true)));
    fs::write(game.native_data_dir("a").join("toggle"), "false").unwrap();
    assert_eq!(mod_settings::typed(&game.native_data_dir("a"), "toggle"), Some(mod_api::SettingValue::Toggle(true)));
    assert_eq!(mod_settings::typed(&game.root.join("other"), "toggle"), Some(mod_api::SettingValue::Toggle(false)));
    game.set_setting("a", "toggle", "false");
    assert_eq!(mod_settings::typed(&game.native_data_dir("a"), "toggle"), Some(mod_api::SettingValue::Toggle(false)));
    fs::remove_file(game.native_data_dir("a").join("toggle")).unwrap();
    fs::create_dir(game.native_data_dir("a").join("toggle")).unwrap();
    assert!(!game.store_setting("a", "toggle", "true"));
    game.native_setting_event(0, 0);
    assert_eq!(game.notifications, 1, "failed persistence must not notify");
    fs::create_dir_all(game.root.join("b")).unwrap();
    fs::write(game.root.join("b/secret"), "secret").unwrap();
    game.set_setting("a", "../b/secret", "bad");
    assert_eq!(fs::read_to_string(game.root.join("b/secret")).unwrap(), "secret");
    assert_eq!(game.setting("a", "../b/secret"), None);
}}
}}
fn main() {{ game::run(std::env::args_os().nth(1).unwrap().into()); }}
"#
        );
        let source = dir.join("boundary.rs");
        let binary = dir.join("boundary");
        std::fs::write(&source, program).unwrap();
        let output = std::process::Command::new("rustc")
            .arg("--edition=2024")
            .arg(&source)
            .arg("-o")
            .arg(&binary)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let data = dir.join("data");
        if data.exists() {
            std::fs::remove_dir_all(&data).unwrap();
        }
        assert!(
            std::process::Command::new(binary)
                .arg(data)
                .status()
                .unwrap()
                .success()
        );
    }

    #[test]
    #[ignore = "daily"]
    fn an_answered_collect_names_the_mod_of_each_entry() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let hook = Hook {
            name: "gather".into(),
            contract: Contract::Collect,
            out: Some("entries".into()),
            answered_by_mod: true,
            ..Hook::sample(
                "horizon_client::game::native_hooks::gather::Game<'_>::native_gather",
                Side::Server,
            )
        };
        let method: syn::ItemFn = syn::parse_quote! {
            fn native_gather(&mut self, entries: &mut Vec<Answered<u32>>) {
                let _ = (entries,);
            }
        };
        let sources = emit(&root, std::iter::once(&hook), &[method], &[]).unwrap();
        let (traits, dispatch) = (sources.traits, sources.dispatch);
        let program = format!(
            r#"
#![allow(dead_code, unused_variables)]
struct Gatherer {{ pushes: &'static [u32] }}
static MODS: [(&str, Gatherer); 3] = [("a", Gatherer {{ pushes: &[1, 2] }}), ("quiet", Gatherer {{ pushes: &[] }}), ("b", Gatherer {{ pushes: &[3] }})];
fn listened(_: generated::HookId) -> bool {{ true }}
fn entered_with_id(_: generated::HookId, _: &str) -> impl Iterator<Item = (&'static str, &'static Gatherer)> {{ MODS.iter().map(|(id, m)| (*id, m)) }}
impl Gatherer {{ fn gather(&self, _: &mut dyn generated::ServerHost, event: &mut generated::GatherEvent<'_>) {{ assert!(event.entries.is_empty()); event.entries.extend(self.pushes); }} }}
mod generated {{
pub trait ServerHost {{}}
impl ServerHost for () {{}}
pub enum HookId {{ Gather }}
#[derive(Debug, PartialEq)]
pub struct Answered<T> {{ pub answer: T, pub mod_id: &'static str }}
pub trait Hooks {{ {traits} }}
{dispatch}
}}
use generated::Answered;
fn main() {{
    let mut entries = vec![Answered {{ answer: 0, mod_id: "game" }}];
    generated::dispatch_gather(&mut (), &mut entries);
    let entry = |answer, mod_id| Answered {{ answer, mod_id }};
    assert_eq!(entries, [entry(0, "game"), entry(1, "a"), entry(2, "a"), entry(3, "b")]);
}}
"#
        );
        let dir = root.join("target/answered-collect-test");
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("main.rs");
        let binary = dir.join("main");
        std::fs::write(&source, program).unwrap();
        let output = std::process::Command::new("rustc")
            .arg("--edition=2024")
            .arg(&source)
            .arg("-o")
            .arg(&binary)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let run = std::process::Command::new(binary).output().unwrap();
        assert!(
            run.status.success(),
            "{}",
            String::from_utf8_lossy(&run.stderr)
        );
    }

    #[test]
    fn an_answered_collect_needs_answered_entries() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let hook = Hook {
            name: "gather".into(),
            contract: Contract::Collect,
            out: Some("entries".into()),
            answered_by_mod: true,
            ..Hook::sample(
                "horizon_client::game::native_hooks::gather::Game<'_>::native_gather",
                Side::Server,
            )
        };
        let method: syn::ItemFn = syn::parse_quote! {
            fn native_gather(&mut self, entries: &mut Vec<u32>) {
                let _ = (entries,);
            }
        };
        let refused = emit(&root, std::iter::once(&hook), &[method], &[]).err();
        assert_eq!(
            refused.as_deref(),
            Some("gather: output must be Vec<Answered<T>>")
        );
    }

    #[test]
    fn a_new_hook_over_the_limit_needs_the_carrier() {
        assert!(target_parameters("new_hook", "native_new_hook", 7, "NewHookCall").is_ok());
        let refused = target_parameters("new_hook", "native_new_hook", 8, "NewHookCall");
        assert!(refused.unwrap_err().contains("one NewHookCall"));
    }

    #[test]
    #[ignore = "daily"]
    fn scoped_native_hook_uses_selected_host_and_game_hooks_keep_their_receiver() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let fire = super::super::sources(&root, super::super::FillMode::Require)
            .unwrap()
            .fire;
        let compact: String = fire.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(compact.contains("mod_api::dispatch_living_change_target(self.host,call)"));
        assert!(
            compact.contains("mod_api::dispatch_living_visibility(self,entity,looking,visibility)")
        );
    }

    #[test]
    #[ignore = "daily"]
    fn a_worldgen_part_method_takes_the_worldgen_context_and_reads_the_published_table() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let hooks = super::super::sources(&root, super::super::FillMode::Require)
            .unwrap()
            .hooks;
        let compact: String = hooks.chars().filter(|c| !c.is_whitespace()).collect();
        fn dispatch<'a>(compact: &'a str, name: &str) -> &'a str {
            let start = compact
                .find(&format!("fndispatch_{name}("))
                .unwrap_or_else(|| panic!("no dispatch for {name}"));
            let body = &compact[start..];
            &body[..body.find("#[doc(hidden)]").unwrap_or(body.len())]
        }
        let method = &compact[compact.find("fnon_tree_grow(").unwrap()..];
        assert!(method.starts_with("fnon_tree_grow(&self,context:&dynWorldgenHost,"));
        let worldgen = dispatch(&compact, "on_tree_grow");
        assert!(worldgen.contains("=super::worldgen_parts();"));
        assert!(!worldgen.contains("super::parts("));
        // Every other scoped hook keeps this thread's table, `is_ladder`'s loop context included.
        assert!(
            dispatch(&compact, "is_ladder")
                .contains("=super::parts(HookId::IsLadder,\"is_ladder\");")
        );
    }
}
fn contains_boundary(ty: &Type, names: &[&str]) -> bool {
    use syn::visit::Visit;
    struct Boundary<'a> {
        found: bool,
        names: &'a [&'a str],
    }
    impl<'ast> Visit<'ast> for Boundary<'_> {
        fn visit_type_path(&mut self, path: &'ast syn::TypePath) {
            // Only bare names have the emitted API module's provenance.
            self.found |= path.path.segments.len() == 1
                && path
                    .path
                    .segments
                    .last()
                    .is_some_and(|s| self.names.contains(&s.ident.to_string().as_str()));
            syn::visit::visit_type_path(self, path);
        }
    }
    let mut boundary = Boundary {
        found: false,
        names,
    };
    boundary.visit_type(ty);
    boundary.found
}

fn stack_list_output(ty: &Type) -> bool {
    contains_boundary(ty, &["ItemStack"]) && contains_boundary(ty, &["Vec"])
}

#[cfg(test)]
#[path = "dispatch/stack_count_tests.rs"]
mod stack_count_tests;
