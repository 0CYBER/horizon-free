//! `#[point]`: an injection point's head check, record copy-in and copy-out.
use point_shape::{Arg, Pass, Receiver, Returns, Side};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::ImplItemFn;

/// The parameter local the outer function binds parameter `input` (receiver included) to.
fn local(input: usize) -> syn::Ident {
    format_ident!("__p{input}")
}

/// The local holding a listener's copy of a `&T` parameter, and the reference the body gets.
fn shared(input: usize) -> (syn::Ident, syn::Ident) {
    (format_ident!("__s{input}"), format_ident!("__r{input}"))
}

/// The API record's field value of `arg`, bound at `input`, before the body (`fresh` after it,
/// where a `&T` is what the function read). Mirrors take the converted value `__m<input>`.
fn copy_in(arg: &Arg, input: usize, after: bool) -> TokenStream {
    let p = local(input);
    let (_, r) = shared(input);
    let m = format_ident!("__m{input}");
    let field = &arg.field;
    if arg.mirror {
        return quote!(#m);
    }
    match arg.pass {
        Pass::Value if arg.wide.is_some() => quote!(#p as #field),
        Pass::Value if after => quote!(#m),
        Pass::Value => quote!(#p),
        Pass::Shared if arg.text && after => quote!(::std::borrow::Cow::Borrowed(#r)),
        Pass::Shared if arg.text => quote!(::std::borrow::Cow::Borrowed(#p)),
        Pass::Shared => {
            let source = if after { quote!(#r) } else { quote!(#p) };
            if arg.wide.is_some() {
                quote!(*#source as #field)
            } else if matches!(&arg.ty, syn::Type::Reference(r) if matches!(*r.elem, syn::Type::Slice(_)))
            {
                quote!(#source.to_vec())
            } else {
                quote!(::std::clone::Clone::clone(#source))
            }
        }
        Pass::Mut if arg.wide.is_some() => quote!(*#p as #field),
        Pass::Mut => quote!(::std::clone::Clone::clone(&*#p)),
    }
}

/// The mirror conversion of `arg` (only a mirror has one), from what the record copies.
fn mirror_in(arg: &Arg, input: usize, after: bool) -> Option<(syn::Ident, TokenStream)> {
    arg.mirror.then(|| {
        let p = local(input);
        let (_, r) = shared(input);
        let source = match (arg.pass, after) {
            (Pass::Value, _) => quote!(&#p),
            (Pass::Shared, true) => quote!(#r),
            (Pass::Shared, false) => quote!(#p),
            (Pass::Mut, _) => quote!(&*#p),
        };
        (
            format_ident!("__m{input}"),
            quote!(crate::game::points::mirror_in(#source)),
        )
    })
}

/// Writes a listener's change of `arg` back where the function reads it. `point` names the
/// point in the log line of a value the engine refuses.
fn copy_out(arg: &Arg, input: usize, point: &str) -> TokenStream {
    let p = local(input);
    let (s, _) = shared(input);
    let name = &arg.name;
    let value = quote!(__args.#name);
    let ty = match (&arg.wide, &arg.ty) {
        (Some(wide), _) => quote!(#wide),
        _ => quote!(_),
    };
    let target = match arg.pass {
        Pass::Value => quote!(#p),
        Pass::Mut => quote!(*#p),
        Pass::Shared => quote!(#s),
    };
    let wrap = |value: TokenStream| match arg.pass {
        Pass::Shared => quote!(::std::option::Option::Some(#value)),
        _ => value,
    };
    if arg.mirror {
        let converted = wrap(quote!(__value));
        quote! {
            if let ::std::option::Option::Some(__value) = crate::game::points::mirror_out(&#value, #point) {
                #target = #converted;
            }
        }
    } else if arg.text {
        quote! {
            if let ::std::borrow::Cow::Owned(__value) = #value {
                #target = ::std::option::Option::Some(__value);
            }
        }
    } else if arg.wide.is_some() {
        let converted = wrap(quote!(#value as #ty));
        quote!(#target = #converted;)
    } else {
        let converted = wrap(value);
        quote!(#target = #converted;)
    }
}
/// `#[point]` on a function: see the attribute's documentation in the crate root.
pub fn expand(attribute: TokenStream, mut function: ImplItemFn) -> TokenStream {
    if let Some(inline) = function.attrs.iter().find(|attr| {
        attr.path().is_ident("inline")
            && attr
                .parse_args::<syn::Ident>()
                .is_ok_and(|arg| arg == "always")
    }) {
        return syn::Error::new_spanned(
            inline,
            "a point keeps its function out of line: drop #[inline(always)]",
        )
        .to_compile_error();
    }
    let shape = match point_shape::shape(attribute, &function.sig) {
        Ok(shape) => shape,
        Err(e) => return e.to_compile_error(),
    };
    let name = shape.name.clone();
    let table = quote!(::horizon_engine_slots);
    // The body moves to an inlined inner function, so the return listeners run however it
    // returns; the outer function takes each parameter by a plain, mutable name.
    let inner = syn::Ident::new(
        &format!(
            "__point_{}",
            function.sig.ident.to_string().trim_start_matches("r#")
        ),
        function.sig.ident.span(),
    );
    let mut outer = function.clone();
    // Each typed parameter's input index, in order: the API record's arguments line up with them.
    let mut inputs = Vec::new();
    for (i, input) in outer.sig.inputs.iter_mut().enumerate() {
        if let syn::FnArg::Typed(typed) = input {
            let plain = local(i);
            *typed.pat = syn::parse_quote!(mut #plain);
            inputs.push(i);
        }
    }
    if !shape.api() {
        return syn::Error::new_spanned(
            &function.sig.ident,
            format!("point {} has no API record", function.sig.ident),
        )
        .to_compile_error();
    }
    let Some(slot) = shape.slot else {
        return syn::Error::new_spanned(
            &function.sig.ident,
            format!(
                "point {} has no replacement cell: the staging pass writes `slot = N`",
                function.sig.ident
            ),
        )
        .to_compile_error();
    };
    let args: Vec<(&Arg, usize)> = shape.args.iter().zip(inputs.iter().copied()).collect();
    // What the body gets: a `&T` the listeners may have replaced, a `&mut` reborrowed so the
    // return listeners can still copy it.
    let call: Vec<TokenStream> = inputs
        .iter()
        .map(|&input| {
            let p = local(input);
            match args
                .iter()
                .find(|(_, at)| *at == input)
                .map(|(arg, _)| arg.pass)
            {
                Some(Pass::Shared) => {
                    let (_, r) = shared(input);
                    quote!(#r)
                }
                Some(Pass::Mut) => quote!(&mut *#p),
                _ => quote!(#p),
            }
        })
        .collect();
    let generics: Vec<_> = function
        .sig
        .generics
        .params
        .iter()
        .filter_map(|param| match param {
            syn::GenericParam::Type(ty) => Some(&ty.ident),
            syn::GenericParam::Const(c) => Some(&c.ident),
            syn::GenericParam::Lifetime(_) => None,
        })
        .collect();
    let turbofish = if generics.is_empty() {
        quote!()
    } else {
        quote!(::<#(#generics),*>)
    };
    let (receiver, callee) = match (shape.receiver, shape.free) {
        (Receiver::None, true) => (quote!(), quote!(#inner)),
        (Receiver::None, false) => (quote!(), quote!(Self::#inner)),
        (Receiver::Shared, _) => (quote!(self,), quote!(self.#inner)),
        (Receiver::Mut, _) => (quote!(&mut *self,), quote!(self.#inner)),
    };
    // What the API listeners get before their record: the shape's host, if it has one.
    let host = match (shape.receiver, shape.side) {
        (_, None) => quote!(),
        (Receiver::Mut, Some(Side::Both)) => {
            quote!(crate::game::points::host_side(&mut *self),)
        }
        (Receiver::Mut, Some(_)) => quote!(&mut *self,),
        (_, Some(_)) => quote!(self,),
    };
    let record = shape.record();
    let api_value = match &shape.returns {
        Returns::Value(value) => Some(value),
        _ => None,
    };
    let cancels = matches!(shape.returns, Returns::Unit | Returns::UnitResult);
    let results = api_value.is_some();
    let listened = |variant: &syn::Ident| quote!(#table::__private::POINTS[#table::PointId::#variant as usize].load(::std::sync::atomic::Ordering::Acquire));

    // The head: copy in, the API listeners, copy out.
    let shared_locals =
        args.iter()
            .filter(|(arg, _)| arg.pass == Pass::Shared)
            .map(|(_, input)| {
                let (s, _) = shared(*input);
                quote!(let mut #s = ::std::option::Option::None;)
            });
    let shared_refs = args
        .iter()
        .filter(|(arg, _)| arg.pass == Pass::Shared)
        .map(|(arg, input)| {
            let (s, r) = shared(*input);
            let p = local(*input);
            let syn::Type::Reference(reference) = &arg.ty else {
                unreachable!("a shared argument is a reference")
            };
            let pointee = &reference.elem;
            quote!(let #r: &#pointee = match &#s { ::std::option::Option::Some(__value) => __value, ::std::option::Option::None => #p };)
        });
    let record_call = |after: bool, mirrored: &[(syn::Ident, TokenStream)]| {
        let fields = args.iter().map(|(arg, input)| copy_in(arg, *input, after));
        let made = quote!(#table::#record::__new(#(#fields),*));
        let (names, values): (Vec<_>, Vec<_>) = mirrored.iter().cloned().unzip();
        (made, names, values)
    };
    let head_api = {
        let mirrored: Vec<_> = args
            .iter()
            .filter_map(|(arg, input)| mirror_in(arg, *input, false))
            .collect();
        let (made, names, values) = record_call(false, &mirrored);
        let outs = args.iter().map(|(arg, input)| copy_out(arg, *input, &name));
        let outcome = match (&shape.returns, api_value) {
            (Returns::Unit | Returns::UnitResult, _) => quote!(__cancel = __args.cancel;),
            (_, Some(value)) => {
                let out = result_out(value, &name);
                quote! {
                    if let ::std::option::Option::Some(__value) = __args.result {
                        if let ::std::option::Option::Some(__value) = #out { __result_slot = ::std::option::Option::Some(__value); }
                    }
                }
            }
            _ => quote!(),
        };
        let method = syn::Ident::new(&name, proc_macro2::Span::call_site());
        let body = quote! {
            #[allow(unused_mut)]
            let mut __args = #made;
            #table::dispatch::#method(#host &mut __args);
            #(#outs)*
            #outcome
        };
        guarded(&names, &values, body, &name)
    };
    let stop = {
        let cancel = match (&shape.returns, cancels) {
            (Returns::Unit, true) => quote!(if __cancel {
                return;
            }),
            (Returns::UnitResult, true) => quote!(if __cancel {
                return Ok(());
            }),
            _ => quote!(),
        };
        let result = results.then(|| {
            quote!(
                if let ::std::option::Option::Some(__value) = __result_slot.take() {
                    return __value;
                }
            )
        });
        quote!(#cancel #result)
    };
    let head_variant = shape.variant();
    let head_listened = listened(&head_variant);

    // The return: the API listeners get the value returned and copies of the arguments (written
    // back only for `&mut` ones).
    let has_api_return = shape.has_return();
    let back = has_api_return.then(|| {
        // A by-value argument is spent by the body: it is copied (or converted) before.
        let before: Vec<_> = args
            .iter()
            .filter(|(arg, _)| arg.pass == Pass::Value && arg.wide.is_none())
            .map(|(arg, input)| {
                let m = format_ident!("__m{input}");
                let p = local(*input);
                let value = if arg.mirror {
                    quote!(crate::game::points::mirror_in(&#p))
                } else {
                    quote!(::std::clone::Clone::clone(&#p))
                };
                (m, value)
            })
            .collect();
        let (names, values): (Vec<_>, Vec<_>) = before.into_iter().unzip();
        let before = quote! {
            let __back = if __returned {
                #table::__private::cold(|| ::std::option::Option::Some((#(#values,)*)))
            } else {
                ::std::option::Option::None
            };
        };
        (before, names)
    });
    let returned = {
        let api_part = has_api_return.then(|| {
            let (_, before_names) = back.as_ref().unwrap();
            // By-value mirrors were converted before the body; the rest convert now.
            let mirrored: Vec<_> = args
                .iter()
                .filter(|(arg, _)| arg.pass != Pass::Value)
                .filter_map(|(arg, input)| mirror_in(arg, *input, true))
                .collect();
            let by_value_mirrors: Vec<syn::Ident> = args
                .iter()
                .filter(|(arg, _)| arg.pass == Pass::Value && arg.mirror)
                .map(|(_, input)| format_ident!("__m{input}"))
                .collect();
            let (made, mut names, mut values) = record_call(true, &mirrored);
            // A by-value mirror is an `Option` already: unwrap it with the others.
            for name in &by_value_mirrors {
                names.push(name.clone());
                values.push(quote!(#name));
            }
            let (result_in, result_out) = match api_value {
                Some(value) => {
                    let into = if value.mirror {
                        names.push(format_ident!("__mres"));
                        values.push(quote!(crate::game::points::mirror_in(&__result)));
                        quote!(::std::option::Option::Some(__mres))
                    } else {
                        result_in(value)
                    };
                    let out = result_out(value, &name);
                    (
                        quote!(__args.result = #into;),
                        quote! {
                            if let ::std::option::Option::Some(__value) = __args.result {
                                if let ::std::option::Option::Some(__value) = #out { __result = __value; }
                            }
                        },
                    )
                }
                None => (quote!(), quote!()),
            };
            let outs = args
                .iter()
                .filter(|(arg, _)| arg.pass == Pass::Mut)
                .map(|(arg, input)| copy_out(arg, *input, &name));
            let method = format_ident!("{}_return", name);
            let body = quote! {
                #[allow(unused_mut)]
                let mut __args = #made;
                #result_in
                #table::dispatch::#method(#host &mut __args);
                #result_out
                #(#outs)*
            };
            let body = guarded(&names, &values, body, &name);
            quote! {
                if let ::std::option::Option::Some((#(#before_names,)*)) = __back {
                    #body
                }
            }
        });
        // One check of the return's slot, before the body: the return path runs out of line.
        api_part.is_some().then(|| {
            quote! {
                if __returned {
                    #table::__private::cold(|| { #api_part });
                }
            }
        })
    };
    let back = back.map(|(before, _)| before);
    let returned_local = returned.is_some().then(|| {
        let listened = listened(&shape.return_variant());
        quote!(let __returned = #listened;)
    });
    // The head's listened path moves the by-value arguments into its record: it runs out of line
    // and hands them back, the listeners' changes included.
    let owned: Vec<syn::Ident> = args
        .iter()
        .filter(|(arg, _)| arg.pass == Pass::Value)
        .map(|(_, input)| local(*input))
        .collect();
    let cancel_local = cancels.then(|| quote!(let mut __cancel = false;));
    let listeners_path = quote! {
        (#(#owned,)*) = #table::__private::cold(|| { #head_api (#(#owned,)*) });
        #stop
    };
    let result_local =
        results.then(|| quote!(let mut __result_slot = ::std::option::Option::None;));
    // The body, or the copy a mixin build installed in its place: the listeners run around either.
    let reader = |kind: &str| {
        let reader = format_ident!("__fnr_read_{kind}_{slot}");
        if shape.free {
            quote!(#reader())
        } else {
            quote!(Self::#reader())
        }
    };
    let installed = reader("body");
    // The dispatcher: what the function's cell holds while a mod listens here. Its listened
    // paths stay out of line.
    let stem = function.sig.ident.to_string();
    let stem = stem.trim_start_matches("r#");
    let dispatcher = format_ident!("__pointd_{stem}");
    let armer = format_ident!("__pointa_{stem}");
    let mut dispatch = outer.clone();
    dispatch.sig.ident = dispatcher.clone();
    dispatch.vis = syn::Visibility::Inherited;
    dispatch.attrs = vec![
        syn::parse_quote!(#[doc(hidden)]),
        syn::parse_quote!(#[allow(clippy::all)]),
    ];
    dispatch.block = syn::parse_quote!({
        #cancel_local
        #result_local
        #(#shared_locals)*
        if #head_listened {
            #listeners_path
        }
        #(#shared_refs)*
        #returned_local
        #back
        #[allow(unused_mut)]
        let mut __result = match #installed {
            ::std::option::Option::Some(__fnr) => __fnr(#receiver #(#call),*),
            ::std::option::Option::None => #callee #turbofish(#(#call),*),
        };
        #returned
        __result
    });
    // The function itself: its one head check, then the body. It keeps the function's own
    // `#[inline]`, so a small accessor stays inlinable into its callers, check and all.
    let entry = reader("slot");
    let own = match shape.receiver {
        Receiver::None => quote!(),
        _ => quote!(self,),
    };
    let plain: Vec<_> = inputs.iter().map(|&input| local(input)).collect();
    outer.attrs.push(syn::parse_quote!(#[allow(unused_mut)]));
    outer.block = syn::parse_quote!({
        if let ::std::option::Option::Some(__fnr) = #entry {
            return __fnr(#own #(#plain),*);
        }
        #callee #turbofish(#(#plain),*)
    });
    let dispatcher_path = if shape.free {
        quote!(#dispatcher)
    } else {
        quote!(Self::#dispatcher)
    };
    let places = std::iter::once(shape.variant())
        .chain(shape.has_return().then(|| shape.return_variant()))
        .map(|variant| listened(&variant));
    let arm = quote! {
        /// Installs this point's dispatcher as the function's entry while a mod listens at its
        /// head or return. Called once, after the points are sealed.
        #[allow(dead_code, clippy::all)]
        fn #armer() {
            if #(#places)||* {
                crate::__REPLACE[#slot].store(
                    #dispatcher_path as *mut (),
                    ::std::sync::atomic::Ordering::Release,
                );
            }
        }
    };
    function.sig.ident = inner;
    function.vis = syn::Visibility::Inherited;
    function
        .attrs
        .retain(|attr| !attr.path().is_ident("doc") && !attr.path().is_ident("inline"));
    function.attrs.push(syn::parse_quote!(#[inline(always)]));
    quote!(#outer #function #dispatch #arm)
}

/// `body`, run only when every mirror converted; a value the API can't carry skips the listeners
/// for this call, logged, rather than giving them a wrong value.
fn guarded(
    names: &[syn::Ident],
    values: &[TokenStream],
    body: TokenStream,
    point: &str,
) -> TokenStream {
    if names.is_empty() {
        return quote!({ #body });
    }
    quote! {
        if let (#(::std::option::Option::Some(#names),)*) = (#(#values,)*) {
            #body
        } else {
            crate::game::points::unconverted(#point);
        }
    }
}

/// The record's `result` from the value returned, `__result` (a mirror converts in the guard).
fn result_in(value: &point_shape::Value) -> TokenStream {
    let field = &value.field;
    if point_shape::wide(&value.ty).is_some() {
        quote!(::std::option::Option::Some(__result as #field))
    } else {
        quote!(::std::option::Option::Some(::std::clone::Clone::clone(
            &__result
        )))
    }
}

/// The engine value of a listener's `result`, `__value`: none when the engine refuses it.
fn result_out(value: &point_shape::Value, point: &str) -> TokenStream {
    let ty = &value.ty;
    if value.mirror {
        quote!(crate::game::points::mirror_out(&__value, #point))
    } else if point_shape::wide(ty).is_some() {
        quote!(::std::option::Option::Some(__value as #ty))
    } else {
        quote!(::std::option::Option::Some(__value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn refuses_missing_api_record_and_removed_raw_argument() {
        let function: ImplItemFn = syn::parse_quote!(
            fn opaque(value: Unknown) {}
        );
        let error = expand(quote!(free), function.clone()).to_string();
        assert!(error.contains("point opaque has no API record"), "{error}");
        assert!(
            expand(quote!(raw, free), function)
                .to_string()
                .contains("compile_error")
        );
    }
}
