#![cfg(test)]
//! Post-placement notices through the actual player and dispenser block-item paths.
use crate::game::native_entity_refs::tests::{bind_local_player, run_fresh_cases, with_game_at};
use crate::game::{Game, GuestUse, Hit};
use horizon_client::{
    chunk::Chunk, containers::BlockEntity, items::ItemStack, motion::PlayerMotion,
};
use horizon_core::{
    geometry::{Direction, Vec3},
    nbt::{Compound, Tag},
};
use mod_api::{
    BlockEntityPart, BlockPart, Hooks, HostBlockStates, ItemPart, OnBlockPlacedByEvent, ServerHost,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

const CRATE: &str = "placed_probe:crate";

#[derive(Debug, PartialEq)]
struct Notice {
    item: String,
    count: u32,
    placer: Option<u128>,
    pos: [i32; 3],
    face: mod_api::BlockFace,
    click: [f64; 3],
}

#[derive(Clone, Default)]
struct Probe {
    notices: Arc<Mutex<Vec<Notice>>>,
    cancel: Arc<AtomicBool>,
}

struct Crate;
#[mod_api::horizon_part]
impl BlockPart for Crate {}
#[mod_api::horizon_part]
impl BlockEntityPart for Crate {
    fn block_entity(
        &self,
        _: mod_api::LevelSide<'_>,
        _: &mut mod_api::BlockEntityEvent<'_>,
    ) -> Option<Compound> {
        Some(Compound::from([("Marker".into(), Tag::Int(0))]))
    }
}

#[mod_api::horizon_part]
impl ItemPart for Probe {
    fn on_block_placed_by(&self, host: &mut dyn ServerHost, event: &mut OnBlockPlacedByEvent<'_>) {
        let level = host.level("minecraft:overworld").unwrap();
        assert_eq!(level.block_state(host, event.pos), Some(event.state));
        let item = event.stack.item().id(host);
        match item.as_str() {
            CRATE => {
                let entity = host
                    .block_entity(level, event.pos)
                    .expect("new block entity");
                assert_eq!(
                    entity.data(host).unwrap().get(&"Marker".into()),
                    Some(&Tag::Int(7)),
                    "stack block-entity data must already be loaded"
                );
                let name = event
                    .stack
                    .components()
                    .unwrap()
                    .as_nbt()
                    .get(&"minecraft:custom_name".into())
                    .unwrap()
                    .clone();
                assert_eq!(
                    entity.merge_data(
                        host,
                        Compound::from([
                            ("CustomName".into(), name),
                            ("PlacedByPart".into(), Tag::Byte(1)),
                        ])
                    ),
                    Ok(true)
                );
            }
            "minecraft:red_bed" => {
                assert_eq!(
                    event.state.property(&host.block_states(), "part").unwrap(),
                    Some("foot")
                );
                let [x, y, z] = event.pos;
                let head = level
                    .block_state(host, [x, y, z + 1])
                    .expect("bed head exists");
                assert_eq!(head.block(), event.state.block());
                assert_eq!(
                    head.property(&host.block_states(), "part").unwrap(),
                    Some("head")
                );
            }
            "minecraft:shulker_box" => {
                assert!(event.placer.is_none(), "dispensers have no placer");
                assert_eq!(
                    event
                        .state
                        .property(&host.block_states(), "facing")
                        .unwrap(),
                    Some("north")
                );
                let entity = host
                    .block_entity(level, event.pos)
                    .expect("shulker entity exists");
                assert_eq!(entity.name(host).as_deref(), Some("Shulker"));
            }
            _ => panic!("unsubscribed item {item}"),
        }
        self.notices.lock().unwrap().push(Notice {
            item,
            count: event.stack.count(),
            placer: event.placer.map(|player| player.uuid()),
            pos: event.pos,
            face: event.face,
            click: event.click,
        });
    }
}

#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        let kind = r.block_entity_type("crate").blocks(&[CRATE]).build();
        r.block_entity_part(&kind, Crate).unwrap();
        for item in [CRATE, "minecraft:red_bed", "minecraft:shulker_box"] {
            let key = r.item_key(item);
            r.item_part(key, self.clone());
        }
    }
    fn blocks(&self, _: &mod_api::RegistrationContext, event: &mut mod_api::BlocksEvent<'_>) {
        event
            .blocks
            .push(mod_api::Block::new(CRATE, "minecraft:stone"));
    }
    fn entity_place(
        &self,
        _: &mut dyn ServerHost,
        event: &mut mod_api::EntityPlaceEvent,
    ) -> mod_api::Flow {
        if self.cancel.load(Ordering::SeqCst) {
            if event.placer.is_none() {
                assert!(matches!(event.placement, mod_api::Placement::Single));
            }
            mod_api::Flow::Cancel
        } else {
            mod_api::Flow::Continue
        }
    }
}

fn place(game: &mut Game<'_>, pos: [i32; 3], stack: ItemStack) {
    *game.held_mut() = Some(stack);
    game.guest_use = Some(GuestUse::OnBlock(mod_api::Hand::Main));
    let [x, y, z] = pos;
    let hit = Hit {
        pos: [x, y - 1, z],
        face: [0, 1, 0],
        point: [f64::from(x) + 0.25, f64::from(y), f64::from(z) + 0.75],
        inside: false,
    };
    assert!(game.use_hand(Some(&hit)).unwrap());
}

fn crate_stack() -> ItemStack {
    let mut stack = ItemStack::new(CRATE, 3);
    stack.data_mut().custom_name = Some(Tag::String("Crate name".into()));
    stack.data_mut().set_other(
        "minecraft:block_entity_data",
        Some(Tag::Compound(Compound::from([
            ("id".into(), Tag::String(CRATE.into())),
            ("Marker".into(), Tag::Int(7)),
        ]))),
    );
    stack
}

