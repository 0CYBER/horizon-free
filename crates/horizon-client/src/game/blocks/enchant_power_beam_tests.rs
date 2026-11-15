//! Block parts answer the enchanting power an enchanting table counts (`enchant_power_bonus`)
//! and the colour a beacon beam takes (`beacon_color_multiplier`).
use crate::game::{Game, native_entity_refs::tests::with_game};
use horizon_client::chunk::Chunk;
use std::sync::Arc;

const TABLE: [i32; 3] = [8, 64, 8];

pub(in crate::game) fn place(game: &mut Game<'_>, offset: [i32; 3], block: &str) {
    let state = game.pack.assets.registry.default_state(block).unwrap();
    let [x, y, z] = [0, 1, 2].map(|i| TABLE[i] + offset[i]);
    game.world.set(x, y, z, state).unwrap();
}

#[test]
#[ignore = "daily"]
fn block_parts_answer_the_enchanting_power_and_the_beam_colour() {
    // The fixture's parts: glass is worth two bookshelves and tints a beam red, and a
    // bookshelf is worth nothing.
    mod_platform::__private::part_tests::enchant_power_and_beam_game_thread();
    with_game(|game| {
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        // A bookshelf whose part answers 0 is not counted.
        place(game, [-2, 0, -2], "minecraft:bookshelf");
        assert_eq!(game.count_bookshelves(TABLE), 0);
        // Glass answering 2 counts as two bookshelves.
        place(game, [-2, 0, -1], "minecraft:glass");
        assert_eq!(game.count_bookshelves(TABLE), 2);
        // Unless the block between it and the table does not transmit.
        place(game, [-1, 0, 0], "minecraft:stone");
        assert_eq!(game.count_bookshelves(TABLE), 0);

        let registry = game.pack.assets.registry;
        let color = |game: &Game<'_>, block: &str| {
            let state = registry.default_state(block).unwrap();
            game.beacon_color(state, block, [8, 65, 8], [8, 64, 8])
        };
        // Glass gives no colour of its own; its part tints the beam red, alpha dropped.
        assert_eq!(color(game, "minecraft:glass"), Some(0xFF_0000));
        // Stained glass without a part keeps its dye colour, and stone none.
        assert_eq!(color(game, "minecraft:red_stained_glass"), Some(0xB0_2E26));
        assert_eq!(color(game, "minecraft:stone"), None);
    });
}

/// Drive the public worker entry, observing real particle requests rather than cache bits.
pub(in crate::game) fn enchant_particles(game: &Game<'_>) -> usize {
    use horizon_client::{
        animate_tick::{AnimateContext, animate_tick},
        particles::{ParticleEngine, ParticleType},
    };
    let mut engine = ParticleEngine::new(
        &horizon_assets::resource_source::ResourceSource::directory(game.options.resources.clone()),
        &Default::default(),
        &Default::default(),
    );
    engine.seed_randoms(5, 9, 0);
    engine.camera = [8.5, 65.0, 8.5];
    engine.created_requests = Some(Vec::new());
    let ctx = AnimateContext {
        enchant_power: &game.ambience.enchant_power,
        world: &game.world,
        facts: game.res.facts(),
        collision: game.res.collision(),
        biomes: game.res.biomes(),
        registry: game.pack.assets.registry,
        raining_at: &|_| false,
        height_no_leaves: &|_| 0,
        firefly_bush_sounds: false,
        creaking_active: false,
        sky_darken: 0,
        marker: None,
        model_offset: &|_, _, _| None,
        gateway_particles: &|_, _| 0,
        thundering: false,
        ultrawarm: false,
    };
    for _ in 0..4096 {
        animate_tick(game.res.animate(), &mut engine, &ctx, TABLE);
    }
    engine
        .created_requests
        .take()
        .unwrap()
        .iter()
        .filter(|(options, ..)| options.ty == ParticleType::Enchant)
        .count()
}

