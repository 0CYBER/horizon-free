//! The shape of an injection point (arismoko/horizon#589): what a `#[point]` function hands its listeners. Shared
//! by the `#[point]` attribute (the check at the function's head) and the mod API generator (the
//! records, `PointId` and `points.json`), so the two always agree.
pub mod build_data;

use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::{FnArg, Pat, Type};

/// Where a point's function runs, as a hook's `side`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Server,
    Client,
    Both,
}

impl Side {
    pub fn name(self) -> &'static str {
        match self {
            Self::Server => "server",
            Self::Client => "client",
            Self::Both => "both",
        }
    }
}

/// How a point's function takes a parameter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pass {
    /// By value: the record holds it, and the function runs with the record's value.
    Value,
    /// `&T`: the record holds an owned copy, and the function runs with a changed copy.
    Shared,
    /// `&mut T`: copied into the record, and the record's value is written back to the place.
    Mut,
}

/// One parameter a point's API listeners get: every parameter, or the point has no API record.
pub struct Arg {
    pub name: syn::Ident,
    /// The parameter's type, as written.
    pub ty: Type,
    pub pass: Pass,
    /// The record's field: the parameter's owned value, an engine type as its `mod_api` mirror.
    pub field: Type,
    /// It converts through the game's `Mirror` (an engine type with a `mod_api` mirror).
    pub mirror: bool,
    /// The native-width type (`usize`, `isize`) the record carries as 64 bits.
    pub wide: Option<Type>,
    /// `&str`, carried as a `Cow`, so a listener may replace it with an owned `String`.
    pub text: bool,
}

/// A value the API carries back: the function's return type in the record.
pub struct Value {
    /// The return type, as written.
    pub ty: Type,
    /// The record's `result` type.
    pub field: Type,
    pub mirror: bool,
}

/// What a point's listeners may do with the function's return value.
pub enum Returns {
    /// `()`: a head listener may `cancel`, and the function returns at once.
    Unit,
    /// `Result<()>`: the same, returning `Ok(())`.
    UnitResult,
    /// An API-typed value: a head listener may set `result`, which the function returns at once;
    /// a return listener sees the value in `result` and may change it.
    Value(Value),
    /// A value the API can't carry: the listeners neither stop the function nor see its value.
    Opaque,
}

/// How a point's function takes `self`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Receiver {
    /// A free function, or an associated one without `self`.
    None,
    /// `&self`.
    Shared,
    /// `&mut self`.
    Mut,
}

/// A point's items in `mod_points` ([`Shape::api_items`]), each list one entry per place (its head,
/// then its return).
#[derive(Default)]
pub struct ApiItems {
    /// Its typed function aliases and constructors.
    pub listeners: TokenStream,
    /// Its `PointId` variants.
    pub variants: TokenStream,
    /// Its typed native and wasm dispatch functions.
    pub dispatch: TokenStream,
}

/// A `#[point]` function's point.
pub struct Shape {
    /// Unique among the game's points: the attribute's `name`, or the function's own.
    pub name: String,
    /// The side its `Game` method runs on, which picks its API listeners' host. A point off `Game`
    /// has none, and its API listeners get no host.
    pub side: Option<Side>,
    pub receiver: Receiver,
    /// Every parameter, in order, as its API listeners get it; empty when the point has none.
    pub args: Vec<Arg>,
    /// Why the point has no API record: the first parameter the API can't carry, or a receiver
    /// with no API handle. Its API listeners are refused at registration with this reason.
    pub api_refusal: Option<String>,
    pub returns: Returns,
    /// The value it returns borrows (`-> &T`, `-> Option<&mut T>`): the receiver is still
    /// borrowed when the body ends, so its return place is unavailable.
    pub borrowed_return: bool,
    /// A free function (`free` in the attribute), not an associated one.
    pub free: bool,
    /// The function's entry cell in its crate's replacement array (`slot = N`), written by the
    /// staging pass: the function's head is that cell's one load, which a listened point's
    /// dispatcher is installed through.
    pub slot: Option<usize>,
}

