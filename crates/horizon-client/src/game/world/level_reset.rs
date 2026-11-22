//! `RuntimeLevels::reset_vanilla`: the Overworld, the Nether and the End regenerated with a new seed while the
//! server runs (docs/modding/src/how-to/world/runtime-levels.md, horizon-rust/horizon#18369).
//!
//! A request is checked when the mod makes it and queued with the runtime levels' requests; it is applied after the
//! server tick's level loop (`apply_runtime_levels`), never inside a level's tick. The mod has moved every player
//! out of the three levels first, into a runtime level of its own, which then holds the place of the level in use
//! while the three are dropped without a save and their terrain files are removed. They open again from the new seed
//! the next time somebody enters them.
use super::{Dim, Game};
use mod_api::LevelResetError;
use std::path::{Path, PathBuf};

/// The levels every world has, which a reset regenerates.
const VANILLA: [Dim; 3] = [Dim::Overworld, Dim::Nether, Dim::End];

/// A level's own folders that hold its terrain and what lives in it, under `dimensions/<namespace>/<path>/`.
const TERRAIN_FOLDERS: [&str; 3] = ["region", "entities", "poi"];

/// Files of a level's `data/` that belong to its terrain: the raids, the chunk tickets of its forced chunks and of
/// the mods', the dragon fight (the End's) and the mods' attachments. Its world border is the world's setting, not
/// the terrain's, and stays.
const TERRAIN_DATA: [&str; 5] = [
    "data/minecraft/raids.dat",
    "data/minecraft/chunk_tickets.dat",
    "data/minecraft/ender_dragon_fight.dat",
    "data/horizon/attachments.dat",
    "data/horizon/mod_chunk_tickets.dat",
];

