//! Rust as the surface writes it: one normalising function for types and signatures (paths
//! relative to `mod_api`, lifetimes kept, one spacing), and each type's wire layout description
//! for the shape function.
use super::wire_forms::WireForm;
use crate::index::{Found, Index, ItemId};
use mod_surface::shape::{Body, Definition, Scalar, Shape};
use proc_macro2::{Delimiter, Spacing, TokenStream, TokenTree};
use quote::ToTokens;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use syn::visit_mut::VisitMut;
use syn::{Item, Type};

/// The crates whose paths are written by their last segment (`Arc`, `HashMap`, `Duration`).
const STD: &[&str] = &["std", "core", "alloc"];

/// `tokens` with one spacing: a space after `,`, `;` and a lone `:`, around `->`, `=`, `+` and
/// `=>`, between two words, after a word-ending `>`, `)` or `]` before a word; none elsewhere.
/// A comma before a closing bracket or the end is dropped.
pub(super) fn print(tokens: TokenStream) -> String {
    #[derive(Clone, Copy, PartialEq)]
    enum Last {
        Start,
        Word,
        Close,
        Glue,
        Spaced,
    }
    fn walk(tokens: TokenStream, out: &mut String, last: &mut Last) {
        let trees: Vec<TokenTree> = tokens.into_iter().collect();
        let mut i = 0;
        while i < trees.len() {
            match &trees[i] {
                TokenTree::Ident(ident) => {
                    if matches!(*last, Last::Word | Last::Close) {
                        out.push(' ');
                    }
                    out.push_str(&ident.to_string());
                    *last = Last::Word;
                }
                TokenTree::Literal(literal) => {
                    if matches!(*last, Last::Word | Last::Close) {
                        out.push(' ');
                    }
                    out.push_str(&literal.to_string());
                    *last = Last::Word;
                }
                TokenTree::Group(group) => {
                    let (open, close) = match group.delimiter() {
                        Delimiter::Parenthesis => ("(", ")"),
                        Delimiter::Bracket => ("[", "]"),
                        Delimiter::Brace => (" { ", " }"),
                        Delimiter::None => ("", ""),
                    };
                    out.push_str(open);
                    let mut inner = Last::Start;
                    walk(group.stream(), out, &mut inner);
                    out.push_str(close);
                    *last = Last::Close;
                }
                TokenTree::Punct(punct) => {
                    // Gather a joint run (`::`, `->`, `'a`).
                    let mut run = punct.as_char().to_string();
                    let mut joint = punct.spacing() == Spacing::Joint;
                    while joint && i + 1 < trees.len() {
                        let TokenTree::Punct(next) = &trees[i + 1] else {
                            break;
                        };
                        if punct.as_char() == '\'' {
                            break;
                        }
                        run.push(next.as_char());
                        joint = next.spacing() == Spacing::Joint;
                        i += 1;
                    }
                    let end = trees.get(i + 1).is_none()
                        || matches!(trees.get(i + 1), Some(TokenTree::Punct(p)) if p.as_char() == '>');
                    match run.as_str() {
                        "," if end => {}
                        "," | ";" | ":" => {
                            out.push_str(&run);
                            out.push(' ');
                            *last = Last::Spaced;
                        }
                        "->" | "=" | "+" | "=>" => {
                            if !out.ends_with(' ') && !out.is_empty() {
                                out.push(' ');
                            }
                            out.push_str(&run);
                            out.push(' ');
                            *last = Last::Spaced;
                        }
                        "'" => {
                            if *last == Last::Word {
                                out.push(' ');
                            }
                            out.push('\'');
                            // The lifetime's name follows as a word glued to the quote.
                            *last = Last::Glue;
                        }
                        ">" => {
                            out.push('>');
                            *last = Last::Close;
                        }
                        "&" | "&&" if *last == Last::Word => {
                            out.push(' ');
                            out.push_str(&run);
                            *last = Last::Glue;
                        }
                        _ => {
                            out.push_str(&run);
                            *last = Last::Glue;
                        }
                    }
                }
            }
            i += 1;
        }
    }
    let mut out = String::new();
    walk(tokens, &mut out, &mut Last::Start);
    out.trim().to_owned()
}

/// Rewrites every path of a type or signature to the path a mod writes, collecting the ids of
/// the API types and traits it names.
pub(super) struct Paths<'p, 'a> {
    pub(super) index: &'p mut Index<'a>,
    pub(super) public: &'p HashMap<ItemId, Vec<String>>,
    pub(super) module: usize,
    /// Generic parameters in scope (and `Self`), written as they are.
    pub(super) params: Vec<String>,
    pub(super) refs: BTreeSet<String>,
}