#[test]
#[ignore = "daily"]
fn enchant_particles_share_native_power_and_rebuild_only_on_parts_reload() {
    use crate::game::native_entity_refs::tests::{run_fresh_cases, with_game_role};
    use mod_api::{BlockPart, Hooks, ModEntry, Mods, Registrar};
    use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

    const CHILD: &str = "HORIZON_ENCHANT_PARTICLES_CHILD";
    if run_fresh_cases(
        "game::enchant_power_beam_tests::enchant_particles_share_native_power_and_rebuild_only_on_parts_reload",
        CHILD,
        &["vanilla", "parts"],
    ) {
        return;
    }
    const CUSTOM: &str = "power_test:provider";
    static POWER: AtomicU32 = AtomicU32::new(2.0f32.to_bits());
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    struct Provider;
    #[mod_api::horizon_part]
    impl BlockPart for Provider {
        fn enchant_power_bonus(
            &self,
            _: &dyn mod_api::Host,
            event: &mut mod_api::EnchantPowerBonusEvent,
        ) {
            CALLS.fetch_add(1, Ordering::Relaxed);
            event.power = f32::from_bits(POWER.load(Ordering::Relaxed));
        }
    }
    struct Suppressed;
    #[mod_api::horizon_part]
    impl BlockPart for Suppressed {
        fn enchant_power_bonus(
            &self,
            _: &dyn mod_api::Host,
            event: &mut mod_api::EnchantPowerBonusEvent,
        ) {
            event.power = 0.0;
        }
    }
    struct PowerMod;
    #[mod_api::__subscriptions]
    impl Hooks for PowerMod {
        fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
            let provider = r.block_key(CUSTOM);
            r.block_part(provider, Provider);
            let bookshelf = r.block_key("minecraft:bookshelf");
            r.block_part(bookshelf, Suppressed);
        }
        fn blocks(&self, _: &mod_api::RegistrationContext, event: &mut mod_api::BlocksEvent<'_>) {
            event
                .blocks
                .push(mod_api::Block::new(CUSTOM, "minecraft:glass"));
        }
    }
    let parts = std::env::var(CHILD).unwrap() == "parts";
    let mods: Mods = if parts {
        vec![ModEntry::new("power_test", Box::new(PowerMod))].into()
    } else {
        Vec::new().into()
    };
    mod_platform::__private::install(mods.clone());
    crate::game::api_values::install_parts(&mods).unwrap();
    crate::entry::install_client_hooks();
    let base = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let root = crate::game::mods::content_root(&base, &Default::default())
        .unwrap()
        .prepared()
        .to_path_buf();
    with_game_role(
        root,
        crate::game::execution_role::ExecutionRole::Client,
        |game| {
            game.native_lifecycle
                .begin_open(mod_api::Authority::RemoteClient);
            game.resolve_parts().unwrap();
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            place(game, [0, 0, 0], "minecraft:enchanting_table");
            place(game, [-2, 0, -1], "minecraft:bookshelf");
            let shelves = enchant_particles(game);
            assert_eq!(shelves > 0, !parts, "bookshelf default or explicit zero");
            assert_eq!(game.count_bookshelves(TABLE), i32::from(!parts));
            if !parts {
                place(game, [-1, 0, 0], "minecraft:stone");
                assert_eq!(enchant_particles(game), 0);
                assert_eq!(game.count_bookshelves(TABLE), 0);
                return;
            }
            place(game, [-2, 0, -1], CUSTOM);
            assert_eq!(game.count_bookshelves(TABLE), 2);
            let calls = CALLS.load(Ordering::Relaxed);
            assert!(enchant_particles(game) > 0, "custom non-tagged provider");
            assert_eq!(
                CALLS.load(Ordering::Relaxed),
                calls,
                "no per-frame dispatch"
            );
            place(game, [-1, 0, 0], "minecraft:stone");
            assert_eq!(enchant_particles(game), 0);
            assert_eq!(game.count_bookshelves(TABLE), 0);
            place(game, [-1, 0, 0], "minecraft:air");
            POWER.store(0.0f32.to_bits(), Ordering::Relaxed);
            assert!(
                enchant_particles(game) > 0,
                "old snapshot until parts reload"
            );
            assert_eq!(CALLS.load(Ordering::Relaxed), calls);
            game.resolve_parts().unwrap();
            assert_eq!(
                CALLS.load(Ordering::Relaxed),
                calls + 1,
                "one query per custom state"
            );
            assert_eq!(
                enchant_particles(game),
                0,
                "reloaded answer suppresses particles"
            );
            assert_eq!(CALLS.load(Ordering::Relaxed), calls + 1);
            POWER.store((-0.5f32).to_bits(), Ordering::Relaxed);
            game.resolve_parts().unwrap();
            assert!(
                enchant_particles(game) > 0,
                "any nonzero power, not only positive"
            );
        },
    );
}
