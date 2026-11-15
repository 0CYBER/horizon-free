use super::*;
use crate::game::{Audio, Options, Progress, Settings, commands::Target, load_resources};
use horizon_client::motion::PlayerMotion;
fn with_game(test: impl FnOnce(&mut Game<'_>)) {
    let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let settings = Settings::default();
    let loaded = load_resources(
        root.clone(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        true,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.game_dir = Some(
        std::path::PathBuf::from(std::env::var_os("HORIZON_HOME").unwrap())
            .join(format!("h161-{:?}", std::thread::current().id())),
    );
    options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
    let mut game = Game::new(
        &loaded.resources,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&root),
        None,
    )
    .unwrap();
    game.build_registries();
    game.set_allow_commands(true);
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    assert!(game.run_command("forceload add 0 0"));
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(2.5, 200.0, 2.5)).unwrap());
    game.creative = true;
    let time = game.world.game_time;
    test(&mut game);
    assert_eq!(game.world.game_time, time);
}

#[test]
#[ignore = "needs prepared resources; full H161 completion baseline"]
fn valid_nonmob_rider_is_live_and_merge_does_not_adopt_passengers() {
    with_game(|game| {
        assert!(game.run_command("summon minecraft:pig 4.5 200 4.5 {UUID:[I;161,26,3,100],NoAI:1b,NoGravity:1b,PersistenceRequired:1b,Passengers:[{id:\"minecraft:armor_stand\",UUID:[I;161,26,3,200],NoGravity:1b}]}"));
        let parents: Vec<_> = game
            .mobs
            .iter()
            .filter(|m| game.persist.uuid_of(m.id) == Some([161, 26, 3, 100]))
            .map(|m| m.id)
            .collect();
        assert_eq!(parents.len(), 1, "one actually loaded parent");
        let riders: Vec<_> = game
            .armor_stands
            .iter()
            .filter(|a| game.persist.uuid_of(a.id) == Some([161, 26, 3, 200]))
            .map(|a| a.id)
            .collect();
        assert_eq!(
            riders.len(),
            1,
            "Java admits one live armor stand, not only a saved raw body"
        );
        game.run_command("scoreboard objectives add h161base dummy");
        game.run_command("scoreboard players set #r h161base -99");
        game.run_command("scoreboard players set #s h161base -99");
        game.run_command("execute store result score #r h161base store success score #s h161base as @e[nbt={UUID:[I;161,26,3,200]},limit=1] on vehicle if entity @s[nbt={UUID:[I;161,26,3,100]}]");
        assert_eq!(game.score.board.score("#r", "h161base").unwrap().value, 1);
        assert_eq!(game.score.board.score("#s", "h161base").unwrap().value, 1);
        assert!(game.run_command("data merge entity @e[nbt={UUID:[I;161,26,3,100]},limit=1] {CustomName:\"changed\",Passengers:[{id:\"minecraft:pig\",UUID:[I;161,26,3,900]}]}"));
        assert!(
            !game.persist.uuids.values().any(|v| *v == [161, 26, 3, 900]),
            "merge does not create input children"
        );
        game.run_command("scoreboard objectives add h161base dummy");
        game.run_command("scoreboard players set #r h161base -99");
        game.run_command("scoreboard players set #s h161base -99");
        game.run_command("execute store result score #r h161base store success score #s h161base as @e[nbt={UUID:[I;161,26,3,200]},limit=1] on vehicle if entity @s[nbt={UUID:[I;161,26,3,100]}]");
        assert_eq!(game.score.board.score("#r", "h161base").unwrap().value, 1);
        assert_eq!(game.score.board.score("#s", "h161base").unwrap().value, 1);
        let saved = game
            .entity_nbt(Target::Mob(parents[0]))
            .expect("real live parent saves");
        let Some(Tag::List(children)) = saved.get("Passengers") else {
            panic!("live rider must save")
        };
        assert_eq!(children.len(), 1);
        assert_eq!(entity_nbt::uuid(&children[0]), Some([161, 26, 3, 200]));
        assert_eq!(
            entity_nbt::kind(&children[0]).as_deref(),
            Some("minecraft:armor_stand")
        );
    });
}

