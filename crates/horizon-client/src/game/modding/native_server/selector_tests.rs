//! Acceptance tests through the registered native command dispatcher and real server selectors.

use super::*;
use horizon_client::mobs::Mob;
use horizon_core::geometry::Vec3;
use mod_api::{CommandArgumentValue as Value, CommandInvocation};
use std::sync::{Arc, Mutex};

struct SelectorCommands(Arc<Mutex<Vec<CommandInvocation>>>);

#[mod_api::__subscriptions]
impl mod_api::Hooks for SelectorCommands {
    fn register_commands(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::RegisterCommandsEvent<'_>,
    ) {
        event.commands.extend([
            mod_api::Command::new("heal <targets:players>"),
            mod_api::Command::new("selectone <target:entity>"),
            mod_api::Command::new("selectall <targets:entities>"),
            mod_api::Command::new("selectplayer <target:player>"),
            mod_api::Command::new("optionalplayers [targets:players]"),
            mod_api::Command::new("optionalentity [target:entity]"),
            mod_api::Command::new("optionalentities [targets:entities]"),
        ]);
    }

    fn run_mod_command(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::RunModCommandEvent<'_>,
    ) -> mod_api::Flow {
        self.0.lock().unwrap().push(event.command.clone());
        mod_api::Flow::Cancel
    }
}

