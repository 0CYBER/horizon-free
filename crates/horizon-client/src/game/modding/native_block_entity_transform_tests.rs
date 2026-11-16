//! Placement exercises the real processor and the game-side registration bridge.
use super::native_block_entity_transform::prepare;
use horizon_core::nbt::{Compound, Limits, Tag, write_network};
use horizon_world::{
    registry::{BlockRegistry, BlockStateId},
    worldgen::{
        anchor::WorldGenerationContext,
        feature::FeatureWorld,
        placement::{PlacementContext, PlacementQueryError},
        structure::{
            Mirror, Rotation, StateTransforms, StructureResources, StructureTemplate,
            processor::{PlaceSettings, place_in_world},
        },
    },
};
use std::sync::{Arc, Mutex};

type Calls = Arc<Mutex<Vec<mod_api::StructureTransform>>>;
struct Registration(Calls);
#[mod_api::__subscriptions]
impl mod_api::Hooks for Registration {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        let calls = self.0.clone();
        r.block_entity_type("machine")
            .transform(move |data: &Compound, t: mod_api::StructureTransform| {
                calls.lock().unwrap().push(t);
                let mut data = data.clone();
                let facing = match data.get(&"facing".into()) {
                    Some(Tag::String(s)) => s.to_string().unwrap(),
                    _ => return Err(mod_api::TransformRefusal::new("missing facing")),
                };
                let facing = match (t.mirror, facing.as_str()) {
                    (mod_api::StructureMirror::LeftRight, "north") => "south",
                    (mod_api::StructureMirror::LeftRight, "south") => "north",
                    (mod_api::StructureMirror::FrontBack, "east") => "west",
                    (mod_api::StructureMirror::FrontBack, "west") => "east",
                    (_, f) => f,
                };
                let directions = ["north", "east", "south", "west"];
                let index = directions.iter().position(|f| *f == facing).unwrap();
                let turns = match t.rotation {
                    mod_api::StructureRotation::None => 0,
                    mod_api::StructureRotation::Clockwise90 => 1,
                    mod_api::StructureRotation::Clockwise180 => 2,
                    mod_api::StructureRotation::Counterclockwise90 => 3,
                    _ => unreachable!(),
                };
                data.insert(
                    "facing".into(),
                    Tag::String(directions[(index + turns) % 4].into()),
                );
                Ok(data)
            })
            .build();
        r.block_entity_type("hostile")
            .transform(|_: &Compound, _: mod_api::StructureTransform| {
                Ok([
                    ("id".into(), Tag::String("minecraft:chest".into())),
                    ("facing".into(), Tag::String("east".into())),
                ]
                .into())
            })
            .build();
        r.block_entity_type("plain").build();
        r.block_entity_type("refuses")
            .transform(|_: &Compound, _: mod_api::StructureTransform| {
                Err(mod_api::TransformRefusal::new("fixture refusal"))
            })
            .build();
        r.block_entity_type("too_deep")
            .transform(|_: &Compound, _: mod_api::StructureTransform| {
                let mut tag = Tag::Byte(1);
                for _ in 0..514 {
                    tag = Tag::Compound([("child".into(), tag)].into());
                }
                Ok([("nested".into(), tag)].into())
            })
            .build();
        r.block_entity_type("panics")
            .transform(
                |_: &Compound,
                 _: mod_api::StructureTransform|
                 -> Result<Compound, mod_api::TransformRefusal> {
                    panic!("fixture panic")
                },
            )
            .build();
    }
}
struct World {
    state: BlockStateId,
    stored: Option<Compound>,
}
impl FeatureWorld for World {
    type Error = PlacementQueryError;
    fn block(&mut self, _: [i32; 3]) -> Result<BlockStateId, Self::Error> {
        Ok(self.state)
    }
    fn set_block(&mut self, _: [i32; 3], state: BlockStateId, _: i32) -> Result<bool, Self::Error> {
        self.state = state;
        Ok(true)
    }
}
impl PlacementContext for World {
    fn set_block_entity(&mut self, _: [i32; 3], data: Compound) -> Result<(), PlacementQueryError> {
        assert_ne!(self.state, BlockStateId(1));
        self.stored = Some(data);
        Ok(())
    }
    fn generation(&self) -> WorldGenerationContext {
        WorldGenerationContext::new(0, 256, 63)
    }
    fn min_y(&mut self) -> Result<i32, PlacementQueryError> {
        Ok(0)
    }
    fn height(
        &mut self,
        _: horizon_world::heightmap::HeightmapType,
        _: i32,
        _: i32,
    ) -> Result<i32, PlacementQueryError> {
        Ok(64)
    }
}
const NAMES: [&str; 6] = [
    "minecraft:air",
    "minecraft:barrier",
    "test:north",
    "test:east",
    "test:south",
    "test:west",
];
fn resources(dir: &std::path::Path) -> StructureResources {
    let registry = BlockRegistry::from_nbt(&Tag::compound([
        ("DataVersion", Tag::Int(horizon_core::version::DATA_VERSION)),
        (
            "states",
            Tag::List(
                NAMES
                    .iter()
                    .map(|n| Tag::compound([("id", Tag::String((*n).into()))]))
                    .collect(),
            ),
        ),
        (
            "defaults",
            Tag::compound(NAMES.iter().zip(0..).map(|(n, i)| (*n, Tag::Int(i)))),
        ),
    ]))
    .unwrap();
    // Hand-written vanilla north/east/south/west transforms; barrier and air stay fixed.
    let transforms = StateTransforms::from_json(
        r#"{
        "rotate":[[0,0,0],[1,1,1],[3,4,5],[4,5,2],[5,2,3],[2,3,4]],
        "mirror":[[0,0],[1,1],[4,2],[3,5],[2,4],[5,3]],
        "full_block":[false,true,false,false,false,false],
        "loot_containers":[],"block_entities":{}}
    "#,
        6,
    )
    .unwrap();
    StructureResources::new(
        Box::leak(Box::new(registry)),
        transforms,
        dir,
        Default::default(),
        &Default::default(),
    )
}
fn place(
    resources: &StructureResources,
    id: &str,
    facing: &str,
    rotation: Rotation,
    mirror: Mirror,
) -> (World, Compound) {
    place_block(
        resources,
        id,
        facing,
        &format!("test:{facing}"),
        rotation,
        mirror,
    )
}
fn place_block(
    resources: &StructureResources,
    id: &str,
    facing: &str,
    block: &str,
    rotation: Rotation,
    mirror: Mirror,
) -> (World, Compound) {
    let data: Compound = [
        ("id".into(), Tag::String(id.into())),
        ("facing".into(), Tag::String(facing.into())),
        ("x".into(), Tag::Int(3)),
        ("y".into(), Tag::Int(64)),
        ("z".into(), Tag::Int(5)),
        ("bits".into(), Tag::Float(0x7fc00001)),
        (
            "utf16".into(),
            Tag::String(horizon_core::nbt::JavaString(vec![0xd800])),
        ),
    ]
    .into();
    let tag = Tag::compound([
        ("size", Tag::List(vec![Tag::Int(1); 3])),
        (
            "palette",
            Tag::List(vec![Tag::compound([("Name", Tag::String(block.into()))])]),
        ),
        (
            "blocks",
            Tag::List(vec![Tag::compound([
                ("pos", Tag::List(vec![Tag::Int(0); 3])),
                ("state", Tag::Int(0)),
                ("nbt", Tag::Compound(data.clone())),
            ])]),
        ),
    ]);
    let template = StructureTemplate::from_nbt(&tag, resources.registry, &resources.transforms);
    let mut world = World {
        state: BlockStateId(0),
        stored: None,
    };
    let mut settings = PlaceSettings::new(rotation);
    settings.mirror = mirror;
    settings.known_shape = true;
    settings.waterlogging = false;
    assert!(
        place_in_world(
            &mut world,
            resources,
            &template,
            [3, 64, 5],
            [3, 64, 5],
            &mut settings,
            None,
            2
        )
        .unwrap()
    );
    (world, data)
}
struct Capture(Mutex<Vec<String>>);
impl log::Log for Capture {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }
    fn log(&self, record: &log::Record<'_>) {
        self.0.lock().unwrap().push(record.args().to_string());
    }
    fn flush(&self) {}
}
static LOG: Capture = Capture(Mutex::new(Vec::new()));