fn id(game: &Game<'_>, n: i32) -> Option<u32> {
    game.persist
        .uuids
        .iter()
        .find_map(|(&id, &u)| (u == [161, 26, 3, n] && game.passenger_present(id)).then_some(id))
}
fn child_uuids(tag: &Tag) -> Vec<i32> {
    match tag.get("Passengers") {
        Some(Tag::List(v)) => v.iter().map(|t| entity_nbt::uuid(t).unwrap()[3]).collect(),
        _ => Vec::new(),
    }
}
#[test]
#[ignore = "needs prepared resources; pinned Java admitted typed passenger families"]
fn admitted_families_load_merge_save_and_reload() {
    with_game(|g| {
        let cases: &[(&str, i32, &[i32], &[i32])] = &[
            (
                r#"summon minecraft:pig 4.5 200 4.5 {id:"minecraft:pig",UUID:[I;161,26,3,100],Tags:["h161family0"],NoGravity:1b,Invulnerable:1b,Silent:1b,NoAI:1b,PersistenceRequired:1b,Passengers:[{id:"minecraft:armor_stand",UUID:[I;161,26,3,200],Tags:["h161family0_c0"],NoGravity:1b,Invulnerable:1b,Silent:1b}]}"#,
                100,
                &[200],
                &[200],
            ),
            (
                r#"summon minecraft:pig 4.5 200 4.5 {id:"minecraft:pig",UUID:[I;161,26,3,101],Tags:["h161family1"],NoGravity:1b,Invulnerable:1b,Silent:1b,NoAI:1b,PersistenceRequired:1b,Passengers:[{id:"minecraft:item",UUID:[I;161,26,3,201],Tags:["h161family1_c0"],NoGravity:1b,Invulnerable:1b,Silent:1b,Item:{id:"minecraft:stone",count:2}}]}"#,
                101,
                &[201],
                &[201],
            ),
            (
                r#"summon minecraft:pig 4.5 200 4.5 {id:"minecraft:pig",UUID:[I;161,26,3,102],Tags:["h161family2"],NoGravity:1b,Invulnerable:1b,Silent:1b,NoAI:1b,PersistenceRequired:1b,Passengers:[{id:"minecraft:marker",UUID:[I;161,26,3,202],Tags:["h161family2_c0"],NoGravity:1b,Invulnerable:1b,Silent:1b}]}"#,
                102,
                &[202],
                &[202],
            ),
            (
                r#"summon minecraft:pig 4.5 200 4.5 {id:"minecraft:pig",UUID:[I;161,26,3,103],Tags:["h161family3"],NoGravity:1b,Invulnerable:1b,Silent:1b,NoAI:1b,PersistenceRequired:1b,Passengers:[{id:"minecraft:text_display",UUID:[I;161,26,3,203],Tags:["h161family3_c0"],NoGravity:1b,Invulnerable:1b,Silent:1b,text:"rider"}]}"#,
                103,
                &[203],
                &[203],
            ),
            (
                r#"summon minecraft:pig 4.5 200 4.5 {id:"minecraft:pig",UUID:[I;161,26,3,104],Tags:["h161family4"],NoGravity:1b,Invulnerable:1b,Silent:1b,NoAI:1b,PersistenceRequired:1b,Passengers:[{id:"minecraft:oak_boat",UUID:[I;161,26,3,204],Tags:["h161family4_c0"],NoGravity:1b,Invulnerable:1b,Silent:1b}]}"#,
                104,
                &[204],
                &[204],
            ),
            (
                r#"summon minecraft:pig 4.5 200 4.5 {id:"minecraft:pig",UUID:[I;161,26,3,105],Tags:["h161family5"],NoGravity:1b,Invulnerable:1b,Silent:1b,NoAI:1b,PersistenceRequired:1b,Passengers:[{id:"minecraft:minecart",UUID:[I;161,26,3,205],Tags:["h161family5_c0"],NoGravity:1b,Invulnerable:1b,Silent:1b}]}"#,
                105,
                &[205],
                &[205],
            ),
            (
                r#"summon minecraft:pig 4.5 200 4.5 {id:"minecraft:pig",UUID:[I;161,26,3,106],Tags:["h161family6"],NoGravity:1b,Invulnerable:1b,Silent:1b,NoAI:1b,PersistenceRequired:1b,Passengers:[{id:"minecraft:experience_orb",UUID:[I;161,26,3,206],Tags:["h161family6_c0"],NoGravity:1b,Invulnerable:1b,Silent:1b,Value:3}]}"#,
                106,
                &[206],
                &[206],
            ),
            (
                r#"summon minecraft:pig 4.5 200 4.5 {id:"minecraft:pig",UUID:[I;161,26,3,107],Tags:["h161family7"],NoGravity:1b,Invulnerable:1b,Silent:1b,NoAI:1b,PersistenceRequired:1b,Passengers:[{id:"minecraft:arrow",UUID:[I;161,26,3,207],Tags:["h161family7_c0"],NoGravity:1b,Invulnerable:1b,Silent:1b}]}"#,
                107,
                &[207],
                &[207],
            ),
            (
                r#"summon minecraft:pig 4.5 200 4.5 {id:"minecraft:pig",UUID:[I;161,26,3,108],Tags:["h161family8"],NoGravity:1b,Invulnerable:1b,Silent:1b,NoAI:1b,PersistenceRequired:1b,Passengers:[{id:"minecraft:tnt",UUID:[I;161,26,3,208],Tags:["h161family8_c0"],NoGravity:1b,Invulnerable:1b,Silent:1b,fuse:200s}]}"#,
                108,
                &[208],
                &[208],
            ),
            (
                r#"summon minecraft:pig 4.5 200 4.5 {id:"minecraft:pig",UUID:[I;161,26,3,109],Tags:["h161family9"],NoGravity:1b,Invulnerable:1b,Silent:1b,NoAI:1b,PersistenceRequired:1b,Passengers:[{id:"minecraft:falling_block",UUID:[I;161,26,3,209],Tags:["h161family9_c0"],NoGravity:1b,Invulnerable:1b,Silent:1b,BlockState:{Name:"minecraft:stone"},Time:1}]}"#,
                109,
                &[209],
                &[209],
            ),
            (
                r#"summon minecraft:pig 4.5 200 4.5 {id:"minecraft:pig",UUID:[I;161,26,3,110],Tags:["h161family10"],NoGravity:1b,Invulnerable:1b,Silent:1b,NoAI:1b,PersistenceRequired:1b,Passengers:[{id:"minecraft:item_frame",UUID:[I;161,26,3,210],Tags:["h161family10_c0"],NoGravity:1b,Invulnerable:1b,Silent:1b,Facing:2b,Fixed:1b}]}"#,
                110,
                &[210],
                &[210],
            ),
            (
                r#"summon minecraft:pig 4.5 200 4.5 {id:"minecraft:pig",UUID:[I;161,26,3,111],Tags:["h161family11"],NoGravity:1b,Invulnerable:1b,Silent:1b,NoAI:1b,PersistenceRequired:1b,Passengers:[{id:"minecraft:painting",UUID:[I;161,26,3,211],Tags:["h161family11_c0"],NoGravity:1b,Invulnerable:1b,Silent:1b,Facing:2b,variant:"minecraft:kebab"}]}"#,
                111,
                &[211],
                &[211],
            ),
            (
                r#"summon minecraft:armor_stand 4.5 200 4.5 {id:"minecraft:armor_stand",UUID:[I;161,26,3,300],Tags:["h161root0"],NoGravity:1b,Invulnerable:1b,Silent:1b,Passengers:[{id:"minecraft:armor_stand",UUID:[I;161,26,3,400],Tags:["h161root0_c0"],NoGravity:1b,Invulnerable:1b,Silent:1b},{id:"minecraft:pig",UUID:[I;161,26,3,500],Tags:["h161root0_c1"],NoGravity:1b,Invulnerable:1b,Silent:1b,NoAI:1b,PersistenceRequired:1b}]}"#,
                300,
                &[400, 500],
                &[400, 500],
            ),
            (
                r#"summon minecraft:marker 4.5 200 4.5 {id:"minecraft:marker",UUID:[I;161,26,3,301],Tags:["h161root1"],NoGravity:1b,Invulnerable:1b,Silent:1b,Passengers:[{id:"minecraft:armor_stand",UUID:[I;161,26,3,401],Tags:["h161root1_c0"],NoGravity:1b,Invulnerable:1b,Silent:1b},{id:"minecraft:pig",UUID:[I;161,26,3,501],Tags:["h161root1_c1"],NoGravity:1b,Invulnerable:1b,Silent:1b,NoAI:1b,PersistenceRequired:1b}]}"#,
                301,
                &[],
                &[],
            ),
            (
                r#"summon minecraft:text_display 4.5 200 4.5 {id:"minecraft:text_display",UUID:[I;161,26,3,302],Tags:["h161root2"],NoGravity:1b,Invulnerable:1b,Silent:1b,Passengers:[{id:"minecraft:armor_stand",UUID:[I;161,26,3,402],Tags:["h161root2_c0"],NoGravity:1b,Invulnerable:1b,Silent:1b},{id:"minecraft:pig",UUID:[I;161,26,3,502],Tags:["h161root2_c1"],NoGravity:1b,Invulnerable:1b,Silent:1b,NoAI:1b,PersistenceRequired:1b}]}"#,
                302,
                &[402, 502],
                &[402, 502],
            ),
            (
                r#"summon minecraft:item 4.5 200 4.5 {id:"minecraft:item",UUID:[I;161,26,3,303],Tags:["h161root3"],NoGravity:1b,Invulnerable:1b,Silent:1b,Item:{id:"minecraft:stone",count:1},Passengers:[{id:"minecraft:armor_stand",UUID:[I;161,26,3,403],Tags:["h161root3_c0"],NoGravity:1b,Invulnerable:1b,Silent:1b},{id:"minecraft:pig",UUID:[I;161,26,3,503],Tags:["h161root3_c1"],NoGravity:1b,Invulnerable:1b,Silent:1b,NoAI:1b,PersistenceRequired:1b}]}"#,
                303,
                &[403, 503],
                &[403, 503],
            ),
            (
                r#"summon minecraft:pig 4.5 200 4.5 {id:"minecraft:pig",UUID:[I;161,26,3,600],Tags:["h161null"],NoGravity:1b,Invulnerable:1b,Silent:1b,NoAI:1b,PersistenceRequired:1b,Passengers:[{id:"minecraft:player",UUID:[I;161,26,3,601],Tags:["h161null_c0"],NoGravity:1b,Invulnerable:1b,Silent:1b},{id:"minecraft:fishing_bobber",UUID:[I;161,26,3,602],Tags:["h161null_c1"],NoGravity:1b,Invulnerable:1b,Silent:1b},{id:"minecraft:armor_stand",UUID:[I;161,26,3,603],Tags:["h161null_c2"],NoGravity:1b,Invulnerable:1b,Silent:1b}]}"#,
                600,
                &[602, 603],
                &[603],
            ),
        ];
        for &(command, parent, live, saved) in cases {
            assert!(g.run_command(command), "{command}");
            let root = id(g, parent).expect("actual root");
            let children = g.ordered_live_passengers(root);
            assert_eq!(
                children
                    .iter()
                    .map(|id| g.persist.uuid_of(*id).unwrap()[3])
                    .collect::<Vec<_>>(),
                live,
                "live {parent}"
            );
            for &child in &children {
                assert_eq!(g.passenger_vehicle(child), Some(root));
            }
            let mut tracking = horizon_client::lan::entity::Riding::default();
            g.lan_riding(&[], &mut tracking);
            assert_eq!(
                tracking
                    .passengers
                    .get(&(root as i32))
                    .cloned()
                    .unwrap_or_default(),
                children.iter().map(|id| *id as i32).collect::<Vec<_>>()
            );

            let body = g.entity_nbt(g.level_entity(root as i32).unwrap()).unwrap();
            eprintln!("h161 loaded parent={parent} body={body:?}");
            assert_eq!(child_uuids(&body), saved, "save {parent}");
            assert!(g.run_command(&format!("data merge entity @e[nbt={{UUID:[I;161,26,3,{parent}]}},limit=1] {{CustomName:\"changed{parent}\",Passengers:[{{id:\"minecraft:pig\",UUID:[I;161,26,3,900]}}]}}")));
            assert!(id(g, 900).is_none());
            assert_eq!(g.ordered_live_passengers(root), children);
            let body = g.entity_nbt(g.level_entity(root as i32).unwrap()).unwrap();
            assert_eq!(child_uuids(&body), saved);
        }
        let keys = HashSet::from([(0, 0)]);
        let chunks = g.entity_chunks(&keys, true);
        eprintln!("h161 actual chunk payload={chunks:?}");
        g.world.loaded_entities.extend(
            chunks
                .into_iter()
                .map(|(key, tags)| (key, tags.into_iter().map(Into::into).collect())),
        );
        g.spawn_loaded_entities();
        for &(_, parent, _, saved) in cases {
            let root = id(g, parent).expect("reloaded root");
            let body = g.entity_nbt(g.level_entity(root as i32).unwrap()).unwrap();
            assert_eq!(child_uuids(&body), saved, "reload {parent}");
            for &child in saved {
                assert_eq!(g.passenger_vehicle(id(g, child).unwrap()), Some(root));
            }
        }
    });
}
#[test]
#[ignore = "needs prepared resources; pinned Java nested and lifecycle contract"]
fn nested_nonmob_tree_dismount_remove_and_root_vehicle() {
    with_game(|g| {
        assert!(g.run_command(r#"summon pig 4.5 200 4.5 {UUID:[I;161,26,3,700],NoAI:1b,NoGravity:1b,Passengers:[{id:"minecraft:armor_stand",UUID:[I;161,26,3,701],NoGravity:1b,Passengers:[{id:"minecraft:item",UUID:[I;161,26,3,702],Item:{id:"minecraft:stone",count:2}}]},{id:"missing:entity"},{id:"minecraft:pig",UUID:[I;161,26,3,703],NoAI:1b}]}"#));
        let mut root = id(g, 700).unwrap();
        let mut stand = id(g, 701).unwrap();
        let mut item = id(g, 702).unwrap();
        assert_eq!(g.passenger_vehicle(item), Some(stand));
        let body = g.vehicle_tree_nbt(root).unwrap();
        assert_eq!(child_uuids(&body), vec![701, 703]);
        assert_eq!(
            child_uuids({
                let Some(Tag::List(children)) = body.get("Passengers") else {
                    panic!("children")
                };
                &children[0]
            }),
            vec![702]
        );
        let (wrapper, parent, attach) = g.parent_vehicle_tag(Some(stand), &[]).unwrap();
        assert_eq!(parent, root);
        assert_eq!(attach, [161, 26, 3, 701]);
        assert_eq!(wrapper.get("Entity"), Some(&body));
        // Pinned EntityType.PIG passenger attachment, Entity's default zero
        // rider attachment, and ArmorStand's 1.975F height. This exercises the
        // seating phase directly without pretending a non-ticking chunk ticked.
        g.sync_riders();
        let seat = 200.0 + f64::from(0.86875_f32);
        assert_eq!(
            g.target_position(Target::Stand(stand)).unwrap(),
            [4.5, seat, 4.5]
        );
        assert_eq!(
            g.target_position(Target::Item(item)).unwrap(),
            [4.5, seat + f64::from(1.975_f32), 4.5]
        );
        g.remove_vehicle_tree(root);
        assert!(id(g, 700).is_none() && id(g, 701).is_none() && id(g, 702).is_none());
        let saved_player = Tag::compound([("RootVehicle", wrapper)]);
        stand = g
            .restore_saved_vehicle([161, 26, 3, 701], Some(&saved_player))
            .unwrap();
        root = id(g, 700).unwrap();
        item = id(g, 702).unwrap();
        assert_eq!(g.passenger_vehicle(stand), Some(root));
        assert_eq!(g.passenger_vehicle(item), Some(stand));
        assert!(g.run_command("ride @e[nbt={UUID:[I;161,26,3,702]},limit=1] dismount"));
        assert_eq!(g.passenger_vehicle(item), None);
        assert!(g.run_command("kill @e[nbt={UUID:[I;161,26,3,701]},limit=1]"));
        assert!(!g.passenger_present(stand));
        assert!(g.passenger_present(item));
        assert_eq!(child_uuids(&g.vehicle_tree_nbt(root).unwrap()), vec![703]);
        g.remove_vehicle_tree(root);
        assert!(!g.passenger_present(root));
        assert!(g.passenger_present(item));
    });
}

#[test]
#[ignore = "needs prepared resources; first typed-tree UUID preflight uses loaded registry"]
fn cold_registry_nonmob_uuid_collision_refuses_the_entire_tree() {
    with_game(|g| {
        let tag = horizon_world::nbt::parse_snbt(
            r#"{id:"minecraft:armor_stand",UUID:[I;161,26,3,800],Pos:[4.5d,200d,4.5d]}"#,
        )
        .unwrap();
        assert!(!g.entity_types_loaded(), "cold registry precondition");
        g.load_listed(
            "minecraft:armor_stand",
            Some([161, 26, 3, 800]),
            None,
            tag.0,
        )
        .unwrap();
        let before = id(g, 800).unwrap();
        assert!(!g.run_command(r#"summon pig 4.5 200 4.5 {UUID:[I;161,26,3,801],Passengers:[{id:"minecraft:armor_stand",UUID:[I;161,26,3,800]}]}"#));
        assert!(id(g, 801).is_none());
        assert_eq!(id(g, 800), Some(before));
    });
}

#[test]
#[ignore = "needs prepared resources; admitted remaining noSave type policies"]
fn registered_nosave_riders_are_live_but_not_serialized_or_vehicles() {
    with_game(|g| {
        for (i, kind) in ["leash_knot", "lightning_bolt"].iter().enumerate() {
            let n = 850 + i as i32 * 10;
            assert!(g.run_command(&format!("summon pig 4.5 200 4.5 {{UUID:[I;161,26,3,{n}],NoAI:1b,NoGravity:1b,Passengers:[{{id:\"minecraft:{kind}\",UUID:[I;161,26,3,{}]}}]}}",n+1)));
            let parent = id(g, n).unwrap();
            let child = id(g, n + 1).unwrap();
            assert_eq!(g.passenger_vehicle(child), Some(parent));
            assert!(child_uuids(&g.vehicle_tree_nbt(parent).unwrap()).is_empty());
            assert!(g.run_command(&format!("summon {kind} 7 200 7 {{UUID:[I;161,26,3,{}],Passengers:[{{id:\"minecraft:pig\",UUID:[I;161,26,3,{}]}}]}}",n+2,n+3)));
            assert!(id(g, n + 2).is_some());
            assert!(id(g, n + 3).is_none());
        }
        let chunks = g.entity_chunks(&HashSet::from([(0, 0)]), true);
        g.world.loaded_entities.extend(
            chunks
                .into_iter()
                .map(|(key, tags)| (key, tags.into_iter().map(Into::into).collect())),
        );
        g.spawn_loaded_entities();
        for n in [850, 860] {
            assert!(id(g, n).is_some());
            for offset in [1, 2, 3] {
                assert!(id(g, n + offset).is_none());
            }
        }
    });
}

#[test]
#[ignore = "needs prepared resources; shared nested graph across level carriers"]
fn nested_tree_transfer_keeps_uuid_order_and_clears_source_level() {
    with_game(|g| {
        assert!(g.run_command(r#"summon pig 4.5 200 4.5 {UUID:[I;161,26,3,910],NoAI:1b,NoGravity:1b,Passengers:[{id:"minecraft:armor_stand",UUID:[I;161,26,3,911],NoGravity:1b,Passengers:[{id:"minecraft:item",UUID:[I;161,26,3,912],NoGravity:1b,Item:{id:"minecraft:stone",count:2}}]}]}"#));
        let root = id(g, 910).unwrap();
        assert!(
            g.teleport_vehicle_across(
                Target::Mob(root),
                crate::game::Dim::Nether,
                [8.5, 200.0, 8.5],
                None,
                false
            )
            .unwrap()
        );
        g.enter_level(None, crate::game::Dim::Nether).unwrap();
        let (root, stand, item) = (
            id(g, 910).unwrap(),
            id(g, 911).unwrap(),
            id(g, 912).unwrap(),
        );
        assert_eq!(g.passenger_vehicle(stand), Some(root));
        assert_eq!(g.passenger_vehicle(item), Some(stand));
        assert_eq!(child_uuids(&g.vehicle_tree_nbt(root).unwrap()), vec![911]);
        for target in [Target::Mob(root), Target::Stand(stand), Target::Item(item)] {
            assert_eq!(g.target_position(target).unwrap(), [8.5, 200.0, 8.5]);
        }
        g.enter_level(None, crate::game::Dim::Overworld).unwrap();
        assert!(id(g, 910).is_none() && id(g, 911).is_none() && id(g, 912).is_none());
        g.enter_level(None, crate::game::Dim::Nether).unwrap();
        assert_eq!(g.passenger_vehicle(item), Some(stand));
        g.enter_level(None, crate::game::Dim::Overworld).unwrap();
    });
}

#[test]
#[ignore = "needs prepared resources; level-bounded Java noSave transfer contract"]
fn nosave_riders_transfer_live_without_becoming_saved() {
    with_game(|g| {
        for (i, kind) in ["leash_knot", "lightning_bolt"].iter().enumerate() {
            let n = 940 + i as i32 * 10;
            assert!(g.run_command(&format!("summon pig 4.5 200 4.5 {{UUID:[I;161,26,3,{n}],NoAI:1b,NoGravity:1b,Passengers:[{{id:\"minecraft:{kind}\",UUID:[I;161,26,3,{}]}}]}}",n+1)));
            let root = id(g, n).unwrap();
            let child = id(g, n + 1).unwrap();
            assert_eq!(g.passenger_vehicle(child), Some(root));
            assert!(
                g.teleport_vehicle_across(
                    Target::Mob(root),
                    crate::game::Dim::Nether,
                    [8.5, 200.0, 8.5],
                    None,
                    false
                )
                .unwrap()
            );
            g.enter_level(None, crate::game::Dim::Nether).unwrap();
            let root = id(g, n).unwrap();
            let child = id(g, n + 1)
                .expect("Java transfers the live noSave rider; a disk-save tree is insufficient");
            assert_eq!(g.passenger_vehicle(child), Some(root));
            assert!(child_uuids(&g.vehicle_tree_nbt(root).unwrap()).is_empty());
            g.enter_level(None, crate::game::Dim::Overworld).unwrap();
            assert!(id(g, n).is_none() && id(g, n + 1).is_none());
            assert!(g.run_command(&format!(
                "summon {kind} 6.5 200 6.5 {{UUID:[I;161,26,3,{}]}}",
                n + 2
            )));
            let root = id(g, n + 2).unwrap();
            let target = g.level_entity(root as i32).unwrap();
            assert!(
                g.teleport_vehicle_across(
                    target,
                    crate::game::Dim::Nether,
                    [9.5, 200.0, 9.5],
                    None,
                    false
                )
                .unwrap()
            );
            g.enter_level(None, crate::game::Dim::Nether).unwrap();
            assert!(id(g, n + 2).is_some());
            g.enter_level(None, crate::game::Dim::Overworld).unwrap();
            assert!(id(g, n + 2).is_none());
        }
    });
}

#[path = "completion_tests/generic_seating_tests.rs"]
mod generic_seating_tests;
