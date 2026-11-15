//! A falling block's parts hear each tick it ends in the air (`falling_tick`), and not the
//! tick it lands.
use crate::game::{FallingBlock, native_entity_refs::tests::with_game};
use horizon_client::chunk::Chunk;
use horizon_core::{geometry::Vec3, position::BlockPos};
use mod_api::{
    BlockPart, FallingTickEvent, Hooks, ModEntry, Registrar, RegistrationContext, ServerHost,
};
use mod_platform::__private::part_tests::{falling_tick_game_thread, falling_ticks_heard};
use std::sync::{Arc, Mutex};

#[test]
#[ignore = "daily"]
fn a_falling_part_hears_each_airborne_tick_and_not_the_landing() {
    falling_tick_game_thread();
    with_game(|game| {
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let registry = game.pack.assets.registry;
        let stone = registry.default_state("minecraft:stone").unwrap();
        game.world.set(0, 60, 0, stone).unwrap();
        let sand = registry.default_state("minecraft:sand").unwrap();
        let at = Vec3::new(0.5, 70.0, 0.5);
        game.falling.push(FallingBlock {
            on_ground: false,
            attachment_lifetime: Default::default(),
            rotation: [0.0; 2],
            join: Default::default(),
            id: 71,
            state: sand,
            start_pos: BlockPos::new(0, 70, 0),
            position: at,
            previous: at,
            velocity: Vec3::ZERO,
            age: 0,
            no_gravity: false,
            portal: Default::default(),
            drop_item: false,
            fall_distance: 0.0,
            stuck: Vec3::ZERO,
            damage: Default::default(),
            saved: None,
        });
        let mut ticks = 0;
        while !game.falling.is_empty() {
            assert!(ticks < 200, "the sand never landed");
            game.tick_one_falling(0, &[]).unwrap();
            game.deliver_falling_ticks();
            ticks += 1;
        }
        // Every tick but the landing one.
        assert!(ticks > 1);
        assert_eq!(falling_ticks_heard(), ticks - 1);
    });
}

/// What the part heard: the falling block's id and its position as the part reads it.
type Heard = Arc<Mutex<Vec<(u32, Option<[f64; 3]>)>>>;

/// A sand part that records each falling tick and gives the block an upward velocity.
#[derive(Clone)]
struct Recorder(Heard);

#[mod_api::__subscriptions]
impl Hooks for Recorder {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let sand = r.block_key("minecraft:sand");
        r.block_part(sand, self.clone());
    }
}
#[mod_api::horizon_part]
impl BlockPart for Recorder {
    fn falling_tick(&self, host: &mut dyn ServerHost, event: &mut FallingTickEvent) {
        self.0
            .lock()
            .unwrap()
            .push((event.entity.id(), event.entity.position(host)));
        assert!(event.entity.set_velocity(host, [0.0, 0.5, 0.0]));
    }
}

fn sand_at(game: &mut crate::game::Game<'_>, id: u32) {
    let sand = game
        .pack
        .assets
        .registry
        .default_state("minecraft:sand")
        .unwrap();
    let at = Vec3::new(0.5, 70.0, 0.5);
    game.falling.push(FallingBlock {
        on_ground: false,
        attachment_lifetime: Default::default(),
        rotation: [0.0; 2],
        join: Default::default(),
        id,
        state: sand,
        start_pos: BlockPos::new(0, 70, 0),
        position: at,
        previous: at,
        velocity: Vec3::ZERO,
        age: 0,
        no_gravity: false,
        portal: Default::default(),
        drop_item: false,
        fall_distance: 0.0,
        stuck: Vec3::ZERO,
        damage: Default::default(),
        saved: None,
    });
}

fn with_recorder(check: impl FnOnce(&mut crate::game::Game<'_>, &Heard)) {
    let heard = Heard::default();
    let mods: mod_api::Mods =
        vec![ModEntry::new("recorder", Box::new(Recorder(heard.clone())))].into();
    mod_platform::testing::with_mods(mods.clone(), || {
        crate::game::api_values::install_parts(&mods).unwrap();
        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            check(game, &heard);
        });
    });
}

/// The listener names the falling block and reads where the tick left it, and what it writes
/// to the block's velocity is what the stored falling block carries afterwards.
#[test]
#[ignore = "daily"]
fn a_falling_listener_reads_its_block_and_its_velocity_write_lands() {
    with_recorder(|game, heard| {
        sand_at(game, 71);
        game.tick_one_falling(0, &[]).unwrap();
        game.deliver_falling_ticks();
        let heard = heard.lock().unwrap();
        let [(id, Some([x, y, z]))] = &heard[..] else {
            panic!("one tick heard with a position: {heard:?}");
        };
        assert_eq!((*id, *x, *z), (71, 0.5, 0.5));
        assert!(*y < 70.0, "the sand has fallen: {y}");
        assert_eq!(game.falling[0].velocity, Vec3::new(0.0, 0.5, 0.0));
    });
}

/// A falling block gone before the delivery is skipped, and the next pass starts empty.
#[test]
#[ignore = "daily"]
fn a_falling_block_gone_before_delivery_is_skipped() {
    with_recorder(|game, heard| {
        sand_at(game, 71);
        game.tick_one_falling(0, &[]).unwrap();
        game.falling.retain(|block| block.id != 71);
        game.deliver_falling_ticks();
        assert!(heard.lock().unwrap().is_empty());
        game.deliver_falling_ticks();
        assert!(heard.lock().unwrap().is_empty());
    });
}