#[test]
#[ignore = "daily"]
fn structure_transform_placement_rotations_mirrors_unset_and_refusal() {
    // The logger is process-global; isolate this assertion from other log-capture tests.
    if std::env::var_os("HORIZON_STRUCTURE_TRANSFORM_LOG_CHILD").is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--include-ignored", "--exact", "game::native_block_entity_transform_tests::structure_transform_placement_rotations_mirrors_unset_and_refusal", "--nocapture"])
            .env("HORIZON_STRUCTURE_TRANSFORM_LOG_CHILD", "1")
            .status().unwrap();
        assert!(status.success());
        return;
    }
    log::set_logger(&LOG).unwrap();
    log::set_max_level(log::LevelFilter::Warn);
    let calls = Calls::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "test",
        Box::new(Registration(calls.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods.clone(), || {
        mod_platform::__private::part_tests::install_parts_fixture(&mods).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let resources = resources(dir.path());
        prepare(resources.registry);
        for (rotation, mirror, input, expected, api_rotation, api_mirror) in [
            (
                Rotation::None,
                Mirror::None,
                "north",
                "north",
                mod_api::StructureRotation::None,
                mod_api::StructureMirror::None,
            ),
            (
                Rotation::None,
                Mirror::LeftRight,
                "north",
                "south",
                mod_api::StructureRotation::None,
                mod_api::StructureMirror::LeftRight,
            ),
            (
                Rotation::None,
                Mirror::FrontBack,
                "north",
                "north",
                mod_api::StructureRotation::None,
                mod_api::StructureMirror::FrontBack,
            ),
            (
                Rotation::Clockwise90,
                Mirror::None,
                "north",
                "east",
                mod_api::StructureRotation::Clockwise90,
                mod_api::StructureMirror::None,
            ),
            (
                Rotation::Clockwise90,
                Mirror::LeftRight,
                "north",
                "west",
                mod_api::StructureRotation::Clockwise90,
                mod_api::StructureMirror::LeftRight,
            ),
            (
                Rotation::Clockwise90,
                Mirror::FrontBack,
                "north",
                "east",
                mod_api::StructureRotation::Clockwise90,
                mod_api::StructureMirror::FrontBack,
            ),
            (
                Rotation::Clockwise180,
                Mirror::None,
                "north",
                "south",
                mod_api::StructureRotation::Clockwise180,
                mod_api::StructureMirror::None,
            ),
            (
                Rotation::Clockwise180,
                Mirror::LeftRight,
                "north",
                "north",
                mod_api::StructureRotation::Clockwise180,
                mod_api::StructureMirror::LeftRight,
            ),
            (
                Rotation::Clockwise180,
                Mirror::FrontBack,
                "north",
                "south",
                mod_api::StructureRotation::Clockwise180,
                mod_api::StructureMirror::FrontBack,
            ),
            (
                Rotation::Counterclockwise90,
                Mirror::None,
                "north",
                "west",
                mod_api::StructureRotation::Counterclockwise90,
                mod_api::StructureMirror::None,
            ),
            (
                Rotation::Counterclockwise90,
                Mirror::LeftRight,
                "north",
                "east",
                mod_api::StructureRotation::Counterclockwise90,
                mod_api::StructureMirror::LeftRight,
            ),
            (
                Rotation::Counterclockwise90,
                Mirror::FrontBack,
                "north",
                "west",
                mod_api::StructureRotation::Counterclockwise90,
                mod_api::StructureMirror::FrontBack,
            ),
        ] {
            let (world, _) = place(&resources, "test:machine", input, rotation, mirror);
            assert_eq!(
                calls.lock().unwrap().pop(),
                Some(mod_api::StructureTransform::new(api_rotation, api_mirror))
            );
            assert_eq!(
                world.state,
                BlockStateId(
                    NAMES
                        .iter()
                        .position(|n| *n == format!("test:{expected}"))
                        .unwrap() as u32
                )
            );
            assert_eq!(
                world.stored.unwrap().get(&"facing".into()),
                Some(&Tag::String(expected.into()))
            );
        }
        // Keep the east-facing mirror cases alongside the full north-facing matrix.
        for (mirror, expected) in [(Mirror::LeftRight, "east"), (Mirror::FrontBack, "west")] {
            let (world, _) = place(&resources, "test:machine", "east", Rotation::None, mirror);
            assert_eq!(
                world.stored.unwrap().get(&"facing".into()),
                Some(&Tag::String(expected.into()))
            );
            assert!(calls.lock().unwrap().pop().is_some());
        }
        for id in [
            "test:plain",
            "test:too_deep",
            "test:refuses",
            "test:panics",
            "minecraft:chest",
        ] {
            let (world, original) =
                place(&resources, id, "north", Rotation::Clockwise90, Mirror::None);
            let stored = world.stored.unwrap();
            assert_eq!(stored, original);
            assert_eq!(
                write_network(&Tag::Compound(stored), Limits::default()).unwrap(),
                write_network(&Tag::Compound(original), Limits::default()).unwrap()
            );
            assert!(calls.lock().unwrap().is_empty());
        }
        let logs = LOG.0.lock().unwrap();
        assert!(logs.iter().any(|line| line.contains("test:refuses")
            && line.contains("fixture refusal")
            && line.contains("64")
            && line.contains("3")
            && line.contains("5")));
        assert!(
            logs.iter()
                .any(|line| line.contains("test:panics") && line.contains("keeping original data"))
        );
    });
}

