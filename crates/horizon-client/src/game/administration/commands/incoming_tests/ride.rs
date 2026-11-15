//! `/ride`: `EntityArgument.parse` refuses a selector whose `getMaxResults`
//! is over one for a single entity argument, and `Mob.startRiding` drops the
//! lead of the mob that mounts.
use crate::game::*;
use horizon_client::mobs::leash::Holder;

const PIG: &str = "0000609a-0000-0000-0000-000000000001";
const COW: &str = "0000609a-0000-0000-0000-000000000002";
const PIGLIN: &str = "0000609a-0000-0000-0000-000000000003";
const HOGLIN: &str = "0000609a-0000-0000-0000-000000000004";

/// A server-side game on a stone floor at y=200 with an oak fence at 8 201 0
/// to tie a lead to, or None when the prepared resources are missing (CI has
/// no vanilla data).
fn game<'a>(resources: &'a mut Option<std::sync::Arc<crate::game::Resources>>) -> Option<Game<'a>> {
    let root = std::env::var_os("HORIZON_TEST_RESOURCES")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../.local/horizon-client")
                .join(horizon_core::VANILLA_VERSION)
        });
    if !root.join("movement-properties.nbt").exists() {
        return None;
    }
    let settings = Settings::default();
    let loaded = load_resources(
        horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        crate::game::execution_role::ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .ok()?;
    let mut options = Options::parse(std::iter::empty()).ok()??;
    options.resources = root.clone();
    options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
    let res = resources.insert(loaded.resources);
    let mut game = Game::new(
        res,
        loaded.pack,
        loaded.gui?,
        options,
        settings,
        Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
            root.clone(),
        )),
        None,
    )
    .ok()?;
    game.build_registries();
    game.set_allow_commands(true);
    for x in 0..2 {
        game.world.chunks.insert(
            (x, 0),
            std::sync::Arc::new(horizon_client::chunk::Chunk::empty(x, 0)),
        );
        game.block_ticks
            .add_empty_chunk(horizon_core::position::ChunkPos::new(x, 0))
            .unwrap();
    }
    game.player = Some(
        horizon_client::motion::PlayerMotion::new(game.motion, Vec3::new(12.5, 201.0, 2.5))
            .unwrap(),
    );
    for command in [
        "forceload add 0 0 31 15",
        "fill 0 200 0 31 200 15 stone",
        "tick freeze",
        "setblock 8 201 0 oak_fence",
    ] {
        assert!(game.run_command(command), "{command}");
    }
    Some(game)
}

/// Runs `command`: whether it succeeded and what the issuer was told.
fn run(game: &mut Game<'_>, command: &str) -> (bool, Vec<String>) {
    game.chat_capture = Some(Vec::new());
    let ok = game.run_command(command);
    let said = game.chat_capture.take().unwrap_or_default();
    (
        ok,
        said.iter().map(horizon_client::text::Text::plain).collect(),
    )
}

/// Summons `kind` at `x` 201 0.5 under `uuid`, held by the fence at 8 201 0
/// when `leash` is set.
fn summon(game: &mut Game<'_>, kind: &str, x: f64, uuid: [i32; 4], leash: bool) {
    let leash = if leash { ",leash:[I;8,201,0]" } else { "" };
    summon_with(game, kind, x, uuid, leash);
}

/// `summon`, with `extra` NBT after the common flags (a hoglin's zombification
/// immunity, say).
fn summon_with(game: &mut Game<'_>, kind: &str, x: f64, uuid: [i32; 4], extra: &str) {
    let command = format!(
        "summon {kind} {x} 201 0.5 {{NoAI:1b,PersistenceRequired:1b,NoGravity:1b,Silent:1b{extra},UUID:[I;{},{},{},{}]}}",
        uuid[0], uuid[1], uuid[2], uuid[3]
    );
    let (ok, said) = run(game, &command);
    assert!(ok, "{command}: {said:?}");
}

/// The index in `mobs` of the mob under `uuid`.
fn mob(game: &Game<'_>, uuid: &str) -> usize {
    let target = game
        .target_with_uuid(horizon_client::save::uuid_from_text(uuid).unwrap())
        .unwrap_or_else(|| panic!("no entity {uuid}"));
    match target {
        super::Target::Mob(id) => game.mobs.iter().position(|m| m.id == id).unwrap(),
        other => panic!("{other:?}"),
    }
}

fn step(game: &mut Game<'_>) {
    assert!(game.run_command("tick step 1"));
    game.tick_server(false).unwrap();
}

