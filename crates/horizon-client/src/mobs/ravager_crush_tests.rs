//! `Ravager.aiStep` tramples leaves and `Entity.checkInsideBlocks` ->
//! `CropBlock.entityInside` / `PitcherCropBlock.entityInside` breaks crops:
//! both test the block's *class*, so a content mod's block that copies
//! `minecraft:wheat`'s or `minecraft:oak_leaves`'s class chain counts, and a
//! block named `..._leaves` that is not a `LeavesBlock` does not.
use crate::mobs::{Mob, MobEvent, world};
use crate::mod_content::ContentMod;
use horizon_core::geometry::Vec3;
use std::{
    path::PathBuf,
    sync::{Arc, OnceLock},
};

/// The vanilla data plus a content mod whose blocks only inherit classes:
/// a wheat crop, a pitcher crop, a leaf and a stone block that is named
/// `_leaves`. `None` without the prepared vanilla data (CI).
fn resources() -> Option<&'static std::path::Path> {
    static ROOT: OnceLock<Option<PathBuf>> = OnceLock::new();
    ROOT.get_or_init(|| {
        let base =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.local/horizon-client/26.3");
        if !base.join("block-behaviours.json").exists() {
            return None;
        }
        let scratch = std::env::temp_dir().join(format!("ravager-crush-{}", std::process::id()));
        let dir = scratch.join("mod");
        let mut files = vec![(
            "horizon-mod.toml".to_owned(),
            "id = \"fieldtest\"\nname = \"Field test\"\nversion = \"1.0\"\napi = \"0.2\"\nside = \"both\"\ncapabilities = [\"registries\"]\n".to_owned(),
        )];
        for (id, template) in [
            ("grain", "wheat"),
            ("pod", "pitcher_crop"),
            ("canopy", "oak_leaves"),
            ("fake_leaves", "stone"),
        ] {
            files.push((
                format!("data/fieldtest/horizon/block/{id}.json"),
                format!(r#"{{"template":"minecraft:{template}"}}"#),
            ));
        }
        for (name, text) in files {
            let path = dir.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let package = horizon_mods::Package::read(&dir).unwrap();
        let content = horizon_mods::Content::read(&dir, &package.manifest).unwrap();
        let out = scratch.join("resources");
        crate::mod_content::build(&base, &[ContentMod { id: "fieldtest".into(), content }], &out)
            .unwrap();
        Some(out)
    })
    .as_deref()
}

fn put(
    world: &mut crate::world::World,
    x: i32,
    y: i32,
    z: i32,
    state: horizon_world::registry::BlockStateId,
) {
    if !world.chunks.contains_key(&(x >> 4, z >> 4)) {
        world.chunks.insert(
            (x >> 4, z >> 4),
            Arc::new(crate::chunk::Chunk::empty(x >> 4, z >> 4)),
        );
    }
    world.set(x, y, z, state).unwrap();
}

/// `Ravager.aiStep` on a ravager whose box holds all four blocks; the
/// positions it destroys.
#[test]
fn a_ravager_crushes_by_class_and_not_by_name() {
    let Some(root) = resources() else {
        return;
    };
    let ran = world::with_resource_test_world(
        root,
        &[],
        true,
        true,
        |world, reg| {
            for (id, pos) in [
                ("fieldtest:grain", [0, 1, 0]),
                ("fieldtest:pod", [1, 1, 0]),
                ("fieldtest:canopy", [0, 1, 1]),
                ("fieldtest:fake_leaves", [0, 1, -1]),
            ] {
                put(
                    world,
                    pos[0],
                    pos[1],
                    pos[2],
                    reg.default_state(id).unwrap(),
                );
            }
        },
        |w, types| {
            let mut m = Mob::new(
                1,
                Arc::new(types.types["minecraft:ravager"].clone()),
                Vec3::new(0.5, 1.0, 0.5),
                0.0,
            );
            // Ravager.aiStep's leaf branch runs on a horizontal collision;
            // being off the ground keeps it from jumping.
            m.horizontal_collision = true;
            let mut events = Vec::new();
            super::ai_step(&mut m, w, &mut events);
            let mut broken: Vec<[i32; 3]> = events
                .iter()
                .filter_map(|e| match e {
                    MobEvent::BreakBlock { pos, drop } => {
                        assert!(drop, "the crop and the leaves drop");
                        Some(*pos)
                    }
                    _ => None,
                })
                .collect();
            broken.sort();
            assert_eq!(
                broken,
                vec![[0, 1, 0], [0, 1, 1], [1, 1, 0]],
                "the wheat and pitcher crops and the leaf go, the stone named \
                 fake_leaves stays"
            );
        },
    );
    let _ = ran;
}