#[test]
fn structure_transform_unregistered_path_keeps_data_without_an_adapter() {
    let dir = tempfile::tempdir().unwrap();
    let resources = resources(dir.path());
    assert!(
        resources
            .registry
            .structure_block_entity_transform
            .get()
            .is_none()
    );
    let (world, original) = place(
        &resources,
        "test:machine",
        "north",
        Rotation::Clockwise90,
        Mirror::FrontBack,
    );
    assert_eq!(world.state, BlockStateId(3));
    assert_eq!(world.stored, Some(original));
}

#[test]
fn structure_transform_hostile_output_keeps_original_identity() {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "test",
        Box::new(Registration(Calls::default())),
    )]
    .into();
    mod_platform::testing::with_mods(mods.clone(), || {
        mod_platform::__private::part_tests::install_parts_fixture(&mods).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let resources = resources(dir.path());
        prepare(resources.registry);
        let (world, _) = place(
            &resources,
            "test:hostile",
            "north",
            Rotation::Clockwise90,
            Mirror::None,
        );
        let stored = world.stored.unwrap();
        assert_eq!(
            stored,
            [
                ("id".into(), Tag::String("test:hostile".into())),
                ("x".into(), Tag::Int(3)),
                ("y".into(), Tag::Int(64)),
                ("z".into(), Tag::Int(5)),
                ("facing".into(), Tag::String("east".into())),
            ]
            .into()
        );
    });
}

