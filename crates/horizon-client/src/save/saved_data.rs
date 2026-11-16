//! `SavedDataStorage`'s files under `data/minecraft/`, moved out of `save.rs` unchanged.
use super::{DATA_VERSION, Result, Tag, read_dat, write_dat};
use std::path::{Path, PathBuf};

/// `SavedDataStorage`: `{data: ..., DataVersion}` under `data/minecraft/`.
pub(super) fn saved_data_path(world: &Path, name: &str) -> PathBuf {
    world
        .join("data")
        .join("minecraft")
        .join(format!("{name}.dat"))
}
pub(super) fn read_saved_data(world: &Path, name: &str) -> Option<Tag> {
    read_dat(&saved_data_path(world, name))?
        .get("data")
        .cloned()
}
pub(super) fn write_saved_data(world: &Path, name: &str, data: Tag) -> Result<()> {
    write_dat(
        &saved_data_path(world, name),
        Tag::compound([("data", data), ("DataVersion", Tag::Int(DATA_VERSION))]),
    )
}
