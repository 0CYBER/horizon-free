use super::*;
use crate::game::commands::Target;
use horizon_client::motion::PlayerMotion;
fn with_game(test: impl FnOnce(&mut Game<'_>)) {
    crate::game::native_entity_refs::tests::with_game(|game| {
        game.options.game_dir = Some(
            std::path::PathBuf::from(std::env::var_os("HORIZON_HOME").unwrap())
                .join(format!("h161-{:?}", std::thread::current().id())),
        );
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
        game.load_ticking_neighbourhood(0, 0);
        game.creative = true;
        let time = game.world.game_time;
        test(game);
        assert_eq!(game.world.game_time, time);
    });
}

#[test]
#[ignore = "daily"]
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
            .command_target_nbt(Target::Mob(parents[0]))
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
#[ignore = "daily"]
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

            let body = g.command_target_nbt(g.level_entity(root as i32).unwrap()).unwrap();
            eprintln!("h161 loaded parent={parent} body={body:?}");
            assert_eq!(child_uuids(&body), saved, "save {parent}");
            assert!(g.run_command(&format!("data merge entity @e[nbt={{UUID:[I;161,26,3,{parent}]}},limit=1] {{CustomName:\"changed{parent}\",Passengers:[{{id:\"minecraft:pig\",UUID:[I;161,26,3,900]}}]}}")));
            assert!(id(g, 900).is_none());
            assert_eq!(g.ordered_live_passengers(root), children);
            let body = g.command_target_nbt(g.level_entity(root as i32).unwrap()).unwrap();
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
            let body = g.command_target_nbt(g.level_entity(root as i32).unwrap()).unwrap();
            assert_eq!(child_uuids(&body), saved, "reload {parent}");
            for &child in saved {
                assert_eq!(g.passenger_vehicle(id(g, child).unwrap()), Some(root));
            }
        }
    });
}
#[test]
#[ignore = "daily"]
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
#[ignore = "daily"]
fn cold_registry_nonmob_uuid_collision_refuses_the_entire_tree() {
    with_game(|g| {
        let tag = horizon_core::nbt::parse_snbt(
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
#[ignore = "daily"]
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
#[ignore = "daily"]
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
                false,
                crate::game::vehicle_teleport::TeleportMotion::Stop,
            )
            .unwrap()
        );
        g.enter_level(crate::game::Dim::Nether).unwrap();
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
        g.enter_level(crate::game::Dim::Overworld).unwrap();
        assert!(id(g, 910).is_none() && id(g, 911).is_none() && id(g, 912).is_none());
        g.enter_level(crate::game::Dim::Nether).unwrap();
        assert_eq!(g.passenger_vehicle(item), Some(stand));
        g.enter_level(crate::game::Dim::Overworld).unwrap();
    });
}

#[test]
#[ignore = "daily"]
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
                    false,
                    crate::game::vehicle_teleport::TeleportMotion::Stop,
                )
                .unwrap()
            );
            g.enter_level(crate::game::Dim::Nether).unwrap();
            let root = id(g, n).unwrap();
            let child = id(g, n + 1)
                .expect("Java transfers the live noSave rider; a disk-save tree is insufficient");
            assert_eq!(g.passenger_vehicle(child), Some(root));
            assert!(child_uuids(&g.vehicle_tree_nbt(root).unwrap()).is_empty());
            g.enter_level(crate::game::Dim::Overworld).unwrap();
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
                    false,
                    crate::game::vehicle_teleport::TeleportMotion::Stop,
                )
                .unwrap()
            );
            g.enter_level(crate::game::Dim::Nether).unwrap();
            assert!(id(g, n + 2).is_some());
            g.enter_level(crate::game::Dim::Overworld).unwrap();
            assert!(id(g, n + 2).is_none());
        }
    });
}

fn passenger_record(kind: &str, uuid: i32, passengers: Vec<Tag>) -> Tag {
    let mut tag = Tag::compound([
        ("id", Tag::String(format!("minecraft:{kind}").as_str().into())),
        ("UUID", Tag::IntArray(vec![161, 26, 3, uuid])),
        ("identity", Tag::Int(uuid)),
        ("NoAI", Tag::Byte(1)),
        ("NoGravity", Tag::Byte(1)),
        ("PersistenceRequired", Tag::Byte(1)),
        (
            "Pos",
            Tag::List(
                [4.5_f64, 200.0, 4.5]
                    .map(|value| Tag::Double(value.to_bits()))
                    .to_vec(),
            ),
        ),
    ]);
    add_passengers(&mut tag, passengers);
    tag
}

