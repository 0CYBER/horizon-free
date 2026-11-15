//! The owned-value Wire derive. Call-table borrowing is deliberately not handled here.
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Data, DeriveInput, Fields, Type, parse_quote, visit::Visit};

pub(crate) fn expand(mut input: DeriveInput) -> syn::Result<TokenStream> {
    let name = &input.ident;
    let mut via: Option<Type> = None;
    let mut contract: Option<syn::Path> = None;
    let mut internal_field_layouts = false;
    for attr in &input.attrs {
        if attr.path().is_ident("wire") {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("crate") {
                    if contract.is_some() {
                        return Err(meta.error("duplicate Wire contract path"));
                    }
                    contract = Some(meta.value()?.parse()?);
                    return Ok(());
                }
                if meta.path.is_ident("internal_field_layouts") && !internal_field_layouts {
                    internal_field_layouts = true;
                    return Ok(());
                }
                if !meta.path.is_ident("via") || via.is_some() {
                    return Err(meta.error("expected one `via = Type`"));
                }
                via = Some(meta.value()?.parse()?);
                Ok(())
            })?;
        }
    }
    let w = contract.unwrap_or_else(|| parse_quote!(::mod_api::wire));
    if via.is_some() && internal_field_layouts {
        return Err(syn::Error::new_spanned(
            name,
            "internal_field_layouts requires a structural derive",
        ));
    }
    if via.is_none() {
        for param in input.generics.type_params_mut() {
            param.bounds.push(parse_quote!(#w::Wire));
        }
    }
    if let Some(via) = &via {
        let where_clause = input.generics.make_where_clause();
        where_clause.predicates.push(parse_quote!(#via: #w::Wire));
        where_clause
            .predicates
            .push(parse_quote!(#via: for<'__wire> ::core::convert::From<&'__wire Self>));
        where_clause
            .predicates
            .push(parse_quote!(Self: ::core::convert::TryFrom<#via>));
    }
    let (imp, ty, clause) = input.generics.split_for_impl();
    if let Some(via) = via {
        return Ok(quote! {
            #[automatically_derived]
            impl #imp #w::Wire for #name #ty #clause {
                const MIN: usize = <#via as #w::Wire>::MIN;
                const LAYOUT_SEED: u64 = #w::layout_name("via");
                fn layout(visiting: &mut Vec<&'static str>) -> u64 {
                    <#via as #w::Wire>::layout(visiting)
                }
                fn put(&self, out: &mut #w::Writer) {
                    out.put(&<#via as ::core::convert::From<&Self>>::from(self));
                }
                fn put_owned(self, out: &mut #w::Writer) { self.put(out); }
                fn take(input: &mut #w::Reader<'_>) -> Result<Self, #w::WireError> {
                    <Self as ::core::convert::TryFrom<#via>>::try_from(input.take::<#via>()?)
                        .map_err(|_| #w::WireError::Invalid(stringify!(#name)))
                }
            }
        });
    }
    let mut layout = quote!(#w::layout_name(stringify!(#name)));
    let mut min = quote!(0usize);
    let mut puts = Vec::new();
    let mut owned = Vec::new();
    let mut takes = Vec::new();
    let shape;
    let nested;
    let mut schema = Vec::new();
    match &input.data {
        Data::Struct(data) => {
            shape = match &data.fields {
                Fields::Named(_) => "record",
                Fields::Unnamed(_) => "tuple",
                Fields::Unit => "unit",
            };
            nested = !matches!(data.fields, Fields::Unit);
            let (pattern, build, fields) = fields(&data.fields);
            for (index, (field, bind)) in data.fields.iter().zip(&fields).enumerate() {
                let label = field
                    .ident
                    .as_ref()
                    .map_or_else(|| index.to_string(), ToString::to_string);
                let ft = &field.ty;
                schema.push(quote!((#label, <#ft as #w::Wire>::layout(&mut vec![]))));
                check_owned(&field.ty)?;
                let ft = &field.ty;
                min = quote!(#min + <#ft as #w::Wire>::MIN);
                layout = field_layout(layout, field, &w);
                puts.push(quote!(out.put(#bind);));
                owned.push(quote!(out.put_owned(#bind);));
            }
            takes.push(quote!(#build));
            puts = vec![quote!(let Self #pattern = self; #(#puts)*)];
            owned = vec![quote!(let Self #pattern = self; #(#owned)*)];
        }
        Data::Enum(data) => {
            shape = "enum";
            schema.push(quote!(("tag", <u32 as #w::Wire>::layout(&mut vec![]))));
            min = quote!(4usize);
            nested = data
                .variants
                .iter()
                .any(|v| !matches!(v.fields, Fields::Unit));
            for (index, variant) in data.variants.iter().enumerate() {
                let index = index as u32;
                let case = &variant.ident;
                layout = quote!(#w::layout_mix(#layout, #w::layout_name(stringify!(#case))));
                let (pat, _, binds) = fields(&variant.fields);
                let pat = quote!(Self::#case #pat);
                // `fields` returns a constructor beginning with Self; enum cases supply that prefix.
                let build = construct(&variant.fields, quote!(Self::#case));
                let mut ps = Vec::new();
                let mut os = Vec::new();
                if matches!(variant.fields, Fields::Unit) {
                    let label = format!("{index}:{case}");
                    schema.push(quote!((#label, <() as #w::Wire>::layout(&mut vec![]))));
                }
                for (field_index, (field, bind)) in variant.fields.iter().zip(binds).enumerate() {
                    let field_name = field
                        .ident
                        .as_ref()
                        .map_or_else(|| field_index.to_string(), ToString::to_string);
                    let label = format!("{index}:{case}.{field_name}");
                    let ft = &field.ty;
                    schema.push(quote!((#label, <#ft as #w::Wire>::layout(&mut vec![]))));
                    check_owned(&field.ty)?;
                    layout = field_layout(layout, field, &w);
                    ps.push(quote!(out.put(#bind);));
                    os.push(quote!(out.put_owned(#bind);));
                }
                puts.push(quote!(#pat => { out.put(&#index); #(#ps)* }));
                owned.push(quote!(#pat => { out.put(&#index); #(#os)* }));
                takes.push(quote!(#index => #build,));
            }
            puts = vec![quote!(match self { #(#puts,)* _ => unreachable!() })];
            owned = vec![quote!(match self { #(#owned,)* _ => unreachable!() })];
            takes = vec![
                quote!(match input.take::<u32>()? { #(#takes)* tag => return Err(#w::WireError::Tag(tag)) }),
            ];
        }
        Data::Union(_) => return Err(syn::Error::new_spanned(name, "Wire cannot encode a union")),
    }
    layout = quote!(#w::layout_mix(#w::layout_name(#shape), #layout));
    // Unit enum variants have no child layouts or recursion to evaluate. Fold
    // their constant hash chain here rather than leaving its loops to LLVM.
    if matches!(&input.data, Data::Enum(_)) && !nested {
        layout = quote!(const { #layout });
    }
    let take = if nested {
        quote!(input.nested(|input| Ok(#(#takes)*)))
    } else {
        quote!(Ok(#(#takes)*))
    };
    let put = if nested {
        quote!(out.nested(|out| { #(#puts)* }))
    } else {
        quote!(#(#puts)*)
    };
    let put_owned = if nested {
        quote!(out.nested(|out| { #(#owned)* }))
    } else {
        quote!(#(#owned)*)
    };
    // Explicit internal opt-in: do not emit a cfg(test) trait method in external
    // derives, whose mod_api dependency is compiled without cfg(test).
    let schema = internal_field_layouts.then(|| {
        quote! {
            #[cfg(test)]
            fn __field_layouts() -> Option<Vec<(&'static str, u64)>> { Some(vec![#(#schema),*]) }
        }
    });
    Ok(quote! {
        #[allow(unreachable_patterns, unused_variables)]
        #[automatically_derived]
            impl #imp #w::Wire for #name #ty #clause {
            #schema
            const MIN: usize = #min;
            const LAYOUT_SEED: u64 = #w::layout_name(stringify!(#name));
            fn layout(visiting: &mut Vec<&'static str>) -> u64 {
                let identity = ::core::any::type_name::<Self>();
                if visiting.contains(&identity) { return #w::layout_name("recursive"); }
                visiting.push(identity);
                let hash = #layout;
                visiting.pop();
                hash
            }
            fn put(&self, out: &mut #w::Writer) { #put }
            fn put_owned(self, out: &mut #w::Writer) { #put_owned }
            fn take(input: &mut #w::Reader<'_>) -> Result<Self, #w::WireError> { #take }
        }
    })
}

fn fields(fields: &Fields) -> (TokenStream, TokenStream, Vec<syn::Ident>) {
    let binds: Vec<_> = fields
        .iter()
        .enumerate()
        .map(|(i, _)| format_ident!("__wire_f_{i}"))
        .collect();
    let pattern = match fields {
        Fields::Named(fields) => {
            let names = fields.named.iter().map(|f| &f.ident);
            quote!({ #(#names: #binds),* })
        }
        Fields::Unnamed(_) => quote!((#(#binds),*)),
        Fields::Unit => quote!(),
    };
    (pattern, construct(fields, quote!(Self)), binds)
}

fn construct(fields: &Fields, prefix: TokenStream) -> TokenStream {
    match fields {
        Fields::Named(fields) => {
            let names = fields.named.iter().map(|f| &f.ident);
            quote!(#prefix { #(#names: input.take()?),* })
        }
        Fields::Unnamed(fields) => {
            let takes = fields.unnamed.iter().map(|_| quote!(input.take()?));
            quote!(#prefix (#(#takes),*))
        }
        Fields::Unit => prefix,
    }
}

fn field_layout(previous: TokenStream, field: &syn::Field, w: &syn::Path) -> TokenStream {
    let ty = &field.ty;
    let name = field
        .ident
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_default();
    quote!(#w::layout_mix(#w::layout_mix(#previous, #w::layout_name(#name)), <#ty as #w::Wire>::layout(visiting)))
}

fn check_owned(ty: &Type) -> syn::Result<()> {
    struct Borrowed(Option<syn::Error>);
    impl<'ast> Visit<'ast> for Borrowed {
        fn visit_type_path(&mut self, p: &'ast syn::TypePath) {
            if p.path.segments.last().is_some_and(|s| s.ident == "Cow") {
                if let syn::PathArguments::AngleBracketed(args) =
                    &p.path.segments.last().unwrap().arguments
                {
                    for arg in &args.args {
                        if let syn::GenericArgument::Type(t) = arg {
                            self.visit_type(t);
                        }
                    }
                }
                return;
            }
            syn::visit::visit_type_path(self, p);
        }
        fn visit_lifetime(&mut self, lifetime: &'ast syn::Lifetime) {
            if lifetime.ident != "static" {
                self.0 = Some(syn::Error::new_spanned(
                    lifetime,
                    "Wire is for owned values; a borrowed record crosses as a call argument",
                ));
            }
        }
        fn visit_type_reference(&mut self, r: &'ast syn::TypeReference) {
            let static_str = r.mutability.is_none()
                && r.lifetime.as_ref().is_some_and(|l| l.ident == "static")
                && matches!(&*r.elem, Type::Path(p) if p.path.is_ident("str"));
            if !static_str {
                self.0 = Some(syn::Error::new_spanned(
                    r,
                    "Wire is for owned values; a borrowed record crosses as a call argument",
                ));
            }
        }
    }
    let mut check = Borrowed(None);
    check.visit_type(ty);
    check.0.map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_expand_to_rust_items() {
        for source in [
            "struct Named { a: u32, b: String }",
            "struct Tuple(u32, String);",
            "struct Unit;",
            "enum Plain { A = 7, B = 9 }",
            "enum Empty {}",
            "struct Shadow { out: u32, input: u8 }",
            "enum Payload { A(u32), B { text: String }, C }",
            "struct Generic<T, const N: usize> { value: [T; N] }",
            "struct Recursive { children: Vec<Recursive> }",
            "#[wire(via = String)] struct Checked(String);",
            "struct Static { text: &'static str }",
        ] {
            let expanded = expand(syn::parse_str(source).unwrap()).unwrap();
            syn::parse2::<syn::ItemImpl>(expanded).unwrap_or_else(|e| panic!("{source}: {e}"));
        }
    }

    #[test]
    fn contract_path_reaches_bounds_fields_and_via() {
        for source in [
            "#[wire(crate = ::mod_vocab::wire)] struct Generic<T> { value: T }",
            "#[wire(crate = ::mod_vocab::wire, via = String)] struct Checked(String);",
            "#[wire(crate = ::mod_vocab::wire)] enum Payload { A(String), B }",
        ] {
            let output = expand(syn::parse_str(source).unwrap()).unwrap();
            syn::parse2::<syn::ItemImpl>(output.clone()).unwrap();
            let output = output.to_string();
            assert!(output.contains("mod_vocab"));
            assert!(!output.contains("mod_api"));
        }
        assert!(
            expand(syn::parse_quote!(
                #[wire(crate = a, crate = b)]
                struct Duplicate;
            ))
            .is_err()
        );
    }

    #[test]
    fn field_schema_is_opt_in_for_internal_api_tests_only() {
        let ordinary = expand(syn::parse_quote!(
            struct Ordinary {
                value: u32,
            }
        ))
        .unwrap();
        let internal = expand(syn::parse_quote!(
            #[wire(internal_field_layouts)]
            enum Internal {
                Unit,
                Tuple(u32),
                Record { value: bool },
            }
        ))
        .unwrap();
        let ordinary: syn::ItemImpl = syn::parse2(ordinary).unwrap();
        let internal: syn::ItemImpl = syn::parse2(internal).unwrap();
        let schema = |item: &syn::ImplItem| matches!(item, syn::ImplItem::Fn(f) if f.sig.ident == "__field_layouts");
        assert!(!ordinary.items.iter().any(schema));
        let method = internal.items.iter().find(|item| schema(item)).unwrap();
        let syn::ImplItem::Fn(method) = method else {
            unreachable!()
        };
        assert!(method.attrs.iter().any(|attr| attr.path().is_ident("cfg")));
    }

    #[test]
    fn borrowed_fields_are_rejected_even_inside_containers() {
        for source in [
            "struct Borrow<'a> { value: &'a str }",
            "struct Borrow<'a> { value: Vec<&'a u8> }",
            "struct Borrow { value: &'static [u8] }",
            "struct Borrow { value: &'static mut str }",
            "struct Borrow<'a> { value: Option<Other<'a>> }",
        ] {
            let err = expand(syn::parse_str(source).unwrap()).unwrap_err();
            assert!(err.to_string().contains("Wire is for owned values"));
        }
    }

    #[test]
    fn union_and_unknown_options_are_rejected() {
        for source in [
            "union Union { a: u32, b: u8 }",
            "#[wire(unknown)] struct Unknown;",
            "#[wire(via = String, via = u8)] struct Duplicate;",
            "#[wire(via = String, internal_field_layouts)] struct NonStructural;",
            "#[wire(internal_field_layouts, internal_field_layouts)] struct DuplicateSchema;",
        ] {
            assert!(expand(syn::parse_str(source).unwrap()).is_err());
        }
    }
}