/// Whether the staging pass gives a function with these `attrs` and `sig` a point: one predicate for
/// the pass and for the generator that lists the points, so the two always agree. Left out:
/// the hooks' own functions (`native_*`, whose bodies the generator reads), anything compiled only
/// under a `cfg`, `const`, `async` and `unsafe` functions, methods taking `self` by value,
/// `impl Trait` in the signature, `-> !`, `#[inline(always)]`, and parameters named `cancel` or
/// `result`, which the record has its own of. A `Game` method also needs a `&self` or
/// `&mut self` receiver: its listeners get the game as their host.
pub fn eligible(attrs: &[syn::Attribute], sig: &syn::Signature) -> bool {
    let impl_trait = |ty: &Type| matches!(ty, Type::ImplTrait(_));
    !sig.ident.to_string().starts_with("native_")
        && sig.constness.is_none()
        && sig.asyncness.is_none()
        && sig.unsafety.is_none()
        && sig.abi.is_none()
        && !matches!(sig.inputs.first(), Some(FnArg::Receiver(receiver)) if receiver.reference.is_none())
        && !matches!(&sig.output, syn::ReturnType::Type(_, ty) if impl_trait(ty) || matches!(**ty, Type::Never(_)))
        && sig.inputs.iter().all(|input| match input {
            FnArg::Typed(typed) => {
                !impl_trait(&typed.ty)
                    && !matches!(&*typed.pat, Pat::Ident(pat) if pat.ident == "cancel" || pat.ident == "result")
            }
            FnArg::Receiver(_) => true,
        })
        && !attrs.iter().any(|attr| {
            let path = attr.path();
            path.is_ident("cfg")
                || path.is_ident("cfg_attr")
                || path.is_ident("test")
                || (path.is_ident("inline")
                    && attr
                        .parse_args::<syn::Ident>()
                        .is_ok_and(|arg| arg == "always"))
        })
}

/// Whether `ty` holds a reference or a lifetime other than `'static`: a value that may borrow.
fn borrows(ty: &Type) -> bool {
    struct Find(bool);
    impl<'ast> syn::visit::Visit<'ast> for Find {
        fn visit_type_reference(&mut self, reference: &'ast syn::TypeReference) {
            match &reference.lifetime {
                Some(lifetime) if lifetime.ident == "static" => {}
                _ => self.0 = true,
            }
            syn::visit::visit_type_reference(self, reference);
        }
        fn visit_lifetime(&mut self, lifetime: &'ast syn::Lifetime) {
            if lifetime.ident != "static" {
                self.0 = true;
            }
        }
    }
    let mut find = Find(false);
    syn::visit::Visit::visit_type(&mut find, ty);
    find.0
}

