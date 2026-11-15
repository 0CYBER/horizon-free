//! Configuration for the supervisor's chunk pruning.

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    #[default]
    Off,
    DryRun,
    Delete,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    /// "off", "dry-run" (a report, the game starts at once) or "delete" (before the game starts).
    pub mode: Mode,
    /// Chunks with less `InhabitedTime` than this go (1200 ticks: a minute).
    pub min_inhabited_ticks: i64,
    /// Chunks this close to the spawn (in chunks, on each axis) are kept.
    pub spawn_radius: u32,
    /// Chunks this close to a kept chunk are kept too.
    pub ring: u32,
    /// JSON files in the world folder naming chunks to keep: keys or strings `<dimension> <chunk x>
    /// <chunk z>` (claims), and objects with `dimension`, `x` and `z` (homes, warps). A file that is
    /// missing is skipped; one that can't be read stops the prune.
    pub keep_files: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            mode: Mode::Off,
            min_inhabited_ticks: 1200,
            spawn_radius: 32,
            ring: 1,
            keep_files: vec![
                "horizon-smp/claims.json".into(),
                "horizon-smp/places.json".into(),
            ],
        }
    }
}

impl Settings {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    pub fn check(&self) -> Result<(), String> {
        if self.min_inhabited_ticks < 0 || self.ring > 8 {
            return Err("prune_chunks: min_inhabited_ticks must be 0 or more, ring 0 to 8".into());
        }
        if let Some(bad) = self
            .keep_files
            .iter()
            .find(|f| Path::new(f).is_absolute() || f.split(['/', '\\']).any(|p| p == ".."))
        {
            return Err(format!(
                "prune_chunks.keep_files: {bad:?} must be a path inside the world folder"
            ));
        }
        Ok(())
    }
}
