//! Prints the host API as one line per method: `owner<TAB>name<TAB>signature<TAB>doc<TAB>hidden`.
//! Usage: `host-manifest FILE...`. A trait of an area file (`ItemsHost`, `ItemsBackend`…) is
//! reported under the union it is a part of (`Host`, `Backend`…), so the output of the old
//! generated host and of the area files plus the generated unions compare line for line.
use quote::ToTokens;
use std::fs;

const UNIONS: [&str; 6] = [
    "ClientBackendMut",
    "BackendMut",
    "ClientBackend",
    "Backend",
    "ClientHost",
    "ServerHost",
];

/// The union an area trait is a part of, or the trait itself where it is not one.
fn owner(name: &str) -> String {
    if matches!(
        name,
        "Host" | "ServerHost" | "ClientHost" | "Backend" | "BackendMut" | "ClientBackendMut"
    ) {
        return name.to_owned();
    }
    for union in UNIONS {
        if name.ends_with(union) && name.len() > union.len() {
            return match union {
                "ClientBackend" => "Backend".to_owned(),
                other => other.to_owned(),
            };
        }
    }
    if name.ends_with("Host") && name.len() > 4 {
        return "Host".to_owned();
    }
    name.to_owned()
}

fn docs(attrs: &[syn::Attribute]) -> String {
    attrs
        .iter()
        .filter_map(|attr| match &attr.meta {
            syn::Meta::NameValue(syn::MetaNameValue {
                path,
                value:
                    syn::Expr::Lit(syn::ExprLit {
                        lit: syn::Lit::Str(text),
                        ..
                    }),
                ..
            }) if path.is_ident("doc") => Some(text.value()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn line(owner: &str, sig: &syn::Signature, attrs: &[syn::Attribute]) -> String {
    let hash = format!("{:?}", docs(attrs));
    let hidden = attrs
        .iter()
        .any(|a| a.to_token_stream().to_string().contains("hidden"));
    format!(
        "{owner}\t{}\t{}\t{}{}",
        sig.ident,
        sig.to_token_stream().to_string().replace(" ,)", ")"),
        hash,
        if hidden { "\thidden" } else { "" }
    )
}

fn main() {
    let mut lines = Vec::new();
    for path in std::env::args().skip(1) {
        let file = syn::parse_file(&fs::read_to_string(&path).unwrap()).unwrap();
        for item in file.items {
            match item {
                syn::Item::Trait(item) => {
                    let name = owner(&item.ident.to_string());
                    for member in item.items {
                        if let syn::TraitItem::Fn(method) = member {
                            lines.push(line(&name, &method.sig, &method.attrs));
                        }
                    }
                }
                syn::Item::Impl(item) if item.trait_.is_none() => {
                    let name = item.self_ty.to_token_stream().to_string();
                    for member in item.items {
                        if let syn::ImplItem::Fn(method) = member {
                            lines.push(line(&name, &method.sig, &method.attrs));
                        }
                    }
                }
                _ => {}
            }
        }
    }
    lines.sort();
    println!("{}", lines.join("\n"));
}