/// The point of the function `sig`, from `#[point(side = "…", name = "…", slot = N, free, api(…),
/// mirror(…))]`'s arguments. `api(EntityRef = "::mod_api::EntityRef", …)` names the `mod_api` types
/// with a `Wire` layout the signature writes; `mirror(Text = "::mod_api::Text", …)` the engine
/// types the game carries as their `mod_api` mirrors. The generator resolves both once.
pub fn shape(attribute: TokenStream, sig: &syn::Signature) -> syn::Result<Shape> {
    let (mut name, mut side, mut free) = (sig.ident.to_string(), None, false);
    let mut slot = None;
    let mut mirrors = Vec::new();
    let parser = syn::meta::parser(|meta| {
        if meta.path.is_ident("free") {
            free = true;
        } else if meta.path.is_ident("name") {
            name = meta.value()?.parse::<syn::LitStr>()?.value();
        } else if meta.path.is_ident("slot") {
            slot = Some(meta.value()?.parse::<syn::LitInt>()?.base10_parse()?);
        } else if meta.path.is_ident("mirror") || meta.path.is_ident("api") {
            let convert = meta.path.is_ident("mirror");
            meta.parse_nested_meta(|mirror| {
                let api: Type = mirror.value()?.parse::<syn::LitStr>()?.parse()?;
                mirrors.push(Mirror {
                    written: spelling(&mirror.path),
                    api,
                    convert,
                });
                Ok(())
            })?;
        } else if meta.path.is_ident("side") {
            let value = meta.value()?.parse::<syn::LitStr>()?;
            side = Some(match value.value().as_str() {
                "server" => Side::Server,
                "client" => Side::Client,
                "both" => Side::Both,
                _ => {
                    return Err(syn::Error::new(
                        value.span(),
                        "side is server, client or both",
                    ));
                }
            });
        } else {
            return Err(meta.error(
                "a point takes side = \"…\", name = \"…\", slot = N, free, api(…) and mirror(…)",
            ));
        }
        Ok(())
    });
    syn::parse::Parser::parse2(parser, attribute)?;
    let receiver = match sig.inputs.first() {
        Some(FnArg::Receiver(receiver)) if receiver.reference.is_none() => {
            return Err(syn::Error::new_spanned(
                receiver,
                "a point's function takes `self` by reference, or not at all",
            ));
        }
        Some(FnArg::Receiver(receiver)) if receiver.mutability.is_some() => Receiver::Mut,
        Some(FnArg::Receiver(_)) => Receiver::Shared,
        _ => Receiver::None,
    };
    if side.is_some() && receiver == Receiver::None {
        return Err(syn::Error::new(
            sig.ident.span(),
            "a point with a side is a `&self` or `&mut self` method of `Game`",
        ));
    }
    let mut args = Vec::new();
    let mut refusal = None;
    if receiver != Receiver::None && side.is_none() {
        refusal = Some("its `self` has no API handle yet".to_owned());
    }
    let typed = sig.inputs.iter().filter_map(|input| match input {
        FnArg::Typed(typed) => Some(typed),
        FnArg::Receiver(_) => None,
    });
    for (index, typed) in typed.enumerate() {
        let index = index + 1;
        let arg = match &*typed.pat {
            Pat::Ident(pat) if pat.by_ref.is_none() && pat.subpat.is_none() => {
                if pat.ident == "cancel" || pat.ident == "result" {
                    return Err(syn::Error::new(
                        pat.ident.span(),
                        "a point's record has its own `cancel` and `result`: rename the parameter",
                    ));
                }
                api_arg(&pat.ident, &typed.ty, &mirrors).ok_or_else(|| {
                    format!(
                        "argument {index} `{}` (`{}`) has no API type yet",
                        pat.ident,
                        spelling(&*typed.ty)
                    )
                })
            }
            _ => Err(format!("argument {index} is a pattern, not a name")),
        };
        match arg {
            Ok(arg) => args.push(arg),
            Err(why) => {
                refusal.get_or_insert(why);
            }
        }
    }
    let borrowed_return = matches!(&sig.output, syn::ReturnType::Type(_, ty) if borrows(ty));
    // Without a receiver, a borrowed value may borrow an argument: the function must get it as
    // passed, never a listener's copy.
    if borrowed_return
        && receiver == Receiver::None
        && let Some(arg) = args.iter().find(|arg| arg.pass == Pass::Shared)
    {
        refusal.get_or_insert(format!(
            "the value it returns may borrow argument `{}`",
            arg.name
        ));
    }
    let returns = match &sig.output {
        syn::ReturnType::Default => Returns::Unit,
        syn::ReturnType::Type(_, ty) if unit_result(ty) => Returns::UnitResult,
        syn::ReturnType::Type(_, ty) if !borrows(ty) => match wide(ty) {
            Some(field) => Returns::Value(Value {
                ty: (**ty).clone(),
                field,
                mirror: false,
            }),
            None => owned(ty, &mirrors, true).map_or(Returns::Opaque, |(field, mirror)| {
                Returns::Value(Value {
                    ty: (**ty).clone(),
                    field,
                    mirror,
                })
            }),
        },
        syn::ReturnType::Type(..) => Returns::Opaque,
    };
    Ok(Shape {
        name,
        side,
        receiver,
        args,
        api_refusal: refusal,
        returns,
        borrowed_return,
        free,
        slot,
    })
}

/// A type the signature names that the record carries as a `mod_api` type.
struct Mirror {
    /// As the signature writes it, without spaces.
    written: String,
    api: Type,
    /// An engine type, converted through the game's mirror; else a `mod_api` type itself.
    convert: bool,
}

