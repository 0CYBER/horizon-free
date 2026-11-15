//! Rust types of the mod API's signatures, resolved to the shapes that cross the WebAssembly
//! boundary.
use crate::index::{Found, Index, ItemId};
use quote::ToTokens;
use std::collections::HashMap;
use syn::{Fields, GenericArgument, Item, PathArguments, Type};

/// A Rust type as it crosses the shared-memory boundary.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Ty {
    Unit,
    /// A primitive, by its Rust spelling.
    Prim(&'static str),
    /// `u128`/`i128`, as two halves.
    Wide(&'static str),
    /// `std::time::Duration`, as nanoseconds.
    Duration,
    /// `String`, `&str`, `Box<str>`, `Arc<str>`: owned once lifted, except `&'static str`.
    Str(Str),
    List(Box<Ty>, List),
    Option(Box<Ty>),
    Result(Box<Ty>, Box<Ty>),
    Tuple(Vec<Ty>),
    Array(Box<Ty>, usize),
    /// `&T` or `&mut T`.
    Ref(Box<Ty>, bool),
    Boxed(Box<Ty>),
    /// `Arc<T>`, crossing as `T`.
    Shared(Box<Ty>),
    Named(usize),
    /// `&dyn Trait` or `&mut dyn Trait` (the reference is part of it): a frame resource.
    Dyn(String, bool),
    /// `Arc<dyn Trait>` or `Box<dyn Trait>` (`boxed`) of an exported trait: a handler the mod
    /// keeps, crossing as its index.
    Handler(String, bool),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Str {
    Owned,
    Borrowed,
    Static,
    Boxed,
    Shared,
    /// `Cow<'static, str>`.
    Cow,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum List {
    Vec,
    /// A map, as its container names it, crossing as a list of pairs.
    Map(&'static str),
    /// A set, as its container names it.
    Set(&'static str),
    Slice,
    Boxed,
    Shared,
}

/// A named type's definition.
#[derive(Debug)]
pub(crate) struct Def {
    /// Its path as the generated glue names it (`mod_api::BlockClick`).
    pub(super) rust: String,
    /// Its lifetime parameters: it borrows when there are any.
    pub(super) lifetimes: usize,
    pub(super) borrowed: BorrowedShape,
    pub(super) append: Option<super::append::Append>,
    pub(super) projection: Option<Vec<Field>>,
    pub(super) view: Option<View>,
}

impl Def {
    pub(super) fn borrow_fields(&self) -> Option<&[Field]> {
        if let Some(fields) = &self.projection {
            return Some(fields);
        }
        if let Some(append) = &self.append {
            Some(&append.fields)
        } else {
            self.borrowed.fields()
        }
    }
}
#[derive(Debug)]
pub(super) struct View {
    pub(super) field: String,
    pub(super) pointee: Ty,
    pub(super) mutable: bool,
    pub(super) snapshot: Ty,
}

#[derive(Debug)]
pub(crate) enum BorrowedShape {
    Record(Vec<Field>),
    TupleRecord(Vec<Field>),
    Variant(Vec<Case>),
    /// An owned value: its type owns the Wire implementation.
    None,
}

impl BorrowedShape {
    pub(super) fn fields(&self) -> Option<&[Field]> {
        match self {
            Self::Record(fields) | Self::TupleRecord(fields) => Some(fields),
            _ => None,
        }
    }
}
#[derive(Clone, Debug)]
pub(crate) struct Field {
    pub(super) rust: String,
    pub(super) ty: Ty,
}

#[derive(Clone, Debug)]
pub(crate) struct Case {
    pub(super) rust: String,
    pub(super) payload: Vec<Ty>,
}

/// The resolved types, by definition.
pub(crate) struct Types<'a> {
    pub(crate) index: Index<'a>,
    pub(super) defs: Vec<Def>,
    pub(super) by_item: HashMap<ItemId, usize>,
    /// The `mod_api` root module, where re-exported names are looked up.
    api_root: usize,
    /// Each exported trait with the exported traits its handlers answer as (`super::views`).
    pub(super) views: Vec<(String, Vec<String>)>,
    /// The definition being read, which `Self` in its fields names.
    current: Option<usize>,
}

impl<'a> Types<'a> {
    pub(super) fn new(mut index: Index<'a>) -> Result<Self, String> {
        let api_root = index.module("mod_api", &[])?;
        Ok(Self {
            index,
            views: Vec::new(),
            current: None,
            defs: Vec::new(),
            by_item: HashMap::new(),
            api_root,
        })
    }

    /// `ty`, written in `module`, with `subst` giving the generic parameters in scope.
    pub(super) fn ty(
        &mut self,
        module: usize,
        ty: &Type,
        subst: &[(String, Ty)],
    ) -> Result<Ty, String> {
        Ok(match ty {
            Type::Paren(ty) => self.ty(module, &ty.elem, subst)?,
            Type::Group(ty) => self.ty(module, &ty.elem, subst)?,
            Type::Tuple(tuple) if tuple.elems.is_empty() => Ty::Unit,
            Type::Tuple(tuple) => Ty::Tuple(
                tuple
                    .elems
                    .iter()
                    .map(|ty| self.ty(module, ty, subst))
                    .collect::<Result<_, _>>()?,
            ),
            Type::Array(array) => {
                let len = match &array.len {
                    syn::Expr::Lit(syn::ExprLit {
                        lit: syn::Lit::Int(len),
                        ..
                    }) => len.base10_parse().map_err(|e| e.to_string())?,
                    len => return Err(format!("array length {}", len.to_token_stream())),
                };
                Ty::Array(Box::new(self.ty(module, &array.elem, subst)?), len)
            }
            Type::Slice(slice) => {
                Ty::List(Box::new(self.ty(module, &slice.elem, subst)?), List::Slice)
            }
            Type::Reference(reference) => {
                let mutable = reference.mutability.is_some();
                let is_static = reference
                    .lifetime
                    .as_ref()
                    .is_some_and(|lifetime| lifetime.ident == "static");
                match &*reference.elem {
                    Type::TraitObject(object) => Ty::Dyn(trait_name(object)?, mutable),
                    Type::Path(path) if path.path.is_ident("str") => Ty::Str(if is_static {
                        Str::Static
                    } else {
                        Str::Borrowed
                    }),
                    Type::Slice(slice) if !mutable => {
                        Ty::List(Box::new(self.ty(module, &slice.elem, subst)?), List::Slice)
                    }
                    elem => Ty::Ref(Box::new(self.ty(module, elem, subst)?), mutable),
                }
            }
            Type::Path(path) if path.qself.is_none() => self.path(module, &path.path, subst)?,
            // A trait object behind a box or `Arc` (`Box<T>` with `T = dyn Handler`).
            Type::TraitObject(object) => Ty::Dyn(trait_name(object)?, false),
            ty => return Err(format!("unsupported type {}", ty.to_token_stream())),
        })
    }

    fn path(
        &mut self,
        module: usize,
        path: &syn::Path,
        subst: &[(String, Ty)],
    ) -> Result<Ty, String> {
        let last = path.segments.last().ok_or("empty path")?;
        let name = last.ident.to_string();
        if path.is_ident("Self")
            && let Some(current) = self.current
        {
            return Ok(Ty::Named(current));
        }
        if path.segments.len() == 1
            && let Some((_, ty)) = subst.iter().find(|(param, _)| *param == name)
        {
            return Ok(ty.clone());
        }
        let args = match &last.arguments {
            PathArguments::AngleBracketed(args) => args
                .args
                .iter()
                .filter_map(|arg| match arg {
                    GenericArgument::Type(ty) => Some(self.ty(module, ty, subst)),
                    _ => None,
                })
                .collect::<Result<Vec<_>, _>>()?,
            PathArguments::None => Vec::new(),
            PathArguments::Parenthesized(_) => return Err(format!("closure type {name}")),
        };
        let one = |args: &[Ty]| -> Result<Box<Ty>, String> {
            args.first()
                .cloned()
                .map(Box::new)
                .ok_or_else(|| format!("{name} needs a type argument"))
        };
        if let Some(prim) = primitive(&name) {
            return Ok(prim);
        }
        let segments: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();
        // Prelude and std types, found by name whatever path reaches them.
        let found = self.index.resolve_path(module, &segments);
        let external = match &found {
            Ok(Found::External(path)) => Some(path.last().cloned().unwrap_or_default()),
            Err(_) => Some(name.clone()),
            Ok(Found::Item(_)) => None,
        };
        if let Some(std) = external {
            return Ok(match std.as_str() {
                "String" => Ty::Str(Str::Owned),
                "Option" => Ty::Option(one(&args)?),
                "Vec" => Ty::List(one(&args)?, List::Vec),
                "Result" => {
                    let err = args.get(1).cloned().ok_or("Result needs an error type")?;
                    Ty::Result(one(&args)?, Box::new(err))
                }
                "Box" => match *one(&args)? {
                    Ty::Dyn(name, _) => Ty::Handler(name, true),
                    Ty::Str(_) => Ty::Str(Str::Boxed),
                    Ty::List(item, _) => Ty::List(item, List::Boxed),
                    ty => Ty::Boxed(Box::new(ty)),
                },
                "Arc" | "Rc" => match *one(&args)? {
                    Ty::Dyn(name, _) if std == "Arc" => Ty::Handler(name, false),
                    Ty::Str(_) => Ty::Str(Str::Shared),
                    Ty::List(item, _) => Ty::List(item, List::Shared),
                    ty if std == "Arc" => Ty::Shared(Box::new(ty)),
                    ty => return Err(format!("{std}<{ty:?}>")),
                },
                "Cow" => match *one(&args)? {
                    Ty::Str(Str::Borrowed) => Ty::Str(Str::Cow),
                    ty => return Err(format!("Cow<{ty:?}>")),
                },
                "HashSet" | "BTreeSet" => Ty::List(
                    one(&args)?,
                    List::Set(if std == "HashSet" {
                        "std::collections::HashSet"
                    } else {
                        "std::collections::BTreeSet"
                    }),
                ),
                "str" => Ty::Str(Str::Borrowed),
                "HashMap" | "BTreeMap" => {
                    let value = args.get(1).cloned().ok_or("map needs a value type")?;
                    let map = if std == "HashMap" {
                        "std::collections::HashMap"
                    } else {
                        "std::collections::BTreeMap"
                    };
                    Ty::List(
                        Box::new(Ty::Tuple(vec![*one(&args)?, value])),
                        List::Map(map),
                    )
                }
                "Value" | "Map" => Ty::Named(self.json(&segments)?),
                "Duration" => Ty::Duration,
                _ => return Err(format!("unsupported type {}", segments.join("::"))),
            });
        }
        let Ok(Found::Item(id)) = found else {
            unreachable!()
        };
        self.item(id, &name, args)
    }

    /// A JSON value crossing as its serde form.
    fn json(&mut self, segments: &[String]) -> Result<usize, String> {
        let name = segments.last().unwrap();
        let rust = match name.as_str() {
            "Value" => "serde_json::Value",
            _ => "serde_json::Map<String, serde_json::Value>",
        };
        if let Some(id) = self.defs.iter().position(|def| def.rust == rust) {
            return Ok(id);
        }
        Ok(self.push(Def {
            projection: None,
            append: None,
            view: None,
            rust: rust.into(),
            lifetimes: 0,
            borrowed: BorrowedShape::None,
        }))
    }

    fn push(&mut self, def: Def) -> usize {
        let id = self.defs.len();
        self.defs.push(def);
        id
    }

    /// The named item `id`, used with type arguments `args`.
    fn item(&mut self, id: ItemId, name: &str, args: Vec<Ty>) -> Result<Ty, String> {
        let item = self.index.item(id).clone();
        if let Item::Type(alias) = &item {
            let params: Vec<String> = alias
                .generics
                .type_params()
                .map(|p| p.ident.to_string())
                .collect();
            let subst: Vec<(String, Ty)> = params.into_iter().zip(args).collect();
            return self.ty(id.module, &alias.ty, &subst);
        }
        if let Item::Trait(_) = item {
            return Err(format!("trait {name} used as a type"));
        }
        if let Some(&def) = self.by_item.get(&id) {
            return Ok(Ty::Named(def));
        }
        let rust = self.public_path(id, name);
        let lifetimes = generics(&item).lifetimes().count();
        // Reserve the slot first: a type may refer to itself.
        let def = self.push(Def {
            projection: None,
            append: None,
            view: None,
            rust,
            lifetimes,
            borrowed: BorrowedShape::None,
        });
        self.by_item.insert(id, def);
        let outer = self.current.replace(def);
        let shape = if lifetimes == 0 {
            Ok(BorrowedShape::None)
        } else {
            self.borrowed_shape(id, &item)
        };
        self.current = outer;
        self.defs[def].borrowed = shape?;
        self.defs[def].append = super::append::detect(self, id, &item)?;
        self.defs[def].projection = self.projection(id, &item)?;
        self.defs[def].view = self.view(id, &item)?;
        Ok(Ty::Named(def))
    }

    /// Private readonly records opt in only through a type-owned typed constructor.
    fn projection(&mut self, id: ItemId, item: &Item) -> Result<Option<Vec<Field>>, String> {
        let Item::Struct(record) = item else {
            return Ok(None);
        };
        if record.generics.lifetimes().count() != 1 {
            return Ok(None);
        }
        let Fields::Named(fields) = &record.fields else {
            return Ok(None);
        };
        if fields
            .named
            .iter()
            .any(|f| !matches!(&f.vis, syn::Visibility::Restricted(v) if v.path.is_ident("crate")))
        {
            return Ok(None);
        }
        let constructor = self.index.modules.iter().filter(|m| m.krate == "mod_api").flat_map(|m| &m.items).find_map(|item| {
            let Item::Impl(i) = item else { return None; };
            if !matches!(&*i.self_ty, syn::Type::Path(p) if p.path.segments.last().is_some_and(|s| s.ident == record.ident)) { return None; }
            i.items.iter().find_map(|item| {
                let syn::ImplItem::Fn(f) = item else { return None; };
                (f.sig.ident == "__wire_from_fields" && matches!(f.vis, syn::Visibility::Public(_)) && f.attrs.iter().any(|a| matches!(&a.meta, syn::Meta::List(m) if m.path.is_ident("doc") && m.tokens.to_string() == "hidden"))).then(|| f.sig.clone())
            })
        });
        let Some(constructor) = constructor else {
            return Ok(None);
        };
        if constructor.inputs.len() != fields.named.len()
            || !matches!(&constructor.output, syn::ReturnType::Type(_, t) if matches!(&**t, syn::Type::Path(p) if p.path.is_ident("Self")))
        {
            return Err(format!(
                "{}: invalid borrowed projection constructor",
                record.ident
            ));
        }
        fn read_only(types: &Types<'_>, ty: &Ty) -> bool {
            match ty {
                Ty::Ref(_, true) | Ty::Dyn(..) => false,
                Ty::Ref(t, false)
                | Ty::Option(t)
                | Ty::Array(t, _)
                | Ty::List(t, _)
                | Ty::Boxed(t)
                | Ty::Shared(t) => read_only(types, t),
                Ty::Tuple(v) => v.iter().all(|t| read_only(types, t)),
                Ty::Result(a, b) => read_only(types, a) && read_only(types, b),
                Ty::Named(d) => types.defs[*d].lifetimes == 0,
                _ => true,
            }
        }
        let described = self.fields(id.module, fields.named.iter())?;
        for (field, argument) in described.iter().zip(&constructor.inputs) {
            let syn::FnArg::Typed(argument) = argument else {
                return Ok(None);
            };
            if !matches!(&*argument.pat, syn::Pat::Ident(p) if p.ident == field.rust)
                || self.ty(id.module, &argument.ty, &[])? != field.ty
            {
                return Err(format!(
                    "{}: projection constructor does not match fields",
                    record.ident
                ));
            }
            if !read_only(self, &field.ty) {
                return Ok(None);
            }
        }
        Ok(Some(described))
    }
    fn view(&mut self, id: ItemId, item: &Item) -> Result<Option<View>, String> {
        let Item::Struct(record) = item else {
            return Ok(None);
        };
        if record.generics.lifetimes().count() == 0 {
            return Ok(None);
        }
        let Fields::Named(fields) = &record.fields else {
            return Ok(None);
        };
        // A view may also hold private listener checkpoint state. Only its one
        // borrowed entity crosses the wire; the constructor initializes the rest.
        let borrowed_fields: Vec<_> = fields
            .named
            .iter()
            .filter(|field| matches!(field.ty, syn::Type::Reference(_)))
            .collect();
        let [field] = borrowed_fields[..] else {
            return Ok(None);
        };
        let syn::Type::Reference(reference) = &field.ty else {
            return Ok(None);
        };
        let name = &record.ident;
        let items = self.index.modules[id.module].items.clone();
        let ctor = items.iter().filter_map(|item| match item {
            Item::Impl(i) if matches!(&*i.self_ty, syn::Type::Path(p) if p.path.segments.last().is_some_and(|s| s.ident == *name)) => Some(i),
            _ => None,
        }).flat_map(|i| &i.items).any(|item| matches!(item,syn::ImplItem::Fn(f) if f.sig.ident == "new" && f.sig.inputs.len() == 1));
        if !ctor {
            return Ok(None);
        }
        let pointee = self.ty(id.module, &reference.elem, &[])?;
        let Ty::Named(d) = &pointee else {
            return Ok(None);
        };
        let target = self.defs[*d].rust.split("::").last().unwrap().to_string();
        let snapshot_arg = self.index.modules.iter().flat_map(|m| m.items.iter().map(move |i| (m,i))).find_map(|(module,item)| {
            let Item::Impl(i) = item else { return None; };
            if !matches!(&*i.self_ty,syn::Type::Path(p) if p.path.segments.last().is_some_and(|s| s.ident == target)) { return None; }
            i.items.iter().find_map(|item| {
                let syn::ImplItem::Fn(f) = item else { return None; };
                if f.sig.ident != "apply_snapshot" || f.sig.inputs.len() != 2 { return None; }
                let syn::FnArg::Typed(arg) = f.sig.inputs.last()? else { return None; };
                Some((module.path.clone(),(*arg.ty).clone()))
            })
        });
        let Some((module_path, arg)) = snapshot_arg else {
            return Ok(None);
        };
        let module = self
            .index
            .modules
            .iter()
            .position(|m| m.path == module_path)
            .unwrap();
        let snapshot = self.ty(module, &arg, &[])?;
        Ok(Some(View {
            field: field.ident.as_ref().unwrap().to_string(),
            pointee,
            mutable: reference.mutability.is_some(),
            snapshot,
        }))
    }

    /// Only borrowed call arguments need structural descriptions. Owned values are
    /// opaque to the generator and cross through their explicit Wire implementations.
    fn borrowed_shape(&mut self, id: ItemId, item: &Item) -> Result<BorrowedShape, String> {
        match item {
            Item::Struct(record) => match &record.fields {
                Fields::Named(fields)
                    if fields
                        .named
                        .iter()
                        .all(|f| matches!(f.vis, syn::Visibility::Public(_))) =>
                {
                    self.fields(id.module, fields.named.iter())
                        .map(BorrowedShape::Record)
                }
                Fields::Unnamed(fields)
                    if fields
                        .unnamed
                        .iter()
                        .all(|f| matches!(f.vis, syn::Visibility::Public(_))) =>
                {
                    fields
                        .unnamed
                        .iter()
                        .enumerate()
                        .map(|(i, f)| {
                            Ok(Field {
                                rust: i.to_string(),
                                ty: self.ty(id.module, &f.ty, &[])?,
                            })
                        })
                        .collect::<Result<_, String>>()
                        .map(BorrowedShape::TupleRecord)
                }
                _ => Ok(BorrowedShape::None),
            },
            Item::Enum(record) => {
                let mut cases = Vec::new();
                for case in &record.variants {
                    let Fields::Unnamed(fields) = &case.fields else {
                        return Ok(BorrowedShape::None);
                    };
                    let [field] = fields.unnamed.iter().collect::<Vec<_>>()[..] else {
                        return Ok(BorrowedShape::None);
                    };
                    let payload = self.ty(id.module, &field.ty, &[])?;
                    if !matches!(payload, Ty::Dyn(..)) {
                        return Ok(BorrowedShape::None);
                    }
                    cases.push(Case {
                        rust: case.ident.to_string(),
                        payload: vec![payload],
                    });
                }
                Ok(BorrowedShape::Variant(cases))
            }
            _ => Ok(BorrowedShape::None),
        }
    }

    fn fields<'f>(
        &mut self,
        module: usize,
        fields: impl Iterator<Item = &'f syn::Field>,
    ) -> Result<Vec<Field>, String> {
        fields
            .map(|f| {
                let rust = f.ident.as_ref().unwrap().to_string();
                Ok(Field {
                    ty: self.ty(module, &f.ty, &[])?,
                    rust,
                })
            })
            .collect()
    }

    /// The path generated glue uses for item `id`: `mod_api::Name` when `mod_api`'s root
    /// exports it, its defining module's path otherwise.
    fn public_path(&mut self, id: ItemId, name: &str) -> String {
        if let Ok(Found::Item(root)) = self.index.resolve(self.api_root, name)
            && root == id
        {
            return format!("mod_api::{name}");
        }
        let module = &self.index.modules[id.module];
        let mut path = vec![module.krate.to_string()];
        path.extend(module.path.iter().cloned());
        path.push(name.to_string());
        path.join("::")
    }
}

fn primitive(name: &str) -> Option<Ty> {
    Some(match name {
        "bool" => Ty::Prim("bool"),
        "u8" => Ty::Prim("u8"),
        "u16" => Ty::Prim("u16"),
        "u32" => Ty::Prim("u32"),
        "u64" => Ty::Prim("u64"),
        "usize" => Ty::Prim("usize"),
        "i8" => Ty::Prim("i8"),
        "i16" => Ty::Prim("i16"),
        "i32" => Ty::Prim("i32"),
        "i64" => Ty::Prim("i64"),
        "isize" => Ty::Prim("isize"),
        "f32" => Ty::Prim("f32"),
        "f64" => Ty::Prim("f64"),
        "char" => Ty::Prim("char"),
        "u128" => Ty::Wide("u128"),
        "i128" => Ty::Wide("i128"),
        _ => return None,
    })
}

/// A trait object's trait as the generator names it: its last path segment, with its type
/// arguments as written (`ResourceHandler<ItemResource>`).
pub(super) fn trait_name(object: &syn::TypeTraitObject) -> Result<String, String> {
    object
        .bounds
        .iter()
        .find_map(|bound| match bound {
            syn::TypeParamBound::Trait(bound) => bound.path.segments.last().map(|s| {
                let args = match &s.arguments {
                    PathArguments::AngleBracketed(args) => format!(
                        "<{}>",
                        args.args.iter().map(written).collect::<Vec<_>>().join(", ")
                    ),
                    _ => String::new(),
                };
                format!("{}{args}", s.ident)
            }),
            _ => None,
        })
        .ok_or_else(|| format!("dyn {}", object.to_token_stream()))
}

/// Tokens as written, without the spaces `quote` puts around punctuation
/// (`Option<Direction>`, `dyn ResourceHandler<ItemResource>`).
pub(super) fn written(tokens: &impl ToTokens) -> String {
    tokens
        .to_token_stream()
        .to_string()
        .replace(" < ", "<")
        .replace("< ", "<")
        .replace(" <", "<")
        .replace(" >", ">")
        .replace(" ,", ",")
        .replace("( )", "()")
}

fn generics(item: &Item) -> syn::Generics {
    match item {
        Item::Struct(item) => item.generics.clone(),
        Item::Enum(item) => item.generics.clone(),
        Item::Type(item) => item.generics.clone(),
        _ => syn::Generics::default(),
    }
}

pub(super) fn derives(item: &Item, name: &str) -> bool {
    let attrs = match item {
        Item::Struct(item) => &item.attrs,
        Item::Enum(item) => &item.attrs,
        _ => return false,
    };
    attrs.iter().any(|attr| {
        attr.path().is_ident("derive")
            && attr
                .to_token_stream()
                .to_string()
                .split(|c: char| !c.is_alphanumeric() && c != '_')
                .any(|word| word == name)
    })
}
#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "daily"]
    fn staged_identifier_exposure_does_not_add_backing_storage_to_wire_layouts() {
        std::thread::Builder::new().stack_size(64 << 20).spawn(|| {
            let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
            let generated = crate::sources(&root, crate::FillMode::Require).unwrap();
            let included = [
                ("mod_api_generated.rs", generated.hooks.as_str()),
                ("mod_api_generated_host.rs", generated.host.as_str()),
                ("mod_api_host_areas.rs", generated.areas.as_str()),
            ];
            let mut original_layout_metadata = None;
            for exposed in [false, true] {
                let mut types = super::Types::new(crate::index::Index::new(&root, &included)).unwrap();
                let module = types.index.module("horizon_core", &["identifier".into()]).unwrap();
                if exposed {
                    // Native internals lowering exposes types and members to internals
                    // listeners. That changes access, not the API's wire contract.
                    for item in &mut types.index.modules[module].items {
                        if let syn::Item::Struct(record) = item {
                            record.vis = syn::parse_quote!(pub);
                            for field in &mut record.fields { field.vis = syn::parse_quote!(pub); }
                        }
                    }
                }
                types.ty(module, &syn::parse_quote!(Identifier), &[]).unwrap();
                let layouts = types.defs.iter().filter(|def| def.lifetimes == 0).map(|def| def.rust.as_str()).collect::<Vec<_>>();
                assert_eq!(layouts, ["mod_api::Identifier"], "staged exposure = {exposed}: backing storage must never acquire a global Wire layout bound");
                // Global layout hashing includes the resolved definitions as well
                // as their Wire bounds. Visibility promotion must change neither.
                let metadata = format!("{:?}", types.defs);
                if let Some(original) = &original_layout_metadata {
                    assert_eq!(&metadata, original);
                } else {
                    original_layout_metadata = Some(metadata);
                }
            }
        }).unwrap().join().unwrap();
    }
}
