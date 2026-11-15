//! Parse Game impls for the PR 2c rename collision audit.
use std::{fs, path::Path};
use syn::visit::Visit;

struct Audit<'a> {
    path: &'a Path,
}
impl<'ast> Visit<'ast> for Audit<'_> {
    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        if let syn::Type::Path(ty) = &*item.self_ty
            && ty.path.segments.last().is_some_and(|s| s.ident == "Game")
        {
            for member in &item.items {
                if let syn::ImplItem::Fn(method) = member {
                    println!(
                        "{}\t{}\t{}\t{}",
                        method.sig.ident,
                        self.path.display(),
                        method.sig.ident.span().start().line,
                        if item.trait_.is_some() {
                            "trait"
                        } else {
                            "inherent"
                        }
                    );
                }
            }
        }
        syn::visit::visit_item_impl(self, item);
    }
}
fn walk(path: &Path) {
    if path.is_dir() {
        for entry in fs::read_dir(path).unwrap() {
            walk(&entry.unwrap().path());
        }
    } else if path.extension().is_some_and(|s| s == "rs") {
        let source = fs::read_to_string(path).unwrap();
        let file = syn::parse_file(&source).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        Audit { path }.visit_file(&file);
    }
}
fn main() {
    walk(Path::new("crates/horizon-client/src"));
}
