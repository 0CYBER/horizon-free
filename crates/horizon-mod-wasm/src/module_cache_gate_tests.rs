use super::*;

fn refuses_artifact(cache: ModuleCache) {
    let engine = crate::runtime::test_engine();
    let bytes =
        wat::parse_str("(module (func (export \"answer\") (result i32) i32.const 42))").unwrap();
    let other =
        wat::parse_str("(module (func (export \"answer\") (result i32) i32.const 99))").unwrap();
    fs::create_dir_all(&cache.root).unwrap();
    let path = cache
        .root
        .join(format!("{}.cwasm", ModuleCache::key(&engine, &bytes)));
    // A valid but wrong native artifact must never be deserialized.
    fs::write(
        path,
        Module::new(&engine, other).unwrap().serialize().unwrap(),
    )
    .unwrap();
    for _ in 0..2 {
        let (module, source) = cache.load_or_compile(&engine, &bytes).unwrap();
        assert_eq!(source, Source::Compiled);
        let mut store = wasmtime::Store::new(&engine, ());
        let instance = wasmtime::Instance::new(&mut store, &module, &[]).unwrap();
        assert_eq!(
            instance
                .get_typed_func::<(), i32>(&mut store, "answer")
                .unwrap()
                .call(&mut store, ())
                .unwrap(),
            42
        );
    }
    assert!(
        cache
            .disabled
            .as_ref()
            .unwrap()
            .contains("owner/DACL/inheritance")
    );
    assert!(cache.warned.load(Ordering::Relaxed));
}

#[test]
fn unverified_windows_trust_never_deserializes() {
    let dir = tempfile::tempdir().unwrap();
    refuses_artifact(ModuleCache::new_for_platform(
        dir.path().join("cache"),
        true,
    ));
}

#[cfg(windows)]
#[test]
fn windows_default_never_deserializes() {
    let dir = tempfile::tempdir().unwrap();
    refuses_artifact(ModuleCache::new(dir.path().join("cache")));
}
