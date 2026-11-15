//! Engine registration and vanilla resource conversion for model layers.
use super::Part;
use mod_api::{LayerDefinition, ModelPart, ModelPolygon, ModelVertex, PartPose};
use std::{collections::HashMap, sync::OnceLock};

/// A vanilla layer's part tree from `entity-models.json`.
pub fn from_part(part: &Part) -> ModelPart {
    let p = part.pose;
    let mut pose = PartPose::offset_and_rotation(p[0], p[1], p[2], p[3], p[4], p[5]);
    pose.x_scale = p[6];
    pose.y_scale = p[7];
    pose.z_scale = p[8];
    let polygons = part
        .polygons
        .iter()
        .map(|poly| {
            ModelPolygon::new(
                poly.vertices
                    .map(|v| ModelVertex::new([v[0], v[1], v[2]], v[3], v[4])),
                poly.normal,
            )
        })
        .collect();
    let children = part
        .children
        .iter()
        .map(|c| (c.name.clone(), from_part(c)))
        .collect();
    ModelPart::new(polygons, children, pose)
}

static REGISTERED: OnceLock<HashMap<String, LayerDefinition>> = OnceLock::new();

/// Keeps the layers native mods registered (`native_register_layer_definitions`),
/// once at start. An id is `<namespace>:<path>#<layer>` (NeoForge's
/// `ModelLayerLocation`) in a namespace other than vanilla's; one that isn't,
/// or that another layer has already, is logged and left out.
pub fn register(layers: Vec<(String, LayerDefinition)>) {
    let mut kept = HashMap::new();
    for (id, layer) in layers {
        if !is_mod_layer_id(&id) {
            log::warn!("model layer {id:?} is not <namespace>:<path>#<layer> in a mod's namespace");
            continue;
        }
        if kept.contains_key(&id) {
            log::warn!("model layer {id} is registered twice; the first is kept");
            continue;
        }
        kept.insert(id, layer);
    }
    if REGISTERED.set(kept).is_err() {
        log::warn!("model layers were registered already; these are left out");
    }
}

/// The layer a native mod registered as `id`.
pub fn registered(id: &str) -> Option<&'static LayerDefinition> {
    REGISTERED.get()?.get(id)
}

fn is_mod_layer_id(id: &str) -> bool {
    let Some((location, layer)) = id.split_once('#') else {
        return false;
    };
    let Some((namespace, path)) = location.split_once(':') else {
        return false;
    };
    let ok = |s: &str, extra: &[char]| {
        !s.is_empty()
            && s.chars().all(|c| {
                c.is_ascii_lowercase()
                    || c.is_ascii_digit()
                    || "_.-".contains(c)
                    || extra.contains(&c)
            })
    };
    namespace != "minecraft"
        && ok(namespace, &[])
        && ok(path, &['/'])
        && !layer.is_empty()
        && !layer.contains(['#', ':'])
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::{CubeDeformation, CubeListBuilder, MeshDefinition};
    #[test]
    fn layer_ids_are_a_mods_own_model_and_layer() {
        assert!(is_mod_layer_id("mymod:golem#main"));
        assert!(is_mod_layer_id("my_mod:mobs/golem.v2#outer_armor"));
        for bad in [
            "minecraft:zombie#main",
            "mymod:golem",
            "golem#main",
            "mymod:#main",
            "mymod:golem#",
            "MyMod:golem#main",
            "mymod:golem#a#b",
        ] {
            assert!(!is_mod_layer_id(bad), "{bad}");
        }
    }
    /// `HumanoidModel.createMesh(CubeDeformation.NONE, 0)` built with these
    /// builders bakes what vanilla baked for the zombie
    /// (`minecraft:zombie#main` in the prepared `entity-models.json`):
    /// `cargo test -p horizon-client-lib --lib model_layers -- --ignored`
    /// with HORIZON_TEST_RESOURCES set to the prepared resources.
    #[test]
    #[ignore = "daily"]
    fn the_humanoid_mesh_bakes_as_vanillas_zombie() {
        let root = std::path::PathBuf::from(
            std::env::var_os("HORIZON_TEST_RESOURCES").expect("HORIZON_TEST_RESOURCES"),
        );
        let vanilla =
            super::super::load_model(&root, "minecraft:zombie#main").expect("zombie#main");
        let vanilla = from_part(&vanilla);

        // `HumanoidModel.createMesh`: (part, parent, texOffs, mirror, origin, size, grow, pose).
        let half = CubeDeformation::new(0.5);
        let none = CubeDeformation::NONE;
        let (z4, limb) = ([-4.0, -8.0, -4.0], [4.0, 12.0, 4.0]);
        #[rustfmt::skip]
        let parts = [
            ("head", "", (0, 0), false, z4, [8.0; 3], none, [0.0; 3]),
            ("hat", "head", (32, 0), false, z4, [8.0; 3], half, [0.0; 3]),
            ("body", "", (16, 16), false, [-4.0, 0.0, -2.0], [8.0, 12.0, 4.0], none, [0.0; 3]),
            ("right_arm", "", (40, 16), false, [-3.0, -2.0, -2.0], limb, none, [-5.0, 2.0, 0.0]),
            ("left_arm", "", (40, 16), true, [-1.0, -2.0, -2.0], limb, none, [5.0, 2.0, 0.0]),
            ("right_leg", "", (0, 16), false, [-2.0, 0.0, -2.0], limb, none, [-1.9, 12.0, 0.0]),
            ("left_leg", "", (0, 16), true, [-2.0, 0.0, -2.0], limb, none, [1.9, 12.0, 0.0]),
        ];
        let mut mesh = MeshDefinition::new();
        for (name, parent, (u, v), mirror, origin, size, grow, [x, y, z]) in parts {
            let parent = mesh.child(mesh.root(), parent).unwrap_or(mesh.root());
            let cubes = CubeListBuilder::create()
                .tex_offs(u, v)
                .set_mirror(mirror)
                .add_box_with(origin, size, grow);
            mesh.add_or_replace_child(parent, name, cubes, PartPose::offset(x, y, z));
        }
        let ours = LayerDefinition::create(mesh, 64, 64).bake_root();

        fn same(path: &str, ours: &ModelPart, vanilla: &ModelPart) {
            assert_eq!(ours.store_pose(), vanilla.store_pose(), "{path} pose");
            assert_eq!(ours.polygons, vanilla.polygons, "{path} faces");
            let mut names: Vec<_> = ours.children().map(|(n, _)| n).collect();
            let mut theirs: Vec<_> = vanilla.children().map(|(n, _)| n).collect();
            names.sort_unstable();
            theirs.sort_unstable();
            assert_eq!(names, theirs, "{path} children");
            for (name, child) in ours.children() {
                same(
                    &format!("{path}/{name}"),
                    child,
                    vanilla.get_child(name).unwrap(),
                );
            }
        }
        same("root", &ours, &vanilla);
    }
}