#[test]
#[ignore = "daily"]
fn mixed_riders_keep_load_order_and_offlevel_descendants_through_chunk_reload() {
    with_game(|game| {
        game.spawn_loaded_entity((0, 0), passenger_record("cow", 982, vec![]), None);
        let root = passenger_record(
            "pig",
            980,
            vec![
                passenger_record("armor_stand", 981, vec![]),
                passenger_record(
                    "cow",
                    982,
                    vec![passenger_record("armor_stand", 983, vec![])],
                ),
                passenger_record(
                    "oak_boat",
                    984,
                    vec![passenger_record("cow", 985, vec![])],
                ),
                passenger_record("minecart", 986, vec![]),
            ],
        );
        game.spawn_loaded_entity((0, 0), root, None);
        for round in 0..2 {
            let root = id(game, 980).unwrap();
            let body = game.vehicle_tree_nbt(root).unwrap();
            assert_eq!(child_uuids(&body), [981, 982, 984, 986]);
            let Some(Tag::List(children)) = body.get("Passengers") else {
                panic!("mixed riders must serialize")
            };
            assert_eq!(child_uuids(&children[1]), [983]);
            assert_eq!(child_uuids(&children[2]), [985]);
            let rejected = game.mob_passenger_ids(root)[0];
            assert!(game.persist.off_level.contains_key(&rejected));
            assert_eq!(game.mob_object(rejected).unwrap().extra.mount, Some(root));
            assert_eq!(game.passenger_vehicle(id(game, 983).unwrap()), Some(rejected));
            assert_eq!(game.passenger_tree(root).len(), 7, "no duplicate riders");
            assert_eq!(
                game.ordered_live_passengers(root)
                    .into_iter()
                    .map(|id| game.persist.uuid_of(id).unwrap()[3])
                    .collect::<Vec<_>>(),
                [981, 984, 986],
            );
            if round == 0 {
                let chunks = game.entity_chunks(&HashSet::from([(0, 0)]), true);
                for (key, tags) in chunks {
                    game.world.queue_entity_nbt(key, tags);
                }
                game.spawn_loaded_entities();
            }
        }
    });
}

#[test]
#[ignore = "daily"]
fn mixed_transfer_pairs_runtime_state_with_exact_objects_not_uuid_or_mob_vec_order() {
    use horizon_mod_runtime::attachments::{AttachmentType, Attachments, Holder};

    with_game(|game| {
        let runtime = AttachmentType {
            namespace: "test",
            name: "passenger_identity",
            default: || 0,
            codec: None,
            copy_on_death: false,
            sync: false,
        };
        let mut first = passenger_record("cow", 993, vec![]);
        let mut second = first.clone();
        if let Tag::Compound(fields) = &mut first {
            fields.insert("identity".into(), Tag::Int(9930));
        }
        if let Tag::Compound(fields) = &mut second {
            fields.insert("identity".into(), Tag::Int(9931));
        }
        let root = passenger_record(
            "pig",
            990,
            vec![
                passenger_record(
                    "armor_stand",
                    991,
                    vec![passenger_record("cow", 992, vec![])],
                ),
                first,
                second,
                passenger_record("leash_knot", 994, vec![]),
                passenger_record(
                    "oak_boat",
                    995,
                    vec![passenger_record("armor_stand", 996, vec![])],
                ),
                passenger_record("minecart", 997, vec![]),
            ],
        );
        game.spawn_loaded_entity((0, 0), root, None);
        let root = id(game, 990).unwrap();
        let sources = game.passenger_tree(root);
        assert_eq!(sources.len(), 9);
        for &source in &sources {
            let Some(Tag::Int(identity)) = game.persist.raw[&source].get("identity") else {
                panic!("source object identity")
            };
            let mut map = Attachments::default();
            runtime.set(&mut map, Holder::Entity(source), *identity).unwrap();
            game.entity_attachments.insert(source, map);
        }
        // Resident storage order is deliberately unrelated to the saved graph.
        let count = game.mobs.len();
        for index in 0..count / 2 {
            game.mobs.swap(index, count - 1 - index);
        }
        assert!(
            game.teleport_vehicle_across(
                Target::Mob(root),
                crate::game::Dim::Nether,
                [8.5, 200.0, 8.5],
                None,
                false,
                crate::game::vehicle_teleport::TeleportMotion::Stop,
            )
            .unwrap()
        );
        game.enter_level(crate::game::Dim::Nether).unwrap();
        let root = id(game, 990).unwrap();
        let restored = game.passenger_tree(root);
        assert_eq!(restored.len(), sources.len(), "every live object transfers once");
        let identities = restored
            .iter()
            .map(|id| {
                let Some(Tag::Int(identity)) = game.persist.raw[id].get("identity") else {
                    panic!("restored object identity")
                };
                let map = game.entity_attachments.get_mut(id).unwrap();
                assert_eq!(runtime.get(map, Holder::Entity(*id)), Ok(*identity));
                *identity
            })
            .collect::<HashSet<_>>();
        assert_eq!(identities, HashSet::from([990, 991, 992, 9930, 9931, 994, 995, 996, 997]));
        let children = game.ordered_passenger_slots(root)
            .into_iter()
            .map(|slot| match slot {
                PassengerSlot::Live(id) => game.persist.uuid_of(id).unwrap()[3],
                PassengerSlot::Saved(_) => panic!("every transferred rider is an object"),
            })
            .collect::<Vec<_>>();
        assert_eq!(children, [991, 993, 993, 994, 995, 997]);
        assert_eq!(
            game.ordered_passenger_slots(root)
                .into_iter()
                .map(|slot| match slot {
                    PassengerSlot::Live(id) => match game.persist.raw[&id].get("identity") {
                        Some(Tag::Int(identity)) => *identity,
                        _ => panic!("restored rider identity"),
                    },
                    PassengerSlot::Saved(_) => panic!("every transferred rider is an object"),
                })
                .collect::<Vec<_>>(),
            [991, 9930, 9931, 994, 995, 997],
            "duplicate-UUID objects retain their distinct positions in the live graph",
        );
        assert_eq!(
            child_uuids(&game.vehicle_tree_nbt(root).unwrap()),
            [991, 993, 993, 995, 997],
            "noSave riders transfer but remain excluded from disk saves",
        );
        assert_eq!(
            restored
                .iter()
                .filter(|id| game.persist.off_level.contains_key(*id))
                .count(),
            1,
            "duplicate UUIDs retain distinct rider objects, not duplicate residents",
        );
        game.enter_level(crate::game::Dim::Overworld).unwrap();
        assert!(sources.into_iter().all(|id| !game.passenger_object_present(id)));
    });
}