/// A type as tokens, without spaces: how a `mirror(…)` key names the type the signature writes.
pub fn spelling(tokens: &impl quote::ToTokens) -> String {
    tokens.to_token_stream().to_string().replace(' ', "")
}

/// The parameter `name: ty` as its listeners get it, or none when the API can't carry it.
fn api_arg(name: &syn::Ident, ty: &Type, mirrors: &[Mirror]) -> Option<Arg> {
    let (pass, pointee) = match ty {
        // A named lifetime (`&'static str`) is one a listener's copy can't have.
        Type::Reference(reference) if reference.lifetime.is_none() => (
            if reference.mutability.is_some() {
                Pass::Mut
            } else {
                Pass::Shared
            },
            &*reference.elem,
        ),
        Type::Reference(_) => return None,
        ty => (Pass::Value, ty),
    };
    let arg = |field, mirror, wide, text| {
        Some(Arg {
            name: name.clone(),
            ty: ty.clone(),
            pass,
            field,
            mirror,
            wide,
            text,
        })
    };
    if let Some(field) = wide(pointee) {
        return arg(field, false, Some(pointee.clone()), false);
    }
    match pointee {
        Type::Path(path) if pass == Pass::Shared && path.path.is_ident("str") => arg(
            syn::parse_quote!(::std::borrow::Cow<'a, str>),
            false,
            None,
            true,
        ),
        // `&[T]` is carried as a `Vec<T>`, and the function gets a slice of it.
        Type::Slice(slice) if pass == Pass::Shared => {
            let (elem, _) = owned(&slice.elem, mirrors, false)?;
            arg(
                syn::parse_quote!(::std::vec::Vec<#elem>),
                false,
                None,
                false,
            )
        }
        pointee => {
            let (field, mirror) = owned(pointee, mirrors, true)?;
            arg(field, mirror, None, false)
        }
    }
}

/// The record type of an owned value: scalars, `String`, `mod_api` types and the std containers
/// of them, and the engine types `mirrors` converts (only alone, or in an `Option` or `Vec`). The flag says whether it
/// holds a mirror. Every type it admits has a `Wire` layout.
fn owned(ty: &Type, mirrors: &[Mirror], mirror_here: bool) -> Option<(Type, bool)> {
    const SCALARS: [&str; 15] = [
        "bool", "char", "f32", "f64", "i8", "i16", "i32", "i64", "i128", "u8", "u16", "u32", "u64",
        "u128", "usize",
    ];
    match ty {
        Type::Paren(inner) => owned(&inner.elem, mirrors, mirror_here),
        Type::Group(inner) => owned(&inner.elem, mirrors, mirror_here),
        // `Wire` lays out tuples of up to six.
        Type::Tuple(tuple) if tuple.elems.len() <= 6 => {
            let mut elems = syn::punctuated::Punctuated::<Type, syn::Token![,]>::new();
            for elem in &tuple.elems {
                elems.push(owned(elem, mirrors, false)?.0);
            }
            if tuple.elems.len() == 1 {
                elems.push_punct(Default::default());
            }
            Some((
                Type::Tuple(syn::TypeTuple {
                    paren_token: tuple.paren_token,
                    elems,
                }),
                false,
            ))
        }
        Type::Array(array) => {
            let (elem, _) = owned(&array.elem, mirrors, false)?;
            let len = &array.len;
            matches!(
                len,
                syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Int(_),
                    ..
                })
            )
            .then(|| (syn::parse_quote!([#elem; #len]), false))
        }
        Type::Path(path) if path.qself.is_none() => {
            let written = spelling(&path.path);
            if let Some(mirror) = mirrors.iter().find(|mirror| mirror.written == written) {
                return match mirror.convert {
                    true => mirror_here.then(|| (mirror.api.clone(), true)),
                    false => Some((mirror.api.clone(), false)),
                };
            }
            let [segment] = path.path.segments.iter().collect::<Vec<_>>()[..] else {
                return None;
            };
            let name = segment.ident.to_string();
            let types: Vec<&Type> = match &segment.arguments {
                syn::PathArguments::None => Vec::new(),
                syn::PathArguments::AngleBracketed(args) => args
                    .args
                    .iter()
                    .map(|arg| match arg {
                        syn::GenericArgument::Type(ty) => Some(ty),
                        _ => None,
                    })
                    .collect::<Option<_>>()?,
                syn::PathArguments::Parenthesized(_) => return None,
            };
            let inner = |i: usize, mirror: bool| owned(types[i], mirrors, mirror);
            match (name.as_str(), types.len()) {
                (scalar, 0) if SCALARS.contains(&scalar) => Some((ty.clone(), false)),
                ("String", 0) => Some((syn::parse_quote!(::std::string::String), false)),
                ("Option", 1) => {
                    let (inner, mirror) = inner(0, mirror_here)?;
                    Some((syn::parse_quote!(::std::option::Option<#inner>), mirror))
                }
                ("Vec", 1) => {
                    let (inner, mirror) = inner(0, mirror_here)?;
                    Some((syn::parse_quote!(::std::vec::Vec<#inner>), mirror))
                }
                ("Box", 1) => {
                    let (inner, _) = inner(0, false)?;
                    Some((syn::parse_quote!(::std::boxed::Box<#inner>), false))
                }
                (set @ ("VecDeque" | "BTreeSet" | "HashSet"), 1) => {
                    let (inner, _) = inner(0, false)?;
                    let set = syn::Ident::new(set, Span::call_site());
                    Some((syn::parse_quote!(::std::collections::#set<#inner>), false))
                }
                (map @ ("BTreeMap" | "HashMap"), 2) => {
                    let ((key, _), (value, _)) = (inner(0, false)?, inner(1, false)?);
                    let map = syn::Ident::new(map, Span::call_site());
                    Some((
                        syn::parse_quote!(::std::collections::#map<#key, #value>),
                        false,
                    ))
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// The types a point's signature names as leaves: the ones an `api(…)` or `mirror(…)` key may name,
/// as spelled. The generator resolves each against the game's imports.
pub fn leaves(sig: &syn::Signature) -> Vec<syn::Path> {
    struct Leaves(Vec<syn::Path>);
    impl<'ast> syn::visit::Visit<'ast> for Leaves {
        fn visit_type_path(&mut self, path: &'ast syn::TypePath) {
            if path.qself.is_none()
                && path
                    .path
                    .segments
                    .iter()
                    .all(|segment| segment.arguments.is_none())
            {
                self.0.push(path.path.clone());
            }
            syn::visit::visit_type_path(self, path);
        }
    }
    let mut leaves = Leaves(Vec::new());
    for input in &sig.inputs {
        if let FnArg::Typed(typed) = input {
            syn::visit::Visit::visit_type(&mut leaves, &typed.ty);
        }
    }
    if let syn::ReturnType::Type(_, ty) = &sig.output {
        syn::visit::Visit::visit_type(&mut leaves, ty);
    }
    leaves.0
}

/// `Result<()>` (the game's `anyhow` alias) or `Result<(), E>`.
fn unit_result(ty: &Type) -> bool {
    let Type::Path(path) = ty else { return false };
    let Some(last) = path.path.segments.last() else {
        return false;
    };
    let syn::PathArguments::AngleBracketed(generics) = &last.arguments else {
        return false;
    };
    last.ident == "Result"
        && matches!(generics.args.first(), Some(syn::GenericArgument::Type(Type::Tuple(unit))) if unit.elems.is_empty())
}

/// The type a scalar crosses as in a record: `usize`/`isize` as `u64`/`i64`, one width on every
/// host and in a wasm guest (whose `usize` is 32-bit); none for a type that crosses as itself.
pub fn wide(ty: &Type) -> Option<Type> {
    let Type::Path(path) = ty else { return None };
    if path.path.is_ident("usize") {
        Some(syn::parse_quote!(u64))
    } else if path.path.is_ident("isize") {
        Some(syn::parse_quote!(i64))
    } else {
        None
    }
}

/// `tick_return` as `TickReturn`.
pub fn camel(name: &str) -> String {
    name.split('_')
        .map(|word| {
            let mut chars = word.chars();
            chars
                .next()
                .map(|first| first.to_ascii_uppercase().to_string() + chars.as_str())
                .unwrap_or_default()
        })
        .collect()
}

impl Shape {
    /// The `PointId` variant of its head: `Tick` for `tick`.
    pub fn variant(&self) -> syn::Ident {
        syn::Ident::new(&camel(&self.name), Span::call_site())
    }

    /// The `PointId` variant of its return.
    pub fn return_variant(&self) -> syn::Ident {
        syn::Ident::new(&camel(&format!("{}_return", self.name)), Span::call_site())
    }

    /// The record its listeners get, in `mod_points`: `TickArgs` for `tick`.
    pub fn record(&self) -> syn::Ident {
        syn::Ident::new(&format!("{}Args", camel(&self.name)), Span::call_site())
    }

    /// Whether the record borrows (a `&str` parameter).
    pub fn borrows(&self) -> bool {
        self.args.iter().any(|arg| arg.text)
    }

    /// Whether its API listeners have a record: every parameter has an API type.
    pub fn api(&self) -> bool {
        self.api_refusal.is_none()
    }

    /// The record's `cancel` or `result` field, with its type.
    fn outcome_field(&self) -> Option<(syn::Ident, TokenStream)> {
        match &self.returns {
            Returns::Unit | Returns::UnitResult => Some((format_ident!("cancel"), quote!(bool))),
            Returns::Value(value) => {
                let field = &value.field;
                Some((
                    format_ident!("result"),
                    quote!(::std::option::Option<#field>),
                ))
            }
            Returns::Opaque => None,
        }
    }

    /// The record's declaration in `mod_points`, documented from the function's `target`. Its
    /// fields hold only API types: scalars, text, std containers and `mod_api`'s own types.
    pub fn record_item(&self, target: &str) -> TokenStream {
        let record = self.record();
        let lifetime = if self.borrows() {
            quote!(<'a>)
        } else {
            quote!()
        };
        let names: Vec<_> = self.args.iter().map(|arg| &arg.name).collect();
        let fields: Vec<_> = self.args.iter().map(|arg| &arg.field).collect();
        let field_docs = self.args.iter().map(|arg| match arg.pass {
            Pass::Value => format!(
                "`{}`: a head listener may change it, and the function runs with the change.",
                arg.name
            ),
            Pass::Shared => format!(
                "`{}`, a copy: a head listener may change it, and the function reads the change.",
                arg.name
            ),
            Pass::Mut => format!(
                "`{}`, a copy of the place the function writes: a listener may change it, and the \
                 change is written to the place after the listener (at the head, before the body).",
                arg.name
            ),
        });
        let (extra, extra_param, extra_init) = match &self.returns {
            Returns::Unit | Returns::UnitResult => (
                quote! {
                    /// Set by a head listener: the function returns at once, skipping its body.
                    pub cancel: bool,
                },
                quote!(),
                quote!(cancel: false,),
            ),
            Returns::Value(value) => {
                let ty = &value.field;
                (
                    quote! {
                        /// Set by a head listener: the function returns this at once, skipping its body.
                        /// Return listeners find the value it returns here, and may change it.
                        pub result: Option<#ty>,
                    },
                    quote!(),
                    quote!(result: None,),
                )
            }
            Returns::Opaque => (quote!(), quote!(), quote!()),
        };
        let doc = format!(
            "The `{}` point's record: what its listeners get from `{target}`, a slot filled at load.",
            self.name
        );
        quote! {
            #[doc = #doc]
            #[non_exhaustive]
            #[cfg_attr(any(target_arch = "wasm32", feature = "wasm-guest"), derive(::mod_api::Wire))]
            pub struct #record #lifetime {
                #( #[doc = #field_docs] pub #names: #fields, )*
                #extra
            }
            impl #lifetime #record #lifetime {
                /// The record of a call, its fields already API values (the game's side of the point).
                #[doc(hidden)]
                #[inline]
                #[allow(clippy::too_many_arguments)]
                pub fn __new(#(#names: #fields,)* #extra_param) -> Self {
                    Self { #(#names,)* #extra_init }
                }
            }
        }
    }

    /// The record fields as a shared shape codec's nested mutable field list.
    pub fn wire_fields(&self, args: TokenStream) -> TokenStream {
        let mut fields: Vec<_> = self.args.iter().map(|arg| arg.name.clone()).collect();
        if let Some((name, _)) = self.outcome_field() {
            fields.push(name);
        }
        fields
            .iter()
            .rev()
            .fold(quote!(()), |tail, field| quote!((&mut #args.#field, #tail)))
    }

    /// Typed API function constructors, dispatchers and place ids for this point.
    pub fn api_items(&self, target: &str) -> ApiItems {
        let mut items = ApiItems::default();
        if !self.api() {
            return items;
        }
        let record = self.record();
        let record = if self.borrows() {
            quote!(#record<'_>)
        } else {
            quote!(#record)
        };
        let (host, passed, reborrow) = match (self.receiver, self.side) {
            (_, None) => (quote!(), quote!(), quote!()),
            (Receiver::Mut, Some(Side::Server)) => (
                quote!(host: &mut dyn ::mod_api::ServerHost,),
                quote!(&mut *host,),
                quote!(),
            ),
            (Receiver::Mut, Some(Side::Client)) => (
                quote!(host: &mut dyn ::mod_api::ClientHost,),
                quote!(&mut *host,),
                quote!(),
            ),
            (Receiver::Mut, Some(Side::Both)) => (
                quote!(host: ::mod_api::HostSide<'_>,),
                quote!(host.reborrow(),),
                quote!(let mut host = host;),
            ),
            (_, Some(_)) => (quote!(host: &dyn ::mod_api::Host,), quote!(host,), quote!()),
        };
        let forwarded_host = match (self.receiver, self.side) {
            (_, None) => quote!(super::__private::PointHost::None),
            (Receiver::Mut, Some(Side::Server)) => {
                quote!(super::__private::PointHost::Server(&mut *host))
            }
            (Receiver::Mut, Some(Side::Client)) => {
                quote!(super::__private::PointHost::Client(&mut *host))
            }
            (Receiver::Mut, Some(Side::Both)) => {
                quote!(super::__private::PointHost::Side(host.reborrow()))
            }
            (_, Some(_)) => quote!(super::__private::PointHost::Read(host)),
        };
        let wire_fields = self.wire_fields(quote!(args));
        let mut places = vec![(self.name.clone(), "head")];
        if self.has_return() {
            places.push((format!("{}_return", self.name), "return"));
        }
        for (method, place) in places {
            let listener = format_ident!("{}Listener", camel(&method));
            let variant = format_ident!("{}", camel(&method));
            let method = format_ident!("{method}");
            let doc = format!("`{target}`'s {place} function slot.");
            items.listeners.extend(quote! {
                #[doc = #doc]
                pub type #listener = fn(#host args: &mut #record);
                #[doc = #doc]
                pub const fn #method(id: &'static str, priority: i32, inject: &'static str, listener: #listener) -> super::__private::SlotListener {
                    // SAFETY: the alias checks this place's host and record before erasure.
                    unsafe { super::__private::SlotListener::api(id, PointId::#variant.place(), listener as *const ()) }.ordered(priority, inject)
                }
            });
            items.dispatch.extend(quote! {
                #[doc = #doc]
                pub fn #method(#host args: &mut #record) {
                    #reborrow
                    super::__private::fire_api(PointId::#variant, &mut |kind| match kind {
                        super::__private::ApiKind::Typed(function) => {
                            // SAFETY: the typed constructor checked this place's exact function type.
                            let function = unsafe { ::std::mem::transmute::<*const (), super::listeners::#listener>(function) };
                            function(#passed args);
                        }
                        super::__private::ApiKind::Forwarded(endpoint, id) => {
                            endpoint.forward(id, #forwarded_host, &mut ::mod_api::wire::Fields(#wire_fields));
                        }
                    });
                }
            });
            items.variants.extend(quote!(#variant,));
        }
        items
    }

    pub fn has_return(&self) -> bool {
        !self.borrowed_return
    }

    /// Whether it has a return place: for API listeners.
    pub fn at_return(&self) -> bool {
        self.has_return()
    }
}