#[test]
#[ignore = "daily"]
fn native_selectors_deliver_ordered_handles_once_and_refuse_invalid_selections() {
    let heard = Arc::new(Mutex::new(Vec::new()));
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new(
            "selector_acceptance",
            Box::new(SelectorCommands(heard.clone())),
        )]
        .into(),
        || {
            super::super::native_guest_turn_tests::with_guests(|game| {
                game.build_registries();
                game.register_native_commands();
                let first = game.player(12).unwrap();
                let second = game.player(13).unwrap();
                let cow = game.res.mob_types().get("minecraft:cow").unwrap().clone();
                // Deliberately neither ID order nor insertion order is distance order.
                for (id, x) in [(101, 12.0), (103, 4.0), (102, 7.0)] {
                    game.mobs
                        .push(Mob::new(id, cow.clone(), Vec3::new(x, 64.0, 5.0), 0.0));
                }
                let near = game.entity(103).unwrap();
                let middle = game.entity(102).unwrap();
                let far = game.entity(101).unwrap();
                game.as_guest(12, |game| {
                    assert!(game.perform_native_command(12, 4, "heal @a"));
                    {
                        let calls = heard.lock().unwrap();
                        assert_eq!(
                            calls.len(),
                            1,
                            "one invocation, not one per selected player"
                        );
                        assert_eq!(
                            calls[0].argument("targets"),
                            Some(&Value::Players(vec![first, second]))
                        );
                    }
                    for line in [
                        "selectone @e[type=minecraft:cow,limit=1,sort=nearest]",
                        "selectall @e[type=minecraft:cow,sort=furthest]",
                        "selectall @e[type=minecraft:cow,sort=nearest]",
                    ] {
                        assert!(game.perform_native_command(12, 4, line), "{line}");
                    }
                    {
                        let calls = heard.lock().unwrap();
                        assert_eq!(calls.len(), 4);
                        assert_eq!(calls[1].argument("target"), Some(&Value::Entity(near)));
                        assert_eq!(
                            calls[2].argument("targets"),
                            Some(&Value::Entities(vec![far, middle, near]))
                        );
                        assert_eq!(
                            calls[3].argument("targets"),
                            Some(&Value::Entities(vec![near, middle, far]))
                        );
                    }
                    // Selector permission is independent of the command's permission (all
                    // these commands are public). Names must still work at level zero.
                    let random = game.world.random.state();
                    for line in ["heal @a", "selectone @r", "selectall @e", "selectplayer @s"] {
                        game.chat_capture = Some(Vec::new());
                        assert!(game.perform_native_command(12, 0, line), "{line}");
                        let output = game.chat_capture.take().unwrap();
                        assert_eq!(
                            output[0].plain(),
                            Text::translate("argument.entity.selector.not_allowed", &[]).plain(),
                            "{line}"
                        );
                    }
                    assert_eq!(game.world.random.state(), random);
                    assert_eq!(
                        heard.lock().unwrap().len(),
                        4,
                        "denied selectors never dispatch"
                    );
                    for line in [
                        "heal guest13",
                        "selectone guest13",
                        "selectall guest13",
                        "selectplayer guest13",
                    ] {
                        assert!(game.perform_native_command(12, 0, line), "{line}");
                    }
                    for line in [
                        "optionalplayers",
                        "optionalentity",
                        "optionalentities",
                        "optionalplayers @a",
                        "optionalentity @s",
                        "optionalentities @s",
                    ] {
                        assert!(game.perform_native_command(12, 4, line), "{line}");
                    }
                })
                .unwrap();
                {
                    let calls = heard.lock().unwrap();
                    assert_eq!(calls.len(), 14);
                    assert_eq!(
                        calls[4].argument("targets"),
                        Some(&Value::Players(vec![second]))
                    );
                    assert_eq!(
                        calls[5].argument("target"),
                        Some(&Value::Entity(second.entity()))
                    );
                    assert_eq!(
                        calls[6].argument("targets"),
                        Some(&Value::Entities(vec![second.entity()]))
                    );
                    assert_eq!(calls[7].argument("target"), Some(&Value::Player(second)));
                    assert!(calls[8..11].iter().all(|call| call.arguments.is_empty()));
                    assert_eq!(
                        calls[11].argument("targets"),
                        Some(&Value::Players(vec![first, second]))
                    );
                    assert_eq!(
                        calls[12].argument("target"),
                        Some(&Value::Entity(first.entity()))
                    );
                    assert_eq!(
                        calls[13].argument("targets"),
                        Some(&Value::Entities(vec![first.entity()]))
                    );
                }
                for (line, key) in [
                    ("heal @a[name=missing]", "argument.entity.notfound.player"),
                    ("selectplayer missing", "argument.entity.notfound.player"),
                    (
                        "selectall @e[type=minecraft:cow,name=missing]",
                        "argument.entity.notfound.entity",
                    ),
                    (
                        "selectone @e[type=minecraft:cow,name=missing,limit=1]",
                        "argument.entity.notfound.entity",
                    ),
                    ("selectplayer @a", "argument.player.toomany"),
                    (
                        "selectone @e[type=minecraft:cow]",
                        "argument.entity.toomany",
                    ),
                    (
                        "optionalplayers @a[name=missing]",
                        "argument.entity.notfound.player",
                    ),
                    (
                        "optionalentities @e[type=minecraft:cow,name=missing]",
                        "argument.entity.notfound.entity",
                    ),
                ] {
                    game.chat_capture = Some(Vec::new());
                    assert!(game.perform_native_console_command(line), "{line}");
                    let output = game.chat_capture.take().unwrap();
                    assert_eq!(
                        output[0].plain(),
                        Text::translate(key, &[]).plain(),
                        "{line}"
                    );
                    assert_eq!(
                        heard.lock().unwrap().len(),
                        14,
                        "{line} must not reach the listener"
                    );
                }

                // Resident mobs remain selector candidates while their native handle
                // is temporarily held out during a mutation.
                game.held_out.push(103);
                assert!(game.entity(103).is_none());
                for line in [
                    "selectall @e[type=minecraft:cow,x=3,y=64,z=5,distance=..2]",
                    "selectone @e[type=minecraft:cow,x=3,y=64,z=5,limit=1,sort=nearest]",
                ] {
                    game.chat_capture = Some(Vec::new());
                    assert!(game.perform_native_console_command(line));
                    let output = game.chat_capture.take().unwrap();
                    assert_eq!(
                        output[0].plain(),
                        Text::translate("argument.entity.notfound.entity", &[]).plain()
                    );
                    assert_eq!(heard.lock().unwrap().len(), 14);
                }
                // Drop only the unavailable handle, preserving the remaining order.
                game.as_guest(12, |game| {
                    assert!(game.perform_native_command(
                        12,
                        4,
                        "selectall @e[type=minecraft:cow,sort=nearest]"
                    ));
                })
                .unwrap();
                game.held_out.pop();
                let calls = heard.lock().unwrap();
                assert_eq!(calls.len(), 15);
                assert_eq!(
                    calls[14].argument("targets"),
                    Some(&Value::Entities(vec![middle, far]))
                );
            });
        },
    );
}

