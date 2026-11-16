fn main() {
    let root =
        std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let out_dir = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    horizon_mod_api_gen::generate_points(&root, &out_dir).expect("generating the points");
    // One point catalog (arismoko/horizon#3643): once `horizon-engine-slots` names a `PointId` of its own, this
    // crate's tests check at compile time that it is the very type this crate exports
    // (`tests/one_point_catalog.rs`), so the two halves of a dispatch can never disagree index for index.
    println!("cargo::rustc-check-cfg=cfg(engine_slots_point_id)");
    let slots = root.join("crates/horizon-engine-slots");
    println!("cargo::rerun-if-changed={}", slots.display());
    if defines_point_id(&slots) {
        println!("cargo::rustc-cfg=engine_slots_point_id");
    }
}

/// Whether a Rust file under `folder` defines `PointId` (`enum PointId`) or re-exports one (a
/// `pub use` naming it): a mere mention, in a comment say, does not count.
fn defines_point_id(folder: &std::path::Path) -> bool {
    std::fs::read_dir(folder)
        .into_iter()
        .flatten()
        .flatten()
        .any(|entry| {
            let path = entry.path();
            if path.is_dir() {
                path.file_name() != Some("target".as_ref()) && defines_point_id(&path)
            } else {
                path.extension() == Some("rs".as_ref())
                    && std::fs::read_to_string(&path).is_ok_and(|text| {
                        text.contains("enum PointId")
                            || text.match_indices("pub use").any(|(at, _)| {
                                text[at..]
                                    .split(';')
                                    .next()
                                    .is_some_and(|item| item.contains("PointId"))
                            })
                    })
            }
        })
}