impl Game<'_> {
    /// `RuntimeLevels::reset_vanilla`: check the request now, queue it for after the level loop.
    pub(in crate::game) fn queue_level_reset(&mut self, seed: i64) -> Result<(), LevelResetError> {
        let owner = mod_api::content::crash::current_native_mod();
        // An unidentified caller is granted nothing, whatever the bundles declare.
        let permitted = owner.is_some() && self.native_grant(|native| native.level_reset).is_some();
        let result = self.check_level_reset(owner, seed, permitted);
        if let Err(error) = &result {
            log::warn!(
                "mod {}: runtime_levels reset_vanilla: refused ({error:?})",
                owner.unwrap_or("?"),
            );
        }
        result
    }

    /// The rules of a request: `owner` is the calling mod (None: nobody identified) and `permitted` whether its
    /// manifest declares `level_reset`.
    pub(in crate::game) fn check_level_reset(
        &mut self,
        owner: Option<&'static str>,
        seed: i64,
        permitted: bool,
    ) -> Result<(), LevelResetError> {
        if !self.is_logical_server() || self.native_lifecycle.epoch().is_none() {
            return Err(LevelResetError::Unavailable);
        }
        let owner = owner
            .filter(|_| permitted)
            .ok_or(LevelResetError::NotPermitted)?;
        if VANILLA.iter().any(|dim| self.level_has_players(*dim)) {
            return Err(LevelResetError::PlayersInside);
        }
        if self.levels.runtime.holding_level_of(owner).is_none() {
            return Err(LevelResetError::NoHoldingLevel);
        }
        self.levels.runtime.queue_reset(owner, seed)
    }

    /// Apply a queued reset. A reset that cannot be applied is logged, and the mod can ask again: the levels it
    /// had dropped open again from whichever seed is in force.
    pub(in crate::game) fn apply_level_reset(&mut self, owner: &'static str, seed: i64) {
        if let Err(why) = self.reset_vanilla_levels(owner, seed) {
            log::warn!("mod {owner}: reset_vanilla was not completed: {why}");
        }
    }

    fn reset_vanilla_levels(
        &mut self,
        owner: &'static str,
        seed: i64,
    ) -> horizon_client::Result<()> {
        if VANILLA.iter().any(|dim| self.level_has_players(*dim)) {
            return Err("a player came in".into());
        }
        // The level in use is one of the mod's, so the three are parked or closed. It is found before anything is written: a mod that unloaded it earlier in
        // the tick leaves a reset that changes nothing.
        let hold = self
            .levels
            .runtime
            .holding_level_of(owner)
            .and_then(|id| Dim::extra(&id))
            .ok_or("the mod has no runtime level open")?;
        // Every refusal that can be known now comes before the first change: a link under any level's terrain.
        check_terrain(&self.world_root)?;
        // The seed is written first: when it cannot be, nothing has been dropped and nothing has changed.
        horizon_client::save::write_world_seed(&self.world_root, seed)?;
        if VANILLA.contains(&self.dim) {
            self.enter_level(hold)?;
        }
        for dim in VANILLA {
            if self.levels.parked.contains_key(&dim) {
                self.discard_parked_level(dim)?;
            }
        }
        // A level that opens while the files go opens from the new seed.
        self.level.seed = seed;
        if let Some(host) = self.lan.as_mut() {
            host.hub.config.seed = seed;
        }
        let mut failed = Vec::new();
        for dim in VANILLA {
            if let Err(why) = erase_terrain(&self.world_root, dim.id()) {
                failed.push(format!("{}: {why}", dim.id()));
            }
        }
        // What belongs to the old world as a whole.
        let trader = self.world_root.join("data/minecraft/wandering_trader.dat");
        if let Err(why) = std::fs::remove_file(&trader)
            && why.kind() != std::io::ErrorKind::NotFound
        {
            failed.push(format!("wandering_trader.dat: {why}"));
        }
        self.creatures.reset_trader();
        // The active level is the mod's own: its entities' portal processes and cooldowns stay.
        let entity_portals = std::mem::take(&mut self.portals.entities);
        self.portals = Default::default();
        self.portals.entities = entity_portals;
        self.fight = Default::default();
        self.end = Default::default();
        // The world spawn is found again, in the new terrain.
        self.level.world_spawn = None;
        self.level.spawn = None;
        self.world_spawn_pending = true;
        self.level.world_spawn_dimension = None;
        self.spawn = None;
        self.spawn_origin = None;
        self.spawn_hook_fired = false;
        self.spawn_hook_cancelled = false;
        self.spawn_spiral = Default::default();
        self.player_spawn = None;
        // The mods are told only of a reset that was carried out: a failure is returned and logged, and asking
        // again repeats it.
        if !failed.is_empty() {
            return Err(format!("files that could not be removed: {}", failed.join("; ")).into());
        }
        self.native_levels_reset();
        Ok(())
    }

    /// Drop the parked level `dim` without saving it: the mods hear its chunks and the level unload, as
    /// `close_parked_level` tells them, but what it held is not written, because its files are about to go.
    fn discard_parked_level(&mut self, dim: Dim) -> horizon_client::Result<()> {
        let back = self.dim;
        self.enter_level(dim)?;
        // A border changed since the last save lives only in memory, and the border is kept.
        self.save_border();
        let chunks: Vec<_> = self.world.chunks.values().cloned().collect();
        self.native_chunks_unloaded(&chunks);
        for id in self.persist.uuids.keys() {
            self.entity_attachments.remove(id);
        }
        self.enter_level(back)?;
        self.levels.parked.remove(&dim);
        self.native_level_closed(dim);
        Ok(())
    }
}

/// One path of a level's terrain: what to check for links, up to which folder, and how to remove it.
struct Target {
    path: PathBuf,
    base: PathBuf,
    whole_folder: bool,
    directory: bool,
}