#[test]
fn selector_kinds_publish_vanilla_flags_and_share_parser_suggestions() {
    let commands = [
        mod_api::Command::new("one <target:entity>"),
        mod_api::Command::new("many <target:entities>"),
        mod_api::Command::new("person <target:player>"),
        mod_api::Command::new("people <target:players>"),
    ];
    let (tree, _, problems) = command_tree(&commands, |_| false, &HashMap::new());
    assert!(problems.is_empty(), "{problems:?}");
    let mut dispatcher = Dispatcher::new();
    register_dispatcher(
        &mut dispatcher,
        &tree,
        "native_mod_command",
        &HashMap::new(),
    );
    let mut registries = horizon_client::command::Registries {
        permission: 4,
        players: vec!["Alice".into(), "Bob".into()],
        selected_entities: vec!["00000000-0000-0000-0000-000000000007".into()],
        ..Default::default()
    };
    for (name, amount, kind, single, players) in [
        ("one", "single", "entities", true, false),
        ("many", "multiple", "entities", false, false),
        ("person", "single", "players", true, true),
        ("people", "multiple", "players", false, true),
    ] {
        let node = &tree["children"][name]["children"]["target"];
        assert_eq!(node["parser"], "minecraft:entity");
        assert_eq!(node["properties"], json!({"amount": amount, "type": kind}));
        let root = dispatcher.nodes[0]
            .children
            .iter()
            .copied()
            .find(|id| {
                matches!(&dispatcher.nodes[*id].kind,
                horizon_client::command::NodeKind::Literal(literal) if literal == name)
            })
            .unwrap();
        let argument = dispatcher.nodes[root].children[0];
        assert!(matches!(&dispatcher.nodes[argument].kind,
            horizon_client::command::NodeKind::Argument {
                ty: ArgType::Entity { single: s, players: p }, ..
            } if *s == single && *p == players));
        for (prefix, expected) in [("Al", "Alice"), ("@", "@a")] {
            let input = format!("{name} {prefix}");
            let parsed = dispatcher.parse(&input, &registries);
            let (_, suggestions) = dispatcher.suggestions(&parsed, input.len(), &registries);
            assert!(
                suggestions
                    .iter()
                    .any(|suggestion| suggestion.text == expected),
                "{input}: {suggestions:?}"
            );
        }
        let input = format!("{name} 000");
        let parsed = dispatcher.parse(&input, &registries);
        let (_, suggestions) = dispatcher.suggestions(&parsed, input.len(), &registries);
        assert_eq!(
            suggestions
                .iter()
                .any(|suggestion| suggestion.text == "00000000-0000-0000-0000-000000000007"),
            !players
        );
        registries.permission = 0;
        let input = format!("{name} @");
        let parsed = dispatcher.parse(&input, &registries);
        let (_, suggestions) = dispatcher.suggestions(&parsed, input.len(), &registries);
        assert!(
            !suggestions
                .iter()
                .any(|suggestion| suggestion.text.starts_with('@'))
        );
        registries.permission = 4;
    }
}

#[test]
#[ignore = "daily"]
fn entity_selection_has_no_additional_count_cap() {
    let heard = Arc::new(Mutex::new(Vec::new()));
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new(
            "selector_uncapped",
            Box::new(SelectorCommands(heard.clone())),
        )]
        .into(),
        || {
            super::super::native_guest_turn_tests::with_guests(|game| {
                game.build_registries();
                game.register_native_commands();
                let cow = game.res.mob_types().get("minecraft:cow").unwrap().clone();
                // Reversed insertion order rules out accidental list/ID ordering.
                for id in (1000..1300).rev() {
                    game.mobs.push(Mob::new(
                        id,
                        cow.clone(),
                        Vec3::new(f64::from(id - 1000) + 4.0, 64.0, 5.0),
                        0.0,
                    ));
                }
                let epoch = game.player(12).unwrap().entity().epoch();
                game.as_guest(12, |game| {
                    assert!(game.perform_native_command(
                        12,
                        4,
                        "selectall @e[type=minecraft:cow,sort=nearest]"
                    ));
                })
                .unwrap();
                let calls = heard.lock().unwrap();
                assert_eq!(calls.len(), 1);
                assert_eq!(
                    calls[0].argument("targets"),
                    Some(&Value::Entities(
                        (1000..1300)
                            .map(|id| mod_api::EntityRef::new(epoch, id))
                            .collect()
                    ))
                );
            });
        },
    );
}