#[test]
#[ignore = "daily"]
fn dismount_and_reboard_appends_a_loaded_mob_after_existing_mixed_riders() {
    with_game(|game| {
        let root = passenger_record(
            "pig",
            1000,
            vec![
                passenger_record("cow", 1001, vec![]),
                passenger_record("armor_stand", 1002, vec![]),
                passenger_record("cow", 1003, vec![]),
            ],
        );
        game.spawn_loaded_entity((0, 0), root, None);
        let root = id(game, 1000).unwrap();
        let rider = id(game, 1001).unwrap();
        assert_eq!(
            child_uuids(&game.vehicle_tree_nbt(root).unwrap()),
            [1001, 1002, 1003],
        );
        game.dismount_mob(rider);
        game.ride_mount(Target::Mob(rider), Target::Mob(root)).unwrap();
        assert_eq!(
            child_uuids(&game.vehicle_tree_nbt(root).unwrap()),
            [1002, 1003, 1001],
            "boarding again appends the rider instead of reviving its old slot",
        );
        assert_eq!(
            game.ordered_live_passengers(root),
            [id(game, 1002).unwrap(), id(game, 1003).unwrap(), rider],
        );
    });
}

#[test]
#[ignore = "daily"]
fn saved_nonmob_slot_takes_destination_admission_without_moving_its_uuid_owner() {
    with_game(|game| {
        game.spawn_loaded_entity((0, 0), passenger_record("armor_stand", 1011, vec![]), None);
        let original = id(game, 1011).unwrap();
        let root = passenger_record(
            "pig",
            1010,
            vec![passenger_record(
                "armor_stand",
                1011,
                vec![passenger_record("cow", 1012, vec![])],
            )],
        );
        game.spawn_loaded_entity((0, 0), root, None);
        let root = id(game, 1010).unwrap();
        assert!(game.ordered_live_passengers(root).is_empty());
        assert!(id(game, 1012).is_none());
        assert!(
            game.teleport_vehicle_across(
                Target::Mob(root),
                crate::game::Dim::Nether,
                [8.5, 200.0, 8.5],
                None,
                false,
                crate::game::vehicle_teleport::TeleportMotion::Stop,
            )
            .unwrap()
        );
        game.enter_level(crate::game::Dim::Nether).unwrap();
        let root = id(game, 1010).unwrap();
        let stand = id(game, 1011).unwrap();
        let cow = id(game, 1012).unwrap();
        assert_ne!(stand, original);
        assert_eq!(game.passenger_vehicle(stand), Some(root));
        assert_eq!(game.passenger_vehicle(cow), Some(stand));
        assert_eq!(game.passenger_tree(root).len(), 3);
        game.enter_level(crate::game::Dim::Overworld).unwrap();
        assert_eq!(id(game, 1011), Some(original));
        assert!(id(game, 1010).is_none() && id(game, 1012).is_none());
    });
}

#[path = "completion_tests/generic_seating_tests.rs"]
mod generic_seating_tests;
