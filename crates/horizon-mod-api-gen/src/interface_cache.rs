//! Build-local cache for walks that inspect signatures and module declarations, never bodies.
use quote::ToTokens;
use sha2::{Digest, Sha256};
use std::{
    cell::RefCell,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
use syn::visit_mut::VisitMut;

thread_local! {
    static DIRECTORY: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

pub(crate) fn with_cache<T>(
    out: &Path,
    run: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    // A new generator executable invalidates all summaries, including after parser upgrades.
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let identity = Sha256::digest(fs::read(executable).map_err(|e| e.to_string())?);
    let directory = out.join(format!("interfaces-{identity:x}"));
    fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    struct Reset(Option<PathBuf>);
    impl Drop for Reset {
        fn drop(&mut self) {
            DIRECTORY.with(|slot| *slot.borrow_mut() = self.0.take());
        }
    }
    let _reset = Reset(DIRECTORY.with(|slot| slot.replace(Some(directory))));
    run()
}

pub(crate) fn parse(path: &Path, source: &str) -> Result<syn::File, String> {
    let directory = DIRECTORY.with(|slot| slot.borrow().clone());
    let Some(directory) = directory else {
        return syn::parse_file(source).map_err(|e| format!("{}: {e}", path.display()));
    };
    let mut key = Sha256::new();
    key.update(path.as_os_str().as_encoded_bytes());
    key.update([0]);
    key.update(source.as_bytes());
    let cached = directory.join(format!("{:x}.cache", key.finalize()));
    match fs::read(&cached) {
        Ok(bytes) => {
            // Cache files are disposable, not authoritative source. A truncated entry (even
            // an empty but syntactically valid Rust file) must never remove points silently.
            if let Ok(text) = std::str::from_utf8(&bytes)
                && let Some((digest, source)) = text.split_once('\n')
                && digest == format!("{:x}", Sha256::digest(source.as_bytes()))
                && let Ok(file) = syn::parse_file(source)
            {
                return Ok(file);
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.to_string()),
    }
    let mut file = syn::parse_file(source).map_err(|e| format!("{}: {e}", path.display()))?;
    struct Signatures;
    impl VisitMut for Signatures {
        fn visit_item_fn_mut(&mut self, function: &mut syn::ItemFn) {
            function.block.stmts.clear();
        }
        fn visit_impl_item_fn_mut(&mut self, function: &mut syn::ImplItemFn) {
            function.block.stmts.clear();
        }
        fn visit_trait_item_fn_mut(&mut self, function: &mut syn::TraitItemFn) {
            if let Some(body) = &mut function.default {
                body.stmts.clear();
            }
        }
    }
    Signatures.visit_file_mut(&mut file);
    let source = file.to_token_stream().to_string();
    let bytes = format!("{:x}\n{source}", Sha256::digest(source.as_bytes()));
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let temporary = cached.with_extension(format!(
        "tmp-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|e| e.to_string())?;
    output
        .write_all(bytes.as_bytes())
        .map_err(|e| e.to_string())?;
    drop(output);
    fs::rename(temporary, cached).map_err(|e| e.to_string())?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corrupt_entries_regenerate_and_scope_resets_after_error() {
        let out = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target")
            .join(format!("interface-cache-unit-{}", std::process::id()));
        let path = Path::new("example.rs");
        let source = "pub fn example(value: i32) -> i32 { value + 1 }";
        with_cache(&out, || {
            let cold = parse(path, source)?.to_token_stream().to_string();
            assert_eq!(cold, parse(path, source)?.to_token_stream().to_string());
            let directory = DIRECTORY.with(|slot| slot.borrow().clone().unwrap());
            let cached = fs::read_dir(directory)
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path();
            for corrupt in ["", "truncated", "wrong-digest\nfn another() {}"] {
                fs::write(&cached, corrupt).unwrap();
                assert_eq!(cold, parse(path, source)?.to_token_stream().to_string());
            }
            Ok(())
        })
        .unwrap();
        let failed: Result<(), String> = with_cache(&out, || Err("expected failure".into()));
        assert!(failed.is_err());
        assert!(DIRECTORY.with(|slot| slot.borrow().is_none()));
        let original = parse(path, source).unwrap();
        let syn::Item::Fn(function) = &original.items[0] else {
            panic!("missing function")
        };
        assert_eq!(function.block.stmts.len(), 1);
        fs::remove_dir_all(out).unwrap();
    }
}
