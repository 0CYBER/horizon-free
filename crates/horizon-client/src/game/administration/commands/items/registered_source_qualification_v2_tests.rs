#[cfg(test)]
mod registered_source_completion_qualification_v2 {
    use super::*;
    #[test]
    #[ignore = "daily"]
    fn registered_sources_reach_live_read_write_from_and_executor_context_v2() {
        fn copy_tree(from: &std::path::Path, to: &std::path::Path) {
            std::fs::create_dir_all(to).unwrap();
            for entry in std::fs::read_dir(from).unwrap() {
                let entry = entry.unwrap();
                let path = entry.path();
                let dst = to.join(entry.file_name());
                if path.is_dir() {
                    copy_tree(&path, &dst)
                } else {
                    std::fs::copy(path, dst).unwrap();
                }
            }
        }
        let mut game = crate::game::native_machines::tests::server_game();
        // The fixture lives in the repository: tools/fixtures/registered_slot_sources.
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tools/fixtures/registered_slot_sources");
        let root = std::env::temp_dir().join(format!(
            "complete-source-world-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let pack = fixture.join("pack");
        copy_tree(&pack, &root.join("datapacks/complete"));
        game.world_root = root;
        game.build_registries();
        game.set_allow_commands(true);
        assert!(game.run_command("reload"));
        assert!(
            game.ui
                .registries
                .datapacks_enabled
                .contains(&"file/complete".to_owned())
        );
        let declared: serde_json::Value = serde_json::from_slice(
            &std::fs::read(pack.join("data/complete/slot_source/first_three.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            game.fns.store.slot_sources.get("complete:first_three"),
            Some(&declared)
        );
        assert!(
            game.ui.registries.by_registry["minecraft:slot_source"]
                .ids
                .iter()
                .any(|id| id == "complete:first_three")
        );
        assert!(
            game.fns.store.errors.is_empty(),
            "{:?}",
            game.fns.store.errors
        );
        game.world.chunks.remove(&(0, 0));
        game.streaming.paused = false;
        assert!(game.run_command("forceload add 0 0 15 15"));
        game.update_chunk_tickets();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        while !game.streaming.is_lit(0, 0) || !game.world.chunks.contains_key(&(0, 0)) {
            let _ = game.streaming.update(&mut game.world);
            game.spawn_loaded_entities();
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        game.spawn_loaded_entities();
        game.update_chunk_tickets();
        for p in [[0, 100, 0], [4, 100, 0]] {
            assert!(game.run_command(&format!("execute if loaded {} {} {}", p[0], p[1], p[2])));
        }
        assert!(game.run_command(r#"setblock 0 100 0 minecraft:chest[facing=north,type=single,waterlogged=false]{Items:[{Slot:0b,id:"minecraft:stone",count:5},{Slot:1b,id:"minecraft:dirt",count:2},{Slot:3b,id:"minecraft:stone",count:7},{Slot:4b,id:"minecraft:shulker_box",count:1}]}"#));
        assert!(game.run_command(
            "setblock 4 100 0 minecraft:chest[facing=north,type=single,waterlogged=false]"
        ));
        assert!(game.run_command(r#"summon minecraft:armor_stand 8 100 0 {Tags:["complete_executor"],UUID:[I;21,22,23,24],NoGravity:1b,Invisible:1b}"#));
        assert!(
            game.run_command(
                "execute if entity @e[tag=complete_executor,type=minecraft:armor_stand]"
            )
        );
        assert!(game.run_command(
            "execute if data entity @e[tag=complete_executor,limit=1] {UUID:[I;21,22,23,24]}"
        ));
        assert!(game.run_command("item replace entity @e[tag=complete_executor,limit=1] weapon.mainhand with minecraft:stone 11"));
        for (slot, item, count) in [
            (0, "minecraft:stone", 5),
            (1, "minecraft:dirt", 2),
            (3, "minecraft:stone", 7),
            (4, "minecraft:shulker_box", 1),
        ] {
            let actual = game.slot_get(&Slot::Block([0, 100, 0], slot)).unwrap();
            assert_eq!((&*actual.item, actual.count), (item, count));
        }
        let source = game.block_entity_nbt([0, 100, 0]).unwrap();
        let clock = game.world.game_time;
        assert!(game.run_command("scoreboard objectives add fullslot dummy"));
        let stages: serde_json::Value = serde_json::from_slice(
            &std::fs::read(fixture.join("stages.json")).unwrap(),
        )
        .unwrap();
        let mut failures = Vec::new();
        let mut expected_target: Option<(&str, u32)> = None;
        for op in stages[0]["operations"].as_array().unwrap() {
            let label = op["id"].as_str().unwrap();
            match label {
                "write/named" => expected_target = Some(("minecraft:diamond", 3)),
                "modify/named" => expected_target = Some(("minecraft:diamond", 2)),
                "from/named" => expected_target = Some(("minecraft:stone", 5)),
                "override/named" => expected_target = Some(("minecraft:dirt", 4)),
                _ => {}
            }
            let cmd = op["command"].as_str().unwrap();
            let want = op["result"].as_i64();
            for h in ["r", "s"] {
                assert!(game.run_command(&format!("scoreboard players set #{h} fullslot -99")));
            }
            let accepted = game.run_command(&format!(
                "execute store result score #r fullslot store success score #s fullslot run {cmd}"
            ));
            let result = game.score.board.score("#r", "fullslot").unwrap().value;
            let success = game.score.board.score("#s", "fullslot").unwrap().value;
            let expected = (
                want.unwrap_or(-99) as i32,
                want.map_or(-99, |n| i32::from(n > 0)),
            );
            println!(
                "{label}: accepted={accepted}, result={result}, success={success}, expected={expected:?}"
            );
            if (result, success) != expected {
                failures.push(format!("{label}: {:?} != {expected:?}", (result, success)));
            }
            assert_eq!(
                game.block_entity_nbt([0, 100, 0]).unwrap(),
                source,
                "{label}: full source chest"
            );
            assert_eq!(game.world.game_time, clock, "{label}: clock");
            // Each operation's `after` lists checks that must pass once it ran.
            for check in op["after"].as_array().unwrap() {
                let check = check.as_str().unwrap();
                if !game.run_command(check) {
                    failures.push(format!("{label}: after check failed: {check}"));
                }
            }
            let actual_target = game.slot_get(&Slot::Block([4, 100, 0], 0));
            let actual_target = actual_target
                .as_ref()
                .filter(|s| s.count > 0)
                .map(|s| (&*s.item, s.count));
            if actual_target != expected_target {
                failures.push(format!(
                    "{label}: target={actual_target:?}, expected={expected_target:?}"
                ));
            }
            println!(
                "{label}: target full={:?}",
                game.block_entity_nbt([4, 100, 0])
            );
            if label == "fill/named" {
                let stack = game.slot_get(&Slot::Block([4, 100, 0], 2));
                if stack.as_ref().map(|s| (&*s.item, s.count)) != Some(("minecraft:emerald", 3)) {
                    failures.push(format!("{label}: empty-slot recipient wrong {stack:?}"));
                }
            }
            if label=="context/executor_write" && !game.run_command(r#"execute if data entity @e[tag=complete_executor,limit=1] {equipment:{mainhand:{id:"minecraft:gold_ingot",count:6}}}"#) {failures.push("executor did not receive six gold ingots".to_owned());}
            if label == "context/executor_write" {
                for predicate in ["minecraft:gold_ingot[count=6]", "minecraft:gold_ingot[count~6]", "minecraft:gold_ingot"] {
                    let result = game.run_command(&format!("execute if items entity @e[tag=complete_executor,limit=1] weapon.mainhand {predicate}"));
                    println!("count matcher diagnostic (outside source-resolution assertion): {predicate}: {result}");
                }
            }
            if label.starts_with("read/") && cmd.starts_with("execute if ") {
                let complement = cmd.replacen("execute if ", "execute unless ", 1);
                if game.run_command(&complement) {
                    failures.push(format!("{label}: unless complement unexpectedly passed"));
                }
            }
        }
        assert!(
            failures.is_empty(),
            "registered-source completion failures: {failures:#?}"
        );
    }
}