/// One overworld cow (entity 1, tag `over`) and one nether cow (entity 2, tag `under`), then
/// `lines` run as the first guest with the console's permission; each line's last invocation
/// names the expected entities in its `argument` (their ids, sorted). Entity ids are handed
/// out in summon order.
fn in_two_levels(lines: &[(&str, &str, &[u32])]) {
    let heard = Arc::new(Mutex::new(Vec::new()));
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new(
            "selector_two_levels",
            Box::new(SelectorCommands(heard.clone())),
        )]
        .into(),
        || {
            super::super::native_guest_turn_tests::with_guests(|game| {
                game.build_registries();
                game.register_native_commands();
                game.ui.registries.permission = 4;
                assert!(game.run_command(r#"summon minecraft:cow 4 64 5 {Tags:["over"]}"#));
                assert!(game.run_command(
                    r#"execute in minecraft:the_nether run summon minecraft:cow 4 64 5 {Tags:["under"]}"#
                ));
                game.lan_assign_ids();
                game.as_guest(12, |game| {
                    for (line, argument, expected) in lines {
                        game.ui.registries.permission = 4;
                        let before = heard.lock().unwrap().len();
                        assert!(game.run_command(line), "{line}");
                        let calls = heard.lock().unwrap();
                        assert_eq!(calls.len(), before + 1, "{line} reaches the mod once");
                        let mut ids = match calls.last().unwrap().argument(argument) {
                            Some(Value::Entity(entity)) => vec![entity.id()],
                            Some(Value::Entities(entities)) => {
                                entities.iter().map(|entity| entity.id()).collect()
                            }
                            other => panic!("{line}: {other:?}"),
                        };
                        ids.sort_unstable();
                        assert_eq!(&ids, expected, "{line}");
                    }
                })
                .unwrap();
            });
        },
    );
}

/// `native_command_entity_target` looks a selected entity up in the level that holds it
/// (`in_target_level`): a mod command run from the overworld reaches the entity that lives in
/// the nether, and each tag picks its own cow, not the one with the other id.
#[test]
#[ignore = "daily"]
fn a_typed_entity_argument_reaches_an_entity_in_another_level() {
    in_two_levels(&[
        (
            "selectone @e[type=minecraft:cow,tag=over,limit=1]",
            "target",
            &[1],
        ),
        (
            "selectone @e[type=minecraft:cow,tag=under,limit=1]",
            "target",
            &[2],
        ),
    ]);
}

/// A selector with no positional option reaches every level; `distance` (like `x`, `y`, `z`,
/// `dx`, `dy` and `dz`) confines it to the source's level, which `execute in` sets; and
/// `execute as` makes the chosen entity `@s`, not the invoker.
#[test]
#[ignore = "daily"]
fn execute_moves_the_selector_source_to_its_level_and_entity() {
    in_two_levels(&[
        // No positional option: both levels, whatever level the source is in.
        ("selectall @e[type=minecraft:cow]", "targets", &[1, 2]),
        (
            "execute in minecraft:the_nether run selectall @e[type=minecraft:cow]",
            "targets",
            &[1, 2],
        ),
        (
            "execute in minecraft:the_nether run selectall @e[type=minecraft:cow,tag=over]",
            "targets",
            &[1],
        ),
        // Confined by `distance`: the level of the source `execute in` made.
        (
            "execute in minecraft:the_nether run selectall @e[type=minecraft:cow,distance=..1000]",
            "targets",
            &[2],
        ),
        (
            "execute in minecraft:overworld run selectall @e[type=minecraft:cow,distance=..1000]",
            "targets",
            &[1],
        ),
        (
            "selectall @e[type=minecraft:cow,distance=..1000]",
            "targets",
            &[1],
        ),
        // `as` the overworld cow: `@s` is that cow, not the invoker.
        (
            "execute as @e[type=minecraft:cow,tag=over] run selectone @s",
            "target",
            &[1],
        ),
    ]);
}