#[test]
fn a_selector_matching_two_entities_mounts_nothing() {
    let mut resources = None;
    let Some(mut game) = game(&mut resources) else {
        return;
    };
    summon(&mut game, "minecraft:pig", 4.5, [24730, 0, 0, 1], false);
    summon(&mut game, "minecraft:pig", 6.5, [24730, 0, 0, 2], false);
    // `EntityArgument.parse` refuses a selector whose `getMaxResults` is over
    // one for a single entity argument (`argument.entity.toomany`), before
    // `EntitySelector.findSingleEntity` can see two matches.
    let (ok, said) = run(&mut game, "ride @e[type=minecraft:pig] mount @s");
    assert!(!ok, "two pigs match the target selector");
    // The chat carries vanilla's translated message when the language data is
    // loaded, the key otherwise (another test in the same process may leave it
    // unloaded).
    assert!(
        said.iter().any(|line| {
            line.contains("Only one entity is allowed") || line.contains("argument.entity.toomany")
        }),
        "{said:?}"
    );
    assert!(
        game.mobs.iter().all(|m| m.extra.mount.is_none()),
        "no pig rides"
    );
    // A selector that matches one entity still mounts it: the near pig
    // rides the far one.
    let (ok, said) = run(
        &mut game,
        "ride @e[type=minecraft:pig,limit=1,sort=nearest] mount @e[type=minecraft:pig,limit=1,sort=furthest]",
    );
    assert!(ok, "{said:?}");
    assert_eq!(
        game.mobs.iter().filter(|m| m.extra.mount.is_some()).count(),
        1,
        "one pig rides the other"
    );
}

#[test]
fn a_mounting_mob_drops_its_lead() {
    let mut resources = None;
    let Some(mut game) = game(&mut resources) else {
        return;
    };
    summon(&mut game, "minecraft:pig", 6.5, [24730, 0, 0, 1], true);
    summon(&mut game, "minecraft:cow", 5.5, [24730, 0, 0, 2], false);
    // The lead read from the save becomes the fence's knot.
    step(&mut game);
    let pig = mob(&game, PIG);
    let cow = mob(&game, COW);
    assert_eq!(game.mobs[pig].extra.leash, Some(Holder::Knot([8, 201, 0])));
    let at = game.mobs[pig].position;
    let (ok, said) = run(
        &mut game,
        "ride @e[type=minecraft:pig,limit=1] mount @e[type=minecraft:cow,limit=1]",
    );
    assert!(ok, "{said:?}");
    // Mob.startRiding: the pig rides the cow, and its lead is dropped
    // (`dropLeash()`: the lead falls as an item at the pig's feet).
    assert_eq!(game.mobs[pig].extra.mount, Some(game.mobs[cow].id));
    assert_eq!(game.mobs[pig].extra.leash, None, "the lead went");
    let leads: Vec<_> = game
        .dropped
        .iter()
        .filter(|d| &*d.stack.item == "minecraft:lead")
        .collect();
    assert_eq!(leads.len(), 1, "one lead fell");
    assert_eq!(leads[0].position, at, "at the pig's feet");
}

#[test]
fn a_brain_boarding_mob_drops_its_lead() {
    let mut resources = None;
    let Some(mut game) = game(&mut resources) else {
        return;
    };
    // A hoglin is a monster: Peaceful refuses to summon it.
    assert!(game.run_command("difficulty normal"));
    summon(&mut game, "minecraft:piglin", 6.5, [24730, 0, 0, 3], true);
    summon_with(
        &mut game,
        "minecraft:hoglin",
        5.5,
        [24730, 0, 0, 4],
        ",IsImmuneToZombification:1b",
    );
    // The lead read from the save becomes the fence's knot.
    step(&mut game);
    let piglin = mob(&game, PIGLIN);
    let hoglin = mob(&game, HOGLIN);
    assert_eq!(
        game.mobs[piglin].extra.leash,
        Some(Holder::Knot([8, 201, 0]))
    );
    let at = game.mobs[piglin].position;
    // `PiglinAi.startRiding` queues the mount during the mob's tick; the
    // group loop applies brain events through `mobs::apply_brain_events`,
    // then `tick_mobs` seats the riders (`sync_riders`) before it handles
    // the rest. `Mob.startRiding` drops the lead where the piglin stood,
    // before `positionRider` moves it onto the hoglin.
    let mut events = vec![horizon_client::mobs::MobEvent::StartRiding {
        rider: game.mobs[piglin].id,
        vehicle: game.mobs[hoglin].id,
    }];
    let mut mobs = game.mobs.lift();
    let mut lent = horizon_client::health::LentLevel::take(&mut game.world);
    {
        let mut level = horizon_client::health::DamageLevel::lent(
            &game.world,
            game.pack.assets.registry,
            &mut lent,
            Some(world_items!(game)),
            Some(&game.pack.block_registries),
            &game.damage_hooks,
        );
        horizon_client::mobs::apply_brain_events(&mut mobs, &mut level, &mut events, 0);
    }
    lent.give_back(&mut game.world);
    game.mobs
        .restore(mobs, horizon_client::join_seq::RestoreOrder::SurvivorsFirst);
    game.sync_riders();
    assert_ne!(
        game.mobs[piglin].position, at,
        "the piglin sits on the hoglin"
    );
    game.handle_mob_events(events).unwrap();
    assert_eq!(game.mobs[piglin].extra.mount, Some(game.mobs[hoglin].id));
    assert_eq!(game.mobs[piglin].extra.leash, None, "the lead went");
    let leads: Vec<_> = game
        .dropped
        .iter()
        .filter(|d| &*d.stack.item == "minecraft:lead")
        .collect();
    assert_eq!(leads.len(), 1, "one lead fell");
    assert_eq!(leads[0].position, at, "at the piglin's feet");
}
