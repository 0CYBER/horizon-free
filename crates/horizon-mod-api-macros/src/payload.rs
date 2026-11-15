//! The `PayloadCodec` derive: a typed payload's value written field by field
//! through the bounded payload reader and writer.
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::parse::Parser;
use syn::{Data, DeriveInput, Fields, parse_quote};

struct UsedTypeParams<'a> {
    params: &'a std::collections::HashSet<String>,
    self_type: &'a str,
    found: bool,
    recursive: bool,
    used_params: std::collections::HashSet<String>,
}

impl<'ast> syn::visit::Visit<'ast> for UsedTypeParams<'_> {
    fn visit_type_path(&mut self, path: &'ast syn::TypePath) {
        self.recursive |= path
            .path
            .segments
            .iter()
            .any(|segment| segment.ident == self.self_type);
        if let Some(segment) = path.path.segments.first() {
            let ident = segment.ident.to_string();
            if self.params.contains(&ident) {
                self.found = true;
                self.used_params.insert(ident);
            }
        }
        syn::visit::visit_type_path(self, path);
    }

    fn visit_macro(&mut self, _mac: &'ast syn::Macro) {
        self.found = true;
    }
}

pub(crate) fn expand(mut input: DeriveInput) -> syn::Result<TokenStream> {
    let mut contract: Option<syn::Path> = None;
    let mut bound: Option<syn::LitStr> = None;
    for attr in &input.attrs {
        if attr.path().is_ident("payload") {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("crate") {
                    if contract.is_some() {
                        return Err(meta.error("expected one `crate = path`"));
                    }
                    contract = Some(meta.value()?.parse()?);
                } else if meta.path.is_ident("bound") {
                    if bound.is_some() {
                        return Err(meta.error("expected one `bound = \"...\"`"));
                    }
                    bound = Some(meta.value()?.parse()?);
                } else {
                    return Err(meta.error("expected `crate = path` or `bound = \"...\"`"));
                }
                Ok(())
            })?;
        }
    }
    let reject = |attrs: &[syn::Attribute]| -> syn::Result<()> {
        for attr in attrs {
            if attr.path().is_ident("payload") {
                return Err(syn::Error::new_spanned(
                    attr,
                    "payload attributes belong on the type",
                ));
            }
        }
        Ok(())
    };
    match &input.data {
        Data::Struct(data) => {
            for field in &data.fields {
                reject(&field.attrs)?;
            }
        }
        Data::Enum(data) => {
            for variant in &data.variants {
                reject(&variant.attrs)?;
                for field in &variant.fields {
                    reject(&field.attrs)?;
                }
            }
        }
        Data::Union(data) => {
            for field in &data.fields.named {
                reject(&field.attrs)?;
            }
        }
    }
    let c = contract.unwrap_or_else(|| parse_quote!(::mod_api));
    if let Some(bound) = bound {
        let parser =
            syn::punctuated::Punctuated::<syn::WherePredicate, syn::Token![,]>::parse_terminated;
        let predicates = parser
            .parse_str(&bound.value())
            .map_err(|error| syn::Error::new(bound.span(), error))?;
        input
            .generics
            .make_where_clause()
            .predicates
            .extend(predicates);
    } else {
        let generic_params: Vec<_> = input
            .generics
            .type_params()
            .map(|param| param.ident.clone())
            .collect();
        let type_params: std::collections::HashSet<_> =
            generic_params.iter().map(ToString::to_string).collect();
        let mut field_types: Vec<syn::Type> = Vec::new();
        match &input.data {
            Data::Struct(data) => {
                field_types.extend(data.fields.iter().map(|field| field.ty.clone()))
            }
            Data::Enum(data) => field_types.extend(
                data.variants
                    .iter()
                    .flat_map(|variant| variant.fields.iter().map(|field| field.ty.clone())),
            ),
            Data::Union(_) => {}
        }
        let mut predicates = std::collections::HashSet::new();
        let self_type = input.ident.to_string();
        for ty in field_types {
            let mut used = UsedTypeParams {
                params: &type_params,
                self_type: &self_type,
                found: false,
                recursive: false,
                used_params: std::collections::HashSet::new(),
            };
            syn::visit::Visit::visit_type(&mut used, &ty);
            if used.recursive {
                for param in &generic_params {
                    if used.used_params.contains(&param.to_string()) {
                        let predicate = quote!(#param: #c::PayloadCodec);
                        if predicates.insert(predicate.to_string()) {
                            input
                                .generics
                                .make_where_clause()
                                .predicates
                                .push(parse_quote!(#param: #c::PayloadCodec));
                        }
                    }
                }
            } else if used.found {
                let predicate = quote!(#ty: #c::PayloadCodec);
                if predicates.insert(predicate.to_string()) {
                    input
                        .generics
                        .make_where_clause()
                        .predicates
                        .push(parse_quote!(#ty: #c::PayloadCodec));
                }
            }
        }
    }
    let name = &input.ident;
    let (encode, decode, shape) = match &input.data {
        Data::Struct(data) => {
            let (pattern, encodes, build) = fields(&data.fields, quote!(Self), &c);
            let shape = shape(&data.fields, &c);
            (
                quote!(let Self #pattern = self; #encodes),
                quote!(::core::result::Result::Ok(#build)),
                quote!(let hash = #c::payload_schema_leaf("struct"); #shape hash),
            )
        }
        Data::Enum(data) => {
            if data.variants.is_empty() {
                return Err(syn::Error::new_spanned(
                    name,
                    "an enum with no variants has no payload form",
                ));
            }
            let mut encodes = Vec::new();
            let mut decodes = Vec::new();
            let mut shapes = Vec::new();
            for (tag, variant) in data.variants.iter().enumerate() {
                let tag = i32::try_from(tag)
                    .map_err(|_| syn::Error::new_spanned(variant, "too many variants"))?;
                let case = &variant.ident;
                let (pattern, writes, build) = fields(&variant.fields, quote!(Self::#case), &c);
                encodes.push(quote!(Self::#case #pattern => { out.var_int(#tag); #writes }));
                decodes.push(quote!(#tag => ::core::result::Result::Ok(#build),));
                shapes.push(shape(&variant.fields, &c));
            }
            let count = data.variants.len() as u64;
            (
                quote!(match self { #(#encodes)* }),
                quote! {
                    match input.var_int()? {
                        #(#decodes)*
                        value => ::core::result::Result::Err(#c::PayloadDecodeError::InvalidDiscriminant { value }),
                    }
                },
                quote! {
                    let hash = #c::payload_schema_mix(#c::payload_schema_leaf("enum"), #count);
                    #(#shapes)*
                    hash
                },
            )
        }
        Data::Union(_) => {
            return Err(syn::Error::new_spanned(name, "a union has no payload form"));
        }
    };
    let (imp, ty, clause) = input.generics.split_for_impl();
    Ok(quote! {
        #[allow(unused_variables)]
        #[automatically_derived]
        impl #imp #c::PayloadCodec for #name #ty #clause {
            fn encode(&self, out: &mut #c::PayloadWriter) { #encode }
            fn decode(
                input: &mut #c::PayloadReader<'_>,
            ) -> ::core::result::Result<Self, #c::PayloadDecodeError> {
                #decode
            }
            fn schema(visiting: &mut ::std::vec::Vec<::core::any::TypeId>) -> u64
            where
                Self: 'static,
            {
                #c::payload_schema_named::<Self>(visiting, |visiting| { #shape })
            }
        }
    })
}

/// The statements that add `fields` to `hash`: their count, then each field's schema in
/// declaration order. Names are not part of the schema.
fn shape(fields: &Fields, c: &syn::Path) -> TokenStream {
    let count = fields.len() as u64;
    let types = fields.iter().map(|field| &field.ty);
    quote! {
        let hash = #c::payload_schema_mix(hash, #count);
        #(let hash = #c::payload_schema_mix(hash, <#types as #c::PayloadCodec>::schema(visiting));)*
    }
}

/// The pattern that binds `fields`, the statements that encode the bindings
/// in declaration order, and the expression that decodes them into `prefix`.
fn fields(
    fields: &Fields,
    prefix: TokenStream,
    c: &syn::Path,
) -> (TokenStream, TokenStream, TokenStream) {
    let binds: Vec<_> = (0..fields.len())
        .map(|index| format_ident!("__payload_f_{index}"))
        .collect();
    let types: Vec<_> = fields.iter().map(|field| &field.ty).collect();
    let encodes = quote!(#(<#types as #c::PayloadCodec>::encode(#binds, out);)*);
    let decodes = types
        .iter()
        .map(|ty| quote!(<#ty as #c::PayloadCodec>::decode(input)?));
    match fields {
        Fields::Named(named) => {
            let names: Vec<_> = named.named.iter().map(|field| &field.ident).collect();
            (
                quote!({ #(#names: #binds),* }),
                encodes,
                quote!(#prefix { #(#names: #decodes),* }),
            )
        }
        Fields::Unnamed(_) => (
            quote!((#(#binds),*)),
            encodes,
            quote!(#prefix (#(#decodes),*)),
        ),
        Fields::Unit => (quote!(), encodes, prefix),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_expand_to_rust_items() {
        for source in [
            "struct Named { a: i32, b: String }",
            "struct Tuple(i32, String);",
            "struct Unit;",
            "struct Shadow { out: i32, input: u8 }",
            "enum Plain { A = 7, B = 9 }",
            "enum Mixed { A(i32), B { text: String }, C }",
            "struct Generic<T> { value: Vec<T> }",
            "struct Recursive { children: Vec<Recursive> }",
        ] {
            let expanded = expand(syn::parse_str(source).unwrap()).unwrap();
            syn::parse2::<syn::ItemImpl>(expanded).unwrap_or_else(|e| panic!("{source}: {e}"));
        }
    }

    #[test]
    fn shapes_without_a_payload_form_are_refused() {
        for source in [
            "enum Empty {}",
            "struct Bad { #[payload(with = other)] a: u8 }",
            "enum Bad { #[payload(with = other)] A }",
            "enum Bad { A(#[payload(with = other)] u8) }",
            "union Raw { a: i32, b: u32 }",
            "#[payload(with = other)] struct Unknown;",
            "#[payload(crate = ::a, crate = ::b)] struct Twice;",
            "#[payload(bound = \"not a predicate\")] struct BadBound;",
        ] {
            assert!(expand(syn::parse_str(source).unwrap()).is_err(), "{source}");
        }
    }
}