#[test]
#[ignore = "daily"]
fn structure_transform_survives_real_datapack_reload_with_its_registry() {
    use crate::game::{native_entity_refs::tests::with_game, world_loading::WorldRequest};
    let calls = Calls::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "test",
        Box::new(Registration(calls.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let dir = tempfile::tempdir().unwrap();
            let mut request = WorldRequest::new(
                "transform_reload",
                Some("transform_reload"),
                Some(1),
                true,
                None,
                Arc::new(horizon_client::gamerules::RuleRegistry::vanilla()),
            )
            .unwrap();
            request.save_root = dir.path().to_owned();
            game.load_world(request).unwrap();
            let registry = game.pack.assets.registry;
            let frozen = registry
                .structure_block_entity_transform
                .get()
                .unwrap()
                .clone();
            let transforms = StateTransforms::from_json(
                &std::fs::read_to_string(
                    game.options
                        .resources
                        .join("worldgen/blocks/structure-blocks.json"),
                )
                .unwrap(),
                registry.len(),
            )
            .unwrap();
            let resources = StructureResources::new(
                registry,
                transforms,
                dir.path(),
                Default::default(),
                &Default::default(),
            );
            let check_placement = || {
                let (world, _) = place_block(
                    &resources,
                    "test:machine",
                    "north",
                    "minecraft:furnace",
                    Rotation::Clockwise90,
                    Mirror::None,
                );
                assert_eq!(
                    world.stored.unwrap().get(&"facing".into()),
                    Some(&Tag::String("east".into()))
                );
                assert_eq!(
                    registry.state(world.state).unwrap().properties["facing"],
                    "east"
                );
            };
            check_placement();
            assert!(game.run_command("reload"));
            assert!(std::ptr::eq(registry, game.pack.assets.registry));
            assert!(Arc::ptr_eq(
                &frozen,
                game.pack
                    .assets
                    .registry
                    .structure_block_entity_transform
                    .get()
                    .unwrap()
            ));
            check_placement();
            assert_eq!(
                *calls.lock().unwrap(),
                vec![
                    mod_api::StructureTransform::new(
                        mod_api::StructureRotation::Clockwise90,
                        mod_api::StructureMirror::None
                    );
                    2
                ]
            );
        })
    });
}
