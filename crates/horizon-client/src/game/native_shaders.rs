//! Horizon's own shader packs in the game (docs/shaders.md): the pick in `shaderpacks/` (a
//! folder with `horizon-shaders.toml`, or a `.hshader`; `game/shader_packs.rs`) runs once it
//! passes `horizon_shader_api::check`. The pack is looked at twice a second: a saved file is checked and the pack rebuilt, and a
//! pack that fails is refused with its problems in the chat while the last good one keeps
//! drawing.

use std::{
    hash::{Hash, Hasher},
    path::Path,
    time::{Duration, Instant},
};

use horizon_client::text::Text;
use horizon_shader_api::{Pack, check};

use super::Game;

const LOOK_EVERY: Duration = Duration::from_millis(500);
const RED: u32 = 0xFF5555;

#[derive(Default)]
pub(super) struct NativeShaders {
    /// The pack's name in `shaderpacks/`, of either kind (`game/shader_packs.rs`); empty: none.
    pub picked: String,
    /// The pack's files when it was last read (`fingerprint`).
    stamp: Option<u64>,
    looked: Option<Instant>,
}

impl NativeShaders {
    pub fn picking(name: &str) -> Self {
        Self {
            picked: name.to_owned(),
            ..Self::default()
        }
    }
}

impl Game<'_> {
    /// Builds the pack at `path` when its files changed since it was last read.
    pub(super) fn tick_native_shader_pack(&mut self, path: &Path) {
        let now = Instant::now();
        let state = &mut self.native_shaders;
        if state.looked.is_some_and(|t| now - t < LOOK_EVERY) {
            return;
        }
        state.looked = Some(now);
        let stamp = fingerprint(path);
        if stamp == state.stamp {
            return;
        }
        let reload = state.stamp.is_some();
        state.stamp = stamp;
        let Some(renderer) = &mut self.renderer else {
            return;
        };
        let result = Pack::read(path).map_err(|e| vec![e]).and_then(|pack| {
            let report = check(&pack)
                .map_err(|p| p.iter().map(ToString::to_string).collect::<Vec<String>>())?;
            let constants = pack
                .manifest
                .options
                .iter()
                .map(|o| (o.id.clone(), o.kind.default_constant()))
                .collect();
            let labels = renderer
                .set_native_shader_pack(Some((report, constants)))
                .map_err(|e| vec![e])?;
            Ok((pack.manifest, labels))
        });
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        match result {
            Ok((m, labels)) => {
                log::info!(
                    "shader pack {} {}: {} in {} ms",
                    m.id,
                    m.version,
                    labels.join(", "),
                    now.elapsed().as_millis()
                );
                if reload {
                    self.system_message(Text::literal(format!("Shader pack {} reloaded", m.name)));
                }
            }
            Err(problems) => {
                log::warn!("shader pack {name} refused: {}", problems.join("; "));
                self.system_message(Text::colored(format!("Shader pack {name} refused:"), RED));
                for p in problems {
                    self.system_message(Text::colored(p, RED));
                }
            }
        }
    }
}

/// A hash of the pack's file names, sizes and modification times (`None` when unreadable).
fn fingerprint(path: &Path) -> Option<u64> {
    fn walk(path: &Path, hasher: &mut std::hash::DefaultHasher) -> Option<()> {
        let meta = std::fs::metadata(path).ok()?;
        path.hash(hasher);
        if meta.is_dir() {
            let mut entries: Vec<_> = std::fs::read_dir(path)
                .ok()?
                .flatten()
                .map(|e| e.path())
                .collect();
            entries.sort();
            for entry in entries {
                walk(&entry, hasher)?;
            }
        } else {
            meta.len().hash(hasher);
            meta.modified().ok()?.hash(hasher);
        }
        Some(())
    }
    let mut hasher = std::hash::DefaultHasher::new();
    walk(path, &mut hasher)?;
    Some(hasher.finish())
}