/// The id prefix of a public item that a signature can name.
pub(super) fn ref_kind(item: &Item) -> Option<&'static str> {
    match item {
        Item::Struct(_) | Item::Enum(_) | Item::Union(_) | Item::Type(_) => Some("type"),
        Item::Trait(_) => Some("trait"),
        _ => None,
    }
}

impl Paths<'_, '_> {
    fn rewrite(&mut self, path: &mut syn::Path) {
        let segments: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();
        if path.leading_colon.is_none()
            && segments.len() == 1
            && self.params.iter().any(|p| *p == segments[0])
            || segments.first().is_some_and(|s| s == "Self")
        {
            return;
        }
        let written = match self.index.resolve_path(self.module, &segments) {
            Ok(Found::Item(id)) => match self.public.get(&id) {
                Some(public) => {
                    if let Some(kind) = ref_kind(self.index.item(id)) {
                        self.refs.insert(format!("{kind}:{}", public.join("::")));
                    }
                    public.clone()
                }
                None => vec![segments.last().cloned().unwrap_or_default()],
            },
            Ok(Found::External(full)) if STD.contains(&full[0].as_str()) => {
                vec![full.last().cloned().unwrap_or_default()]
            }
            Ok(Found::External(full)) => full,
            Err(_) => segments.clone(),
        };
        if written == segments {
            return;
        }
        let arguments = path
            .segments
            .last()
            .map(|s| s.arguments.clone())
            .unwrap_or(syn::PathArguments::None);
        let mut rewritten: syn::Path =
            syn::parse_str(&written.join("::")).unwrap_or_else(|_| path.clone());
        if let Some(last) = rewritten.segments.last_mut() {
            last.arguments = arguments;
        }
        rewritten.leading_colon = None;
        *path = rewritten;
    }
}

impl VisitMut for Paths<'_, '_> {
    fn visit_type_path_mut(&mut self, ty: &mut syn::TypePath) {
        syn::visit_mut::visit_type_path_mut(self, ty);
        if ty.qself.is_none() {
            self.rewrite(&mut ty.path);
        }
    }
    fn visit_trait_bound_mut(&mut self, bound: &mut syn::TraitBound) {
        syn::visit_mut::visit_trait_bound_mut(self, bound);
        self.rewrite(&mut bound.path);
    }
    fn visit_attribute_mut(&mut self, _: &mut syn::Attribute) {}
}

/// The generic parameter names of `generics`, lifetimes left out.
pub(super) fn param_names(generics: &syn::Generics) -> Vec<String> {
    generics
        .params
        .iter()
        .filter_map(|param| match param {
            syn::GenericParam::Type(t) => Some(t.ident.to_string()),
            syn::GenericParam::Const(c) => Some(c.ident.to_string()),
            syn::GenericParam::Lifetime(_) => None,
        })
        .collect()
}

/// Builds wire layout descriptions of the types the API's types contain, keyed by their
/// defining path, which never enters a shape. A type is described by how it crosses
/// ([`WireForm`]); one that does not cross, or crosses by a hand-written impl that declares no
/// layout, gets no definition, so every shape that contains it is refused.
pub(super) struct Layouts<'f> {
    pub(super) table: BTreeMap<String, Definition>,
    keys: HashMap<ItemId, String>,
    pending: Vec<ItemId>,
    forms: &'f HashMap<ItemId, WireForm>,
    ids: &'f HashMap<ItemId, Vec<String>>,
}

impl<'f> Layouts<'f> {
    pub(super) fn new(
        forms: &'f HashMap<ItemId, WireForm>,
        ids: &'f HashMap<ItemId, Vec<String>>,
    ) -> Self {
        Self {
            table: BTreeMap::new(),
            keys: HashMap::new(),
            pending: Vec::new(),
            forms,
            ids,
        }
    }

    /// The key of `id`'s definition, scheduling it to be described.
    pub(super) fn key(&mut self, index: &Index<'_>, id: ItemId) -> String {
        if let Some(key) = self.keys.get(&id) {
            return key.clone();
        }
        let module = &index.modules[id.module];
        let name = match index.item(id) {
            Item::Struct(i) => i.ident.to_string(),
            Item::Enum(i) => i.ident.to_string(),
            Item::Union(i) => i.ident.to_string(),
            _ => String::new(),
        };
        let key = std::iter::once(module.krate.to_owned())
            .chain(module.path.iter().cloned())
            .chain([name, format!("#{}", id.item)])
            .collect::<Vec<_>>()
            .join("::");
        self.keys.insert(id, key.clone());
        self.pending.push(id);
        key
    }

    /// The type a key names, for a note.
    pub(super) fn named(&self, key: &str) -> String {
        let name = key.rsplit_once("::#").map_or(key, |(name, _)| name);
        self.keys
            .iter()
            .find(|(_, k)| k.as_str() == key)
            .and_then(|(id, _)| self.ids.get(id))
            .map_or_else(|| name.to_owned(), |path| path.join("::"))
    }