/// Every path a reset removes for the vanilla level `id`: its region, entity and POI folders, the data files that
/// belong to it and its structure references.
fn terrain_targets(root: &Path, id: &str) -> std::io::Result<Vec<Target>> {
    let dimensions = root.join("dimensions");
    let folder = horizon_client::save::dimension_dir(root, id);
    if folder == dimensions || !folder.starts_with(&dimensions) {
        return Err(std::io::Error::other("not a level folder"));
    }
    let inside = |path: PathBuf, directory: bool| Target {
        path,
        base: dimensions.clone(),
        whole_folder: true,
        directory,
    };
    let mut targets: Vec<Target> = TERRAIN_FOLDERS
        .iter()
        .map(|sub| inside(folder.join(sub), true))
        .collect();
    targets.extend(TERRAIN_DATA.iter().map(|file| inside(folder.join(file), false)));
    targets.push(Target {
        path: super::world_loading::structure_references_file(root, id),
        base: root.join("data"),
        whole_folder: false,
        directory: false,
    });
    Ok(targets)
}

/// The refusals that can be known before anything changes: a link on the way to any path the reset would remove, in
/// any of the three levels. A reset that passes this can still fail on the disk, but never on a link.
fn check_terrain(root: &Path) -> std::io::Result<()> {
    for dim in VANILLA {
        for target in terrain_targets(root, dim.id())? {
            let checked =
                super::runtime_levels::ensure_no_links(&target.path, &target.base, target.whole_folder);
            super::runtime_levels::ignore_missing(checked)?;
        }
    }
    Ok(())
}

/// Remove the terrain of the vanilla level `id`. Nothing that is not the level's own: a link on the way is refused,
/// and nothing is removed from a path whose check failed.
fn erase_terrain(root: &Path, id: &str) -> std::io::Result<()> {
    for target in terrain_targets(root, id)? {
        let checked =
            super::runtime_levels::ensure_no_links(&target.path, &target.base, target.whole_folder);
        super::runtime_levels::ignore_missing(checked)?;
        super::runtime_levels::ignore_missing(if target.directory {
            std::fs::remove_dir_all(&target.path)
        } else {
            std::fs::remove_file(&target.path)
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::erase_terrain;
    use std::fs;

    fn write(path: &std::path::Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"x").unwrap();
    }

    #[test]
    fn the_terrain_of_a_level_goes_and_its_border_and_other_levels_stay() {
        let root = std::env::temp_dir().join(format!("level-reset-erase-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let overworld = root.join("dimensions/minecraft/overworld");
        let nether = root.join("dimensions/minecraft/the_nether");
        for file in [
            "region/r.0.0.mca",
            "entities/r.0.0.mca",
            "poi/r.0.0.mca",
            "data/minecraft/raids.dat",
            "data/minecraft/chunk_tickets.dat",
            "data/minecraft/world_border.dat",
        ] {
            write(&overworld.join(file));
            write(&nether.join(file));
        }
        erase_terrain(&root, "minecraft:overworld").unwrap();
        for file in [
            "region/r.0.0.mca",
            "entities/r.0.0.mca",
            "poi/r.0.0.mca",
            "data/minecraft/raids.dat",
            "data/minecraft/chunk_tickets.dat",
        ] {
            assert!(!overworld.join(file).exists(), "{file} should be gone");
            assert!(nether.join(file).exists(), "the nether's {file} stays");
        }
        assert!(
            overworld.join("data/minecraft/world_border.dat").exists(),
            "the border is the world's setting"
        );
        // A level with nothing on disk is not an error.
        erase_terrain(&root, "minecraft:the_end").unwrap();
        fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_linked_level_folder_is_refused_and_what_it_leads_to_stays() {
        let root = std::env::temp_dir().join(format!("level-reset-link-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let outside = root.join("outside");
        write(&outside.join("region/r.0.0.mca"));
        fs::create_dir_all(root.join("dimensions/minecraft")).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("dimensions/minecraft/overworld")).unwrap();
        assert!(erase_terrain(&root, "minecraft:overworld").is_err());
        assert!(outside.join("region/r.0.0.mca").exists());
        // A link under one level refuses the whole reset up front, so no other level is erased first.
        write(&root.join("dimensions/minecraft/the_nether/region/r.0.0.mca"));
        assert!(super::check_terrain(&root).is_err());
        assert!(root.join("dimensions/minecraft/the_nether/region/r.0.0.mca").exists());
        fs::remove_dir_all(&root).unwrap();
    }
}