fn shulker_stack() -> ItemStack {
    let mut stack = ItemStack::new("minecraft:shulker_box", 4);
    stack.data_mut().custom_name = Some(Tag::String("Shulker".into()));
    stack.data_mut().set_other(
        "minecraft:block_state",
        Some(Tag::Compound(Compound::from([(
            "facing".into(),
            Tag::String("north".into()),
        )]))),
    );
    stack
}

#[test]
#[ignore = "daily"]
fn block_items_notify_after_setup_and_before_consumption() {
    if run_fresh_cases(
        "game::native_hooks::on_block_placed_by_tests::block_items_notify_after_setup_and_before_consumption",
        "HORIZON_BLOCK_ITEM_PLACED_CHILD",
        &["game"],
    ) {
        return;
    }
    let probe = Probe::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "placed_probe",
        Box::new(probe.clone()),
    )]
    .into();
    mod_platform::__private::install(mods.clone());
    crate::game::api_values::install_parts(&mods).unwrap();
    let base = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let root = crate::game::mods::content_root(&base, &Default::default())
        .unwrap()
        .prepared()
        .to_path_buf();
    with_game_at(root, |game| {
        game.options.dedicated = Some(Vec::new());
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.block_ticks
            .add_empty_chunk(horizon_core::position::ChunkPos::new(0, 0))
            .unwrap();
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        let air = game.res.facts().air;
        for x in 0..16 {
            for z in 0..16 {
                game.world.set(x, 63, z, stone).unwrap();
            }
        }
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(12.5, 64.0, 12.5)).unwrap());
        bind_local_player(game, 20, 123);
        game.lan_acting_name = Some("Placer".into());
        game.lan_acting_permission = Some(0);
        game.creative = false;
        game.yaw = 0.0;

        // The global veto runs before placement; neither the block nor its Part's write exists.
        probe.cancel.store(true, Ordering::SeqCst);
        place(game, [2, 64, 2], crate_stack());
        assert!(probe.notices.lock().unwrap().is_empty());
        assert_eq!(game.world.block(2, 64, 2), air);
        assert!(game.world.block_entities.get(&(2, 64, 2)).is_none());
        assert_eq!(game.held().unwrap().count, 3);

        probe.cancel.store(false, Ordering::SeqCst);
        place(game, [2, 64, 2], crate_stack());
        let Some(BlockEntity::Mod(entity)) = game.world.block_entities.get(&(2, 64, 2)) else {
            panic!("placed mod entity missing");
        };
        assert_eq!(
            entity.nbt.get(&"CustomName".into()),
            Some(&Tag::String("Crate name".into()))
        );
        assert_eq!(entity.nbt.get(&"PlacedByPart".into()), Some(&Tag::Byte(1)));
        assert_eq!(game.held().unwrap().count, 2);

        // A refused bed (blocked head) never notifies, while a successful bed notifies only its foot.
        game.world.set(8, 64, 3, stone).unwrap();
        place(game, [8, 64, 2], ItemStack::new("minecraft:red_bed", 2));
        assert_eq!(game.world.block(8, 64, 2), air);
        assert_eq!(game.held().unwrap().count, 2);
        assert_eq!(probe.notices.lock().unwrap().len(), 1);
        place(game, [6, 64, 2], ItemStack::new("minecraft:red_bed", 2));
        assert_eq!(game.held().unwrap().count, 1);

        // The dispenser enters the same BlockItem placement path with no player and sees the
        // component-adjusted state, loaded block entity and full original stack before shrink.
        let left = game
            .dispense_item((9, 64, 6), Direction::East, shulker_stack())
            .unwrap()
            .unwrap();
        assert_eq!(left.count, 3);
        let notices = probe.notices.lock().unwrap();
        assert_eq!(
            *notices,
            [
                Notice {
                    item: CRATE.into(),
                    count: 3,
                    placer: Some(123),
                    pos: [2, 64, 2],
                    face: mod_api::BlockFace::Up,
                    click: [2.25, 64.0, 2.75]
                },
                Notice {
                    item: "minecraft:red_bed".into(),
                    count: 2,
                    placer: Some(123),
                    pos: [6, 64, 2],
                    face: mod_api::BlockFace::Up,
                    click: [6.25, 64.0, 2.75]
                },
                Notice {
                    item: "minecraft:shulker_box".into(),
                    count: 4,
                    placer: None,
                    pos: [10, 64, 6],
                    face: mod_api::BlockFace::Up,
                    click: [10.5, 64.0, 6.5]
                },
            ]
        );
        drop(notices);

        probe.cancel.store(true, Ordering::SeqCst);
        let left = game
            .dispense_item((9, 64, 8), Direction::East, shulker_stack())
            .unwrap()
            .unwrap();
        assert_eq!(left.count, 4);
        assert_eq!(game.world.block(10, 64, 8), air);
        probe.cancel.store(false, Ordering::SeqCst);
        game.world.set(10, 64, 10, stone).unwrap();
        let left = game
            .dispense_item((9, 64, 10), Direction::East, shulker_stack())
            .unwrap()
            .unwrap();
        assert_eq!(left.count, 4);
        assert_eq!(game.world.block(10, 64, 10), stone);
        // An unrelated item still places normally without dispatching any Part.
        place(game, [4, 64, 2], ItemStack::new("minecraft:stone", 2));
        assert_eq!(game.world.block(4, 64, 2), stone);
        assert_eq!(game.held().unwrap().count, 1);
        assert_eq!(probe.notices.lock().unwrap().len(), 3);
    });
}