    /// Describes every scheduled type and those they contain.
    pub(super) fn finish(&mut self, index: &mut Index<'_>) {
        while let Some(id) = self.pending.pop() {
            let key = self.keys[&id].clone();
            let item = index.item(id).clone();
            let (attrs, generics) = match &item {
                Item::Struct(s) => (&s.attrs, &s.generics),
                Item::Enum(e) => (&e.attrs, &e.generics),
                Item::Union(u) => (&u.attrs, &u.generics),
                _ => continue,
            };
            let names = param_names(generics);
            let derives = super::derives(attrs);
            let body = match super::wire_forms::form_of(id, attrs, &derives, self.forms) {
                None | Some(WireForm::Manual) => continue,
                Some(WireForm::Via(ty, module)) => {
                    Body::Via(self.shape(index, module, &ty, &names, 0))
                }
                Some(WireForm::Fields(fields, module)) => Body::Record(
                    fields
                        .iter()
                        .map(|(name, ty)| (name.clone(), self.shape(index, module, ty, &names, 0)))
                        .collect(),
                ),
                Some(WireForm::Derive) => match &item {
                    Item::Struct(s) => self.fields(index, id.module, &s.fields, &names),
                    Item::Enum(e) => Body::Enum(
                        e.variants
                            .iter()
                            .map(|v| {
                                (
                                    v.ident.to_string(),
                                    self.fields(index, id.module, &v.fields, &names),
                                )
                            })
                            .collect(),
                    ),
                    Item::Union(u) => {
                        let fields = syn::Fields::Named(u.fields.clone());
                        self.fields(index, id.module, &fields, &names)
                    }
                    _ => continue,
                },
            };
            self.table.insert(
                key,
                Definition {
                    params: names.len(),
                    body,
                },
            );
        }
    }

    /// A type the description does not look into, as the surface writes it.
    fn opaque(&self, index: &mut Index<'_>, module: usize, ty: &Type, params: &[String]) -> Shape {
        let mut ty = ty.clone();
        let mut paths = Paths {
            index,
            public: self.ids,
            module,
            params: params.to_vec(),
            refs: BTreeSet::new(),
        };
        paths.visit_type_mut(&mut ty);
        Shape::Opaque(print(ty.to_token_stream()))
    }

    fn fields(
        &mut self,
        index: &mut Index<'_>,
        module: usize,
        fields: &syn::Fields,
        params: &[String],
    ) -> Body {
        match fields {
            syn::Fields::Named(named) => Body::Record(
                named
                    .named
                    .iter()
                    .map(|f| {
                        (
                            f.ident
                                .as_ref()
                                .map(ToString::to_string)
                                .unwrap_or_default(),
                            self.shape(index, module, &f.ty, params, 0),
                        )
                    })
                    .collect(),
            ),
            syn::Fields::Unnamed(unnamed) => Body::Tuple(
                unnamed
                    .unnamed
                    .iter()
                    .map(|f| self.shape(index, module, &f.ty, params, 0))
                    .collect(),
            ),
            syn::Fields::Unit => Body::Unit,
        }
    }

