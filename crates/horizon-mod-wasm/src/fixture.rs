//! Real-runtime fixture access for downstream adapter tests; never enabled by game builds.
use crate::{WasmMod, link::Member, runtime};

/// Instantiate a synthetic shared-memory guest with the production runtime.
pub fn module(bytes: &[u8]) -> Result<WasmMod, String> {
    let engine = runtime::engine().map_err(|error| error.to_string())?;
    WasmMod::instantiate(
        &engine,
        runtime::DEFAULT_GROUP_MEMORY_MIB,
        &[Member {
            name: "fixture",
            module: bytes,
        }],
        None,
    )
    .map_err(|error| format!("{error:?}"))?
    .into_iter()
    .next()
    .ok_or_else(|| "fixture exported no mod boundary".to_owned())
}

/// Resolve a production host row without pinning generated numeric IDs in a fixture.
pub fn import_id(owner: &str, name: &str) -> Option<u32> {
    crate::boundary::import_id(owner, name)
}

/// Load through the shipped compiler command, substituting the downstream game executable.
#[cfg(unix)]
pub fn cached_answer(
    root: &std::path::Path,
    executable: &std::path::Path,
    bytes: &[u8],
) -> Result<(i32, bool), String> {
    let engine = runtime::engine().map_err(|e| e.to_string())?;
    let cache = crate::module_cache::ModuleCache::new(root.to_owned());
    let command = |input: &std::path::Path, key: &str| {
        let production = crate::module_cache::production_command(input, key);
        let mut command = std::process::Command::new(executable);
        command.env_clear().args(production.get_args());
        command
    };
    let (module, source) = cache
        .load_with_child_command(&engine, bytes, Some((command, cache.budget.limit)))
        .map_err(|e| e.to_string())?;
    let mut store = wasmtime::Store::new(&engine, ());
    store.set_epoch_deadline(u64::MAX / 2);
    let instance = wasmtime::Instance::new(&mut store, &module, &[]).map_err(|e| e.to_string())?;
    let answer = instance
        .get_typed_func::<(), i32>(&mut store, "answer")
        .and_then(|f| f.call(&mut store, ()))
        .map_err(|e| e.to_string())?;
    Ok((answer, source == crate::module_cache::Source::Cached))
}

/// Invoke a synthetic fixture's load hook with the production load deadline.
/// Overflow proofs must not race the shorter per-tick epoch deadline.
pub fn call_load(module: &WasmMod) -> Option<()> {
    module.call(
        runtime::CallClass::Load,
        crate::boundary::Frame::default(),
        0,
        None,
        |_| {},
        |reader| reader.take(),
    )
}
