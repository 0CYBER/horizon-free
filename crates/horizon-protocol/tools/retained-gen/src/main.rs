//! Generate required owned-field traversals for manually declared packet values.
//! Run from the repository root. packet! and packet_enum! own their own traversal.
use quote::{format_ident, quote};
use std::{fs, path::PathBuf};
use syn::{Fields, Item};
const MARKER: &str =
    "// Generated retained-allocation traversals; regenerate with protocol-retained-gen.";
fn main() {
    let root = PathBuf::from("crates/horizon-protocol/src");
    let mut paths = vec![
        root.join("item.rs"),
        root.join("item/trim.rs"),
        root.join("item/custom.rs"),
        root.join("entity_data.rs"),
        root.join("chunk.rs"),
    ];
    fn collect(dir: PathBuf, paths: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                collect(path, paths);
            } else if path.extension().is_some_and(|x| x == "rs") {
                paths.push(path);
            }
        }
    }
    collect(root.join("packets"), &mut paths);
    for path in paths {
        if path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains("tests")
            || path.file_name().unwrap() == "registry.rs"
        {
            continue;
        }
        let original = fs::read_to_string(&path).unwrap();
        let original = if let Some((before, after)) = original.split_once(MARKER) {
            format!(
                "{}{}",
                before,
                after
                    .split_once("// End generated retained traversals.")
                    .map_or("", |(_, tail)| tail)
            )
        } else {
            original
        };
        let original = original.trim_end();
        let file = syn::parse_file(original).unwrap();
        let mut generated = Vec::new();
        for item in file.items {
            let (name, arms) = match item {
                Item::Struct(s) if s.generics.params.is_empty() => {
                    let name = s.ident;
                    let names: Vec<_> = s
                        .fields
                        .iter()
                        .enumerate()
                        .map(|(i, f)| {
                            f.ident
                                .clone()
                                .unwrap_or_else(|| format_ident!("field_{i}"))
                        })
                        .collect();
                    let pat = match s.fields {
                        Fields::Named(_) => quote! {{ #(#names),* }},
                        Fields::Unnamed(_) => quote! {( #(#names),* )},
                        Fields::Unit => quote! {},
                    };
                    (
                        name,
                        quote! { let Self #pat = self; 0usize #(.saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(#names)))* },
                    )
                }
                Item::Enum(e) if e.generics.params.is_empty() => {
                    let name = e.ident;
                    let arms: Vec<_> = e.variants.into_iter().map(|v| {
                        let variant = v.ident;
                        let names: Vec<_> = v.fields.iter().enumerate().map(|(i,f)| f.ident.clone().unwrap_or_else(|| format_ident!("field_{i}"))).collect();
                        let pat = match v.fields { Fields::Named(_) => quote!{{ #(#names),* }}, Fields::Unnamed(_) => quote!{( #(#names),* )}, Fields::Unit => quote!{} };
                        quote! { Self::#variant #pat => 0usize #(.saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(#names)))* }
                    }).collect();
                    (name, quote! { match self { #(#arms),* } })
                }
                _ => continue,
            };
            generated.push(quote! { impl crate::retained::OwnedHeap for #name { fn owned_heap_bytes(&self) -> usize { #arms } } });
        }
        let at = original.find("\n#[cfg(test)]").unwrap_or(original.len());
        let (before, after) = original.split_at(at);
        fs::write(
            path,
            format!(
                "{before}\n\n{MARKER}\n{}\n// End generated retained traversals.\n{after}\n",
                quote! { #(#generated)* }
            ),
        )
        .unwrap();
    }
}