    /// The layout description of `ty`, written in `module` with type parameters `params`.
    /// `depth` bounds alias expansion.
    pub(super) fn shape(
        &mut self,
        index: &mut Index<'_>,
        module: usize,
        ty: &Type,
        params: &[String],
        depth: usize,
    ) -> Shape {
        let boxed = |s: Shape| Box::new(s);
        match ty {
            Type::Paren(inner) => self.shape(index, module, &inner.elem, params, depth),
            Type::Group(inner) => self.shape(index, module, &inner.elem, params, depth),
            Type::Reference(r) => match &*r.elem {
                Type::Path(p) if p.path.is_ident("str") => Shape::Scalar(Scalar::Str),
                elem => self.shape(index, module, elem, params, depth),
            },
            Type::Slice(s) => Shape::List(boxed(self.shape(index, module, &s.elem, params, depth))),
            Type::Array(a) => match &a.len {
                syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Int(n),
                    ..
                }) => match n.base10_parse::<u64>() {
                    Ok(n) => {
                        Shape::Array(boxed(self.shape(index, module, &a.elem, params, depth)), n)
                    }
                    Err(_) => self.opaque(index, module, ty, params),
                },
                _ => self.opaque(index, module, ty, params),
            },
            Type::Tuple(t) if t.elems.is_empty() => Shape::Scalar(Scalar::Unit),
            Type::Tuple(t) => Shape::Tuple(
                t.elems
                    .iter()
                    .map(|e| self.shape(index, module, e, params, depth))
                    .collect(),
            ),
            Type::Path(p) if p.qself.is_none() => {
                self.path(index, module, ty, &p.path, params, depth)
            }
            _ => self.opaque(index, module, ty, params),
        }
    }

    fn path(
        &mut self,
        index: &mut Index<'_>,
        module: usize,
        ty: &Type,
        path: &syn::Path,
        params: &[String],
        depth: usize,
    ) -> Shape {
        let opaque = self.opaque(index, module, ty, params);
        let Some(last) = path.segments.last() else {
            return opaque;
        };
        let name = last.ident.to_string();
        let args: Vec<Type> = match &last.arguments {
            syn::PathArguments::AngleBracketed(a) => a
                .args
                .iter()
                .filter_map(|arg| match arg {
                    syn::GenericArgument::Type(t) => Some(t.clone()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        if path.segments.len() == 1
            && let Some(position) = params.iter().position(|p| *p == name)
        {
            return Shape::Param(position);
        }
        let segments: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();
        let resolved = index.resolve_path(module, &segments);
        if let Ok(Found::Item(id)) = resolved {
            match index.item(id).clone() {
                Item::Struct(_) | Item::Enum(_) | Item::Union(_) => {
                    let key = self.key(index, id);
                    let args = args
                        .iter()
                        .map(|a| self.shape(index, module, a, params, depth))
                        .collect();
                    return Shape::Named(key, args);
                }
                Item::Type(alias) if depth < 16 => {
                    // An alias is its target, its parameters replaced by the arguments.
                    let names = param_names(&alias.generics);
                    let mut target = (*alias.ty).clone();
                    let mut substitute = Substitute {
                        names: &names,
                        args: &args,
                    };
                    substitute.visit_type_mut(&mut target);
                    let _ = substitute;
                    return self.shape(index, id.module, &target, params, depth + 1);
                }
                _ => return opaque,
            }
        }
        let one = |this: &mut Self, index: &mut Index<'_>, i: usize| {
            args.get(i)
                .map(|a| this.shape(index, module, a, params, depth))
                .unwrap_or(Shape::Opaque(String::new()))
        };
        let scalar = match name.as_str() {
            "bool" => Some(Scalar::Bool),
            "char" => Some(Scalar::Char),
            "u8" => Some(Scalar::U8),
            "u16" => Some(Scalar::U16),
            "u32" => Some(Scalar::U32),
            "u64" => Some(Scalar::U64),
            "u128" => Some(Scalar::U128),
            "usize" => Some(Scalar::Usize),
            "i8" => Some(Scalar::I8),
            "i16" => Some(Scalar::I16),
            "i32" => Some(Scalar::I32),
            "i64" => Some(Scalar::I64),
            "i128" => Some(Scalar::I128),
            "isize" => Some(Scalar::Isize),
            "f32" => Some(Scalar::F32),
            "f64" => Some(Scalar::F64),
            "str" | "String" | "Cow" => Some(Scalar::Str),
            "Duration" => Some(Scalar::Duration),
            _ => None,
        };
        if let Some(scalar) = scalar {
            if name == "Cow"
                && !matches!(args.first(), Some(Type::Path(p)) if p.path.is_ident("str"))
            {
                return one(self, index, 0);
            }
            return Shape::Scalar(scalar);
        }
        match name.as_str() {
            "Vec" | "VecDeque" => Shape::List(Box::new(one(self, index, 0))),
            "HashSet" | "BTreeSet" | "IndexSet" => Shape::Set(Box::new(one(self, index, 0))),
            "HashMap" | "BTreeMap" | "IndexMap" => {
                Shape::Map(Box::new(one(self, index, 0)), Box::new(one(self, index, 1)))
            }
            "Option" => Shape::Option(Box::new(one(self, index, 0))),
            "Result" if args.len() == 2 => {
                Shape::Result(Box::new(one(self, index, 0)), Box::new(one(self, index, 1)))
            }
            "Box" | "Arc" | "Rc" => match args.first() {
                Some(Type::Path(p)) if p.path.is_ident("str") => Shape::Scalar(Scalar::Str),
                Some(Type::Slice(s)) => {
                    Shape::List(Box::new(self.shape(index, module, &s.elem, params, depth)))
                }
                Some(Type::TraitObject(_)) | None => opaque,
                Some(_) => one(self, index, 0),
            },
            _ => opaque,
        }
    }
}

/// Replaces an alias's type parameters by its arguments.
struct Substitute<'s> {
    names: &'s [String],
    args: &'s [Type],
}

impl VisitMut for Substitute<'_> {
    fn visit_type_mut(&mut self, ty: &mut Type) {
        if let Type::Path(p) = ty
            && p.qself.is_none()
            && p.path.segments.len() == 1
            && let Some(i) = self.names.iter().position(|n| p.path.is_ident(n))
            && let Some(arg) = self.args.get(i)
        {
            *ty = arg.clone();
            return;
        }
        syn::visit_mut::visit_type_mut(self, ty);
    }
}
